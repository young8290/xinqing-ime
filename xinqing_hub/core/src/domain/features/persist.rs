//! 个人基线的持久化（FR-STA-03 第 4、5 条，09 D-09）：Hub 启动时和每天 04:00 用最近 7 天的窗口特征重算，
//! 结果写入 `baseline` 表；“重置基线”后只用重置以后的窗口，重新进入冷启动。
//!
//! 基线表只存统计值（中位数、MAD、样本数），不存原始窗口；窗口特征本身在 `window_features`（D-06，30 天）。

use super::baseline::{BaselineStats, compute_stats};
use crate::infra::store::{Db, StoreError};

/// 重算用最近多久的窗口（滚动 7 天）。
pub const WINDOW_MS: i64 = 7 * 24 * 3_600_000;

/// 记录上次“重置基线”时间的内部键（Unix 毫秒）。它不是用户设置，不在设置键注册表里，
/// 只借用 `settings` 表存放，登记在 `settings::INTERNAL_KEYS`，导出导入不带它（ADR 0014）。
pub const RESET_KEY: &str = "baseline.reset_ts";

fn reset_ts(db: &Db) -> Result<i64, StoreError> {
    Ok(db
        .settings_get(RESET_KEY)?
        .and_then(|v| v.parse().ok())
        .unwrap_or(i64::MIN))
}

/// 用 `now_ms` 之前 7 天内（且在上次重置之后）的窗口重算基线，写入 `baseline` 表并返回结果。
pub fn recompute(db: &Db, now_ms: i64) -> Result<BaselineStats, StoreError> {
    let since = (now_ms - WINDOW_MS).max(reset_ts(db)?);
    let windows = db.window_features_since(since)?;
    let stats = compute_stats(&windows);
    db.baseline_replace(&stats.rows, now_ms)?;
    Ok(stats)
}

/// 重置基线（设置页“重置基线”，FR-SET-04）：清空个人统计值，之后只用重置以后的窗口。
pub fn reset(db: &Db, now_ms: i64) -> Result<BaselineStats, StoreError> {
    db.settings_set(RESET_KEY, &now_ms.to_string())?;
    db.baseline_replace(&[], now_ms)?;
    Ok(BaselineStats::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::features::{Bucket, WindowFeatures};
    use crate::domain::rules::Hints;
    use crate::infra::templates::AppCat;

    fn put(db: &Db, end_ms: i64, kpm: f64) {
        let f = WindowFeatures {
            hour: 14,
            kpm: Some(kpm),
            ..Default::default()
        };
        db.insert_window(end_ms - 1, end_ms, AppCat::Chat, &f, &Hints::default())
            .unwrap();
    }

    #[test]
    fn recompute_uses_last_seven_days_and_writes_the_table() {
        let db = Db::open_in_memory().unwrap();
        let now = 10 * 24 * 3_600_000;
        // 8 天前的 50 个窗口不参与
        for i in 0..50 {
            put(&db, now - 8 * 24 * 3_600_000 + i, 999.0);
        }
        for i in 0..40 {
            put(&db, now - 1_000 - i, 100.0 + i as f64);
        }
        let stats = recompute(&db, now).unwrap();
        assert_eq!(stats.windows, 40);
        let rows = db.baseline_load().unwrap();
        let kpm = rows
            .iter()
            .find(|r| r.bucket == Bucket::Day && r.feature == "kpm")
            .unwrap();
        assert_eq!((kpm.value.med, kpm.n), (119.5, 40));
        assert_eq!(rows.len(), stats.rows.len());
    }

    #[test]
    fn reset_starts_over() {
        let db = Db::open_in_memory().unwrap();
        let now = 10 * 24 * 3_600_000;
        for i in 0..40 {
            put(&db, now - 10_000 + i, 120.0);
        }
        assert_eq!(recompute(&db, now).unwrap().windows, 40);
        assert_eq!(reset(&db, now).unwrap(), BaselineStats::default());
        assert!(db.baseline_load().unwrap().is_empty());
        // 重置以后的窗口才算
        for i in 0..3 {
            put(&db, now + 1_000 + i, 120.0);
        }
        assert_eq!(recompute(&db, now + 5_000).unwrap().windows, 3);
    }
}
