//! `diary`（09 D-13）的读写：情绪日记（FR-DIA-02/03，C-10）。保留到用户删除，导出已包含。

use rusqlite::{OptionalExtension, params};

use super::{Db, StoreError};

/// 一篇日记。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiaryRow {
    pub id: i64,
    /// 本地日期 `YYYY-MM-DD`
    pub date: String,
    pub content: String,
    /// `ai_draft` / `ai_edited` / `manual`
    pub source: String,
    pub created_ts: i64,
    pub updated_ts: i64,
}

const COLS: &str = "id, date, content, source, created_ts, updated_ts";

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<DiaryRow> {
    Ok(DiaryRow {
        id: r.get(0)?,
        date: r.get(1)?,
        content: r.get(2)?,
        source: r.get(3)?,
        created_ts: r.get(4)?,
        updated_ts: r.get(5)?,
    })
}

impl Db {
    pub fn diary_insert(
        &self,
        date: &str,
        content: &str,
        source: &str,
        ts: i64,
    ) -> Result<i64, StoreError> {
        self.conn.execute(
            "INSERT INTO diary (date, content, source, created_ts, updated_ts) VALUES (?1, ?2, ?3, ?4, ?4)",
            params![date, content, source, ts],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn diary_get(&self, id: i64) -> Result<Option<DiaryRow>, StoreError> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {COLS} FROM diary WHERE id = ?1"),
                [id],
                row,
            )
            .optional()?)
    }

    /// 改内容与来源；没有这篇时返回 `false`。
    pub fn diary_update(
        &self,
        id: i64,
        content: &str,
        source: &str,
        ts: i64,
    ) -> Result<bool, StoreError> {
        Ok(self.conn.execute(
            "UPDATE diary SET content = ?2, source = ?3, updated_ts = ?4 WHERE id = ?1",
            params![id, content, source, ts],
        )? > 0)
    }

    /// 全部日记，新的在前（同一天按写的先后倒序）。
    pub fn diaries(&self) -> Result<Vec<DiaryRow>, StoreError> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM diary ORDER BY date DESC, created_ts DESC, id DESC"
        ))?;
        let rows = stmt.query_map([], row)?.collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// 删除一篇；没有这篇时返回 `false`。
    pub fn diary_delete(&self, id: i64) -> Result<bool, StoreError> {
        Ok(self.conn.execute("DELETE FROM diary WHERE id = ?1", [id])? > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_update_list_delete() {
        let db = Db::open_in_memory().unwrap();
        let a = db.diary_insert("2026-10-05", "昨天", "manual", 1).unwrap();
        let b = db
            .diary_insert("2026-10-06", "草稿", "ai_draft", 2)
            .unwrap();
        let c = db
            .diary_insert("2026-10-06", "又一篇", "manual", 3)
            .unwrap();
        let ids: Vec<i64> = db.diaries().unwrap().iter().map(|d| d.id).collect();
        assert_eq!(ids, [c, b, a], "新的在前，每天可以有多篇");

        assert!(db.diary_update(b, "改过的草稿", "ai_edited", 9).unwrap());
        let got = db.diary_get(b).unwrap().unwrap();
        assert_eq!(
            (
                got.content.as_str(),
                got.source.as_str(),
                got.created_ts,
                got.updated_ts
            ),
            ("改过的草稿", "ai_edited", 2, 9)
        );
        assert!(db.diary_delete(a).unwrap());
        assert!(!db.diary_delete(a).unwrap());
        assert_eq!(db.diary_get(a).unwrap(), None);
        assert!(!db.diary_update(a, "x", "manual", 10).unwrap());
    }
}
