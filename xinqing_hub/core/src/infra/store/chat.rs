//! 对话（09 D-11）与长期记忆（D-14）的读写，外加对话上下文要用的今日统计（FR-CHT-05）。

use rusqlite::{OptionalExtension, params};
use xqp::MoodState;

use super::{Db, StoreError, parse_state};
use crate::domain::chat::{ChatMode, SafeMode};

/// 会话列表的一行（左侧抽屉，FR-CHT-02 第 3 条）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRow {
    pub id: i64,
    pub title: String,
    pub created_ts: i64,
    /// 最后一条消息的时间；还没有消息时等于 `created_ts`
    pub last_ts: i64,
    pub safe_mode: SafeMode,
    /// 快捷指令切换的对话方式（ADR 0021）
    pub mode: ChatMode,
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
    s.safe_mode, s.mode";

fn session_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<SessionRow> {
    Ok(SessionRow {
        id: r.get(0)?,
        title: r.get(1)?,
        created_ts: r.get(2)?,
        last_ts: r.get(3)?,
        safe_mode: SafeMode::from_db(r.get(4)?),
        mode: ChatMode::from_db(&r.get::<_, String>(5)?),
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

    /// 切换会话的对话方式；会话不存在时返回 `false`。
    pub fn chat_set_mode(&self, id: i64, mode: ChatMode) -> Result<bool, StoreError> {
        Ok(self.conn.execute(
            "UPDATE chat_session SET mode = ?2 WHERE id = ?1",
            params![id, mode.as_db()],
        )? != 0)
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
}
