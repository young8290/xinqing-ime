//! Windows 命名管道传输（10 第 2.1 节）：核心是服务端，Hub 以客户端身份连接。
//! 客户端逻辑（握手、重连、补发）在 `xinqing_hub_core::infra::xqp`，这里只负责打开管道。

use std::io;
use std::time::Duration;

use async_trait::async_trait;
use tokio::net::windows::named_pipe::ClientOptions;
use xinqing_hub_core::infra::xqp::{Conn, Connector};

/// `ERROR_PIPE_BUSY`：服务端的实例都被占用（核心正在断开旧连接、接受新连接）。
const ERROR_PIPE_BUSY: i32 = 231;

pub struct PipeConnector {
    path: String,
}

impl PipeConnector {
    pub fn new(name: &str) -> Self {
        Self {
            path: format!(r"\\.\pipe\{name}"),
        }
    }
}

#[async_trait]
impl Connector for PipeConnector {
    async fn connect(&self) -> io::Result<Conn> {
        // 管道不存在（核心没运行）直接返回错误，由客户端 2 秒后重试；管道忙时稍等再试一次
        let client = match ClientOptions::new().open(&self.path) {
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
                tokio::time::sleep(Duration::from_millis(200)).await;
                ClientOptions::new().open(&self.path)?
            }
            other => other?,
        };
        let (r, w) = tokio::io::split(client);
        Ok(Conn {
            reader: Box::new(r),
            writer: Box::new(w),
        })
    }
}
