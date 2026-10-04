//! `comfort_log`（09 D-10）的读写：暖心话的去重、模板屏蔽、每日上限与冷却都从这里取数。

use rusqlite::{OptionalExtension, params};

use super::{Db, StoreError};

/// 写入 `comfort_log` 的一条暖心话。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComfortRecord<'a> {
    pub ts: i64,
    /// 触发时的显示状态（`hesitant` 等）
    pub state: &'a str,
    pub text: &'a str,
    /// `llm` / `template`
    pub source: &'a str,
    pub template_id: Option<&'a str>,
    pub model: Option<&'a str>,
    pub prompt_ver: Option<&'a str>,
    /// `auto`（主动关怀）/ `self_report`（负面自评后的回应）
    pub trigger: &'a str,
}

impl Db {
    pub fn comfort_insert(&self, r: &ComfortRecord<'_>) -> Result<i64, StoreError> {
        self.conn.execute(
            "INSERT INTO comfort_log (ts, state, text, source, template_id, model, prompt_ver, trigger)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                r.ts,
                r.state,
                r.text,
                r.source,
                r.template_id,
                r.model,
                r.prompt_ver,
                r.trigger
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// 最近 `limit` 条暖心话的正文，从旧到新（V5 去重、提示词里的“最近说过的话”）。
    pub fn comfort_recent_texts(&self, limit: usize) -> Result<Vec<String>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT text FROM comfort_log ORDER BY ts DESC, id DESC LIMIT ?1")?;
        let mut out = stmt
            .query_map([limit as i64], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        out.reverse();
        Ok(out)
    }

    /// 用户标记过“不合适”的模板句（FR-CMF-03 第 2 条、FR-CMF-05）。
    pub fn comfort_blocked_templates(&self) -> Result<Vec<String>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT template_id FROM comfort_log
             WHERE source = 'template' AND feedback = 'unfit' AND template_id IS NOT NULL",
        )?;
        Ok(stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<Result<_, _>>()?)
    }

    /// 给一句暖心话记反馈（`useful` / `unfit` / `mute`），后点的覆盖先点的。返回这句话是否还在。
    pub fn comfort_set_feedback(&self, id: i64, verdict: &str) -> Result<bool, StoreError> {
        let n = self.conn.execute(
            "UPDATE comfort_log SET feedback = ?2 WHERE id = ?1",
            params![id, verdict],
        )?;
        Ok(n > 0)
    }

    /// `since` 之后的暖心话里有反馈的：`(暖心话的时间, 反馈)`，按时间先后（自动降档用）。
    pub fn comfort_feedback_since(&self, since: i64) -> Result<Vec<(i64, String)>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT ts, feedback FROM comfort_log
             WHERE ts >= ?1 AND feedback IS NOT NULL ORDER BY ts, id",
        )?;
        Ok(stmt
            .query_map([since], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?)
    }

    /// `since` 之后主动关怀的次数和最近一次的时间（FR-CMF-06 每日上限、FR-CMF-01 第 3 条冷却）。
    /// 自评后的回应不计入（ADR 0015）。
    pub fn comfort_auto_since(&self, since: i64) -> Result<(u32, Option<i64>), StoreError> {
        let count: u32 = self.conn.query_row(
            "SELECT count(*) FROM comfort_log WHERE trigger = 'auto' AND ts >= ?1",
            [since],
            |r| r.get(0),
        )?;
        let last: Option<i64> = self
            .conn
            .query_row(
                "SELECT max(ts) FROM comfort_log WHERE trigger = 'auto'",
                [],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        Ok((count, last))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec<'a>(ts: i64, text: &'a str, trigger: &'a str) -> ComfortRecord<'a> {
        ComfortRecord {
            ts,
            state: "low",
            text,
            source: "template",
            template_id: Some("gl01"),
            model: None,
            prompt_ver: None,
            trigger,
        }
    }

    #[test]
    fn counts_only_proactive_care() {
        let db = Db::open_in_memory().unwrap();
        db.comfort_insert(&rec(100, "一", "auto")).unwrap();
        db.comfort_insert(&rec(200, "二", "auto")).unwrap();
        db.comfort_insert(&rec(300, "三", "self_report")).unwrap();
        assert_eq!(db.comfort_auto_since(150).unwrap(), (1, Some(200)));
        assert_eq!(db.comfort_auto_since(0).unwrap(), (2, Some(200)));
        assert_eq!(db.comfort_recent_texts(2).unwrap(), ["二", "三"]);
    }

    #[test]
    fn blocked_templates_come_from_unfit_feedback() {
        let db = Db::open_in_memory().unwrap();
        let id = db.comfort_insert(&rec(1, "一", "auto")).unwrap();
        assert!(db.comfort_blocked_templates().unwrap().is_empty());
        db.conn()
            .execute(
                "UPDATE comfort_log SET feedback = 'unfit' WHERE id = ?1",
                [id],
            )
            .unwrap();
        assert_eq!(db.comfort_blocked_templates().unwrap(), ["gl01"]);
    }

    #[test]
    fn trigger_column_rejects_unknown_values() {
        let db = Db::open_in_memory().unwrap();
        assert!(db.comfort_insert(&rec(1, "一", "other")).is_err());
        assert_eq!(db.comfort_auto_since(0).unwrap(), (0, None));
    }
}
