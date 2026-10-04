//! 改写用的“最近上屏”缓冲（10 第 3.1 节）：只在内存里，不经 XQP 发送，
//! 只在用户按下改写快捷键时随 `rewrite_req` 发出。

/// `rewrite_req.text` 上限（10 第 2.4 节）。
pub const MAX_CHARS: usize = 300;

const SENTENCE_END: &[char] = &['。', '！', '？', '…', '.', '!', '?'];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentText {
    pub text: String,
    /// 将要删除的原文长度，UTF-16 代码单元，与清风 `ReplaceBackward.count` 一致。
    pub utf16_len: u32,
}

#[derive(Debug, Default)]
pub(crate) struct Recent {
    text: String,
    chars: usize,
    /// 上一次上屏以句末标点结尾：下一次上屏另起一句。
    sealed: bool,
}

impl Recent {
    pub fn push(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if self.sealed {
            self.clear();
        }
        self.text.push_str(text);
        self.chars += text.chars().count();
        if self.chars > MAX_CHARS {
            // 只留最后 300 字：替换时从光标往回删的正是这一段
            let cut = self.chars - MAX_CHARS;
            let at = self.text.char_indices().nth(cut).map_or(0, |(i, _)| i);
            self.text.drain(..at);
            self.chars = MAX_CHARS;
        }
        self.sealed = text.trim_end().ends_with(SENTENCE_END);
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.chars = 0;
        self.sealed = false;
    }

    pub fn take(&mut self) -> Option<RecentText> {
        if self.text.trim().is_empty() {
            self.clear();
            return None;
        }
        let text = std::mem::take(&mut self.text);
        self.clear();
        let utf16_len = text.encode_utf16().count() as u32;
        Some(RecentText { text, utf16_len })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consecutive_commits_join_until_sentence_end() {
        let mut r = Recent::default();
        r.push("今天");
        r.push("好累。");
        assert_eq!(r.take().unwrap().text, "今天好累。");
        r.push("一句。");
        r.push("下一句");
        assert_eq!(r.take().unwrap().text, "下一句");
        assert!(r.take().is_none());
    }

    #[test]
    fn keeps_last_300_chars_and_counts_utf16() {
        let mut r = Recent::default();
        r.push(&"字".repeat(290));
        r.push(&"😀".repeat(20));
        let t = r.take().unwrap();
        assert_eq!(t.text.chars().count(), MAX_CHARS);
        assert!(t.text.ends_with("😀"));
        assert_eq!(t.utf16_len, 280 + 20 * 2);
    }

    #[test]
    fn clear_drops_everything() {
        let mut r = Recent::default();
        r.push("还没说完");
        r.clear();
        assert!(r.take().is_none());
    }
}
