//! `letter`（09 D-26）的读写：晴晴的周信（FR-REV-02，C-10）。保留 1 年由 `retention` 清理，导出已包含。

use rusqlite::params;

use super::{Db, StoreError};

/// 一封周信。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LetterRow {
    pub id: i64,
    /// 那周的周一，`YYYY-MM-DD`
    pub week_start: String,
    pub content: String,
    /// `llm` / `template`
    pub source: String,
    pub read: bool,
    pub created_ts: i64,
}

/// 新写入的一封。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewLetter<'a> {
    pub week_start: &'a str,
    pub content: &'a str,
    pub source: &'a str,
    pub model: Option<&'a str>,
    pub prompt_ver: Option<&'a str>,
    pub ts: i64,
}

impl Db {
    pub fn letter_insert(&self, l: &NewLetter<'_>) -> Result<i64, StoreError> {
        self.conn.execute(
            "INSERT INTO letter (week_start, content, source, model, prompt_ver, created_ts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                l.week_start,
                l.content,
                l.source,
                l.model,
                l.prompt_ver,
                l.ts
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// 全部周信，新的在前。
    pub fn letters(&self) -> Result<Vec<LetterRow>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, week_start, content, source, read, created_ts FROM letter
             ORDER BY week_start DESC, id DESC",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(LetterRow {
                    id: r.get(0)?,
                    week_start: r.get(1)?,
                    content: r.get(2)?,
                    source: r.get(3)?,
                    read: r.get::<_, i64>(4)? != 0,
                    created_ts: r.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// 标记已读；没有这封时返回 `false`。
    pub fn letter_mark_read(&self, id: i64) -> Result<bool, StoreError> {
        Ok(self
            .conn
            .execute("UPDATE letter SET read = 1 WHERE id = ?1", [id])?
            > 0)
    }

    /// 删除一封；没有这封时返回 `false`。
    pub fn letter_delete(&self, id: i64) -> Result<bool, StoreError> {
        Ok(self
            .conn
            .execute("DELETE FROM letter WHERE id = ?1", [id])?
            > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new<'a>(week: &'a str, source: &'a str, ts: i64) -> NewLetter<'a> {
        NewLetter {
            week_start: week,
            content: "你好呀",
            source,
            model: (source == "llm").then_some("m"),
            prompt_ver: (source == "llm").then_some("P-LETTER v1"),
            ts,
        }
    }

    #[test]
    fn insert_list_read_delete() {
        let db = Db::open_in_memory().unwrap();
        let a = db.letter_insert(&new("2026-09-28", "template", 1)).unwrap();
        let b = db.letter_insert(&new("2026-10-05", "llm", 2)).unwrap();
        let all = db.letters().unwrap();
        assert_eq!(
            all.iter().map(|l| l.id).collect::<Vec<_>>(),
            [b, a],
            "新的在前"
        );
        assert_eq!(all[0].source, "llm");
        assert!(!all[0].read);

        assert!(db.letter_mark_read(b).unwrap());
        assert!(db.letters().unwrap()[0].read);
        assert!(db.letter_delete(a).unwrap());
        assert!(!db.letter_delete(a).unwrap());
        assert_eq!(db.letters().unwrap().len(), 1);
    }
}
