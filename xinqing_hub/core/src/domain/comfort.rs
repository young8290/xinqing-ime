//! 暖心话的领域逻辑（C-04：05 FR-CMF-01～03/06、04 FR-STA-10 第 2 条、08 P-COMFORT 与第 5 节）。
//!
//! 这里只有纯函数和小状态机：触发判断 [`Trigger`]、状态摘要 [`Summary`]、提示词 [`ComfortPrompt`]、
//! 输出校验 [`validate`]、本地模板 [`ComfortTemplates`]。什么时候调用、调网关、写库、推送界面在
//! [`crate::care`]。规格没写清的地方写在 docs/adr/0015。
//!
//! 隐私：状态摘要只有状态、时长、时刻这类统计值，不含任何文字（D-21）；自评备注从不进来（NFR-PRI-09）。

use std::collections::{HashMap, HashSet};
use std::path::Path;

use chrono::{DateTime, Local, Timelike};
use serde::Deserialize;
use xqp::MoodState;

use super::self_report::SelfWeather;
use super::validate::{self, BannedWords, Scene};
use crate::infra::templates::{TemplateDirs, TemplateError, read_toml};

/// FR-CMF-01 第 1 条：Jev 的 `need_comfort` 至少这么高才主动关怀。
pub const NEED_COMFORT: f64 = 0.75;
/// FR-CMF-01 第 2 条：显示状态至少持续这么多个窗口。
pub const MIN_WINDOWS: u32 = 2;
/// V5 与“最近说过的话”看最近几条（FR-CMF-02 第 3 条、FR-CMF-03 第 2 条）。
pub const RECENT: usize = 10;
/// 暖心话的长度（V3，按汉字计）。
pub const MIN_HAN: usize = 6;
pub const MAX_HAN: usize = 30;
/// “简洁”风格从温柔模板里选不超过这么多字的句子（comfort.toml 文件头）。
pub const BRIEF_MAX_HAN: usize = 15;
/// FR-CMF-01 第 5 条的默认勿扰应用（小写比较）。PowerPoint 放映属于全屏，由外壳的全屏探测覆盖。
pub const DND_APPS: &[&str] = &["wemeetapp.exe", "zoom.exe", "ms-teams.exe"];

/// 主动关怀频率（设置 `care.level`，FR-CMF-06）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CareLevel {
    More,
    Normal,
    Less,
    Off,
}

impl CareLevel {
    /// 设置值：`more` / `normal` / `less` / `off`；不认识的值按默认“适中”。
    pub fn parse(s: &str) -> Self {
        match s {
            "more" => CareLevel::More,
            "less" => CareLevel::Less,
            "off" => CareLevel::Off,
            _ => CareLevel::Normal,
        }
    }

    pub fn daily_cap(self) -> u32 {
        match self {
            CareLevel::More => 8,
            CareLevel::Normal => 5,
            CareLevel::Less => 2,
            CareLevel::Off => 0,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            CareLevel::More => "more",
            CareLevel::Normal => "normal",
            CareLevel::Less => "less",
            CareLevel::Off => "off",
        }
    }

    /// 自动降一档（FR-CMF-06 第 2 条）：最低降到“少一些”，不会自动关掉；已经是“少一些”或关闭时为 `None`。
    pub fn reduced(self) -> Option<CareLevel> {
        match self {
            CareLevel::More => Some(CareLevel::Normal),
            CareLevel::Normal => Some(CareLevel::Less),
            CareLevel::Less | CareLevel::Off => None,
        }
    }

    pub fn cooldown_ms(self) -> i64 {
        const MIN: i64 = 60_000;
        match self {
            CareLevel::More => 20 * MIN,
            CareLevel::Normal => 30 * MIN,
            CareLevel::Less => 60 * MIN,
            CareLevel::Off => i64::MAX,
        }
    }
}

/// 晴晴的说话风格（设置 `care.style`，FR-CHT-10）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Gentle,
    Lively,
    Brief,
}

impl Style {
    pub fn parse(s: &str) -> Self {
        match s {
            "lively" => Style::Lively,
            "brief" => Style::Brief,
            _ => Style::Gentle,
        }
    }

    /// 08 第 4 节 `{style_block}` 的填入内容；温柔风格为空。
    pub fn style_block(self) -> &'static str {
        match self {
            Style::Gentle => "",
            Style::Lively => "语气轻快、带一点鼓劲，可以用“冲”“没事”这类口语，但不夸张、不说教。",
            Style::Brief => "尽量简短克制，能用一句话就不用两句，不加语气词。",
        }
    }

    /// 模板文件里的风格分组：简洁从温柔里挑短句。
    fn template_style(self) -> &'static str {
        match self {
            Style::Lively => "lively",
            Style::Gentle | Style::Brief => "gentle",
        }
    }
}

/// 模板分组（comfort.toml）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Group {
    Hesitant,
    Low,
    Agitated,
    Tired,
    LateNight,
    Cheer,
}

impl Group {
    pub const ALL: [Group; 6] = [
        Group::Hesitant,
        Group::Low,
        Group::Agitated,
        Group::Tired,
        Group::LateNight,
        Group::Cheer,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Group::Hesitant => "hesitant",
            Group::Low => "low",
            Group::Agitated => "agitated",
            Group::Tired => "tired",
            Group::LateNight => "late_night",
            Group::Cheer => "cheer",
        }
    }

    /// 深夜一律用 `late_night` 组（轻轻提醒休息），其余时间按状态分组。
    pub fn pick(state: MoodState, time: TimeOfDay) -> Self {
        if time == TimeOfDay::LateNight {
            return Group::LateNight;
        }
        match state {
            MoodState::Hesitant => Group::Hesitant,
            MoodState::Low => Group::Low,
            MoodState::Agitated => Group::Agitated,
            MoodState::Tired => Group::Tired,
            MoodState::Fluent | MoodState::Unknown => Group::Cheer,
        }
    }
}

/// 状态摘要里的时段（FR-CMF-02 示例的 `time_of_day`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeOfDay {
    Morning,
    Noon,
    Afternoon,
    Evening,
    LateNight,
}

impl TimeOfDay {
    /// 05–11 上午、11–14 中午、14–18 下午、18–23 晚上、23–05 深夜。
    pub fn from_hour(h: u32) -> Self {
        match h {
            5..=10 => TimeOfDay::Morning,
            11..=13 => TimeOfDay::Noon,
            14..=17 => TimeOfDay::Afternoon,
            18..=22 => TimeOfDay::Evening,
            _ => TimeOfDay::LateNight,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            TimeOfDay::Morning => "morning",
            TimeOfDay::Noon => "noon",
            TimeOfDay::Afternoon => "afternoon",
            TimeOfDay::Evening => "evening",
            TimeOfDay::LateNight => "late_night",
        }
    }
}

/// 主动关怀能不能发生的外部条件（由服务从设置、库和外壳收集）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gate {
    pub level: CareLevel,
    /// 今天已经主动关怀的次数
    pub today: u32,
    /// 上一次主动关怀的时间（Unix 毫秒）
    pub last_ms: Option<i64>,
    /// 无痕 / 暂停感知（FR-CMF-01 第 6 条）
    pub paused: bool,
    /// 勿扰情形（FR-CMF-01 第 5 条）
    pub dnd: bool,
    /// 用户点了 🔕 今天先别说了（FR-CMF-05）
    pub muted: bool,
}

/// 这一次没有主动关怀的原因（只用于测试和调试日志，不含用户数据）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skip {
    /// 没有 Jev 结果（降级运行时不主动关怀，04 FR-STA-08 第 2 条）
    NoJev,
    LowNeed,
    State,
    TooShort,
    Off,
    DailyCap,
    Cooldown,
    Paused,
    Dnd,
    Muted,
}

/// `ComfortWanted`（10 第 5.3 节）：要为哪个状态说一句话、这个状态持续了多久。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wanted {
    pub state: MoodState,
    pub duration_min: u32,
    pub at_ms: i64,
}

/// FR-CMF-01 的触发判断：跟踪显示状态持续了几个窗口、从什么时候开始。
#[derive(Debug, Clone, Default)]
pub struct Trigger {
    shown: Option<MoodState>,
    windows: u32,
    since_ms: i64,
}

impl Trigger {
    /// 每个窗口（`MoodEvent::Sample`）调用一次。
    pub fn on_sample(
        &mut self,
        ts: i64,
        shown: MoodState,
        need_comfort: Option<f64>,
        gate: Gate,
    ) -> Result<Wanted, Skip> {
        if self.shown == Some(shown) {
            self.windows += 1;
        } else {
            self.shown = Some(shown);
            self.windows = 1;
            self.since_ms = ts;
        }
        let need = need_comfort.ok_or(Skip::NoJev)?;
        if need < NEED_COMFORT {
            return Err(Skip::LowNeed);
        }
        if !matches!(
            shown,
            MoodState::Hesitant | MoodState::Low | MoodState::Agitated | MoodState::Tired
        ) {
            return Err(Skip::State);
        }
        if self.windows < MIN_WINDOWS {
            return Err(Skip::TooShort);
        }
        if gate.level == CareLevel::Off {
            return Err(Skip::Off);
        }
        if gate.today >= gate.level.daily_cap() {
            return Err(Skip::DailyCap);
        }
        if gate
            .last_ms
            .is_some_and(|last| ts - last < gate.level.cooldown_ms())
        {
            return Err(Skip::Cooldown);
        }
        if gate.muted {
            return Err(Skip::Muted);
        }
        if gate.paused {
            return Err(Skip::Paused);
        }
        if gate.dnd {
            return Err(Skip::Dnd);
        }
        Ok(Wanted {
            state: shown,
            duration_min: minutes(ts - self.since_ms),
            at_ms: ts,
        })
    }

    /// 无痕、断开或会话重置后，之前的状态持续不再算数。
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

fn minutes(ms: i64) -> u32 {
    u32::try_from(ms.max(0) / 60_000).unwrap_or(u32::MAX)
}

/// 应用是否在默认勿扰列表里（忽略大小写）。
pub fn is_dnd_app(app: &str) -> bool {
    let a = app.to_lowercase();
    DND_APPS.iter().any(|d| *d == a)
}

/// 负面自评对应的状态（FR-STA-10 第 2 条）；其余自评不回应。
pub fn self_report_state(w: SelfWeather) -> Option<MoodState> {
    w.is_negative().then(|| w.state()).flatten()
}

/// 状态摘要（FR-CMF-02 第 1 条，09 D-21）：只有统计值，没有任何原文。
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    pub state: MoodState,
    pub duration_min: u32,
    pub local_time: DateTime<Local>,
    pub session_min: u32,
    /// `falling` / `rising` / `steady`；没有 Jev 的效价时不写
    pub valence_trend: Option<&'static str>,
}

impl Summary {
    pub fn time_of_day(&self) -> TimeOfDay {
        TimeOfDay::from_hour(self.local_time.hour())
    }

    pub fn to_json(&self) -> serde_json::Value {
        let mut m = serde_json::Map::new();
        m.insert("state".into(), state_str(self.state).into());
        m.insert("duration_min".into(), self.duration_min.into());
        m.insert(
            "local_time".into(),
            self.local_time.format("%H:%M").to_string().into(),
        );
        m.insert("session_min".into(), self.session_min.into());
        if let Some(t) = self.valence_trend {
            m.insert("valence_trend".into(), t.into());
        }
        m.insert("time_of_day".into(), self.time_of_day().as_str().into());
        serde_json::Value::Object(m)
    }
}

/// 最近几个窗口的效价（Jev Q-VALENCE，0–4，从旧到新）的走向；少于 2 个值时为 `None`。
pub fn valence_trend(recent: &[f64]) -> Option<&'static str> {
    let (first, last) = (recent.first()?, recent.last()?);
    if recent.len() < 2 {
        return None;
    }
    Some(match last - first {
        d if d <= -0.5 => "falling",
        d if d >= 0.5 => "rising",
        _ => "steady",
    })
}

pub fn state_str(s: MoodState) -> &'static str {
    match s {
        MoodState::Fluent => "fluent",
        MoodState::Hesitant => "hesitant",
        MoodState::Low => "low",
        MoodState::Agitated => "agitated",
        MoodState::Tired => "tired",
        MoodState::Unknown => "unknown",
    }
}

/// P-COMFORT 提示词（`prompts/comfort.md`，只用出厂版本：提示词改动要评审并重跑评测，08 第 8 节）。
#[derive(Debug, Clone)]
pub struct ComfortPrompt {
    pub version: u32,
    body: String,
}

impl ComfortPrompt {
    pub fn load(dirs: &TemplateDirs) -> Result<Self, TemplateError> {
        let path = dirs.factory_path("prompts/comfort.md");
        let text = std::fs::read_to_string(&path).map_err(|source| TemplateError::Io {
            path: path.clone(),
            source,
        })?;
        Ok(Self::parse(&text))
    }

    /// 首行 `<!-- version: N -->`；所有 `<!--` 开头的行是注释，不发给模型。
    pub fn parse(text: &str) -> Self {
        let version = text
            .lines()
            .next()
            .and_then(|l| l.trim().strip_prefix("<!-- version:"))
            .and_then(|l| l.trim_end_matches("-->").trim().parse().ok())
            .unwrap_or(0);
        let body = text
            .lines()
            .filter(|l| !l.trim_start().starts_with("<!--"))
            .collect::<Vec<_>>()
            .join("\n");
        Self { version, body }
    }

    /// 记录在 `comfort_log.prompt_ver`、`CompleteRequest.prompt_ver`。
    pub fn ver(&self) -> String {
        format!("P-COMFORT v{}", self.version)
    }

    pub fn render(&self, style: Style, summary: &Summary, recent: &[String]) -> String {
        let block = style.style_block();
        let body = if block.is_empty() {
            // 温柔风格不填：连同占位符所在的空行一起去掉
            self.body.replace("{style_block}\n", "")
        } else {
            self.body.replace("{style_block}", block)
        };
        body.replace("{summary_json}", &summary.to_json().to_string())
            .replace(
                "{recent_texts}",
                &serde_json::to_string(recent).unwrap_or_else(|_| "[]".into()),
            )
            .trim()
            .to_string()
    }
}

/// 大模型输出没通过校验的原因（08 第 5 节）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Invalid {
    /// V1：不是 JSON
    Parse,
    /// V2：缺字段、类型或枚举值不对
    Shape,
    /// V3：不是 6–30 个汉字
    Length,
    /// V4：命中禁用词
    Banned,
    /// V5：和最近 10 条太像
    Repeat,
    /// V7：中文太少
    Language,
}

/// 依次执行 V1、V2、V3、V4、V5、V7，通过时返回要显示的句子。
pub fn validate(raw: &str, banned: &BannedWords, recent: &[String]) -> Result<String, Invalid> {
    let v = validate::extract_json(raw).ok_or(Invalid::Parse)?;
    let text = v
        .get("text")
        .and_then(|t| t.as_str())
        .map(str::trim)
        .ok_or(Invalid::Shape)?;
    let kind_ok = v
        .get("kind")
        .and_then(|k| k.as_str())
        .is_some_and(|k| matches!(k, "comfort" | "rest" | "cheer"));
    if !kind_ok {
        return Err(Invalid::Shape);
    }
    let n = validate::han_count(text);
    if !(MIN_HAN..=MAX_HAN).contains(&n) {
        return Err(Invalid::Length);
    }
    if banned.find(text, Scene::Other).is_some() {
        return Err(Invalid::Banned);
    }
    if !validate::not_repetitive(text, recent) {
        return Err(Invalid::Repeat);
    }
    if !validate::chinese_ratio_ok(text) {
        return Err(Invalid::Language);
    }
    Ok(text.to_string())
}

/// 一句本地模板。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Line {
    pub id: String,
    pub text: String,
}

/// 本地暖心话模板（`comfort.toml`，FR-CMF-03）。允许用户目录同名文件整体覆盖。
#[derive(Debug, Clone, Default)]
pub struct ComfortTemplates {
    pub version: u32,
    lines: HashMap<(&'static str, Group), Vec<Line>>,
}

impl ComfortTemplates {
    pub fn load(dirs: &TemplateDirs) -> Result<Self, TemplateError> {
        Self::from_path(&dirs.resolve("comfort.toml"))
    }

    pub fn from_path(path: &Path) -> Result<Self, TemplateError> {
        #[derive(Deserialize)]
        struct Raw {
            version: u32,
            #[serde(default)]
            gentle: HashMap<String, Vec<Line>>,
            #[serde(default)]
            lively: HashMap<String, Vec<Line>>,
        }
        let raw: Raw = read_toml(path)?;
        let mut lines = HashMap::new();
        for (style, groups) in [("gentle", raw.gentle), ("lively", raw.lively)] {
            for (name, group_lines) in groups {
                if let Some(g) = Group::ALL.into_iter().find(|g| g.as_str() == name) {
                    lines.insert((style, g), group_lines);
                }
            }
        }
        Ok(Self {
            version: raw.version,
            lines,
        })
    }

    fn group(&self, style: Style, group: Group) -> Vec<&Line> {
        let all = self
            .lines
            .get(&(style.template_style(), group))
            .map(Vec::as_slice)
            .unwrap_or_default();
        all.iter()
            .filter(|l| style != Style::Brief || validate::han_count(&l.text) <= BRIEF_MAX_HAN)
            .collect()
    }

    /// 随机选一句：排除最近 10 条说过的和用户屏蔽的（FR-CMF-03 第 2 条）。都被排除时放宽“最近说过”，
    /// 仍没有就退到 `cheer` 组；`seed` 由调用方给（测试可固定）。
    pub fn pick(
        &self,
        style: Style,
        group: Group,
        recent: &[String],
        blocked: &HashSet<String>,
        seed: u64,
    ) -> Option<Line> {
        let recent: Vec<&String> = recent.iter().rev().take(RECENT).collect();
        let usable = |g: Group, skip_recent: bool| -> Vec<&Line> {
            self.group(style, g)
                .into_iter()
                .filter(|l| !blocked.contains(&l.id))
                .filter(|l| !skip_recent || !recent.contains(&&l.text))
                .collect()
        };
        [
            (group, true),
            (group, false),
            (Group::Cheer, true),
            (Group::Cheer, false),
        ]
        .into_iter()
        .map(|(g, skip)| usable(g, skip))
        .find(|c| !c.is_empty())
        .map(|c| c[(mix(seed) % c.len() as u64) as usize].clone())
    }
}

/// splitmix64：把时间戳这类相邻的种子打散成均匀的下标。
fn mix(seed: u64) -> u64 {
    let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use chrono::TimeZone;

    use super::*;

    fn dirs() -> TemplateDirs {
        TemplateDirs::factory_only(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../hub_templates"),
        )
    }

    fn open() -> Gate {
        Gate {
            level: CareLevel::Normal,
            today: 0,
            last_ms: None,
            paused: false,
            dnd: false,
            muted: false,
        }
    }

    const MIN: i64 = 60_000;

    #[test]
    fn triggers_after_two_windows_of_a_negative_state() {
        let mut t = Trigger::default();
        let low = MoodState::Low;
        assert_eq!(t.on_sample(0, low, Some(0.9), open()), Err(Skip::TooShort));
        let w = t.on_sample(12 * MIN, low, Some(0.9), open()).unwrap();
        assert_eq!((w.state, w.duration_min, w.at_ms), (low, 12, 12 * MIN));
        // 状态变了重新计数
        assert_eq!(
            t.on_sample(13 * MIN, MoodState::Tired, Some(0.9), open()),
            Err(Skip::TooShort)
        );
    }

    #[test]
    fn every_condition_of_fr_cmf_01_blocks() {
        let mut t = Trigger::default();
        let s = MoodState::Hesitant;
        t.on_sample(0, s, None, open()).unwrap_err();
        assert_eq!(t.on_sample(1, s, None, open()), Err(Skip::NoJev));
        assert_eq!(t.on_sample(2, s, Some(0.74), open()), Err(Skip::LowNeed));
        let gate = |f: fn(&mut Gate)| {
            let mut g = open();
            f(&mut g);
            g
        };
        let now = 100 * MIN;
        assert_eq!(
            t.on_sample(now, s, Some(0.8), gate(|g| g.level = CareLevel::Off)),
            Err(Skip::Off)
        );
        assert_eq!(
            t.on_sample(now, s, Some(0.8), gate(|g| g.today = 5)),
            Err(Skip::DailyCap)
        );
        assert_eq!(
            t.on_sample(now, s, Some(0.8), gate(|g| g.last_ms = Some(71 * MIN))),
            Err(Skip::Cooldown),
            "适中档冷却 30 分钟"
        );
        assert!(
            t.on_sample(now, s, Some(0.8), gate(|g| g.last_ms = Some(70 * MIN)))
                .is_ok()
        );
        assert_eq!(
            t.on_sample(now, s, Some(0.8), gate(|g| g.paused = true)),
            Err(Skip::Paused)
        );
        assert_eq!(
            t.on_sample(now, s, Some(0.8), gate(|g| g.dnd = true)),
            Err(Skip::Dnd)
        );
        assert_eq!(
            t.on_sample(now, s, Some(0.8), gate(|g| g.muted = true)),
            Err(Skip::Muted)
        );
        let mut f = Trigger::default();
        f.on_sample(0, MoodState::Fluent, Some(0.9), open())
            .unwrap_err();
        assert_eq!(
            f.on_sample(1, MoodState::Fluent, Some(0.9), open()),
            Err(Skip::State)
        );
    }

    #[test]
    fn care_levels_follow_fr_cmf_06() {
        let more = CareLevel::parse("more");
        assert_eq!((more.daily_cap(), more.cooldown_ms()), (8, 20 * MIN));
        let normal = CareLevel::parse("whatever");
        assert_eq!((normal.daily_cap(), normal.cooldown_ms()), (5, 30 * MIN));
        let less = CareLevel::parse("less");
        assert_eq!((less.daily_cap(), less.cooldown_ms()), (2, 60 * MIN));
        assert_eq!(CareLevel::parse("off").daily_cap(), 0);
        for l in [
            CareLevel::More,
            CareLevel::Normal,
            CareLevel::Less,
            CareLevel::Off,
        ] {
            assert_eq!(CareLevel::parse(l.as_str()), l);
        }
        assert_eq!(CareLevel::More.reduced(), Some(CareLevel::Normal));
        assert_eq!(CareLevel::Less.reduced(), None, "不会自动关掉");
    }

    #[test]
    fn dnd_apps_and_self_report_mapping() {
        assert!(is_dnd_app("WeMeetApp.exe"));
        assert!(!is_dnd_app("WeChat.exe"));
        assert_eq!(self_report_state(SelfWeather::Rain), Some(MoodState::Low));
        assert_eq!(self_report_state(SelfWeather::Sunny), None);
        assert_eq!(self_report_state(SelfWeather::Unsure), None);
    }

    fn summary(h: u32, m: u32) -> Summary {
        Summary {
            state: MoodState::Hesitant,
            duration_min: 12,
            local_time: Local.with_ymd_and_hms(2026, 10, 5, h, m, 0).unwrap(),
            session_min: 52,
            valence_trend: valence_trend(&[2.0, 1.2]),
        }
    }

    #[test]
    fn summary_matches_the_spec_example() {
        let j = summary(23, 47).to_json();
        assert_eq!(
            j,
            serde_json::json!({"state":"hesitant","duration_min":12,"local_time":"23:47",
                "session_min":52,"valence_trend":"falling","time_of_day":"late_night"})
        );
        assert_eq!(valence_trend(&[2.0]), None);
        assert_eq!(valence_trend(&[2.0, 2.2]), Some("steady"));
        let mut no_jev = summary(14, 0);
        no_jev.valence_trend = None;
        assert!(no_jev.to_json().get("valence_trend").is_none());
    }

    #[test]
    fn prompt_fills_variables_and_drops_comments() {
        let p = ComfortPrompt::load(&dirs()).unwrap();
        assert_eq!(p.ver(), "P-COMFORT v1");
        let recent = vec!["慢慢来".to_string()];
        let gentle = p.render(Style::Gentle, &summary(23, 47), &recent);
        assert!(!gentle.contains("<!--"), "注释行不发给模型");
        assert!(gentle.contains("\"local_time\":\"23:47\""));
        assert!(gentle.contains("[\"慢慢来\"]"));
        assert!(!gentle.contains("{style_block}") && !gentle.contains("{summary_json}"));
        let lively = p.render(Style::Lively, &summary(9, 0), &[]);
        assert!(lively.contains("语气轻快"));
        assert!(lively.contains("最近说过的话（不要重复）：[]"));
    }

    fn banned() -> BannedWords {
        BannedWords::load(&dirs()).unwrap()
    }

    #[test]
    fn validation_runs_v1_to_v7() {
        let b = banned();
        let ok = r#"```json
{"text":"先停十秒，深呼吸一下？","kind":"rest"}
```"#;
        assert_eq!(validate(ok, &b, &[]).unwrap(), "先停十秒，深呼吸一下？");
        assert_eq!(validate("好的", &b, &[]), Err(Invalid::Parse));
        assert_eq!(
            validate(r#"{"text":"慢慢来不着急哦","kind":"hug"}"#, &b, &[]),
            Err(Invalid::Shape)
        );
        assert_eq!(
            validate(r#"{"text":"加油","kind":"cheer"}"#, &b, &[]),
            Err(Invalid::Length)
        );
        assert_eq!(
            validate(
                r#"{"text":"你可能有点抑郁，早点休息吧","kind":"comfort"}"#,
                &b,
                &[]
            ),
            Err(Invalid::Banned),
            "TC-CMF-05"
        );
        assert_eq!(
            validate(
                r#"{"text":"想说的话不急着说完，慢慢来。","kind":"comfort"}"#,
                &b,
                &["想说的话不急着说完，慢慢来吧。".to_string()]
            ),
            Err(Invalid::Repeat)
        );
        assert_eq!(
            validate(
                r#"{"text":"Take it easy, 休息一下下吧好不好呀","kind":"rest"}"#,
                &b,
                &[]
            ),
            Err(Invalid::Language)
        );
    }

    #[test]
    fn every_factory_template_passes_the_same_checks() {
        // 模板是人工撰写的，但也必须过长度与禁用词（comfort.toml 文件头的要求）
        let t = ComfortTemplates::load(&dirs()).unwrap();
        let b = banned();
        for style in [Style::Gentle, Style::Lively] {
            for g in Group::ALL {
                let lines = t.group(style, g);
                assert!(lines.len() >= 15, "{style:?}/{g:?} 不足 15 句");
                for l in lines {
                    let n = validate::han_count(&l.text);
                    assert!((MIN_HAN..=MAX_HAN).contains(&n), "{} 长度 {n}", l.id);
                    assert!(
                        b.find(&l.text, Scene::Other).is_none(),
                        "{} 命中禁用词",
                        l.id
                    );
                }
            }
        }
        // 简洁风格每组都还有句子可选
        for g in Group::ALL {
            assert!(
                !t.group(Style::Brief, g).is_empty(),
                "简洁风格 {g:?} 没有短句"
            );
        }
    }

    #[test]
    fn template_pick_skips_recent_and_blocked_and_rarely_repeats() {
        let t = ComfortTemplates::load(&dirs()).unwrap();
        let mut recent: Vec<String> = Vec::new();
        let blocked: HashSet<String> = ["gl01".to_string()].into();
        // TC-CMF-03：连续 20 次，10 次以内不重复，屏蔽的句子不出现
        for i in 0..20u64 {
            let l = t
                .pick(Style::Gentle, Group::Low, &recent, &blocked, 1_000 + i)
                .unwrap();
            assert_ne!(l.id, "gl01");
            assert!(
                !recent.iter().rev().take(RECENT).any(|r| *r == l.text),
                "第 {i} 次与最近 10 条重复"
            );
            recent.push(l.text);
        }
        // 整组都被屏蔽：退到 cheer
        let all: HashSet<String> = t
            .group(Style::Gentle, Group::Tired)
            .iter()
            .map(|l| l.id.clone())
            .collect();
        let l = t.pick(Style::Gentle, Group::Tired, &[], &all, 7).unwrap();
        assert!(l.id.starts_with("gc"), "{}", l.id);
        // 简洁：只出短句
        let l = t
            .pick(Style::Brief, Group::Hesitant, &[], &HashSet::new(), 3)
            .unwrap();
        assert!(validate::han_count(&l.text) <= BRIEF_MAX_HAN);
    }

    #[test]
    fn late_night_uses_its_own_group() {
        assert_eq!(
            Group::pick(MoodState::Low, TimeOfDay::from_hour(23)),
            Group::LateNight
        );
        assert_eq!(
            Group::pick(MoodState::Low, TimeOfDay::from_hour(4)),
            Group::LateNight
        );
        assert_eq!(
            Group::pick(MoodState::Low, TimeOfDay::from_hour(5)),
            Group::Low
        );
        assert_eq!(
            Group::pick(MoodState::Agitated, TimeOfDay::Afternoon),
            Group::Agitated
        );
    }
}
