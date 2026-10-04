//! 窗口管理（07 第 2 节窗口清单）。窗口的尺寸与样式都写在 `tauri.conf.json`，且全部 `create: false`：
//! 启动时由 `lib.rs` 按引导状态决定先开哪个，其余按需创建；关掉的窗口下次再从配置重建。

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{Manager, Runtime, WebviewWindowBuilder};

use crate::error::UiError;

/// `open_window` 与 `--open` 的目标。标签与 `tauri.conf.json` 的窗口 `label`、
/// 前端 `src/windows/<label>/` 目录一一对应。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum WindowTarget {
    Widget,
    Chat,
    Dashboard,
    Settings,
    Onboarding,
}

impl WindowTarget {
    pub const ALL: [WindowTarget; 5] = [
        WindowTarget::Widget,
        WindowTarget::Chat,
        WindowTarget::Dashboard,
        WindowTarget::Settings,
        WindowTarget::Onboarding,
    ];

    pub fn label(self) -> &'static str {
        match self {
            WindowTarget::Widget => "widget",
            WindowTarget::Chat => "chat",
            WindowTarget::Dashboard => "dashboard",
            WindowTarget::Settings => "settings",
            WindowTarget::Onboarding => "onboarding",
        }
    }

    pub fn from_label(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.label() == s)
    }
}

/// 显示一个窗口；还没创建就按配置创建。小组件不抢焦点（FR-WGT-07 的“不抢焦点”不变量）。
pub fn open<R: Runtime, M: Manager<R>>(app: &M, target: WindowTarget) -> Result<(), UiError> {
    let label = target.label();
    let win = match app.get_webview_window(label) {
        Some(w) => w,
        None => {
            let config = app
                .config()
                .app
                .windows
                .iter()
                .find(|w| w.label == label)
                .cloned()
                .ok_or_else(|| UiError::internal("window.not_configured", label))?;
            WebviewWindowBuilder::from_config(app, &config)?.build()?
        }
    };
    if win.is_minimized()? {
        win.unminimize()?;
    }
    win.show()?;
    if target != WindowTarget::Widget {
        win.set_focus()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_round_trip_and_match_serde() {
        for t in WindowTarget::ALL {
            assert_eq!(WindowTarget::from_label(t.label()), Some(t));
            assert_eq!(serde_json::to_value(t).unwrap(), t.label());
        }
        assert_eq!(WindowTarget::from_label("tray"), None);
    }

    /// 每个目标都必须在 `tauri.conf.json` 里有窗口配置，且不随应用启动自动创建。
    #[test]
    fn every_target_is_configured_and_created_on_demand() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let windows = conf["app"]["windows"].as_array().unwrap();
        for t in WindowTarget::ALL {
            let w = windows
                .iter()
                .find(|w| w["label"] == t.label())
                .unwrap_or_else(|| panic!("tauri.conf.json 缺少窗口 {}", t.label()));
            assert_eq!(w["create"], false, "{} 应按需创建", t.label());
            assert_eq!(w["url"], format!("{}/index.html", t.label()));
        }
        assert_eq!(windows.len(), WindowTarget::ALL.len());
    }
}
