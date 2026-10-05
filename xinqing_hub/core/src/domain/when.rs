//! 从一句话里按 FR-SCH-04 的规则算出日期和时刻（06 FR-SCH-04“不信任模型给出的日期”），纯函数。
//!
//! 只认产品书列出的说法：今天 / 明天 / 后天 / 大后天、今晚 / 明早 / 明晚，`周X`、`这周X`、`本周X`、`下周X`、
//! `下下周X`（`星期`、`礼拜`同），`M月D号`、`M.D`、`M/D`、`M-D`、`D号`；时刻是 `[时段]h点[半|一刻|三刻|m分]`、
//! `h:mm`，以及“两点到四点”这样的区间。认不出的部分留空，由调用方沿用模型的结果。

use std::sync::LazyLock;

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, Weekday};
use regex::Regex;

/// 代码从原句里算出来的日期与时刻。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct When {
    pub date: Option<NaiveDate>,
    pub time: Option<NaiveTime>,
    pub end_time: Option<NaiveTime>,
    /// 只说了“晚上”“明早”“下午”这类时段而没有钟点（FR-SCH-04 时刻规则、补充规则 2）：时刻为空，按全天处理
    pub period_only: bool,
    /// 句中有截止类说法（“截止”“DDL”“之前交”“X点前…交”，补充规则 4、5）
    pub deadline: bool,
}

const NUM: &str = "[0-9]{1,2}|[零一二两三四五六七八九十]{1,3}";

static RELATIVE_DAY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new("大后天|后天|明天|明日|明早|明晚|今天|今日|今早|今晚").unwrap());
static WEEKDAY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new("(上上|上|这|本|下下|下)?(?:个)?(?:周|星期|礼拜)([一二三四五六日天])").unwrap()
});
static MONTH_DAY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        "({NUM})月({NUM})[日号]?|(?:^|[^0-9.点])(1[0-2]|0?[1-9])[./-](3[01]|[12][0-9]|0?[1-9])(?:[^0-9.%]|$)"
    ))
    .unwrap()
});
static DAY_ONLY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!("(?:^|[^月0-9.])({NUM})[号日]")).unwrap());
static CLOCK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        "(凌晨|早上|早晨|上午|中午|下午|傍晚|晚上|夜里|今晚|明晚|明早|今早)?\
         (?:({NUM})[点时](?:(半)|(一刻)|(三刻)|({NUM})分?)?|([0-9]{{1,2}})[:：]([0-5][0-9]))"
    ))
    .unwrap()
});
static RANGE: LazyLock<Regex> = LazyLock::new(|| Regex::new("^\\s*(?:到|至|-|~|—)\\s*").unwrap());
static PERIOD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new("凌晨|早上|早晨|上午|中午|下午|傍晚|晚上|夜里|今晚|明晚|明早|今早").unwrap()
});
static DEADLINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        "截止|DDL|ddl|[Dd]eadline|之前.{0,8}(?:交|提交|完成)|[号日天周一二三四五六七八九十点0-9]前.{0,8}(?:交|提交|完成)",
    )
    .unwrap()
});

/// 解析 `0`–`99` 的阿拉伯数字或中文数字（“十二”“两”“二十一”）。
fn number(s: &str) -> Option<u32> {
    if let Ok(n) = s.parse() {
        return Some(n);
    }
    let digit = |c: char| {
        "零一二三四五六七八九"
            .find(c)
            .map(|i| (i / 3) as u32)
            .or((c == '两').then_some(2))
    };
    let chars: Vec<char> = s.chars().collect();
    match chars.as_slice() {
        ['十'] => Some(10),
        [c] => digit(*c),
        ['十', d] => Some(10 + digit(*d)?),
        [t, '十'] => Some(digit(*t)? * 10),
        [t, '十', d] => Some(digit(*t)? * 10 + digit(*d)?),
        _ => None,
    }
}

fn weekday(c: &str) -> Weekday {
    match c {
        "一" => Weekday::Mon,
        "二" => Weekday::Tue,
        "三" => Weekday::Wed,
        "四" => Weekday::Thu,
        "五" => Weekday::Fri,
        "六" => Weekday::Sat,
        _ => Weekday::Sun,
    }
}

/// 本自然周（一周从周一开始）里的周 `w`。
fn in_week(today: NaiveDate, weeks_ahead: i64, w: Weekday) -> NaiveDate {
    let monday = today - Duration::days(today.weekday().num_days_from_monday() as i64);
    monday + Duration::days(weeks_ahead * 7 + w.num_days_from_monday() as i64)
}

/// 句中第一个时刻（含区间的结束时刻）。时段词修饰钟点：下午 / 晚上 h ≤ 11 → h + 12，中午 1–5 点 → 13–17 点。
fn clock(sentence: &str) -> Option<(NaiveTime, Option<NaiveTime>, Option<&str>)> {
    let to_time = |c: &regex::Captures<'_>, period: Option<&str>| -> Option<NaiveTime> {
        let (mut h, m) = if let Some(h) = c.get(2) {
            let m = if c.get(3).is_some() {
                30
            } else if c.get(4).is_some() {
                15
            } else if c.get(5).is_some() {
                45
            } else {
                c.get(6).map_or(Some(0), |m| number(m.as_str()))?
            };
            (number(h.as_str())?, m)
        } else {
            (
                c.get(7)?.as_str().parse().ok()?,
                c.get(8)?.as_str().parse().ok()?,
            )
        };
        match period {
            Some("下午" | "傍晚" | "晚上" | "夜里" | "今晚" | "明晚") if h <= 11 => {
                h += 12
            }
            Some("中午") if (1..=5).contains(&h) => h += 12,
            _ => {}
        }
        NaiveTime::from_hms_opt(h, m, 0)
    };
    let first = CLOCK.captures(sentence)?;
    let period = first.get(1).map(|p| p.as_str());
    let start = to_time(&first, period)?;
    let rest = &sentence[first.get(0)?.end()..];
    let end = RANGE.find(rest).and_then(|sep| {
        let c = CLOCK.captures(&rest[sep.end()..])?;
        (c.get(0)?.start() == 0).then_some(())?;
        // 结束时刻没写时段时沿用开始的时段（“下午两点到四点”）
        to_time(&c, c.get(1).map(|p| p.as_str()).or(period))
    });
    Some((start, end.filter(|e| *e > start), period))
}

fn date_of(sentence: &str, now: NaiveDateTime, time: Option<NaiveTime>) -> Option<NaiveDate> {
    let today = now.date();
    if let Some(m) = RELATIVE_DAY.find(sentence) {
        let days = match m.as_str() {
            "大后天" => 3,
            "后天" => 2,
            "明天" | "明日" | "明早" | "明晚" => 1,
            _ => 0,
        };
        return Some(today + Duration::days(days));
    }
    if let Some(c) = WEEKDAY.captures(sentence) {
        let w = weekday(&c[2]);
        return match c.get(1).map(|m| m.as_str()) {
            Some("上" | "上上") => None,
            Some("这" | "本") => Some(in_week(today, 0, w)),
            Some("下") => Some(in_week(today, 1, w)),
            Some("下下") => Some(in_week(today, 2, w)),
            _ => {
                // 无修饰：今天之后最近的周 X；今天就是周 X 且时刻未过取今天
                let ahead =
                    (w.num_days_from_monday() + 7 - today.weekday().num_days_from_monday()) % 7;
                let passed = time.is_some_and(|t| t <= now.time());
                let ahead = if ahead == 0 && passed { 7 } else { ahead };
                Some(today + Duration::days(ahead as i64))
            }
        };
    }
    if let Some(c) = MONTH_DAY.captures(sentence) {
        let (m, d) = match (c.get(1), c.get(2)) {
            (Some(m), Some(d)) => (number(m.as_str())?, number(d.as_str())?),
            _ => (
                c.get(3)?.as_str().parse().ok()?,
                c.get(4)?.as_str().parse().ok()?,
            ),
        };
        // 今年已经过去 → 明年同日（补充规则 1）
        let this_year = NaiveDate::from_ymd_opt(today.year(), m, d)?;
        return Some(if this_year < today {
            NaiveDate::from_ymd_opt(today.year() + 1, m, d)?
        } else {
            this_year
        });
    }
    if let Some(c) = DAY_ONLY.captures(sentence) {
        // 只说“18号”：本月这一天，已经过去就是下个月
        let d = number(&c[1])?;
        let this_month = NaiveDate::from_ymd_opt(today.year(), today.month(), d)?;
        if this_month >= today {
            return Some(this_month);
        }
        let (y, m) = if today.month() == 12 {
            (today.year() + 1, 1)
        } else {
            (today.year(), today.month() + 1)
        };
        return NaiveDate::from_ymd_opt(y, m, d);
    }
    None
}

/// 按 FR-SCH-04 解析一句话。`now` 是本地时间，用来判断“今天的周 X 时刻是否已过”。
/// 有时刻没有日期时不补日期，由调用方按校验第 4 条处理。
pub fn parse(sentence: &str, now: NaiveDateTime) -> When {
    let clock = clock(sentence);
    let time = clock.map(|(t, _, _)| t);
    When {
        date: date_of(sentence, now, time),
        time,
        end_time: clock.and_then(|(_, e, _)| e),
        period_only: clock.is_none() && PERIOD.is_match(sentence),
        deadline: DEADLINE.is_match(sentence),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, 3)
            .unwrap()
            .and_hms_opt(10, 0, 0)
            .unwrap()
    }

    fn d(m: u32, day: u32) -> Option<NaiveDate> {
        NaiveDate::from_ymd_opt(2026, m, day)
    }

    fn t(h: u32, m: u32) -> Option<NaiveTime> {
        NaiveTime::from_hms_opt(h, m, 0)
    }

    #[test]
    fn chinese_numbers() {
        assert_eq!(number("十二"), Some(12));
        assert_eq!(number("两"), Some(2));
        assert_eq!(number("十"), Some(10));
        assert_eq!(number("二十一"), Some(21));
        assert_eq!(number("08"), Some(8));
        assert_eq!(number("百"), None);
    }

    /// 06 FR-SCH-04 的验收表（今天 2026-10-03 周六）。
    #[test]
    fn fr_sch_04_acceptance_table() {
        let w = parse("好的，周五下午三点在实验楼开组会", now());
        assert_eq!((w.date, w.time), (d(10, 9), t(15, 0)));
        let w = parse("明晚八点前交数据库作业", now());
        assert_eq!((w.date, w.time, w.deadline), (d(10, 4), t(20, 0), true));
        let w = parse("下周二和室友去看电影", now());
        assert_eq!((w.date, w.time, w.period_only), (d(10, 6), None, false));
        assert_eq!(parse("这周五交报告", now()).date, d(10, 2));
        let w = parse("10月15号上午9点半面试", now());
        assert_eq!((w.date, w.time), (d(10, 15), t(9, 30)));
    }

    #[test]
    fn same_weekday_today_depends_on_whether_time_passed() {
        assert_eq!(parse("周六晚上七点和高中同学聚餐", now()).date, d(10, 3));
        assert_eq!(parse("周六上午九点去驾校练车", now()).date, d(10, 10));
        assert_eq!(parse("下下周三英语四级模拟考试", now()).date, d(10, 14));
        assert_eq!(parse("上周五我们开会", now()).date, None);
    }

    #[test]
    fn ranges_periods_and_rollover() {
        let w = parse("10.20下午两点到四点期中考试", now());
        assert_eq!(
            (w.date, w.time, w.end_time),
            (d(10, 20), t(14, 0), t(16, 0))
        );
        assert_eq!(
            parse("1月8号期末考试", now()).date,
            NaiveDate::from_ymd_opt(2027, 1, 8)
        );
        let w = parse("明早去体检，记得空腹", now());
        assert_eq!((w.date, w.time, w.period_only), (d(10, 4), None, true));
        assert_eq!(parse("明天下午四点一刻在星巴克见面", now()).time, t(16, 15));
        assert_eq!(parse("下周五下午三点三刻听讲座", now()).time, t(15, 45));
        assert_eq!(parse("后天中午12点跟导师吃饭", now()).time, t(12, 0));
        assert_eq!(parse("18号上午十点校医院体检", now()).date, d(10, 18));
        assert_eq!(parse("2号交表", now()).date, d(11, 2));
        assert_eq!(parse("绩点3.5真的很高", now()).date, None);
    }
}
