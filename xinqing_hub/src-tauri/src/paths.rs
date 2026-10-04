//! 数据目录（03 第 5.2 节）：`%LOCALAPPDATA%\XinQing\hub\`，dev 构建为 `XinQingDev`，
//! 与输入法核心的变体目录一致（`wind-config::variant::app_dir_name`）。

use std::path::PathBuf;

/// 测试与演示时可用环境变量把数据目录指到别处，避免碰到真实用户数据。
pub const DATA_DIR_ENV: &str = "XQ_HUB_DATA_DIR";

pub fn app_dir_name() -> &'static str {
    if cfg!(debug_assertions) {
        "XinQingDev"
    } else {
        "XinQing"
    }
}

pub fn hub_data_dir() -> anyhow::Result<PathBuf> {
    if let Some(dir) = std::env::var_os(DATA_DIR_ENV) {
        return Ok(PathBuf::from(dir));
    }
    let base = dirs::data_local_dir().ok_or_else(|| anyhow::anyhow!("找不到本地应用数据目录"))?;
    Ok(base.join(app_dir_name()).join("hub"))
}
