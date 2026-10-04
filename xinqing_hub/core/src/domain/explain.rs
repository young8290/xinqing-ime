//! 状态解释（FR-STA-09）：用一两句话说清“为什么这样判断”，全部在本地生成，不调用 AI。
//!
//! [`build`] 只产出结构化的结果（信号种类 + 数字），界面按 `explain.toml` 的文案键自行拼句，
//! 这里不放任何用户文字、应用名（DS-COPY-03）；[`ExplainCopy::render`] 按同一份模板拼出整句，
//! 给回放工具、评测和文案校验用。
//!
//! 信号挑选（表格条件见 04 FR-STA-09 第 1 条）：
//! 1. 候选来自触发切换的窗口和上一个窗口，同一信号只留触发窗口的值；
//! 2. 与显示状态相符的信号排在前面（例如低落只认“慢”和“间隔长”，不会说“比平时快”）；
//!    没有相符的信号时才用其余信号，一条都没有时界面用 `note.no_signal`，保证每次切换至少一条说明（KPI-10）；
//! 3. 同一档里，本地规则的证据（停顿、放弃、删已上屏、手误、翻页、时长、深夜）排在 z 分数信号前面，
//!    z 分数信号按 |z| 从大到小；
//! 4. 最多 [`MAX_SIGNALS`] 条。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use xqp::MoodState;

use super::features::{Baseline, Bucket, WindowFeatures};
use super::rules::{Hint, Hints};
use crate::infra::templates::{TemplateDirs, TemplateError, read_toml};

/// 一次解释最多几条说明（与 `explain.toml` 的 `max_signals` 一致）。
pub const MAX_SIGNALS: usize = 3;

/// 说明种类，序列化名就是 `explain.toml` 中 `[signal.*]` 的键。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum SignalKind {
    /// `kpm_z ≤ -1`，值为 {p}
    KpmSlow,
    /// `kpm_z ≥ 1`，值为 {p}
    KpmFast,
    /// `iki_med_z ≥ 1`
    IkiLong,
    /// `iki_iqr_z ≥ 1.5`
    IkiMessy,
    /// `pause_cnt ≥ 2`，值为 {n}
    Pause,
    /// `abandon = true`
    Abandon,
    /// `delete_committed ≥ 5`，值为 {n}
    DeleteCommitted,
    /// `bs_rate_z ≥ 1`
    BsMore,
    /// `typo_cnt ≥ 2`，值为 {n}
    Typo,
    /// `page_flips ≥ 3`，值为 {n}
    PageFlips,
    /// `session_min ≥ 45`，值为 {m}
    Session,
    /// R5 命中
    Late,
}

impl SignalKind {
    pub fn key(self) -> &'static str {
        match self {
            SignalKind::KpmSlow => "kpm_slow",
            SignalKind::KpmFast => "kpm_fast",
            SignalKind::IkiLong => "iki_long",
            SignalKind::IkiMessy => "iki_messy",
            SignalKind::Pause => "pause",
            SignalKind::Abandon => "abandon",
            SignalKind::DeleteCommitted => "delete_committed",
            SignalKind::BsMore => "bs_more",
            SignalKind::Typo => "typo",
            SignalKind::PageFlips => "page_flips",
            SignalKind::Session => "session",
            SignalKind::Late => "late",
        }
    }

    /// 文案里的变量名（`{p}` / `{n}` / `{m}`），没有变量的为 `None`。
    pub fn var(self) -> Option<&'static str> {
        match self {
            SignalKind::KpmSlow | SignalKind::KpmFast => Some("p"),
            SignalKind::Pause
            | SignalKind::DeleteCommitted
            | SignalKind::Typo
            | SignalKind::PageFlips => Some("n"),
            SignalKind::Session => Some("m"),
            SignalKind::IkiLong
            | SignalKind::IkiMessy
            | SignalKind::Abandon
            | SignalKind::BsMore
            | SignalKind::Late => None,
        }
    }

    /// 这条信号是否支持显示状态（04 第 3.1 节的状态含义）。
    fn fits(self, state: MoodState) -> bool {
        use SignalKind::*;
        match state {
            MoodState::Hesitant => matches!(
                self,
                Pause | Abandon | DeleteCommitted | PageFlips | IkiLong | KpmSlow | BsMore
            ),
            MoodState::Low => matches!(self, KpmSlow | IkiLong),
            MoodState::Agitated => matches!(self, KpmFast | BsMore | IkiMessy | Typo),
            MoodState::Tired => matches!(self, Session | Late | KpmSlow | BsMore | Typo | IkiLong),
            MoodState::Fluent | MoodState::Unknown => false,
        }
    }
}

/// 一条说明。`value` 是文案变量（{p} 百分比、{n} 次数、{m} 分钟）的值。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Signal {
    pub kind: SignalKind,
    pub value: Option<u32>,
}

/// 判断来源（`explain.toml` 的 `[source]`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum ExplainSource {
    Jev,
    Rule,
    /// 自评（FR-STA-10）
    #[serde(rename = "self")]
    SelfReport,
}

impl ExplainSource {
    pub fn key(self) -> &'static str {
        match self {
            ExplainSource::Jev => "jev",
            ExplainSource::Rule => "rule",
            ExplainSource::SelfReport => "self",
        }
    }
}

/// 一次状态解释（`state_explain` 的返回值，10 第 5.1 节）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Explanation {
    pub state: MoodState,
    /// 可能性百分比 0–100（快照里的 `prob` 是 0–1 小数，这里用整数百分比，名字上区分）；
    /// 只有本地规则或冷启动时为 `None`，界面不显示百分比。
    pub prob_pct: Option<u8>,
    /// 最多 [`MAX_SIGNALS`] 条；为空时界面显示 `note.no_signal`。
    pub signals: Vec<Signal>,
    pub source: ExplainSource,
    /// 冷启动期间追加“还在熟悉你的习惯，判断可能不准”。
    pub cold_start: bool,
}

impl Explanation {
    /// 自评期间的解释（FR-STA-10）：状态是用户说的，没有可能性和信号，来源注明“你说的”。
    pub fn self_report(state: MoodState) -> Self {
        Self {
            state,
            prob_pct: None,
            signals: Vec::new(),
            source: ExplainSource::SelfReport,
            cold_start: false,
        }
    }
}

/// 一个窗口的证据：特征和规则提示。
#[derive(Debug, Clone, Copy)]
pub struct Evidence<'a> {
    pub features: &'a WindowFeatures,
    pub hints: &'a Hints,
}

struct Candidate {
    signal: Signal,
    /// 规则证据排在 z 分数前面；z 分数信号按 |z| 排序。
    score: f64,
    /// 0 = 触发窗口，1 = 上一个窗口
    age: u8,
}

/// `p = round(|x / med − 1| × 100)`，`med` 为个人基线中位数（冷启动时为人群默认值）。
fn pct(x: Option<f64>, med: Option<f64>) -> Option<u32> {
    let (x, med) = (x?, med?);
    if med <= 0.0 || !x.is_finite() {
        return None;
    }
    Some(((x / med - 1.0).abs() * 100.0).round() as u32)
}

/// 按 FR-STA-09 第 1 条的表格列出一个窗口满足条件的全部信号。
fn candidates(e: Evidence<'_>, baseline: &Baseline, age: u8, out: &mut Vec<Candidate>) {
    let f = e.features;
    // 规则证据的排序分：高于任何 z 分数（z 截断在 ±5）
    const RULE: f64 = 10.0;
    let mut push = |kind, value, score| {
        out.push(Candidate {
            signal: Signal { kind, value },
            score,
            age,
        })
    };

    if let Some(z) = f.kpm_z {
        let med = baseline.med(Bucket::from_hour(f.hour), "kpm");
        if z <= -1.0 || z >= 1.0 {
            // 没有中位数就算不出 {p}，这条不说
            if let Some(p) = pct(f.kpm, med) {
                let kind = if z < 0.0 {
                    SignalKind::KpmSlow
                } else {
                    SignalKind::KpmFast
                };
                push(kind, Some(p), z.abs());
            }
        }
    }
    if let Some(z) = f.iki_med_z.filter(|z| *z >= 1.0) {
        push(SignalKind::IkiLong, None, z);
    }
    if let Some(z) = f.iki_iqr_z.filter(|z| *z >= 1.5) {
        push(SignalKind::IkiMessy, None, z);
    }
    if f.pause_cnt >= 2 {
        push(SignalKind::Pause, Some(f.pause_cnt), RULE);
    }
    if f.abandon == 1 {
        push(SignalKind::Abandon, None, RULE);
    }
    if let Some(n) = f.delete_committed.filter(|n| *n >= 5) {
        push(SignalKind::DeleteCommitted, Some(n), RULE);
    }
    if let Some(z) = f.bs_rate_z.filter(|z| *z >= 1.0) {
        push(SignalKind::BsMore, None, z);
    }
    if f.typo_cnt >= 2 {
        push(SignalKind::Typo, Some(f.typo_cnt), RULE);
    }
    if f.page_flips >= 3 {
        push(SignalKind::PageFlips, Some(f.page_flips), RULE);
    }
    if f.session_min >= 45.0 {
        push(
            SignalKind::Session,
            Some(f.session_min.floor() as u32),
            RULE,
        );
    }
    if e.hints.has(Hint::LateNight) {
        push(SignalKind::Late, None, RULE);
    }
}

/// 状态切换时生成解释（17 第 2.4 节 `explain::build()`）。
///
/// `cur` 是触发切换的窗口，`prev` 是它的上一个窗口；`prob` 是显示状态的可能性 0–1。
pub fn build(
    state: MoodState,
    prob: Option<f64>,
    source: ExplainSource,
    cur: Evidence<'_>,
    prev: Option<Evidence<'_>>,
    baseline: &Baseline,
) -> Explanation {
    let mut cands = Vec::new();
    candidates(cur, baseline, 0, &mut cands);
    if let Some(p) = prev {
        candidates(p, baseline, 1, &mut cands);
    }
    // 同一信号只留触发窗口的值
    cands.sort_by_key(|c| c.age);
    let mut seen = Vec::new();
    cands.retain(|c| {
        if seen.contains(&c.signal.kind) {
            false
        } else {
            seen.push(c.signal.kind);
            true
        }
    });

    let any_fit = cands.iter().any(|c| c.signal.kind.fits(state));
    if any_fit {
        cands.retain(|c| c.signal.kind.fits(state));
    } else if matches!(state, MoodState::Fluent | MoodState::Unknown) {
        // 流畅时不罗列偏离平时的地方，只说“节奏和平时差不多”
        cands.clear();
    }
    // 排序：规则证据在前，然后 |z| 大的在前，同分时触发窗口在前；sort_by 是稳定排序，再同分保持表格顺序
    cands.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.age.cmp(&b.age)));
    let signals = cands
        .into_iter()
        .take(MAX_SIGNALS)
        .map(|c| c.signal)
        .collect();

    Explanation {
        state,
        prob_pct: prob.map(|p| (p.clamp(0.0, 1.0) * 100.0).round() as u8),
        signals,
        source,
        cold_start: baseline.is_cold(),
    }
}

/// `explain.toml`（10 第 7 节）。
#[derive(Debug, Clone, Deserialize)]
pub struct ExplainCopy {
    pub version: u32,
    signal: HashMap<String, SignalCopy>,
    hedge: HashMap<String, String>,
    source: HashMap<String, String>,
    note: NoteCopy,
}

/// `[note]` 表。`format`、`separator`、`max_signals` 写在 `[note]` 之后，按 TOML 语法属于这张表
/// （前端生成的文案键也是 `explain.note.format`），这里照此读取。
#[derive(Debug, Clone, Deserialize)]
struct NoteCopy {
    cold_start: String,
    no_signal: String,
    format: String,
    separator: String,
    max_signals: usize,
}

#[derive(Debug, Clone, Deserialize)]
struct SignalCopy {
    text: String,
}

impl ExplainCopy {
    pub fn load(dirs: &TemplateDirs) -> Result<Self, TemplateError> {
        read_toml(&dirs.resolve("explain.toml"))
    }

    /// 一条说明的文案，`None` 表示模板缺这个键。
    pub fn signal_text(&self, s: &Signal) -> Option<String> {
        let text = &self.signal.get(s.kind.key())?.text;
        Some(match (s.kind.var(), s.value) {
            (Some(var), Some(v)) => text.replace(&format!("{{{var}}}"), &v.to_string()),
            _ => text.clone(),
        })
    }

    /// 按模板拼出整句，例如
    /// “看起来有点犹豫（可能性 80%）· 句子中间停顿了 3 次 · 有一段话打了又删｜AI 根据打字节奏判断，可能不准”。
    /// 没有可能性时去掉“（可能性 …%）”这一段；冷启动时在末尾追加附注。
    pub fn render(&self, e: &Explanation) -> String {
        let state = match e.state {
            MoodState::Unknown => MoodState::Fluent,
            s => s,
        };
        let hedge = self.hedge.get(state.as_str()).cloned().unwrap_or_default();
        let mut signals: Vec<String> = e
            .signals
            .iter()
            .take(self.note.max_signals)
            .filter_map(|s| self.signal_text(s))
            .collect();
        if signals.is_empty() {
            signals.push(self.note.no_signal.clone());
        }
        let source = self.source.get(e.source.key()).cloned().unwrap_or_default();

        let mut format = self.note.format.clone();
        if e.prob_pct.is_none() {
            format = strip_prob(&format);
        }
        let mut out = format
            .replace("{hedge}", &hedge)
            .replace(
                "{prob}",
                &e.prob_pct.map(|p| p.to_string()).unwrap_or_default(),
            )
            .replace("{signals}", &signals.join(&self.note.separator))
            .replace("{source}", &source);
        if e.cold_start {
            out.push_str(&self.note.separator);
            out.push_str(&self.note.cold_start);
        }
        out
    }
}

/// 去掉格式串中包着 `{prob}` 的那一对括号（含括号），找不到括号时只去掉 `{prob}`。
/// 全角括号自带间距，去掉后若紧跟的是分隔符，补一个空格（“看起来有点犹豫 · …”）。
fn strip_prob(format: &str) -> String {
    let Some(at) = format.find("{prob}") else {
        return format.to_string();
    };
    let open = format[..at].rfind(['（', '(']);
    let close = format[at..].find(['）', ')']).map(|i| {
        let c = format[at + i..].chars().next().map_or(1, char::len_utf8);
        at + i + c
    });
    match (open, close) {
        (Some(o), Some(c)) => {
            let rest = &format[c..];
            let pad = if rest.is_empty() || rest.starts_with(char::is_whitespace) {
                ""
            } else {
                " "
            };
            format!("{}{pad}{rest}", &format[..o])
        }
        _ => format.replacen("{prob}", "", 1),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::domain::validate::{BannedWords, Scene};
    use crate::infra::templates::{BaselineDefault, MedMad};

    fn dirs() -> TemplateDirs {
        TemplateDirs::factory_only(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../hub_templates"),
        )
    }

    fn baseline() -> Baseline {
        Baseline::from_defaults(&BaselineDefault::load(&dirs()).unwrap())
    }

    /// 个人基线已建立、白天 kpm 中位数 200 的基线。
    fn warm_baseline() -> Baseline {
        let mut b = baseline();
        b.set_personal(
            Bucket::Day,
            "kpm",
            MedMad {
                med: 200.0,
                mad: 40.0,
            },
        );
        b.windows = 500;
        b
    }

    fn feat() -> WindowFeatures {
        WindowFeatures {
            n_keys: 40,
            active_ms: 12_000,
            kpm: Some(200.0),
            hour: 14,
            minute_of_day: 14 * 60,
            kpm_z: Some(0.0),
            iki_med_z: Some(0.0),
            iki_iqr_z: Some(0.0),
            bs_rate_z: Some(0.0),
            ..Default::default()
        }
    }

    fn ev<'a>(f: &'a WindowFeatures, h: &'a Hints) -> Evidence<'a> {
        Evidence {
            features: f,
            hints: h,
        }
    }

    fn kinds(e: &Explanation) -> Vec<SignalKind> {
        e.signals.iter().map(|s| s.kind).collect()
    }

    #[test]
    fn spec_example_hesitant() {
        // 04 FR-STA-09 示例：停顿 3 次、打了又删、慢 35%
        let mut f = feat();
        f.pause_cnt = 3;
        f.abandon = 1;
        f.kpm = Some(130.0);
        f.kpm_z = Some(-1.2);
        f.iki_med_z = Some(0.8);
        let h = Hints(vec![Hint::HesitationHint]);
        let b = warm_baseline();
        let e = build(
            MoodState::Hesitant,
            Some(0.8),
            ExplainSource::Jev,
            ev(&f, &h),
            None,
            &b,
        );
        assert_eq!(
            e.signals,
            vec![
                Signal {
                    kind: SignalKind::Pause,
                    value: Some(3)
                },
                Signal {
                    kind: SignalKind::Abandon,
                    value: None
                },
                Signal {
                    kind: SignalKind::KpmSlow,
                    value: Some(35)
                },
            ]
        );
        assert_eq!(e.prob_pct, Some(80));
        assert!(!e.cold_start);
        let copy = ExplainCopy::load(&dirs()).unwrap();
        assert_eq!(
            copy.render(&e),
            "看起来有点犹豫（可能性 80%）· 句子中间停顿了 3 次 · 有一段话打了又删 · 打字速度比平时慢 35%｜AI 根据打字节奏判断，可能不准"
        );
    }

    #[test]
    fn numbers_match_features() {
        // 每个带数字的说明都与特征一致（FR-STA-09 验收标准）
        let mut f = feat();
        f.kpm = Some(290.0);
        f.kpm_z = Some(2.0);
        f.typo_cnt = 4;
        f.bs_rate_z = Some(1.8);
        f.iki_iqr_z = Some(1.6);
        let h = Hints(vec![Hint::AgitationHint]);
        let e = build(
            MoodState::Agitated,
            Some(0.91),
            ExplainSource::Jev,
            ev(&f, &h),
            None,
            &warm_baseline(),
        );
        // 规则证据（手误）在前，然后按 |z|：kpm 2.0 > bs 1.8 > iqr 1.6，最多 3 条
        assert_eq!(
            kinds(&e),
            vec![SignalKind::Typo, SignalKind::KpmFast, SignalKind::BsMore]
        );
        assert_eq!(e.signals[0].value, Some(4));
        assert_eq!(e.signals[1].value, Some(45)); // |290/200 − 1| = 45%
        assert_eq!(e.prob_pct, Some(91));

        let mut g = feat();
        g.session_min = 52.7;
        g.delete_committed = Some(6);
        g.page_flips = 3;
        let h = Hints(vec![Hint::HesitationHint]);
        let e = build(
            MoodState::Hesitant,
            None,
            ExplainSource::Rule,
            ev(&g, &h),
            None,
            &warm_baseline(),
        );
        assert_eq!(
            e.signals,
            vec![
                Signal {
                    kind: SignalKind::DeleteCommitted,
                    value: Some(6)
                },
                Signal {
                    kind: SignalKind::PageFlips,
                    value: Some(3)
                },
            ],
            "时长与犹豫无关，不出现"
        );
    }

    #[test]
    fn signals_must_fit_state() {
        // 低落时“比平时快”和“手误”都不该出现
        let mut cur = feat();
        cur.kpm = Some(120.0);
        cur.kpm_z = Some(-1.5);
        cur.iki_med_z = Some(1.3);
        cur.typo_cnt = 3;
        let mut prev = feat();
        prev.kpm_z = Some(1.4);
        prev.kpm = Some(260.0);
        let h = Hints(vec![Hint::LowHint]);
        let none = Hints::default();
        let e = build(
            MoodState::Low,
            Some(0.85),
            ExplainSource::Jev,
            ev(&cur, &h),
            Some(ev(&prev, &none)),
            &warm_baseline(),
        );
        assert_eq!(kinds(&e), vec![SignalKind::KpmSlow, SignalKind::IkiLong]);
        assert_eq!(e.signals[0].value, Some(40));
    }

    #[test]
    fn previous_window_fills_in_and_current_wins() {
        let mut cur = feat();
        cur.pause_cnt = 2;
        let mut prev = feat();
        prev.pause_cnt = 4;
        prev.abandon = 1;
        let h = Hints(vec![Hint::HesitationHint]);
        let e = build(
            MoodState::Hesitant,
            Some(0.9),
            ExplainSource::Jev,
            ev(&cur, &h),
            Some(ev(&prev, &h)),
            &warm_baseline(),
        );
        // 停顿取触发窗口的 2 次；“打了又删”来自上一个窗口
        assert_eq!(
            e.signals,
            vec![
                Signal {
                    kind: SignalKind::Pause,
                    value: Some(2)
                },
                Signal {
                    kind: SignalKind::Abandon,
                    value: None
                },
            ]
        );
    }

    #[test]
    fn tired_late_night_rule_only_cold_start() {
        let mut f = feat();
        f.hour = 0;
        f.minute_of_day = 10;
        f.session_min = 15.0;
        let h = Hints(vec![Hint::LateNight]);
        let b = baseline();
        assert!(b.is_cold());
        let e = build(
            MoodState::Tired,
            None,
            ExplainSource::Rule,
            ev(&f, &h),
            None,
            &b,
        );
        assert_eq!(kinds(&e), vec![SignalKind::Late]);
        assert!(e.cold_start);
        let copy = ExplainCopy::load(&dirs()).unwrap();
        assert_eq!(
            copy.render(&e),
            "可能有点累了 · 现在已经很晚了｜本地规则判断（离线） · 还在熟悉你的习惯，判断可能不准"
        );
    }

    #[test]
    fn every_change_has_at_least_one_line() {
        // KPI-10：没有任何信号时（包括回到流畅）也有一条说明
        let f = feat();
        let h = Hints::default();
        let copy = ExplainCopy::load(&dirs()).unwrap();
        for s in [
            MoodState::Fluent,
            MoodState::Hesitant,
            MoodState::Low,
            MoodState::Agitated,
            MoodState::Tired,
        ] {
            let e = build(
                s,
                Some(0.95),
                ExplainSource::Jev,
                ev(&f, &h),
                None,
                &warm_baseline(),
            );
            assert!(e.signals.is_empty());
            let text = copy.render(&e);
            assert!(text.contains("节奏和平时差不多"), "{s:?}: {text}");
        }
        // 流畅时不罗列偏离
        let mut g = feat();
        g.bs_rate_z = Some(2.0);
        let e = build(
            MoodState::Fluent,
            Some(0.7),
            ExplainSource::Jev,
            ev(&g, &h),
            None,
            &warm_baseline(),
        );
        assert!(e.signals.is_empty());
        // 状态与信号都对不上时，退而说出现有的信号，而不是“和平时差不多”
        let e = build(
            MoodState::Low,
            Some(0.85),
            ExplainSource::Jev,
            ev(&g, &h),
            None,
            &warm_baseline(),
        );
        assert_eq!(kinds(&e), vec![SignalKind::BsMore]);
    }

    #[test]
    fn missing_median_drops_speed_signal() {
        let mut f = feat();
        f.kpm = None;
        f.kpm_z = Some(-2.0);
        let h = Hints::default();
        let e = build(
            MoodState::Low,
            Some(0.9),
            ExplainSource::Jev,
            ev(&f, &h),
            None,
            &warm_baseline(),
        );
        assert!(e.signals.is_empty());
        assert_eq!(pct(Some(100.0), Some(0.0)), None);
    }

    #[test]
    fn rendered_copy_passes_banned_words() {
        // DS-COPY-03：所有说明、不确定说法、来源与附注都过禁用词校验
        let copy = ExplainCopy::load(&dirs()).unwrap();
        let bw = BannedWords::load(&dirs()).unwrap();
        let all = [
            SignalKind::KpmSlow,
            SignalKind::KpmFast,
            SignalKind::IkiLong,
            SignalKind::IkiMessy,
            SignalKind::Pause,
            SignalKind::Abandon,
            SignalKind::DeleteCommitted,
            SignalKind::BsMore,
            SignalKind::Typo,
            SignalKind::PageFlips,
            SignalKind::Session,
            SignalKind::Late,
        ];
        for k in all {
            let s = Signal {
                kind: k,
                value: k.var().map(|_| 12),
            };
            let text = copy.signal_text(&s).expect("explain.toml 缺少信号文案");
            assert!(!text.contains('{'), "{k:?} 变量没替换：{text}");
        }
        for state in [
            MoodState::Fluent,
            MoodState::Hesitant,
            MoodState::Low,
            MoodState::Agitated,
            MoodState::Tired,
        ] {
            for source in [
                ExplainSource::Jev,
                ExplainSource::Rule,
                ExplainSource::SelfReport,
            ] {
                for chunk in all.chunks(MAX_SIGNALS) {
                    let e = Explanation {
                        state,
                        prob_pct: Some(77),
                        signals: chunk
                            .iter()
                            .map(|k| Signal {
                                kind: *k,
                                value: k.var().map(|_| 12),
                            })
                            .collect(),
                        source,
                        cold_start: true,
                    };
                    let text = copy.render(&e);
                    assert!(!text.contains('{'), "变量没替换：{text}");
                    assert_eq!(bw.find(&text, Scene::Other), None, "{text}");
                }
            }
        }
    }

    #[test]
    fn strip_prob_variants() {
        assert_eq!(
            strip_prob("{hedge}（可能性 {prob}%）· {signals}｜{source}"),
            "{hedge} · {signals}｜{source}"
        );
        assert_eq!(strip_prob("{hedge} ({prob}%) x"), "{hedge}  x");
        assert_eq!(strip_prob("{hedge} {prob}"), "{hedge} ");
        assert_eq!(strip_prob("{hedge}"), "{hedge}");
    }

    #[test]
    fn serializes_with_template_keys() {
        let e = Explanation {
            state: MoodState::Tired,
            prob_pct: None,
            signals: vec![Signal {
                kind: SignalKind::DeleteCommitted,
                value: Some(5),
            }],
            source: ExplainSource::SelfReport,
            cold_start: false,
        };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["signals"][0]["kind"], "delete_committed");
        assert_eq!(v["source"], "self");
        assert_eq!(v["state"], "tired");
    }
}
