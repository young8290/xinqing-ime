//! Hub 的全局快捷键（07 FR-ENT-04，ADR 0036）：`Ctrl+Alt+Q` 打开对话、`Ctrl+Alt+W` 显示 / 隐藏小组件、`Ctrl+Alt+D` 打开看板。
//! `Ctrl+Alt+P`（暂停感知）与 `Ctrl+Alt+R`（温柔改写）在输入法核心里，Hub 异常时也能用，不归这里。
//!
//! 组合键存在设置键 `hotkey.*`（标量字段，清空即不设）；启动时和设置改了时整组重新注册。注册失败（被别的软件占用、
//! 或几项设成了同一组键）不弹窗，记在 [`Hotkeys`] 里，设置页用 `hotkey_status` 读出来提示冲突。

use std::str::FromStr;
use std::sync::Mutex;

use serde::Serialize;
use specta::Type;
use tauri::plugin::TauriPlugin;
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_global_shortcut::{
    Builder, Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState,
};
use xinqing_hub_core::domain::hotkey::Hotkey;
use xinqing_hub_core::domain::settings;

use crate::state::AppState;
use crate::windows::{self, WindowTarget};

/// 快捷键做什么。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum HotkeyAction {
    /// 打开对话（FR-CHT-01）
    Chat,
    /// 显示 / 隐藏小组件（FR-WGT-01）
    Widget,
    /// 打开看板
    Dashboard,
}

impl HotkeyAction {
    pub const ALL: [HotkeyAction; 3] = [
        HotkeyAction::Chat,
        HotkeyAction::Widget,
        HotkeyAction::Dashboard,
    ];

    /// 对应的设置键。
    pub fn setting_key(self) -> &'static str {
        match self {
            HotkeyAction::Chat => "hotkey.chat",
            HotkeyAction::Widget => "hotkey.widget",
            HotkeyAction::Dashboard => "hotkey.dashboard",
        }
    }
}

/// 一项快捷键现在的样子（设置页“常规”分类）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct HotkeyStatus {
    pub action: HotkeyAction,
    /// 设置里的组合键（规范写法，如 `ctrl+alt+q`）；空串表示没设
    pub keys: String,
    /// 设了但没注册上：被别的软件占用，或与另一项重复。界面提示 `error.hotkey_conflict`
    pub conflict: bool,
}

/// 外壳托管的注册结果：`(注册上的快捷键 id, 动作)` 与给设置页看的状态。
#[derive(Default)]
pub struct Hotkeys {
    bound: Mutex<Vec<(u32, HotkeyAction)>>,
    status: Mutex<Vec<HotkeyStatus>>,
}

impl Hotkeys {
    pub fn status(&self) -> Vec<HotkeyStatus> {
        self.status.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn action(&self, id: u32) -> Option<HotkeyAction> {
        self.bound
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|(i, _)| *i == id)
            .map(|(_, a)| *a)
    }
}

/// 插件：按下时按 id 找动作执行（松开不管）。
pub fn plugin<R: Runtime>() -> TauriPlugin<R> {
    Builder::new()
        .with_handler(|app, shortcut, event| {
            if event.state != ShortcutState::Pressed {
                return;
            }
            let action = app
                .try_state::<Hotkeys>()
                .and_then(|h| h.action(shortcut.id()));
            if let Some(a) = action
                && let Err(e) = run(app, a)
            {
                eprintln!("快捷键 {a:?} 执行失败：{e}");
            }
        })
        .build()
}

fn run<R: Runtime>(app: &AppHandle<R>, action: HotkeyAction) -> Result<(), crate::error::UiError> {
    match action {
        HotkeyAction::Chat => windows::open(app, WindowTarget::Chat),
        HotkeyAction::Dashboard => windows::open(app, WindowTarget::Dashboard),
        HotkeyAction::Widget => match app.get_webview_window(WindowTarget::Widget.label()) {
            Some(w) if w.is_visible()? => Ok(w.hide()?),
            _ => windows::open(app, WindowTarget::Widget),
        },
    }
}

/// `Hotkey` → 插件的 `Shortcut`。
fn shortcut(hk: &Hotkey) -> Option<Shortcut> {
    let mut mods = Modifiers::empty();
    for (on, m) in [
        (hk.ctrl, Modifiers::CONTROL),
        (hk.alt, Modifiers::ALT),
        (hk.shift, Modifiers::SHIFT),
        (hk.win, Modifiers::SUPER),
    ] {
        if on {
            mods |= m;
        }
    }
    let code = match hk.key.as_bytes() {
        [c] if c.is_ascii_lowercase() => format!("Key{}", c.to_ascii_uppercase() as char),
        [c] if c.is_ascii_digit() => format!("Digit{}", *c as char),
        _ => hk.key.to_ascii_uppercase(),
    };
    Some(Shortcut::new(Some(mods), Code::from_str(&code).ok()?))
}

/// 按当前设置整组重新注册。须在 `AppState`、[`Hotkeys`] 托管之后调用；设置改了 `hotkey.*` 时再调。
pub fn apply(app: &AppHandle) {
    let (Some(state), Some(hotkeys)) = (app.try_state::<AppState>(), app.try_state::<Hotkeys>())
    else {
        return;
    };
    let gs = app.global_shortcut();
    if let Err(e) = gs.unregister_all() {
        eprintln!("注销全局快捷键失败：{e}");
    }
    let mut bound = Vec::new();
    let mut status = Vec::new();
    for action in HotkeyAction::ALL {
        let raw = settings::get(&state.db(), action.setting_key())
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default();
        let parsed = Hotkey::parse(&raw);
        let keys = parsed.as_ref().map(Hotkey::normalized).unwrap_or_default();
        let mut conflict = false;
        if let Some(sc) = parsed.as_ref().and_then(shortcut) {
            match gs.register(sc) {
                Ok(()) => bound.push((sc.id(), action)),
                Err(e) => {
                    eprintln!("注册快捷键 {keys} 失败：{e}");
                    conflict = true;
                }
            }
        }
        status.push(HotkeyStatus {
            action,
            keys,
            conflict,
        });
    }
    *hotkeys.bound.lock().unwrap_or_else(|e| e.into_inner()) = bound;
    *hotkeys.status.lock().unwrap_or_else(|e| e.into_inner()) = status;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_action_has_a_registered_setting_key_with_a_default() {
        for a in HotkeyAction::ALL {
            let spec = settings::KEYS
                .iter()
                .find(|k| k.key == a.setting_key())
                .unwrap_or_else(|| panic!("{} 没登记", a.setting_key()));
            let d = (spec.default)();
            let d = d.as_str().unwrap();
            assert!(shortcut(&Hotkey::parse(d).unwrap()).is_some(), "{d}");
        }
    }

    #[test]
    fn keys_map_to_plugin_codes() {
        let sc = |s: &str| shortcut(&Hotkey::parse(s).unwrap()).unwrap();
        assert_eq!(sc("ctrl+alt+q").key, Code::KeyQ);
        assert_eq!(sc("ctrl+alt+7").key, Code::Digit7);
        assert_eq!(sc("win+f12").key, Code::F12);
        assert!(sc("ctrl+shift+win+a").mods.contains(Modifiers::SUPER | Modifiers::SHIFT));
    }
}
