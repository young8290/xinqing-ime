//! 日程与待办的提醒时刻（06 FR-SCH-07、FR-SCH-14），纯函数。调度（到点、稍后再提醒、错过的提醒）在
//! [`crate::reminder`]。实现说明见 docs/adr/0032。

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime};
use serde::Deserialize;

use super::notify::{Notice, NoticeButton};
use crate::infra::store::{ScheduleRow, TodoRow};
use crate::infra::templates::{TemplateDirs, TemplateError, read_toml};

/// 有时刻的事件默认提前 10 分钟（秒）；截止类默认截止前 1 天 + 2 小时（FR-SCH-07）。
pub const DEFAULT_OFFSETS: [i64; 1] = [600];
pub const DEADLINE_OFFSETS: [i64; 2] = [86_400, 7_200];
/// 卡片和设置里可选的提前量（秒）：0 / 5 / 10 / 30 / 60 分钟、提前 1 天（FR-SCH-07）。
pub const OFFSET_CHOICES: [i64; 6] = [0, 300, 600, 1_800, 3_600, 86_400];
/// 设置 `sch.default_offsets` 的取值（分钟，与 [`OFFSET_CHOICES`] 一一对应）。
pub const OFFSET_MINUTES: &[&str] = &["0", "5", "10", "30", "60", "1440"];

/// 设置 `sch.default_offsets` 的值 → 提前量（秒）。认不出时用默认 10 分钟。
pub fn offsets_of_setting(minutes: &str) -> Vec<i64> {
    minutes
        .parse::<i64>()
        .ok()
        .filter(|m| OFFSET_MINUTES.contains(&minutes) && *m >= 0)
        .map_or_else(|| DEFAULT_OFFSETS.to_vec(), |m| vec![m * 60])
}
/// 内部键：提醒已经看到哪个时刻（Unix 毫秒），启动时据此补“错过的提醒”（ADR 0032）。
pub const CHECKED_KEY: &str = "sch.reminded_until";
/// 错过的提醒只补 12 小时内的（FR-SCH-07）。
pub const MISSED_WINDOW_H: i64 = 12;

const NINE: NaiveTime = NaiveTime::from_hms_opt(9, 0, 0).unwrap();
const EIGHT_PM: NaiveTime = NaiveTime::from_hms_opt(20, 0, 0).unwrap();

/// 提醒的种类，也是 `reminder:due.kind`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RemindKind {
    /// 有时刻的日程：开始前（`remind_offsets`）
    Schedule,
    /// 截止类：截止前（默认 1 天 + 2 小时）
    Deadline,
    /// 全天日程：当天 09:00
    AllDay,
    /// 有截止日期的待办：截止当天 09:00（系统通知）
    Todo,
    /// 有截止日期的待办：截止前一天 20:00（只在小组件提醒）
    TodoEve,
    /// 每天 09:00 汇总未完成待办（`todo.daily_digest`，默认关）
    TodoDigest,
}

impl RemindKind {
    pub fn as_str(self) -> &'static str {
        match self {
            RemindKind::Schedule => "schedule",
            RemindKind::Deadline => "deadline",
            RemindKind::AllDay => "all_day",
            RemindKind::Todo => "todo",
            RemindKind::TodoEve => "todo_eve",
            RemindKind::TodoDigest => "todo_digest",
        }
    }

    /// 是否同时弹系统通知（FR-SCH-07 方式；FR-SCH-14 前一天晚上那次只在小组件）。
    pub fn toast(self) -> bool {
        !matches!(self, RemindKind::TodoEve)
    }
}

/// 一次提醒：哪一条（日程或待办的行号，汇总为 0）、什么时候。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Due {
    pub at: NaiveDateTime,
    pub kind: RemindKind,
    pub ref_id: i64,
}

/// 新日程的默认提前量：截止类 1 天 + 2 小时；全天日程固定当天 09:00，不用提前量；其余用设置
/// `sch.default_offsets`（默认 10 分钟）。
pub fn default_offsets(is_deadline: bool, all_day: bool, timed: &[i64]) -> Vec<i64> {
    if is_deadline {
        DEADLINE_OFFSETS.to_vec()
    } else if all_day {
        Vec::new()
    } else {
        timed.to_vec()
    }
}

fn date(s: Option<&str>) -> Option<NaiveDate> {
    s.and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
}

/// 一条已添加日程的全部提醒。没有日期的没有提醒。
pub fn schedule_dues(r: &ScheduleRow) -> Vec<Due> {
    let Some(day) = date(r.date.as_deref()).filter(|_| r.status == "added") else {
        return Vec::new();
    };
    let start = r
        .time
        .as_deref()
        .and_then(|t| NaiveTime::parse_from_str(t, "%H:%M").ok());
    let Some(start) = start else {
        return vec![Due {
            at: day.and_time(NINE),
            kind: RemindKind::AllDay,
            ref_id: r.id,
        }];
    };
    let kind = if r.is_deadline {
        RemindKind::Deadline
    } else {
        RemindKind::Schedule
    };
    let mut out: Vec<Due> = r
        .remind_offsets
        .iter()
        .map(|o| Due {
            at: day.and_time(start) - Duration::seconds(*o),
            kind,
            ref_id: r.id,
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// 一件未完成待办的提醒：有截止日期时截止前一天 20:00（小组件）和截止当天 09:00（系统通知）。
pub fn todo_dues(t: &TodoRow) -> Vec<Due> {
    let Some(day) = date(t.due_date.as_deref()).filter(|_| t.status == "open") else {
        return Vec::new();
    };
    vec![
        Due {
            at: (day - Duration::days(1)).and_time(EIGHT_PM),
            kind: RemindKind::TodoEve,
            ref_id: t.id,
        },
        Due {
            at: day.and_time(NINE),
            kind: RemindKind::Todo,
            ref_id: t.id,
        },
    ]
}

/// 每天 09:00 的待办汇总时刻（`todo.daily_digest` 开着时）。
pub fn digest_due(day: NaiveDate) -> Due {
    Due {
        at: day.and_time(NINE),
        kind: RemindKind::TodoDigest,
        ref_id: 0,
    }
}

/// `(after, until]` 之间到期的提醒，按时间先后。
pub fn due_between(
    all: impl IntoIterator<Item = Due>,
    after: NaiveDateTime,
    until: NaiveDateTime,
) -> Vec<Due> {
    let mut out: Vec<Due> = all
        .into_iter()
        .filter(|d| d.at > after && d.at <= until)
        .collect();
    out.sort();
    out
}

/// 提醒上的操作（卡片按钮与系统通知按钮交回的参数，`reminder_action` 的取值）。
pub const ACTION_OK: &str = "ok";
pub const ACTION_SNOOZE_5: &str = "snooze_5";
pub const ACTION_SNOOZE_10: &str = "snooze_10";

/// 提醒里的时间写法：当天只写钟点，否则带上日期（“10月9日 15:00”）。
pub fn time_text(at: NaiveDateTime, today: NaiveDate) -> String {
    if at.date() == today {
        at.format("%H:%M").to_string()
    } else {
        format!("{}月{}日 {}", at.month(), at.day(), at.format("%H:%M"))
    }
}

/// 提醒的文案（`ui_copy.toml` 的 `[notify]`，系统通知与小组件卡片共用；不出现任何情绪状态词，DS-COPY-08）。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ReminderCopy {
    schedule_title: String,
    schedule_body: String,
    deadline_body: String,
    allday_body: String,
    location_suffix: String,
    missed_title: String,
    missed_body: String,
    todo_title: String,
    todo_body: String,
    todo_eve_title: String,
    digest_title: String,
    digest_body: String,
    btn_ok: String,
    btn_snooze: String,
    btn_snooze10: String,
}

#[derive(Debug, Deserialize)]
struct RawCopy {
    version: u32,
    notify: ReminderCopy,
}

/// 一次提醒要显示的内容（来自那条日程或待办）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subject<'a> {
    pub title: &'a str,
    /// 开始或截止时刻的写法（[`time_text`]），全天与待办为空串
    pub time: &'a str,
    pub location: Option<&'a str>,
    /// 待办汇总的件数
    pub count: usize,
}

impl ReminderCopy {
    pub fn load(dirs: &TemplateDirs) -> Result<Self, TemplateError> {
        let banned = crate::domain::validate::BannedWords::load(dirs)?;
        dirs.load_with_fallback("ui_copy.toml", |path| {
            let raw: RawCopy = read_toml(path)?;
            let c = raw.notify;
            let fields = [
                (&c.schedule_title, &["title"][..]),
                (&c.schedule_body, &["time", "location_suffix"][..]),
                (&c.deadline_body, &["time"][..]),
                (&c.allday_body, &["location_suffix"][..]),
                (&c.location_suffix, &["location"][..]),
                (&c.missed_title, &[][..]), (&c.missed_body, &["n"][..]),
                (&c.todo_title, &[][..]), (&c.todo_body, &["title"][..]),
                (&c.todo_eve_title, &[][..]), (&c.digest_title, &[][..]),
                (&c.digest_body, &["n"][..]), (&c.btn_ok, &[][..]),
                (&c.btn_snooze, &[][..]), (&c.btn_snooze10, &[][..]),
            ];
            if raw.version == 0 || fields.iter().any(|(text, keys)| text.trim().is_empty()
                || banned.find(text, crate::domain::validate::Scene::Other).is_some()
                || !copy_placeholders_ok(text, keys)) {
                return Err(TemplateError::Invalid { file: "ui_copy.toml", reason: "提醒文案版本、正文、禁用词或占位符不合法" });
            }
            Ok(c)
        })
    }

    fn buttons(&self, snooze: bool) -> Vec<NoticeButton> {
        let mut b = vec![NoticeButton {
            label: self.btn_ok.clone(),
            action: ACTION_OK,
        }];
        if snooze {
            b.push(NoticeButton {
                label: self.btn_snooze.clone(),
                action: ACTION_SNOOZE_5,
            });
            b.push(NoticeButton {
                label: self.btn_snooze10.clone(),
                action: ACTION_SNOOZE_10,
            });
        }
        b
    }

    pub fn notice(&self, kind: RemindKind, s: &Subject<'_>) -> Notice {
        let suffix = s.location.map_or(String::new(), |l| {
            self.location_suffix.replace("{location}", l)
        });
        let (title, body) = match kind {
            RemindKind::Schedule => (
                self.schedule_title.replace("{title}", s.title),
                self.schedule_body
                    .replace("{time}", s.time)
                    .replace("{location_suffix}", &suffix),
            ),
            RemindKind::Deadline => (
                self.schedule_title.replace("{title}", s.title),
                self.deadline_body.replace("{time}", s.time),
            ),
            RemindKind::AllDay => (
                self.schedule_title.replace("{title}", s.title),
                self.allday_body.replace("{location_suffix}", &suffix),
            ),
            RemindKind::Todo => (
                self.todo_title.clone(),
                self.todo_body.replace("{title}", s.title),
            ),
            RemindKind::TodoEve => (
                self.todo_eve_title.clone(),
                self.todo_body.replace("{title}", s.title),
            ),
            RemindKind::TodoDigest => (
                self.digest_title.clone(),
                self.digest_body.replace("{n}", &s.count.to_string()),
            ),
        };
        Notice {
            title,
            body,
            buttons: self.buttons(!matches!(kind, RemindKind::TodoDigest)),
        }
    }

    /// 错过的提醒（FR-SCH-07）：Hub 启动时把 12 小时内错过的集中提示一次。
    pub fn missed(&self, n: usize) -> Notice {
        Notice {
            title: self.missed_title.clone(),
            body: self.missed_body.replace("{n}", &n.to_string()),
            buttons: self.buttons(false),
        }
    }
}

fn copy_placeholders_ok(text: &str, keys: &[&str]) -> bool {
    if keys.iter().any(|key| !text.contains(&format!("{{{key}}}"))) { return false; }
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        if rest[..open].contains('}') { return false; }
        let Some(close) = rest[open + 1..].find('}').map(|i| open + 1 + i) else { return false; };
        if !keys.contains(&&rest[open + 1..close]) { return false; }
        rest = &rest[close + 1..];
    }
    !rest.contains('}')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(d: u32, h: u32, m: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, d)
            .unwrap()
            .and_hms_opt(h, m, 0)
            .unwrap()
    }

    fn row(time: Option<&str>, deadline: bool, offsets: Vec<i64>) -> ScheduleRow {
        ScheduleRow {
            id: 3,
            title: Some("组会".into()),
            date: Some("2026-10-09".into()),
            time: time.map(Into::into),
            end_time: None,
            all_day: time.is_none(),
            location: None,
            is_deadline: deadline,
            remind_offsets: offsets,
            status: "added".into(),
            source: "ai".into(),
            flags: vec![],
            created_ts: 0,
        }
    }

    #[test]
    fn schedule_reminders_follow_fr_sch_07() {
        let timed = schedule_dues(&row(Some("15:00"), false, vec![600]));
        assert_eq!(
            timed.iter().map(|d| (d.at, d.kind)).collect::<Vec<_>>(),
            [(at(9, 14, 50), RemindKind::Schedule)]
        );
        let ddl = schedule_dues(&row(
            Some("23:59"),
            true,
            default_offsets(true, false, &DEFAULT_OFFSETS),
        ));
        assert_eq!(
            ddl.iter().map(|d| d.at).collect::<Vec<_>>(),
            [at(8, 23, 59), at(9, 21, 59)]
        );
        assert!(ddl.iter().all(|d| d.kind == RemindKind::Deadline));
        let all_day = schedule_dues(&row(
            None,
            false,
            default_offsets(false, true, &DEFAULT_OFFSETS),
        ));
        assert_eq!(
            all_day.iter().map(|d| (d.at, d.kind)).collect::<Vec<_>>(),
            [(at(9, 9, 0), RemindKind::AllDay)]
        );
        let mut pending = row(Some("15:00"), false, vec![600]);
        pending.status = "pending".into();
        assert!(schedule_dues(&pending).is_empty(), "只提醒已添加的");
        assert_eq!(
            schedule_dues(&row(Some("15:00"), false, vec![600, 600, 0])).len(),
            2
        );
    }

    #[test]
    fn offsets_setting() {
        assert_eq!(offsets_of_setting("30"), [1_800]);
        assert_eq!(offsets_of_setting("1440"), [86_400]);
        assert_eq!(offsets_of_setting("7"), DEFAULT_OFFSETS);
        for (m, s) in OFFSET_MINUTES.iter().zip(OFFSET_CHOICES) {
            assert_eq!(offsets_of_setting(m), [s]);
        }
    }

    #[test]
    fn todo_reminders_follow_fr_sch_14() {
        let t = TodoRow {
            id: 5,
            title: Some("交报告".into()),
            due_date: Some("2026-10-09".into()),
            status: "open".into(),
            source: "ai".into(),
            created_ts: 0,
            done_ts: None,
        };
        let d = todo_dues(&t);
        assert_eq!(
            d.iter().map(|d| (d.at, d.kind)).collect::<Vec<_>>(),
            [
                (at(8, 20, 0), RemindKind::TodoEve),
                (at(9, 9, 0), RemindKind::Todo)
            ]
        );
        assert!(!RemindKind::TodoEve.toast() && RemindKind::Todo.toast());
        assert!(
            todo_dues(&TodoRow {
                due_date: None,
                ..t.clone()
            })
            .is_empty()
        );
        assert!(
            todo_dues(&TodoRow {
                status: "done".into(),
                ..t
            })
            .is_empty()
        );
    }

    #[test]
    fn window_is_half_open() {
        let all = [
            Due {
                at: at(9, 9, 0),
                kind: RemindKind::AllDay,
                ref_id: 1,
            },
            Due {
                at: at(9, 8, 0),
                kind: RemindKind::Todo,
                ref_id: 2,
            },
        ];
        let got = due_between(all, at(9, 8, 0), at(9, 9, 0));
        assert_eq!(got.iter().map(|d| d.ref_id).collect::<Vec<_>>(), [1]);
        assert_eq!(digest_due(at(9, 0, 0).date()).at, at(9, 9, 0));
    }

    #[test]
    fn notices_use_notify_copy() {
        let dirs = TemplateDirs::factory_only(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../hub_templates"),
        );
        let copy = ReminderCopy::load(&dirs).unwrap();
        let s = Subject {
            title: "组会",
            time: "15:00",
            location: Some("实验楼"),
            count: 0,
        };
        let n = copy.notice(RemindKind::Schedule, &s);
        assert_eq!(
            (n.title.as_str(), n.body.as_str()),
            ("组会", "15:00 开始，实验楼")
        );
        assert_eq!(
            n.buttons.iter().map(|b| b.action).collect::<Vec<_>>(),
            [ACTION_OK, ACTION_SNOOZE_5, ACTION_SNOOZE_10]
        );
        let n = copy.notice(
            RemindKind::AllDay,
            &Subject {
                location: None,
                ..s.clone()
            },
        );
        assert_eq!(n.body, "今天");
        assert_eq!(copy.notice(RemindKind::Deadline, &s).body, "15:00 截止");
        assert_eq!(copy.notice(RemindKind::Todo, &s).body, "组会");
        let d = copy.notice(RemindKind::TodoDigest, &Subject { count: 3, ..s });
        assert_eq!(
            (d.body.as_str(), d.buttons.len()),
            ("有 3 件待办还没完成", 1)
        );
        assert_eq!(copy.missed(2).body, "有 2 个提醒在你离开时到期了");
        assert_eq!(time_text(at(9, 15, 0), at(9, 0, 0).date()), "15:00");
        assert_eq!(time_text(at(9, 15, 0), at(8, 0, 0).date()), "10月9日 15:00");
        let banned = crate::domain::validate::BannedWords::load(&dirs).unwrap();
        for kind in [
            RemindKind::Schedule,
            RemindKind::Deadline,
            RemindKind::AllDay,
            RemindKind::Todo,
            RemindKind::TodoEve,
            RemindKind::TodoDigest,
        ] {
            let n = copy.notice(
                kind,
                &Subject {
                    title: "",
                    time: "",
                    location: None,
                    count: 1,
                },
            );
            for t in [&n.title, &n.body] {
                assert_eq!(banned.find(t, crate::domain::validate::Scene::Other), None);
            }
        }
    }
}
