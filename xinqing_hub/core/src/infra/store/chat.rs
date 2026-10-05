//! 对话（09 D-11）与长期记忆（D-14）的读写，外加对话上下文要用的今日统计（FR-CHT-05）。

use rusqlite::{OptionalExtension, params};
use xqp::MoodState;

use super::{Db, StoreError, parse_state};
use crate::domain::chat::SafeMode;

/// 会话列表的一行（左侧抽屉，FR-CHT-02 第 3 条）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRow {
    pub id: i64,
    pub title: String,
    pub created_ts: i64,
    /// 最后一条消息的时间；还没有消息时等于 `created_ts`
    pub last_ts: i64,
    pub safe_mode: SafeMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageRow {
    pub id: i64,
    pub session_id: i64,
    /// `user` / `assistant` / `system_notice`
    pub role: String,
    pub content: String,
    pub ts: i64,
    pub ai_generated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryRow {
    pub id: i64,
    pub content: String,
    pub created_ts: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatSearchRow {
    pub id: i64,
    pub session_id: i64,
    pub title: String,
    pub role: String,
    pub content: String,
    pub ts: i64,
}

/// 新写入的一条消息。
pub struct NewMessage<'a> {
    pub session_id: i64,
    pub role: &'a str,
    pub content: &'a str,
    pub ts: i64,
    pub ai_generated: bool,
    pub model: Option<&'a str>,
    pub prompt_ver: Option<&'a str>,
}

const SESSION_COLS: &str = "s.id, s.title, s.created_ts,
    coalesce((SELECT max(ts) FROM chat_message m WHERE m.session_id = s.id), s.created_ts),
    s.safe_mode";

fn session_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<SessionRow> {
    Ok(SessionRow {
        id: r.get(0)?,
        title: r.get(1)?,
        created_ts: r.get(2)?,
        last_ts: r.get(3)?,
        safe_mode: SafeMode::from_db(r.get(4)?),
    })
}

impl Db {
    pub fn chat_session_create(&self, title: &str, ts: i64) -> Result<i64, StoreError> {
        self.conn.execute(
            "INSERT INTO chat_session (title, created_ts) VALUES (?1, ?2)",
            params![title, ts],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn chat_session(&self, id: i64) -> Result<Option<SessionRow>, StoreError> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {SESSION_COLS} FROM chat_session s WHERE s.id = ?1"),
                [id],
                session_row,
            )
            .optional()?)
    }

    /// 全部会话，最近有消息的在前。
    pub fn chat_sessions(&self) -> Result<Vec<SessionRow>, StoreError> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {SESSION_COLS} FROM chat_session s ORDER BY 4 DESC, s.id DESC"
        ))?;
        let rows = stmt.query_map([], session_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn chat_set_safe_mode(&self, id: i64, mode: SafeMode) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE chat_session SET safe_mode = ?2 WHERE id = ?1",
            params![id, mode.to_db()],
        )?;
        Ok(())
    }

    pub fn chat_message_insert(&self, m: &NewMessage<'_>) -> Result<i64, StoreError> {
        self.conn.execute(
            "INSERT INTO chat_message (session_id, role, content, ts, ai_generated, model, prompt_ver)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                m.session_id,
                m.role,
                m.content,
                m.ts,
                m.ai_generated as i64,
                m.model,
                m.prompt_ver
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// 一个会话的全部消息，从旧到新。
    pub fn chat_messages(&self, session_id: i64) -> Result<Vec<MessageRow>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, role, content, ts, ai_generated FROM chat_message
             WHERE session_id = ?1 ORDER BY ts, id",
        )?;
        let rows = stmt.query_map([session_id], |r| {
            Ok(MessageRow {
                id: r.get(0)?,
                session_id: r.get(1)?,
                role: r.get(2)?,
                content: r.get(3)?,
                ts: r.get(4)?,
                ai_generated: r.get::<_, i64>(5)? != 0,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn chat_message(&self, id: i64) -> Result<Option<MessageRow>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, session_id, role, content, ts, ai_generated FROM chat_message WHERE id = ?1",
                [id],
                |r| {
                    Ok(MessageRow {
                        id: r.get(0)?,
                        session_id: r.get(1)?,
                        role: r.get(2)?,
                        content: r.get(3)?,
                        ts: r.get(4)?,
                        ai_generated: r.get::<_, i64>(5)? != 0,
                    })
                },
            )
            .optional()?)
    }

    /// 删除一个会话（消息随 `ON DELETE CASCADE` 删除）；`None` 删除全部对话（FR-CHT-08）。
    pub fn chat_delete(&self, session_id: Option<i64>) -> Result<(), StoreError> {
        match session_id {
            Some(id) => self
                .conn
                .execute("DELETE FROM chat_session WHERE id = ?1", [id])?,
            None => self.conn.execute("DELETE FROM chat_session", [])?,
        };
        Ok(())
    }

    /// 最近的长期记忆，从新到旧（FR-CHT-05、FR-CHT-07）。
    pub fn memories_recent(&self, limit: usize) -> Result<Vec<String>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT content FROM memory ORDER BY created_ts DESC, id DESC LIMIT ?1")?;
        let rows = stmt.query_map([limit as i64], |r| r.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn memories_list(&self) -> Result<Vec<MemoryRow>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, content, created_ts FROM memory ORDER BY created_ts DESC, id DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(MemoryRow {
                id: r.get(0)?,
                content: r.get(1)?,
                created_ts: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// 已有 `max` 条时不写入，返回 `None`（FR-CHT-07 第 2 条）。计数和插入在同一条语句里。
    pub fn memory_insert(
        &self,
        content: &str,
        ts: i64,
        max: usize,
    ) -> Result<Option<i64>, StoreError> {
        let inserted = self.conn.execute(
            "INSERT INTO memory (content, created_ts)
             SELECT ?1, ?2 WHERE (SELECT COUNT(*) FROM memory) < ?3",
            params![content, ts, max as i64],
        )?;
        Ok((inserted != 0).then(|| self.conn.last_insert_rowid()))
    }

    pub fn memory_update(&self, id: i64, content: &str) -> Result<bool, StoreError> {
        Ok(self.conn.execute(
            "UPDATE memory SET content = ?2 WHERE id = ?1",
            params![id, content],
        )? != 0)
    }

    pub fn memory_delete(&self, id: i64) -> Result<bool, StoreError> {
        Ok(self
            .conn
            .execute("DELETE FROM memory WHERE id = ?1", [id])?
            != 0)
    }

    pub fn chat_search(&self, query: &str, limit: usize) -> Result<Vec<ChatSearchRow>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT m.id, m.session_id, s.title, m.role, m.content, m.ts
             FROM chat_message m JOIN chat_session s ON s.id = m.session_id
             WHERE instr(lower(m.content), lower(?1)) > 0
             ORDER BY m.ts DESC, m.id DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![query, limit as i64], |r| {
            Ok(ChatSearchRow {
                id: r.get(0)?,
                session_id: r.get(1)?,
                title: r.get(2)?,
                role: r.get(3)?,
                content: r.get(4)?,
                ts: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// `since` 之后每次显示的状态（今日状态摘要），从旧到新。
    pub fn shown_states_since(&self, since: i64) -> Result<Vec<(i64, MoodState)>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT ts, shown_state FROM mood_state WHERE ts >= ?1 ORDER BY ts, id")?;
        let rows = stmt.query_map([since], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (ts, s) = row?;
            if let Some(state) = parse_state(&s) {
                out.push((ts, state));
            }
        }
        Ok(out)
    }

    /// `since` 之后窗口的有效打字时长合计（毫秒，`features_json.active_ms`）。
    pub fn active_ms_since(&self, since: i64) -> Result<i64, StoreError> {
        Ok(self.conn.query_row(
            "SELECT coalesce(sum(json_extract(features_json, '$.active_ms')), 0)
             FROM window_features WHERE end_ts >= ?1",
            [since],
            |r| r.get::<_, f64>(0),
        )? as i64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg<'a>(session_id: i64, role: &'a str, content: &'a str, ts: i64) -> NewMessage<'a> {
        NewMessage {
            session_id,
            role,
            content,
            ts,
            ai_generated: role == "assistant",
            model: None,
            prompt_ver: None,
        }
    }

    #[test]
    fn sessions_messages_and_delete() {
        let db = Db::open_in_memory().unwrap();
        let a = db.chat_session_create("第一个", 100).unwrap();
        let b = db.chat_session_create("第二个", 200).unwrap();
        db.chat_message_insert(&msg(a, "user", "你好", 300))
            .unwrap();
        let reply = db
            .chat_message_insert(&msg(a, "assistant", "嗨", 310))
            .unwrap();
        // 最近有消息的在前；没有消息的按创建时间
        let list = db.chat_sessions().unwrap();
        assert_eq!(list.iter().map(|s| s.id).collect::<Vec<_>>(), [a, b]);
        assert_eq!(list[0].last_ts, 310);
        assert_eq!(list[1].last_ts, 200);
        let m = db.chat_messages(a).unwrap();
        assert_eq!(m.len(), 2);
        assert!(!m[0].ai_generated && m[1].ai_generated);
        assert_eq!(db.chat_message(reply).unwrap().unwrap().content, "嗨");

        db.chat_set_safe_mode(a, SafeMode::Dismissed).unwrap();
        assert_eq!(
            db.chat_session(a).unwrap().unwrap().safe_mode,
            SafeMode::Dismissed
        );
        db.chat_delete(Some(a)).unwrap();
        assert!(db.chat_messages(a).unwrap().is_empty(), "消息随会话删除");
        db.chat_delete(None).unwrap();
        assert!(db.chat_sessions().unwrap().is_empty());
    }

    #[test]
    fn today_stats() {
        let db = Db::open_in_memory().unwrap();
        db.conn
            .execute_batch(
                "INSERT INTO window_features (start_ts, end_ts, app_cat, features_json)
                   VALUES (0, 50, 'chat', '{\"active_ms\": 1000}'),
                          (100, 150, 'chat', '{\"active_ms\": 60000}'),
                          (200, 250, 'doc', '{\"active_ms\": 30000.0}');
                 INSERT INTO mood_state (ts, state, shown_state, source)
                   VALUES (90, 'low', 'low', 'rule'), (160, 'fluent', 'fluent', 'rule'),
                          (170, 'x', 'bogus', 'rule');",
            )
            .unwrap();
        assert_eq!(db.active_ms_since(100).unwrap(), 90_000);
        assert_eq!(
            db.shown_states_since(100).unwrap(),
            [(160, MoodState::Fluent)]
        );
        db.conn
            .execute(
                "INSERT INTO memory (content, created_ts) VALUES ('旧', 1), ('新', 2)",
                [],
            )
            .unwrap();
        assert_eq!(db.memories_recent(10).unwrap(), ["新", "旧"]);
    }

    #[test]
    fn memory_insert_stops_at_cap() {
        let db = Db::open_in_memory().unwrap();
        assert!(db.memory_insert("一", 1, 2).unwrap().is_some());
        assert!(db.memory_insert("二", 2, 2).unwrap().is_some());
        assert_eq!(db.memory_insert("三", 3, 2).unwrap(), None);
        assert_eq!(db.memories_list().unwrap().len(), 2);
    }

    #[test]
    fn memory_crud_and_chat_search() {
        let db = Db::open_in_memory().unwrap();
        let id = db.memory_insert("记住我喜欢晴天", 10, 50).unwrap().unwrap();
        assert_eq!(db.memories_list().unwrap()[0].content, "记住我喜欢晴天");
        assert!(db.memory_update(id, "记住我喜欢晴天和咖啡").unwrap());
        assert_eq!(
            db.memories_list().unwrap()[0].content,
            "记住我喜欢晴天和咖啡"
        );
        assert!(db.memory_delete(id).unwrap());
        assert!(db.memories_list().unwrap().is_empty());

        let session = db.chat_session_create("搜索测试", 20).unwrap();
        db.chat_message_insert(&msg(session, "user", "今天想喝咖啡", 30))
            .unwrap();
        let found = db.chat_search("咖啡", 50).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].session_id, session);
        assert!(db.chat_search("不存在", 50).unwrap().is_empty());
    }
}
