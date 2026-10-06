use chrono::{NaiveDate, NaiveTime};
use rusqlite::{OptionalExtension, params};
use sha2::{Digest, Sha256};

use crate::domain::schedule::{ScheduleDraft, TodoDraft};

use super::{Db, StoreError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleRow {
    pub id: i64,
    pub title: Option<String>,
    pub date: Option<String>,
    pub time: Option<String>,
    pub end_time: Option<String>,
    pub all_day: bool,
    pub location: Option<String>,
    pub is_deadline: bool,
    pub remind_offsets: Vec<i64>,
    pub status: String,
    pub source: String,
    pub flags: Vec<String>,
    pub created_ts: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoRow {
    pub id: i64,
    pub title: Option<String>,
    pub due_date: Option<String>,
    pub status: String,
    pub source: String,
    pub created_ts: i64,
    pub done_ts: Option<i64>,
}

/// 去重窗口：24 小时内同一哈希不再提示（FR-SCH-06）。时间戳一律是 Unix 毫秒。
const DEDUP_WINDOW_MS: i64 = 24 * 60 * 60 * 1000;

const SCHEDULE_COLS: &str = "id,title,date,time,end_time,all_day,location,is_deadline,remind_offsets,status,source,flags,created_ts";
const TODO_COLS: &str = "id,title,due_date,status,source,created_ts,done_ts";

fn schedule_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ScheduleRow> {
    let offsets: String = row.get(8)?;
    let flags: String = row.get(11)?;
    Ok(ScheduleRow {
        id: row.get(0)?,
        title: row.get(1)?,
        date: row.get(2)?,
        time: row.get(3)?,
        end_time: row.get(4)?,
        all_day: row.get(5)?,
        location: row.get(6)?,
        is_deadline: row.get(7)?,
        remind_offsets: serde_json::from_str(&offsets).unwrap_or_default(),
        status: row.get(9)?,
        source: row.get(10)?,
        flags: flags
            .split(',')
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect(),
        created_ts: row.get(12)?,
    })
}

fn todo_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TodoRow> {
    Ok(TodoRow {
        id: row.get(0)?,
        title: row.get(1)?,
        due_date: row.get(2)?,
        status: row.get(3)?,
        source: row.get(4)?,
        created_ts: row.get(5)?,
        done_ts: row.get(6)?,
    })
}

fn valid_date(date: Option<&str>) -> bool {
    date.is_none_or(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").is_ok())
}

fn valid_time(time: Option<&str>) -> bool {
    time.is_none_or(|t| NaiveTime::parse_from_str(t, "%H:%M").is_ok())
}

fn check_schedule(draft: &ScheduleDraft) -> Result<(), StoreError> {
    if draft.title.trim().is_empty()
        || draft.title.chars().count() > 12
        || !matches!(draft.source.as_str(), "ai" | "manual")
        || draft.remind_offsets.iter().any(|offset| *offset < 0)
        || !valid_date(draft.date.as_deref())
        || !valid_time(draft.time.as_deref())
        || !valid_time(draft.end_time.as_deref())
        || draft.all_day && draft.time.is_some()
    {
        return Err(StoreError::InvalidSchedule);
    }
    Ok(())
}

fn schedule_hash(draft: &ScheduleDraft) -> String {
    digest(&[
        &normalize(&draft.title),
        draft.date.as_deref().unwrap_or_default(),
        draft.time.as_deref().unwrap_or_default(),
    ])
}

fn check_todo(draft: &TodoDraft) -> Result<(), StoreError> {
    if draft.title.trim().is_empty()
        || draft.title.chars().count() > 16
        || !matches!(draft.source.as_str(), "ai" | "manual")
        || !valid_date(draft.due_date.as_deref())
    {
        return Err(StoreError::InvalidTodo);
    }
    Ok(())
}

fn todo_hash(draft: &TodoDraft) -> String {
    digest(&[
        &normalize(&draft.title),
        draft.due_date.as_deref().unwrap_or_default(),
    ])
}

impl Db {
    /// 插入结构化日程（`pending` 待确认或 `added` 已添加）。24 小时内哈希重复时返回既有 id 与 `false`。
    pub fn schedule_create(
        &self,
        draft: &ScheduleDraft,
        status: &str,
        created_ts: i64,
    ) -> Result<(i64, bool), StoreError> {
        check_schedule(draft)?;
        if !matches!(status, "pending" | "added") {
            return Err(StoreError::InvalidSchedule);
        }
        let hash = schedule_hash(draft);
        if let Some(id) = self.conn.query_row(
            "SELECT id FROM schedule WHERE dedup_hash = ?1 AND created_ts >= ?2 ORDER BY created_ts DESC LIMIT 1",
            params![hash, created_ts.saturating_sub(DEDUP_WINDOW_MS)],
            |row| row.get(0),
        ).optional()? {
            return Ok((id, false));
        }
        let offsets = serde_json::to_string(&draft.remind_offsets)?;
        let flags = draft.flags.join(",");
        self.conn.execute(
            "INSERT INTO schedule (title, date, time, end_time, all_day, location, is_deadline, remind_offsets, status, source, flags, dedup_hash, created_ts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![draft.title.trim(), draft.date, draft.time, draft.end_time, draft.all_day, draft.location,
                draft.is_deadline, offsets, status, draft.source, flags, hash, created_ts],
        )?;
        Ok((self.conn.last_insert_rowid(), true))
    }

    pub fn schedule_get(&self, id: i64) -> Result<Option<ScheduleRow>, StoreError> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {SCHEDULE_COLS} FROM schedule WHERE id = ?1"),
                [id],
                schedule_row,
            )
            .optional()?)
    }

    pub fn schedules_by_status(&self, status: &str) -> Result<Vec<ScheduleRow>, StoreError> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {SCHEDULE_COLS} FROM schedule WHERE status = ?1 ORDER BY date,time,id"
        ))?;
        let rows = stmt.query_map([status], schedule_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// 某一天已添加的日程（冲突与“可能已经添加过”的判断，FR-SCH-06、FR-SCH-11）。
    pub fn schedules_added_on(&self, date: &str) -> Result<Vec<ScheduleRow>, StoreError> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {SCHEDULE_COLS} FROM schedule WHERE status = 'added' AND date = ?1 ORDER BY time,id"
        ))?;
        let rows = stmt.query_map([date], schedule_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// 从 `from`（`YYYY-MM-DD`）起已添加的日程（提醒调度，FR-SCH-07）。
    pub fn schedules_added_from(&self, from: &str) -> Result<Vec<ScheduleRow>, StoreError> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {SCHEDULE_COLS} FROM schedule WHERE status = 'added' AND date >= ?1 ORDER BY date,time,id"
        ))?;
        let rows = stmt.query_map([from], schedule_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// 修改待确认或已添加的日程（卡片上的“修改”、日程列表的编辑，FR-SCH-05、FR-SCH-09），哈希随字段重算。
    /// 来源与创建时间不变。没有这条或它已被忽略时返回 `false`。
    pub fn schedule_update(&self, id: i64, draft: &ScheduleDraft) -> Result<bool, StoreError> {
        check_schedule(draft)?;
        let offsets = serde_json::to_string(&draft.remind_offsets)?;
        Ok(self.conn.execute(
            "UPDATE schedule SET title=?2, date=?3, time=?4, end_time=?5, all_day=?6, location=?7, is_deadline=?8,
                 remind_offsets=?9, flags=?10, dedup_hash=?11
             WHERE id=?1 AND status IN ('pending','added')",
            params![id, draft.title.trim(), draft.date, draft.time, draft.end_time, draft.all_day, draft.location,
                draft.is_deadline, offsets, draft.flags.join(","), schedule_hash(draft)],
        )? == 1)
    }

    /// 待确认 → 已添加（卡片上的“添加”，FR-SCH-05）。
    pub fn schedule_confirm(&self, id: i64) -> Result<bool, StoreError> {
        Ok(self.conn.execute(
            "UPDATE schedule SET status='added' WHERE id=?1 AND status='pending'",
            [id],
        )? == 1)
    }

    /// 忽略后只保留哈希与时间，按 FR-SCH-05/10 删除结构化内容。
    pub fn schedule_ignore(&self, id: i64) -> Result<bool, StoreError> {
        Ok(self.conn.execute(
            "UPDATE schedule SET title=NULL,date=NULL,time=NULL,end_time=NULL,location=NULL,remind_offsets='[]',status='ignored',flags='' WHERE id=?1 AND status='pending'",
            [id],
        )? == 1)
    }

    pub fn schedule_delete(&self, id: i64) -> Result<bool, StoreError> {
        Ok(self
            .conn
            .execute("DELETE FROM schedule WHERE id=?1", [id])?
            == 1)
    }

    /// 清理超过 7 天仍未确认的 AI 日程。
    pub fn schedules_expire_pending(&self, now_ts: i64) -> Result<usize, StoreError> {
        Ok(self.conn.execute(
            "DELETE FROM schedule WHERE status='pending' AND source='ai' AND created_ts < ?1",
            [now_ts.saturating_sub(7 * DEDUP_WINDOW_MS)],
        )?)
    }

    /// 插入待办（`pending` 待确认或 `open` 未完成）。24 小时内哈希重复时返回既有 id 与 `false`。
    pub fn todo_create(
        &self,
        draft: &TodoDraft,
        status: &str,
        created_ts: i64,
    ) -> Result<(i64, bool), StoreError> {
        check_todo(draft)?;
        if !matches!(status, "pending" | "open") {
            return Err(StoreError::InvalidTodo);
        }
        let hash = todo_hash(draft);
        if let Some(id) = self.conn.query_row(
            "SELECT id FROM todo WHERE dedup_hash=?1 AND created_ts >= ?2 ORDER BY created_ts DESC LIMIT 1",
            params![hash, created_ts.saturating_sub(DEDUP_WINDOW_MS)],
            |row| row.get(0),
        ).optional()? {
            return Ok((id, false));
        }
        self.conn.execute(
            "INSERT INTO todo (title,due_date,status,source,dedup_hash,created_ts) VALUES (?1,?2,?3,?4,?5,?6)",
            params![draft.title.trim(), draft.due_date, status, draft.source, hash, created_ts],
        )?;
        Ok((self.conn.last_insert_rowid(), true))
    }

    pub fn todo_get(&self, id: i64) -> Result<Option<TodoRow>, StoreError> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {TODO_COLS} FROM todo WHERE id = ?1"),
                [id],
                todo_row,
            )
            .optional()?)
    }

    pub fn todos_open(&self) -> Result<Vec<TodoRow>, StoreError> {
        self.todos_by_status("open")
    }

    pub fn todos_by_status(&self, status: &str) -> Result<Vec<TodoRow>, StoreError> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {TODO_COLS} FROM todo WHERE status = ?1 ORDER BY due_date IS NULL,due_date,id"
        ))?;
        let rows = stmt.query_map([status], todo_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// 修改待确认或未完成的待办，哈希随字段重算。没有这条或状态不对时返回 `false`。
    pub fn todo_update(&self, id: i64, draft: &TodoDraft) -> Result<bool, StoreError> {
        check_todo(draft)?;
        Ok(self.conn.execute(
            "UPDATE todo SET title=?2, due_date=?3, dedup_hash=?4 WHERE id=?1 AND status IN ('pending','open')",
            params![id, draft.title.trim(), draft.due_date, todo_hash(draft)],
        )? == 1)
    }

    /// 待确认 → 未完成（卡片上的“加入待办”，FR-SCH-13）。
    pub fn todo_confirm(&self, id: i64) -> Result<bool, StoreError> {
        Ok(self.conn.execute(
            "UPDATE todo SET status='open' WHERE id=?1 AND status='pending'",
            [id],
        )? == 1)
    }

    /// 忽略：只留哈希，标题与截止日期立即删除（与日程相同，FR-SCH-13、FR-SCH-10）。
    pub fn todo_ignore(&self, id: i64) -> Result<bool, StoreError> {
        Ok(self.conn.execute(
            "UPDATE todo SET title=NULL,due_date=NULL,status='ignored' WHERE id=?1 AND status='pending'",
            [id],
        )? == 1)
    }

    pub fn todo_delete(&self, id: i64) -> Result<bool, StoreError> {
        Ok(self.conn.execute("DELETE FROM todo WHERE id=?1", [id])? == 1)
    }

    /// `date`（`YYYY-MM-DD`）当天已加入、且到 `now_hhmm`（`HH:MM`）为止已经开始的日程数；全天日程算已完成
    /// （晚间小结的“完成日程”，FR-REV-01，ADR 0029）。
    pub fn schedules_done_on(&self, date: &str, now_hhmm: &str) -> Result<u32, StoreError> {
        Ok(self.conn.query_row(
            "SELECT count(*) FROM schedule WHERE status='added' AND date=?1
             AND (all_day=1 OR time IS NULL OR time<=?2)",
            params![date, now_hhmm],
            |r| r.get(0),
        )?)
    }

    /// `[from, to)`（Unix 毫秒）之间完成的待办数。
    pub fn todos_done_between(&self, from: i64, to: i64) -> Result<u32, StoreError> {
        Ok(self.conn.query_row(
            "SELECT count(*) FROM todo WHERE status='done' AND done_ts>=?1 AND done_ts<?2",
            params![from, to],
            |r| r.get(0),
        )?)
    }

    pub fn todo_mark_done(&self, id: i64, done_ts: i64) -> Result<bool, StoreError> {
        Ok(self.conn.execute(
            "UPDATE todo SET status='done',done_ts=?2 WHERE id=?1 AND status='open'",
            params![id, done_ts],
        )? == 1)
    }
}

fn normalize(text: &str) -> String {
    text.chars()
        .filter(|c| {
            !c.is_whitespace()
                && !c.is_ascii_punctuation()
                && !"，。！？；：、（）【】《》“”‘’".contains(*c)
        })
        .collect()
}

fn digest(parts: &[&str]) -> String {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update(part.as_bytes());
        hash.update([0]);
    }
    format!("{:x}", hash.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schedule() -> ScheduleDraft {
        ScheduleDraft {
            title: "组会".into(),
            date: Some("2026-10-09".into()),
            time: Some("15:00".into()),
            end_time: None,
            all_day: false,
            location: Some("实验楼".into()),
            is_deadline: false,
            remind_offsets: vec![600],
            source: "ai".into(),
            flags: vec![],
        }
    }

    #[test]
    fn counts_for_the_evening_summary() {
        let db = Db::open_in_memory().unwrap();
        db.schedule_create(&schedule(), "added", 1).unwrap();
        let mut later = schedule();
        later.title = "晚课".into();
        later.time = Some("19:00".into());
        db.schedule_create(&later, "added", 2).unwrap();
        let mut pending = schedule();
        pending.title = "未确认".into();
        db.schedule_create(&pending, "pending", 3).unwrap();
        assert_eq!(db.schedules_done_on("2026-10-09", "16:00").unwrap(), 1);
        assert_eq!(db.schedules_done_on("2026-10-09", "22:30").unwrap(), 2);
        assert_eq!(db.schedules_done_on("2026-10-10", "22:30").unwrap(), 0);

        let draft = |t: &str| TodoDraft {
            title: t.into(),
            due_date: None,
            source: "manual".into(),
        };
        let (a, _) = db.todo_create(&draft("交报告"), "open", 1).unwrap();
        let (b, _) = db.todo_create(&draft("买菜"), "open", 1).unwrap();
        db.todo_create(&draft("还书"), "open", 1).unwrap();
        db.todo_mark_done(a, 150).unwrap();
        db.todo_mark_done(b, 250).unwrap();
        assert_eq!(db.todos_done_between(100, 200).unwrap(), 1);
        assert_eq!(db.todos_done_between(100, 300).unwrap(), 2);
    }

    #[test]
    fn deduplicates_and_scrubs_ignored_schedule() {
        let db = Db::open_in_memory().unwrap();
        let (id, inserted) = db.schedule_create(&schedule(), "pending", 100_000).unwrap();
        assert!(inserted);
        assert_eq!(
            db.schedule_create(&schedule(), "pending", 110_000).unwrap(),
            (id, false)
        );
        // 窗口是 24 小时（毫秒）：一天多以后同一条会再提示
        assert!(
            db.schedule_create(&schedule(), "pending", 100_000 + 86_400_001)
                .unwrap()
                .1
        );
        assert!(db.schedule_ignore(id).unwrap());
        let stored: (Option<String>, Option<String>, String) = db
            .conn
            .query_row(
                "SELECT title,date,status FROM schedule WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(stored, (None, None, "ignored".into()));
    }

    #[test]
    fn todo_deduplicates_and_completes() {
        let db = Db::open_in_memory().unwrap();
        let draft = TodoDraft {
            title: "打印简历".into(),
            due_date: Some("2026-10-04".into()),
            source: "ai".into(),
        };
        let (id, inserted) = db.todo_create(&draft, "pending", 100_000).unwrap();
        assert!(inserted);
        assert_eq!(
            db.todo_create(&draft, "open", 100_001).unwrap(),
            (id, false)
        );
        assert!(
            !db.todo_mark_done(id, 200_000).unwrap(),
            "待确认的不能直接完成"
        );
        assert!(db.todo_confirm(id).unwrap());
        assert_eq!(db.todos_open().unwrap().len(), 1);
        assert!(db.todo_mark_done(id, 200_000).unwrap());
        assert!(db.todos_open().unwrap().is_empty());
        // 24 小时（毫秒）以后同一件事可以再提示
        let (again, inserted) = db
            .todo_create(&draft, "pending", 100_000 + 86_400_001)
            .unwrap();
        assert!(inserted && again != id);
    }

    #[test]
    fn schedule_edit_confirm_and_lists() {
        let db = Db::open_in_memory().unwrap();
        let (id, _) = db.schedule_create(&schedule(), "pending", 1).unwrap();
        let mut edited = schedule();
        edited.title = "改期组会".into();
        edited.time = Some("16:00".into());
        assert!(db.schedule_update(id, &edited).unwrap());
        assert!(db.schedules_added_on("2026-10-09").unwrap().is_empty());
        assert!(db.schedule_confirm(id).unwrap());
        assert!(!db.schedule_confirm(id).unwrap(), "只有待确认的能添加");
        let row = db.schedule_get(id).unwrap().unwrap();
        assert_eq!(
            (
                row.title.as_deref(),
                row.time.as_deref(),
                row.status.as_str()
            ),
            (Some("改期组会"), Some("16:00"), "added")
        );
        assert_eq!(db.schedules_added_on("2026-10-09").unwrap().len(), 1);
        assert_eq!(db.schedules_added_from("2026-10-01").unwrap().len(), 1);
        assert!(db.schedules_added_from("2026-10-10").unwrap().is_empty());
        let mut bad = schedule();
        bad.time = Some("25:00".into());
        assert!(matches!(
            db.schedule_update(id, &bad),
            Err(StoreError::InvalidSchedule)
        ));
        assert!(db.schedule_delete(id).unwrap());
        assert_eq!(db.schedule_get(id).unwrap(), None);
    }

    #[test]
    fn todo_ignore_scrubs_and_update_rehashes() {
        let db = Db::open_in_memory().unwrap();
        let draft = TodoDraft {
            title: "打印简历".into(),
            due_date: None,
            source: "ai".into(),
        };
        let (id, _) = db.todo_create(&draft, "pending", 1).unwrap();
        assert!(db.todo_ignore(id).unwrap());
        let row = db.todo_get(id).unwrap().unwrap();
        assert_eq!((row.title, row.status.as_str()), (None, "ignored"));
        let (open, _) = db
            .todo_create(
                &TodoDraft {
                    title: "买菜".into(),
                    ..draft.clone()
                },
                "open",
                2,
            )
            .unwrap();
        let renamed = TodoDraft {
            title: "买水果".into(),
            due_date: Some("2026-10-10".into()),
            ..draft
        };
        assert!(db.todo_update(open, &renamed).unwrap());
        assert_eq!(
            db.todos_by_status("open").unwrap()[0].due_date.as_deref(),
            Some("2026-10-10")
        );
        assert!(db.todo_delete(open).unwrap());
    }
}
