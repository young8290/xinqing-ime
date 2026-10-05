//! 演示数据库的预置写入（FR-DMO-03 第 2 条，B-10）。只给 `domain::demo::seed` 用。

use rusqlite::params;

use super::{Db, StoreError};

/// 预置一天的 `daily_summary` 计时列（与 `routine::record_*` 写的列相同）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SummarySeed<'a> {
    pub date: &'a str,
    pub typing_min: u32,
    pub rests_due: u32,
    pub rests_done: u32,
    pub water: u32,
    /// 这一晚的停止打字时间（Unix 毫秒），写在当晚日期这一行
    pub last_active_ts: Option<i64>,
}

impl Db {
    /// 在一个事务里执行 `f`，失败时整体回滚。`f` 里不能再开事务（例如 `baseline_replace`）。
    pub fn seed_tx<T>(
        &self,
        f: impl FnOnce(&Db) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        let tx = self.conn.unchecked_transaction()?;
        let out = f(self)?;
        tx.commit()?;
        Ok(out)
    }

    pub fn summary_seed(&self, s: &SummarySeed<'_>) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO daily_summary (date, typing_min, rests_due, rests_done, water, last_active_ts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(date) DO UPDATE SET typing_min = excluded.typing_min,
               rests_due = excluded.rests_due, rests_done = excluded.rests_done,
               water = excluded.water, last_active_ts = excluded.last_active_ts",
            params![
                s.date,
                s.typing_min,
                s.rests_due,
                s.rests_done,
                s.water,
                s.last_active_ts
            ],
        )?;
        Ok(())
    }
}
