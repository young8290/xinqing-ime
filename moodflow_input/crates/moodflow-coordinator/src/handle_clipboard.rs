//! 剪贴板历史接入（XSIME 吸收计划 §2.4）：采集、落盘、面板、粘贴。
//!
//! - **采集**：Windows 由 UI 线程的剪贴板监听（`moodflow-ui::clipboard_monitor`）产生
//!   `UiEvent::ClipboardChanged`；Android 由宿主在键盘可见期间读剪贴板后经 `inject_ui_event` 注入。
//!   `clipboard.enabled` 关闭时不监听；隐私环境（密码框、安全桌面、无痕、名单应用）下的复制不记。
//! - **落盘**：用户目录 `clipboard_history.json`，每次变化后整份重写（条数有上限，文件很小）。
//! - **面板**：`keys.clipboard_panel` 打开一个菜单：最近的条目（收藏的在前），点一下即粘贴到
//!   当前输入框；另有收藏、删除子菜单与「清空（保留收藏）」。复用弹出菜单而不是新窗口——
//!   键盘导航、鼠标、DPI、主题都已现成。
//! - **正文不进日志**（AGENTS.md 日志规范）。

use std::path::PathBuf;
use std::sync::Mutex;

use moodflow_clipboard::{History, Limits, Recorded, Source};
use moodflow_config::ClipboardConfig;
use moodflow_ui_types::{MenuAnchor, MenuCmd, MenuItemSpec as M, MenuKind, UiCommand};
use tracing::{debug, warn};

use crate::coordinator::Coordinator;

/// 面板里最多列多少条。
const PANEL_MAX: usize = 30;
/// 菜单项标签最多多少个字符。
const LABEL_MAX: usize = 32;
const FILE_NAME: &str = "clipboard_history.json";

/// 协调器持有的剪贴板历史。
pub(crate) struct ClipboardState {
    history: Mutex<History>,
    path: Option<PathBuf>,
    /// 最近一次打开面板时「菜单序号 → 条目 id」：菜单开着期间剪贴板可能又变了，按序号会点错。
    menu_ids: Mutex<Vec<u64>>,
}

fn limits_of(cfg: &ClipboardConfig) -> Limits {
    Limits {
        max_entries: cfg.max_entries.max(1),
        retention_days: cfg.retention_days,
        ..Limits::default()
    }
}

fn now_secs() -> u64 {
    crate::coordinator::now_unix_secs().max(0) as u64
}

impl ClipboardState {
    pub(crate) fn open(user_dir: Option<&std::path::Path>, cfg: &ClipboardConfig) -> Self {
        let path = user_dir.map(|d| d.join(FILE_NAME));
        let mut history = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| match History::from_json(&t) {
                Ok(h) => Some(h),
                Err(e) => {
                    warn!("剪贴板历史文件损坏，按空历史处理: 行 {}", e.line);
                    None
                }
            })
            .unwrap_or_else(|| History::new(limits_of(cfg)));
        history.set_limits(limits_of(cfg), now_secs());
        Self {
            history: Mutex::new(history),
            path,
            menu_ids: Mutex::new(Vec::new()),
        }
    }

    fn save(&self, h: &History) {
        let Some(p) = &self.path else {
            return;
        };
        let tmp = p.with_extension("json.tmp");
        if std::fs::write(&tmp, h.to_json())
            .and_then(|_| std::fs::rename(&tmp, p))
            .is_err()
        {
            warn!("剪贴板历史写盘失败");
        }
    }

    /// 收录一次复制。返回是否新增或刷新了条目。
    pub(crate) fn record(&self, text: &str, sensitive: bool) -> bool {
        let mut h = self.history.lock().unwrap_or_else(|e| e.into_inner());
        let r = h.record(text, Source { sensitive }, now_secs());
        let changed = matches!(r, Recorded::New(_) | Recorded::Refreshed(_));
        if changed {
            self.save(&h);
        }
        changed
    }

    pub(crate) fn set_limits(&self, cfg: &ClipboardConfig) {
        let mut h = self.history.lock().unwrap_or_else(|e| e.into_inner());
        h.set_limits(limits_of(cfg), now_secs());
        self.save(&h);
    }

    /// 面板要列的条目：收藏的在前，各自按最近使用排序。`(id, 文本, 是否收藏)`。
    pub(crate) fn panel_entries(&self) -> Vec<(u64, String, bool)> {
        let h = self.history.lock().unwrap_or_else(|e| e.into_inner());
        let mut out: Vec<(u64, String, bool)> = h
            .entries()
            .iter()
            .filter(|e| e.pinned)
            .chain(h.entries().iter().filter(|e| !e.pinned))
            .take(PANEL_MAX)
            .map(|e| (e.id, e.text.clone(), e.pinned))
            .collect();
        out.truncate(PANEL_MAX);
        out
    }

    fn text_of(&self, id: u64) -> Option<String> {
        let h = self.history.lock().unwrap_or_else(|e| e.into_inner());
        h.get(id).map(|e| e.text.clone())
    }

    fn toggle_pin(&self, id: u64) {
        let mut h = self.history.lock().unwrap_or_else(|e| e.into_inner());
        let pinned = h.get(id).is_some_and(|e| e.pinned);
        if h.set_pinned(id, !pinned) {
            self.save(&h);
        }
    }

    fn remove(&self, id: u64) {
        let mut h = self.history.lock().unwrap_or_else(|e| e.into_inner());
        if h.remove(id) {
            self.save(&h);
        }
    }

    fn clear_unpinned(&self) {
        let mut h = self.history.lock().unwrap_or_else(|e| e.into_inner());
        h.clear_unpinned();
        self.save(&h);
    }

    fn id_at(&self, menu_index: u16) -> Option<u64> {
        self.menu_ids
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(menu_index as usize)
            .copied()
    }
}

/// 菜单标签：单行、去首尾空白、超长截断。
pub(crate) fn panel_label(text: &str) -> String {
    let one_line: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut label: String = one_line.chars().take(LABEL_MAX).collect();
    if one_line.chars().count() > LABEL_MAX {
        label.push('…');
    }
    label
}

impl Coordinator {
    /// 扩展类功能随配置同步：剪贴板监听、插件注册表。构造后与每次配置重载后调用。
    pub(crate) fn sync_extensions(&self) {
        self.sync_clipboard_monitor();
        self.sync_plugins();
        self.sync_chord();
    }

    /// 按当前配置开关剪贴板监听。
    pub(crate) fn sync_clipboard_monitor(&self) {
        let cfg = self.rt().config.clipboard.clone();
        self.clipboard.set_limits(&cfg);
        let _ = self.ui_tx.send(UiCommand::SetClipboardMonitor(cfg.enabled));
    }

    /// 剪贴板变化（UI 线程监听或移动端宿主注入）。
    pub(crate) fn on_clipboard_changed(&self, text: &str, sensitive: bool) {
        if !self.rt().config.clipboard.enabled {
            return;
        }
        // 当前输入焦点处于隐私环境时，这次复制多半来自那里（密码框里全选复制），一律不记。
        if !crate::moodflow::input_privacy().input_allowed() {
            debug!("剪贴板变化发生在隐私环境，不记录");
            return;
        }
        if self.clipboard.record(text, sensitive) {
            debug!("剪贴板历史：收录一条（{} 字）", text.chars().count());
        }
    }

    /// 打开剪贴板历史面板（`keys.clipboard_panel`）。
    pub(crate) fn show_clipboard_panel(&self) {
        let cmd = |c: MenuCmd| MenuKind::Command(c);
        let enabled = self.rt().config.clipboard.enabled;
        let entries = self.clipboard.panel_entries();
        *self
            .clipboard
            .menu_ids
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = entries.iter().map(|e| e.0).collect();

        let mut items = Vec::new();
        if !enabled {
            items.push(M::label("剪贴板历史未开启（设置项 clipboard.enabled）"));
        }
        if entries.is_empty() {
            items.push(M::label("（还没有记录）"));
        }
        for (i, (_, text, pinned)) in entries.iter().enumerate() {
            let label = if *pinned {
                format!("★ {}", panel_label(text))
            } else {
                panel_label(text)
            };
            items.push(M::leaf(
                label,
                cmd(MenuCmd::ClipboardPaste(i as u16)),
                true,
                false,
            ));
        }
        if !entries.is_empty() {
            items.push(M::separator());
            let pins = entries
                .iter()
                .enumerate()
                .map(|(i, (_, t, p))| {
                    M::leaf(
                        panel_label(t),
                        cmd(MenuCmd::ClipboardTogglePin(i as u16)),
                        true,
                        *p,
                    )
                })
                .collect();
            items.push(M::submenu("收藏 / 取消收藏", pins));
            let dels = entries
                .iter()
                .enumerate()
                .map(|(i, (_, t, _))| {
                    M::leaf(
                        panel_label(t),
                        cmd(MenuCmd::ClipboardRemove(i as u16)),
                        true,
                        false,
                    )
                })
                .collect();
            items.push(M::submenu("删除", dels));
            items.push(M::leaf(
                "清空（保留收藏）",
                cmd(MenuCmd::ClipboardClear),
                true,
                false,
            ));
        }

        let anchor = {
            let (cx, cy, ch) = {
                let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
                (s.caret_x, s.caret_y, s.caret_height)
            };
            let (x, y, h, valid) = self.resolve_caret_for_ui(cx, cy, ch);
            if valid {
                MenuAnchor::at_point(x, y + h)
            } else {
                MenuAnchor::at_point(i32::MIN, i32::MIN)
            }
        };
        self.mark_menu_open(0, String::new());
        let _ = self
            .ui_tx
            .send(UiCommand::ShowCandidateMenu { items, anchor });
    }

    /// 剪贴板面板里的菜单命令。
    pub(crate) fn run_clipboard_menu_cmd(&self, cmd: MenuCmd) {
        match cmd {
            MenuCmd::ClipboardPaste(i) => {
                if let Some(text) = self
                    .clipboard
                    .id_at(i)
                    .and_then(|id| self.clipboard.text_of(id))
                {
                    // 粘贴是用户在面板上的显式动作：隐私环境里也照办（文本是用户自己选的），
                    // 只是不进任何采集。
                    self.push_commit_text(&text);
                }
            }
            MenuCmd::ClipboardTogglePin(i) => {
                if let Some(id) = self.clipboard.id_at(i) {
                    self.clipboard.toggle_pin(id);
                }
            }
            MenuCmd::ClipboardRemove(i) => {
                if let Some(id) = self.clipboard.id_at(i) {
                    self.clipboard.remove(id);
                }
            }
            MenuCmd::ClipboardClear => self.clipboard.clear_unpinned(),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("mf-clip-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn labels_are_single_line_and_truncated() {
        assert_eq!(panel_label("  a\n b\tc  "), "a b c");
        let long = "字".repeat(40);
        let l = panel_label(&long);
        assert_eq!(l.chars().count(), LABEL_MAX + 1);
        assert!(l.ends_with('…'));
    }

    #[test]
    fn records_persist_and_sensitive_is_skipped() {
        let dir = tmp("persist");
        let cfg = ClipboardConfig::default();
        let s = ClipboardState::open(Some(&dir), &cfg);
        assert!(s.record("第一条", false));
        assert!(!s.record("密码", true));
        assert!(!s.record("   ", false));
        assert!(s.record("第二条", false));
        let again = ClipboardState::open(Some(&dir), &cfg);
        let texts: Vec<String> = again.panel_entries().into_iter().map(|e| e.1).collect();
        assert_eq!(texts, vec!["第二条".to_string(), "第一条".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pinned_entries_come_first_and_survive_clear() {
        let dir = tmp("pin");
        let s = ClipboardState::open(Some(&dir), &ClipboardConfig::default());
        s.record("旧", false);
        s.record("新", false);
        let old_id = s.panel_entries()[1].0;
        s.toggle_pin(old_id);
        assert_eq!(s.panel_entries()[0].1, "旧");
        s.clear_unpinned();
        let left: Vec<String> = s.panel_entries().into_iter().map(|e| e.1).collect();
        assert_eq!(left, vec!["旧".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn menu_index_maps_to_entry_id_snapshot() {
        let s = ClipboardState::open(None, &ClipboardConfig::default());
        s.record("甲", false);
        s.record("乙", false);
        *s.menu_ids.lock().unwrap() = s.panel_entries().iter().map(|e| e.0).collect();
        // 面板开着时又复制了一条：序号仍指向打开面板那一刻的条目。
        s.record("丙", false);
        assert_eq!(
            s.id_at(0).and_then(|id| s.text_of(id)).as_deref(),
            Some("乙")
        );
        assert_eq!(s.id_at(9), None);
    }

    #[test]
    fn corrupt_file_falls_back_to_empty() {
        let dir = tmp("corrupt");
        std::fs::write(dir.join(FILE_NAME), "{坏").unwrap();
        let s = ClipboardState::open(Some(&dir), &ClipboardConfig::default());
        assert!(s.panel_entries().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
