//! 回放可复现选项（07 FR-DMO-01 `--baseline`、`--start-at`）在 Hub 这一侧的入口（ADR 0021）。
//!
//! 只在调试构建里读两个环境变量，`xq-sim` 收到对应参数时会打印设好它们的 Hub 启动命令：
//! - `XQ_SIM_BASELINE=<基线.toml>`：感知用这份固定基线（格式同 `baseline_default.toml`），
//!   不读写真实基线：启动时和每天 04:00 都不重算，设置页“重置基线”返回错误；
//! - `XQ_SIM_START_AT=HH:MM`：Hub 的时钟从当天这个时刻起走（[`OffsetClock`]），感知、休息提醒、
//!   暖心话、对话和网关共用同一份。
//!
//! 发行构建忽略这两个变量：真实用户的时间和基线不能被环境变量改掉。

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use chrono::Local;
use xinqing_hub_core::domain::features::Baseline;
use xinqing_hub_core::infra::clock::{self, Clock, OffsetClock, SystemClock};
use xinqing_hub_core::infra::templates::{BaselineDefault, read_toml};

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

/// Hub 各服务共用的时钟：设置了 `XQ_SIM_START_AT` 时从那一刻起走，否则是系统时间。
/// 第一次调用时确定偏移，之后每次返回同一份。
pub fn clock() -> Arc<dyn Clock> {
    static CLOCK: OnceLock<Arc<dyn Clock>> = OnceLock::new();
    CLOCK
        .get_or_init(|| {
            let Some(raw) = var(START_AT_ENV) else {
                return Arc::new(SystemClock);
            };
            match clock::today_at(&raw, Local::now().date_naive()) {
                Some(start) => {
                    eprintln!("{START_AT_ENV}：Hub 时钟从 {} 起走", start.format("%H:%M"));
                    Arc::new(OffsetClock::starting_at(start))
                }
                None => {
                    eprintln!("{START_AT_ENV}={raw} 无效（格式应为 HH:MM），使用系统时间");
                    Arc::new(SystemClock)
                }
            }
        })
        .clone()
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
