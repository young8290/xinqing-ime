//! 剪贴板变化监听（Windows），供剪贴板历史（`moodflow-clipboard`，计划 §2.4）采集。
//!
//! 在 UI 线程建一个**仅消息窗口**（`HWND_MESSAGE`，不可见、不进 Z 序），向系统注册
//! `AddClipboardFormatListener`；每次剪贴板变化收到 `WM_CLIPBOARDUPDATE`，窗口过程只置标记，
//! UI 主循环在消息泵之后 [`take_change`] 取走并读文本——与系统明暗变化同一套「置标记 → 泵后取」做法，
//! 窗口过程里不做任何可能阻塞的事。
//!
//! # 敏感内容
//!
//! 密码管理器等程序会附带两个约定格式，**任一命中即标记为敏感**，由历史侧整条丢弃：
//! - `ExcludeClipboardContentFromMonitorProcessing`：存在即排除；
//! - `CanIncludeInClipboardHistory`：值为 0 表示不得进入剪贴板历史。
//!
//! 本进程自己写剪贴板（复制候选等）后会记下序列号（[`note_own_write`]），监听到同一序列号时跳过，
//! 不把自己的写入当成用户的复制。
//!
//! 正文只经 `UiEvent::ClipboardChanged` 交给协调器，**不进日志**。

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// 窗口过程置位、主循环取走。
static CHANGED: AtomicBool = AtomicBool::new(false);
/// 本进程最近一次写剪贴板后的序列号（0 = 无）。
static OWN_SEQ: AtomicU32 = AtomicU32::new(0);

/// 一次剪贴板变化读到的内容。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardChange {
    pub text: String,
    pub sensitive: bool,
}

/// 本进程刚写完剪贴板：记下写后的序列号，监听到它时跳过。
pub fn note_own_write(seq: u32) {
    OWN_SEQ.store(seq, Ordering::Relaxed);
}

#[cfg(windows)]
mod imp {
    use super::*;
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::DataExchange::{
        AddClipboardFormatListener, CloseClipboard, GetClipboardData, GetClipboardSequenceNumber,
        IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW,
        RemoveClipboardFormatListener,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, HMENU, HWND_MESSAGE, RegisterClassExW,
        WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLIPBOARDUPDATE, WNDCLASSEXW,
    };
    use windows::core::{PCWSTR, w};

    /// 监听器：持有仅消息窗口，Drop 时注销并销毁。
    pub struct ClipboardMonitor {
        hwnd: HWND,
    }

    unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        if msg == WM_CLIPBOARDUPDATE {
            CHANGED.store(true, Ordering::Relaxed);
            return LRESULT(0);
        }
        unsafe { DefWindowProcW(hwnd, msg, w, l) }
    }

    impl ClipboardMonitor {
        pub fn new() -> Result<Self, String> {
            unsafe {
                let instance = GetModuleHandleW(None).map_err(|e| e.to_string())?;
                let class = w!("MoodflowClipboardMonitor");
                let wc = WNDCLASSEXW {
                    cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                    lpfnWndProc: Some(wnd_proc),
                    hInstance: instance.into(),
                    lpszClassName: class,
                    ..Default::default()
                };
                // 重复注册（监听器关了又开）返回 0，不影响后续创建。
                RegisterClassExW(&wc);
                let hwnd = CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    class,
                    class,
                    WINDOW_STYLE::default(),
                    0,
                    0,
                    0,
                    0,
                    HWND_MESSAGE,
                    HMENU::default(),
                    instance,
                    None,
                )
                .map_err(|e| format!("CreateWindowExW: {e}"))?;
                if let Err(e) = AddClipboardFormatListener(hwnd) {
                    let _ = DestroyWindow(hwnd);
                    return Err(format!("AddClipboardFormatListener: {e}"));
                }
                Ok(Self { hwnd })
            }
        }
    }

    impl Drop for ClipboardMonitor {
        fn drop(&mut self) {
            unsafe {
                let _ = RemoveClipboardFormatListener(self.hwnd);
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }

    fn format_id(name: PCWSTR) -> u32 {
        unsafe { RegisterClipboardFormatW(name) }
    }

    /// 剪贴板已由调用方打开。读一个 DWORD 格式的值。
    unsafe fn read_dword(format: u32) -> Option<u32> {
        unsafe {
            let h = GetClipboardData(format).ok()?;
            let g = windows::Win32::Foundation::HGLOBAL(h.0);
            let p = GlobalLock(g) as *const u32;
            if p.is_null() {
                return None;
            }
            let v = (GlobalSize(g) >= 4).then(|| p.read_unaligned());
            let _ = GlobalUnlock(g);
            v
        }
    }

    /// 读当前剪贴板的敏感标记。打不开剪贴板时按敏感处理（宁可漏记，不可错记）。
    fn sensitive_now() -> bool {
        let exclude = format_id(w!("ExcludeClipboardContentFromMonitorProcessing"));
        let can_include = format_id(w!("CanIncludeInClipboardHistory"));
        unsafe {
            if exclude != 0 && IsClipboardFormatAvailable(exclude).is_ok() {
                return true;
            }
            if can_include == 0 || IsClipboardFormatAvailable(can_include).is_err() {
                return false;
            }
            if OpenClipboard(HWND::default()).is_err() {
                return true;
            }
            let v = read_dword(can_include);
            let _ = CloseClipboard();
            v != Some(1)
        }
    }

    pub fn take_change() -> Option<ClipboardChange> {
        if !CHANGED.swap(false, Ordering::Relaxed) {
            return None;
        }
        let seq = unsafe { GetClipboardSequenceNumber() };
        if seq != 0 && seq == OWN_SEQ.load(Ordering::Relaxed) {
            return None;
        }
        let sensitive = sensitive_now();
        // 敏感内容连读都不读，正文不进本进程内存。
        let text = if sensitive {
            String::new()
        } else {
            crate::popup_menu::get_clipboard_text()
        };
        Some(ClipboardChange { text, sensitive })
    }
}

#[cfg(windows)]
pub use imp::{ClipboardMonitor, take_change};

/// 非 Windows：没有监听器（Android 由宿主在键盘可见期间读剪贴板后注入事件）。
#[cfg(not(windows))]
pub struct ClipboardMonitor;

#[cfg(not(windows))]
impl ClipboardMonitor {
    pub fn new() -> Result<Self, String> {
        Err("clipboard monitor is Windows-only".into())
    }
}

#[cfg(not(windows))]
pub fn take_change() -> Option<ClipboardChange> {
    CHANGED.swap(false, Ordering::Relaxed);
    None
}
