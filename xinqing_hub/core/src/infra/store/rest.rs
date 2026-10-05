//! `reminder_log`（09 D-16）的写入：每次用户对休息提醒卡片的操作记一条（FR-RST-08）。

use rusqlite::params;

use super::{Db, StoreError};

impl Db {
    /// `kind` 为 `eye` / `water` / `move` / `night`，`action` 为 `done` / `later` / `today_off`。
    pub fn reminder_insert(&self, ts: i64, kind: &str, action: &str) -> Result<i64, StoreError> {
        self.conn.execute(
            "INSERT INTO reminder_log (ts, kind, action) VALUES (?1, ?2, ?3)",
            params![ts, kind, action],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// `since_ms` 之后每种操作的次数，按 `action` 排序（看板周报的休息完成率，FR-RST-08）。
    pub fn reminder_counts(&self, since_ms: i64) -> Result<Vec<(String, u32)>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT action, COUNT(*) FROM reminder_log WHERE ts >= ?1 GROUP BY action ORDER BY action",
        )?;
        let rows = stmt
            .query_map([since_ms], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_actions_since() {
        // TC-RST-10：完成 3 次、稍后 1 次、关闭 1 次 → 5 条，完成率 3/5
        let db = Db::open_in_memory().unwrap();
        for (ts, action) in [
            (10, "done"),
            (20, "done"),
            (30, "later"),
            (40, "done"),
            (50, "today_off"),
        ] {
            db.reminder_insert(ts, "eye", action).unwrap();
        }
        db.reminder_insert(5, "water", "done").unwrap();
        assert_eq!(
            db.reminder_counts(10).unwrap(),
            vec![
                ("done".to_string(), 3),
                ("later".to_string(), 1),
                ("today_off".to_string(), 1)
            ]
        );
    }
}
