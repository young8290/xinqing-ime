//! moodflow-plugin-registry：已安装插件的注册表（docs/design/plugin-system.md §3、§5）。
//!
//! 协调器只经这里接触插件：
//!
//! - [`Registry::load`]：从 `plugins_dir/<id>/<版本>/` 加载**已启用**（`plugins.enabled` 数组，
//!   不用 map 子键——AGENTS.md「出厂预置的绑定/开关」）、支持当前平台、未被安全模式停用的插件。
//!   一个插件加载失败只记进 [`LoadReport`]，不影响其他插件与输入。
//! - [`Registry::candidates`]：汇总各候选来源插件的候选，按清单 `priority` 从高到低排列来源；
//!   候选顺序仍以 `weight` 为准，由协调器并入候选列表时落到 weight（AGENTS.md 硬约定）。
//! - [`Registry::transform`]：上屏变换按 `priority` 串联。
//! - [`Registry::cmd`]：命令直通车函数；内置函数优先，插件只补内置没有的名字（协调器保证）。
//!
//! 隐私闸门在插件宿主里：[`InputContext`] 不允许输入时插件根本不被调用。
//! 插件私有存储落在 `data_dir/<id>/storage.json`，按插件隔离，单个插件总量有上限。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub use moodflow_plugin_api::{Manifest, Platform};
pub use moodflow_plugin_host::{CallError, InputContext, Limits, PluginCandidate};
use moodflow_plugin_host::{HostServices, PluginInstance};
use serde::Serialize;

/// 单个插件私有存储的总字节上限。
pub const STORAGE_LIMIT_BYTES: usize = 1024 * 1024;
/// 存储键的最大长度。
pub const STORAGE_KEY_MAX: usize = 256;

/// 插件发出的提示（toast），由协调器转给界面。插件 id 与文本。
pub type Notifier = Arc<dyn Fn(&str, &str) + Send + Sync>;

/// 一个已加载插件的状态，给设置页与 RPC 用。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PluginStatus {
    pub id: String,
    pub name: String,
    pub version: String,
    /// 因连续失败被自动停用。
    pub disabled: bool,
    pub candidate_provider: bool,
    pub commit_transform: bool,
    pub cmdbar_functions: Vec<String>,
}

/// 加载结果。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct LoadReport {
    pub loaded: Vec<String>,
    /// `(id, 原因)`。
    pub failed: Vec<(String, String)>,
    /// 已安装但未启用的插件 id。
    pub not_enabled: Vec<String>,
}

struct Loaded {
    priority: u8,
    inst: PluginInstance,
}

/// 注册表。按扩展点优先级排好序。
#[derive(Default)]
pub struct Registry {
    plugins: Vec<Loaded>,
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registry")
            .field("plugins", &self.status())
            .finish()
    }
}

/// 加载参数。
pub struct LoadOptions<'a> {
    pub plugins_dir: &'a Path,
    pub data_dir: &'a Path,
    pub enabled: &'a [String],
    pub host_version: &'a str,
    pub platform: Platform,
    /// 安全模式：所有第三方插件都不加载（§5）。
    pub safe_mode: bool,
    pub limits: Limits,
    pub notifier: Notifier,
}

impl Registry {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn load(opts: &LoadOptions<'_>) -> (Self, LoadReport) {
        let mut report = LoadReport::default();
        let mut plugins = Vec::new();
        if opts.safe_mode {
            return (Self { plugins }, report);
        }
        for (id, version) in moodflow_market::installed(opts.plugins_dir) {
            if !opts.enabled.contains(&id) {
                report.not_enabled.push(id);
                continue;
            }
            let dir = opts.plugins_dir.join(&id).join(&version);
            match load_one(&dir, &id, opts) {
                Ok(loaded) => {
                    report.loaded.push(id);
                    plugins.push(loaded);
                }
                Err(e) => {
                    tracing::warn!(plugin = %id, error = %e, "插件加载失败");
                    report.failed.push((id, e));
                }
            }
        }
        // 优先级高的在前；同优先级按 id 排，结果稳定。
        plugins.sort_by(|a: &Loaded, b: &Loaded| {
            b.priority
                .cmp(&a.priority)
                .then_with(|| a.inst.manifest().id.cmp(&b.inst.manifest().id))
        });
        (Self { plugins }, report)
    }

    /// 测试与内置插件用：直接放入一个已加载实例。
    pub fn insert(&mut self, inst: PluginInstance) {
        let priority = priority_of(inst.manifest());
        let pos = self
            .plugins
            .iter()
            .position(|p| p.priority < priority)
            .unwrap_or(self.plugins.len());
        self.plugins.insert(pos, Loaded { priority, inst });
    }

    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    pub fn status(&self) -> Vec<PluginStatus> {
        self.plugins
            .iter()
            .map(|p| {
                let m = p.inst.manifest();
                PluginStatus {
                    id: m.id.clone(),
                    name: m.name.clone(),
                    version: m.version.clone(),
                    disabled: p.inst.is_disabled(),
                    candidate_provider: m.contributes.candidate_provider.is_some(),
                    commit_transform: m.contributes.commit_transform.is_some(),
                    cmdbar_functions: m.contributes.cmdbar_functions.clone(),
                }
            })
            .collect()
    }

    /// 用户手动重新启用一个被自动停用的插件。
    pub fn reenable(&mut self, id: &str) -> bool {
        match self.plugins.iter_mut().find(|p| p.inst.manifest().id == id) {
            Some(p) => {
                p.inst.reenable();
                true
            }
            None => false,
        }
    }

    pub fn has_candidate_providers(&self) -> bool {
        self.plugins
            .iter()
            .any(|p| p.inst.manifest().contributes.candidate_provider.is_some())
    }

    /// 汇总候选。单个插件失败（超时、陷阱、坏输出）只丢掉它这一次的结果。
    /// 返回 `(插件 id, 候选)`，来源按优先级从高到低。
    pub fn candidates(&mut self, ctx: &InputContext, code: &str) -> Vec<(String, PluginCandidate)> {
        if code.is_empty() || !ctx.input_allowed() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for p in &mut self.plugins {
            if p.inst.manifest().contributes.candidate_provider.is_none() || p.inst.is_disabled() {
                continue;
            }
            match p.inst.candidates(ctx, code) {
                Ok(list) => {
                    let id = p.inst.manifest().id.clone();
                    out.extend(
                        list.into_iter()
                            .filter(|c| !c.text.is_empty())
                            .map(|c| (id.clone(), c)),
                    );
                }
                Err(e) => {
                    tracing::debug!(plugin = %p.inst.manifest().id, error = %e, "候选来源调用失败")
                }
            }
        }
        out
    }

    /// 上屏变换：按优先级串联；某个插件失败就跳过它，用上一步的结果继续。
    pub fn transform(&mut self, ctx: &InputContext, text: &str) -> String {
        let mut cur = text.to_owned();
        if !ctx.input_allowed() {
            return cur;
        }
        for p in &mut self.plugins {
            if p.inst.manifest().contributes.commit_transform.is_none() || p.inst.is_disabled() {
                continue;
            }
            match p.inst.transform(ctx, &cur) {
                Ok(t) => cur = t,
                Err(e) => {
                    tracing::debug!(plugin = %p.inst.manifest().id, error = %e, "上屏变换调用失败")
                }
            }
        }
        cur
    }

    pub fn has_transforms(&self) -> bool {
        self.plugins
            .iter()
            .any(|p| p.inst.manifest().contributes.commit_transform.is_some())
    }

    /// 插件提供的命令函数名（去重，按优先级）。
    pub fn cmd_names(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for p in &self.plugins {
            for n in &p.inst.manifest().contributes.cmdbar_functions {
                if !names.contains(n) {
                    names.push(n.clone());
                }
            }
        }
        names
    }

    /// 命令函数：交给第一个声明了这个名字的插件。没有插件声明时返回 `None`。
    pub fn cmd(&mut self, name: &str, args: &[String]) -> Option<Result<String, CallError>> {
        let p = self.plugins.iter_mut().find(|p| {
            p.inst
                .manifest()
                .contributes
                .cmdbar_functions
                .iter()
                .any(|n| n == name)
        })?;
        Some(p.inst.cmd(name, args))
    }
}

fn priority_of(m: &Manifest) -> u8 {
    let c = &m.contributes;
    [
        c.candidate_provider,
        c.commit_transform,
        c.candidate_filter,
        c.assoc_source,
    ]
    .iter()
    .flatten()
    .map(|p| p.priority)
    .max()
    .unwrap_or(0)
}

fn load_one(dir: &Path, id: &str, opts: &LoadOptions<'_>) -> Result<Loaded, String> {
    let text = std::fs::read_to_string(dir.join("manifest.toml")).map_err(|e| e.to_string())?;
    let manifest = Manifest::from_toml(&text, opts.host_version).map_err(|e| e.to_string())?;
    if manifest.id != id {
        return Err(format!("目录名与清单 id 不一致：{}", manifest.id));
    }
    if !manifest.supports(opts.platform) {
        return Err("不支持当前平台".into());
    }
    let moodflow_plugin_api::Kind::Wasm { entry } = &manifest.kind else {
        // 内容包不经注册表执行，由各自的加载器读取（§2.1）。
        return Err("内容包不由注册表加载".into());
    };
    let wasm = std::fs::read(dir.join(entry)).map_err(|e| e.to_string())?;
    let host = FileHost::new(opts.data_dir.join(id), opts.notifier.clone());
    let priority = priority_of(&manifest);
    let inst = PluginInstance::load(manifest, &wasm, opts.limits, Box::new(host))
        .map_err(|e| e.to_string())?;
    Ok(Loaded { priority, inst })
}

/// 落盘的宿主能力实现：每个插件一个 `storage.json`。
struct FileHost {
    dir: PathBuf,
    cache: Option<BTreeMap<String, String>>,
    notifier: Notifier,
}

impl FileHost {
    fn new(dir: PathBuf, notifier: Notifier) -> Self {
        Self {
            dir,
            cache: None,
            notifier,
        }
    }

    fn file(&self) -> PathBuf {
        self.dir.join("storage.json")
    }

    fn map(&mut self) -> &mut BTreeMap<String, String> {
        if self.cache.is_none() {
            let loaded = std::fs::read_to_string(self.file())
                .ok()
                .and_then(|t| serde_json::from_str(&t).ok())
                .unwrap_or_default();
            self.cache = Some(loaded);
        }
        self.cache.get_or_insert_with(BTreeMap::new)
    }
}

impl HostServices for FileHost {
    fn storage_get(&mut self, _plugin: &str, key: &str) -> Option<Vec<u8>> {
        self.map().get(key).map(|v| v.as_bytes().to_vec())
    }

    fn storage_set(&mut self, _plugin: &str, key: &str, value: &[u8]) -> bool {
        if key.is_empty() || key.len() > STORAGE_KEY_MAX {
            return false;
        }
        // 值按 UTF-8 文本存（JSON 文件可读、可迁移）；二进制由插件自行编码。
        let Ok(value) = std::str::from_utf8(value) else {
            return false;
        };
        let mut next = self.map().clone();
        next.insert(key.to_owned(), value.to_owned());
        let Ok(text) = serde_json::to_string(&next) else {
            return false;
        };
        if text.len() > STORAGE_LIMIT_BYTES {
            return false;
        }
        if std::fs::create_dir_all(&self.dir).is_err() {
            return false;
        }
        let tmp = self.dir.join("storage.json.tmp");
        if std::fs::write(&tmp, &text).is_err() || std::fs::rename(&tmp, self.file()).is_err() {
            return false;
        }
        self.cache = Some(next);
        true
    }

    fn notify(&mut self, plugin: &str, text: &str) {
        // 限长：一条提示最多 200 字，防止插件刷屏。
        let short: String = text.chars().take(200).collect();
        (self.notifier)(plugin, &short);
    }
}

#[cfg(test)]
mod tests;
