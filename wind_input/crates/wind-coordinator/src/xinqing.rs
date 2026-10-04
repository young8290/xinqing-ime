//! 心晴：把协调器的按键、上屏、焦点、组字、候选事件交给 `wind-xinqing-tap`
//! （产品书 17 第 1.2 节，任务 A-04；FR-SEN-01～04）。
//!
//! Tap 是进程级单例，服务启动时由 [`start`] 安装。没安装时（单元测试、headless、移动端）
//! 所有钩子都是一次 `OnceLock::get` 后直接返回，不改变清风原有行为。
//! 钩子只调 Tap 的非阻塞方法（闸门判断 + `try_send`），可以在 `state` 锁内调用。
//!
//! Hub 守护（A-06，17 第 1.5 节）也在这里装：`start` 时起 `xq-hub-guard`，配置热重载时更新
//! 是否需要 Hub。下行的 `mood`、`badge`、`pending` 记进 [`hub_view`]，菜单与工具栏从那里读。
//! 输入法里的心晴入口（A-07）：主菜单“心晴”分组、`tip` 的光标旁气泡、工具栏天气按钮。
//! 温柔改写模式（A-08）在子模块 [`rewrite`]。

pub(crate) mod perf;
mod rewrite;

use std::cell::Cell;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

use wind_bridge::handler::KeyEventData;
use wind_config::XinqingConfig;
use wind_ipc::protocol::{EVENT_KEY_DOWN, MOD_SHORTCUT};
use wind_keys::keymap::{
    VK_0, VK_9, VK_A, VK_BACK, VK_BACKSLASH, VK_BACKTICK, VK_CAPITAL, VK_COMMA, VK_DELETE, VK_DOWN,
    VK_END, VK_EQUAL, VK_ESCAPE, VK_HOME, VK_LBRACKET, VK_LEFT, VK_LSHIFT, VK_MINUS, VK_NEXT,
    VK_PERIOD, VK_PRIOR, VK_QUOTE, VK_RBRACKET, VK_RETURN, VK_RIGHT, VK_SEMICOLON, VK_SLASH,
    VK_SPACE, VK_UP, VK_Z,
};
use wind_store::stats::CommitSource;
use wind_xinqing_tap::guard::{ExeLauncher, GuardState, HubGuard};
use wind_xinqing_tap::{
    CandOp, CommitHook, CompOp, Down, Endpoint, FocusHook, KeyHook, KeyKind, MoodState, OpenTarget,
    PIPE_NAME, PIPE_NAME_DEV, PauseBy, Scope, Tap, TapConfig,
};

use crate::coordinator::Coordinator;
use wind_ui_types::{
    MenuCmd, MenuItemSpec, MenuKind, ToastKind, ToastPosition, XinqingCell, XinqingWeather,
};

pub(crate) use rewrite::{RewriteMode, debug_phase as debug_rewrite_phase, replay_if_exited};

use crate::input_diag::InputDiagReason;
use crate::key_convert::numpad_char;

/// 设了这个环境变量（如 `127.0.0.1:18765`）时改在本机 TCP 上监听，供 Hub 的同名变量联调；
/// 非 Windows 平台只能这样启用。
pub const XQP_TCP_ENV: &str = "XQ_XQP_TCP";

/// 设了这个环境变量时从这里拉起 Hub（开发联调）；否则取核心可执行文件旁边的 [`HUB_EXE`]
/// （03 第 5.1 节安装目录）。
pub const HUB_EXE_ENV: &str = "XQ_HUB_EXE";
#[cfg(windows)]
pub const HUB_EXE: &str = "xinqing_hub.exe";
#[cfg(not(windows))]
pub const HUB_EXE: &str = "xinqing_hub";

static TAP: OnceLock<Arc<Tap>> = OnceLock::new();
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

/// 启动 XQP 服务端（10 第 2.1 节）。`pipe_suffix` 取 `wind_config::variant::pipe_suffix()`，
/// dev 构建用 `xinqing_tap_dev`。失败只记日志：心晴组件出问题不能影响打字。
///
/// 总开关与应用名单取自 `[xinqing]` 配置；`xinqing.remember_pause` 打开时恢复上次的无痕状态。
pub fn start(c: &Arc<Coordinator>, ime_ver: &str, pipe_suffix: &str) {
    if TAP.get().is_some() {
        return;
    }
    attach(c);
    let endpoint = match std::env::var(XQP_TCP_ENV) {
        Ok(addr) => match addr.parse::<SocketAddr>() {
            Ok(a) => Endpoint::Tcp(a),
            Err(e) => {
                tracing::warn!("{XQP_TCP_ENV}={addr} 无效：{e}");
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
    let xq = c.rt().config.xinqing.clone();
    let mut cfg = TapConfig::new(ime_ver, endpoint);
    cfg.enabled = xq.enabled;
    cfg.app_blocklist = xq.app_blocklist.clone();
    cfg.app_allowlist = xq.app_allowlist.clone();
    match Tap::start(cfg, Box::new(downlink)) {
        Ok(tap) => {
            if xq.remember_pause && c.xinqing_stored_pause() {
                // 还没有连接；Hub 握手后 Tap 会补发 pause_changed
                tap.set_paused(true, PauseBy::Menu);
            }
            let probe = Arc::clone(&tap);
            let _ = TAP.set(tap);
            tracing::info!("心晴 XQP 服务端已启动");
            start_guard(probe, xq.enabled && xq.hub_autostart);
        }
        Err(e) => tracing::warn!("心晴 XQP 服务端启动失败：{e}"),
    }
}

/// 起 Hub 守护（FR-OPS-03）。Hub 程序找不到也照常起：拉不起来就按失败计数，次数用完停下，
/// 菜单显示“心晴组件未运行，点击重试”。
fn start_guard(tap: Arc<Tap>, wanted: bool) {
    let path = hub_exe_path();
    tracing::debug!("心晴 Hub 路径：{}", path.display());
    match HubGuard::start(
        Box::new(move || tap.is_linked()),
        Box::new(ExeLauncher { path }),
        wanted,
    ) {
        Ok(g) => {
            watch_link(&g);
            let _ = GUARD.set(g);
        }
        Err(e) => tracing::warn!("心晴 Hub 守护线程启动失败：{e}"),
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
pub(crate) fn apply_config(cfg: &XinqingConfig) {
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
pub fn weather_of(mood: Option<MoodState>) -> XinqingWeather {
    match mood {
        Some(MoodState::Hesitant) => XinqingWeather::Cloudy,
        Some(MoodState::Low) => XinqingWeather::Rain,
        Some(MoodState::Agitated) => XinqingWeather::Storm,
        Some(MoodState::Tired) => XinqingWeather::Night,
        Some(MoodState::Fluent | MoodState::Unknown) | None => XinqingWeather::Clear,
    }
}

/// 菜单“心晴组件未运行，点击重试”。
pub(crate) fn retry_hub() {
    if let Some(g) = GUARD.get() {
        g.retry();
    }
}

/// 菜单“心晴”分组里请 Hub 打开的窗口，下标即 `MenuCmd::XinqingOpen` 的参数。
pub const OPEN_TARGETS: [OpenTarget; 4] = [
    OpenTarget::Chat,
    OpenTarget::Dashboard,
    OpenTarget::Schedule,
    OpenTarget::Settings,
];

/// 菜单“和晴晴聊聊”等：请 Hub 打开窗口（XQP `open`）。Hub 没连着时菜单项是灰的。
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
    /// state.toml 里记着的无痕状态（`xinqing.remember_pause` 打开时启动恢复用）。
    fn xinqing_stored_pause(&self) -> bool {
        wind_config::Config::state_dir()
            .map(|d| wind_config::RuntimeState::load(&d).xinqing_paused)
            .unwrap_or(false)
    }

    /// 记下当前无痕状态。不管 `remember_pause` 开没开都记，开关只决定启动时用不用它：
    /// 这样之后再打开「记住」，恢复的也是最后一次的真实状态，而不是很久以前的旧值。
    pub(crate) fn xinqing_store_pause(&self, on: bool) {
        self.state_writer
            .schedule("xinqing_pause", move |rs| rs.xinqing_paused = on);
    }

    /// 菜单“开启 / 关闭心晴功能”（FR-IME-02）：写用户配置并即时生效。关掉时 Tap 发
    /// `bye{disabled}`、停止监听，守护不再拉起 Hub；打开时立即拉起。
    pub(crate) fn xinqing_toggle_enabled(&self) {
        let next = !self.rt().config.xinqing.enabled;
        if let Err(e) =
            wind_config::Config::set_user_value(&["xinqing", "enabled"], toml::Value::Boolean(next))
        {
            tracing::warn!("写入 xinqing.enabled 失败：{e}");
        }
        self.refresh_config_in_memory(|c| c.xinqing.enabled = next);
        // 天气按钮跟着出现或消失
        self.notify_toolbar();
        self.show_toast(
            if next {
                "已开启心晴功能"
            } else {
                "已关闭心晴功能"
            },
            ToastPosition::BottomCenter,
            ToastKind::Info,
        );
    }

    /// Hub 的 `tip`（FR-ENT-03）：只在没有组字时显示，10 秒内最多 1 条，时长限定 1.5–2.5 秒。
    fn xinqing_tip(&self, text: &str, ms: u32) {
        if self.has_active_session() {
            tracing::debug!("心晴提示：正在组字，不显示");
            return;
        }
        {
            let mut last = LAST_TIP.lock().unwrap_or_else(|e| e.into_inner());
            let now = Instant::now();
            if last.is_some_and(|t| now.duration_since(t) < TIP_GAP) {
                tracing::debug!("心晴提示：10 秒内已显示过，跳过");
                return;
            }
            *last = Some(now);
        }
        // 10 第 2.5 节：≤ 16 字。Hub 发送前已校验，这里只防万一
        let text: String = text.chars().take(16).collect();
        self.show_xinqing_tip(&text, u64::from(ms.clamp(1500, 2500)));
    }

    /// 工具栏天气按钮的状态（FR-ENT-02）；心晴没启动或总开关关着时 `None`，这一格不画。
    /// Hub 没连上或正在无痕时淡显；有未读暖心话时加小圆点。不取 `state` 锁。
    pub(crate) fn xinqing_toolbar_cell(&self) -> Option<XinqingCell> {
        let t = tap()?;
        if !self.rt().config.xinqing.enabled {
            return None;
        }
        let linked = t.is_linked();
        let v = hub_view();
        Some(XinqingCell {
            weather: weather_of(v.mood),
            dot: v.badge,
            dim: !linked || t.paused(),
        })
    }

    /// 左键天气按钮：打开和晴晴的对话。Hub 没连上时改为重新拉起它，免得点了没反应。
    pub(crate) fn xinqing_weather_click(&self) {
        let Some(t) = tap() else {
            return;
        };
        if t.is_linked() {
            t.send_open(OpenTarget::Chat);
        } else {
            retry_hub();
            self.show_toast(
                "心晴组件未运行，正在重新启动",
                ToastPosition::BottomCenter,
                ToastKind::Info,
            );
        }
    }

    /// 右键天气按钮的小菜单：暂停 / 恢复感知、情绪看板（FR-ENT-02）。没装心晴时 `None`，
    /// 回落主菜单。
    pub(crate) fn xinqing_weather_menu(&self) -> Option<Vec<MenuItemSpec>> {
        let t = tap()?;
        let pause = if t.paused() {
            "恢复感知"
        } else {
            "暂停感知"
        };
        Some(vec![
            MenuItemSpec::leaf(
                pause.to_string(),
                MenuKind::Command(MenuCmd::XinqingTogglePause),
                true,
                false,
            ),
            MenuItemSpec::leaf(
                "情绪看板".to_string(),
                MenuKind::Command(MenuCmd::XinqingOpen(1)),
                t.is_linked(),
                false,
            ),
        ])
    }

    /// 主菜单“心晴”分组（FR-ENT-01）。心晴没启动时为空；总开关关着时只剩“开启心晴功能”；
    /// Hub 没连上时，除“暂停感知”和总开关外都置灰，并多一项“心晴组件未运行，点击重试”。
    pub(crate) fn xinqing_menu_group(&self) -> Vec<MenuItemSpec> {
        let Some(t) = tap() else {
            return Vec::new();
        };
        let cmd = |c: MenuCmd| MenuKind::Command(c);
        let leaf = |label: &str, c: MenuCmd, enabled: bool| {
            MenuItemSpec::leaf(label.to_string(), cmd(c), enabled, false)
        };
        if !self.rt().config.xinqing.enabled {
            return vec![
                leaf("开启心晴功能", MenuCmd::XinqingToggleEnabled, true),
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
            leaf("和晴晴聊聊", MenuCmd::XinqingOpen(0), linked),
            leaf("情绪看板", MenuCmd::XinqingOpen(1), linked),
            leaf(&schedule, MenuCmd::XinqingOpen(2), linked),
            leaf(
                if t.paused() {
                    "恢复感知"
                } else {
                    "暂停感知"
                },
                MenuCmd::XinqingTogglePause,
                true,
            ),
            leaf("心晴设置", MenuCmd::XinqingOpen(3), linked),
            leaf("关闭心晴功能", MenuCmd::XinqingToggleEnabled, true),
        ];
        if !linked {
            v.push(leaf(
                "心晴组件未运行，点击重试",
                MenuCmd::XinqingRetryHub,
                true,
            ));
        }
        v.push(MenuItemSpec::separator());
        v
    }

    /// 菜单与 Ctrl+Alt+P 的共同出口（FR-SEN-05/06）：切换、记住、气泡提示。
    /// 心晴没启动时什么都不做，返回 `None`。
    pub(crate) fn xinqing_toggle_pause(&self, by: PauseBy) -> Option<bool> {
        let on = toggle_pause(by)?;
        self.xinqing_store_pause(on);
        // 天气按钮跟着淡显或恢复
        self.notify_toolbar();
        self.show_toast(
            if on {
                "心晴已暂停感知"
            } else {
                "心晴已恢复感知"
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
                c.xinqing_store_pause(on);
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
                c.xinqing_tip(&text, ms);
            }
        }
        Down::RewriteResult {
            req_id,
            style,
            cands,
        } => {
            if let Some(c) = COORD.get().and_then(Weak::upgrade) {
                c.xinqing_rewrite_result(req_id, style, cands);
            }
        }
        Down::RewriteFail { req_id, reason } => {
            if let Some(c) = COORD.get().and_then(Weak::upgrade) {
                c.xinqing_rewrite_fail(req_id, reason);
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
}

/// 取当前组字快照；没装 Tap 时返回 `None`，调用方据此跳过钩子、不取锁。
pub(crate) fn snap(c: &crate::coordinator::Coordinator) -> Option<CompSnap> {
    tap()?;
    let _t = perf::hook_timer();
    let s = c.state.lock().unwrap_or_else(|e| e.into_inner());
    Some(CompSnap {
        len: s.input_buffer.chars().count(),
        page: s.current_page,
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

/// `input_diag` 的判定 → XQP `focus.scope`（FR-SEN-03 第 2 条）。
pub(crate) fn scope_of(reason: InputDiagReason) -> Scope {
    match reason {
        InputDiagReason::InputScopePassword
        | InputDiagReason::NumericPassword
        | InputDiagReason::ContextPassword => Scope::Password,
        InputDiagReason::CompartmentDisabled | InputDiagReason::ContextDisabled => Scope::Disabled,
        InputDiagReason::None => Scope::Normal,
    }
}

/// 清风 `CommitSource` → XQP `commit.src`。
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
            tap.hook_comp(CompOp::Update, after.len.min(u16::MAX as usize) as u16);
        } else if before.len > 0 && after.page != before.page {
            tap.hook_cand(CandOp::Page, None);
        }
    } else if before.len > 0 {
        match key_kind(data.key_code) {
            Some(KeyKind::Esc) => tap.hook_comp(CompOp::Cancel, 0),
            Some(KeyKind::Backspace) => tap.hook_comp(CompOp::Clear, 0),
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

/// 包住 `record_input_stats` 的兜底记账。清风靠 `stat_recorded` 判断具体路径记过没有，
/// 但统计关着（或 headless 没有收集器）时它不置位，兜底会把同一次上屏再记一遍；
/// 心晴这边自己记，避免 Hub 收到重复的 `commit`。
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

/// 焦点或输入框类型变化（FR-SEN-03）。`app` 是小写进程名，空串表示未知。
pub(crate) fn on_focus(app: &str, scope: Scope) {
    let Some(tap) = tap() else {
        return;
    };
    // 先定安全桌面，再报焦点：锁屏、UAC 框里连 focus 也不发（C-PLT-05）
    tap.set_secure_desktop(on_secure_desktop(app));
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
        tap.hook_comp(CompOp::Terminated, 0);
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
    use wind_keys::keymap::VK_TAB;

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

    /// wind-config 不依赖 tap，出厂黑名单两边各写一份（FR-SEN-05），在这里对照。
    #[test]
    fn default_blocklist_matches_tap() {
        assert_eq!(
            wind_config::XINQING_DEFAULT_BLOCKLIST,
            wind_xinqing_tap::DEFAULT_BLOCKLIST
        );
        assert_eq!(
            wind_config::Config::default().xinqing.app_blocklist,
            wind_xinqing_tap::DEFAULT_BLOCKLIST
        );
    }
}
