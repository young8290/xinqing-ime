//! Buffer 的移动端门面（设计见 `docs/design/buffer.md` §3 的 Android 一列）。
//!
//! 在 [`moodflow_buffer::Buffer`] 外面只多做一件事：**替宿主保管在途投递的凭据**。
//! `DeliveryToken` 是不透明类型，跨 JNI/Swift 边界要么造一份可伪造的整数投影，要么留在这一侧——
//! 留在这一侧，宿主手里就不存在「拿旧凭据投到新目标」的可能。
//!
//! Android 的失效语义（与 XSIME Android 一致）：换字段、隐藏键盘即 [`MobileBuffer::discard`]；
//! 同一字段重进（`onStartInput` 修订号变化）只 [`MobileBuffer::focus_changed`] 暂停，需重新绑定。
//! 「哪种情况算换字段」由宿主判断，本层只提供动作。

use moodflow_buffer::{Buffer, Delivery, DeliveryToken, FieldKind, Outcome};

pub use moodflow_buffer::{
    BindError, DEFAULT_CAPACITY_UTF16, DeliverError, EditError, SendKeyAction, State, Target,
};

/// 宿主报告的写入结果，与 [`moodflow_buffer::Outcome`] 一一对应。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteResult {
    /// `commitText` 返回 true
    Accepted,
    /// `commitText` 返回 false：保留可重试
    Rejected,
    /// 连接已断开、无法确认：保留并暂停投递，等用户确认（防重复上屏）
    Unknown,
}

/// 移动端 Buffer：一个键盘服务实例一个。
#[derive(Debug, Default)]
pub struct MobileBuffer {
    inner: Buffer,
    /// 在途投递：凭据与它所属的目标
    pending: Option<(DeliveryToken, Target)>,
}

impl MobileBuffer {
    pub fn new(capacity_utf16: usize) -> Self {
        Self {
            inner: Buffer::new(capacity_utf16),
            pending: None,
        }
    }

    /// 绑定当前字段。私密字段返回 `Err` 且撤销原有绑定。
    pub fn bind(&mut self, target: Target, private: bool) -> Result<(), BindError> {
        self.pending = None;
        let field = if private {
            FieldKind::Private
        } else {
            FieldKind::Normal
        };
        self.inner.bind(target, field)
    }

    /// 焦点落到 `target`：离开绑定目标即暂停（回到同一目标也不自动恢复）。
    pub fn focus_changed(&mut self, target: Target) {
        self.inner.focus_changed(target);
        self.drop_stale_pending();
    }

    /// 撤销授权但保留内容。
    pub fn suspend(&mut self) {
        self.inner.suspend();
        self.pending = None;
    }

    /// 丢弃内容与绑定（Android 换字段、隐藏键盘、服务销毁）。
    pub fn discard(&mut self) {
        self.inner.discard();
        self.pending = None;
    }

    pub fn release_hold(&mut self) {
        self.inner.release_hold();
    }

    pub fn push_phrase(&mut self, text: &str) -> Result<(), EditError> {
        self.inner.push_phrase(text).map(|_| ())
    }

    pub fn push_latin(&mut self, ch: char) -> Result<(), EditError> {
        self.inner.push_latin(ch).map(|_| ())
    }

    pub fn backspace(&mut self) -> Result<(), EditError> {
        self.inner.backspace()
    }

    pub fn delete_last_block(&mut self) -> Result<(), EditError> {
        self.inner.delete_last_block()
    }

    pub fn clear(&mut self) {
        self.inner.clear();
        self.pending = None;
    }

    /// 各块原文（界面逐块绘制；块间的视觉间隔不是内容）。
    pub fn blocks(&self) -> Vec<String> {
        self.inner
            .blocks()
            .iter()
            .map(|b| b.text().to_owned())
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// 全部内容（复制兜底）。
    pub fn text(&self) -> String {
        self.inner.text()
    }

    pub fn len_utf16(&self) -> usize {
        self.inner.len_utf16()
    }

    pub fn state(&self) -> State {
        self.inner.state()
    }

    pub fn send_key_action(&self, composing: bool) -> SendKeyAction {
        self.inner.send_key_action(composing)
    }

    /// 取出下一块（`all = false`）或冻结的全部内容（`all = true`），返回要写入的文本。
    /// 凭据留在本层；宿主写入前调 [`Self::is_authorized`]，写完调 [`Self::finish`]。
    pub fn begin(&mut self, target: Target, all: bool) -> Result<String, DeliverError> {
        let Delivery { token, text, .. } = if all {
            self.inner.begin_all(target)?
        } else {
            self.inner.begin_next(target)?
        };
        self.pending = Some((token, target));
        Ok(text)
    }

    /// 写入前复核：在途投递仍属于 `target` 且授权未被撤销。
    pub fn is_authorized(&self, target: Target) -> bool {
        self.pending
            .is_some_and(|(token, t)| t == target && self.inner.is_authorized(token, target))
    }

    /// 报告写入结果。没有在途投递（已被撤销）时返回 `false`。
    pub fn finish(&mut self, result: WriteResult) -> bool {
        let Some((token, _)) = self.pending.take() else {
            return false;
        };
        let outcome = match result {
            WriteResult::Accepted => Outcome::Accepted,
            WriteResult::Rejected => Outcome::Rejected,
            WriteResult::Unknown => Outcome::Unknown,
        };
        self.inner.finish(token, outcome).is_ok()
    }

    /// 内层已作废在途投递（暂停、换焦点）时，同步丢掉本层保管的凭据。
    fn drop_stale_pending(&mut self) {
        if let Some((token, target)) = self.pending
            && !self.inner.is_authorized(token, target)
        {
            self.pending = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: Target = Target::new(1, 1);
    const A2: Target = Target::new(1, 2);
    const B: Target = Target::new(2, 1);

    fn bound() -> MobileBuffer {
        let mut b = MobileBuffer::default();
        b.bind(A, false).unwrap();
        b
    }

    #[test]
    fn deliver_next_then_accept_removes_block() {
        let mut b = bound();
        b.push_phrase("你好").unwrap();
        b.push_phrase("世界").unwrap();
        assert_eq!(b.begin(A, false).unwrap(), "你好");
        assert!(b.is_authorized(A));
        assert!(b.finish(WriteResult::Accepted));
        assert_eq!(b.blocks(), vec!["世界".to_string()]);
    }

    #[test]
    fn rejected_keeps_text_for_retry() {
        let mut b = bound();
        b.push_phrase("你好").unwrap();
        b.begin(A, true).unwrap();
        assert!(b.finish(WriteResult::Rejected));
        assert_eq!(b.text(), "你好");
        assert_eq!(b.begin(A, true).unwrap(), "你好");
    }

    #[test]
    fn unknown_holds_until_released() {
        let mut b = bound();
        b.push_phrase("你好").unwrap();
        b.begin(A, true).unwrap();
        assert!(b.finish(WriteResult::Unknown));
        assert_eq!(b.state(), State::Held);
        assert_eq!(b.begin(A, true), Err(DeliverError::Held));
        b.release_hold();
        assert_eq!(b.begin(A, true).unwrap(), "你好");
    }

    #[test]
    fn field_restart_revokes_in_flight_delivery() {
        let mut b = bound();
        b.push_phrase("你好").unwrap();
        b.begin(A, true).unwrap();
        // 同一字段重进：修订号变了，旧投递作废，不能写进新上下文
        b.focus_changed(A2);
        assert!(!b.is_authorized(A2));
        assert!(!b.is_authorized(A));
        assert!(!b.finish(WriteResult::Accepted));
        assert_eq!(b.text(), "你好", "内容保留");
        assert_eq!(b.state(), State::Suspended);
        assert_eq!(b.begin(A2, true), Err(DeliverError::Suspended));
        // 显式重新绑定后才可投递
        b.bind(A2, false).unwrap();
        assert_eq!(b.begin(A2, true).unwrap(), "你好");
    }

    #[test]
    fn discard_on_field_change_never_delivers_to_new_field() {
        let mut b = bound();
        b.push_phrase("旧内容").unwrap();
        b.begin(A, true).unwrap();
        b.discard();
        assert!(b.is_empty());
        assert!(!b.is_authorized(B));
        assert_eq!(b.begin(B, true), Err(DeliverError::NotBound));
    }

    #[test]
    fn private_field_refuses_binding() {
        let mut b = bound();
        assert_eq!(b.bind(B, true), Err(BindError::PrivateField));
        assert_eq!(b.state(), State::Unbound);
    }

    #[test]
    fn latin_backspace_is_per_char() {
        let mut b = bound();
        b.push_latin('h').unwrap();
        b.push_latin('i').unwrap();
        b.backspace().unwrap();
        assert_eq!(b.blocks(), vec!["h".to_string()]);
        assert_eq!(b.send_key_action(true), SendKeyAction::ConfirmComposition);
        assert_eq!(b.send_key_action(false), SendKeyAction::SendNext);
    }
}
