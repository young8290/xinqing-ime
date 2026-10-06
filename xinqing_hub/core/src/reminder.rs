//! 日程与待办的提醒调度（C-05、C-06，06 FR-SCH-07、FR-SCH-14；17 第 2.6 节的 `scheduler`）。
//!
//! 每 [`TICK`] 从库里算一次 `(上次看到, 现在]` 之间到期的提醒（[`crate::domain::reminder`]），交给外壳弹系统通知并推
//! `reminder:due`；“5 / 10 分钟后再提醒”记在内存里。看到哪儿了记在内部键 [`CHECKED_KEY`]：Hub 启动时把 12 小时内
//! 错过的集中提示一次（“错过的提醒”），更早的不补。与外壳之间只通过 [`ReminderPort`]（ADR 0007）。实现说明见 docs/adr/0032。

use std::collections::HashMap;
use std::ops::Deref;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Duration as ChronoDuration, Local, NaiveDateTime, TimeZone};
use tokio::sync::mpsc;
use tokio::time::MissedTickBehavior;

use crate::domain::notify::Notice;
use crate::domain::reminder::{
    self, ACTION_SNOOZE_5, ACTION_SNOOZE_10, Due, MISSED_WINDOW_H, RemindKind, ReminderCopy,
    Subject,
};
use crate::infra::clock::Clock;
use crate::infra::store::{Db, StoreError};

/// 判断间隔。演示模式的时钟 60 倍速，1 秒是演示时间的 1 分钟。
pub const TICK: Duration = Duration::from_secs(1);
pub use crate::domain::reminder::CHECKED_KEY;
/// 内部键最多隔这么久写一次（普通数据，排队写）。
const SAVE_EVERY_MS: i64 = 60_000;

/// 交给界面的一次提醒（`reminder:due`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reminder {
    /// 本次运行内的编号，`reminder_action` 用它
    pub id: u32,
    pub kind: RemindKind,
    /// 日程或待办的行号；汇总为 0
    pub ref_id: i64,
    pub notice: Notice,
}

/// `reminder_action` 交回的操作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReminderCmd {
    Act { id: u32, action: String },
}

/// 外壳提供给提醒服务的能力。
pub trait ReminderPort: Send + Sync {
    /// 只读连接。
    fn db(&self) -> Box<dyn Deref<Target = Db> + '_>;
    /// 内部键 [`CHECKED_KEY`] 的读写（写是普通数据，排队）。
    fn checked(&self) -> Option<i64>;
    fn set_checked(&self, ms: i64);
    /// 设置 `todo.daily_digest`（默认关）。
    fn digest_enabled(&self) -> bool;
    /// 到点：推 `reminder:due`；`toast` 时同时弹系统通知（FR-SCH-07 方式、FR-NTF-01）。
    fn remind(&self, r: &Reminder, toast: bool);
    /// 启动时发现的错过的提醒（只提示一次）。
    fn missed(&self, notice: &Notice, items: &[Reminder]);
    /// 运行日志，只会传入不含用户数据的内容（NFR-LOG）。
    fn note(&self, _msg: &str) {}
}

pub struct ReminderService {
    port: Arc<dyn ReminderPort>,
    clock: Arc<dyn Clock>,
    copy: ReminderCopy,
    /// 看到哪个时刻了（本地时间）
    last: Option<NaiveDateTime>,
    last_saved_ms: i64,
    /// 稍后再提醒的：到点时刻 → 那次提醒
    snoozed: Vec<(NaiveDateTime, Reminder)>,
    /// 显示中的提醒，等用户操作
    shown: HashMap<u32, Reminder>,
    next_id: u32,
}

impl ReminderService {
    pub fn new(port: Arc<dyn ReminderPort>, clock: Arc<dyn Clock>, copy: ReminderCopy) -> Self {
        Self {
            port,
            clock,
            copy,
            last: None,
            last_saved_ms: 0,
            snoozed: Vec::new(),
            shown: HashMap::new(),
            next_id: 1,
        }
    }

    pub async fn run(mut self, mut cmds: mpsc::Receiver<ReminderCmd>) {
        self.start();
        let mut tick = tokio::time::interval(TICK);
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                Some(cmd) = cmds.recv() => self.on_cmd(cmd),
                _ = tick.tick() => {
                    self.on_tick();
                }
            }
        }
    }

    fn now(&self) -> NaiveDateTime {
        self.clock.now().naive_local()
    }

    fn to_ms(t: NaiveDateTime) -> i64 {
        Local
            .from_local_datetime(&t)
            .earliest()
            .map_or(0, |t: DateTime<Local>| t.timestamp_millis())
    }

    /// 启动：上次看到之后、12 小时以内到期的算“错过的提醒”，集中提示一次；第一次运行时从现在算起。
    pub fn start(&mut self) -> Vec<Reminder> {
        let now = self.now();
        let window = now - ChronoDuration::hours(MISSED_WINDOW_H);
        let after = self
            .port
            .checked()
            .and_then(|ms| Local.timestamp_millis_opt(ms).single())
            .map(|t| t.naive_local().max(window));
        self.last = Some(now);
        self.save(now, true);
        let Some(after) = after else {
            return Vec::new();
        };
        let missed = match self.collect(after, now) {
            Ok(m) => m,
            Err(e) => {
                self.port.note(&format!("读取错过的提醒失败：{e}"));
                return Vec::new();
            }
        };
        if !missed.is_empty() {
            self.port.missed(&self.copy.missed(missed.len()), &missed);
            for r in &missed {
                self.shown.insert(r.id, r.clone());
            }
        }
        missed
    }

    /// 看一次：`(上次, 现在]` 到期的和稍后再提醒到点的都提醒。返回这次提醒的。
    pub fn on_tick(&mut self) -> Vec<Reminder> {
        let now = self.now();
        let Some(last) = self.last else {
            self.start();
            return Vec::new();
        };
        if now <= last {
            return Vec::new();
        }
        let mut out = match self.collect(last, now) {
            Ok(v) => v,
            Err(e) => {
                // 读不出来：下次再从 last 看，不丢提醒
                self.port.note(&format!("读取提醒失败：{e}"));
                return Vec::new();
            }
        };
        let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut self.snoozed)
            .into_iter()
            .partition(|(at, _)| *at <= now);
        self.snoozed = later;
        out.extend(due.into_iter().map(|(_, r)| r));
        for r in &out {
            self.port.remind(r, r.kind.toast());
            self.shown.insert(r.id, r.clone());
        }
        self.last = Some(now);
        self.save(now, !out.is_empty());
        out
    }

    fn save(&mut self, now: NaiveDateTime, force: bool) {
        let ms = Self::to_ms(now);
        if force || ms - self.last_saved_ms >= SAVE_EVERY_MS {
            self.port.set_checked(ms);
            self.last_saved_ms = ms;
        }
    }

    /// 卡片或通知上的操作：知道了 → 收起；5 / 10 分钟后 → 到时再提醒一次（FR-SCH-07）。
    pub fn on_cmd(&mut self, cmd: ReminderCmd) {
        let ReminderCmd::Act { id, action } = cmd;
        let Some(r) = self.shown.remove(&id) else {
            return;
        };
        let minutes = match action.as_str() {
            ACTION_SNOOZE_5 => 5,
            ACTION_SNOOZE_10 => 10,
            _ => return,
        };
        let at = self.now() + ChronoDuration::minutes(minutes);
        self.snoozed.push((at, r));
    }

    /// `(after, until]` 到期的提醒，配好文案。
    fn collect(
        &mut self,
        after: NaiveDateTime,
        until: NaiveDateTime,
    ) -> Result<Vec<Reminder>, StoreError> {
        let today = until.date();
        let from = (after.date() - ChronoDuration::days(1))
            .format("%Y-%m-%d")
            .to_string();
        let db = self.port.db();
        let schedules = db.schedules_added_from(&from)?;
        let todos = db.todos_open()?;
        drop(db);
        let mut all: Vec<Due> = schedules.iter().flat_map(reminder::schedule_dues).collect();
        all.extend(todos.iter().flat_map(reminder::todo_dues));
        if self.port.digest_enabled() && !todos.is_empty() {
            let mut d = after.date();
            while d <= until.date() {
                all.push(reminder::digest_due(d));
                d += ChronoDuration::days(1);
            }
        }
        let mut out = Vec::new();
        for due in reminder::due_between(all, after, until) {
            let (title, start, location) = match due.kind {
                RemindKind::Todo | RemindKind::TodoEve => {
                    let t = todos.iter().find(|t| t.id == due.ref_id);
                    (t.and_then(|t| t.title.clone()), None, None)
                }
                RemindKind::TodoDigest => (None, None, None),
                _ => {
                    let s = schedules.iter().find(|s| s.id == due.ref_id);
                    let start = s.and_then(|s| {
                        let d = chrono::NaiveDate::parse_from_str(s.date.as_deref()?, "%Y-%m-%d")
                            .ok()?;
                        let t =
                            chrono::NaiveTime::parse_from_str(s.time.as_deref()?, "%H:%M").ok()?;
                        Some(d.and_time(t))
                    });
                    (
                        s.and_then(|s| s.title.clone()),
                        start,
                        s.and_then(|s| s.location.clone()),
                    )
                }
            };
            let time = start.map_or(String::new(), |t| reminder::time_text(t, today));
            let notice = self.copy.notice(
                due.kind,
                &Subject {
                    title: title.as_deref().unwrap_or_default(),
                    time: &time,
                    location: location.as_deref(),
                    count: todos.len(),
                },
            );
            let id = self.next_id;
            self.next_id += 1;
            out.push(Reminder {
                id,
                kind: due.kind,
                ref_id: due.ref_id,
                notice,
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Mutex;

    use super::*;
    use crate::domain::schedule::{ScheduleDraft, TodoDraft};
    use crate::infra::clock::ManualClock;
    use crate::infra::templates::TemplateDirs;

    struct Port {
        db: Mutex<Db>,
        checked: Mutex<Option<i64>>,
        digest: bool,
        reminded: Mutex<Vec<(Reminder, bool)>>,
        missed: Mutex<Vec<(Notice, usize)>>,
    }

    struct Guard<'a>(std::sync::MutexGuard<'a, Db>);
    impl Deref for Guard<'_> {
        type Target = Db;
        fn deref(&self) -> &Db {
            &self.0
        }
    }

    impl ReminderPort for Port {
        fn db(&self) -> Box<dyn Deref<Target = Db> + '_> {
            Box::new(Guard(self.db.lock().unwrap()))
        }
        fn checked(&self) -> Option<i64> {
            *self.checked.lock().unwrap()
        }
        fn set_checked(&self, ms: i64) {
            *self.checked.lock().unwrap() = Some(ms);
        }
        fn digest_enabled(&self) -> bool {
            self.digest
        }
        fn remind(&self, r: &Reminder, toast: bool) {
            self.reminded.lock().unwrap().push((r.clone(), toast));
        }
        fn missed(&self, notice: &Notice, items: &[Reminder]) {
            self.missed
                .lock()
                .unwrap()
                .push((notice.clone(), items.len()));
        }
    }

    fn at(d: u32, h: u32, m: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 10, d, h, m, 0).unwrap()
    }

    fn rig(digest: bool, start: DateTime<Local>) -> (ReminderService, Arc<Port>, Arc<ManualClock>) {
        let dirs = TemplateDirs::factory_only(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../hub_templates"),
        );
        let port = Arc::new(Port {
            db: Mutex::new(Db::open_in_memory().unwrap()),
            checked: Mutex::default(),
            digest,
            reminded: Mutex::default(),
            missed: Mutex::default(),
        });
        let clock = Arc::new(ManualClock::new(start));
        let s = ReminderService::new(
            port.clone(),
            clock.clone(),
            ReminderCopy::load(&dirs).unwrap(),
        );
        (s, port, clock)
    }

    fn add_schedule(port: &Port, title: &str, time: &str) -> i64 {
        let d = ScheduleDraft {
            title: title.into(),
            date: Some("2026-10-09".into()),
            time: Some(time.into()),
            end_time: None,
            all_day: false,
            location: Some("实验楼".into()),
            is_deadline: false,
            remind_offsets: vec![600],
            source: "manual".into(),
            flags: vec![],
        };
        port.db().schedule_create(&d, "added", 0).unwrap().0
    }

    #[test]
    fn reminds_at_the_offset_and_snoozes() {
        let (mut s, port, clock) = rig(false, at(9, 14, 0));
        let id = add_schedule(&port, "组会", "15:00");
        assert!(s.start().is_empty(), "第一次运行不补");
        clock.advance_ms(49 * 60_000);
        assert!(s.on_tick().is_empty());
        clock.advance_ms(60_000);
        let got = s.on_tick();
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].kind, got[0].ref_id), (RemindKind::Schedule, id));
        assert_eq!(
            (got[0].notice.title.as_str(), got[0].notice.body.as_str()),
            ("组会", "15:00 开始，实验楼")
        );
        assert!(port.reminded.lock().unwrap()[0].1, "日程提醒弹系统通知");
        assert!(s.on_tick().is_empty(), "同一次不重复");

        s.on_cmd(ReminderCmd::Act {
            id: got[0].id,
            action: ACTION_SNOOZE_5.into(),
        });
        clock.advance_ms(4 * 60_000);
        assert!(s.on_tick().is_empty());
        clock.advance_ms(60_000);
        assert_eq!(s.on_tick().len(), 1, "5 分钟后再提醒");
    }

    #[test]
    fn missed_within_12_hours_once_on_start() {
        let (mut s, port, clock) = rig(false, at(9, 20, 0));
        add_schedule(&port, "组会", "15:00");
        add_schedule(&port, "早课", "08:00");
        // 上次 Hub 运行到 9 号凌晨 1 点
        *port.checked.lock().unwrap() = Some(at(9, 1, 0).timestamp_millis());
        let missed = s.start();
        // 早课 07:50 在 12 小时窗口外（20:00 往前 12 小时是 08:00），只补组会
        assert_eq!(missed.len(), 1);
        assert_eq!(port.missed.lock().unwrap()[0].1, 1);
        assert_eq!(
            port.missed.lock().unwrap()[0].0.body,
            "有 1 个提醒在你离开时到期了"
        );
        assert_eq!(
            *port.checked.lock().unwrap(),
            Some(at(9, 20, 0).timestamp_millis())
        );
        clock.advance_ms(60_000);
        assert!(s.on_tick().is_empty(), "不再重复");
    }

    #[test]
    fn todo_reminders_and_digest() {
        let (mut s, port, clock) = rig(true, at(8, 19, 59));
        let draft = TodoDraft {
            title: "交报告".into(),
            due_date: Some("2026-10-09".into()),
            source: "manual".into(),
        };
        let (id, _) = port.db().todo_create(&draft, "open", 0).unwrap();
        s.start();
        clock.advance_ms(60_000);
        let eve = s.on_tick();
        assert_eq!((eve[0].kind, eve[0].ref_id), (RemindKind::TodoEve, id));
        assert!(
            !port.reminded.lock().unwrap()[0].1,
            "前一天晚上只在小组件提醒"
        );
        clock.advance_ms(13 * 3_600_000);
        let morning: Vec<RemindKind> = s.on_tick().iter().map(|r| r.kind).collect();
        assert_eq!(morning, [RemindKind::Todo, RemindKind::TodoDigest]);
    }
}
