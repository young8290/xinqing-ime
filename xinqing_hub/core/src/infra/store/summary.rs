//! `daily_summary`（09 D-08）里由使用计时与休息提醒维护的列：`typing_min`、`last_active_ts`（D-31）、
//! `rests_due`、`rests_done`、`water`（FR-RST-08、FR-REV-03）。其余列由各自的汇总写入。

use rusqlite::params;

use super::{Db, StoreError};

impl Db {
    /// 记一个活跃分钟（FR-RST-01）：`date` 当天 `typing_min` 加 1；`evening` 为 18:00–次日 06:00 所属的
    /// “当晚”日期时，把它的 `last_active_ts` 推到 `ts`（只往后推）。日期都是本地 `YYYY-MM-DD`。
    pub fn summary_active_minute(
        &self,
        date: &str,
        evening: Option<&str>,
        ts: i64,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO daily_summary (date, typing_min) VALUES (?1, 1)
             ON CONFLICT(date) DO UPDATE SET typing_min = typing_min + 1",
            [date],
        )?;
        if let Some(evening) = evening {
            self.conn.execute(
                "INSERT INTO daily_summary (date, last_active_ts) VALUES (?1, ?2)
                 ON CONFLICT(date) DO UPDATE SET last_active_ts =
                   max(coalesce(last_active_ts, excluded.last_active_ts), excluded.last_active_ts)",
                params![evening, ts],
            )?;
        }
        Ok(())
    }

    /// 休息提醒计数：显示一次 `rests_due` 加 1；点“已完成”`rests_done` 加 1，喝水提醒的完成同时记 `water`。
    pub fn summary_rest(&self, date: &str, shown: bool, water: bool) -> Result<(), StoreError> {
        let sql = match (shown, water) {
            (true, _) => {
                "INSERT INTO daily_summary (date, rests_due) VALUES (?1, 1)
                 ON CONFLICT(date) DO UPDATE SET rests_due = rests_due + 1"
            }
            (false, false) => {
                "INSERT INTO daily_summary (date, rests_done) VALUES (?1, 1)
                 ON CONFLICT(date) DO UPDATE SET rests_done = rests_done + 1"
            }
            (false, true) => {
                "INSERT INTO daily_summary (date, rests_done, water) VALUES (?1, 1, 1)
                 ON CONFLICT(date) DO UPDATE SET rests_done = rests_done + 1, water = water + 1"
            }
        };
        self.conn.execute(sql, [date])?;
        Ok(())
    }

    /// `from`..=`to`（本地日期）各晚的停止打字时间，只返回有记录的日期，按日期升序。
    pub fn summary_stop_times(
        &self,
        from: &str,
        to: &str,
    ) -> Result<Vec<(String, i64)>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT date, last_active_ts FROM daily_summary
             WHERE date BETWEEN ?1 AND ?2 AND last_active_ts IS NOT NULL ORDER BY date",
        )?;
        let rows = stmt
            .query_map([from, to], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// 一天的 (`typing_min`, `rests_due`, `rests_done`, `water`)；没有这一天时为 `None`。
    pub fn summary_counts(&self, date: &str) -> Result<Option<(u32, u32, u32, u32)>, StoreError> {
        use rusqlite::OptionalExtension;
        Ok(self
            .conn
            .query_row(
                "SELECT typing_min, rests_due, rests_done, water FROM daily_summary WHERE date = ?1",
                [date],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_minutes_count_and_stop_time_only_moves_later() {
        let db = Db::open_in_memory().unwrap();
        db.summary_active_minute("2026-10-05", None, 100).unwrap();
        db.summary_active_minute("2026-10-05", Some("2026-10-05"), 300)
            .unwrap();
        // 次日凌晨的分钟算到前一晚
        db.summary_active_minute("2026-10-06", Some("2026-10-05"), 500)
            .unwrap();
        // 时钟回拨时不把停止时间往前拉
        db.summary_active_minute("2026-10-06", Some("2026-10-05"), 400)
            .unwrap();
        assert_eq!(db.summary_counts("2026-10-05").unwrap(), Some((2, 0, 0, 0)));
        assert_eq!(db.summary_counts("2026-10-06").unwrap(), Some((2, 0, 0, 0)));
        assert_eq!(
            db.summary_stop_times("2026-10-01", "2026-10-06").unwrap(),
            vec![("2026-10-05".to_string(), 500)]
        );
    }

    #[test]
    fn rest_counts() {
        let db = Db::open_in_memory().unwrap();
        db.summary_rest("2026-10-05", true, false).unwrap();
        db.summary_rest("2026-10-05", true, true).unwrap();
        db.summary_rest("2026-10-05", false, false).unwrap();
        db.summary_rest("2026-10-05", false, true).unwrap();
        assert_eq!(db.summary_counts("2026-10-05").unwrap(), Some((0, 2, 2, 1)));
        assert_eq!(db.summary_counts("2026-10-04").unwrap(), None);
    }
}
