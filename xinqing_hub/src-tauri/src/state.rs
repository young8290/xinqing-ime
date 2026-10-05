//! 外壳持有的共享状态。数据库开两个连接（17 第 2.9 节，ADR 0020）：写连接交给 [`DbWriter`]，所有写入都经过它；
//! [`AppState::db`] 是只读连接，debug 构建下写入会报错。

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, RwLock};

use xinqing_hub_core::domain::consent::ConsentState;
use xinqing_hub_core::domain::explain::Explanation;
use xinqing_hub_core::domain::settings;
use xinqing_hub_core::domain::status::StatusSnapshot;
use xinqing_hub_core::infra::store::{Db, DbWriter, StoreError};
use xqp::MoodState;

use crate::error::UiError;

pub const DB_FILE: &str = "xinqing.db";

pub struct AppState {
    /// 只读连接
    db: Mutex<Db>,
    writer: DbWriter,
    pub status: RwLock<StatusSnapshot>,
    /// 最近一次状态切换的解释（FR-STA-09），由感知任务写入，缓存到下一次切换。
    explanation: RwLock<Option<Explanation>>,
    /// 自动判断最近一个窗口的显示状态（自评期间也照常更新），自评时记入 `self_report.auto_state`。
    auto_state: RwLock<Option<MoodState>>,
    /// 本次启动时数据库损坏并已重建（FR-DAT-01）。界面提示 `error.db_rebuilt` 随小组件一句话区（D-02）接入
    #[allow(dead_code)]
    pub db_rebuilt: bool,
}

impl AppState {
    pub fn init(data_dir: &Path) -> anyhow::Result<Self> {
        std::fs::create_dir_all(data_dir)?;
        let path = data_dir.join(DB_FILE);
        let (db, db_rebuilt) = open_or_rebuild(&path)?;
        Ok(Self {
            db: Mutex::new(Db::open_reader(&path)?),
            writer: DbWriter::new(db),
            status: RwLock::new(StatusSnapshot::default()),
            explanation: RwLock::new(None),
            auto_state: RwLock::new(None),
            db_rebuilt,
        })
    }

    /// 只读连接。写入一律走 [`AppState::writer`]。
    pub fn db(&self) -> MutexGuard<'_, Db> {
        // 持锁线程 panic 后数据库连接本身仍可用，不让一次 panic 拖垮之后所有命令
        self.db.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 写入口：用户确认类数据用 `write_sync`，普通数据用 `enqueue`。
    pub fn writer(&self) -> &DbWriter {
        &self.writer
    }

    pub fn status(&self) -> StatusSnapshot {
        self.status
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// 修改状态快照，返回修改后的快照及是否有变化。
    pub fn update_status(
        &self,
        f: impl FnOnce(&mut StatusSnapshot) -> bool,
    ) -> (StatusSnapshot, bool) {
        let mut s = self.status.write().unwrap_or_else(|e| e.into_inner());
        let changed = f(&mut s);
        (s.clone(), changed)
    }

    pub fn explanation(&self) -> Option<Explanation> {
        self.explanation
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn set_explanation(&self, e: Explanation) {
        *self.explanation.write().unwrap_or_else(|e| e.into_inner()) = Some(e);
    }

    pub fn auto_state(&self) -> Option<MoodState> {
        *self.auto_state.read().unwrap_or_else(|e| e.into_inner())
    }

    pub fn set_auto_state(&self, s: MoodState) {
        *self.auto_state.write().unwrap_or_else(|e| e.into_inner()) = Some(s);
    }

    pub fn needs_onboarding(&self) -> Result<bool, UiError> {
        Ok(ConsentState::load(&self.db())?.needs_onboarding())
    }

    pub fn widget_visible(&self) -> Result<bool, UiError> {
        Ok(settings::get(&self.db(), "widget.visible")?
            .as_bool()
            .unwrap_or(true))
    }
}

/// 打开数据库；打不开或迁移失败时把损坏的文件改名备份，再新建一个（FR-DAT-01）。
fn open_or_rebuild(path: &Path) -> anyhow::Result<(Db, bool)> {
    match Db::open(path) {
        Ok(db) => Ok((db, false)),
        Err(e @ StoreError::Sql(_)) if path.exists() => {
            let backup = backup_path(path);
            eprintln!(
                "数据库无法打开（{e}），已备份到 {} 并重建",
                backup.display()
            );
            std::fs::rename(path, &backup)?;
            for ext in ["-wal", "-shm"] {
                let side = PathBuf::from(format!("{}{ext}", path.display()));
                let _ = std::fs::remove_file(side);
            }
            Ok((Db::open(path)?, true))
        }
        Err(e) => Err(e.into()),
    }
}

fn backup_path(path: &Path) -> PathBuf {
    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
    path.with_file_name(format!("{DB_FILE}.corrupt-{ts}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("xq-hub-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn fresh_dir_creates_database() {
        let d = tmp_dir("fresh");
        let s = AppState::init(&d).unwrap();
        assert!(!s.db_rebuilt);
        assert!(d.join(DB_FILE).exists());
        assert!(s.needs_onboarding().unwrap());
        assert!(s.widget_visible().unwrap());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn corrupted_database_is_backed_up_and_rebuilt() {
        let d = tmp_dir("corrupt");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join(DB_FILE),
            b"this is not a sqlite file at all, just garbage bytes",
        )
        .unwrap();
        let s = AppState::init(&d).unwrap();
        assert!(s.db_rebuilt);
        let backups: Vec<_> = std::fs::read_dir(&d)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".corrupt-"))
            .collect();
        assert_eq!(backups.len(), 1);
        assert!(s.needs_onboarding().unwrap());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn writes_go_through_the_writer_and_reads_see_them() {
        let d = tmp_dir("writer");
        let s = AppState::init(&d).unwrap();
        s.writer()
            .write_sync(|db| settings::set(db, "widget.visible", &false.into()))
            .unwrap();
        assert!(!s.widget_visible().unwrap(), "同步写入返回后读连接就能读到");
        if cfg!(debug_assertions) {
            assert!(
                settings::set(&s.db(), "widget.visible", &true.into()).is_err(),
                "读连接不许写"
            );
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn update_status_reports_changes() {
        let d = tmp_dir("status");
        let s = AppState::init(&d).unwrap();
        let (snap, changed) = s.update_status(|st| StatusSnapshot::set(&mut st.paused, true));
        assert!(changed && snap.paused);
        let (_, changed) = s.update_status(|st| StatusSnapshot::set(&mut st.paused, true));
        assert!(!changed);
        let _ = std::fs::remove_dir_all(&d);
    }
}
