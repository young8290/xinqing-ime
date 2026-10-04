//! 传输层：Windows 命名管道（正式）与本机 TCP（测试、非 Windows 联调）。
//!
//! 服务端逻辑只依赖这里的 [`Listener`] / [`Conn`]，所以能在 Linux 上用 TCP 跑完整测试。

use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::time::Duration;

/// 关闭连接：让另一线程上阻塞的读写立即返回错误。可重复调用。
pub(crate) type Closer = Arc<dyn Fn() + Send + Sync>;

pub(crate) trait ConnRead: Read + Send {
    /// `None` 表示一直等。
    fn set_read_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()>;
}

pub(crate) struct Conn {
    pub reader: Box<dyn ConnRead>,
    pub writer: Box<dyn Write + Send>,
    pub closer: Closer,
}

pub(crate) trait Listener: Send {
    /// 最多等 `wait`；超时返回 `Ok(None)`，好让调用方检查是否该退出或停用。
    fn accept(&mut self, wait: Duration) -> io::Result<Option<Conn>>;

    /// 停用期间释放正在监听的实例，让客户端连不上（命名管道用；TCP 无事可做）。
    fn idle(&mut self) {}
}

pub(crate) struct TcpServer {
    listener: TcpListener,
}

impl TcpServer {
    /// 只允许回环地址：这条通道带着按键时序，绝不能对外开放。
    pub fn bind(addr: SocketAddr) -> io::Result<Self> {
        if !addr.ip().is_loopback() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "XQP 的 TCP 传输只允许回环地址",
            ));
        }
        let listener = TcpListener::bind(addr)?;
        listener.set_nonblocking(true)?;
        Ok(Self { listener })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }
}

const POLL: Duration = Duration::from_millis(20);

impl Listener for TcpServer {
    fn accept(&mut self, wait: Duration) -> io::Result<Option<Conn>> {
        let mut waited = Duration::ZERO;
        loop {
            match self.listener.accept() {
                Ok((stream, _)) => return tcp_conn(stream).map(Some),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    if waited >= wait {
                        return Ok(None);
                    }
                    std::thread::sleep(POLL);
                    waited += POLL;
                }
                Err(e) => return Err(e),
            }
        }
    }
}

fn tcp_conn(stream: TcpStream) -> io::Result<Conn> {
    stream.set_nonblocking(false)?;
    stream.set_nodelay(true)?;
    let writer = stream.try_clone()?;
    let shut = stream.try_clone()?;
    Ok(Conn {
        reader: Box::new(TcpRead(stream)),
        writer: Box::new(writer),
        closer: Arc::new(move || {
            let _ = shut.shutdown(Shutdown::Both);
        }),
    })
}

struct TcpRead(TcpStream);

impl Read for TcpRead {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf)
    }
}

impl ConnRead for TcpRead {
    fn set_read_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.0.set_read_timeout(timeout)
    }
}
