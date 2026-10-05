//! SQLite 存储（09 第 3 节）：WAL 模式、按版本号递增执行迁移、结构约束检查。
//!
//! 当前是同步实现；17 第 2.9 节的单写线程 `DbWriter` 在接入 Tauri 时包在外面。

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

use xqp::MoodState;

use crate::domain::explain::{self, Evidence, ExplainSource, Explanation};
use crate::domain::features::{Baseline, BaselineRow, Bucket, WindowFeatures};
use crate::domain::fusion::Source;
use crate::domain::rules::Hints;
use crate::infra::gateway::NetLogEntry;
use crate::infra::templates::AppCat;

mod comfort;
pub mod export;
mod rest;
mod schedule;
pub use comfort::ComfortRecord;
pub use schedule::{ScheduleRow, TodoRow};

/// `net_log` 只保留最近这么多条（09 D-20）。
pub const NET_LOG_KEEP: i64 = 200;

/// 按顺序执行的迁移脚本：`(版本号, SQL)`。已发布的脚本禁止修改。
const MIGRATIONS: &[(i64, &str)] = &[
    (1, include_str!("../../../migrations/0001_init.sql")),
    (
        2,
        include_str!("../../../migrations/0002_comfort_trigger.sql"),
    ),
];

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("数据库错误：{0}")]
    Sql(#[from] rusqlite::Error),
    #[error("features_json 只允许数值和 null（09 第 3 节结构约束）")]
    NonNumericFeatures,
    #[error("序列化失败：{0}")]
    Json(#[from] serde_json::Error),
    #[error("日程字段无效")]
    InvalidSchedule,
    #[error("待办字段无效")]
    InvalidTodo,
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
        // 只对还没有表的新库生效，让 FR-DAT-02 清理后的增量 VACUUM 能回收空间（docs/adr/0013 第 6 条）
        conn.pragma_update(None, "auto_vacuum", "INCREMENTAL")?;
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

    /// 写一条状态记录（D-07）。`state` 是本窗口的候选状态，`shown` 是融合后实际显示的状态；
    /// 只有本地规则时 `source = rule`，Jev 的概率、效价等列留空。
    pub fn insert_mood_state(
        &self,
        ts: i64,
        window_id: Option<i64>,
        state: MoodState,
        shown: MoodState,
        source: Source,
    ) -> Result<i64, StoreError> {
        let source = match source {
            Source::Jev => "jev",
            Source::Rule => "rule",
        };
        self.conn.execute(
            "INSERT INTO mood_state (ts, window_id, state, shown_state, source) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![ts, window_id, state.as_str(), shown.as_str(), source],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// 一个已存特征窗口的特征和规则提示。
    fn window_evidence(&self, id: i64) -> Result<Option<(WindowFeatures, Hints)>, StoreError> {
        let row: Option<(String, String)> = self
            .conn
            .query_row(
                "SELECT features_json, hints FROM window_features WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        row.map(|(json, hints)| Ok((serde_json::from_str(&json)?, Hints::parse(&hints))))
            .transpose()
    }

    /// 看板时间线上某个状态点的解释（`state_explain(mood_state_id)`，FR-STA-09）。
    ///
    /// 不另存解释：从 `mood_state.window_id` 找回那个窗口和它的上一个窗口，重新生成一次。
    /// 冷启动按“该窗口之前 7 天内（重置基线之后）的窗口数”判断，与实时路径一致；`{p}` 用传入的（当前）基线中位数。
    /// 记录不存在或没有关联窗口（窗口已被清理）时返回 `None`。
    pub fn explain_mood_state(
        &self,
        mood_state_id: i64,
        baseline: &Baseline,
    ) -> Result<Option<Explanation>, StoreError> {
        let row: Option<(Option<i64>, String, String, Option<String>)> = self
            .conn
            .query_row(
                "SELECT window_id, shown_state, source, probs_json FROM mood_state WHERE id = ?1",
                [mood_state_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        let Some((Some(window_id), shown, source, probs)) = row else {
            return Ok(None);
        };
        let Some(shown) = parse_state(&shown) else {
            return Ok(None);
        };
        let Some(cur) = self.window_evidence(window_id)? else {
            return Ok(None);
        };
        let prev_id: Option<i64> = self.conn.query_row(
            "SELECT max(id) FROM window_features WHERE id < ?1",
            [window_id],
            |r| r.get(0),
        )?;
        let prev = match prev_id {
            Some(id) => self.window_evidence(id)?,
            None => None,
        };
        // 与实时路径同一口径：该窗口之前 7 天内（且在上次重置基线之后）的窗口数
        let reset: i64 = self
            .settings_get(crate::domain::features::persist::RESET_KEY)?
            .and_then(|v| v.parse().ok())
            .unwrap_or(i64::MIN);
        let windows: i64 = self.conn.query_row(
            "SELECT count(*) FROM window_features w, window_features cur
             WHERE cur.id = ?1 AND w.id <= cur.id
               AND w.end_ts >= max(cur.end_ts - ?2, ?3)",
            params![
                window_id,
                crate::domain::features::persist::WINDOW_MS,
                reset
            ],
            |r| r.get(0),
        )?;
        let mut baseline = baseline.clone();
        baseline.windows = u32::try_from(windows).unwrap_or(u32::MAX);
        let source = if source == "jev" {
            ExplainSource::Jev
        } else {
            ExplainSource::Rule
        };
        let prob = probs
            .and_then(|j| {
                serde_json::from_str::<std::collections::HashMap<MoodState, f64>>(&j).ok()
            })
            .and_then(|m| m.get(&shown).copied());
        Ok(Some(explain::build(
            shown,
            prob,
            source,
            Evidence {
                features: &cur.0,
                hints: &cur.1,
            },
            prev.as_ref()
                .map(|(features, hints)| Evidence { features, hints }),
            &baseline,
        )))
    }

    /// 最近一条状态记录的 id（小组件对“当前状态”的反馈记到这条上，FR-STA-07）。
    pub fn latest_mood_state_id(&self) -> Result<Option<i64>, StoreError> {
        Ok(self
            .conn
            .query_row("SELECT max(id) FROM mood_state", [], |r| r.get(0))?)
    }

    /// 一条状态记录当时显示的状态。
    pub fn mood_shown_state(&self, id: i64) -> Result<Option<MoodState>, StoreError> {
        let shown: Option<String> = self
            .conn
            .query_row(
                "SELECT shown_state FROM mood_state WHERE id = ?1",
                [id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(shown.as_deref().and_then(parse_state))
    }

    /// 写一条反馈（D-17）。
    pub fn insert_feedback(
        &self,
        ts: i64,
        target: &str,
        target_id: Option<i64>,
        verdict: &str,
    ) -> Result<i64, StoreError> {
        self.conn.execute(
            "INSERT INTO feedback (ts, target, target_id, verdict) VALUES (?1, ?2, ?3, ?4)",
            params![ts, target, target_id, verdict],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// `since` 之后对状态记录的某种反馈，连同被评价的显示状态，按时间先后。
    /// 关联的状态记录已被清理的反馈跳过。
    pub fn unfit_since(
        &self,
        target: &str,
        verdict: &str,
        since: i64,
    ) -> Result<Vec<(MoodState, i64)>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT m.shown_state, f.ts FROM feedback f JOIN mood_state m ON m.id = f.target_id
             WHERE f.target = ?1 AND f.verdict = ?2 AND f.ts >= ?3 ORDER BY f.ts, f.id",
        )?;
        let rows = stmt.query_map(params![target, verdict, since], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (state, ts) = row?;
            if let Some(s) = parse_state(&state) {
                out.push((s, ts));
            }
        }
        Ok(out)
    }

    /// 写一条自评（D-24）。备注只存本地。
    pub fn insert_self_report(
        &self,
        ts: i64,
        weather: &str,
        note: Option<&str>,
        auto_state: Option<&str>,
        source: &str,
    ) -> Result<i64, StoreError> {
        self.conn.execute(
            "INSERT INTO self_report (ts, weather, note, auto_state, source) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![ts, weather, note, auto_state, source],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// 最近的 `limit` 条自评，新的在前。
    pub fn self_reports_recent(
        &self,
        limit: usize,
    ) -> Result<Vec<crate::domain::self_report::SelfReportRow>, StoreError> {
        self.self_reports_query(
            "SELECT id, ts, weather, note, auto_state FROM self_report ORDER BY ts DESC, id DESC LIMIT ?1",
            params![limit as i64],
        )
    }

    /// `[start, end)` 内的自评，按时间先后。
    pub fn self_reports_between(
        &self,
        start: i64,
        end: i64,
    ) -> Result<Vec<crate::domain::self_report::SelfReportRow>, StoreError> {
        self.self_reports_query(
            "SELECT id, ts, weather, note, auto_state FROM self_report
             WHERE ts >= ?1 AND ts < ?2 ORDER BY ts, id",
            params![start, end],
        )
    }

    fn self_reports_query(
        &self,
        sql: &str,
        p: impl rusqlite::Params,
    ) -> Result<Vec<crate::domain::self_report::SelfReportRow>, StoreError> {
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map(p, |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// `end_ts >= since` 的窗口特征（基线重算用），按时间先后。
    pub fn window_features_since(&self, since: i64) -> Result<Vec<WindowFeatures>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT features_json FROM window_features WHERE end_ts >= ?1 ORDER BY end_ts, id",
        )?;
        let rows = stmt.query_map([since], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for json in rows {
            out.push(serde_json::from_str(&json?)?);
        }
        Ok(out)
    }

    /// 用一次重算的结果整体替换 `baseline` 表（D-09）。
    pub fn baseline_replace(
        &self,
        rows: &[BaselineRow],
        updated_ts: i64,
    ) -> Result<(), StoreError> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM baseline", [])?;
        for r in rows {
            tx.execute(
                "INSERT INTO baseline (feature, bucket, med, mad, n, updated_ts) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![r.feature, r.bucket.as_str(), r.value.med, r.value.mad, r.n, updated_ts],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// 读出 `baseline` 表；不认识的特征或时段跳过。
    pub fn baseline_load(&self) -> Result<Vec<BaselineRow>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT feature, bucket, med, mad, n FROM baseline ORDER BY bucket, feature",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, f64>(2)?,
                r.get::<_, f64>(3)?,
                r.get::<_, u32>(4)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (feature, bucket, med, mad, n) = row?;
            let Some(feature) = crate::domain::features::baseline::BASE_FEATURES
                .into_iter()
                .find(|f| *f == feature)
            else {
                continue;
            };
            let bucket = match bucket.as_str() {
                "day" => Bucket::Day,
                "night" => Bucket::Night,
                _ => continue,
            };
            out.push(BaselineRow {
                bucket,
                feature,
                value: crate::infra::templates::MedMad { med, mad },
                n,
            });
        }
        Ok(out)
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

/// 库里的状态名（`mood_state.state` / `shown_state`）转回枚举，不认识的为 `None`。
fn parse_state(s: &str) -> Option<MoodState> {
    serde_json::from_value(serde_json::Value::String(s.to_string())).ok()
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
        assert_eq!(db.schema_version().unwrap(), MIGRATIONS.len() as i64);
        db.migrate().unwrap();
        let n: i64 = db
            .conn()
            .query_row("SELECT count(*) FROM schema_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, MIGRATIONS.len() as i64);
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
    fn mood_state_links_to_window() {
        let db = Db::open_in_memory().unwrap();
        let w = db
            .insert_window(
                0,
                1,
                AppCat::Chat,
                &WindowFeatures::default(),
                &Hints::default(),
            )
            .unwrap();
        db.insert_mood_state(
            1,
            Some(w),
            MoodState::Hesitant,
            MoodState::Fluent,
            Source::Rule,
        )
        .unwrap();
        let row: (i64, String, String, String) = db
            .conn()
            .query_row(
                "SELECT window_id, state, shown_state, source FROM mood_state",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(row, (w, "hesitant".into(), "fluent".into(), "rule".into()));
    }

    #[test]
    fn explain_stored_mood_state() {
        use crate::domain::explain::SignalKind;
        use crate::domain::rules::Hint;
        use crate::infra::templates::{BaselineDefault, TemplateDirs};

        let dirs = TemplateDirs::factory_only(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../hub_templates"),
        );
        let base = Baseline::from_defaults(&BaselineDefault::load(&dirs).unwrap());
        let db = Db::open_in_memory().unwrap();
        let prev = WindowFeatures {
            abandon: 1,
            ..Default::default()
        };
        let cur = WindowFeatures {
            pause_cnt: 3,
            kpm: Some(120.0),
            kpm_z: Some(0.2),
            ..Default::default()
        };
        let h = Hints(vec![Hint::HesitationHint]);
        let w1 = db.insert_window(0, 1, AppCat::Chat, &prev, &h).unwrap();
        let w2 = db.insert_window(2, 3, AppCat::Chat, &cur, &h).unwrap();
        db.insert_mood_state(
            1,
            Some(w1),
            MoodState::Hesitant,
            MoodState::Fluent,
            Source::Rule,
        )
        .unwrap();
        let id = db
            .insert_mood_state(
                3,
                Some(w2),
                MoodState::Hesitant,
                MoodState::Hesitant,
                Source::Rule,
            )
            .unwrap();

        let e = db.explain_mood_state(id, &base).unwrap().unwrap();
        assert_eq!(e.state, MoodState::Hesitant);
        assert_eq!(e.source, ExplainSource::Rule);
        assert_eq!(e.prob_pct, None);
        assert!(e.cold_start);
        let kinds: Vec<_> = e.signals.iter().map(|s| (s.kind, s.value)).collect();
        assert_eq!(
            kinds,
            vec![(SignalKind::Pause, Some(3)), (SignalKind::Abandon, None)],
            "停顿来自触发窗口，“打了又删”来自上一个窗口"
        );

        // Jev 记录带概率
        db.conn()
            .execute(
                "UPDATE mood_state SET source = 'jev', probs_json = '{\"hesitant\":0.83}' WHERE id = ?1",
                [id],
            )
            .unwrap();
        let e = db.explain_mood_state(id, &base).unwrap().unwrap();
        assert_eq!((e.source, e.prob_pct), (ExplainSource::Jev, Some(83)));

        // 不存在、没有关联窗口时为 None
        assert_eq!(db.explain_mood_state(999, &base).unwrap(), None);
        let orphan = db
            .insert_mood_state(4, None, MoodState::Low, MoodState::Low, Source::Rule)
            .unwrap();
        assert_eq!(db.explain_mood_state(orphan, &base).unwrap(), None);
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
