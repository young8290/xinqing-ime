//! Buffer 接入（Windows，docs/design/buffer.md §3）：上屏前的文本工作台。
//!
//! - `keys.toggle_buffer`（出厂 Alt+Shift+B）：打开 Buffer 并**显式绑定**当前输入框；已打开但
//!   绑定已暂停（换过框、切过应用）时再按一次重新绑定；已打开且绑定有效时再按一次关闭（内容保留）。
//! - 打开且绑定有效时，按键动作里的上屏正文**改进 Buffer**（每次一块），宿主只收到一次空上屏
//!   用来结束组字——候选窗、组字区照常收尾，文档里什么都没写进去。
//! - `keys.buffer_send`（出厂 Ctrl+Shift+Enter）：把整个 Buffer 投递到绑定的输入框。有组字时只
//!   提示先确认组字，这次按键不顺带发送（契约 7）。
//! - 右键浮窗：发送、删除最后一块、清空、关闭。
//!
//! # 授权与目标
//!
//! 目标身份取 TSF 客户端令牌（进程 + 文本服务实例）。TSF 正常路径上拿不到文档/上下文指针
//! （只在诊断快照里有），故**同一进程内换框**目前识别不到——作为补偿，任何一次有效的失焦都
//! 撤销授权（契约 4），用户回到输入框后按一次绑定键即可。宁可多按一次，也不让旧内容落进别的框。
//!
//! 投递走按键响应（`KeyAction::InsertText`），在宿主的按键编辑会话里同步写入，按 Accepted 结算；
//! 菜单发起的发送没有按键上下文，经 push 通道交给当前活动客户端，同样先核对目标再投递。
//! 推送通道没有回执，「结果不明 → 暂停不重发」（契约 6）要等 TSF 加回执后才能真正区分。
//!
//! 正文是临时数据，不落盘、不进日志（契约 12）。

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use moodflow_bridge::handler::KeyAction;
use moodflow_buffer::{
    BindError, Buffer, DeliverError, EditError, FieldKind, Outcome, State, Target,
};
use moodflow_ui_types::{BufferView, MenuAnchor, MenuCmd, MenuItemSpec as M, MenuKind, UiCommand};

use crate::coordinator::Coordinator;

/// Buffer 容量：16,384 个 UTF-16 码元（契约 11）。
const CAPACITY_UTF16: usize = 16_384;

pub(crate) struct BufferState {
    inner: Mutex<Inner>,
    /// 本次按键动作是 Buffer 自己的投递，出口处不要再截回 Buffer。
    delivering: AtomicBool,
}

struct Inner {
    buf: Buffer,
    open: bool,
    /// 当前焦点所在的目标（最近一次 focus_gained）。
    current: Option<Target>,
}

impl Default for BufferState {
    fn default() -> Self {
        Self {
            inner: Mutex::new(Inner {
                buf: Buffer::new(CAPACITY_UTF16),
                open: false,
                current: None,
            }),
            delivering: AtomicBool::new(false),
        }
    }
}

/// 截获结果。
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Capture {
    /// 不归 Buffer 管，原样交给宿主。
    PassThrough,
    /// 已收进 Buffer；宿主那边改成空上屏。
    Captured,
    /// Buffer 满了，整段拒收（不截断），原样交给宿主并提示。
    Full,
}

impl BufferState {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn is_open(&self) -> bool {
        self.lock().open
    }

    /// 打开且绑定有效时把一段上屏正文收进 Buffer。
    pub(crate) fn capture(&self, text: &str) -> Capture {
        if text.is_empty() || self.delivering.load(Ordering::Relaxed) {
            return Capture::PassThrough;
        }
        let mut g = self.lock();
        if !g.open || g.buf.state() != State::Ready {
            return Capture::PassThrough;
        }
        match g.buf.push_phrase(text) {
            Ok(_) => Capture::Captured,
            Err(EditError::CapacityExceeded) => Capture::Full,
            Err(_) => Capture::PassThrough,
        }
    }

    fn focus_gained(&self, token: u64) {
        let mut g = self.lock();
        let t = Target::new(token, 0);
        g.current = Some(t);
        g.buf.focus_changed(t);
    }

    fn focus_lost(&self) {
        self.lock().buf.suspend();
    }

    /// 取出整个 Buffer 准备投递到当前目标，立即按 Accepted 结算（调用方保证随后写入）。
    fn take_all(&self) -> Result<String, DeliverError> {
        let mut g = self.lock();
        let Some(cur) = g.current else {
            return Err(DeliverError::NotBound);
        };
        let d = g.buf.begin_all(cur)?;
        if !g.buf.is_authorized(d.token, cur) {
            return Err(DeliverError::TargetMismatch);
        }
        let _ = g.buf.finish(d.token, Outcome::Accepted);
        Ok(d.text)
    }

    fn view(&self, hint_send: &str, hint_toggle: &str) -> (String, String) {
        let g = self.lock();
        let status = match g.buf.state() {
            State::Ready => format!("已绑定 · {hint_send} 发送 · 右键更多"),
            State::Suspended | State::Unbound => {
                format!("已暂停（换过输入框）· 回到要发送的框按 {hint_toggle} 重新绑定")
            }
            State::Held => "上次发送结果不明，已暂停 · 右键确认后继续".to_string(),
        };
        (g.buf.text(), status)
    }
}

fn deliver_error_tip(e: DeliverError) -> &'static str {
    match e {
        DeliverError::Empty => "Buffer 是空的",
        DeliverError::NotBound | DeliverError::Suspended | DeliverError::TargetMismatch => {
            "Buffer 未绑定当前输入框：先按绑定键重新绑定"
        }
        DeliverError::Held => "上次发送结果不明，已暂停：确认后再发",
        DeliverError::Busy => "正在发送",
    }
}

impl Coordinator {
    fn buffer_key_labels(&self) -> (String, String) {
        let k = &self.rt().config.keys;
        let label = |s: &str| {
            if s.is_empty() {
                "（未设置）".to_string()
            } else {
                s.to_string()
            }
        };
        (label(&k.buffer_send), label(&k.toggle_buffer))
    }

    /// 刷新或隐藏浮窗。
    pub(crate) fn push_buffer_panel(&self) {
        if !self.buffer.is_open() {
            let _ = self.ui_tx.send(UiCommand::HideBuffer);
            return;
        }
        let (send, toggle) = self.buffer_key_labels();
        let (text, status) = self.buffer.view(&send, &toggle);
        let (cx, cy, ch) = {
            let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            (s.caret_x, s.caret_y, s.caret_height)
        };
        let (caret_x, caret_y, caret_height, _) = self.resolve_caret_for_ui(cx, cy, ch);
        let _ = self.ui_tx.send(UiCommand::ShowBuffer(BufferView {
            text,
            status,
            caret_x,
            caret_y,
            caret_height,
        }));
    }

    /// `keys.toggle_buffer`。
    pub(crate) fn toggle_buffer(&self) {
        let private = crate::moodflow::input_privacy().private_field;
        let tip = {
            let mut g = self.buffer.lock();
            let cur = g.current;
            match (g.open, g.buf.state()) {
                (true, State::Ready) => {
                    g.open = false;
                    g.buf.suspend();
                    None
                }
                _ => match cur {
                    None => Some("没有可绑定的输入框"),
                    Some(t) => {
                        let field = if private {
                            FieldKind::Private
                        } else {
                            FieldKind::Normal
                        };
                        match g.buf.bind(t, field) {
                            Ok(()) => {
                                g.open = true;
                                None
                            }
                            Err(BindError::PrivateField) => {
                                g.open = false;
                                Some("密码框里不能用 Buffer")
                            }
                        }
                    }
                },
            }
        };
        if let Some(t) = tip {
            self.show_tip(t);
        }
        self.push_buffer_panel();
    }

    /// `keys.buffer_send`（按键上下文）。Buffer 没打开时返回 `None`，不吞键。
    pub(crate) fn buffer_send_keyed(&self) -> Option<KeyAction> {
        if !self.buffer.is_open() {
            return None;
        }
        if self.has_pending_composition() {
            self.show_tip("先确认组字，再发送 Buffer");
            return Some(KeyAction::Consumed);
        }
        match self.buffer.take_all() {
            Ok(text) => {
                self.buffer.delivering.store(true, Ordering::Relaxed);
                self.push_buffer_panel();
                let chinese_mode = self
                    .state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .chinese_mode;
                Some(KeyAction::InsertText {
                    text,
                    new_composition: None,
                    mode_changed: false,
                    chinese_mode,
                    has_new_composition: false,
                })
            }
            Err(e) => {
                self.show_tip(deliver_error_tip(e));
                Some(KeyAction::Consumed)
            }
        }
    }

    /// 按键动作出口：Buffer 打开且绑定有效时，把上屏正文截进 Buffer（见模块文档）。
    pub(crate) fn route_commit_to_buffer(&self, action: KeyAction) -> KeyAction {
        // 自己的投递：放行一次并清标记。
        if self.buffer.delivering.swap(false, Ordering::Relaxed) {
            return action;
        }
        let KeyAction::InsertText {
            text,
            new_composition,
            mode_changed,
            chinese_mode,
            has_new_composition,
        } = action
        else {
            return action;
        };
        let text = match self.buffer.capture(&text) {
            Capture::PassThrough => text,
            Capture::Captured => {
                self.push_buffer_panel();
                String::new()
            }
            Capture::Full => {
                self.show_tip("Buffer 已满，这段直接上屏了");
                text
            }
        };
        KeyAction::InsertText {
            text,
            new_composition,
            mode_changed,
            chinese_mode,
            has_new_composition,
        }
    }

    pub(crate) fn buffer_focus_gained(&self, token: u64) {
        self.buffer.focus_gained(token);
        if self.buffer.is_open() {
            self.push_buffer_panel();
        }
    }

    pub(crate) fn buffer_focus_lost(&self) {
        self.buffer.focus_lost();
        if self.buffer.is_open() {
            self.push_buffer_panel();
        }
    }

    /// 浮窗右键菜单。
    pub(crate) fn show_buffer_menu(&self, x: i32, y: i32) {
        let cmd = |c: MenuCmd| MenuKind::Command(c);
        let (empty, held) = {
            let g = self.buffer.lock();
            (g.buf.is_empty(), g.buf.state() == State::Held)
        };
        let mut items = vec![
            M::leaf("全部发送", cmd(MenuCmd::BufferSend), !empty, false),
            M::leaf(
                "删除最后一块",
                cmd(MenuCmd::BufferDeleteLast),
                !empty,
                false,
            ),
            M::leaf("清空", cmd(MenuCmd::BufferClear), !empty, false),
        ];
        if held {
            items.push(M::separator());
            items.push(M::label("上次发送结果不明，确认没有写进去后再发"));
        }
        items.push(M::separator());
        items.push(M::leaf(
            "关闭 Buffer（内容保留）",
            cmd(MenuCmd::BufferClose),
            true,
            false,
        ));
        self.mark_menu_open(0, String::new());
        let _ = self.ui_tx.send(UiCommand::ShowCandidateMenu {
            items,
            anchor: MenuAnchor::at_point(x, y),
        });
    }

    pub(crate) fn run_buffer_menu_cmd(&self, cmd: MenuCmd) {
        match cmd {
            MenuCmd::BufferSend => match self.buffer.take_all() {
                // 没有按键上下文：经 push 通道交给当前活动客户端（目标已在 take_all 里核对）。
                Ok(text) => self.push_commit_text(&text),
                Err(e) => self.show_tip(deliver_error_tip(e)),
            },
            MenuCmd::BufferDeleteLast => {
                let _ = self.buffer.lock().buf.delete_last_block();
            }
            MenuCmd::BufferClear => self.buffer.lock().buf.clear(),
            MenuCmd::BufferClose => {
                let mut g = self.buffer.lock();
                g.open = false;
                g.buf.suspend();
            }
            _ => {}
        }
        self.push_buffer_panel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn insert(text: &str) -> KeyAction {
        KeyAction::InsertText {
            text: text.into(),
            new_composition: None,
            mode_changed: false,
            chinese_mode: true,
            has_new_composition: false,
        }
    }

    fn text_of(a: &KeyAction) -> &str {
        match a {
            KeyAction::InsertText { text, .. } => text,
            _ => panic!("不是 InsertText：{a:?}"),
        }
    }

    fn coord() -> std::sync::Arc<Coordinator> {
        crate::moodflow::TEST_PRIVACY.with(|p| p.set(Some(Default::default())));
        Coordinator::new_headless_with_ui(moodflow_config::Config::default(), None).0
    }

    #[test]
    fn closed_buffer_passes_commits_through() {
        let c = coord();
        c.buffer_focus_gained(1);
        assert_eq!(text_of(&c.route_commit_to_buffer(insert("你好"))), "你好");
        assert!(c.buffer_send_keyed().is_none(), "没打开时发送键不吞");
    }

    #[test]
    fn open_buffer_captures_then_sends_once_to_the_bound_field() {
        let c = coord();
        c.buffer_focus_gained(1);
        c.toggle_buffer();
        assert!(c.buffer.is_open());
        assert_eq!(text_of(&c.route_commit_to_buffer(insert("你好"))), "");
        assert_eq!(text_of(&c.route_commit_to_buffer(insert("世界"))), "");
        let sent = c.buffer_send_keyed().unwrap();
        assert_eq!(text_of(&sent), "你好世界", "块之间不加空格");
        // 投递动作本身经过出口时不能被截回 Buffer。
        assert_eq!(text_of(&c.route_commit_to_buffer(sent)), "你好世界");
        assert!(c.buffer.lock().buf.is_empty());
        // 之后的上屏照常进 Buffer。
        assert_eq!(text_of(&c.route_commit_to_buffer(insert("再来"))), "");
    }

    #[test]
    fn focus_change_suspends_and_nothing_leaks_into_the_new_field() {
        let c = coord();
        c.buffer_focus_gained(1);
        c.toggle_buffer();
        c.route_commit_to_buffer(insert("给甲的"));
        c.buffer_focus_lost();
        c.buffer_focus_gained(2);
        // 暂停期间上屏直接进宿主，Buffer 内容不动、也发不出去。
        assert_eq!(
            text_of(&c.route_commit_to_buffer(insert("打给乙"))),
            "打给乙"
        );
        assert!(matches!(c.buffer_send_keyed(), Some(KeyAction::Consumed)));
        assert_eq!(c.buffer.lock().buf.text(), "给甲的");
        // 在乙这里重新绑定后才能发。
        c.toggle_buffer();
        assert_eq!(text_of(&c.buffer_send_keyed().unwrap()), "给甲的");
    }

    #[test]
    fn toggle_again_closes_but_keeps_content() {
        let c = coord();
        c.buffer_focus_gained(1);
        c.toggle_buffer();
        c.route_commit_to_buffer(insert("留着"));
        c.toggle_buffer();
        assert!(!c.buffer.is_open());
        assert_eq!(text_of(&c.route_commit_to_buffer(insert("直上"))), "直上");
        c.toggle_buffer();
        assert_eq!(c.buffer.lock().buf.text(), "留着");
    }

    #[test]
    fn password_field_refuses_to_bind() {
        let c = coord();
        crate::moodflow::TEST_PRIVACY.with(|p| {
            p.set(Some(crate::moodflow::InputPrivacy {
                private_field: true,
                ..Default::default()
            }))
        });
        c.buffer_focus_gained(1);
        c.toggle_buffer();
        assert!(!c.buffer.is_open());
        assert_eq!(text_of(&c.route_commit_to_buffer(insert("mima"))), "mima");
    }
}
