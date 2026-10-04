//! Windows 发键：`SendInput` 发虚拟键（带扫描码），按键照常经过 TSF 和输入法。

use anyhow::{Result, bail};
use windows_sys::Win32::Media::{TIMERR_NOERROR, timeBeginPeriod, timeEndPeriod};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, MAPVK_VK_TO_VSC, MapVirtualKeyW,
    SendInput,
};
use windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

/// 把系统计时器精度调到 1 ms，析构时还原
pub struct HighResTimer(bool);

impl HighResTimer {
    pub fn new() -> Self {
        // SAFETY: 无指针参数；成功时必须配对 timeEndPeriod，见 Drop
        Self(unsafe { timeBeginPeriod(1) } == TIMERR_NOERROR)
    }
}

impl Drop for HighResTimer {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: 与 new 里成功的 timeBeginPeriod(1) 配对
            unsafe { timeEndPeriod(1) };
        }
    }
}

/// 前台窗口句柄（只用来比较是否变了）
pub fn foreground() -> usize {
    // SAFETY: 无参数，返回值可能为空，只做比较
    unsafe { GetForegroundWindow() as usize }
}

fn key_input(vk: u16, scan: u16, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// 按下并松开一个键
pub fn tap(vk: u16) -> Result<()> {
    // SAFETY: 纯查表，无指针参数
    let scan = unsafe { MapVirtualKeyW(u32::from(vk), MAPVK_VK_TO_VSC) } as u16;
    let inputs = [key_input(vk, scan, 0), key_input(vk, scan, KEYEVENTF_KEYUP)];
    // SAFETY: inputs 在调用期间有效，长度与 cbsize 与数组一致
    let n = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            size_of::<INPUT>() as i32,
        )
    };
    if n as usize != inputs.len() {
        bail!(
            "SendInput 只送出了 {n}/2 个事件（目标窗口可能是管理员权限，typer 也要用管理员身份运行）"
        );
    }
    Ok(())
}
