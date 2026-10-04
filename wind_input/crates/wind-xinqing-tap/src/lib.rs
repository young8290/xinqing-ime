//! 心晴：核心服务侧的 XQP 服务端（产品书 17 第 1 节，任务 A-03）。
//!
//! 协调器只调用 [`Tap`] 的方法：钩子在按键线程上只做闸门判断、构造事件、`try_send`，
//! 不做 IO、不等锁（NFR-PERF-01）；序列化与写管道都在 `xq-tap-writer` 线程。
//! 禁止 tokio（C-PLT-03），全部用标准库线程与通道。
//!
//! 与 17 第 1.1 节的差异：消息类型与帧编解码直接用心晴工作区的 `xqp` crate（Hub、xq-sim 同用），
//! 不再另写 `event.rs`；队列用标准库 `sync_channel`，不引入 crossbeam。
//! Hub 守护线程（17 第 1.5 节）属于 A-06，改写模式（17 第 1.6 节）在协调器里实现，都不在本 crate。

mod gate;
#[cfg(windows)]
mod pipe;
mod recent;
mod server;
mod transport;

use std::io;
use std::net::SocketAddr;
use std::sync::Arc;

pub use gate::{AppFilter, BUILTIN_EXCLUDED, DEFAULT_BLOCKLIST};
pub use recent::RecentText;
pub use xqp::{
    CandOp, CompOp, Down, KeyKind, OpenTarget, PauseBy, RewriteOutcome, RewriteSource,
    RewriteStyle, Scope,
};

use gate::Gate;
use server::{Shared, Static, lock};
use transport::{Listener, TcpServer};
use xqp::{ByeReason, KeySrc, Up};

/// `commit.text` 上限（FR-SEN-02 第 3 条）。
pub const MAX_COMMIT_TEXT: usize = 500;

/// 下行回调：`pause`、`mood`、`badge`、`tip`、`pending`、`rewrite_result`、`rewrite_fail`。
/// 在 `xq-tap-reader` 线程上调用，由协调器在状态锁内处理并刷新界面。
pub type Downlink = Box<dyn Fn(Down) + Send + Sync>;

#[derive(Debug, Clone)]
pub enum Endpoint {
    /// 命名管道名，不含 `\\.\pipe\` 前缀：`xqp::PIPE_NAME` 或 dev 的 `xqp::PIPE_NAME_DEV`。仅 Windows。
    Pipe(String),
    /// 本机 TCP（测试与非 Windows 联调，配合 Hub 的 `XQ_XQP_TCP`）；只允许回环地址，端口 0 表示随机。
    Tcp(SocketAddr),
}

#[derive(Debug, Clone)]
pub struct TapConfig {
    /// `xinqing.enabled`；为假时不监听、不采集。
    pub enabled: bool,
    pub ime_ver: String,
    /// 上行 `hello.caps`（10 第 2.3 节）。
    pub caps: Vec<String>,
    /// `xinqing.app_blocklist`；Hub 的 `cfg` 带名单时以 Hub 为准。
    pub app_blocklist: Vec<String>,
    pub app_allowlist: Vec<String>,
    pub endpoint: Endpoint,
}

impl TapConfig {
    pub fn new(ime_ver: impl Into<String>, endpoint: Endpoint) -> Self {
        Self {
            enabled: true,
            ime_ver: ime_ver.into(),
            caps: vec!["core_keys".into()],
            app_blocklist: DEFAULT_BLOCKLIST.iter().map(|s| s.to_string()).collect(),
            app_allowlist: Vec::new(),
            endpoint,
        }
    }
}

/// 按键钩子参数（FR-SEN-01）。
#[derive(Debug, Clone, Copy)]
pub struct KeyHook {
    pub kind: KeyKind,
    pub vk: u8,
    pub in_comp: bool,
    /// 按着 Ctrl / Alt / Win：只记 `other`，不记键位（FR-SEN-01 第 4 条）。
    pub modified: bool,
}

/// 上屏钩子参数（FR-SEN-02）。
#[derive(Debug, Clone, Copy)]
pub struct CommitHook<'a> {
    pub chars: u32,
    pub keystrokes: u32,
    /// 候选序号，-1 表示未知。
    pub cand_pos: i32,
    /// 清风 `CommitSource` 的名字（candidate / punctuation / mode_switch 等）。
    pub src: &'a str,
    pub text: &'a str,
}

/// 焦点钩子参数（FR-SEN-03）。
#[derive(Debug, Clone, Copy)]
pub struct FocusHook<'a> {
    /// 进程名，取自 `pid_names` 缓存；只发给本机 Hub。
    pub app: Option<&'a str>,
    pub scope: Scope,
}

#[derive(Debug, Clone)]
pub struct RewriteReq {
    pub req_id: u32,
    pub source: RewriteSource,
    /// 超过 300 字时只发前 300 字。
    pub text: String,
    pub style: RewriteStyle,
    /// 仅 `recent` 来源：将要删除的原文长度（UTF-16）。
    pub replace_len: Option<u32>,
}

pub struct Tap {
    shared: Arc<Shared>,
    local_addr: Option<SocketAddr>,
}

impl Tap {
    /// 启动 `xq-tap-accept` 与 `xq-tap-writer`。TCP 端口绑定失败、或在非 Windows 上用命名管道时返回错误。
    pub fn start(cfg: TapConfig, downlink: Downlink) -> io::Result<Arc<Tap>> {
        let (listener, local_addr): (Box<dyn Listener>, _) = match &cfg.endpoint {
            Endpoint::Tcp(addr) => {
                let s = TcpServer::bind(*addr)?;
                let local = s.local_addr()?;
                (Box::new(s), Some(local))
            }
            #[cfg(windows)]
            Endpoint::Pipe(name) => (Box::new(pipe::PipeServer::new(name)?), None),
            #[cfg(not(windows))]
            Endpoint::Pipe(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "命名管道只在 Windows 上可用，请改用 Endpoint::Tcp",
                ));
            }
        };
        let gate = Gate::default();
        gate.set_enabled(cfg.enabled);
        let info = Static {
            ime_ver: cfg.ime_ver,
            caps: cfg.caps,
            session: uuid::Uuid::new_v4().to_string(),
        };
        let filter = AppFilter::new(&cfg.app_blocklist, &cfg.app_allowlist);
        let (shared, rx) = Shared::new(gate, filter, info, downlink);

        let s = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("xq-tap-writer".into())
            .spawn(move || s.write_loop(rx))?;
        let s = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("xq-tap-accept".into())
            .spawn(move || s.accept_loop(listener))?;
        Ok(Arc::new(Tap { shared, local_addr }))
    }

    /// `Endpoint::Tcp` 实际监听的地址。
    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.local_addr
    }

    /// 本次会话的 uuid（上行 `hello.session`）。
    pub fn session(&self) -> &str {
        &self.shared.info.session
    }

    /// Hub 是否已连上并完成握手。
    pub fn is_linked(&self) -> bool {
        self.shared.is_linked()
    }

    /// 无痕模式是否开启（菜单显示用）。
    pub fn paused(&self) -> bool {
        self.shared.gate.paused()
    }

    /// 改写能否使用；为假时改写快捷键只弹一次同意询问（10 第 2.5 节 `cfg.rewrite`）。
    pub fn rewrite_enabled(&self) -> bool {
        self.shared.gate.rewrite() && self.shared.is_linked()
    }

    // —— 钩子：只做闸门判断 + try_send ——

    /// `handle_key_event` 入口（FR-SEN-01）。
    pub fn hook_key(&self, ev: KeyHook) {
        let s = &self.shared;
        let kind = if ev.modified { KeyKind::Other } else { ev.kind };
        // 光标移动、删除、回车等会让“最近上屏”与光标前的文字对不上，清空（10 第 3.1 节）
        let breaks_recent = !ev.in_comp
            && matches!(
                kind,
                KeyKind::Backspace
                    | KeyKind::Delete
                    | KeyKind::Nav
                    | KeyKind::Enter
                    | KeyKind::Esc
                    | KeyKind::Other
            );
        if breaks_recent {
            lock(&s.recent).clear();
        }
        if !s.gate.allow() {
            return;
        }
        let vk = matches!(kind, KeyKind::Letter | KeyKind::Backspace).then_some(ev.vk);
        s.push(
            true,
            Up::Key {
                ts: s.ts(),
                seq: None,
                kind,
                vk,
                in_comp: ev.in_comp,
                src: KeySrc::Core,
                eaten: None,
                del_committed: None,
            },
        );
    }

    /// `record_commit_ks`（FR-SEN-02），同时写“最近上屏”缓冲。
    pub fn hook_commit(&self, ev: CommitHook<'_>) {
        let s = &self.shared;
        if s.gate.rewrite() {
            lock(&s.recent).push(ev.text);
        }
        if !s.gate.allow() {
            return;
        }
        let (text, truncated) = if s.gate.send_text() && !ev.text.is_empty() {
            match ev.text.char_indices().nth(MAX_COMMIT_TEXT) {
                Some((cut, _)) => (Some(ev.text[..cut].to_string()), Some(true)),
                None => (Some(ev.text.to_string()), None),
            }
        } else {
            (None, None)
        };
        s.push(
            true,
            Up::Commit {
                ts: s.ts(),
                seq: None,
                chars: ev.chars,
                keystrokes: ev.keystrokes,
                cand_pos: ev.cand_pos,
                src: ev.src.to_string(),
                text,
                truncated,
            },
        );
    }

    /// `handle_focus_gained` / `handle_input_state_report`（FR-SEN-03）。
    /// 进入密码框或名单应用时闸门关闭，但仍发一次 `focus` 让 Hub 显示“闭眼”。
    pub fn hook_focus(&self, ev: FocusHook<'_>) {
        let s = &self.shared;
        let mut st = lock(&s.focus);
        if st.app.as_deref() != ev.app {
            st.app = ev.app.map(str::to_string);
            lock(&s.recent).clear();
        }
        st.scope = Some(ev.scope);
        let blocked = s
            .filter
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .blocked(ev.app);
        s.gate.set_focus(ev.scope != Scope::Normal, blocked);
        s.emit_focus(&mut st, false);
    }

    /// 组字变化（FR-SEN-04）；`len` 只在 `update` 时有意义。
    pub fn hook_comp(&self, op: CompOp, len: u16) {
        let s = &self.shared;
        if !s.gate.allow() {
            return;
        }
        let len = (op == CompOp::Update).then_some(len);
        s.push(
            true,
            Up::Comp {
                ts: s.ts(),
                seq: None,
                op,
                len,
            },
        );
    }

    /// 候选翻页 / 选词（FR-SEN-04）；`pos` 只在选词时有。
    pub fn hook_cand(&self, op: CandOp, pos: Option<u32>) {
        let s = &self.shared;
        if !s.gate.allow() {
            return;
        }
        s.push(
            true,
            Up::Cand {
                ts: s.ts(),
                seq: None,
                op,
                pos,
            },
        );
    }

    /// 输入法激活与中英状态。
    pub fn hook_ime(&self, active: bool, chinese: bool) {
        let s = &self.shared;
        if !active {
            lock(&s.recent).clear();
        }
        if !s.gate.link_open() {
            return;
        }
        s.push(
            true,
            Up::Ime {
                ts: s.ts(),
                seq: None,
                active,
                chinese,
            },
        );
    }

    /// 选区变化：清空“最近上屏”缓冲。
    pub fn hook_selection_changed(&self) {
        lock(&self.shared.recent).clear();
    }

    /// 进入改写模式时取“最近上屏”，取走即清空。
    pub fn take_recent(&self) -> Option<RecentText> {
        let s = &self.shared;
        let mut r = lock(&s.recent);
        if !s.gate.rewrite() {
            r.clear();
            return None;
        }
        r.take()
    }

    /// 发 `rewrite_req`；未连接、未同意改写或闸门关闭时返回假，协调器据此直接提示失败。
    pub fn send_rewrite_req(&self, req: RewriteReq) -> bool {
        let s = &self.shared;
        if !s.gate.rewrite() {
            return false;
        }
        let text = match req.text.char_indices().nth(recent::MAX_CHARS) {
            Some((cut, _)) => req.text[..cut].to_string(),
            None => req.text,
        };
        s.push(
            false,
            Up::RewriteReq {
                ts: s.ts(),
                seq: None,
                req_id: req.req_id,
                source: req.source,
                text,
                style: req.style,
                replace_len: req.replace_len,
            },
        )
    }

    /// 改写模式结束（FR-RWR-04，不含文字）。
    pub fn send_rewrite_done(
        &self,
        req_id: u32,
        chosen: Option<u8>,
        outcome: RewriteOutcome,
    ) -> bool {
        let s = &self.shared;
        s.push(
            false,
            Up::RewriteDone {
                ts: s.ts(),
                seq: None,
                req_id,
                chosen,
                outcome,
            },
        )
    }

    /// 用户在输入法菜单或快捷键切换无痕（FR-SEN-06）：立即关闭闸门，并告诉 Hub。
    pub fn set_paused(&self, on: bool, by: PauseBy) {
        let s = &self.shared;
        let was_open = s.gate.link_open();
        s.gate.set_paused(on);
        if on {
            lock(&s.recent).clear();
        }
        if s.gate.enabled() {
            s.push(
                false,
                Up::PauseChanged {
                    ts: s.ts(),
                    seq: None,
                    on,
                    by,
                },
            );
        }
        if !on && !was_open {
            let mut st = lock(&s.focus);
            s.emit_focus(&mut st, true);
        }
    }

    /// 请 Hub 打开某个窗口（FR-ENT-01/02）。引导窗口要在同意之前就能打开，所以不受 `collect` 约束。
    pub fn send_open(&self, target: OpenTarget) -> bool {
        let s = &self.shared;
        s.gate.enabled()
            && s.push(
                false,
                Up::Open {
                    ts: s.ts(),
                    seq: None,
                    target,
                },
            )
    }

    /// 进入或离开安全桌面（C-PLT-05）。
    pub fn set_secure_desktop(&self, on: bool) {
        let s = &self.shared;
        s.gate.set_secure_desktop(on);
        if on {
            lock(&s.recent).clear();
        }
    }

    /// 本地配置里的名单变了（Hub 的 `cfg` 带名单时会再覆盖）。
    pub fn set_app_lists(&self, blocklist: &[String], allowlist: &[String]) {
        self.shared.set_lists(Some(blocklist), Some(allowlist));
    }

    /// `xinqing.enabled` 变化：关闭时发 `bye{disabled}`、断开并停止监听。
    pub fn set_enabled(&self, on: bool) {
        let s = &self.shared;
        if s.gate.enabled() == on {
            return;
        }
        s.gate.set_enabled(on);
        if !on {
            s.send_now(Up::Bye {
                ts: Some(s.ts()),
                seq: None,
                reason: ByeReason::Disabled,
            });
            s.close_current();
            lock(&s.recent).clear();
        }
    }

    /// 核心退出：发 `bye{shutdown}`，断开并结束后台线程。
    pub fn shutdown(&self) {
        let s = &self.shared;
        s.send_now(Up::Bye {
            ts: Some(s.ts()),
            seq: None,
            reason: ByeReason::Shutdown,
        });
        s.stop();
        s.close_current();
    }
}
