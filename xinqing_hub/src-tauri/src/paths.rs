//! 数据目录（03 第 5.2 节）：`%LOCALAPPDATA%\XinQing\hub\`，dev 构建为 `XinQingDev`，
//! 与输入法核心的变体目录一致（`wind-config::variant::app_dir_name`）。

use std::path::PathBuf;
use xinqing_hub_core::infra::templates::TemplateDirs;

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

/// 可用环境变量指定出厂模板目录（演示、测试时用）。
pub const TEMPLATES_ENV: &str = "XQ_HUB_TEMPLATES";

/// 出厂模板目录：环境变量 → 可执行文件旁的 `hub_templates/`（安装包放在这里，A-10）
/// → 调试构建时仓库里的 `hub_templates/`。
pub fn templates_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os(TEMPLATES_ENV) {
        return Some(PathBuf::from(dir));
    }
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.join("hub_templates")));
    if let Some(d) = beside.filter(|d| d.is_dir()) {
        return Some(d);
    }
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../hub_templates");
    (cfg!(debug_assertions) && repo.is_dir()).then_some(repo)
}

/// 暖心话模板的出厂目录与用户覆盖目录；调试变体和 XQ_HUB_DATA_DIR 沿用数据目录规则。
pub fn hub_template_dirs() -> anyhow::Result<TemplateDirs> {
    let factory = templates_dir().ok_or_else(|| anyhow::anyhow!("找不到 hub_templates"))?;
    Ok(template_dirs_for(factory, hub_data_dir()?))
}

fn template_dirs_for(factory: PathBuf, data_dir: PathBuf) -> TemplateDirs {
    TemplateDirs {
        factory,
        user: Some(data_dir.join("templates")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overrides_follow_the_selected_data_directory() {
        for data_dir in ["XinQing/hub", "XinQingDev/hub", "isolated-demo/hub"] {
            let factory = PathBuf::from("factory");
            let data = PathBuf::from(data_dir);
            let dirs = template_dirs_for(factory.clone(), data.clone());
            assert_eq!(dirs.factory, factory);
            assert_eq!(dirs.user, Some(data.join("templates")));
        }
    }
}
