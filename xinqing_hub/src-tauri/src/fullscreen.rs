//! 前台是全屏应用时自动隐藏小组件，退出后恢复（07 FR-WGT-01）。
//!
//! 每秒问一次系统“现在适不适合打扰用户”（`SHQueryUserNotificationState`，系统通知也按它决定弹不弹）：
//! 全屏应用、全屏 D3D 游戏、演示模式都算全屏。判断逻辑是平台无关的 [`AutoHide`]，只有探测函数分平台。

use std::time::Duration;

use tauri::{AppHandle, Manager};

use crate::windows::WindowTarget;

/// 轮询间隔。调用本身是微秒级，一秒一次不费电；进出全屏晚一秒反应可以接受。
const POLL: Duration = Duration::from_secs(1);

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Hide,
    Show,
    Nothing,
}

/// 只在进出全屏的那一刻动手，而且退出时只恢复自己藏起来的小组件：
/// 用户自己隐藏的不会被叫回来；全屏期间用户主动叫出来的，也不再藏回去。
#[derive(Debug, Default)]
pub struct AutoHide {
    fullscreen: bool,
    hidden_by_us: bool,
}

impl AutoHide {
    pub fn step(&mut self, fullscreen: bool, widget_visible: bool) -> Action {
        let entered = fullscreen && !self.fullscreen;
        let left = !fullscreen && self.fullscreen;
        self.fullscreen = fullscreen;
        if entered && widget_visible {
            self.hidden_by_us = true;
            return Action::Hide;
        }
        if left && std::mem::take(&mut self.hidden_by_us) {
            return Action::Show;
        }
        if fullscreen && widget_visible {
            // 全屏期间用户把小组件叫了出来：算用户的决定，之后的去留不归这里管
            self.hidden_by_us = false;
        }
        Action::Nothing
    }
}

/// 前台是否为全屏应用（含全屏游戏与演示模式）。
#[cfg(windows)]
pub fn foreground_fullscreen() -> bool {
    use windows_sys::Win32::UI::Shell::{
        QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN,
        SHQueryUserNotificationState,
    };
    let mut state = 0;
    // SAFETY: 唯一的参数是一个整数出参，指向栈上的局部变量
    let hr = unsafe { SHQueryUserNotificationState(&mut state) };
    hr >= 0
        && matches!(
            state,
            QUNS_BUSY | QUNS_RUNNING_D3D_FULL_SCREEN | QUNS_PRESENTATION_MODE
        )
}

/// 其他平台暂不检测（Hub 目前只在 Windows 上发布，03 第 7 节）。
#[cfg(not(windows))]
pub fn foreground_fullscreen() -> bool {
    false
}

/// 启动轮询。非 Windows 上探测恒为 false，不必空转。
pub fn start(app: &AppHandle) {
    if !cfg!(windows) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut auto = AutoHide::default();
        let mut tick = tokio::time::interval(POLL);
        loop {
            tick.tick().await;
            // 小组件还没建（引导页阶段）或已关闭时按“不可见”处理
            let widget = app.get_webview_window(WindowTarget::Widget.label());
            let visible = widget
                .as_ref()
                .is_some_and(|w| w.is_visible().unwrap_or(false));
            let result = match (auto.step(foreground_fullscreen(), visible), &widget) {
                (Action::Hide, Some(w)) => w.hide(),
                // 只显示不聚焦：不能把焦点从刚退出全屏的应用那里抢走
                (Action::Show, Some(w)) => w.show(),
                _ => Ok(()),
            };
            if let Err(e) = result {
                eprintln!("全屏时隐藏 / 恢复小组件失败：{e}");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hides_on_entering_fullscreen_and_restores_on_leaving() {
        let mut a = AutoHide::default();
        assert_eq!(a.step(false, true), Action::Nothing);
        assert_eq!(a.step(true, true), Action::Hide);
        // 藏起来以后一直全屏：不重复动作
        assert_eq!(a.step(true, false), Action::Nothing);
        assert_eq!(a.step(false, false), Action::Show);
        assert_eq!(a.step(false, true), Action::Nothing);
    }

    #[test]
    fn never_brings_back_a_widget_the_user_hid() {
        let mut a = AutoHide::default();
        // 用户先隐藏了小组件，再进出全屏
        assert_eq!(a.step(true, false), Action::Nothing);
        assert_eq!(a.step(false, false), Action::Nothing);
    }

    #[test]
    fn respects_the_user_showing_it_during_fullscreen() {
        let mut a = AutoHide::default();
        assert_eq!(a.step(true, true), Action::Hide);
        // 全屏期间用户把它叫出来，又自己藏了：退出全屏时不再恢复
        assert_eq!(a.step(true, true), Action::Nothing);
        assert_eq!(a.step(true, false), Action::Nothing);
        assert_eq!(a.step(false, false), Action::Nothing);
    }

    #[test]
    fn stays_put_while_fullscreen_continues() {
        let mut a = AutoHide::default();
        assert_eq!(a.step(true, true), Action::Hide);
        for _ in 0..5 {
            assert_eq!(a.step(true, false), Action::Nothing);
        }
    }

    #[test]
    fn probe_is_false_off_windows() {
        if !cfg!(windows) {
            assert!(!foreground_fullscreen());
        }
    }
}
