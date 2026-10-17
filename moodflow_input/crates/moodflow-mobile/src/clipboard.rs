//! 剪贴板历史的移动端门面：[`moodflow_clipboard::History`] + 落盘到用户目录的一个 JSON 文件。
//!
//! # 采集时机归宿主
//!
//! Android 10+ 只有获得焦点的默认输入法读得到剪贴板，所以宿主只在**键盘可见**期间把变化交进来，
//! 并报告 `ClipDescription.EXTRA_IS_SENSITIVE` 与「当前是不是私密字段」（`sensitive`）——
//! 见 `docs/design/xsime-feature-port-plan.md` §2.4。本层不知道这些，只负责收录、保留与落盘。
//!
//! # 日志
//!
//! 条目正文**绝不**进日志（AGENTS.md 日志规范）：这里的日志只有路径与错误位置。
//!
//! 计划里的长期存储是 redb（moodflow-store）；骨架期先用 crate 自带的 JSON 快照，
//! 换存储时只改本模块的 `load` / `save`。

use std::path::{Path, PathBuf};

use moodflow_clipboard::{History, Limits, Recorded, Source};

pub use moodflow_clipboard::{Entry, EntryKind};

/// 一次收录的结果（宿主据此决定要不要刷新面板）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordResult {
    New,
    Refreshed,
    Skipped,
}

/// 落盘的剪贴板历史。
#[derive(Debug)]
pub struct MobileClipboard {
    history: History,
    path: PathBuf,
}

impl MobileClipboard {
    /// 打开（或新建）历史文件。
    ///
    /// 文件损坏时**不删除**：原文件改名为 `*.bad` 留给用户，历史从空开始——
    /// 与用户词库同一条原则（XSIME 吸收计划 §3.2：不自动删除、不覆盖原数据）。
    pub fn open(path: &Path, now: u64) -> Self {
        let history = match std::fs::read_to_string(path) {
            Ok(text) => match History::from_json(&text) {
                Ok(mut h) => {
                    h.prune(now);
                    h
                }
                Err(e) => {
                    let bad = path.with_extension("bad");
                    let kept = std::fs::rename(path, &bad).is_ok();
                    tracing::warn!("剪贴板历史无法解析（{e}），已另存为 *.bad: {kept}");
                    History::new(Limits::default())
                }
            },
            Err(_) => History::new(Limits::default()),
        };
        Self {
            history,
            path: path.to_path_buf(),
        }
    }

    /// 收录一次复制。`sensitive` = 平台标记敏感或来自私密字段，此时不收录。
    pub fn record(&mut self, text: &str, sensitive: bool, now: u64) -> RecordResult {
        let r = self.history.record(text, Source { sensitive }, now);
        match r {
            Recorded::New(_) => {
                self.save();
                RecordResult::New
            }
            Recorded::Refreshed(_) => {
                self.save();
                RecordResult::Refreshed
            }
            Recorded::Skipped(_) => RecordResult::Skipped,
        }
    }

    /// 新到旧；`query` 非空时按子串过滤（不区分大小写）。
    pub fn entries(&self, query: &str) -> Vec<Entry> {
        self.history.search(query).into_iter().cloned().collect()
    }

    /// 只看链接或只看文本。
    pub fn entries_of(&self, link: bool) -> Vec<Entry> {
        let kind = if link {
            EntryKind::Link
        } else {
            EntryKind::Text
        };
        self.history.filter(kind).into_iter().cloned().collect()
    }

    pub fn set_pinned(&mut self, id: u64, pinned: bool) -> bool {
        let hit = self.history.set_pinned(id, pinned);
        if hit {
            self.save();
        }
        hit
    }

    pub fn remove(&mut self, id: u64) -> bool {
        let hit = self.history.remove(id);
        if hit {
            self.save();
        }
        hit
    }

    /// 清空未收藏的条目。
    pub fn clear_unpinned(&mut self) {
        self.history.clear_unpinned();
        self.save();
    }

    /// 原子写入（临时文件 + 改名）。失败只记警告：历史丢一次不该打断输入。
    pub fn save(&self) -> bool {
        let tmp = self.path.with_extension("tmp");
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let ok = std::fs::write(&tmp, self.history.to_json()).is_ok()
            && std::fs::rename(&tmp, &self.path).is_ok();
        if !ok {
            tracing::warn!("剪贴板历史写入失败: {}", self.path.display());
        }
        ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_file(name: &str) -> PathBuf {
        // 目录名带 pid：多 worktree / 多会话并行跑测试时固定名会互删夹具
        let dir = std::env::temp_dir().join(format!("moodflow_mobile_clip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        let _ = std::fs::remove_file(&p);
        p
    }

    #[test]
    fn record_persists_and_reloads() {
        let p = tmp_file("persist.json");
        let mut c = MobileClipboard::open(&p, 100);
        assert_eq!(c.record("你好", false, 100), RecordResult::New);
        assert_eq!(
            c.record("https://example.com", false, 101),
            RecordResult::New
        );
        assert_eq!(c.record("你好", false, 102), RecordResult::Refreshed);
        let c2 = MobileClipboard::open(&p, 103);
        let texts: Vec<String> = c2.entries("").into_iter().map(|e| e.text).collect();
        assert_eq!(texts, vec!["你好", "https://example.com"]);
        assert_eq!(c2.entries_of(true).len(), 1);
    }

    #[test]
    fn sensitive_is_never_recorded() {
        let p = tmp_file("sensitive.json");
        let mut c = MobileClipboard::open(&p, 0);
        assert_eq!(c.record("hunter2", true, 0), RecordResult::Skipped);
        assert!(c.entries("").is_empty());
        assert!(!p.exists(), "没有收录就不落盘");
    }

    #[test]
    fn corrupt_file_is_kept_aside_not_deleted() {
        let p = tmp_file("corrupt.json");
        std::fs::write(&p, "{ not json").unwrap();
        let c = MobileClipboard::open(&p, 0);
        assert!(c.entries("").is_empty());
        assert!(p.with_extension("bad").exists());
    }

    #[test]
    fn pinned_survives_clear() {
        let p = tmp_file("pin.json");
        let mut c = MobileClipboard::open(&p, 0);
        c.record("a", false, 0);
        c.record("b", false, 0);
        let id = c.entries("a")[0].id;
        assert!(c.set_pinned(id, true));
        c.clear_unpinned();
        let texts: Vec<String> = c.entries("").into_iter().map(|e| e.text).collect();
        assert_eq!(texts, vec!["a"]);
    }
}
