//! Moodflow：把协调器的按键、上屏、焦点、组字、候选事件交给 `moodflow-tap`
//! （产品书 17 第 1.2 节，任务 A-04；FR-SEN-01～04）。
//!
//! Tap 是进程级单例，服务启动时由 [`start`] 安装。没安装时（单元测试、headless、移动端）
//! 所有钩子都是一次 `OnceLock::get` 后直接返回，不改变 Moodflow 原有行为。
//! 钩子只调 Tap 的非阻塞方法（闸门判断 + `try_send`），可以在 `state` 锁内调用。
//!
//! Hub 守护（A-06，17 第 1.5 节）也在这里装：`start` 时起 `mf-hub-guard`，配置热重载时更新
//! 是否需要 Hub。下行的 `mood`、`badge`、`pending` 记进 [`hub_view`]，菜单与工具栏从那里读。
//! 输入法里的 Moodflow 入口（A-07）：主菜单“Moodflow”分组、`tip` 的光标旁气泡、工具栏天气按钮。
//! 温柔改写模式（A-08）在子模块 [`rewrite`]。

pub(crate) mod perf;
mod rewrite;

use std::cell::Cell;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

use moodflow_bridge::handler::KeyEventData;
use moodflow_config::MoodflowConfig;
use moodflow_ipc::protocol::{EVENT_KEY_DOWN, MOD_SHORTCUT};
use moodflow_keys::keymap::{
    VK_0, VK_9, VK_A, VK_BACK, VK_BACKSLASH, VK_BACKTICK, VK_CAPITAL, VK_COMMA, VK_DELETE, VK_DOWN,
    VK_END, VK_EQUAL, VK_ESCAPE, VK_HOME, VK_LBRACKET, VK_LEFT, VK_LSHIFT, VK_MINUS, VK_NEXT,
    VK_PERIOD, VK_PRIOR, VK_QUOTE, VK_RBRACKET, VK_RETURN, VK_RIGHT, VK_SEMICOLON, VK_SLASH,
    VK_SPACE, VK_UP, VK_Z,
};
use moodflow_store::stats::CommitSource;
use moodflow_tap::guard::{ExeLauncher, GuardState, HubGuard};
use moodflow_tap::{
    CandOp, CommitHook, CompOp, Down, Endpoint, FocusHook, KeyHook, KeyKind, MoodState, OpenTarget,
    PIPE_NAME, PIPE_NAME_DEV, PauseBy, Scope, Tap, TapConfig,
};

use crate::coordinator::Coordinator;
use moodflow_ui_types::{
    MenuCmd, MenuItemSpec, MenuKind, MoodflowCell, MoodflowWeather, ToastKind, ToastPosition,
};

pub(crate) use rewrite::{RewriteMode, debug_phase as debug_rewrite_phase, replay_if_exited};

use crate::input_diag::InputDiagReason;
use crate::key_convert::numpad_char;

/// 设了这个环境变量（如 `127.0.0.1:18765`）时改在本机 TCP 上监听，供 Hub 的同名变量联调；
/// 非 Windows 平台只能这样启用。
pub const MFP_TCP_ENV: &str = "MF_MFP_TCP";

/// 设了这个环境变量时从这里拉起 Hub（开发联调）；否则取核心可执行文件旁边的 [`HUB_EXE`]
/// （03 第 5.1 节安装目录）。
pub const HUB_EXE_ENV: &str = "MF_HUB_EXE";
#[cfg(windows)]
pub const HUB_EXE: &str = "moodflow_hub.exe";
#[cfg(not(windows))]
pub const HUB_EXE: &str = "moodflow_hub";

static TAP: OnceLock<Arc<Tap>> = OnceLock::new();
/// Moodflow：当前焦点所在的进程名（小写，空串表示未知）。温柔改写判断“只插入不替换”名单用（ADR 0041）。
static FOCUS_APP: Mutex<String> = Mutex::new(String::new());

/// Moodflow：当前焦点所在的进程名（小写，空串表示未知）。
pub(crate) fn focus_app() -> String {
    FOCUS_APP.lock().unwrap_or_else(|e| e.into_inner()).clone()
}
/// 下行回调要回到协调器（记住 Hub 发来的无痕状态）；弱引用，不延长协调器寿命。
static COORD: OnceLock<Weak<Coordinator>> = OnceLock::new();
static GUARD: OnceLock<Arc<HubGuard>> = OnceLock::new();
static HUB_VIEW: Mutex<HubView> = Mutex::new(HubView {
    mood: None,
    offline: false,
    badge: false,
    pending: 0,
});

/// 光标旁气泡 10 秒内最多 1 条（FR-ENT-03）。
const TIP_GAP: Duration = Duration::from_secs(10);
static LAST_TIP: Mutex<Option<Instant>> = Mutex::new(None);

/// Hub 下发的、要在输入法界面上显示的状态（10 第 2.5 节）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HubView {
    /// 工具栏天气按钮（FR-ENT-02）；还没收到过时 `None`。
    pub mood: Option<MoodState>,
    /// AI 服务不可用，天气按钮显示离线样式。
    pub offline: bool,
    /// 有未读暖心话，天气按钮右上角小圆点（FR-CMF-04）。
    pub badge: bool,
    /// 待确认的日程与待办数（FR-ENT-01 菜单项）。
    pub pending: u32,
}

/// 启动 MFP 服务端（10 第 2.1 节）。`pipe_suffix` 取 `moodflow_config::variant::pipe_suffix()`，
/// dev 构建用 `moodflow_tap_dev`。失败只记日志：Moodflow 组件出问题不能影响打字。
///
/// 总开关与应用名单取自 `[moodflow]` 配置；`moodflow.remember_pause` 打开时恢复上次的无痕状态。
pub fn start(c: &Arc<Coordinator>, ime_ver: &str, pipe_suffix: &str) {
    if TAP.get().is_some() {
        return;
    }
    attach(c);
    let endpoint = match std::env::var(MFP_TCP_ENV) {
        Ok(addr) => match addr.parse::<SocketAddr>() {
            Ok(a) => Endpoint::Tcp(a),
            Err(e) => {
                tracing::warn!("{MFP_TCP_ENV}={addr} 无效：{e}");
                return;
            }
        },
        Err(_) => {
            let name = if pipe_suffix.is_empty() {
                PIPE_NAME
            } else {
                PIPE_NAME_DEV
            };
            Endpoint::Pipe(name.to_string())
        }
    };
    let mf = c.rt().config.moodflow.clone();
    let mut cfg = TapConfig::new(ime_ver, endpoint);
    cfg.enabled = mf.enabled;
    cfg.app_blocklist = mf.app_blocklist.clone();
    cfg.app_allowlist = mf.app_allowlist.clone();
    match Tap::start(cfg, Box::new(downlink)) {
        Ok(tap) => {
            if mf.remember_pause && c.moodflow_stored_pause() {
                // 还没有连接；Hub 握手后 Tap 会补发 pause_changed
                tap.set_paused(true, PauseBy::Menu);
            }
            let probe = Arc::clone(&tap);
            let _ = TAP.set(tap);
            tracing::info!("Moodflow MFP 服务端已启动");
            start_guard(probe, mf.enabled && mf.hub_autostart);
        }
        Err(e) => tracing::warn!("Moodflow MFP 服务端启动失败：{e}"),
    }
}

/// 起 Hub 守护（FR-OPS-03）。Hub 程序找不到也照常起：拉不起来就按失败计数，次数用完停下，
/// 菜单显示“Moodflow 组件未运行，点击重试”。
fn start_guard(tap: Arc<Tap>, wanted: bool) {
    let path = hub_exe_path();
    tracing::debug!("Moodflow Hub 路径：{}", path.display());
    match HubGuard::start(
        Box::new(move || tap.is_linked()),
        Box::new(ExeLauncher { path }),
        wanted,
    ) {
        Ok(g) => {
            watch_link(&g);
            let _ = GUARD.set(g);
        }
        Err(e) => tracing::warn!("Moodflow Hub 守护线程启动失败：{e}"),
    }
}

fn hub_exe_path() -> PathBuf {
    if let Some(p) = std::env::var_os(HUB_EXE_ENV) {
        return PathBuf::from(p);
    }
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join(HUB_EXE)))
        .unwrap_or_else(|| PathBuf::from(HUB_EXE))
}

/// 配置热重载后调用：总开关与应用名单即时生效。
pub(crate) fn apply_config(cfg: &MoodflowConfig) {
    if let Some(t) = tap() {
        t.set_app_lists(&cfg.app_blocklist, &cfg.app_allowlist);
        t.set_enabled(cfg.enabled);
    }
    // 总开关关掉时 Tap 发 bye{disabled} 并停止监听，守护不再拉起；打开时立即拉起（FR-IME-02）
    if let Some(g) = GUARD.get() {
        g.set_wanted(cfg.enabled && cfg.hub_autostart);
    }
}

/// Hub 连上或断开时刷新工具栏：天气按钮跟着变灰或恢复。连上后 Hub 补发的 `mood` 也会再刷一次；
/// 这里主要管没发 `bye` 就断开的情况（Hub 崩溃）。测试自己装守护时也调它。
pub fn watch_link(g: &HubGuard) {
    g.on_link_change(Box::new(|_| refresh_toolbar()));
}

/// 下行状态变了，重画工具栏天气按钮。
fn refresh_toolbar() {
    if let Some(c) = COORD.get().and_then(Weak::upgrade) {
        c.notify_toolbar();
    }
}

/// `mood` → 天气图标（04 第 3.1 节）。还没收到过时显示晴。
pub fn weather_of(mood: Option<MoodState>) -> MoodflowWeather {
    match mood {
        Some(MoodState::Hesitant) => MoodflowWeather::Cloudy,
        Some(MoodState::Low) => MoodflowWeather::Rain,
        Some(MoodState::Agitated) => MoodflowWeather::Storm,
        Some(MoodState::Tired) => MoodflowWeather::Night,
        Some(MoodState::Fluent | MoodState::Unknown) | None => MoodflowWeather::Clear,
    }
}

/// 菜单“Moodflow 组件未运行，点击重试”。
pub(crate) fn retry_hub() {
    if let Some(g) = GUARD.get() {
        g.retry();
    }
}

/// 菜单“Moodflow”分组里请 Hub 打开的窗口，下标即 `MenuCmd::MoodflowOpen` 的参数。
pub const OPEN_TARGETS: [OpenTarget; 4] = [
    OpenTarget::Chat,
    OpenTarget::Dashboard,
    OpenTarget::Schedule,
    OpenTarget::Settings,
];

/// 菜单“和晴晴聊聊”等：请 Hub 打开窗口（MFP `open`）。Hub 没连着时菜单项是灰的。
pub(crate) fn open(idx: u8) {
    if let (Some(t), Some(target)) = (tap(), OPEN_TARGETS.get(usize::from(idx))) {
        t.send_open(*target);
    }
}

/// 守护线程当前状态；没装时 `None`。
pub fn guard_state() -> Option<GuardState> {
    GUARD.get().map(|g| g.state())
}

/// Hub 下发的界面状态；Hub 没连着时一律是默认值（天气按钮显示未连接）。
pub fn hub_view() -> HubView {
    if tap().is_some_and(|t| t.is_linked()) {
        *HUB_VIEW.lock().unwrap_or_else(|e| e.into_inner())
    } else {
        HubView::default()
    }
}

/// 切换无痕模式，返回切换后的状态；没装 Tap 时 `None`。
pub(crate) fn toggle_pause(by: PauseBy) -> Option<bool> {
    let t = tap()?;
    let on = !t.paused();
    t.set_paused(on, by);
    Some(on)
}

/// 让下行回调能找回协调器（Hub 发来的无痕、提示气泡）。`start` 里已调用；测试自己装 Tap 时调。
pub fn attach(c: &Arc<Coordinator>) {
    let _ = COORD.set(Arc::downgrade(c));
}

/// 测试用：安装一个已启动的 Tap。进程内只能装一次，已装过返回假。
pub fn install(tap: Arc<Tap>) -> bool {
    TAP.set(tap).is_ok()
}

/// 测试用：安装一个已启动的 Hub 守护。进程内只能装一次。
pub fn install_guard(g: Arc<HubGuard>) -> bool {
    GUARD.set(g).is_ok()
}

/// 核心退出前调用：发 `bye{shutdown}`。
pub fn shutdown() {
    // 先停守护：核心退出不该再拉起 Hub（Hub 自己留着，C-PLT-12）
    if let Some(g) = GUARD.get() {
        g.stop();
    }
    if let Some(t) = TAP.get() {
        t.shutdown();
    }
}

fn tap() -> Option<&'static Arc<Tap>> {
    TAP.get()
}

impl Coordinator {
    /// state.toml 里记着的无痕状态（`moodflow.remember_pause` 打开时启动恢复用）。
    fn moodflow_stored_pause(&self) -> bool {
        moodflow_config::Config::state_dir()
            .map(|d| moodflow_config::RuntimeState::load(&d).moodflow_paused)
            .unwrap_or(false)
    }

    /// 记下当前无痕状态。不管 `remember_pause` 开没开都记，开关只决定启动时用不用它：
    /// 这样之后再打开「记住」，恢复的也是最后一次的真实状态，而不是很久以前的旧值。
    pub(crate) fn moodflow_store_pause(&self, on: bool) {
        self.state_writer
            .schedule("moodflow_pause", move |rs| rs.moodflow_paused = on);
    }

    /// 菜单“开启 / 关闭 Moodflow 功能”（FR-IME-02）：写用户配置并即时生效。关掉时 Tap 发
    /// `bye{disabled}`、停止监听，守护不再拉起 Hub；打开时立即拉起。
    pub(crate) fn moodflow_toggle_enabled(&self) {
        let next = !self.rt().config.moodflow.enabled;
        if let Err(e) = moodflow_config::Config::set_user_value(
            &["moodflow", "enabled"],
            toml::Value::Boolean(next),
        ) {
            tracing::warn!("写入 moodflow.enabled 失败：{e}");
        }
        self.refresh_config_in_memory(|c| c.moodflow.enabled = next);
        // 天气按钮跟着出现或消失
        self.notify_toolbar();
        self.show_toast(
            if next {
                "已开启 Moodflow 功能"
            } else {
                "已关闭 Moodflow 功能"
            },
            ToastPosition::BottomCenter,
            ToastKind::Info,
        );
    }

    /// Hub 的 `tip`（FR-ENT-03）：只在没有组字时显示，10 秒内最多 1 条，时长限定 1.5–2.5 秒。
    fn moodflow_tip(&self, text: &str, ms: u32) {
        if self.has_active_session() {
            tracing::debug!("Moodflow 提示：正在组字，不显示");
            return;
        }
        {
            let mut last = LAST_TIP.lock().unwrap_or_else(|e| e.into_inner());
            let now = Instant::now();
            if last.is_some_and(|t| now.duration_since(t) < TIP_GAP) {
                tracing::debug!("Moodflow 提示：10 秒内已显示过，跳过");
                return;
            }
            *last = Some(now);
        }
        // 10 第 2.5 节：≤ 16 字。Hub 发送前已校验，这里只防万一
        let text: String = text.chars().take(16).collect();
        self.show_moodflow_tip(&text, u64::from(ms.clamp(1500, 2500)));
    }

    /// 工具栏天气按钮的状态（FR-ENT-02）；Moodflow 没启动或总开关关着时 `None`，这一格不画。
    /// Hub 没连上或正在无痕时淡显；有未读暖心话时加小圆点。不取 `state` 锁。
    pub(crate) fn moodflow_toolbar_cell(&self) -> Option<MoodflowCell> {
        let t = tap()?;
        if !self.rt().config.moodflow.enabled {
            return None;
        }
        let linked = t.is_linked();
        let v = hub_view();
        Some(MoodflowCell {
            weather: weather_of(v.mood),
            dot: v.badge,
            dim: !linked || t.paused(),
        })
    }

    /// 左键天气按钮：打开和晴晴的对话。Hub 没连上时改为重新拉起它，免得点了没反应。
    pub(crate) fn moodflow_weather_click(&self) {
        let Some(t) = tap() else {
            return;
        };
        if t.is_linked() {
            t.send_open(OpenTarget::Chat);
        } else {
            retry_hub();
            self.show_toast(
                "Moodflow 组件未运行，正在重新启动",
                ToastPosition::BottomCenter,
                ToastKind::Info,
            );
        }
    }

    /// 右键天气按钮的小菜单：暂停 / 恢复感知、情绪看板（FR-ENT-02）。没装 Moodflow 时 `None`，
    /// 回落主菜单。
    pub(crate) fn moodflow_weather_menu(&self) -> Option<Vec<MenuItemSpec>> {
        let t = tap()?;
        let pause = if t.paused() {
            "恢复感知"
        } else {
            "暂停感知"
        };
        Some(vec![
            MenuItemSpec::leaf(
                pause.to_string(),
                MenuKind::Command(MenuCmd::MoodflowTogglePause),
                true,
                false,
            ),
            MenuItemSpec::leaf(
                "情绪看板".to_string(),
                MenuKind::Command(MenuCmd::MoodflowOpen(1)),
                t.is_linked(),
                false,
            ),
        ])
    }

    /// 主菜单“Moodflow”分组（FR-ENT-01）。Moodflow 没启动时为空；总开关关着时只剩“开启 Moodflow 功能”；
    /// Hub 没连上时，除“暂停感知”和总开关外都置灰，并多一项“Moodflow 组件未运行，点击重试”。
    pub(crate) fn moodflow_menu_group(&self) -> Vec<MenuItemSpec> {
        let Some(t) = tap() else {
            return Vec::new();
        };
        let cmd = |c: MenuCmd| MenuKind::Command(c);
        let leaf = |label: &str, c: MenuCmd, enabled: bool| {
            MenuItemSpec::leaf(label.to_string(), cmd(c), enabled, false)
        };
        if !self.rt().config.moodflow.enabled {
            return vec![
                leaf("开启 Moodflow 功能", MenuCmd::MoodflowToggleEnabled, true),
                MenuItemSpec::separator(),
            ];
        }
        let linked = t.is_linked();
        let pending = hub_view().pending;
        let schedule = if pending > 0 {
            format!("待确认日程与待办（{pending}）")
        } else {
            "待确认日程与待办".to_string()
        };
        let mut v = vec![
            leaf("和晴晴聊聊", MenuCmd::MoodflowOpen(0), linked),
            leaf("情绪看板", MenuCmd::MoodflowOpen(1), linked),
            leaf(&schedule, MenuCmd::MoodflowOpen(2), linked),
            leaf(
                if t.paused() {
                    "恢复感知"
                } else {
                    "暂停感知"
                },
                MenuCmd::MoodflowTogglePause,
                true,
            ),
            leaf("Moodflow 设置", MenuCmd::MoodflowOpen(3), linked),
            leaf("关闭 Moodflow 功能", MenuCmd::MoodflowToggleEnabled, true),
        ];
        if !linked {
            v.push(leaf(
                "Moodflow 组件未运行，点击重试",
                MenuCmd::MoodflowRetryHub,
                true,
            ));
        }
        v.push(MenuItemSpec::separator());
        v
    }

    /// 菜单与 Ctrl+Alt+P 的共同出口（FR-SEN-05/06）：切换、记住、气泡提示。
    /// Moodflow 没启动时什么都不做，返回 `None`。
    pub(crate) fn moodflow_toggle_pause(&self, by: PauseBy) -> Option<bool> {
        let on = toggle_pause(by)?;
        self.moodflow_store_pause(on);
        // 天气按钮跟着淡显或恢复
        self.notify_toolbar();
        self.show_toast(
            if on {
                "Moodflow 已暂停感知"
            } else {
                "Moodflow 已恢复感知"
            },
            ToastPosition::BottomCenter,
            ToastKind::Info,
        );
        Some(on)
    }
}

/// Tap 的下行回调（服务启动时由 [`start`] 传入；测试自己起 Tap 时也传它）。
pub fn downlink(msg: Down) {
    match msg {
        Down::Pause { on } => {
            if let Some(c) = COORD.get().and_then(Weak::upgrade) {
                // 小组件右键等 Hub 侧入口切的无痕，与菜单、快捷键一样按需记住
                c.moodflow_store_pause(on);
                c.notify_toolbar();
            }
        }
        Down::Mood { state, offline } => {
            {
                let mut v = HUB_VIEW.lock().unwrap_or_else(|e| e.into_inner());
                // “未知”保持上一状态（04 第 3.1 节）
                if state != MoodState::Unknown {
                    v.mood = Some(state);
                }
                v.offline = offline;
            }
            refresh_toolbar();
        }
        Down::Badge { on } => {
            HUB_VIEW.lock().unwrap_or_else(|e| e.into_inner()).badge = on;
            refresh_toolbar();
        }
        Down::Pending { count } => {
            HUB_VIEW.lock().unwrap_or_else(|e| e.into_inner()).pending = count;
        }
        Down::Bye { .. } => {
            // Hub 正常退出（用户在 Hub 里点了退出），不当崩溃重拉
            if let Some(g) = GUARD.get() {
                g.hub_quit();
            }
            *HUB_VIEW.lock().unwrap_or_else(|e| e.into_inner()) = HubView::default();
            refresh_toolbar();
        }
        Down::Tip { text, ms } => {
            if let Some(c) = COORD.get().and_then(Weak::upgrade) {
                c.moodflow_tip(&text, ms);
            }
        }
        Down::RewriteResult {
            req_id,
            style,
            cands,
        } => {
            if let Some(c) = COORD.get().and_then(Weak::upgrade) {
                c.moodflow_rewrite_result(req_id, style, cands);
            }
        }
        Down::RewriteFail { req_id, reason } => {
            if let Some(c) = COORD.get().and_then(Weak::upgrade) {
                c.moodflow_rewrite_fail(req_id, reason);
            }
        }
        _ => {}
    }
}

/// 一次按键前后的组字快照：编码长度（字符数）与候选页码。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CompSnap {
    pub len: usize,
    pub page: usize,
    /// Moodflow：全拼组字串里有切不出来的音节（R1b，ADR 0041）。双拼、码表方案恒为 false。
    pub invalid: bool,
}

/// 取当前组字快照；没装 Tap 时返回 `None`，调用方据此跳过钩子、不取锁。
pub(crate) fn snap(c: &crate::coordinator::Coordinator) -> Option<CompSnap> {
    tap()?;
    let _t = perf::hook_timer();
    let s = c.state.lock().unwrap_or_else(|e| e.into_inner());
    let full_pinyin = matches!(
        c.engine_mgr.current_engine_type(),
        Some(moodflow_engine::EngineType::Pinyin)
    ) && !c.engine_mgr.pinyin_is_shuangpin();
    Some(CompSnap {
        len: s.input_buffer.chars().count(),
        page: s.current_page,
        invalid: full_pinyin
            && moodflow_engine::pinyin::syllable::has_invalid_syllable(&s.input_buffer),
    })
}

/// 单按的修饰键（含左右之分的具体键码与 Win 键）。keymap 只收了左右 Shift / Ctrl 四个，
/// 改写模式吃键期间 C++ 会把其余几个也转发过来，这里补全。
pub(crate) fn is_modifier_vk(vk: u32) -> bool {
    const VK_SHIFT: u32 = 0x10;
    const VK_MENU: u32 = 0x12;
    const VK_LWIN: u32 = 0x5B;
    const VK_RWIN: u32 = 0x5C;
    const VK_RMENU: u32 = 0xA5;
    matches!(vk, VK_SHIFT..=VK_MENU | VK_LWIN | VK_RWIN | VK_LSHIFT..=VK_RMENU | VK_CAPITAL)
}

/// 虚拟键码 → FR-SEN-01 的按键类别。单按的 Shift / Ctrl / Alt / Win / CapsLock 不算按键，
/// 返回 `None`。
pub(crate) fn key_kind(vk: u32) -> Option<KeyKind> {
    const PUNCT: [u32; 11] = [
        VK_SEMICOLON,
        VK_EQUAL,
        VK_COMMA,
        VK_MINUS,
        VK_PERIOD,
        VK_SLASH,
        VK_BACKTICK,
        VK_LBRACKET,
        VK_BACKSLASH,
        VK_RBRACKET,
        VK_QUOTE,
    ];
    let kind = match vk {
        _ if is_modifier_vk(vk) => return None,
        VK_A..=VK_Z => KeyKind::Letter,
        VK_0..=VK_9 => KeyKind::Digit,
        VK_SPACE => KeyKind::Space,
        VK_RETURN => KeyKind::Enter,
        VK_BACK => KeyKind::Backspace,
        VK_DELETE => KeyKind::Delete,
        VK_ESCAPE => KeyKind::Esc,
        VK_PRIOR | VK_NEXT | VK_END | VK_HOME | VK_LEFT | VK_UP | VK_RIGHT | VK_DOWN => {
            KeyKind::Nav
        }
        _ if PUNCT.contains(&vk) => KeyKind::Punct,
        _ => match numpad_char(vk) {
            Some(c) if c.is_ascii_digit() => KeyKind::Digit,
            Some(_) => KeyKind::Punct,
            None => KeyKind::Other,
        },
    };
    Some(kind)
}

/// `input_diag` 的判定 → MFP `focus.scope`（FR-SEN-03 第 2 条）。
pub(crate) fn scope_of(reason: InputDiagReason) -> Scope {
    match reason {
        InputDiagReason::InputScopePassword
        | InputDiagReason::NumericPassword
        | InputDiagReason::ContextPassword => Scope::Password,
        InputDiagReason::CompartmentDisabled | InputDiagReason::ContextDisabled => Scope::Disabled,
        InputDiagReason::None => Scope::Normal,
    }
}

/// Moodflow `CommitSource` → MFP `commit.src`。
pub(crate) fn source_name(src: CommitSource) -> &'static str {
    match src {
        CommitSource::Candidate => "candidate",
        CommitSource::RawInput => "raw_input",
        CommitSource::Punctuation => "punctuation",
        CommitSource::TempEnglish => "temp_english",
        CommitSource::TempPinyin => "temp_pinyin",
        CommitSource::QuickInput => "quick_input",
        CommitSource::FullWidth => "full_width",
        CommitSource::ModeSwitch => "mode_switch",
        CommitSource::TsfDirect => "tsf_direct",
        CommitSource::SpecialMode => "special_mode",
        CommitSource::Url => "url",
        CommitSource::Mix => "mix",
        CommitSource::Email => "email",
    }
}

/// 按键进入协调器之前（FR-SEN-01）：先于本键可能产生的上屏发出。`rewrite_hotkey` 为真时
/// 本键是改写快捷键，不报：它带修饰键，报了会清掉马上要取的“最近上屏”。
pub(crate) fn before_key(data: &KeyEventData, before: CompSnap, rewrite_hotkey: bool) {
    let Some(tap) = tap() else {
        return;
    };
    let _t = perf::hook_timer();
    KEY_COMMITTED.with(|c| c.set(false));
    if data.event_type != EVENT_KEY_DOWN || rewrite_hotkey {
        return;
    }
    let Some(kind) = key_kind(data.key_code) else {
        return;
    };
    tap.hook_key(KeyHook {
        kind,
        vk: data.key_code as u8,
        in_comp: before.len > 0,
        modified: data.modifiers & MOD_SHORTCUT != 0,
    });
}

/// 按键处理完之后：比较前后快照，发组字与翻页事件（FR-SEN-04）。选词在上屏钩子里发。
pub(crate) fn after_key(data: &KeyEventData, before: CompSnap, after: CompSnap) {
    let Some(tap) = tap() else {
        return;
    };
    let _t = perf::hook_timer();
    if data.event_type != EVENT_KEY_DOWN {
        return;
    }
    if after.len > 0 {
        if after.len != before.len {
            tap.hook_comp(
                CompOp::Update,
                after.len.min(u16::MAX as usize) as u16,
                after.invalid,
            );
        } else if before.len > 0 && after.page != before.page {
            tap.hook_cand(CandOp::Page, None);
        }
    } else if before.len > 0 {
        match key_kind(data.key_code) {
            Some(KeyKind::Esc) => tap.hook_comp(CompOp::Cancel, 0, false),
            Some(KeyKind::Backspace) => tap.hook_comp(CompOp::Clear, 0, false),
            // 其余是上屏结束了组字，上屏钩子已经发过
            _ => {}
        }
    }
}

thread_local! {
    /// 本次按键里是否已经报过上屏。按键在 bridge 的单个线程上串行处理，所以用线程局部量。
    static KEY_COMMITTED: Cell<bool> = const { Cell::new(false) };
    /// 正处在 `record_input_stats` 的兜底记账里。
    static IN_FALLBACK: Cell<bool> = const { Cell::new(false) };
}

/// 包住 `record_input_stats` 的兜底记账。Moodflow 靠 `stat_recorded` 判断具体路径记过没有，
/// 但统计关着（或 headless 没有收集器）时它不置位，兜底会把同一次上屏再记一遍；
/// Moodflow 这边自己记，避免 Hub 收到重复的 `commit`。
pub(crate) fn fallback_commit(f: impl FnOnce()) {
    IN_FALLBACK.with(|c| c.set(true));
    f();
    IN_FALLBACK.with(|c| c.set(false));
}

/// 上屏（FR-SEN-02），由 `record_commit_ks` 调用。从候选里选词时先发 `cand{select}`。
pub(crate) fn on_commit(text: &str, keystrokes: u32, cand_pos: i32, source: CommitSource) {
    let Some(tap) = tap() else {
        return;
    };
    let _t = perf::hook_timer();
    if IN_FALLBACK.with(Cell::get) && KEY_COMMITTED.with(Cell::get) {
        return;
    }
    KEY_COMMITTED.with(|c| c.set(true));
    if source == CommitSource::Candidate && cand_pos >= 0 {
        tap.hook_cand(CandOp::Select, Some(cand_pos as u32));
    }
    tap.hook_commit(CommitHook {
        chars: text.chars().count() as u32,
        keystrokes,
        cand_pos,
        src: source_name(source),
        text,
    });
}

/// 当前输入环境的隐私状态。**插件、剪贴板历史、打字测速等一切读输入内容的扩展**都以它为准：
/// 任一项为真就不给内容（ADR 0054 第 5 条）。与 MFP 的隐私闸门同源，但不依赖 Hub 的同意状态——
/// 闸门管的是「能不能发给 Hub」，这里管的是「本地扩展能不能看见」。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct InputPrivacy {
    /// 密码框或禁用输入的框。
    pub private_field: bool,
    pub secure_desktop: bool,
    /// 无痕模式（Hub 下发或菜单/快捷键开启的暂停）。
    pub incognito: bool,
    /// 应用名单命中。
    pub app_blocked: bool,
}

impl InputPrivacy {
    pub(crate) fn input_allowed(&self) -> bool {
        !(self.private_field || self.secure_desktop || self.incognito || self.app_blocked)
    }
}

/// `(private_field, secure_desktop)`，每次焦点变化时更新；无 Tap 时也更新。
static FOCUS_PRIVACY: Mutex<(bool, bool)> = Mutex::new((false, false));

#[cfg(test)]
thread_local! {
    /// 测试用：本线程固定的隐私状态。`FOCUS_PRIVACY` 是进程级的，并行的其它用例会报密码框焦点，
    /// 不隔离的话依赖它的用例会随调度时序时绿时红。
    pub(crate) static TEST_PRIVACY: Cell<Option<InputPrivacy>> = const { Cell::new(None) };
}

/// 当前输入环境的隐私状态。
pub(crate) fn input_privacy() -> InputPrivacy {
    #[cfg(test)]
    if let Some(p) = TEST_PRIVACY.with(Cell::get) {
        return p;
    }
    let (private_field, secure_desktop) = *FOCUS_PRIVACY.lock().unwrap_or_else(|e| e.into_inner());
    let (incognito, app_blocked) = tap().map_or((false, false), |t| (t.paused(), t.app_blocked()));
    InputPrivacy {
        private_field,
        secure_desktop,
        incognito,
        app_blocked,
    }
}

/// 焦点或输入框类型变化（FR-SEN-03）。`app` 是小写进程名，空串表示未知。
pub(crate) fn on_focus(app: &str, scope: Scope) {
    *FOCUS_APP.lock().unwrap_or_else(|e| e.into_inner()) = app.to_string();
    let secure = on_secure_desktop(app);
    *FOCUS_PRIVACY.lock().unwrap_or_else(|e| e.into_inner()) = (scope != Scope::Normal, secure);
    let Some(tap) = tap() else {
        return;
    };
    // 先定安全桌面，再报焦点：锁屏、UAC 框里连 focus 也不发（C-PLT-05）
    tap.set_secure_desktop(secure);
    tap.hook_focus(FocusHook {
        app: (!app.is_empty()).then_some(app),
        scope,
    });
}

/// 安全桌面上的宿主进程（C-PLT-05）：登录与锁屏界面、UAC 提权框。
const SECURE_DESKTOP_HOSTS: [&str; 2] = ["logonui.exe", "consent.exe"];

/// 输入焦点是否在安全桌面上。进程名命中 [`SECURE_DESKTOP_HOSTS`]，或当前输入桌面不是
/// `Default`（锁屏、登录、UAC 都切到 `Winlogon` 桌面）。
fn on_secure_desktop(app: &str) -> bool {
    SECURE_DESKTOP_HOSTS
        .iter()
        .any(|h| app.eq_ignore_ascii_case(h))
        || input_desktop_is_secure()
}

#[cfg(windows)]
fn input_desktop_is_secure() -> bool {
    use windows::Win32::Foundation::{E_ACCESSDENIED, HANDLE};
    use windows::Win32::System::StationsAndDesktops::{
        CloseDesktop, DESKTOP_CONTROL_FLAGS, DESKTOP_READOBJECTS, GetUserObjectInformationW,
        OpenInputDesktop, UOI_NAME,
    };
    // SAFETY: 句柄只在本函数里用，用完关闭；名字缓冲按字节长度传入。
    unsafe {
        let desk = match OpenInputDesktop(DESKTOP_CONTROL_FLAGS(0), false, DESKTOP_READOBJECTS) {
            Ok(h) => h,
            // 普通用户进程打不开 Winlogon 桌面，拒绝访问就是在安全桌面上；其他错误不下结论
            Err(e) => return e.code() == E_ACCESSDENIED,
        };
        let mut buf = [0u16; 64];
        let ok = GetUserObjectInformationW(
            HANDLE(desk.0),
            UOI_NAME,
            Some(buf.as_mut_ptr().cast()),
            std::mem::size_of_val(&buf) as u32,
            None,
        )
        .is_ok();
        let _ = CloseDesktop(desk);
        if !ok {
            return false;
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        !String::from_utf16_lossy(&buf[..len]).eq_ignore_ascii_case("Default")
    }
}

#[cfg(not(windows))]
fn input_desktop_is_secure() -> bool {
    false
}

/// 宿主里的选区变了（用户点了别处、选中了文字）：“最近上屏”与光标前的文字对不上了。
pub(crate) fn on_selection_changed() {
    if let Some(t) = tap() {
        t.hook_selection_changed();
    }
}

/// 宿主终止了组字（FR-SEN-04）。
pub(crate) fn on_comp_terminated() {
    if let Some(tap) = tap() {
        tap.hook_comp(CompOp::Terminated, 0, false);
    }
}

/// 输入法激活状态与中英状态。
pub(crate) fn on_ime(active: bool, chinese: bool) {
    if let Some(tap) = tap() {
        tap.hook_ime(active, chinese);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moodflow_keys::keymap::VK_TAB;

    #[test]
    fn key_kinds_follow_fr_sen_01() {
        assert_eq!(key_kind(VK_A), Some(KeyKind::Letter));
        assert_eq!(key_kind(VK_Z), Some(KeyKind::Letter));
        assert_eq!(key_kind(VK_0 + 5), Some(KeyKind::Digit));
        assert_eq!(key_kind(0x60), Some(KeyKind::Digit), "小键盘数字");
        assert_eq!(key_kind(0x6A), Some(KeyKind::Punct), "小键盘 *");
        assert_eq!(key_kind(VK_COMMA), Some(KeyKind::Punct));
        assert_eq!(key_kind(VK_SPACE), Some(KeyKind::Space));
        assert_eq!(key_kind(VK_RETURN), Some(KeyKind::Enter));
        assert_eq!(key_kind(VK_BACK), Some(KeyKind::Backspace));
        assert_eq!(key_kind(VK_DELETE), Some(KeyKind::Delete));
        assert_eq!(key_kind(VK_ESCAPE), Some(KeyKind::Esc));
        assert_eq!(key_kind(VK_PRIOR), Some(KeyKind::Nav));
        assert_eq!(key_kind(VK_LEFT), Some(KeyKind::Nav));
        assert_eq!(key_kind(VK_TAB), Some(KeyKind::Other));
        assert_eq!(key_kind(VK_LSHIFT), None);
        assert_eq!(key_kind(VK_CAPITAL), None);
        for vk in [0x10, 0x11, 0x12, 0xA4, 0xA5, 0x5B, 0x5C] {
            assert_eq!(key_kind(vk), None, "单按修饰键 {vk:#x}");
        }
    }

    #[test]
    fn logon_and_uac_hosts_are_on_the_secure_desktop() {
        assert!(on_secure_desktop("LogonUI.exe"));
        assert!(on_secure_desktop("consent.exe"));
        assert!(!on_secure_desktop("notepad.exe"));
        assert!(!on_secure_desktop(""));
    }

    #[test]
    fn password_reasons_map_to_password_scope() {
        assert_eq!(
            scope_of(InputDiagReason::InputScopePassword),
            Scope::Password
        );
        assert_eq!(scope_of(InputDiagReason::NumericPassword), Scope::Password);
        assert_eq!(scope_of(InputDiagReason::ContextPassword), Scope::Password);
        assert_eq!(
            scope_of(InputDiagReason::CompartmentDisabled),
            Scope::Disabled
        );
        assert_eq!(scope_of(InputDiagReason::ContextDisabled), Scope::Disabled);
        assert_eq!(scope_of(InputDiagReason::None), Scope::Normal);
    }

    #[test]
    fn every_commit_source_has_a_name() {
        assert_eq!(CommitSource::COUNT, 13, "新增来源时同步 source_name");
        assert_eq!(source_name(CommitSource::ModeSwitch), "mode_switch");
    }

    /// moodflow-config 不依赖 tap，出厂黑名单两边各写一份（FR-SEN-05），在这里对照。
    #[test]
    fn default_blocklist_matches_tap() {
        assert_eq!(
            moodflow_config::MOODFLOW_DEFAULT_BLOCKLIST,
            moodflow_tap::DEFAULT_BLOCKLIST
        );
        assert_eq!(
            moodflow_config::Config::default().moodflow.app_blocklist,
            moodflow_tap::DEFAULT_BLOCKLIST
        );
    }
}
