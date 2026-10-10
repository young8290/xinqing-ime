//! 看板与小组件底栏用的统计（07 FR-DSH-02～04、FR-WGT-05，docs/adr/0036）：纯函数，外壳从库里读出原始记录交给这里。
//!
//! 只有计数与状态，没有任何文字（FR-DSH-01：看板不展示用户输入的原文，库里本来也没有）。
//! 周报的一句话总结来自本地模板 `weekly_line.toml`，不调用 AI（FR-DSH-04）。

use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{Datelike, Local, NaiveDate, TimeZone, Timelike};
use serde::{Deserialize, Serialize};
use xqp::MoodState;

use super::validate::{BannedWords, Scene};
use crate::infra::templates::{TemplateDirs, TemplateError, read_toml};

/// 有效窗口少于这么多的天，日历上不显示主导天气（FR-DSH-03）。
pub const MIN_WINDOWS: usize = 10;
/// 时间线上相邻两条状态记录相隔超过这么久就断开：那段时间没在打字。窗口最长 30 秒，合并后不超过 60 秒（ADR 0008）。
pub const TIMELINE_GAP_MS: i64 = 5 * 60_000;
/// 一段的最后一条记录往后延这么久，孤立的一条记录在色带上也看得见。
pub const POINT_SPAN_MS: i64 = 60_000;

/// 参与统计的五种状态，并列时按这个顺序取，结果不随哈希顺序变。
const ORDER: [MoodState; 5] = [
    MoodState::Fluent,
    MoodState::Hesitant,
    MoodState::Low,
    MoodState::Agitated,
    MoodState::Tired,
];

/// 一天的主导状态（FR-DSH-03）：窗口数最多的状态，不计 `unknown`（`typo` 不是显示状态，本来就不在库里）；
/// 有效窗口少于 [`MIN_WINDOWS`] 时为 `None`。与晚间小结的 `evening::dominant` 不同，这里不要求占比 ≥ 40%：
/// 日历每天都要有一个图标。
pub fn dominant(states: &[MoodState]) -> Option<MoodState> {
    let counts = counts(states);
    let known: u32 = counts.values().sum();
    if (known as usize) < MIN_WINDOWS {
        return None;
    }
    ORDER
        .iter()
        .map(|s| (*s, counts.get(s).copied().unwrap_or(0)))
        .fold(None, |best: Option<(MoodState, u32)>, (s, n)| match best {
            Some((_, m)) if m >= n => best,
            _ => Some((s, n)),
        })
        .map(|(s, _)| s)
}

fn counts(states: &[MoodState]) -> HashMap<MoodState, u32> {
    let mut out = HashMap::new();
    for s in states.iter().filter(|s| **s != MoodState::Unknown) {
        *out.entry(*s).or_insert(0) += 1;
    }
    out
}

/// 状态时间线上的一段（FR-DSH-02）：相邻、同状态、间隔不超过 [`TIMELINE_GAP_MS`] 的记录合并成一段。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Segment {
    /// Unix 毫秒
    #[cfg_attr(feature = "specta", specta(type = specta_typescript::Number))]
    pub start_ts: f64,
    #[cfg_attr(feature = "specta", specta(type = specta_typescript::Number))]
    pub end_ts: f64,
    pub state: MoodState,
    /// 这一段最后一条状态记录的行号：点开时用 `state_explain(mood_id)` 取那一刻的解释（FR-STA-09）
    pub mood_id: u32,
}

/// `points` 是 `(mood_state 行号, Unix 毫秒, 显示状态)`，从旧到新。`unknown` 的记录当作空白。
pub fn timeline(points: &[(i64, i64, MoodState)]) -> Vec<Segment> {
    let mut out: Vec<Segment> = Vec::new();
    // 当前这一段：(起点, 最后一条记录的时刻, 状态, 最后一条的行号)
    let mut cur: Option<(i64, i64, MoodState, i64)> = None;
    let close = |out: &mut Vec<Segment>, (start, last, state, id): (i64, i64, MoodState, i64), end: i64| {
        out.push(Segment {
            start_ts: start as f64,
            end_ts: end.max(last) as f64,
            state,
            mood_id: u32::try_from(id).unwrap_or(0),
        });
    };
    for &(id, ts, state) in points {
        let next = match cur {
            Some(c) if state == MoodState::Unknown => {
                close(&mut out, c, c.1 + POINT_SPAN_MS);
                None
            }
            None if state == MoodState::Unknown => None,
            Some((start, last, s, _)) if s == state && ts - last <= TIMELINE_GAP_MS => {
                Some((start, ts, s, id))
            }
            // 换了状态：上一段接到这一刻，色带连续
            Some(c) if ts - c.1 <= TIMELINE_GAP_MS => {
                close(&mut out, c, ts);
                Some((ts, ts, state, id))
            }
            Some(c) => {
                close(&mut out, c, c.1 + POINT_SPAN_MS);
                Some((ts, ts, state, id))
            }
            None => Some((ts, ts, state, id)),
        };
        cur = next;
    }
    if let Some(c) = cur {
        close(&mut out, c, c.1 + POINT_SPAN_MS);
    }
    out
}

/// 某天的概要（看板“今日”的四张概要卡片与时间线、小组件底栏的今日输入时长）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct DayStats {
    /// 本地日期 `YYYY-MM-DD`
    pub date: String,
    /// 输入时长（分钟，FR-RST-01 的活跃分钟）
    pub typing_min: u32,
    /// 休息提醒：出现 / 完成次数；完成率 = 完成 / 出现（FR-RST-08）
    pub rests_due: u32,
    pub rests_done: u32,
    pub water: u32,
    /// 暖心话次数（含自评后的回应）
    pub comforts: u32,
    /// 主导天气（[`dominant`]）；有效窗口不足时为空
    pub dominant: Option<MoodState>,
    pub timeline: Vec<Segment>,
}

/// 情绪日历的一天（FR-DSH-03）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct DayMood {
    pub date: String,
    pub dominant: Option<MoodState>,
    /// 当天有日记：日历上画小本子图标（FR-DIA-03）
    pub has_diary: bool,
}

/// 一个月每天的主导天气。`points` 是这个月的 `(Unix 毫秒, 显示状态)`，按本地日期归天。
pub fn month(
    first: NaiveDate,
    points: &[(i64, MoodState)],
    diary_dates: &HashSet<String>,
) -> Vec<DayMood> {
    let mut by_day: BTreeMap<NaiveDate, Vec<MoodState>> = BTreeMap::new();
    for (ts, s) in points {
        if let Some(t) = Local.timestamp_millis_opt(*ts).single() {
            by_day.entry(t.date_naive()).or_default().push(*s);
        }
    }
    first
        .iter_days()
        .take_while(|d| d.month() == first.month())
        .map(|d| {
            let date = d.format("%Y-%m-%d").to_string();
            DayMood {
                dominant: by_day.get(&d).and_then(|v| dominant(v)),
                has_diary: diary_dates.contains(&date),
                date,
            }
        })
        .collect()
}

/// 周报里的一天（每日输入时长柱状图、休息与饮水、专注）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct WeekDay {
    pub date: String,
    pub typing_min: u32,
    pub rests_due: u32,
    pub rests_done: u32,
    pub water: u32,
    pub focus_min: u32,
}

/// 状态分布的一项（环形图）：这个状态有多少个窗口。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct StateCount {
    pub state: MoodState,
    pub windows: u32,
}

/// 周报（FR-DSH-04）。作息洞察另用 `get_routine`。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct WeekReport {
    /// 那周的周一
    pub week_start: String,
    /// 周一到周日
    pub days: Vec<WeekDay>,
    /// 状态分布，按固定顺序，只列出现过的
    pub states: Vec<StateCount>,
    /// 7 × 24：周一到周日 × 0–23 时，“低落 / 疲劳”出现的窗口数（热力图）
    pub heat: Vec<Vec<u32>>,
    /// 休息完成率（百分比，四舍五入）；这周没出过提醒时为空
    pub rest_rate: Option<u32>,
    pub rests_done: u32,
    pub rests_due: u32,
    pub water: u32,
    /// 本周新加入的日程（已添加的，按创建时间）与完成的待办
    pub schedules_added: u32,
    pub todos_done: u32,
    pub focus_min: u32,
    /// 一句话总结（`weekly_line.toml`，本地模板）
    pub line: String,
}

/// 状态分布，按 [`ORDER`]，只列出现过的。
pub fn state_counts(states: &[MoodState]) -> Vec<StateCount> {
    let c = counts(states);
    ORDER
        .iter()
        .filter_map(|s| {
            c.get(s).map(|n| StateCount {
                state: *s,
                windows: *n,
            })
        })
        .collect()
}

/// 7 × 24 热力图：周一为第 0 行，本地时间的小时为列；只数“低落 / 疲劳”。
pub fn heat(points: &[(i64, MoodState)]) -> Vec<Vec<u32>> {
    let mut grid = vec![vec![0u32; 24]; 7];
    for (ts, s) in points {
        if !matches!(s, MoodState::Low | MoodState::Tired) {
            continue;
        }
        if let Some(t) = Local.timestamp_millis_opt(*ts).single() {
            grid[t.weekday().num_days_from_monday() as usize][t.hour() as usize] += 1;
        }
    }
    grid
}

/// “低落 / 疲劳”出现最多（至少 2 次）的钟点；并列时取更晚的那个（更该提醒早点休息）。
pub fn hard_hour(heat: &[Vec<u32>]) -> Option<u32> {
    // 一天从 05:00 算起，深夜的 0–4 点排在最后
    let rank = |h: u32| (h + 24 - 5) % 24;
    (0..24u32)
        .map(|h| (h, heat.iter().map(|row| row[h as usize]).sum::<u32>()))
        .filter(|(_, n)| *n >= 2)
        .max_by_key(|(h, n)| (*n, rank(*h)))
        .map(|(h, _)| h)
}

/// 周报一句话的分组（`weekly_line.toml`），按文件头注释的顺序取第一条命中的。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LineGroup {
    LateTired,
    RestHigh,
    RestLow,
    Sunny,
    Plain,
}

/// 选一句话要用的统计。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LineFacts {
    /// [`hard_hour`]
    pub hard_hour: Option<u32>,
    /// “周三晚上”这类时段（周信统计的 `hard_slot`）
    pub hard_slot: Option<String>,
    pub rest_rate: Option<u32>,
    /// 流畅（晴）占全部有效窗口的百分比
    pub fluent_share: Option<u32>,
}

#[derive(Deserialize)]
struct Line {
    text: String,
}

#[derive(Deserialize)]
struct RawLines {
    version: u32,
    #[serde(default)]
    late_tired: Vec<Line>,
    #[serde(default)]
    rest_high: Vec<Line>,
    #[serde(default)]
    rest_low: Vec<Line>,
    #[serde(default)]
    sunny: Vec<Line>,
    #[serde(default)]
    plain: Vec<Line>,
}

/// `weekly_line.toml`：周报一句话总结（FR-DSH-04）。
#[derive(Debug, Clone)]
pub struct WeeklyLines {
    groups: HashMap<LineGroup, Vec<String>>,
}

/// 兜底：文件读不出来时也有一句（与 `plain` 组的第一句一致）。
const FALLBACK_LINE: &str = "又过完了一周，辛苦了。";

impl WeeklyLines {
    pub fn load(dirs: &TemplateDirs) -> Result<Self, TemplateError> {
        let banned = BannedWords::load(dirs)?;
        dirs.load_with_fallback("weekly_line.toml", |path| {
            let raw: RawLines = read_toml(path)?;
            let invalid = |reason| TemplateError::Invalid {
                file: "weekly_line.toml",
                reason,
            };
            if raw.version == 0 {
                return Err(invalid("版本号必须为正整数"));
            }
            let texts = |v: Vec<Line>| v.into_iter().map(|l| l.text).collect::<Vec<_>>();
            let groups = HashMap::from([
                (LineGroup::LateTired, texts(raw.late_tired)),
                (LineGroup::RestHigh, texts(raw.rest_high)),
                (LineGroup::RestLow, texts(raw.rest_low)),
                (LineGroup::Sunny, texts(raw.sunny)),
                (LineGroup::Plain, texts(raw.plain)),
            ]);
            if groups[&LineGroup::Plain].is_empty() {
                return Err(invalid("plain 组至少要有一句"));
            }
            if groups.values().flatten().any(|t| {
                t.trim().is_empty() || banned.find(t, Scene::Other).is_some()
            }) {
                return Err(invalid("句子不能为空，也不能含禁用内容"));
            }
            Ok(Self { groups })
        })
    }

    /// 按文件头注释的规则选一句，`seed` 决定同组里用哪句（同一周每次打开都一样）。
    /// 用到的变量缺值的句子跳过；一组都用不了就看下一条规则。
    pub fn pick(&self, f: &LineFacts, seed: u64) -> String {
        let late = f.hard_hour.is_some_and(|h| !(5..22).contains(&h));
        let rules = [
            (LineGroup::LateTired, late),
            (LineGroup::RestHigh, f.rest_rate.is_some_and(|r| r >= 70)),
            (LineGroup::RestLow, f.rest_rate.is_some_and(|r| r < 30)),
            (LineGroup::Sunny, f.fluent_share.is_some_and(|s| s >= 60)),
            (LineGroup::Plain, true),
        ];
        // “晚上 {hour} 点后”：22 → 10，23 → 11，零点以后都说 12
        let hour = f.hard_hour.map(|h| if h >= 22 { h - 12 } else { 12 });
        let vars: [(&str, Option<String>); 3] = [
            ("{hour}", hour.map(|h| h.to_string())),
            ("{slot}", f.hard_slot.clone()),
            ("{rate}", f.rest_rate.map(|r| format!("{r}%"))),
        ];
        for (group, hit) in rules {
            if !hit {
                continue;
            }
            let usable: Vec<String> = self
                .groups
                .get(&group)
                .into_iter()
                .flatten()
                .filter_map(|t| fill(t, &vars))
                .collect();
            if !usable.is_empty() {
                return usable[(seed % usable.len() as u64) as usize].clone();
            }
        }
        FALLBACK_LINE.into()
    }
}

/// 替换变量；句子里有缺值的变量时返回 `None`。
fn fill(text: &str, vars: &[(&str, Option<String>)]) -> Option<String> {
    let mut out = text.to_string();
    for (name, value) in vars {
        if out.contains(name) {
            out = out.replace(name, value.as_deref()?);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use MoodState::*;

    fn at(d: u32, h: u32, m: u32) -> i64 {
        Local
            .with_ymd_and_hms(2026, 10, d, h, m, 0)
            .unwrap()
            .timestamp_millis()
    }

    #[test]
    fn dominant_needs_ten_known_windows_and_takes_the_most_common() {
        let mut v = vec![Tired; 4];
        v.extend([Fluent; 3]);
        v.extend([Unknown; 5]);
        assert_eq!(dominant(&v), None, "unknown 不算有效窗口，只有 7 个");
        v.extend([Fluent; 3]);
        assert_eq!(dominant(&v), Some(Fluent), "6 比 4");
        // 不要求 40%：五种各 2 个时按固定顺序取第一个
        let even: Vec<_> = ORDER.iter().flat_map(|s| [*s, *s]).collect();
        assert_eq!(dominant(&even), Some(Fluent));
        let mut tie = vec![Low; 5];
        tie.extend([Hesitant; 5]);
        assert_eq!(dominant(&tie), Some(Hesitant), "并列按 ORDER 取");
    }

    #[test]
    fn timeline_merges_same_state_and_breaks_on_gaps() {
        let p = [
            (1, at(5, 9, 0), Fluent),
            (2, at(5, 9, 1), Fluent),
            (3, at(5, 9, 2), Tired),
            // 隔了一小时：断开
            (4, at(5, 10, 2), Tired),
            (5, at(5, 10, 3), Unknown),
            (6, at(5, 10, 4), Low),
        ];
        let segs = timeline(&p);
        let got: Vec<_> = segs
            .iter()
            .map(|s| (s.start_ts as i64, s.end_ts as i64, s.state, s.mood_id))
            .collect();
        assert_eq!(
            got,
            [
                (at(5, 9, 0), at(5, 9, 2), Fluent, 2),
                (at(5, 9, 2), at(5, 9, 3), Tired, 3),
                (at(5, 10, 2), at(5, 10, 3), Tired, 4),
                (at(5, 10, 4), at(5, 10, 5), Low, 6),
            ]
        );
        assert!(timeline(&[]).is_empty());
        assert!(timeline(&[(1, at(5, 9, 0), Unknown)]).is_empty());
    }

    #[test]
    fn month_groups_by_local_date_and_marks_diaries() {
        let first = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        let mut p: Vec<(i64, MoodState)> = (0..10).map(|i| (at(3, 9, i), Low)).collect();
        p.push((at(4, 9, 0), Fluent));
        let diaries = HashSet::from(["2026-10-04".to_string()]);
        let m = month(first, &p, &diaries);
        assert_eq!(m.len(), 31);
        assert_eq!(m[0].date, "2026-10-01");
        assert_eq!((m[2].dominant, m[2].has_diary), (Some(Low), false));
        assert_eq!((m[3].dominant, m[3].has_diary), (None, true), "只有 1 个窗口");
        let feb = month(NaiveDate::from_ymd_opt(2026, 2, 1).unwrap(), &[], &diaries);
        assert_eq!(feb.len(), 28);
    }

    #[test]
    fn heat_counts_low_and_tired_by_weekday_and_hour() {
        // 2026-10-05 是周一
        let p = [
            (at(5, 23, 0), Tired),
            (at(5, 23, 5), Low),
            (at(5, 23, 9), Fluent),
            (at(11, 1, 0), Tired),
        ];
        let h = heat(&p);
        assert_eq!(h.len(), 7);
        assert_eq!(h[0][23], 2);
        assert_eq!(h[6][1], 1, "周日凌晨 1 点");
        assert_eq!(h.iter().flatten().sum::<u32>(), 3);
        assert_eq!(hard_hour(&h), Some(23));
        let mut once = vec![vec![0; 24]; 7];
        once[2][15] = 1;
        assert_eq!(hard_hour(&once), None, "至少 2 次");
        // 并列时取更晚的：14 点与凌晨 1 点各 2 次 → 1 点
        let mut tie = vec![vec![0; 24]; 7];
        tie[0][14] = 2;
        tie[1][1] = 2;
        assert_eq!(hard_hour(&tie), Some(1));
    }

    #[test]
    fn state_counts_follow_fixed_order() {
        let got = state_counts(&[Tired, Fluent, Unknown, Tired]);
        assert_eq!(
            got,
            [
                StateCount {
                    state: Fluent,
                    windows: 1
                },
                StateCount {
                    state: Tired,
                    windows: 2
                },
            ]
        );
    }

    fn lines() -> WeeklyLines {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../hub_templates");
        WeeklyLines::load(&TemplateDirs::factory_only(dir)).unwrap()
    }

    #[test]
    fn weekly_line_follows_the_rules_in_order() {
        let l = lines();
        let late = LineFacts {
            hard_hour: Some(23),
            hard_slot: Some("周三深夜".into()),
            rest_rate: Some(80),
            fluent_share: Some(70),
        };
        let a = l.pick(&late, 0);
        let b = l.pick(&late, 1);
        assert!(
            [&a, &b].iter().any(|s| s.contains("晚上 11 点后")),
            "{a} / {b}"
        );
        assert!([&a, &b].iter().any(|s| s.contains("周三深夜")), "{a} / {b}");
        // 没有时段名时只用带 {hour} 的那句
        let no_slot = LineFacts {
            hard_slot: None,
            ..late.clone()
        };
        for seed in 0..4 {
            assert_eq!(l.pick(&no_slot, seed), "这周晚上 11 点后更容易累，早点休息吧。");
        }
        let rest = LineFacts {
            hard_hour: Some(15),
            rest_rate: Some(85),
            ..Default::default()
        };
        assert_eq!(l.pick(&rest, 0), "这周休息完成了 85%，照顾自己做得很好。");
        let low = LineFacts {
            rest_rate: Some(10),
            ..Default::default()
        };
        assert!(l.pick(&low, 0).contains("休息提醒常常被跳过"));
        let sunny = LineFacts {
            fluent_share: Some(65),
            ..Default::default()
        };
        assert!(l.pick(&sunny, 0).contains("晴天"));
        let plain = l.pick(&LineFacts::default(), 0);
        assert!(!plain.contains('{'), "{plain}");
    }

    #[test]
    fn fill_skips_lines_with_missing_values() {
        let vars = [("{slot}", None), ("{rate}", Some("50%".to_string()))];
        assert_eq!(fill("完成了 {rate}", &vars).as_deref(), Some("完成了 50%"));
        assert_eq!(fill("{slot}很累", &vars), None);
        assert_eq!(fill("没有变量", &vars).as_deref(), Some("没有变量"));
    }
}
