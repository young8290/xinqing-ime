//! 心晴：把协调器的按键、上屏、焦点、组字、候选事件交给 `wind-xinqing-tap`
//! （产品书 17 第 1.2 节，任务 A-04；FR-SEN-01～04）。
//!
//! Tap 是进程级单例，服务启动时由 [`start`] 安装。没安装时（单元测试、headless、移动端）
//! 所有钩子都是一次 `OnceLock::get` 后直接返回，不改变清风原有行为。
//! 钩子只调 Tap 的非阻塞方法（闸门判断 + `try_send`），可以在 `state` 锁内调用。
//!
//! 下行消息（mood、tip、badge 等）的界面处理属于 A-06/A-07，这里先只记 debug 日志。

use std::cell::Cell;
use std::net::SocketAddr;
use std::sync::{Arc, OnceLock, Weak};

use wind_bridge::handler::KeyEventData;
use wind_config::XinqingConfig;
use wind_ipc::protocol::{EVENT_KEY_DOWN, MOD_SHORTCUT};
use wind_keys::keymap::{
    VK_0, VK_9, VK_A, VK_BACK, VK_BACKSLASH, VK_BACKTICK, VK_CAPITAL, VK_COMMA, VK_DELETE, VK_DOWN,
    VK_END, VK_EQUAL, VK_ESCAPE, VK_HOME, VK_LBRACKET, VK_LCONTROL, VK_LEFT, VK_LSHIFT, VK_MINUS,
    VK_NEXT, VK_PERIOD, VK_PRIOR, VK_QUOTE, VK_RBRACKET, VK_RCONTROL, VK_RETURN, VK_RIGHT,
    VK_RSHIFT, VK_SEMICOLON, VK_SLASH, VK_SPACE, VK_UP, VK_Z,
};
use wind_store::stats::CommitSource;
use wind_xinqing_tap::{
    CandOp, CommitHook, CompOp, Down, Endpoint, FocusHook, KeyHook, KeyKind, PIPE_NAME,
    PIPE_NAME_DEV, PauseBy, Scope, Tap, TapConfig,
};

use crate::coordinator::Coordinator;
use wind_ui_types::{ToastKind, ToastPosition};

use crate::input_diag::InputDiagReason;
use crate::key_convert::numpad_char;

/// 设了这个环境变量（如 `127.0.0.1:18765`）时改在本机 TCP 上监听，供 Hub 的同名变量联调；
/// 非 Windows 平台只能这样启用。
pub const XQP_TCP_ENV: &str = "XQ_XQP_TCP";

static TAP: OnceLock<Arc<Tap>> = OnceLock::new();
/// 下行回调要回到协调器（记住 Hub 发来的无痕状态）；弱引用，不延长协调器寿命。
static COORD: OnceLock<Weak<Coordinator>> = OnceLock::new();

/// 启动 XQP 服务端（10 第 2.1 节）。`pipe_suffix` 取 `wind_config::variant::pipe_suffix()`，
/// dev 构建用 `xinqing_tap_dev`。失败只记日志：心晴组件出问题不能影响打字。
///
/// 总开关与应用名单取自 `[xinqing]` 配置；`xinqing.remember_pause` 打开时恢复上次的无痕状态。
pub fn start(c: &Arc<Coordinator>, ime_ver: &str, pipe_suffix: &str) {
    if TAP.get().is_some() {
        return;
    }
    let _ = COORD.set(Arc::downgrade(c));
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
    match Tap::start(cfg, Box::new(on_downlink)) {
        Ok(tap) => {
            if xq.remember_pause && c.xinqing_stored_pause() {
                // 还没有连接，不会发 pause_changed；Hub 连上后从焦点事件看出闸门关着
                tap.set_paused(true, PauseBy::Menu);
            }
            let _ = TAP.set(tap);
            tracing::info!("心晴 XQP 服务端已启动");
        }
        Err(e) => tracing::warn!("心晴 XQP 服务端启动失败：{e}"),
    }
}

/// 配置热重载后调用：总开关与应用名单即时生效。
pub(crate) fn apply_config(cfg: &XinqingConfig) {
    if let Some(t) = tap() {
        t.set_app_lists(&cfg.app_blocklist, &cfg.app_allowlist);
        t.set_enabled(cfg.enabled);
    }
}

/// 无痕模式是否开着；没装 Tap 时 `None`（菜单据此不显示心晴项）。
pub(crate) fn paused() -> Option<bool> {
    tap().map(|t| t.paused())
}

/// 切换无痕模式，返回切换后的状态；没装 Tap 时 `None`。
pub(crate) fn toggle_pause(by: PauseBy) -> Option<bool> {
    let t = tap()?;
    let on = !t.paused();
    t.set_paused(on, by);
    Some(on)
}

/// 测试用：安装一个已启动的 Tap。进程内只能装一次，已装过返回假。
pub fn install(tap: Arc<Tap>) -> bool {
    TAP.set(tap).is_ok()
}

/// 核心退出前调用：发 `bye{shutdown}`。
pub fn shutdown() {
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

    /// 菜单与 Ctrl+Alt+P 的共同出口（FR-SEN-05/06）：切换、记住、气泡提示。
    /// 心晴没启动时什么都不做，返回 `None`。
    pub(crate) fn xinqing_toggle_pause(&self, by: PauseBy) -> Option<bool> {
        let on = toggle_pause(by)?;
        self.xinqing_store_pause(on);
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

fn on_downlink(msg: Down) {
    if let Down::Pause { on } = msg
        && let Some(c) = COORD.get().and_then(Weak::upgrade)
    {
        // 小组件右键等 Hub 侧入口切的无痕，与菜单、快捷键一样按需记住
        c.xinqing_store_pause(on);
    }
    // A-06/A-07 接入工具栏、气泡与菜单；内容可能是提示文字，只记类型
    let kind = match &msg {
        Down::Pause { .. } => "pause",
        Down::Mood { .. } => "mood",
        Down::Badge { .. } => "badge",
        Down::Tip { .. } => "tip",
        Down::Pending { .. } => "pending",
        Down::RewriteResult { .. } => "rewrite_result",
        Down::RewriteFail { .. } => "rewrite_fail",
        _ => "other",
    };
    tracing::debug!("XQP 下行：{kind}");
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
    let s = c.state.lock().unwrap_or_else(|e| e.into_inner());
    Some(CompSnap {
        len: s.input_buffer.chars().count(),
        page: s.current_page,
    })
}

/// 虚拟键码 → FR-SEN-01 的按键类别。单按的 Shift / Ctrl / CapsLock 不算按键，返回 `None`。
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
        VK_LSHIFT | VK_RSHIFT | VK_LCONTROL | VK_RCONTROL | VK_CAPITAL => return None,
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

/// 按键进入协调器之前（FR-SEN-01）：先于本键可能产生的上屏发出。
pub(crate) fn before_key(data: &KeyEventData, before: CompSnap) {
    let Some(tap) = tap() else {
        return;
    };
    KEY_COMMITTED.with(|c| c.set(false));
    if data.event_type != EVENT_KEY_DOWN {
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
    tap.hook_focus(FocusHook {
        app: (!app.is_empty()).then_some(app),
        scope,
    });
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
}
