//! 晴晴的周信（C-10 第二部分，05 FR-REV-02、08 P-LETTER 与 V9）：哪一周该写、一周的统计、校验与兜底模板。
//! 全是纯函数；读库、调网关、写 `letter` 表在 [`crate::letter`]。
//!
//! 出网的只有统计 JSON（[`WeeklyStats`]），没有任何原文、对话、日记（FR-REV-02 验收标准）。规格没写清的地方写在 docs/adr/0030。

use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, TimeZone, Timelike, Weekday};
use serde::{Deserialize, Serialize};
use xqp::MoodState;

use super::comfort::{ComfortPrompt, Style};
use super::validate::{BannedWords, Scene};
use crate::infra::templates::{TemplateDirs, TemplateError, read_toml};

/// 一周至少这么多天活跃输入 ≥ 30 分钟才写（FR-REV-02）。
pub const MIN_VALID_DAYS: u32 = 3;
/// 活跃输入至少这么多分钟才算有效的一天。
pub const VALID_DAY_MIN: u32 = 30;
/// 周日这个钟点写信。
pub const SEND_HOUR: u32 = 20;
/// 大模型写的信去掉空白后的字数范围（FR-REV-02：150–250 字；署名、标点留一点余量）。
pub const MIN_CHARS: usize = 150;
pub const MAX_CHARS: usize = 300;

/// 内部键：上一次处理过（写了，或因有效天数不足跳过）的那周的周一（`YYYY-MM-DD`）。
/// 用它而不是查 `letter` 表判断“写过没有”：用户删掉的信不会被重写（ADR 0030 第 2 条）。
pub const LAST_WEEK_KEY: &str = "review.letter.last_week";

/// `date` 所在一周的周一。
pub fn week_start(date: NaiveDate) -> NaiveDate {
    date - Duration::days(i64::from(date.weekday().num_days_from_monday()))
}

/// 某周的写信时刻：那周周日 20:00。
pub fn send_time(week: NaiveDate) -> Option<DateTime<Local>> {
    let sunday = week + Duration::days(6);
    Local
        .from_local_datetime(&sunday.and_hms_opt(SEND_HOUR, 0, 0)?)
        .earliest()
}

/// 现在该写哪一周的信：最近一个已经到了写信时刻的那周（ADR 0030 第 2 条：错过了就在下一周的写信时刻之前补写）。
/// 那周已经写过（`written`）时返回 `None`；有效天数由调用方再查。
pub fn due_week(now: DateTime<Local>, written: impl Fn(NaiveDate) -> bool) -> Option<NaiveDate> {
    let this = week_start(now.date_naive());
    let week = match send_time(this) {
        Some(t) if now >= t => this,
        _ => this - Duration::days(7),
    };
    (!written(week)).then_some(week)
}

/// 一天的计数（`daily_summary`）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DayCounts {
    pub typing_min: u32,
    pub rests_due: u32,
    pub rests_done: u32,
    pub water: u32,
}

/// 一周的原始统计（外壳从库里读出来）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WeekFacts {
    /// 周一到周日，没有记录的天是全 0
    pub days: Vec<DayCounts>,
    /// `(Unix 毫秒, 显示状态)`，从旧到新
    pub states: Vec<(i64, MoodState)>,
    pub schedules_done: u32,
    pub todos_done: u32,
    pub self_reports: u32,
    /// 这周各晚的平均停止打字时间（作息洞察，分钟数，18:00 = 1080）与熬夜天数
    pub avg_stop_min: Option<u32>,
    pub late_nights: u32,
}

impl WeekFacts {
    pub fn valid_days(&self) -> u32 {
        self.days
            .iter()
            .filter(|d| d.typing_min >= VALID_DAY_MIN)
            .count() as u32
    }
}

/// 发给大模型、也用来填兜底模板的统计（只有统计，FR-REV-02）。字段名就是 JSON 的键。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WeeklyStats {
    pub valid_days: u32,
    /// 有效天的日均打字分钟数，与它的中文写法
    pub typing_avg_min: u32,
    pub typing_avg: String,
    /// 各状态占比（百分比，四舍五入）
    pub state_share: BTreeMap<&'static str, u32>,
    /// 最常出现低落或疲劳的时段（如“周三晚上”），最轻松的时段
    pub hard_slot: Option<String>,
    pub best_slot: Option<String>,
    /// 休息完成率（百分比）与次数
    pub rest_rate: Option<u32>,
    pub rests_done: u32,
    pub rests_due: u32,
    pub water: u32,
    pub schedules_done: u32,
    pub todos_done: u32,
    /// 平均停止打字时间（“23:10”）与熬夜天数
    pub stop_avg: Option<String>,
    pub late_nights: u32,
    pub self_reports: u32,
}

/// 时段（04 的五段划分）：05–11 上午、11–14 中午、14–18 下午、18–23 晚上、23–05 深夜。深夜算前一天。
fn slot_of(ts: i64) -> Option<(Weekday, &'static str)> {
    let t = Local.timestamp_millis_opt(ts).single()?;
    let h = t.hour();
    let (day, part) = match h {
        5..=10 => (t, "上午"),
        11..=13 => (t, "中午"),
        14..=17 => (t, "下午"),
        18..=22 => (t, "晚上"),
        23 => (t, "深夜"),
        _ => (t - Duration::days(1), "深夜"),
    };
    Some((day.weekday(), part))
}

fn weekday_cn(w: Weekday) -> &'static str {
    match w {
        Weekday::Mon => "周一",
        Weekday::Tue => "周二",
        Weekday::Wed => "周三",
        Weekday::Thu => "周四",
        Weekday::Fri => "周五",
        Weekday::Sat => "周六",
        Weekday::Sun => "周日",
    }
}

/// 某些状态出现最多（至少 2 次）的时段。次数相同时取最早的。
fn top_slot(states: &[(i64, MoodState)], want: &[MoodState]) -> Option<String> {
    let mut count: HashMap<(Weekday, &'static str), (usize, i64)> = HashMap::new();
    for (ts, s) in states {
        if want.contains(s)
            && let Some(slot) = slot_of(*ts)
        {
            let e = count.entry(slot).or_insert((0, *ts));
            e.0 += 1;
        }
    }
    count
        .into_iter()
        .filter(|(_, (n, _))| *n >= 2)
        .max_by_key(|(_, (n, first))| (*n, -first))
        .map(|((w, part), _)| format!("{}{}", weekday_cn(w), part))
}

/// “3 小时 20 分”“45 分钟”。
pub fn duration_cn(min: u32) -> String {
    match (min / 60, min % 60) {
        (0, m) => format!("{m} 分钟"),
        (h, 0) => format!("{h} 小时"),
        (h, m) => format!("{h} 小时 {m} 分"),
    }
}

/// 分钟数（18:00 = 1080，可过 24 点）写成时刻。
fn clock_cn(stop_min: u32) -> String {
    let m = stop_min % (24 * 60);
    format!("{:02}:{:02}", m / 60, m % 60)
}

pub fn stats(f: &WeekFacts) -> WeeklyStats {
    let valid: Vec<&DayCounts> = f
        .days
        .iter()
        .filter(|d| d.typing_min >= VALID_DAY_MIN)
        .collect();
    let typing_avg_min = if valid.is_empty() {
        0
    } else {
        (valid.iter().map(|d| d.typing_min).sum::<u32>() as f64 / valid.len() as f64).round() as u32
    };
    let known: Vec<MoodState> = f
        .states
        .iter()
        .map(|(_, s)| *s)
        .filter(|s| *s != MoodState::Unknown)
        .collect();
    let mut state_share = BTreeMap::new();
    if !known.is_empty() {
        for s in [
            MoodState::Fluent,
            MoodState::Hesitant,
            MoodState::Low,
            MoodState::Agitated,
            MoodState::Tired,
        ] {
            let n = known.iter().filter(|k| **k == s).count();
            if n > 0 {
                state_share.insert(
                    s.as_str(),
                    ((n * 100) as f64 / known.len() as f64).round() as u32,
                );
            }
        }
    }
    let rests_due: u32 = f.days.iter().map(|d| d.rests_due).sum();
    let rests_done: u32 = f.days.iter().map(|d| d.rests_done).sum();
    WeeklyStats {
        valid_days: valid.len() as u32,
        typing_avg_min,
        typing_avg: duration_cn(typing_avg_min),
        state_share,
        hard_slot: top_slot(&f.states, &[MoodState::Low, MoodState::Tired]),
        best_slot: top_slot(&f.states, &[MoodState::Fluent]),
        rest_rate: (rests_due > 0)
            .then(|| ((rests_done.min(rests_due) * 100) as f64 / rests_due as f64).round() as u32),
        rests_done,
        rests_due,
        water: f.days.iter().map(|d| d.water).sum(),
        schedules_done: f.schedules_done,
        todos_done: f.todos_done,
        stop_avg: f.avg_stop_min.map(clock_cn),
        late_nights: f.late_nights,
        self_reports: f.self_reports,
    }
}

/// V9：信里出现的每个数字都要能在统计 JSON 里找到（08 第 5 节）。中文数字不检查。
pub fn facts_ok(letter: &str, stats_json: &str) -> bool {
    let known: Vec<&str> = digit_runs(stats_json).collect();
    digit_runs(letter).all(|d| known.contains(&d))
}

fn digit_runs(s: &str) -> impl Iterator<Item = &str> {
    s.split(|c: char| !c.is_ascii_digit())
        .filter(|t| !t.is_empty())
}

/// 大模型写的信为什么没通过（不含文字）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reject {
    Length,
    Banned,
    Facts,
}

/// 校验大模型写的信：长度、V4 禁用词、V9 数字（08 第 5 节）。
pub fn check(letter: &str, stats_json: &str, banned: &BannedWords) -> Result<(), Reject> {
    let n = letter.chars().filter(|c| !c.is_whitespace()).count();
    if !(MIN_CHARS..=MAX_CHARS).contains(&n) {
        return Err(Reject::Length);
    }
    if banned.find(letter, Scene::Other).is_some() {
        return Err(Reject::Banned);
    }
    if !facts_ok(letter, stats_json) {
        return Err(Reject::Facts);
    }
    Ok(())
}

/// P-LETTER（`prompts/letter.md`）。
#[derive(Debug, Clone)]
pub struct LetterPrompt {
    pub version: u32,
    body: String,
}

impl LetterPrompt {
    pub fn load(dirs: &TemplateDirs) -> Result<Self, TemplateError> {
        const FILE: &str = "prompts/letter.md";
        dirs.load_with_fallback(FILE, |path| {
            let text = std::fs::read_to_string(path).map_err(|source| TemplateError::Io {
                path: path.to_path_buf(),
                source,
            })?;
            let version = text
                .lines()
                .next()
                .and_then(|l| l.trim().strip_prefix("<!-- version:"))
                .and_then(|l| l.strip_suffix("-->"))
                .and_then(|l| l.trim().parse::<u32>().ok());
            let p = ComfortPrompt::parse(&text);
            if version.is_none_or(|v| v == 0)
                || p.body().trim().is_empty()
                || ["{style_block}", "{weekly_stats_json}"]
                    .iter()
                    .any(|key| !p.body().contains(key))
            {
                return Err(TemplateError::Invalid {
                    file: FILE,
                    reason: "缺少正整数版本号、正文或必需占位符",
                });
            }
            Ok(Self {
                version: p.version,
                body: p.body().to_string(),
            })
        })
    }

    pub fn ver(&self) -> String {
        format!("P-LETTER v{}", self.version)
    }

    pub fn render(&self, style: Style, stats_json: &str) -> String {
        let block = style.style_block();
        let body = if block.is_empty() {
            self.body.replace("{style_block}\n", "")
        } else {
            self.body.replace("{style_block}", block)
        };
        body.replace("{weekly_stats_json}", stats_json)
    }
}

#[derive(Debug, Clone, Deserialize)]
struct RestComment {
    high: String,
    mid: String,
    low: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Tip {
    when: String,
    text: String,
}

#[derive(Debug, Clone, Deserialize)]
struct RawTips {
    version: u32,
    rest_comment: RestComment,
    tip: Vec<Tip>,
}

/// 兜底信件（`letter_fallback.md`）与配套句子（`letter_tips.toml`）。
#[derive(Debug, Clone)]
pub struct LetterFallback {
    body: String,
    tips: RawTips,
}

impl LetterFallback {
    pub fn load(dirs: &TemplateDirs) -> Result<Self, TemplateError> {
        let banned = BannedWords::load(dirs)?;
        let body = dirs.load_with_fallback("letter_fallback.md", |path| {
        let text = std::fs::read_to_string(path).map_err(|source| TemplateError::Io { path: path.to_path_buf(), source })?;
        let version = text.lines().next().and_then(|l| l.trim().strip_prefix("<!-- version:"))
            .and_then(|l| l.strip_suffix("-->"))
            .and_then(|l| l.trim().parse::<u32>().ok());
        let body = text
            .lines()
            .filter(|l| !l.trim_start().starts_with("<!--"))
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string();
        if version.is_none_or(|v| v == 0) || body.trim().is_empty() || banned.find(&body, Scene::Other).is_some() || !fallback_syntax_ok(&body) {
            return Err(TemplateError::Invalid { file: "letter_fallback.md", reason: "版本、正文、禁用词或占位符语法不合法" });
        }
        Ok(body)
        })?;
        const FILE: &str = "letter_tips.toml";
        let tips = dirs.load_with_fallback(FILE, |path| {
            let tips: RawTips = read_toml(path)?;
            let groups = ["late", "rest_low", "hard", "default"];
            if tips.version == 0
                || tips.tip.len() != groups.len()
                || groups
                    .iter()
                    .any(|group| tips.tip.iter().filter(|t| t.when == *group).count() != 1)
            {
                return Err(TemplateError::Invalid {
                    file: FILE,
                    reason: "版本号须为正整数，建议须包含四个不重复的分组",
                });
            }
            let copy = [
                &tips.rest_comment.high,
                &tips.rest_comment.mid,
                &tips.rest_comment.low,
            ]
            .into_iter()
            .chain(tips.tip.iter().map(|t| &t.text));
            if copy
                .into_iter()
                .any(|s| s.trim().is_empty() || banned.find(s, Scene::Other).is_some())
            {
                return Err(TemplateError::Invalid {
                    file: FILE,
                    reason: "评语或建议为空或包含禁用词",
                });
            }
            Ok(tips)
        })?;
        Ok(Self { body, tips })
    }

    /// “下周可以试试”的一条：按 `late` → `rest_low` → `hard` → `default` 的顺序取第一条符合的。
    fn tip(&self, s: &WeeklyStats) -> &str {
        let late = s.late_nights >= 2
            || s.stop_avg
                .as_deref()
                .is_some_and(|t| !("05:00".."23:30").contains(&t));
        let wants = [
            ("late", late),
            ("rest_low", s.rest_rate.is_some_and(|r| r < 30)),
            ("hard", s.hard_slot.is_some()),
            ("default", true),
        ];
        wants
            .iter()
            .filter(|(_, ok)| *ok)
            .find_map(|(w, _)| self.tips.tip.iter().find(|t| t.when == *w))
            .map_or("每天给自己留十分钟什么都不做的时间。", |t| t.text.as_str())
    }

    fn rest_comment(&self, rate: u32) -> &str {
        let c = &self.tips.rest_comment;
        match rate {
            70.. => &c.high,
            30..=69 => &c.mid,
            _ => &c.low,
        }
    }

    /// 按统计填模板：`{?key}…{/key}` 包着的句子在那项没有数据时整句去掉。
    pub fn render(&self, s: &WeeklyStats) -> String {
        let mut vars: Vec<(&str, Option<String>)> = vec![
            ("typing_avg", Some(s.typing_avg.clone())),
            ("rest_rate", s.rest_rate.map(|r| format!("{r}%"))),
            (
                "rest_comment",
                s.rest_rate.map(|r| self.rest_comment(r).to_string()),
            ),
            ("water", (s.water > 0).then(|| s.water.to_string())),
            ("best_slot", s.best_slot.clone()),
            ("hard_slot", s.hard_slot.clone()),
            ("stop_avg", s.stop_avg.clone()),
            (
                "done",
                (s.schedules_done + s.todos_done > 0).then(String::new),
            ),
            ("schedules_done", Some(s.schedules_done.to_string())),
            ("todos_done", Some(s.todos_done.to_string())),
            ("tip", Some(self.tip(s).to_string())),
        ];
        let mut out = self.body.clone();
        // 先处理条件块，再替换变量
        for (k, v) in &vars {
            let (open, close) = (format!("{{?{k}}}"), format!("{{/{k}}}"));
            while let Some(a) = out.find(&open) {
                let Some(b) = out[a..].find(&close).map(|i| a + i) else {
                    break;
                };
                if v.is_some() {
                    out.replace_range(b..b + close.len(), "");
                    out.replace_range(a..a + open.len(), "");
                } else {
                    out.replace_range(a..b + close.len(), "");
                }
            }
        }
        for (k, v) in vars.drain(..) {
            out = out.replace(&format!("{{{k}}}"), v.as_deref().unwrap_or(""));
        }
        // 去掉整段被删空后留下的多余空行
        let mut lines: Vec<&str> = Vec::new();
        for l in out.lines() {
            if l.trim().is_empty() && lines.last().is_some_and(|p: &&str| p.trim().is_empty()) {
                continue;
            }
            lines.push(l);
        }
        lines.join("\n").trim().to_string()
    }

    /// 兜底信件里可能出现的全部固定句子（校验禁用词用）。
    pub fn all_copy(&self) -> Vec<String> {
        let mut v = vec![
            self.tips.rest_comment.high.clone(),
            self.tips.rest_comment.mid.clone(),
            self.tips.rest_comment.low.clone(),
            self.body.clone(),
        ];
        v.extend(self.tips.tip.iter().map(|t| t.text.clone()));
        v
    }
}

/// 只支持已知统计占位符与非嵌套条件块；可空变量必须在对应条件块中。
fn fallback_syntax_ok(body: &str) -> bool {
    const KEYS: &[&str] = &["typing_avg", "rest_rate", "rest_comment", "water", "best_slot", "hard_slot", "stop_avg", "done", "schedules_done", "todos_done", "tip"];
    const CONDITIONS: &[&str] = &["rest_rate", "water", "best_slot", "hard_slot", "stop_avg", "done"];
    let mut condition: Option<&str> = None;
    let mut rest = body;
    while let Some(open) = rest.find('{') {
        if rest[..open].contains('}') { return false; }
        let Some(close) = rest[open + 1..].find('}').map(|i| open + 1 + i) else { return false; };
        let token = &rest[open + 1..close];
        if let Some(key) = token.strip_prefix('?') {
            if condition.is_some() || !CONDITIONS.contains(&key) { return false; }
            condition = Some(key);
        } else if let Some(key) = token.strip_prefix('/') {
            if condition != Some(key) { return false; }
            condition = None;
        } else {
            if !KEYS.contains(&token) || token == "done" { return false; }
            let required = match token { "rest_comment" => Some("rest_rate"), "rest_rate" | "water" | "best_slot" | "hard_slot" | "stop_avg" => Some(token), _ => None };
            if required.is_some() && required != condition { return false; }
        }
        rest = &rest[close + 1..];
    }
    condition.is_none() && !rest.contains('}')
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn dirs() -> TemplateDirs {
        TemplateDirs::factory_only(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../hub_templates"),
        )
    }

    fn at(y: i32, mo: u32, d: u32, h: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(y, mo, d, h, 0, 0).unwrap()
    }

    fn date(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, d).unwrap()
    }

    #[test]
    fn which_week_is_due() {
        // 2026-10-05 是周一，10-11 是周日
        assert_eq!(week_start(date(11)), date(5));
        assert_eq!(
            due_week(at(2026, 10, 11, 19), |_| false),
            Some(date(5) - Duration::days(7))
        );
        assert_eq!(due_week(at(2026, 10, 11, 20), |_| false), Some(date(5)));
        // 错过了周日，下一周补写上一周的
        assert_eq!(due_week(at(2026, 10, 14, 9), |_| false), Some(date(5)));
        assert_eq!(due_week(at(2026, 10, 14, 9), |w| w == date(5)), None);
    }

    fn facts() -> WeekFacts {
        let base = at(2026, 10, 5, 0).timestamp_millis();
        let h = 3_600_000;
        let d = 24 * h;
        WeekFacts {
            days: vec![
                DayCounts {
                    typing_min: 200,
                    rests_due: 4,
                    rests_done: 3,
                    water: 2,
                },
                DayCounts {
                    typing_min: 20,
                    ..Default::default()
                },
                DayCounts {
                    typing_min: 180,
                    rests_due: 3,
                    rests_done: 2,
                    water: 1,
                },
                DayCounts {
                    typing_min: 220,
                    rests_due: 0,
                    rests_done: 0,
                    water: 0,
                },
                DayCounts::default(),
                DayCounts::default(),
                DayCounts::default(),
            ],
            states: vec![
                (base + 9 * h, MoodState::Fluent),
                (base + 10 * h, MoodState::Fluent),
                (base + 2 * d + 20 * h, MoodState::Tired),
                (base + 2 * d + 21 * h, MoodState::Tired),
                (base + 2 * d + 22 * h, MoodState::Low),
            ],
            schedules_done: 2,
            todos_done: 3,
            self_reports: 1,
            avg_stop_min: Some(23 * 60 + 10),
            late_nights: 0,
        }
    }

    #[test]
    fn weekly_stats_are_only_numbers_and_slots() {
        let s = stats(&facts());
        assert_eq!(s.valid_days, 3);
        assert_eq!(s.typing_avg_min, 200);
        assert_eq!(s.typing_avg, "3 小时 20 分");
        assert_eq!(s.state_share.get("fluent"), Some(&40));
        assert_eq!(s.state_share.get("tired"), Some(&40));
        assert_eq!(s.hard_slot.as_deref(), Some("周三晚上"));
        assert_eq!(s.best_slot.as_deref(), Some("周一上午"));
        assert_eq!(s.rest_rate, Some(71));
        assert_eq!((s.water, s.schedules_done, s.todos_done), (3, 2, 3));
        assert_eq!(s.stop_avg.as_deref(), Some("23:10"));
        assert_eq!(duration_cn(45), "45 分钟");
        assert_eq!(duration_cn(120), "2 小时");
    }

    #[test]
    fn v9_numbers_must_come_from_the_stats() {
        let json = serde_json::to_string(&stats(&facts())).unwrap();
        assert!(facts_ok(
            "这周平均每天打字 3 小时 20 分，休息完成了 71%。",
            &json
        ));
        assert!(!facts_ok("这周你打了 6 天字。", &json), "6 不在统计里");
        assert!(
            facts_ok("这周辛苦了，下周三试试早点收工。", &json),
            "没有数字"
        );
    }

    #[test]
    fn fallback_fills_numbers_and_drops_missing_parts() {
        let dirs = dirs();
        let fb = LetterFallback::load(&dirs).unwrap();
        let s = stats(&facts());
        let text = fb.render(&s);
        assert!(text.contains("3 小时 20 分"), "{text}");
        assert!(text.contains("71%"));
        assert!(text.contains("周三晚上好像更容易累一些"));
        assert!(text.contains("23:10"));
        assert!(text.contains("2 个日程、3 件待办"));
        assert!(text.ends_with("晴晴"));
        assert!(!text.contains('{') && !text.contains("<!--"), "{text}");
        let json = serde_json::to_string(&s).unwrap();
        assert!(facts_ok(&text, &json), "兜底信件的数字也都来自统计");

        let mut thin = facts();
        for d in &mut thin.days {
            d.rests_due = 0;
            d.rests_done = 0;
            d.water = 0;
        }
        thin.states.clear();
        thin.schedules_done = 0;
        thin.todos_done = 0;
        thin.avg_stop_min = None;
        let text = fb.render(&stats(&thin));
        assert!(
            !text.contains("休息提醒") && !text.contains("喝了"),
            "{text}"
        );
        assert!(!text.contains("日程") && !text.contains("停止打字"));
        assert!(!text.contains("\n\n\n"));
    }

    #[test]
    fn templates_pass_banned_words() {
        let dirs = dirs();
        let banned = BannedWords::load(&dirs).unwrap();
        for t in LetterFallback::load(&dirs).unwrap().all_copy() {
            assert_eq!(banned.find(&t, Scene::Other), None, "{t}");
        }
    }

    #[test]
    fn llm_letters_are_checked() {
        let dirs = dirs();
        let banned = BannedWords::load(&dirs).unwrap();
        let json = serde_json::to_string(&stats(&facts())).unwrap();
        let good = format!(
            "这一周你辛苦啦。平均每天打字 3 小时 20 分，休息提醒完成了 71%，{}下周可以试试晚上早一点合上电脑。\n晴晴",
            "照顾自己做得挺好。".repeat(14)
        );
        assert_eq!(check(&good, &json, &banned), Ok(()));
        assert_eq!(check("太短了", &json, &banned), Err(Reject::Length));
        let wrong = good.replace("71%", "95%");
        assert_eq!(check(&wrong, &json, &banned), Err(Reject::Facts));
        let preachy = good.replace("下周可以试试", "你必须");
        assert_eq!(check(&preachy, &json, &banned), Err(Reject::Banned));
    }

    #[test]
    fn prompt_gets_stats_and_style() {
        let p = LetterPrompt::load(&dirs()).unwrap();
        let s = p.render(Style::Gentle, "{\"valid_days\":3}");
        assert!(s.contains("本周统计：{\"valid_days\":3}"));
        assert!(!s.contains("{style_block}"));
        let s = p.render(Style::Brief, "{}");
        assert!(s.contains(Style::Brief.style_block()));
    }
}
