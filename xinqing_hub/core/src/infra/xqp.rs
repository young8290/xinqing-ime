//! XQP 客户端（10 第 2 节、17 第 2.1 节 `infra/xqp`）：Hub 连接输入法核心，握手、收上行事件、
//! 发下行消息、断线 2 秒后重连。
//!
//! 传输由 [`Connector`] 提供：Windows 命名管道的实现在外壳（本 crate 不依赖 windows，ADR 0007），
//! 这里只有开发调试用的 [`TcpConnector`]（只允许本机地址，配合 `xq-sim --tcp`）。
//!
//! 几条规格没写、由这里定下的约定：
//! - 核心每 5 秒发一次 `hb`（FR-SEN-07）。本次连接收到过心跳之后，超过 [`LinkOptions::idle_timeout`]
//!   （默认 15 秒，即连续 3 次）收不到任何消息就当连接已死，断开重连；从没收到心跳时不计时，
//!   以免核心在 `cfg{collect:false}` 期间不发心跳时被反复断开；
//! - 状态类下行消息（`cfg`、`pause`、`mood`、`badge`、`pending`）记住最新值，每次握手后补发，
//!   核心重启后不用等 Hub 状态再变一次；`pause` 只补发“开启”，Hub 重启不会把核心那边的无痕关掉；
//! - 其余下行消息（`tip`、`rewrite_*`）未连接时直接丢弃，不排队。

use std::io;
use std::net::SocketAddr;
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use xqp::{ByeReason, Down, FrameError, PROTOCOL_VERSION, Up};

/// 一条已建立的连接。
pub struct Conn {
    pub reader: Box<dyn AsyncRead + Send + Unpin>,
    pub writer: Box<dyn AsyncWrite + Send + Unpin>,
}

/// 建立到核心的连接。连接失败（核心没运行、管道忙）返回错误，由客户端等待后重试。
#[async_trait]
pub trait Connector: Send + Sync {
    async fn connect(&self) -> io::Result<Conn>;
}

/// 开发调试用的 TCP 传输（`xq-sim --tcp 127.0.0.1:<端口>`）。只允许本机地址（C-PLT-07）。
#[derive(Debug, Clone)]
pub struct TcpConnector {
    addr: SocketAddr,
}

impl TcpConnector {
    pub fn new(addr: SocketAddr) -> io::Result<Self> {
        if !addr.ip().is_loopback() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "XQP 只允许连接本机地址（C-PLT-07）",
            ));
        }
        Ok(Self { addr })
    }
}

#[async_trait]
impl Connector for TcpConnector {
    async fn connect(&self) -> io::Result<Conn> {
        let stream = tokio::net::TcpStream::connect(self.addr).await?;
        stream.set_nodelay(true)?;
        let (r, w) = stream.into_split();
        Ok(Conn {
            reader: Box::new(r),
            writer: Box::new(w),
        })
    }
}

#[derive(Debug, Clone)]
pub struct LinkOptions {
    /// 下行 `hello` 里的 Hub 版本。
    pub hub_ver: String,
    /// 连接失败或断开后多久重连（10 第 2.1 节：2 秒）。
    pub retry: Duration,
    /// 协议主版本不一致时多久再试（核心升级前重试没有意义）。
    pub version_retry: Duration,
    /// 等核心回 `hello` 的时间。
    pub hello_timeout: Duration,
    /// 收到过心跳之后，多久收不到任何上行消息就断开（核心每 5 秒一次心跳）。
    pub idle_timeout: Duration,
    /// 写一帧的超时：核心不再读管道时不让 Hub 卡住。
    pub write_timeout: Duration,
}

impl Default for LinkOptions {
    fn default() -> Self {
        Self {
            hub_ver: crate::HUB_VERSION.to_string(),
            retry: Duration::from_secs(2),
            version_retry: Duration::from_secs(30),
            hello_timeout: Duration::from_secs(5),
            idle_timeout: Duration::from_secs(15),
            write_timeout: Duration::from_secs(5),
        }
    }
}

/// 核心在握手 `hello` 里报的信息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerInfo {
    pub ime_ver: String,
    pub session: String,
    pub caps: Vec<String>,
}

impl PeerInfo {
    /// 还原成上行 `hello`，交给状态识别流水线（它据 `caps` 选择按键来源）。
    pub fn to_hello(&self) -> Up {
        Up::Hello {
            v: PROTOCOL_VERSION,
            ime_ver: self.ime_ver.clone(),
            session: self.session.clone(),
            caps: self.caps.clone(),
        }
    }
}

/// 连接断开的原因（只用于状态显示和日志，不含任何用户数据）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Disconnect {
    /// 核心关闭了连接。
    Closed,
    /// 核心发来 `bye`。
    Bye(ByeReason),
    /// 核心的协议主版本与 Hub 不一致。
    Version(u32),
    /// 长度超限、JSON 无法解析或握手顺序不对（10 第 2.1 节“非法帧”）。
    BadFrame(String),
    /// 超过 `idle_timeout` 没有收到任何消息。
    Idle,
    /// 读写出错或超时。
    Io(String),
}

impl Disconnect {
    /// 日志用的简短标签。
    pub fn label(&self) -> String {
        match self {
            Disconnect::Closed => "closed".into(),
            Disconnect::Bye(r) => format!("bye:{}", bye_label(*r)),
            Disconnect::Version(v) => format!("version:{v}"),
            Disconnect::BadFrame(_) => "bad_frame".into(),
            Disconnect::Idle => "idle".into(),
            Disconnect::Io(_) => "io".into(),
        }
    }
}

fn bye_label(r: ByeReason) -> &'static str {
    match r {
        ByeReason::Version => "version",
        ByeReason::Disabled => "disabled",
        ByeReason::Shutdown => "shutdown",
    }
}

/// 客户端交给 Hub 的事件，按收到的顺序。
#[derive(Debug, Clone, PartialEq)]
pub enum LinkEvent {
    /// 握手完成（已收到核心 `hello`，已下发 `cfg`）。
    Connected(PeerInfo),
    /// 一条上行消息（`hello`、`bye` 不在此列）。
    Up(Up),
    /// 已握手的连接断开。连接失败不报告，避免核心没运行时每 2 秒一条。
    Disconnected(Disconnect),
}

/// 下行发送端，可以克隆。所有句柄都丢弃后客户端发 `bye` 并退出。
#[derive(Debug, Clone)]
pub struct XqpHandle {
    tx: mpsc::Sender<Down>,
}

const DOWN_QUEUE: usize = 64;
const EVENT_QUEUE: usize = 1024;

impl XqpHandle {
    /// 句柄与它的接收端（测试或不连接核心时使用；正常请用 [`XqpLink::new`]）。
    pub fn pair() -> (Self, mpsc::Receiver<Down>) {
        let (tx, rx) = mpsc::channel(DOWN_QUEUE);
        (Self { tx }, rx)
    }

    /// 下发一条消息，不等待。取值不合法（10 第 2.5 节）或队列已满时丢弃并返回 `false`。
    pub fn send(&self, msg: Down) -> bool {
        if msg.validate().is_err() {
            return false;
        }
        self.tx.try_send(msg).is_ok()
    }
}

/// 一个 XQP 客户端：`run()` 一直运行到所有 [`XqpHandle`] 被丢弃或事件接收端被丢弃。
pub struct XqpLink {
    connector: Box<dyn Connector>,
    opts: LinkOptions,
    down: mpsc::Receiver<Down>,
    events: mpsc::Sender<LinkEvent>,
    sticky: Sticky,
}

/// 状态类下行消息的最新值，每次握手后补发。
#[derive(Debug, Clone)]
struct Sticky {
    cfg: Down,
    pause: bool,
    mood: Option<Down>,
    badge: bool,
    pending: u32,
}

impl Sticky {
    fn absorb(&mut self, d: &Down) {
        match d {
            Down::Cfg { .. } => self.cfg = d.clone(),
            Down::Pause { on } => self.pause = *on,
            Down::Mood { .. } => self.mood = Some(d.clone()),
            Down::Badge { on } => self.badge = *on,
            Down::Pending { count } => self.pending = *count,
            _ => {}
        }
    }

    /// 握手后按顺序补发：`cfg` 必须第一个（10 第 2.3 节第 3 步）。
    fn replay(&self) -> Vec<Down> {
        let mut out = vec![self.cfg.clone()];
        if self.pause {
            out.push(Down::Pause { on: true });
        }
        out.extend(self.mood.clone());
        if self.badge {
            out.push(Down::Badge { on: true });
        }
        if self.pending > 0 {
            out.push(Down::Pending {
                count: self.pending,
            });
        }
        out
    }
}

/// `run()` 的退出原因之外的循环控制。
enum Flow {
    Continue(Duration),
    Stop,
}

/// 读线程的结束原因：连接问题，或 Hub 不再接收事件。
enum ReadEnd {
    Lost(Disconnect),
    Stopped,
}

impl XqpLink {
    /// `cfg` 是首次握手要下发的采集开关（Hub 是同意状态的唯一真相源，见 `domain::consent::xqp_cfg`）。
    pub fn new(
        connector: Box<dyn Connector>,
        cfg: Down,
        opts: LinkOptions,
    ) -> (Self, XqpHandle, mpsc::Receiver<LinkEvent>) {
        assert!(matches!(cfg, Down::Cfg { .. }), "初始消息必须是 cfg");
        let (handle, down) = XqpHandle::pair();
        let (events, rx) = mpsc::channel(EVENT_QUEUE);
        let link = Self {
            connector,
            opts,
            down,
            events,
            sticky: Sticky {
                cfg,
                pause: false,
                mood: None,
                badge: false,
                pending: 0,
            },
        };
        (link, handle, rx)
    }

    pub async fn run(mut self) {
        loop {
            let flow = match self.connector.connect().await {
                Ok(conn) => self.session(conn).await,
                Err(_) => Flow::Continue(self.opts.retry),
            };
            let wait = match flow {
                Flow::Continue(d) => d,
                Flow::Stop => return,
            };
            if let Flow::Stop = self.idle(wait).await {
                return;
            }
        }
    }

    /// 未连接时等待：记下状态类消息，其余丢弃。
    async fn idle(&mut self, wait: Duration) -> Flow {
        let deadline = tokio::time::sleep(wait);
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                _ = &mut deadline => return Flow::Continue(Duration::ZERO),
                msg = self.down.recv() => match msg {
                    Some(d) => self.sticky.absorb(&d),
                    None => return Flow::Stop,
                },
            }
        }
    }

    async fn session(&mut self, conn: Conn) -> Flow {
        let Conn { mut reader, writer } = conn;
        let mut w = Writer {
            inner: writer,
            timeout: self.opts.write_timeout,
        };
        let peer = match self.handshake(&mut reader, &mut w).await {
            Ok(p) => p,
            Err(d) => return self.retry_after(&d),
        };
        if self.events.send(LinkEvent::Connected(peer)).await.is_err() {
            return Flow::Stop;
        }
        let end = self.serve(reader, &mut w).await;
        match end {
            ReadEnd::Stopped => Flow::Stop,
            ReadEnd::Lost(d) => {
                let flow = self.retry_after(&d);
                if self.events.send(LinkEvent::Disconnected(d)).await.is_err() {
                    return Flow::Stop;
                }
                flow
            }
        }
    }

    fn retry_after(&self, d: &Disconnect) -> Flow {
        match d {
            Disconnect::Version(_) | Disconnect::Bye(ByeReason::Version) => {
                Flow::Continue(self.opts.version_retry)
            }
            _ => Flow::Continue(self.opts.retry),
        }
    }

    /// 10 第 2.3 节：Hub 先发 `hello` → 等核心 `hello` → 发 `cfg`（及其余状态类消息）。
    async fn handshake(
        &mut self,
        reader: &mut (dyn AsyncRead + Send + Unpin),
        w: &mut Writer,
    ) -> Result<PeerInfo, Disconnect> {
        w.send(&Down::Hello {
            v: PROTOCOL_VERSION,
            hub_ver: self.opts.hub_ver.clone(),
        })
        .await?;
        let body = match tokio::time::timeout(self.opts.hello_timeout, read_frame(reader)).await {
            Err(_) => return Err(Disconnect::Io("等待核心 hello 超时".into())),
            Ok(r) => r?.ok_or(Disconnect::Closed)?,
        };
        let peer = match xqp::decode_up(&body).map_err(bad_frame)? {
            Up::Hello { v, .. } if v != PROTOCOL_VERSION => return Err(Disconnect::Version(v)),
            Up::Hello {
                ime_ver,
                session,
                caps,
                ..
            } => PeerInfo {
                ime_ver,
                session,
                caps,
            },
            Up::Bye { reason, .. } => return Err(Disconnect::Bye(reason)),
            _ => return Err(Disconnect::BadFrame("核心第一条消息不是 hello".into())),
        };
        // 握手期间排队的下行消息先并入状态，保证补发的是最新值
        while let Ok(d) = self.down.try_recv() {
            self.sticky.absorb(&d);
        }
        for d in self.sticky.replay() {
            w.send(&d).await?;
        }
        Ok(peer)
    }

    /// 握手之后：读线程转发上行事件，这里负责写下行消息，任一方结束则整个连接结束。
    async fn serve(
        &mut self,
        reader: Box<dyn AsyncRead + Send + Unpin>,
        w: &mut Writer,
    ) -> ReadEnd {
        let mut read = AbortOnDrop(tokio::spawn(read_loop(
            reader,
            self.events.clone(),
            self.opts.idle_timeout,
        )));
        loop {
            tokio::select! {
                end = &mut read.0 => {
                    return end.unwrap_or_else(|e| ReadEnd::Lost(Disconnect::Io(e.to_string())));
                }
                msg = self.down.recv() => {
                    let Some(d) = msg else {
                        // Hub 退出：礼貌地告诉核心（10 第 2.5 节 `bye`），写失败也无所谓
                        let _ = w.send(&Down::Bye { reason: ByeReason::Shutdown }).await;
                        return ReadEnd::Stopped;
                    };
                    self.sticky.absorb(&d);
                    if let Err(e) = w.send(&d).await {
                        return ReadEnd::Lost(e);
                    }
                }
            }
        }
    }
}

struct AbortOnDrop<T>(JoinHandle<T>);

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

struct Writer {
    inner: Box<dyn AsyncWrite + Send + Unpin>,
    timeout: Duration,
}

impl Writer {
    async fn send(&mut self, msg: &Down) -> Result<(), Disconnect> {
        let frame = xqp::encode(msg).map_err(bad_frame)?;
        let write = async {
            self.inner.write_all(&frame).await?;
            self.inner.flush().await
        };
        match tokio::time::timeout(self.timeout, write).await {
            Err(_) => Err(Disconnect::Io("写管道超时".into())),
            Ok(Err(e)) => Err(Disconnect::Io(e.to_string())),
            Ok(Ok(())) => Ok(()),
        }
    }
}

fn bad_frame(e: FrameError) -> Disconnect {
    match e {
        FrameError::Io(e) => Disconnect::Io(e.to_string()),
        other => Disconnect::BadFrame(other.to_string()),
    }
}

async fn read_loop(
    mut reader: Box<dyn AsyncRead + Send + Unpin>,
    events: mpsc::Sender<LinkEvent>,
    idle: Duration,
) -> ReadEnd {
    let mut heartbeat_seen = false;
    loop {
        let frame = if heartbeat_seen {
            match tokio::time::timeout(idle, read_frame(&mut reader)).await {
                Err(_) => return ReadEnd::Lost(Disconnect::Idle),
                Ok(r) => r,
            }
        } else {
            read_frame(&mut reader).await
        };
        let body = match frame {
            Err(d) => return ReadEnd::Lost(d),
            Ok(None) => return ReadEnd::Lost(Disconnect::Closed),
            Ok(Some(b)) => b,
        };
        let up = match xqp::decode_up(&body) {
            Ok(u) => u,
            Err(e) => return ReadEnd::Lost(bad_frame(e)),
        };
        heartbeat_seen |= matches!(up, Up::Hb { .. });
        match up {
            Up::Bye { reason, .. } => return ReadEnd::Lost(Disconnect::Bye(reason)),
            // 重复的 hello 没有意义，忽略
            Up::Hello { .. } => {}
            // 事件队列满时在这里等待：Hub 不读管道，核心那边的队列满了就丢弃并在心跳里计数（FR-SEN-07）
            up => {
                if events.send(LinkEvent::Up(up)).await.is_err() {
                    return ReadEnd::Stopped;
                }
            }
        }
    }
}

/// 读一帧；对端在帧边界关闭时返回 `Ok(None)`。
async fn read_frame<R: AsyncRead + Unpin + ?Sized>(
    r: &mut R,
) -> Result<Option<Vec<u8>>, Disconnect> {
    let mut prefix = [0u8; 4];
    match r.read_exact(&mut prefix).await {
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(Disconnect::Io(e.to_string())),
    }
    let len = xqp::frame_len(prefix).map_err(bad_frame)?;
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)
        .await
        .map_err(|e| Disconnect::Io(e.to_string()))?;
    Ok(Some(body))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(collect: bool) -> Down {
        Down::Cfg {
            collect,
            send_text: false,
            rewrite: false,
            app_blocklist: None,
            app_allowlist: None,
        }
    }

    #[test]
    fn tcp_connector_rejects_remote_addresses() {
        assert!(TcpConnector::new("127.0.0.1:18765".parse().unwrap()).is_ok());
        assert!(TcpConnector::new("[::1]:18765".parse().unwrap()).is_ok());
        assert!(TcpConnector::new("192.168.1.2:18765".parse().unwrap()).is_err());
    }

    #[test]
    fn sticky_replays_cfg_first_and_only_pause_on() {
        let mut s = Sticky {
            cfg: cfg(false),
            pause: false,
            mood: None,
            badge: false,
            pending: 0,
        };
        assert_eq!(s.replay(), vec![cfg(false)]);
        s.absorb(&cfg(true));
        s.absorb(&Down::Pause { on: true });
        s.absorb(&Down::Mood {
            state: xqp::MoodState::Tired,
            offline: true,
        });
        s.absorb(&Down::Pending { count: 2 });
        s.absorb(&Down::Tip {
            text: "好".into(),
            ms: 1500,
        });
        let r = s.replay();
        assert_eq!(r[0], cfg(true));
        assert_eq!(r.len(), 4);
        assert!(r.contains(&Down::Pause { on: true }));
        assert!(!r.iter().any(|d| matches!(d, Down::Tip { .. })));
        s.absorb(&Down::Pause { on: false });
        assert!(!s.replay().contains(&Down::Pause { on: false }));
    }

    #[test]
    fn handle_rejects_invalid_messages() {
        let (h, mut rx) = XqpHandle::pair();
        assert!(!h.send(Down::Tip {
            text: "好".into(),
            ms: 100,
        }));
        assert!(h.send(Down::Pause { on: true }));
        assert_eq!(rx.try_recv().unwrap(), Down::Pause { on: true });
    }

    #[test]
    fn disconnect_labels_carry_no_details() {
        assert_eq!(Disconnect::BadFrame("x".into()).label(), "bad_frame");
        assert_eq!(Disconnect::Bye(ByeReason::Disabled).label(), "bye:disabled");
    }
}
