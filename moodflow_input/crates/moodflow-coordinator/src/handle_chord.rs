//! 并击接入（Windows，XSIME 吸收计划 §2.3）：多键同按出拼音。
//!
//! - 配置 `[chord] enabled` + `keymap`（用户目录下的键位方案 JSON，格式见 `moodflow-chord`）。
//!   方案读不出或校验不过时不启用，并提示一次。
//! - 启用后把方案里所有键的 VK 经 `CONFIG_KEY_CHORD_KEYS` 推给 TSF：TSF 对这些键在**按下时**
//!   照常转发，并额外转发它们的**松开**（平时字母键的 keyup 不转发）。集合为空即并击关闭，
//!   TSF 行为与改造前逐字一致。
//! - 中文态、无修饰键、没有特殊模式时，并击键的按下只交给结算器（不出字母），一组键全部松开时
//!   结算出拼音片段，逐字母喂进正常的按键处理——候选、组字、上屏全走既有通路。
//!
//! 时间取进程内单调时钟（毫秒），结算器只关心差值。

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use moodflow_bridge::handler::{KeyAction, KeyEventData, MessageHandler};
use moodflow_chord::{ChordEvent, Keymap, Settler};
use moodflow_ipc::protocol::{EVENT_KEY_DOWN, EVENT_KEY_UP, MOD_ALT, MOD_CTRL, MOD_WIN};
use moodflow_ui_types::{ToastKind, ToastPosition, UiCommand};
use tracing::{debug, info, warn};

use crate::coordinator::Coordinator;

/// `'`（隔音符）键：VK_OEM_7。
const VK_QUOTE: u32 = 0xDE;

pub(crate) struct ChordRuntime {
    settler: Settler,
    /// VK → 方案里的键名。
    names: HashMap<u32, String>,
}

#[derive(Default)]
pub(crate) struct ChordState {
    runtime: Mutex<Option<ChordRuntime>>,
    /// 上次加载用的 `(enabled, keymap 路径)`；相同则不重载。
    loaded_with: Mutex<Option<(bool, String)>>,
    /// 正在把结算结果喂回按键处理：这期间的字母不能再被并击截走。
    feeding: AtomicBool,
}

fn now_ms() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis() as u64
}

/// 用一份已通过校验的方案构造运行态。键名认不出 VK 的键忽略（校验阶段已提示过）。
pub(crate) fn runtime_from(map: &Keymap) -> ChordRuntime {
    let mut names = HashMap::new();
    for k in map.left_keys.iter().chain(&map.right_keys) {
        if let Some(vk) = moodflow_keys::keymap::key_name_to_vk_with_letters(k) {
            names.insert(vk, k.clone());
        }
    }
    ChordRuntime {
        settler: Settler::new(map),
        names,
    }
}

/// 拼音片段里的字符 → VK（只认 a–z 与隔音符，其余丢弃）。
fn vk_of(c: char) -> Option<u32> {
    match c {
        'a'..='z' => Some(0x41 + (c as u32 - 'a' as u32)),
        'A'..='Z' => Some(c as u32),
        '\'' => Some(VK_QUOTE),
        _ => None,
    }
}

impl Coordinator {
    /// 按 `[chord]` 配置加载或卸下方案，并把并击键集合推给 TSF。
    pub(crate) fn sync_chord(&self) {
        let cfg = self.rt().config.chord.clone();
        let key = (cfg.enabled, cfg.keymap.clone());
        {
            let mut last = self
                .chord
                .loaded_with
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if last.as_ref() == Some(&key) {
                return;
            }
            *last = Some(key);
        }
        let runtime = if cfg.enabled {
            match self.load_chord_keymap(&cfg.keymap) {
                Ok(map) => {
                    info!(
                        "并击方案已加载：{}（{} 条映射）",
                        map.name,
                        map.mappings.len()
                    );
                    Some(runtime_from(&map))
                }
                Err(e) => {
                    warn!("并击方案不可用：{e}");
                    let _ = self.ui_tx.send(UiCommand::ShowToast {
                        text: format!("并击没有启用：{e}"),
                        position: ToastPosition::BottomCenter,
                        kind: ToastKind::Error,
                        duration_ms: 6000,
                        accent: None,
                    });
                    None
                }
            }
        } else {
            None
        };
        *self.chord.runtime.lock().unwrap_or_else(|e| e.into_inner()) = runtime;
        self.push_chord_keys_config(0);
    }

    fn load_chord_keymap(&self, name: &str) -> Result<Keymap, String> {
        let path = std::path::Path::new(name);
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.compat_dirs
                .1
                .as_ref()
                .ok_or("没有用户目录")?
                .join(path)
        };
        let text =
            std::fs::read_to_string(&path).map_err(|_| format!("读不到 {}", path.display()))?;
        let map = Keymap::from_json(&text).map_err(|e| format!("方案格式不对（{e:?}）"))?;
        map.validate_for_apply()
            .map_err(|e| format!("方案校验没通过（{e:?}）"))?;
        Ok(map)
    }

    /// 当前并击键的 VK 集合（未启用为空）。
    pub(crate) fn chord_vks(&self) -> Vec<u32> {
        let g = self.chord.runtime.lock().unwrap_or_else(|e| e.into_inner());
        let mut v: Vec<u32> = g
            .as_ref()
            .map(|r| r.names.keys().copied().collect())
            .unwrap_or_default();
        v.sort_unstable();
        v
    }

    /// 本次按键是否归并击：中文态、无 Ctrl/Alt/Win、没有特殊模式、不在回喂中、键属于方案。
    fn chord_key_name(&self, data: &KeyEventData) -> Option<String> {
        if self.chord.feeding.load(Ordering::Relaxed)
            || data.modifiers & (MOD_CTRL | MOD_ALT | MOD_WIN) != 0
        {
            return None;
        }
        let name = {
            let g = self.chord.runtime.lock().unwrap_or_else(|e| e.into_inner());
            g.as_ref()?.names.get(&data.key_code)?.clone()
        };
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        (s.chinese_mode && s.active.is_none()).then_some(name)
    }

    /// 并击键按下：交给结算器，不出字母。等待中的另一只手超时时这里可能结算出上一组。
    pub(crate) fn chord_key_down(&self, data: &KeyEventData) -> Option<KeyAction> {
        if data.event_type != EVENT_KEY_DOWN {
            return None;
        }
        let name = self.chord_key_name(data)?;
        let ev = {
            let mut g = self.chord.runtime.lock().unwrap_or_else(|e| e.into_inner());
            g.as_mut()?.settler.key_down(&name, now_ms())
        };
        Some(self.apply_chord_event(ev, data))
    }

    /// 并击键松开：一组键全部松开时结算并回喂。
    pub(crate) fn chord_key_up(&self, data: &KeyEventData) -> Option<KeyAction> {
        if data.event_type != EVENT_KEY_UP {
            return None;
        }
        let name = {
            let g = self.chord.runtime.lock().unwrap_or_else(|e| e.into_inner());
            g.as_ref()?.names.get(&data.key_code)?.clone()
        };
        let ev = {
            let mut g = self.chord.runtime.lock().unwrap_or_else(|e| e.into_inner());
            g.as_mut()?.settler.key_up(&name, now_ms())
        };
        Some(self.apply_chord_event(ev, data))
    }

    fn apply_chord_event(&self, ev: Option<ChordEvent>, data: &KeyEventData) -> KeyAction {
        match ev {
            Some(ChordEvent::Emit { output, .. }) => self.feed_pinyin(&output, data),
            Some(ChordEvent::NoMatch { keys }) => {
                debug!("并击：{} 个键的组合没有映射", keys.len());
                KeyAction::Consumed
            }
            None => KeyAction::Consumed,
        }
    }

    /// 把结算出的拼音逐字母喂进正常按键处理，返回最后一个字母的动作（组字区是累计态，
    /// 最后一次即全貌）。
    fn feed_pinyin(&self, output: &str, origin: &KeyEventData) -> KeyAction {
        self.chord.feeding.store(true, Ordering::Relaxed);
        let mut last = KeyAction::Consumed;
        for vk in output.chars().filter_map(vk_of) {
            let ev = KeyEventData {
                key_code: vk,
                scan_code: 0,
                modifiers: 0,
                event_type: EVENT_KEY_DOWN,
                // 带上原事件的锁定键快照：按键处理会据此校准 CapsLock 镜像。
                toggles: origin.toggles,
                event_seq: origin.event_seq,
                prev_char: 0,
            };
            last = MessageHandler::handle_key_event(self, &ev);
        }
        self.chord.feeding.store(false, Ordering::Relaxed);
        last
    }

    /// 把并击键集合推给 TSF（`client_token == 0` 广播）。
    pub fn push_chord_keys_config(&self, client_token: u64) {
        let value = moodflow_ipc::codec::encode_chord_keys_value(&self.chord_vks());
        let msg = moodflow_ipc::codec::encode_sync_config(
            moodflow_ipc::protocol::CONFIG_KEY_CHORD_KEYS,
            &value,
        );
        if client_token != 0 {
            self.push_server.push_to_token(client_token, &msg);
        } else {
            self.push_server.push_per_client(|_| msg.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEYMAP: &str = r#"{
        "version": 1,
        "name": "测试",
        "left_keys": ["a", "s"],
        "right_keys": ["j", "k"],
        "interval_ms": 300,
        "mappings": [
            {"keys": ["a", "j"], "output": "ni", "kind": "syllable"},
            {"keys": ["s", "k"], "output": "hao", "kind": "syllable"}
        ]
    }"#;

    fn key(vk: char, up: bool) -> KeyEventData {
        KeyEventData {
            key_code: vk as u32,
            scan_code: 0,
            modifiers: 0,
            event_type: if up { EVENT_KEY_UP } else { EVENT_KEY_DOWN },
            toggles: 0,
            event_seq: 0,
            prev_char: 0,
        }
    }

    fn coord_with_chord() -> std::sync::Arc<Coordinator> {
        let dir = std::env::temp_dir().join(format!("mf-chord-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("chord_keymap.json"), KEYMAP).unwrap();
        let mut cfg = moodflow_config::Config::default();
        cfg.chord.enabled = true;
        let (c, _rx) = Coordinator::new_headless_with_ui_at(cfg, None, Some(&dir));
        c.sync_chord();
        c
    }

    #[test]
    fn keymap_keys_become_the_tsf_forward_set() {
        let c = coord_with_chord();
        assert_eq!(c.chord_vks(), vec![0x41, 0x4A, 0x4B, 0x53]);
        let off = Coordinator::new_headless_with_ui(moodflow_config::Config::default(), None).0;
        off.sync_chord();
        assert!(off.chord_vks().is_empty(), "未启用时集合为空，TSF 行为不变");
    }

    #[test]
    fn chord_press_is_swallowed_and_release_feeds_pinyin() {
        let c = coord_with_chord();
        // 按下两键：都只进结算器。
        assert!(matches!(
            c.chord_key_down(&key('A', false)),
            Some(KeyAction::Consumed)
        ));
        assert!(matches!(
            c.chord_key_down(&key('J', false)),
            Some(KeyAction::Consumed)
        ));
        assert!(c.state.lock().unwrap().input_buffer.is_empty());
        // 先松一个：还没全松，不结算。
        assert!(matches!(
            c.chord_key_up(&key('A', true)),
            Some(KeyAction::Consumed)
        ));
        // 全松：结算出 ni 并喂进按键处理。
        c.chord_key_up(&key('J', true));
        assert_eq!(c.state.lock().unwrap().input_buffer, "ni");
    }

    #[test]
    fn non_chord_keys_and_modified_keys_are_not_taken() {
        let c = coord_with_chord();
        assert!(c.chord_key_down(&key('Z', false)).is_none());
        let mut ctrl_a = key('A', false);
        ctrl_a.modifiers = MOD_CTRL;
        assert!(c.chord_key_down(&ctrl_a).is_none());
    }

    #[test]
    fn missing_keymap_disables_chord() {
        let dir = std::env::temp_dir().join(format!("mf-chord-missing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut cfg = moodflow_config::Config::default();
        cfg.chord.enabled = true;
        let (c, rx) = Coordinator::new_headless_with_ui_at(cfg, None, Some(&dir));
        c.sync_chord();
        assert!(c.chord_vks().is_empty());
        assert!(rx.try_iter().any(
            |m| matches!(m, UiCommand::ShowToast { text, .. } if text.contains("并击没有启用"))
        ));
    }
}
