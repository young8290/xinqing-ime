//! 窗口管理（07 第 2 节窗口清单）。窗口的尺寸与样式都写在 `tauri.conf.json`，且全部 `create: false`：
//! 启动时由 `lib.rs` 按引导状态决定先开哪个，其余按需创建；关掉的窗口下次再从配置重建。
//! 小组件与卡片层例外：建好后由前端定好位置再显示（`visible: false`）。卡片层随小组件一起建（FR-WGT-07），
//! 有卡片时自己贴着小组件显示，没有时隐藏。

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
    /// 卡片层（07 第 2 节、FR-WGT-07）：依附小组件，由前端决定显示与位置
    Cards,
}

impl WindowTarget {
    pub const ALL: [WindowTarget; 6] = [
        WindowTarget::Widget,
        WindowTarget::Chat,
        WindowTarget::Dashboard,
        WindowTarget::Settings,
        WindowTarget::Onboarding,
        WindowTarget::Cards,
    ];

    pub fn label(self) -> &'static str {
        match self {
            WindowTarget::Widget => "widget",
            WindowTarget::Chat => "chat",
            WindowTarget::Dashboard => "dashboard",
            WindowTarget::Settings => "settings",
            WindowTarget::Onboarding => "onboarding",
            WindowTarget::Cards => "cards",
        }
    }

    pub fn from_label(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.label() == s)
    }
}

/// 显示一个窗口；还没创建就按配置创建。小组件不抢焦点（FR-WGT-07 的“不抢焦点”不变量）。
pub fn open<R: Runtime, M: Manager<R>>(app: &M, target: WindowTarget) -> Result<(), UiError> {
    let label = target.label();
    // 打开小组件就看到了暖心话，熄灭工具栏小圆点（FR-CMF-04 第 4 条）
    if target == WindowTarget::Widget {
        crate::comfort::widget_opened(app);
    }
    // 卡片层跟着小组件一起建好，事件来了才接得住（FR-WGT-07）
    if target == WindowTarget::Widget {
        open(app, WindowTarget::Cards)?;
    }
    let (win, created) = match app.get_webview_window(label) {
        Some(w) => (w, false),
        None => {
            let config = app
                .config()
                .app
                .windows
                .iter()
                .find(|w| w.label == label)
                .cloned()
                .ok_or_else(|| UiError::internal("window.not_configured", label))?;
            (
                WebviewWindowBuilder::from_config(app, &config)?.build()?,
                true,
            )
        }
    };
    // 小组件建好时先不显示（配置里 `visible: false`）：前端挪到记住的位置后自己 show()，
    // 免得先在配置的位置闪一下再跳过去（FR-WGT-01）
    if created && target == WindowTarget::Widget {
        return Ok(());
    }
    // 卡片层只由它自己的前端显示和隐藏：没有卡片时不该出现
    if target == WindowTarget::Cards {
        return Ok(());
    }
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

    /// 小组件与卡片层建好时不显示，等前端定位后再 show()（`useWidgetWindow.ts`、`useCardsWindow.ts`）；其余窗口照常显示。
    #[test]
    fn only_the_widget_and_cards_start_hidden() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        for w in conf["app"]["windows"].as_array().unwrap() {
            let hidden = w["visible"] == false;
            let floating = w["label"] == "widget" || w["label"] == "cards";
            assert_eq!(hidden, floating, "{}", w["label"]);
        }
    }

    /// 卡片层不抢焦点、不进任务栏、跟小组件一样置顶（FR-WGT-07、07 第 2 节）。
    #[test]
    fn cards_layer_does_not_take_focus() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let w = conf["app"]["windows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|w| w["label"] == "cards")
            .unwrap();
        assert_eq!(w["focus"], false);
        assert_eq!(w["skipTaskbar"], true);
        assert_eq!(w["alwaysOnTop"], true);
        assert_eq!(w["width"], 320);
    }
}
