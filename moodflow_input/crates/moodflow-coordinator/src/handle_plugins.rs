//! 插件接入（ADR 0054，docs/design/plugin-system.md §3）：协调器经 `moodflow-plugin-registry`
//! 调用已启用的 WASM 插件。
//!
//! | 扩展点 | 接入位置 | 约束 |
//! |---|---|---|
//! | 候选来源 | [`Coordinator::update_candidates`] 主输入路（无特殊模式时） | **首选位置不让给插件**：插件候选排在第 2 位起，盲打按空格永远上屏引擎首选；与已有候选同文本的丢弃 |
//! | 上屏变换 | 每次按键动作出口（`handle_key_event_policed`，换行改写之前） | 只改最终写入文档的正文，组合区文本不动 |
//! | 命令函数 | 命令栏求值上下文的 `external_func` | 内置函数优先，插件不能遮蔽内置名 |
//!
//! 隐私：每次调用都带 [`crate::moodflow::input_privacy`]——密码框、安全桌面、无痕、名单应用里
//! 插件根本不被调用（宿主侧强制，不靠插件自觉）。
//!
//! 插件装在用户目录 `plugins/<id>/<版本>/`，私有数据在 `plugin-data/<id>/`；启用列表是
//! `plugins.enabled` 数组，`plugins.safe_mode` 一键全停。配置变化时只在启用列表或安全模式真的变了
//! 才重新加载，免得每次改别的设置都重建所有插件实例。

use std::sync::{Arc, Mutex};

use moodflow_candidate::Candidate;
use moodflow_plugin_registry::{
    InputContext, Limits, LoadOptions, Platform, PluginStatus, Registry,
};
use moodflow_ui_types::{ToastKind, ToastPosition, UiCommand};
use tracing::info;

use crate::coordinator::Coordinator;

/// 逐键扩展点每次调用的执行步数上限：wasmi 解释执行约每秒上亿步，50 万步约几毫秒，
/// 足够查表类插件，又不会让一个写坏的插件把按键拖慢到可感知。
const FUEL_PER_KEY: u64 = 500_000;
/// 一次最多并入多少条插件候选（防止插件刷屏把正常候选挤到后面几页）。
const MAX_PLUGIN_CANDIDATES: usize = 9;

#[cfg(windows)]
const PLATFORM: Platform = Platform::Windows;
#[cfg(not(windows))]
const PLATFORM: Platform = Platform::Android;

/// 协调器持有的插件状态。
#[derive(Default)]
pub(crate) struct PluginState {
    registry: Mutex<Registry>,
    /// 上次加载用的 `(enabled, safe_mode)`；相同则不重载。
    loaded_with: Mutex<Option<(Vec<String>, bool)>>,
}

fn to_ctx(p: crate::moodflow::InputPrivacy) -> InputContext {
    InputContext {
        private_field: p.private_field,
        secure_desktop: p.secure_desktop,
        incognito: p.incognito,
        app_blocked: p.app_blocked,
    }
}

/// 把插件候选并入候选列表：从第 2 位起插入（首选不动），与已有候选同文本的丢弃，最多
/// [`MAX_PLUGIN_CANDIDATES`] 条。插件之间按来源优先级、同来源内按 weight 从高到低。
pub(crate) fn merge_plugin_candidates(
    candidates: &mut Vec<Candidate>,
    from_plugins: Vec<(String, moodflow_plugin_registry::PluginCandidate)>,
) {
    if from_plugins.is_empty() {
        return;
    }
    // 来源顺序（registry 已按优先级排好）保持稳定，同来源内按 weight 降序。
    let mut order: Vec<String> = Vec::new();
    for (id, _) in &from_plugins {
        if !order.contains(id) {
            order.push(id.clone());
        }
    }
    let mut sorted = from_plugins;
    sorted.sort_by(|a, b| {
        let ra = order.iter().position(|x| *x == a.0);
        let rb = order.iter().position(|x| *x == b.0);
        ra.cmp(&rb).then(b.1.weight.cmp(&a.1.weight))
    });
    let mut picked: Vec<Candidate> = Vec::new();
    for (_, c) in sorted {
        if picked.len() >= MAX_PLUGIN_CANDIDATES {
            break;
        }
        if candidates.iter().any(|x| x.text == c.text) || picked.iter().any(|x| x.text == c.text) {
            continue;
        }
        picked.push(Candidate {
            text: c.text,
            comment: c.comment,
            weight: c.weight,
            ..Default::default()
        });
    }
    let at = candidates.len().min(1);
    candidates.splice(at..at, picked);
}

impl Coordinator {
    fn plugin_dirs(&self) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
        let user = self.compat_dirs.1.as_ref()?;
        Some((user.join("plugins"), user.join("plugin-data")))
    }

    /// 按 `[plugins]` 配置加载或重载插件。启用列表与安全模式都没变时什么也不做。
    pub(crate) fn sync_plugins(&self) {
        let cfg = self.rt().config.plugins.clone();
        let key = (cfg.enabled.clone(), cfg.safe_mode);
        {
            let mut last = self
                .plugins
                .loaded_with
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if last.as_ref() == Some(&key) {
                return;
            }
            *last = Some(key);
        }
        let Some((plugins_dir, data_dir)) = self.plugin_dirs() else {
            return;
        };
        let ui_tx = self.ui_tx.clone();
        let notifier: moodflow_plugin_registry::Notifier = Arc::new(move |id: &str, text: &str| {
            let _ = ui_tx.send(UiCommand::ShowToast {
                text: format!("{text}\n— 插件 {id}"),
                position: ToastPosition::BottomCenter,
                kind: ToastKind::Info,
                duration_ms: 4000,
                accent: None,
            });
        });
        let opts = LoadOptions {
            plugins_dir: &plugins_dir,
            data_dir: &data_dir,
            enabled: &cfg.enabled,
            host_version: env!("MOODFLOW_APP_VERSION"),
            platform: PLATFORM,
            safe_mode: cfg.safe_mode,
            limits: Limits {
                fuel_per_call: FUEL_PER_KEY,
                ..Limits::default()
            },
            notifier,
        };
        let (registry, report) = Registry::load(&opts);
        info!(
            "插件：已加载 {} 个，失败 {} 个，未启用 {} 个{}",
            report.loaded.len(),
            report.failed.len(),
            report.not_enabled.len(),
            if cfg.safe_mode {
                "（安全模式）"
            } else {
                ""
            }
        );
        if !report.failed.is_empty() {
            let names: Vec<&str> = report.failed.iter().map(|(id, _)| id.as_str()).collect();
            let _ = self.ui_tx.send(UiCommand::ShowToast {
                text: format!("有插件没能加载：{}", names.join("、")),
                position: ToastPosition::BottomCenter,
                kind: ToastKind::Error,
                duration_ms: 6000,
                accent: None,
            });
        }
        *self
            .plugins
            .registry
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = registry;
    }

    /// 主输入路的插件候选（见模块文档）。只在没有特殊模式、候选来源插件存在时调用插件。
    pub(crate) fn add_plugin_candidates(&self, state: &mut crate::coordinator::State) {
        if state.active.is_some() || state.input_buffer.is_empty() {
            return;
        }
        let mut reg = self
            .plugins
            .registry
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if !reg.has_candidate_providers() {
            return;
        }
        let ctx = to_ctx(crate::moodflow::input_privacy());
        let got = reg.candidates(&ctx, &state.input_buffer);
        drop(reg);
        merge_plugin_candidates(&mut state.candidates, got);
    }

    /// 上屏变换：按键动作里最终写入文档的正文依次交给变换插件。
    pub(crate) fn apply_plugin_transform(
        &self,
        action: moodflow_bridge::handler::KeyAction,
    ) -> moodflow_bridge::handler::KeyAction {
        let mut reg = self
            .plugins
            .registry
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if !reg.has_transforms() {
            return action;
        }
        let ctx = to_ctx(crate::moodflow::input_privacy());
        if !ctx.input_allowed() {
            return action;
        }
        crate::coordinator::message_handler::map_commit_text(action, |t| {
            if t.is_empty() {
                t
            } else {
                reg.transform(&ctx, &t)
            }
        })
    }

    /// 命令栏里内置没有的函数：交给声明了它的插件。
    pub(crate) fn plugin_cmd(
        &self,
        name: &str,
        args: &[String],
    ) -> Option<Result<String, moodflow_plugin_registry::CallError>> {
        self.plugins
            .registry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .cmd(name, args)
    }

    /// 已加载插件的状态（设置页与 RPC 用）。
    pub fn plugin_status(&self) -> Vec<PluginStatus> {
        self.plugins
            .registry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .status()
    }

    /// 重新启用一个被自动停用的插件。
    pub fn plugin_reenable(&self, id: &str) -> bool {
        self.plugins
            .registry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .reenable(id)
    }

    /// 插件装了或卸了：强制按当前配置重载。
    pub fn reload_plugins(&self) {
        *self
            .plugins
            .loaded_with
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = None;
        self.sync_plugins();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moodflow_plugin_registry::PluginCandidate;

    fn cand(text: &str) -> Candidate {
        Candidate {
            text: text.into(),
            ..Default::default()
        }
    }

    fn pc(id: &str, text: &str, weight: i32) -> (String, PluginCandidate) {
        (
            id.into(),
            PluginCandidate {
                text: text.into(),
                comment: String::new(),
                weight,
            },
        )
    }

    /// 装一个测试插件：候选来源返回固定的 😀，上屏变换把文本换成「变换后」，命令函数 `answer` 返回 42。
    fn install_test_plugin(user: &std::path::Path) {
        let dir = user.join("plugins").join("dev.test.all").join("1.0.0");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("manifest.toml"),
            r#"
id = "dev.test.all"
name = "测试"
version = "1.0.0"
sdk = 1
min_host_version = "0.1.0"
type = "wasm"
entry = "plugin.wasm"
platforms = ["windows", "android"]
license = "MIT"
[contributes]
candidate_provider = {}
commit_transform = {}
cmdbar_functions = ["answer"]
[capabilities]
"#,
        )
        .unwrap();
        let ret = |name: &str, off: u32, json: &str| {
            let esc = json.replace('"', "\\\"");
            format!(
                r#"(data (i32.const {off}) "{esc}")
  (func (export "{name}") (param i32 i32) (result i64)
    (i64.or (i64.shl (i64.const {off}) (i64.const 32)) (i64.const {len})))"#,
                len = json.len()
            )
        };
        let wasm = wat::parse_str(format!(
            r#"(module
  (memory (export "memory") 1)
  (global $h (mut i32) (i32.const 8192))
  (func (export "mf_alloc") (param $n i32) (result i32)
    (local $p i32) (local.set $p (global.get $h))
    (global.set $h (i32.add (global.get $h) (local.get $n))) (local.get $p))
  {}
  {}
  {}
)"#,
            ret(
                "mf_candidates",
                1024,
                r#"[{"text":"😀","comment":"笑","weight":5}]"#
            ),
            ret("mf_transform", 2048, r#"{"text":"变换后"}"#),
            ret("mf_cmd", 3072, r#"{"value":"42"}"#),
        ))
        .unwrap();
        std::fs::write(dir.join("plugin.wasm"), wasm).unwrap();
    }

    #[test]
    fn enabled_plugin_reaches_candidates_commit_and_cmdbar() {
        let user = std::env::temp_dir().join(format!("mf-coord-plugins-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&user);
        install_test_plugin(&user);

        // 未启用：不加载。
        let (c, _rx) = Coordinator::new_headless_with_ui_at(
            moodflow_config::Config::default(),
            None,
            Some(&user),
        );
        assert!(c.plugin_status().is_empty());

        crate::moodflow::TEST_PRIVACY.with(|p| p.set(Some(Default::default())));
        let mut cfg = moodflow_config::Config::default();
        cfg.plugins.enabled = vec!["dev.test.all".into()];
        let (c, _rx) = Coordinator::new_headless_with_ui_at(cfg, None, Some(&user));
        assert_eq!(c.plugin_status().len(), 1);
        {
            let mut s = c.state.lock().unwrap();
            s.input_buffer = "xiao".into();
            c.update_candidates(&mut s);
            assert!(
                s.candidates
                    .iter()
                    .any(|x| x.text == "😀" && x.comment == "笑")
            );
        }
        let action = moodflow_bridge::handler::KeyAction::InsertText {
            text: "原文".into(),
            new_composition: None,
            mode_changed: false,
            chinese_mode: true,
            has_new_composition: false,
        };
        match c.apply_plugin_transform(action) {
            moodflow_bridge::handler::KeyAction::InsertText { text, .. } => {
                assert_eq!(text, "变换后")
            }
            other => panic!("动作类型不应变化：{other:?}"),
        }
        assert_eq!(c.plugin_cmd("answer", &[]), Some(Ok("42".into())));
        assert_eq!(c.plugin_cmd("nope", &[]), None);
        let _ = std::fs::remove_dir_all(&user);
    }

    fn texts(v: &[Candidate]) -> Vec<&str> {
        v.iter().map(|c| c.text.as_str()).collect()
    }

    #[test]
    fn first_candidate_never_moves() {
        let mut list = vec![cand("笑"), cand("小")];
        merge_plugin_candidates(&mut list, vec![pc("a", "😀", 100)]);
        assert_eq!(texts(&list), vec!["笑", "😀", "小"]);
    }

    #[test]
    fn empty_frame_gets_plugin_candidates_first() {
        let mut list = Vec::new();
        merge_plugin_candidates(&mut list, vec![pc("a", "😀", 1)]);
        assert_eq!(texts(&list), vec!["😀"]);
    }

    #[test]
    fn duplicates_dropped_source_order_kept_and_capped() {
        let mut list = vec![cand("笑"), cand("小")];
        let mut got = vec![
            pc("hi", "小", 99),
            pc("hi", "甲", 1),
            pc("hi", "乙", 5),
            pc("lo", "丙", 1000),
        ];
        got.extend((0..20).map(|i| pc("lo", &format!("x{i}"), 0)));
        merge_plugin_candidates(&mut list, got);
        assert_eq!(&texts(&list)[..5], &["笑", "乙", "甲", "丙", "x0"]);
        assert_eq!(
            list.len(),
            2 + MAX_PLUGIN_CANDIDATES,
            "已有的「小」不重复，插件最多 9 条"
        );
    }
}
