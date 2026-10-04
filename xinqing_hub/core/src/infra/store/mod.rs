//! SQLite 存储（09 第 3 节）：WAL 模式、按版本号递增执行迁移、结构约束检查。
//!
//! 当前是同步实现；17 第 2.9 节的单写线程 `DbWriter` 在接入 Tauri 时包在外面。

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};

use crate::domain::features::WindowFeatures;
use crate::domain::rules::Hints;
use crate::infra::gateway::NetLogEntry;
use crate::infra::templates::AppCat;

/// `net_log` 只保留最近这么多条（09 D-20）。
pub const NET_LOG_KEEP: i64 = 200;

/// 按顺序执行的迁移脚本：`(版本号, SQL)`。已发布的脚本禁止修改。
const MIGRATIONS: &[(i64, &str)] = &[(1, include_str!("../../../migrations/0001_init.sql"))];

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("数据库错误：{0}")]
    Sql(#[from] rusqlite::Error),
    #[error("features_json 只允许数值和 null（09 第 3 节结构约束）")]
    NonNumericFeatures,
    #[error("序列化失败：{0}")]
    Json(#[from] serde_json::Error),
}

pub struct Db {
    conn: Connection,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self, StoreError> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self, StoreError> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    pub fn schema_version(&self) -> Result<i64, StoreError> {
        Ok(self
            .conn
            .query_row("SELECT max(version) FROM schema_version", [], |r| {
                r.get::<_, Option<i64>>(0)
            })?
            .unwrap_or(0))
    }

    fn migrate(&self) -> Result<(), StoreError> {
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL)",
            [],
        )?;
        let current = self.schema_version()?;
        for (ver, sql) in MIGRATIONS.iter().filter(|(v, _)| *v > current) {
            let tx = self.conn.unchecked_transaction()?;
            tx.execute_batch(sql)?;
            tx.execute("INSERT INTO schema_version (version) VALUES (?1)", [ver])?;
            tx.commit()?;
        }
        Ok(())
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// 写入一个特征窗口；写入前检查 JSON 只含数值和 null。
    pub fn insert_window(
        &self,
        start_ts: i64,
        end_ts: i64,
        app_cat: AppCat,
        f: &WindowFeatures,
        hints: &Hints,
    ) -> Result<i64, StoreError> {
        let v = serde_json::to_value(f)?;
        insert_window_json(&self.conn, start_ts, end_ts, app_cat, &v, &hints.joined())
    }

    pub fn settings_get(&self, key: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .conn
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .optional()?)
    }

    pub fn settings_set(&self, key: &str, value: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// 记录一次同意或撤回（09 第 2.2 节）。
    pub fn consent_set(
        &self,
        item: &str,
        policy_ver: i64,
        granted: bool,
        ts: i64,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO consent (item, policy_ver, granted, ts) VALUES (?1, ?2, ?3, ?4)",
            params![item, policy_ver, granted as i64, ts],
        )?;
        Ok(())
    }

    /// 某同意项的最新状态；要求与当前隐私说明版本一致，版本升级后视为未同意（FR-ONB-01）。
    pub fn consent_granted(&self, item: &str, policy_ver: i64) -> Result<bool, StoreError> {
        let row: Option<(i64, i64)> = self
            .conn
            .query_row(
                "SELECT granted, policy_ver FROM consent WHERE item = ?1 ORDER BY ts DESC, id DESC LIMIT 1",
                [item],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        Ok(matches!(row, Some((1, v)) if v == policy_ver))
    }

    /// `safety_log` 只写时间和通道，不存内容（D-18）。
    pub fn safety_log(&self, ts: i64, channel: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO safety_log (ts, channel) VALUES (?1, ?2)",
            params![ts, channel],
        )?;
        Ok(())
    }

    /// 写一条出网记录，并删掉最近 [`NET_LOG_KEEP`] 条以外的旧记录。
    pub fn net_log_insert(&self, e: &NetLogEntry) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO net_log (ts, api, model, fields, latency_ms, status, tokens_in, tokens_out)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                e.ts,
                e.api,
                e.model,
                serde_json::to_string(&e.fields)?,
                e.latency_ms.map(|v| v as i64),
                e.status,
                e.tokens_in,
                e.tokens_out
            ],
        )?;
        self.conn.execute(
            "DELETE FROM net_log WHERE id NOT IN (SELECT id FROM net_log ORDER BY id DESC LIMIT ?1)",
            [NET_LOG_KEEP],
        )?;
        Ok(())
    }

    /// 最近的出网记录，新的在前（设置页隐私分类显示 20 条，FR-SET-09）。
    pub fn net_log_recent(&self, limit: usize) -> Result<Vec<NetLogEntry>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT ts, api, model, fields, latency_ms, status, tokens_in, tokens_out
             FROM net_log ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit as i64], |r| {
            Ok((
                NetLogEntry {
                    ts: r.get(0)?,
                    api: r.get(1)?,
                    model: r.get(2)?,
                    fields: Vec::new(),
                    latency_ms: r.get::<_, Option<i64>>(4)?.map(|v| v as u64),
                    status: r.get(5)?,
                    tokens_in: r.get(6)?,
                    tokens_out: r.get(7)?,
                },
                r.get::<_, String>(3)?,
            ))
        })?;
        rows.map(|row| {
            let (mut e, fields) = row?;
            e.fields = serde_json::from_str(&fields)?;
            Ok(e)
        })
        .collect()
    }
}

fn numeric_only(v: &serde_json::Value) -> bool {
    match v {
        serde_json::Value::Object(m) => m.values().all(|x| x.is_number() || x.is_null()),
        _ => false,
    }
}

/// 结构约束检查后写入（单独暴露便于测试“拒绝写入文本”）。
pub fn insert_window_json(
    conn: &Connection,
    start_ts: i64,
    end_ts: i64,
    app_cat: AppCat,
    features: &serde_json::Value,
    hints: &str,
) -> Result<i64, StoreError> {
    if !numeric_only(features) {
        return Err(StoreError::NonNumericFeatures);
    }
    conn.execute(
        "INSERT INTO window_features (start_ts, end_ts, app_cat, features_json, hints)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            start_ts,
            end_ts,
            app_cat.as_str(),
            features.to_string(),
            hints
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_apply_once() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(db.schema_version().unwrap(), 1);
        db.migrate().unwrap();
        let n: i64 = db
            .conn()
            .query_row("SELECT count(*) FROM schema_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn window_features_reject_text() {
        let db = Db::open_in_memory().unwrap();
        let ok = serde_json::json!({"kpm": 200.0, "dwell_med": null});
        insert_window_json(db.conn(), 0, 1, AppCat::Chat, &ok, "").unwrap();
        let bad = serde_json::json!({"kpm": 200.0, "text": "你好"});
        assert!(matches!(
            insert_window_json(db.conn(), 0, 1, AppCat::Chat, &bad, ""),
            Err(StoreError::NonNumericFeatures)
        ));
        let id = db
            .insert_window(
                0,
                1,
                AppCat::Other,
                &WindowFeatures::default(),
                &Hints::default(),
            )
            .unwrap();
        assert!(id > 0);
    }

    #[test]
    fn settings_and_consent() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(db.settings_get("ui.theme").unwrap(), None);
        db.settings_set("ui.theme", "dark").unwrap();
        db.settings_set("ui.theme", "light").unwrap();
        assert_eq!(
            db.settings_get("ui.theme").unwrap().as_deref(),
            Some("light")
        );

        assert!(!db.consent_granted("sense", 1).unwrap());
        db.consent_set("sense", 1, true, 100).unwrap();
        assert!(db.consent_granted("sense", 1).unwrap());
        assert!(
            !db.consent_granted("sense", 2).unwrap(),
            "隐私说明升级后需重新同意"
        );
        db.consent_set("sense", 1, false, 200).unwrap();
        assert!(!db.consent_granted("sense", 1).unwrap());
    }

    #[test]
    fn net_log_keeps_latest_entries_only() {
        let db = Db::open_in_memory().unwrap();
        let entry = |ts| NetLogEntry {
            ts,
            api: "jev".into(),
            model: Some("jev-latest".into()),
            fields: vec!["kpm".into(), "iki_med".into()],
            latency_ms: Some(120),
            status: "ok".into(),
            tokens_in: None,
            tokens_out: None,
        };
        for ts in 0..NET_LOG_KEEP + 5 {
            db.net_log_insert(&entry(ts)).unwrap();
        }
        let n: i64 = db
            .conn()
            .query_row("SELECT count(*) FROM net_log", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, NET_LOG_KEEP);
        let recent = db.net_log_recent(20).unwrap();
        assert_eq!(recent.len(), 20);
        assert_eq!(recent[0], entry(NET_LOG_KEEP + 4), "新的在前");
    }

    #[test]
    fn safety_log_rejects_unknown_channel() {
        let db = Db::open_in_memory().unwrap();
        db.safety_log(1, "lexicon").unwrap();
        assert!(db.safety_log(2, "something").is_err());
    }
}
