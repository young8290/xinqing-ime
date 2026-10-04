//! XQP 服务端（17 第 1.4 节）：`xq-tap-accept` 接受连接，每个连接一个 `xq-tap-reader`
//! 做握手并处理下行，唯一的 `xq-tap-writer` 把队列里的事件补上 `seq` 写给当前连接，每 5 秒插入 `hb`。
//!
//! - 同一时刻只服务一个客户端，新客户端连上时先断开旧连接（10 第 2.1 节）；
//! - 队列里的每条事件记着入队时的连接编号，写线程只写给同一连接，断线后剩下的直接丢弃，
//!   Hub 未连接时不缓存（FR-SEN-07）；
//! - 收到本连接的 `cfg{collect:true}` 之前，写线程不放行任何采集事件（10 第 2.3 节）。

use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, MutexGuard, RwLock, TryLockError};
use std::time::{Duration, Instant};

use xqp::{ByeReason, Down, FrameError, PROTOCOL_VERSION, Scope, Up};

use crate::gate::{AppFilter, Gate};
use crate::recent::Recent;
use crate::transport::{Closer, Conn, Listener};

/// 队列容量（FR-SEN-07 第 1 条）。
pub(crate) const QUEUE_CAP: usize = 4096;
/// 心跳间隔（FR-SEN-07 第 3 条）。
pub(crate) const HEARTBEAT: Duration = Duration::from_secs(5);
/// 连接后等待 Hub `hello` 的时间。
pub(crate) const HELLO_TIMEOUT: Duration = Duration::from_secs(5);
/// 后台线程检查退出与停用的间隔。
const TICK: Duration = Duration::from_millis(200);
/// 发 `bye` 时最多等写线程让出链路多久（[`Shared::send_now`]）。
const SEND_NOW_WAIT: Duration = Duration::from_millis(200);

const SEQ: Ordering = Ordering::SeqCst;

pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) struct Item {
    conn: u64,
    /// 采集事件（受 `collect` 约束）；`false` 为 `pause_changed`、`open`、改写等控制事件。
    collect: bool,
    msg: Up,
}

struct Link {
    id: u64,
    w: Box<dyn Write + Send>,
    seq: u32,
    closer: Closer,
}

#[derive(Default)]
pub(crate) struct FocusState {
    pub app: Option<String>,
    pub scope: Option<Scope>,
    /// 最近一次发出的 `(app, scope, blocked)`，用于去重：同一焦点只发一次 `focus`。
    sent: Option<(Option<String>, Scope, bool)>,
}

pub(crate) struct Static {
    pub ime_ver: String,
    pub caps: Vec<String>,
    pub session: String,
}

pub(crate) struct Shared {
    pub gate: Gate,
    pub filter: RwLock<AppFilter>,
    pub focus: Mutex<FocusState>,
    pub recent: Mutex<Recent>,
    pub info: Static,
    tx: SyncSender<Item>,
    queued: AtomicU32,
    dropped: AtomicU32,
    /// 已完成握手的连接编号，0 表示没有。
    linked: AtomicU64,
    /// 已收到 `cfg{collect:true}` 的连接编号。
    collecting: AtomicU64,
    /// 最新接受的连接（可能还在握手）。
    current: AtomicU64,
    next_id: AtomicU64,
    link: Mutex<Option<Link>>,
    closer: Mutex<Option<(u64, Closer)>>,
    start: Instant,
    downlink: Box<dyn Fn(Down) + Send + Sync>,
    shutdown: AtomicBool,
}

impl Shared {
    pub fn new(
        gate: Gate,
        filter: AppFilter,
        info: Static,
        downlink: Box<dyn Fn(Down) + Send + Sync>,
    ) -> (Arc<Self>, Receiver<Item>) {
        let (tx, rx) = std::sync::mpsc::sync_channel(QUEUE_CAP);
        let shared = Self {
            gate,
            filter: RwLock::new(filter),
            focus: Mutex::default(),
            recent: Mutex::default(),
            info,
            tx,
            queued: AtomicU32::new(0),
            dropped: AtomicU32::new(0),
            linked: AtomicU64::new(0),
            collecting: AtomicU64::new(0),
            current: AtomicU64::new(0),
            next_id: AtomicU64::new(0),
            link: Mutex::new(None),
            closer: Mutex::new(None),
            start: Instant::now(),
            downlink,
            shutdown: AtomicBool::new(false),
        };
        (Arc::new(shared), rx)
    }

    /// 会话时间：相对 Tap 启动的毫秒数。
    pub fn ts(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    pub fn is_linked(&self) -> bool {
        self.linked.load(SEQ) != 0
    }

    /// 入队，不等待；队列满时丢弃并计数（FR-SEN-07 第 1 条）。
    pub fn push(&self, collect: bool, msg: Up) -> bool {
        let conn = self.linked.load(SEQ);
        if conn == 0 {
            return false;
        }
        // 先加后发：写线程可能在 fetch_add 之前就取走这条，计数不能先减到负数
        self.queued.fetch_add(1, SEQ);
        match self.tx.try_send(Item { conn, collect, msg }) {
            Ok(()) => true,
            Err(e) => {
                self.queued.fetch_sub(1, SEQ);
                if matches!(e, TrySendError::Full(_)) {
                    self.dropped.fetch_add(1, SEQ);
                }
                false
            }
        }
    }

    /// 按当前焦点发 `focus`：`force` 为假时与上次相同就不发。调用方持有 `focus` 锁。
    pub fn emit_focus(&self, st: &mut FocusState, force: bool) {
        if !self.gate.link_open() {
            st.sent = None;
            return;
        }
        let Some(scope) = st.scope else {
            return;
        };
        let blocked = self
            .filter
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .blocked(st.app.as_deref());
        // 命中名单时不带进程名（10 第 2.4 节）
        let app = if blocked { None } else { st.app.clone() };
        let key = (app.clone(), scope, blocked);
        if !force && st.sent.as_ref() == Some(&key) {
            return;
        }
        st.sent = Some(key);
        let ts = self.ts();
        self.push(
            true,
            Up::Focus {
                ts,
                seq: None,
                app,
                scope,
                blocked,
            },
        );
    }

    /// 闸门从关到开时补发当前焦点，让 Hub 知道此刻在哪个应用里。
    fn refocus(&self, was_open: bool) {
        let mut st = lock(&self.focus);
        self.emit_focus(&mut st, !was_open);
    }

    /// 名单变了：按当前焦点重算 `app_blocked`。
    pub fn set_lists(&self, block: Option<&[String]>, allow: Option<&[String]>) {
        let was_open = self.gate.link_open();
        let st = lock(&self.focus);
        let blocked = {
            let mut f = self.filter.write().unwrap_or_else(|e| e.into_inner());
            if let Some(b) = block {
                f.set_blocklist(b);
            }
            if let Some(a) = allow {
                f.set_allowlist(a);
            }
            f.blocked(st.app.as_deref())
        };
        if st.scope.is_some() {
            self.gate.set_app_blocked(blocked);
        }
        drop(st);
        self.refocus(was_open);
    }

    /// 处理一条下行消息；返回假表示应断开。
    fn on_down(&self, id: u64, msg: Down) -> bool {
        match msg {
            Down::Hello { .. } => {}
            Down::Cfg {
                collect,
                send_text,
                rewrite,
                app_blocklist,
                app_allowlist,
            } => {
                let was_open = self.gate.link_open();
                if app_blocklist.is_some() || app_allowlist.is_some() {
                    self.set_lists(app_blocklist.as_deref(), app_allowlist.as_deref());
                }
                self.gate.set_consent(collect, send_text, rewrite);
                self.collecting.store(if collect { id } else { 0 }, SEQ);
                if !self.gate.rewrite() {
                    lock(&self.recent).clear();
                }
                tracing::info!(
                    "XQP cfg：collect={collect} send_text={send_text} rewrite={rewrite}"
                );
                self.refocus(was_open);
            }
            Down::Pause { on } => {
                let was_open = self.gate.link_open();
                self.gate.set_paused(on);
                if on {
                    lock(&self.recent).clear();
                }
                (self.downlink)(Down::Pause { on });
                self.refocus(was_open);
            }
            Down::Bye { reason } => {
                tracing::info!("Hub 断开：{reason:?}");
                return false;
            }
            other => (self.downlink)(other),
        }
        true
    }

    /// 接管新连接：先断开旧连接（含仍在握手的），再起读线程。
    fn adopt(self: &Arc<Self>, conn: Conn) {
        let id = self.next_id.fetch_add(1, SEQ) + 1;
        let old = {
            let mut slot = lock(&self.closer);
            self.current.store(id, SEQ);
            slot.replace((id, conn.closer.clone()))
        };
        if let Some((old_id, c)) = old {
            tracing::info!("新的 Hub 连接，断开旧连接");
            c();
            self.forget(old_id);
        }
        let shared = Arc::clone(self);
        let spawned = std::thread::Builder::new()
            .name("xq-tap-reader".into())
            .spawn(move || shared.serve(id, conn));
        if let Err(e) = spawned {
            tracing::error!("无法启动 xq-tap-reader：{e}");
            self.close_current();
        }
    }

    /// 断开当前连接（停用、退出、被新连接顶替时）。
    pub fn close_current(&self) {
        let cur = lock(&self.closer).take();
        if let Some((id, c)) = cur {
            c();
            self.forget(id);
        }
    }

    /// 连接 `id` 已断：如果它仍是当前链路就清掉，并撤销同意（Hub 是同意状态的唯一真相源）。
    fn forget(&self, id: u64) {
        if self.linked.compare_exchange(id, 0, SEQ, SEQ).is_ok() {
            self.collecting.store(0, SEQ);
            self.gate.clear_consent();
            lock(&self.recent).clear();
            tracing::info!("Hub 已断开");
        }
        let mut link = lock(&self.link);
        if link.as_ref().is_some_and(|l| l.id == id) {
            *link = None;
        }
    }

    /// 读线程：握手 → 循环处理下行 → 断开。
    fn serve(self: Arc<Self>, id: u64, conn: Conn) {
        let Conn {
            mut reader,
            mut writer,
            closer,
        } = conn;
        let hello = reader
            .set_read_timeout(Some(HELLO_TIMEOUT))
            .map_err(FrameError::Io)
            .and_then(|()| read_down(&mut reader));
        match hello {
            Ok(Some(Down::Hello { v, .. })) if v == PROTOCOL_VERSION => {}
            Ok(Some(Down::Hello { v, .. })) => {
                tracing::warn!("Hub 协议版本 {v} 与 {PROTOCOL_VERSION} 不一致，断开");
                let bye = Up::Bye {
                    ts: Some(self.ts()),
                    seq: None,
                    reason: ByeReason::Version,
                };
                let _ = write_frame(&mut writer, &bye);
                closer();
                return;
            }
            _ => {
                tracing::debug!("XQP 握手失败：没有收到 hello");
                closer();
                return;
            }
        }
        {
            let mut link = lock(&self.link);
            if self.current.load(SEQ) != id || self.shutdown.load(SEQ) || !self.gate.enabled() {
                closer();
                return;
            }
            let hello = Up::Hello {
                v: PROTOCOL_VERSION,
                ime_ver: self.info.ime_ver.clone(),
                session: self.info.session.clone(),
                caps: self.info.caps.clone(),
            };
            // 先标记已连接再回 hello：Hub 一收到 hello 就可能触发事件，晚标记的话这些事件
            // 会因“没有连接”被丢掉。写线程要等这把 link 锁，事件不会抢在 hello 前面。
            self.linked.store(id, SEQ);
            if write_frame(&mut writer, &hello).is_err() {
                let _ = self.linked.compare_exchange(id, 0, SEQ, SEQ);
                closer();
                return;
            }
            *link = Some(Link {
                id,
                w: writer,
                seq: 0,
                closer: closer.clone(),
            });
        }
        tracing::info!("Hub 已连接");
        let _ = reader.set_read_timeout(None);
        loop {
            match read_down(&mut reader) {
                Ok(Some(msg)) => {
                    if self.current.load(SEQ) != id || !self.on_down(id, msg) {
                        break;
                    }
                }
                Ok(None) => break,
                Err(FrameError::Invalid(_)) => continue,
                Err(e) => {
                    tracing::debug!("XQP 读取结束：{e}");
                    break;
                }
            }
        }
        closer();
        self.forget(id);
        let mut slot = lock(&self.closer);
        if slot.as_ref().is_some_and(|(cur, _)| *cur == id) {
            *slot = None;
        }
    }

    /// 写线程主体：只有它从队列取事件。
    pub fn write_loop(&self, rx: Receiver<Item>) {
        let mut next_hb = Instant::now() + HEARTBEAT;
        while !self.shutdown.load(SEQ) {
            let wait = next_hb.saturating_duration_since(Instant::now()).min(TICK);
            match rx.recv_timeout(wait) {
                Ok(item) => {
                    self.queued.fetch_sub(1, SEQ);
                    self.write_item(item);
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            if Instant::now() >= next_hb {
                next_hb = Instant::now() + HEARTBEAT;
                self.write_heartbeat();
            }
        }
    }

    fn write_item(&self, item: Item) {
        if item.collect && self.collecting.load(SEQ) != item.conn {
            return;
        }
        let mut link = lock(&self.link);
        let Some(l) = link.as_mut().filter(|l| l.id == item.conn) else {
            return;
        };
        if write_up(l, item.msg).is_err() {
            Self::broken(&mut link);
        }
        drop(link);
        self.after_broken();
    }

    fn write_heartbeat(&self) {
        let mut link = lock(&self.link);
        let Some(l) = link.as_mut() else {
            return;
        };
        let hb = Up::Hb {
            ts: self.ts(),
            seq: None,
            dropped: self.dropped.swap(0, SEQ),
            queue: self.queued.load(SEQ),
        };
        if write_up(l, hb).is_err() {
            Self::broken(&mut link);
        }
        drop(link);
        self.after_broken();
    }

    /// 写失败：关掉连接，读线程随之退出并调用 `forget`。
    fn broken(link: &mut MutexGuard<'_, Option<Link>>) {
        if let Some(l) = link.take() {
            tracing::debug!("XQP 写入失败，断开");
            (l.closer)();
        }
    }

    /// 链路已被写线程清掉时同步撤销同意，不等读线程。
    fn after_broken(&self) {
        let id = self.linked.load(SEQ);
        if id != 0 && lock(&self.link).is_none() {
            self.forget(id);
        }
    }

    /// 立即写一条控制消息（`bye`）。写线程正在写一条消息时等它写完，但最多等
    /// [`SEND_NOW_WAIT`]：Hub 不读、写线程卡死时放弃，不让退出跟着卡住。
    ///
    /// 曾经只 `try_lock` 一次，碰上写线程刚好在发心跳或事件，`bye` 就被静默丢掉。
    pub fn send_now(&self, msg: Up) {
        let deadline = Instant::now() + SEND_NOW_WAIT;
        loop {
            match self.link.try_lock() {
                Ok(mut link) => {
                    if let Some(l) = link.as_mut() {
                        let _ = write_up(l, msg);
                    }
                    return;
                }
                Err(TryLockError::Poisoned(e)) => {
                    if let Some(l) = e.into_inner().as_mut() {
                        let _ = write_up(l, msg);
                    }
                    return;
                }
                Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(TryLockError::WouldBlock) => {
                    tracing::debug!("XQP 写线程占着链路，放弃发送 bye");
                    return;
                }
            }
        }
    }

    pub fn stop(&self) {
        self.shutdown.store(true, SEQ);
    }

    pub fn stopping(&self) -> bool {
        self.shutdown.load(SEQ)
    }

    /// 接受线程主体。
    pub fn accept_loop(self: &Arc<Self>, mut listener: Box<dyn Listener>) {
        while !self.stopping() {
            if !self.gate.enabled() {
                listener.idle();
                std::thread::sleep(TICK);
                continue;
            }
            match listener.accept(TICK) {
                Ok(Some(conn)) => {
                    if self.stopping() || !self.gate.enabled() {
                        (conn.closer)();
                    } else {
                        self.adopt(conn);
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!("XQP 接受连接失败：{e}");
                    std::thread::sleep(Duration::from_secs(1));
                }
            }
        }
        listener.idle();
    }
}

fn write_frame(w: &mut dyn Write, msg: &Up) -> io::Result<()> {
    let buf = match xqp::encode(msg) {
        Ok(b) => b,
        Err(e) => {
            // 只可能是超长：丢掉这一条，连接照常
            tracing::warn!("XQP 消息无法编码，已丢弃：{e}");
            return Ok(());
        }
    };
    w.write_all(&buf)?;
    w.flush()
}

fn write_up(l: &mut Link, mut msg: Up) -> io::Result<()> {
    l.seq = l.seq.wrapping_add(1);
    msg.set_seq(l.seq);
    write_frame(&mut l.w, &msg)
}

/// 读一条下行。JSON 合法但类型或字段不认识时返回 `Invalid`，调用方跳过：
/// 协议只会新增可选字段或消息类型（10 开头的变更规则），旧核心不应因此断开。
fn read_down(mut r: &mut dyn io::Read) -> Result<Option<Down>, FrameError> {
    let Some(body) = xqp::read_frame(&mut r)? else {
        return Ok(None);
    };
    let value: serde_json::Value = serde_json::from_slice(&body)?;
    match serde_json::from_value(value) {
        Ok(msg) => Ok(Some(msg)),
        Err(e) => {
            tracing::debug!("忽略无法识别的下行消息：{e}");
            Err(FrameError::Invalid("down"))
        }
    }
}
