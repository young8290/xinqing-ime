//! Buffer 浮窗（docs/design/buffer.md §3 Windows 列）：显示上屏前攒在 Buffer 里的文本。
//!
//! 不激活、不抢焦点（`LayeredWindow` 默认带 `WS_EX_NOACTIVATE`），焦点始终留在宿主输入框里，
//! 用户照常打字；发送、删末块、清空等经快捷键或右键菜单，菜单树与动作归协调器
//! （右键只回送 `UiEvent::RequestBufferMenu`，与诊断 HUD 同一做法）。
//!
//! 位置：贴在光标所在行的**上方**（候选窗在下方），上方放不下时翻到下方再往下让出一行。
//! 正文过长时只显示末尾一段，前面以 `…` 省略——浮窗是给人确认「攒到哪了」，不是编辑器。

use crate::text::dwrite::TextRenderer;
use crate::view::{Align, Edges, Layout, View};
use crate::window::LayeredWindow;

const BG: [u8; 4] = [36, 40, 48, 240];
const FG: [u8; 4] = [242, 244, 248, 255];
const HINT: [u8; 4] = [170, 178, 190, 255];
const FONT_PX: f32 = 15.0;
/// 正文最多显示的字符数（超出只留末尾）。
const MAX_SHOWN_CHARS: usize = 120;
/// 每行大约多少个字符就折行（中文为主，按字符数近似即可）。
const WRAP_CHARS: usize = 30;

/// 浮窗要显示的内容（协调器给）。
pub use moodflow_ui_types::BufferView;

/// 右键：回送协调器请求菜单。
struct Mouse {
    events: std::sync::mpsc::Sender<crate::manager::UiEvent>,
}

impl crate::window::WindowMouse for Mouse {
    fn on_message(
        &mut self,
        _hwnd: crate::sys::HWND,
        msg: u32,
        _wparam: crate::sys::WPARAM,
        _lparam: crate::sys::LPARAM,
    ) -> Option<crate::sys::LRESULT> {
        use crate::sys::{LRESULT, WM_RBUTTONDOWN};
        if msg == WM_RBUTTONDOWN {
            let mut pt = crate::sys::POINT::default();
            unsafe {
                let _ = crate::sys::GetCursorPos(&mut pt);
            }
            let _ = self
                .events
                .send(crate::manager::UiEvent::RequestBufferMenu { x: pt.x, y: pt.y });
            return Some(LRESULT(0));
        }
        None
    }
}

/// 把正文截成末尾 [`MAX_SHOWN_CHARS`] 个字符，再按 [`WRAP_CHARS`] 折行。纯函数，可单测。
pub(crate) fn display_lines(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().filter(|c| *c != '\r').collect();
    let (prefix, shown) = if chars.len() > MAX_SHOWN_CHARS {
        ("…", &chars[chars.len() - MAX_SHOWN_CHARS..])
    } else {
        ("", &chars[..])
    };
    let mut lines = Vec::new();
    let mut cur = String::from(prefix);
    let mut n = cur.chars().count();
    for &c in shown {
        if c == '\n' {
            lines.push(std::mem::take(&mut cur));
            n = 0;
            continue;
        }
        if n >= WRAP_CHARS {
            lines.push(std::mem::take(&mut cur));
            n = 0;
        }
        cur.push(c);
        n += 1;
    }
    if !cur.is_empty() || lines.is_empty() {
        lines.push(cur);
    }
    lines
}

pub struct BufferPanel {
    window: LayeredWindow,
    renderer: TextRenderer,
    scale: f32,
    _mouse: std::rc::Rc<std::cell::RefCell<Mouse>>,
}

impl BufferPanel {
    pub fn new(events: std::sync::mpsc::Sender<crate::manager::UiEvent>) -> Result<Self, String> {
        let scale = dpi_scale();
        let window = LayeredWindow::create(None, 200, 60, "MoodflowBufferPanel")?;
        let renderer = TextRenderer::new("Microsoft YaHei UI", FONT_PX * scale)?;
        let mouse = std::rc::Rc::new(std::cell::RefCell::new(Mouse { events }));
        window.register_mouse(mouse.clone());
        Ok(Self {
            window,
            renderer,
            scale,
            _mouse: mouse,
        })
    }

    pub fn show(&mut self, v: &BufferView) {
        let s = self.scale;
        let mut col = View::container(Layout::Column)
            .bg(BG)
            .pad(Edges::xy(12.0 * s, 8.0 * s))
            .gap(4.0 * s);
        col.corner_radius = 8.0 * s;
        let body = if v.text.is_empty() {
            vec!["（空）选好的词会先放在这里".to_string()]
        } else {
            display_lines(&v.text)
        };
        for line in body {
            col = col.child(View::leaf(line, FG).text_align(Align::Start));
        }
        col = col.child(View::leaf(v.status.clone(), HINT).text_align(Align::Start));
        col.layout(0.0, 0.0, &self.renderer);
        let (w_f, h_f) = col.measured_size();
        let w = (w_f.ceil() as u32).max(120);
        let h = (h_f.ceil() as u32).max(36);
        self.window.resize(w, h);
        self.window.buffer_mut().fill(0);
        {
            let (bw, bh) = self.window.size();
            let buf = self.window.buffer_mut();
            col.paint(buf, bw, bh, &self.renderer);
        }
        if let Err(e) = self.window.update() {
            tracing::warn!("BufferPanel update failed: {}", e);
        }
        let gap = (6.0 * s) as i32;
        // 先放光标行上方；放不下（顶到屏幕上沿）就放到行下方再让出一行给候选窗。
        let above = v.caret_y - h as i32 - gap;
        let y = if above >= 0 {
            above
        } else {
            v.caret_y + v.caret_height * 2 + gap
        };
        let (x, y) = crate::sys::clamp_to_work_area(v.caret_x, y, w, h);
        self.window.show(x, y);
    }

    pub fn hide(&self) {
        self.window.hide();
    }
}

/// 系统 DPI 缩放因子（同 input_diag_hud）。
fn dpi_scale() -> f32 {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::Graphics::Gdi::*;
        unsafe {
            let hdc = GetDC(HWND::default());
            let dpi = GetDeviceCaps(hdc, LOGPIXELSY);
            ReleaseDC(HWND::default(), hdc);
            if dpi > 0 { dpi as f32 / 96.0 } else { 1.0 }
        }
    }
    #[cfg(not(windows))]
    {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_text_is_one_line() {
        assert_eq!(display_lines("你好"), vec!["你好"]);
        assert_eq!(display_lines(""), vec![""]);
    }

    #[test]
    fn wraps_and_keeps_newlines() {
        let t = format!("{}\n尾", "字".repeat(WRAP_CHARS + 2));
        let l = display_lines(&t);
        assert_eq!(l.len(), 3);
        assert_eq!(l[0].chars().count(), WRAP_CHARS);
        assert_eq!(l[2], "尾");
    }

    #[test]
    fn long_text_shows_only_the_tail() {
        let t = format!("开头{}结尾", "中".repeat(MAX_SHOWN_CHARS));
        let l = display_lines(&t);
        assert!(l[0].starts_with('…'));
        assert!(l.concat().ends_with("结尾"));
        assert!(!l.concat().contains("开头"));
    }
}
