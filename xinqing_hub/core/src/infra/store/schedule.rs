use chrono::NaiveDate;
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

impl Db {
    /// 插入结构化日程。24 小时内哈希重复时返回既有 id 与 `false`。
    pub fn schedule_create(
        &self,
        draft: &ScheduleDraft,
        status: &str,
        created_ts: i64,
    ) -> Result<(i64, bool), StoreError> {
        if draft.title.trim().is_empty()
            || draft.title.chars().count() > 12
            || !matches!(draft.source.as_str(), "ai" | "manual")
            || !matches!(status, "pending" | "added")
            || draft.remind_offsets.iter().any(|offset| *offset < 0)
            || draft
                .date
                .as_deref()
                .is_some_and(|date| NaiveDate::parse_from_str(date, "%Y-%m-%d").is_err())
            || draft.all_day && draft.time.is_some()
        {
            return Err(StoreError::InvalidSchedule);
        }
        let hash = digest(&[
            &normalize(&draft.title),
            draft.date.as_deref().unwrap_or_default(),
            draft.time.as_deref().unwrap_or_default(),
        ]);
        if let Some(id) = self.conn.query_row(
            "SELECT id FROM schedule WHERE dedup_hash = ?1 AND created_ts >= ?2 ORDER BY created_ts DESC LIMIT 1",
            params![hash, created_ts.saturating_sub(86_400)],
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

    pub fn schedules_by_status(&self, status: &str) -> Result<Vec<ScheduleRow>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id,title,date,time,end_time,all_day,location,is_deadline,remind_offsets,status,source,flags,created_ts
             FROM schedule WHERE status = ?1 ORDER BY date,time,id",
        )?;
        let rows = stmt.query_map([status], |row| {
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
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// 忽略后只保留哈希与时间，按 FR-SCH-05/10 删除结构化内容。
    pub fn schedule_ignore(&self, id: i64) -> Result<bool, StoreError> {
        Ok(self.conn.execute(
            "UPDATE schedule SET title=NULL,date=NULL,time=NULL,end_time=NULL,location=NULL,remind_offsets='[]',status='ignored',flags='' WHERE id=?1 AND status='pending'",
            [id],
        )? == 1)
    }

    /// 清理超过 7 天仍未确认的 AI 日程。
    pub fn schedules_expire_pending(&self, now_ts: i64) -> Result<usize, StoreError> {
        Ok(self.conn.execute(
            "DELETE FROM schedule WHERE status='pending' AND source='ai' AND created_ts < ?1",
            [now_ts.saturating_sub(7 * 86_400)],
        )?)
    }

    pub fn todo_create(
        &self,
        draft: &TodoDraft,
        created_ts: i64,
    ) -> Result<(i64, bool), StoreError> {
        if draft.title.trim().is_empty()
            || draft.title.chars().count() > 16
            || !matches!(draft.source.as_str(), "ai" | "manual")
            || draft
                .due_date
                .as_deref()
                .is_some_and(|date| NaiveDate::parse_from_str(date, "%Y-%m-%d").is_err())
        {
            return Err(StoreError::InvalidTodo);
        }
        let hash = digest(&[
            &normalize(&draft.title),
            draft.due_date.as_deref().unwrap_or_default(),
        ]);
        if let Some(id) = self.conn.query_row(
            "SELECT id FROM todo WHERE dedup_hash=?1 AND created_ts >= ?2 ORDER BY created_ts DESC LIMIT 1",
            params![hash, created_ts.saturating_sub(86_400)],
            |row| row.get(0),
        ).optional()? {
            return Ok((id, false));
        }
        self.conn.execute(
            "INSERT INTO todo (title,due_date,status,source,dedup_hash,created_ts) VALUES (?1,?2,'open',?3,?4,?5)",
            params![draft.title.trim(), draft.due_date, draft.source, hash, created_ts],
        )?;
        Ok((self.conn.last_insert_rowid(), true))
    }

    pub fn todos_open(&self) -> Result<Vec<TodoRow>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id,title,due_date,status,source,created_ts,done_ts FROM todo WHERE status='open' ORDER BY due_date IS NULL,due_date,id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(TodoRow {
                id: row.get(0)?,
                title: row.get(1)?,
                due_date: row.get(2)?,
                status: row.get(3)?,
                source: row.get(4)?,
                created_ts: row.get(5)?,
                done_ts: row.get(6)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
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
        let (a, _) = db.todo_create(&draft("交报告"), 1).unwrap();
        let (b, _) = db.todo_create(&draft("买菜"), 1).unwrap();
        db.todo_create(&draft("还书"), 1).unwrap();
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
        let (id, inserted) = db.todo_create(&draft, 100_000).unwrap();
        assert!(inserted);
        assert_eq!(db.todo_create(&draft, 100_001).unwrap(), (id, false));
        assert!(db.todo_mark_done(id, 200_000).unwrap());
        assert!(db.todos_open().unwrap().is_empty());
    }
}
