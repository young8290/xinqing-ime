//! 日程之间的比较与导出（06 FR-SCH-06 第 2 条、FR-SCH-08、FR-SCH-11），纯函数。实现说明见 docs/adr/0032。

use chrono::{DateTime, NaiveDate, NaiveTime, Utc};

use super::schedule::ScheduleDraft;
use super::validate;
use crate::infra::store::ScheduleRow;

/// 没有结束时间的事件按 60 分钟算（FR-SCH-11 第 1 条）。
pub const DEFAULT_EVENT_MIN: i64 = 60;
/// 相似标题：字符二元组 Jaccard ≥ 0.6，开始时刻相差 ≤ 60 分钟（FR-SCH-06 第 2 条）。
pub const SIMILAR_JACCARD: f64 = 0.6;
pub const SIMILAR_WITHIN_MIN: i64 = 60;

/// 要比较的那条日程的时间（从草稿或一行记录取）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slot {
    pub date: Option<NaiveDate>,
    pub start: Option<NaiveTime>,
    pub end: Option<NaiveTime>,
    pub all_day: bool,
    pub is_deadline: bool,
}

fn date(s: Option<&str>) -> Option<NaiveDate> {
    s.and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
}

fn time(s: Option<&str>) -> Option<NaiveTime> {
    s.and_then(|t| NaiveTime::parse_from_str(t, "%H:%M").ok())
}

impl Slot {
    pub fn of_draft(d: &ScheduleDraft) -> Self {
        Self {
            date: date(d.date.as_deref()),
            start: time(d.time.as_deref()),
            end: time(d.end_time.as_deref()),
            all_day: d.all_day,
            is_deadline: d.is_deadline,
        }
    }

    pub fn of_row(r: &ScheduleRow) -> Self {
        Self {
            date: date(r.date.as_deref()),
            start: time(r.time.as_deref()),
            end: time(r.end_time.as_deref()),
            all_day: r.all_day,
            is_deadline: r.is_deadline,
        }
    }

    /// 当天的分钟区间 `[start, end)`；没有开始时刻时为 `None`。结束不晚于开始或没有结束时按 60 分钟。
    fn minutes(&self) -> Option<(i64, i64)> {
        let start = self.start?;
        let s = start.signed_duration_since(NaiveTime::MIN).num_minutes();
        let e = self
            .end
            .filter(|e| *e > start)
            .map_or(s + DEFAULT_EVENT_MIN, |e| {
                e.signed_duration_since(NaiveTime::MIN).num_minutes()
            });
        Some((s, e))
    }
}

/// 与它时间重叠的一条已添加日程（卡片上“⚠ 与「班会」时间重叠（15:00–16:00）”）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub id: i64,
    pub title: String,
    /// 全天日程时为 `None`
    pub start: Option<String>,
    pub end: Option<String>,
}

/// FR-SCH-11：和同一天已添加的日程比时间区间。截止类不参与；全天只和全天比；首尾相接不算重叠。
/// `existing` 里与 `self_id` 相同的那条（修改自己时）跳过。
pub fn conflicts(slot: &Slot, existing: &[ScheduleRow], self_id: Option<i64>) -> Vec<Conflict> {
    if slot.is_deadline || slot.date.is_none() {
        return Vec::new();
    }
    existing
        .iter()
        .filter(|r| Some(r.id) != self_id && r.status == "added")
        .filter_map(|r| {
            let other = Slot::of_row(r);
            if other.is_deadline || other.date != slot.date {
                return None;
            }
            let overlap = match (slot.minutes(), other.minutes()) {
                (Some((s1, e1)), Some((s2, e2))) => s1 < e2 && s2 < e1,
                (None, None) => slot.all_day && other.all_day,
                _ => false,
            };
            overlap.then(|| Conflict {
                id: r.id,
                title: r.title.clone().unwrap_or_default(),
                start: other.start.map(|t| t.format("%H:%M").to_string()),
                end: other
                    .minutes()
                    .map(|(_, e)| format!("{:02}:{:02}", (e / 60) % 24, e % 60)),
            })
        })
        .collect()
}

/// FR-SCH-06 第 2 条：同一天、开始时刻相差 ≤ 1 小时（都没有时刻也算）、标题相似的已添加日程。
pub fn maybe_duplicate(
    draft: &ScheduleDraft,
    existing: &[ScheduleRow],
    self_id: Option<i64>,
) -> bool {
    let slot = Slot::of_draft(draft);
    existing
        .iter()
        .filter(|r| Some(r.id) != self_id && r.status == "added")
        .any(|r| {
            let other = Slot::of_row(r);
            let near = match (slot.minutes(), other.minutes()) {
                (Some((a, _)), Some((b, _))) => (a - b).abs() <= SIMILAR_WITHIN_MIN,
                (None, None) => true,
                _ => false,
            };
            other.date == slot.date
                && near
                && validate::jaccard(&draft.title, r.title.as_deref().unwrap_or_default())
                    >= SIMILAR_JACCARD
        })
}

/// RFC 5545 的文字转义（`\`、`;`、`,`、换行）。
fn escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .replace('\n', "\\n")
}

/// 每行不超过 75 个字节，超出的折到下一行并以空格开头（RFC 5545 3.1），不切开多字节字符。
fn fold(line: &str) -> String {
    let mut out = String::new();
    let mut len = 0;
    for c in line.chars() {
        let n = c.len_utf8();
        if len + n > 75 {
            out.push_str("\r\n ");
            len = 1;
        }
        out.push(c);
        len += n;
    }
    out.push_str("\r\n");
    out
}

/// 导出到系统日历（FR-SCH-08）：RFC 5545 的 `.ics` 文本，时区 Asia/Shanghai（固定 +08:00，无夏令时）。
/// 没有日期的日程跳过。`description` 写明“由心晴识别添加”之类的来源说明。
pub fn ics(rows: &[ScheduleRow], description: &str, now: DateTime<Utc>) -> String {
    let mut lines: Vec<String> = vec![
        "BEGIN:VCALENDAR".into(),
        "VERSION:2.0".into(),
        "PRODID:-//XinQing//Schedule//ZH".into(),
        "CALSCALE:GREGORIAN".into(),
        "BEGIN:VTIMEZONE".into(),
        "TZID:Asia/Shanghai".into(),
        "BEGIN:STANDARD".into(),
        "DTSTART:19700101T000000".into(),
        "TZOFFSETFROM:+0800".into(),
        "TZOFFSETTO:+0800".into(),
        "TZNAME:CST".into(),
        "END:STANDARD".into(),
        "END:VTIMEZONE".into(),
    ];
    let stamp = now.format("%Y%m%dT%H%M%SZ").to_string();
    for r in rows {
        let slot = Slot::of_row(r);
        let Some(day) = slot.date else { continue };
        lines.push("BEGIN:VEVENT".into());
        lines.push(format!(
            "UID:xinqing-schedule-{}-{}@xinqing",
            r.id, r.created_ts
        ));
        lines.push(format!("DTSTAMP:{stamp}"));
        match slot.start {
            Some(start) => {
                let (_, end) = slot.minutes().unwrap_or_default();
                let fmt = |t: chrono::NaiveDateTime| t.format("%Y%m%dT%H%M%S").to_string();
                let begin = day.and_time(start);
                let finish = day.and_time(NaiveTime::MIN) + chrono::Duration::minutes(end);
                lines.push(format!("DTSTART;TZID=Asia/Shanghai:{}", fmt(begin)));
                if !slot.is_deadline {
                    lines.push(format!("DTEND;TZID=Asia/Shanghai:{}", fmt(finish)));
                }
            }
            None => {
                lines.push(format!("DTSTART;VALUE=DATE:{}", day.format("%Y%m%d")));
                lines.push(format!(
                    "DTEND;VALUE=DATE:{}",
                    (day + chrono::Duration::days(1)).format("%Y%m%d")
                ));
            }
        }
        lines.push(format!(
            "SUMMARY:{}",
            escape(r.title.as_deref().unwrap_or_default())
        ));
        if let Some(loc) = &r.location {
            lines.push(format!("LOCATION:{}", escape(loc)));
        }
        lines.push(format!("DESCRIPTION:{}", escape(description)));
        for offset in &r.remind_offsets {
            lines.push("BEGIN:VALARM".into());
            lines.push("ACTION:DISPLAY".into());
            lines.push(format!(
                "DESCRIPTION:{}",
                escape(r.title.as_deref().unwrap_or_default())
            ));
            lines.push(format!("TRIGGER:-PT{}M", offset / 60));
            lines.push("END:VALARM".into());
        }
        lines.push("END:VEVENT".into());
    }
    lines.push("END:VCALENDAR".into());
    lines.iter().map(|l| fold(l)).collect()
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn row(id: i64, title: &str, time: Option<&str>, end: Option<&str>) -> ScheduleRow {
        ScheduleRow {
            id,
            title: Some(title.into()),
            date: Some("2026-10-09".into()),
            time: time.map(Into::into),
            end_time: end.map(Into::into),
            all_day: time.is_none(),
            location: None,
            is_deadline: false,
            remind_offsets: vec![600],
            status: "added".into(),
            source: "ai".into(),
            flags: vec![],
            created_ts: 1,
        }
    }

    fn draft(title: &str, time: Option<&str>) -> ScheduleDraft {
        ScheduleDraft {
            title: title.into(),
            date: Some("2026-10-09".into()),
            time: time.map(Into::into),
            end_time: None,
            all_day: time.is_none(),
            location: None,
            is_deadline: false,
            remind_offsets: vec![600],
            source: "ai".into(),
            flags: vec![],
        }
    }

    #[test]
    fn conflicts_follow_fr_sch_11() {
        let existing = [
            row(1, "班会", Some("15:00"), Some("16:00")),
            row(2, "运动会", None, None),
        ];
        // 验收标准：15:30 冲突，16:00 不算
        let c = conflicts(
            &Slot::of_draft(&draft("组会", Some("15:30"))),
            &existing,
            None,
        );
        assert_eq!(
            c,
            [Conflict {
                id: 1,
                title: "班会".into(),
                start: Some("15:00".into()),
                end: Some("16:00".into())
            }]
        );
        assert!(
            conflicts(
                &Slot::of_draft(&draft("组会", Some("16:00"))),
                &existing,
                None
            )
            .is_empty()
        );
        // 没有结束时间按 60 分钟：14:30 开始到 15:30，与 15:00 重叠
        assert_eq!(
            conflicts(
                &Slot::of_draft(&draft("组会", Some("14:30"))),
                &existing,
                None
            )
            .len(),
            1
        );
        // 全天只和全天比
        let all_day = conflicts(&Slot::of_draft(&draft("秋游", None)), &existing, None);
        assert_eq!(all_day.iter().map(|c| c.id).collect::<Vec<_>>(), [2]);
        assert_eq!(all_day[0].start, None);
        // 截止类不参与；改自己时不和自己比
        let mut ddl = draft("交报告", Some("15:30"));
        ddl.is_deadline = true;
        assert!(conflicts(&Slot::of_draft(&ddl), &existing, None).is_empty());
        assert!(
            conflicts(
                &Slot::of_draft(&draft("班会", Some("15:00"))),
                &existing,
                Some(1)
            )
            .is_empty()
        );
    }

    #[test]
    fn similar_titles_within_an_hour() {
        let existing = [row(1, "实验室组会", Some("15:00"), None)];
        assert!(maybe_duplicate(
            &draft("实验室组会议", Some("15:30")),
            &existing,
            None
        ));
        assert!(!maybe_duplicate(
            &draft("实验室组会议", Some("17:00")),
            &existing,
            None
        ));
        assert!(!maybe_duplicate(
            &draft("看电影", Some("15:00")),
            &existing,
            None
        ));
    }

    #[test]
    fn ics_is_rfc5545() {
        let mut timed = row(7, "组会, 周报", Some("15:00"), None);
        timed.location = Some("实验楼".into());
        let all_day = row(8, "运动会", None, None);
        let mut no_date = row(9, "没日期", None, None);
        no_date.date = None;
        let now = Utc.with_ymd_and_hms(2026, 10, 6, 1, 2, 3).unwrap();
        let text = ics(&[timed, all_day, no_date], "由心晴识别添加", now);
        assert!(text.starts_with("BEGIN:VCALENDAR\r\n"));
        assert!(text.ends_with("END:VCALENDAR\r\n"));
        assert!(text.contains("DTSTART;TZID=Asia/Shanghai:20261009T150000\r\n"));
        assert!(text.contains("DTEND;TZID=Asia/Shanghai:20261009T160000\r\n"));
        assert!(text.contains("SUMMARY:组会\\, 周报\r\n"));
        assert!(text.contains("LOCATION:实验楼\r\n"));
        assert!(text.contains("DTSTART;VALUE=DATE:20261009\r\nDTEND;VALUE=DATE:20261010\r\n"));
        assert!(text.contains("TRIGGER:-PT10M\r\n"));
        assert!(text.contains("DTSTAMP:20261006T010203Z\r\n"));
        assert_eq!(text.matches("BEGIN:VEVENT").count(), 2, "没有日期的跳过");
        assert!(text.lines().all(|l| l.len() <= 75));
        assert_eq!(fold(&"字".repeat(40)).matches("\r\n ").count(), 1);
    }
}
