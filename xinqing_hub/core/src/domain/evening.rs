//! 晚间小结（C-10，05 FR-REV-01）：什么时候出、统计什么、结束语选哪句。全是纯函数，读库、推送界面在 [`crate::evening`]。
//!
//! 结束语按当天主导状态从本地模板选，不调用 AI（FR-REV-01 第 3 条），也不加 AI 标识。只用统计值，不碰任何文字。
//! 规格没写清的地方写在 docs/adr/0029。

use std::collections::HashMap;

use chrono::{DateTime, Duration, Local, NaiveDate, NaiveTime, TimeZone, Timelike};
use serde::{Deserialize, Serialize};
use xqp::MoodState;

use crate::domain::validate::{BannedWords, Scene};
use crate::infra::templates::{TemplateDirs, TemplateError, read_toml};

/// 当天活跃输入至少这么多分钟才出小结。
pub const MIN_TYPING_MIN: u32 = 30;
/// 到点时用户不在电脑前，就等到这个时刻之前的下一次输入（次日凌晨 3 点）。
pub const DEADLINE_HOUR: u32 = 3;
/// 最近这么久内有过按键或上屏，算“在电脑前”。
pub const PRESENT_WITHIN_MS: i64 = 2 * 60_000;
/// 这个时刻之后才出的小结用 `late` 组（evening.toml 文件头：当晚停止打字晚于 23:30）。
pub const LATE_FROM: (u32, u32) = (23, 30);
/// 设置 `review.evening.time` 的可选值（FR-REV-01：21:00–23:30），默认 22:30。
pub const TIMES: &[&str] = &["21:00", "21:30", "22:00", "22:30", "23:00", "23:30"];
/// 内部键：上一次出小结是哪一天的晚上（`YYYY-MM-DD`），登记在 `settings::INTERNAL_KEYS`。
pub const SHOWN_ON_KEY: &str = "review.evening.shown_on";
/// 天气色带最多几格。
pub const BAND_MAX: usize = 8;

/// 这一刻属于哪一天的“晚上”：凌晨 3 点前算前一天。
pub fn evening_date(now: DateTime<Local>) -> NaiveDate {
    let d = now.date_naive();
    if now.hour() < DEADLINE_HOUR {
        d.pred_opt().unwrap_or(d)
    } else {
        d
    }
}

/// 解析 `review.evening.time`（`"22:30"`）；不认识的值回落 22:30。
pub fn parse_time(s: &str) -> NaiveTime {
    NaiveTime::parse_from_str(s, "%H:%M")
        .unwrap_or_else(|_| NaiveTime::from_hms_opt(22, 30, 0).expect("合法时刻"))
}

/// 判断现在该不该出小结所需的事实。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub now: DateTime<Local>,
    pub enabled: bool,
    /// 设置的时刻
    pub at: NaiveTime,
    /// 上一次出小结是哪一天的晚上（内部键 `review.evening.shown_on`）
    pub shown_on: Option<NaiveDate>,
    /// 那天的活跃输入分钟数（`daily_summary.typing_min`）
    pub typing_min: u32,
    /// 最近一次按键或上屏（Unix 毫秒）；Hub 启动后还没有输入时为 `None`
    pub last_input_ms: Option<i64>,
}

/// 现在要不要出小结；要出时返回它属于哪一天。每天最多一次（FR-REV-01 第 1 条）。
pub fn due(c: &Check) -> Option<NaiveDate> {
    if !c.enabled {
        return None;
    }
    let day = evening_date(c.now);
    if c.shown_on == Some(day) {
        return None;
    }
    let start = Local.from_local_datetime(&day.and_time(c.at)).earliest()?;
    if c.now < start {
        return None;
    }
    if c.typing_min < MIN_TYPING_MIN {
        return None;
    }
    // 到点了但人不在：等下一次输入（凌晨 3 点前，过了 3 点 `evening_date` 就换成新的一天）
    let present = c
        .last_input_ms
        .is_some_and(|t| c.now.timestamp_millis() - t <= PRESENT_WITHIN_MS);
    present.then_some(day)
}

/// 结束语的分组（evening.toml）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Group {
    Sunny,
    Hesitant,
    Low,
    Agitated,
    Tired,
    /// 各状态占比都 < 40%
    Mixed,
    /// 23:30 以后才出的小结
    Late,
}

/// 当天主导状态：显示次数最多、且占比 ≥ 40% 的状态；没有就是 `None`（各状态都不到 40%，或没有记录）。
/// `unknown` 不参与。
pub fn dominant(states: &[MoodState]) -> Option<MoodState> {
    let known: Vec<_> = states
        .iter()
        .copied()
        .filter(|s| *s != MoodState::Unknown)
        .collect();
    let mut count: HashMap<MoodState, usize> = HashMap::new();
    for s in &known {
        *count.entry(*s).or_default() += 1;
    }
    // 次数相同时按固定顺序取，结果不随哈希顺序变
    let order = [
        MoodState::Fluent,
        MoodState::Hesitant,
        MoodState::Low,
        MoodState::Agitated,
        MoodState::Tired,
    ];
    let (best, n) = order
        .iter()
        .map(|s| (*s, count.get(s).copied().unwrap_or(0)))
        .max_by_key(|(_, n)| *n)?;
    (n > 0 && n * 5 >= known.len() * 2).then_some(best)
}

impl Group {
    pub fn pick(dominant: Option<MoodState>, shown_at: DateTime<Local>) -> Self {
        let late =
            shown_at.hour() < DEADLINE_HOUR || (shown_at.hour(), shown_at.minute()) >= LATE_FROM;
        if late {
            return Group::Late;
        }
        match dominant {
            Some(MoodState::Fluent) => Group::Sunny,
            Some(MoodState::Hesitant) => Group::Hesitant,
            Some(MoodState::Low) => Group::Low,
            Some(MoodState::Agitated) => Group::Agitated,
            Some(MoodState::Tired) => Group::Tired,
            Some(MoodState::Unknown) | None => Group::Mixed,
        }
    }
}

/// 天气色带（卡片上的 ☀️⛅☀️🌧☀️）：按小时取每小时显示最多的状态，相邻相同的合并，最多 [`BAND_MAX`] 格，
/// 超出时均匀抽取。`states` 是当天 `(Unix 毫秒, 显示状态)`，从旧到新。
pub fn band(states: &[(i64, MoodState)]) -> Vec<MoodState> {
    let mut hours: Vec<(i64, Vec<MoodState>)> = Vec::new();
    for (ts, s) in states {
        if *s == MoodState::Unknown {
            continue;
        }
        let h = ts.div_euclid(3_600_000);
        match hours.last_mut() {
            Some((last, v)) if *last == h => v.push(*s),
            _ => hours.push((h, vec![*s])),
        }
    }
    let mut out: Vec<MoodState> = Vec::new();
    for (_, v) in hours {
        let s = most_common(&v);
        if out.last() != Some(&s) {
            out.push(s);
        }
    }
    if out.len() > BAND_MAX {
        let n = out.len();
        out = (0..BAND_MAX).map(|i| out[i * n / BAND_MAX]).collect();
    }
    out
}

fn most_common(v: &[MoodState]) -> MoodState {
    let mut best = (v[0], 0);
    for s in v {
        let n = v.iter().filter(|x| *x == s).count();
        if n > best.1 {
            best = (*s, n);
        }
    }
    best.0
}

/// 晚间小结卡片的内容（`review:evening` 事件）。只有统计值和一句本地模板。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct EveningSummary {
    /// 哪一天的小结（本地 `YYYY-MM-DD`）
    pub date: String,
    /// 打字时长（分钟）
    pub typing_min: u32,
    /// 喝水次数（点了喝水提醒的“已完成”）
    pub water: u32,
    /// 休息提醒：完成 / 出现次数
    pub rests_done: u32,
    pub rests_due: u32,
    /// 完成的日程（当天已到点、已加入的日程）与待办数
    pub schedules_done: u32,
    pub todos_done: u32,
    /// 天气色带，从早到晚
    pub band: Vec<MoodState>,
    /// 结束语（本地模板，不加 AI 标识）
    pub line: String,
}

/// 一天的统计（外壳从库里读出来交给服务）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DayFacts {
    pub typing_min: u32,
    pub water: u32,
    pub rests_done: u32,
    pub rests_due: u32,
    pub schedules_done: u32,
    pub todos_done: u32,
    /// 当天 `(Unix 毫秒, 显示状态)`，从旧到新
    pub states: Vec<(i64, MoodState)>,
}

/// `evening.toml`。
#[derive(Debug, Clone)]
pub struct EveningTemplates {
    groups: HashMap<Group, Vec<String>>,
}

#[derive(Deserialize)]
struct Line {
    text: String,
}

#[derive(Deserialize)]
struct RawEvening {
    version: u32,
    #[serde(default)]
    sunny: Vec<Line>,
    #[serde(default)]
    hesitant: Vec<Line>,
    #[serde(default)]
    low: Vec<Line>,
    #[serde(default)]
    agitated: Vec<Line>,
    #[serde(default)]
    tired: Vec<Line>,
    #[serde(default)]
    mixed: Vec<Line>,
    #[serde(default)]
    late: Vec<Line>,
}

impl EveningTemplates {
    pub fn load(dirs: &TemplateDirs) -> Result<Self, TemplateError> {
        let banned = BannedWords::load(dirs)?;
        dirs.load_with_fallback("evening.toml", |path| {
            let raw: RawEvening = read_toml(path)?;
            if raw.version == 0 {
                return Err(TemplateError::Invalid {
                    file: "evening.toml",
                    reason: "版本号必须为正整数",
                });
            }
            let texts = |v: Vec<Line>| v.into_iter().map(|l| l.text).collect::<Vec<_>>();
            let groups = HashMap::from([
                (Group::Sunny, texts(raw.sunny)),
                (Group::Hesitant, texts(raw.hesitant)),
                (Group::Low, texts(raw.low)),
                (Group::Agitated, texts(raw.agitated)),
                (Group::Tired, texts(raw.tired)),
                (Group::Mixed, texts(raw.mixed)),
                (Group::Late, texts(raw.late)),
            ]);
            for lines in groups.values() {
                if lines.is_empty()
                    || lines.iter().any(|text| {
                        text.trim().is_empty() || banned.find(text, Scene::Other).is_some()
                    })
                {
                    return Err(TemplateError::Invalid {
                        file: "evening.toml",
                        reason: "七组结束语均须非空且不含禁用内容",
                    });
                }
            }
            Ok(Self { groups })
        })
    }

    /// 从组里选一句；组是空的就退到 `mixed`。`seed` 由调用方给（测试可固定）。
    pub fn pick(&self, group: Group, seed: u64) -> String {
        let pool = self
            .groups
            .get(&group)
            .filter(|v| !v.is_empty())
            .or_else(|| self.groups.get(&Group::Mixed))
            .filter(|v| !v.is_empty());
        match pool {
            Some(v) => v[(mix(seed) % v.len() as u64) as usize].clone(),
            None => "今天辛苦了，明天见。".into(),
        }
    }

    /// 每组的句子（校验用）。
    pub fn all(&self) -> impl Iterator<Item = &String> {
        self.groups.values().flatten()
    }
}

fn mix(seed: u64) -> u64 {
    let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// 组装卡片内容。`shown_at` 决定是不是 `late` 组。
pub fn summarize(
    date: NaiveDate,
    facts: &DayFacts,
    shown_at: DateTime<Local>,
    templates: &EveningTemplates,
) -> EveningSummary {
    let states: Vec<MoodState> = facts.states.iter().map(|(_, s)| *s).collect();
    let group = Group::pick(dominant(&states), shown_at);
    EveningSummary {
        date: date.format("%Y-%m-%d").to_string(),
        typing_min: facts.typing_min,
        water: facts.water,
        rests_done: facts.rests_done,
        rests_due: facts.rests_due,
        schedules_done: facts.schedules_done,
        todos_done: facts.todos_done,
        band: band(&facts.states),
        line: templates.pick(group, shown_at.timestamp_millis() as u64),
    }
}

/// 当天 0 点到次日 0 点的 Unix 毫秒区间（统计“当天”的数据用）。
pub fn day_range_ms(date: NaiveDate) -> (i64, i64) {
    let start = |d: NaiveDate| {
        Local
            .from_local_datetime(&d.and_hms_opt(0, 0, 0).expect("合法时刻"))
            .earliest()
            .map_or(0, |t| t.timestamp_millis())
    };
    let next = date + Duration::days(1);
    (start(date), start(next))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::domain::validate::BannedWords;

    fn at(d: u32, h: u32, m: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 10, d, h, m, 0).unwrap()
    }

    fn check(now: DateTime<Local>) -> Check {
        Check {
            now,
            enabled: true,
            at: parse_time("22:30"),
            shown_on: None,
            typing_min: 45,
            last_input_ms: Some(now.timestamp_millis() - 30_000),
        }
    }

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, d).unwrap()
    }

    #[test]
    fn shows_once_after_the_set_time_when_active_enough() {
        assert_eq!(due(&check(at(5, 22, 29))), None, "没到点");
        assert_eq!(due(&check(at(5, 22, 30))), Some(day(5)));
        let mut c = check(at(5, 22, 31));
        c.shown_on = Some(day(5));
        assert_eq!(due(&c), None, "同一天不重复");
        c.typing_min = 29;
        c.shown_on = None;
        assert_eq!(due(&c), None, "活跃不足 30 分钟");
        c.typing_min = 30;
        c.enabled = false;
        assert_eq!(due(&c), None, "设置里关了");
    }

    #[test]
    fn waits_for_the_user_until_three_am() {
        let mut c = check(at(5, 22, 30));
        c.last_input_ms = Some(at(5, 21, 0).timestamp_millis());
        assert_eq!(due(&c), None, "人不在电脑前");
        // 凌晨 1 点回来打字：仍是 5 号的小结
        let mut c = check(at(6, 1, 0));
        c.typing_min = 45;
        assert_eq!(due(&c), Some(day(5)));
        // 过了 3 点就是新的一天，6 号还没到点
        assert_eq!(due(&check(at(6, 3, 0))), None);
        assert_eq!(evening_date(at(6, 2, 59)), day(5));
    }

    #[test]
    fn dominant_needs_forty_percent() {
        use MoodState::*;
        assert_eq!(dominant(&[Low, Low, Fluent, Tired, Hesitant]), Some(Low));
        assert_eq!(dominant(&[Low, Fluent, Tired, Hesitant, Agitated]), None);
        assert_eq!(
            dominant(&[Unknown, Unknown, Tired]),
            Some(Tired),
            "unknown 不算"
        );
        assert_eq!(dominant(&[]), None);
        assert_eq!(Group::pick(Some(Fluent), at(5, 22, 30)), Group::Sunny);
        assert_eq!(Group::pick(None, at(5, 22, 30)), Group::Mixed);
        assert_eq!(Group::pick(Some(Fluent), at(5, 23, 30)), Group::Late);
        assert_eq!(Group::pick(Some(Low), at(6, 1, 0)), Group::Late);
    }

    #[test]
    fn band_is_hourly_and_collapsed() {
        use MoodState::*;
        let h = 3_600_000;
        let states = [
            (0, Fluent),
            (10, Low),
            (20, Fluent),
            (h, Fluent),
            (2 * h, Low),
            (2 * h + 1, Unknown),
            (3 * h, Low),
            (4 * h, Fluent),
        ];
        assert_eq!(band(&states), [Fluent, Low, Fluent]);
        let many: Vec<_> = (0..20)
            .map(|i| (i * h, if i % 2 == 0 { Fluent } else { Tired }))
            .collect();
        assert_eq!(band(&many).len(), BAND_MAX);
    }

    #[test]
    fn templates_load_and_pass_banned_words() {
        let dirs = TemplateDirs::factory_only(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../hub_templates"),
        );
        let t = EveningTemplates::load(&dirs).unwrap();
        let banned = BannedWords::load(&dirs).unwrap();
        for g in [
            Group::Sunny,
            Group::Hesitant,
            Group::Low,
            Group::Agitated,
            Group::Tired,
            Group::Mixed,
            Group::Late,
        ] {
            assert!(!t.pick(g, 1).is_empty(), "{g:?} 组是空的");
        }
        for line in t.all() {
            assert_eq!(
                banned.find(line, crate::domain::validate::Scene::Other),
                None,
                "{line}"
            );
        }
    }

    #[test]
    fn summary_has_numbers_band_and_a_local_line() {
        let dirs = TemplateDirs::factory_only(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../hub_templates"),
        );
        let t = EveningTemplates::load(&dirs).unwrap();
        let facts = DayFacts {
            typing_min: 252,
            water: 3,
            rests_done: 5,
            rests_due: 7,
            schedules_done: 2,
            todos_done: 3,
            states: vec![(0, MoodState::Fluent), (3_600_000, MoodState::Low)],
        };
        let s = summarize(day(5), &facts, at(5, 22, 30), &t);
        assert_eq!(s.date, "2026-10-05");
        assert_eq!(
            (s.typing_min, s.water, s.rests_done, s.rests_due),
            (252, 3, 5, 7)
        );
        assert_eq!(s.band, [MoodState::Fluent, MoodState::Low]);
        assert!(!s.line.is_empty());
        let (a, b) = day_range_ms(day(5));
        assert_eq!(b - a, 24 * 3_600_000);
    }
}
