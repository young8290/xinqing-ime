//! 回放可复现选项（07 FR-DMO-01 `--baseline`、`--start-at`）在 Hub 这一侧的入口（ADR 0021）。
//!
//! 只在调试构建里读两个环境变量，`xq-sim` 收到对应参数时会打印设好它们的 Hub 启动命令：
//! - `XQ_SIM_BASELINE=<基线.toml>`：感知用这份固定基线（格式同 `baseline_default.toml`），
//!   不读写真实基线：启动时和每天 04:00 都不重算，设置页“重置基线”返回错误；
//! - `XQ_SIM_START_AT=HH:MM`：Hub 的时钟从当天这个时刻起走（[`OffsetClock`]），感知、休息提醒、
//!   暖心话、对话和网关共用同一份。
//!
//! 发行构建忽略这两个变量：真实用户的时间和基线不能被环境变量改掉。
//!
//! 演示模式（07 FR-DMO-03，B-10，ADR 0023）也在这里：以 `--demo` 启动时（发行构建同样有效），
//! 时钟换成 ×60 的 [`ScaledClock`]，数据换成每次重建的演示库（[`open_demo_state`]）。

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use chrono::Local;
use xinqing_hub_core::domain::consent::{self, ConsentItem, ConsentState};
use xinqing_hub_core::domain::demo;
use xinqing_hub_core::domain::features::Baseline;
use xinqing_hub_core::domain::settings;
use xinqing_hub_core::infra::clock::{self, Clock, OffsetClock, ScaledClock, SystemClock};
use xinqing_hub_core::infra::store::Db;
use xinqing_hub_core::infra::templates::{BaselineDefault, TemplateDirs, read_toml};

use crate::paths;
use crate::state::{AppState, DB_FILE};

pub const BASELINE_ENV: &str = "XQ_SIM_BASELINE";
pub const START_AT_ENV: &str = "XQ_SIM_START_AT";

/// 调试构建里读到的环境变量值；发行构建两项都是 `None`。
fn var(name: &str) -> Option<String> {
    if !cfg!(debug_assertions) {
        if std::env::var_os(name).is_some() {
            eprintln!("发行构建忽略 {name}");
        }
        return None;
    }
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

static DEMO: OnceLock<bool> = OnceLock::new();

/// 启动时确定是否进入演示模式，必须在第一次调用 [`clock`] 之前。之后再调用不起作用。
pub fn set_demo(on: bool) {
    if DEMO.set(on).is_err() {
        eprintln!("演示模式已确定，忽略这次设置");
    }
}

/// 真实库里的设置 `dev.demo`（ADR 0025）：为真时这次启动进入演示模式，等同 `--demo`。
/// 数据目录或真实库不存在、读不了时为 `false`；不会新建真实库。
pub fn demo_setting(data_dir: &Path) -> bool {
    let real = data_dir.join(DB_FILE);
    real.exists()
        && Db::open_reader(&real)
            .ok()
            .and_then(|db| settings::get(&db, "dev.demo").ok())
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
}

/// 本次运行是否为演示模式。
pub fn demo() -> bool {
    DEMO.get().copied().unwrap_or(false)
}

/// `XQ_SIM_START_AT` 指定的起始时刻；没设置或无效时为 `None`。
fn start_at() -> Option<chrono::DateTime<Local>> {
    let raw = var(START_AT_ENV)?;
    let start = clock::today_at(&raw, Local::now().date_naive());
    if start.is_none() {
        eprintln!("{START_AT_ENV}={raw} 无效（格式应为 HH:MM），使用系统时间");
    }
    start
}

/// Hub 各服务共用的时钟。演示模式下从现在（或 `XQ_SIM_START_AT`）起 ×60 走；
/// 否则设置了 `XQ_SIM_START_AT` 时从那一刻起正常走，再否则是系统时间。
/// 第一次调用时确定，之后每次返回同一份。
pub fn clock() -> Arc<dyn Clock> {
    static CLOCK: OnceLock<Arc<dyn Clock>> = OnceLock::new();
    CLOCK
        .get_or_init(|| {
            let start = start_at();
            if demo() {
                let start = start.unwrap_or_else(Local::now);
                eprintln!(
                    "演示模式：Hub 时钟从 {} 起 ×{} 走",
                    start.format("%H:%M"),
                    demo::SPEED
                );
                return Arc::new(ScaledClock::new(start, demo::SPEED));
            }
            match start {
                Some(start) => {
                    eprintln!("{START_AT_ENV}：Hub 时钟从 {} 起走", start.format("%H:%M"));
                    Arc::new(OffsetClock::starting_at(start))
                }
                None => Arc::new(SystemClock),
            }
        })
        .clone()
}

/// 演示模式的数据：删掉旧的演示库，新建并预置“过去一周”（FR-DMO-03 第 2 条），不碰真实库。
/// 真实库里已经给过的同意照搬过来，免得每场演示都先走一遍首次引导；真实库不存在或读不了时不搬。
pub fn open_demo_state(data_dir: &Path) -> anyhow::Result<AppState> {
    std::fs::create_dir_all(data_dir)?;
    let path = data_dir.join(demo::DB_FILE);
    for ext in ["", "-wal", "-shm"] {
        let f = PathBuf::from(format!("{}{ext}", path.display()));
        if f.exists() {
            std::fs::remove_file(&f)?;
        }
    }
    let state = AppState::init_file(data_dir, demo::DB_FILE)?;
    let now = clock().now();
    let base = paths::templates_dir()
        .and_then(|d| BaselineDefault::load(&TemplateDirs::factory_only(d)).ok());
    // 只读已有的真实库：打开不存在的文件会新建一个空库
    let real = data_dir.join(DB_FILE);
    let granted: Vec<ConsentItem> = real
        .exists()
        .then(|| Db::open_reader(&real).ok())
        .flatten()
        .and_then(|db| ConsentState::load(&db).ok())
        .map(|c| {
            ConsentItem::ALL
                .iter()
                .copied()
                .filter(|i| c.granted(*i))
                .collect()
        })
        .unwrap_or_default();
    let seeded = state.writer().write_sync(|db| {
        for item in &granted {
            consent::set(db, *item, true, now.timestamp_millis())?;
        }
        match &base {
            Some(b) => demo::seed(db, now, b),
            None => Ok(demo::Seeded::default()),
        }
    })?;
    if base.is_none() {
        eprintln!("演示模式：找不到 baseline_default.toml，演示库没有预置数据");
    }
    eprintln!(
        "演示模式：演示库 {}，预置 {} 个窗口、{} 条待确认日程，沿用 {} 项同意",
        path.display(),
        seeded.windows,
        seeded.schedules,
        granted.len()
    );
    Ok(state)
}

/// `demo_status`：界面据此在标题栏显示“演示模式”（FR-DMO-03）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, specta::Type)]
pub struct DemoStatus {
    pub enabled: bool,
    /// 时间加速倍数；不是演示模式时为 1
    pub speed: u32,
}

pub fn demo_status() -> DemoStatus {
    let enabled = demo();
    DemoStatus {
        enabled,
        speed: if enabled { demo::SPEED } else { 1 },
    }
}

/// `XQ_SIM_BASELINE` 指定的固定基线；没设置或文件读不了时为 `None`（读不了会打日志）。
pub fn fixed_baseline() -> Option<Baseline> {
    let path = PathBuf::from(var(BASELINE_ENV)?);
    match read_toml::<BaselineDefault>(&path) {
        Ok(d) => {
            eprintln!(
                "{BASELINE_ENV}：使用固定基线 v{} {}",
                d.version,
                path.display()
            );
            Some(Baseline::fixed(&d))
        }
        Err(e) => {
            eprintln!(
                "{BASELINE_ENV}={} 读取失败，回放结果不可复现：{e}",
                path.display()
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_database_is_rebuilt_each_time_and_keeps_the_real_one_apart() {
        let d = std::env::temp_dir().join(format!("xq-hub-demo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        // 真实库：只给了 ① 感知
        let real = AppState::init(&d).unwrap();
        real.writer()
            .write_sync(|db| consent::set(db, ConsentItem::Sense, true, 1))
            .unwrap();
        drop(real);

        for _ in 0..2 {
            let s = open_demo_state(&d).unwrap();
            let windows = s.db().window_features_since(0).unwrap().len() as u32;
            assert_eq!(windows, demo::DAYS * 45, "每次重建，不会叠加");
            let c = ConsentState::load(&s.db()).unwrap();
            assert!(!c.needs_onboarding(), "沿用真实库的同意");
            assert!(!c.granted(ConsentItem::LlmSummary));
            assert!(!s.db_rebuilt);
        }
        let real = Db::open_reader(&d.join(DB_FILE)).unwrap();
        assert!(
            real.window_features_since(0).unwrap().is_empty(),
            "不碰真实库"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn no_real_database_is_created() {
        let d = std::env::temp_dir().join(format!("xq-hub-demo-only-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let s = open_demo_state(&d).unwrap();
        assert!(ConsentState::load(&s.db()).unwrap().needs_onboarding());
        assert!(!d.join(DB_FILE).exists());
        drop(s);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn demo_setting_is_read_from_the_real_database_only() {
        let d = std::env::temp_dir().join(format!("xq-hub-demo-key-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        assert!(!demo_setting(&d), "没有数据目录");
        assert!(!d.join(DB_FILE).exists(), "不会新建真实库");
        let real = AppState::init(&d).unwrap();
        assert!(!demo_setting(&d), "默认关");
        real.writer()
            .write_sync(|db| settings::set(db, "dev.demo", &true.into()))
            .unwrap();
        assert!(demo_setting(&d));
        drop(real);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn status_is_off_by_default() {
        assert_eq!(
            demo_status(),
            DemoStatus {
                enabled: false,
                speed: 1
            }
        );
    }
}
