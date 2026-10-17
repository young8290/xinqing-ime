//! tracing → logcat。
//!
//! 不引第三方 android 日志 crate：只需要 `__android_log_write` 一个符号（liblog 是系统库，
//! 每台设备都有）。非 Android 目标（host 上跑单测、clippy）写 stderr。
//!
//! 级别默认 `info`，与桌面生产默认一致——`info` 不含任何用户输入（AGENTS.md 日志规范），
//! 核心那边已按此约束；本 crate 自己也只记生命周期事件。

use std::io;
use std::sync::Once;

use tracing::{Level, Metadata};
use tracing_subscriber::fmt::MakeWriter;

const TAG: &str = "Moodflow";

/// 安装全局 subscriber 与 panic 钩子。重复调用无副作用。
pub fn init() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = tracing_subscriber::fmt()
            .with_writer(LogcatMakeWriter)
            .with_ansi(false)
            .without_time()
            .with_target(false)
            .with_max_level(Level::INFO)
            .try_init();
        // panic 信息可能夹带运行期数据，只记位置
        std::panic::set_hook(Box::new(|info| {
            let at = info
                .location()
                .map(|l| format!("{}:{}", l.file(), l.line()))
                .unwrap_or_default();
            tracing::error!("native panic at {at}");
        }));
    });
}

struct LogcatMakeWriter;

impl<'a> MakeWriter<'a> for LogcatMakeWriter {
    type Writer = LogcatWriter;

    fn make_writer(&'a self) -> Self::Writer {
        LogcatWriter::new(Level::INFO)
    }

    fn make_writer_for(&'a self, meta: &Metadata<'_>) -> Self::Writer {
        LogcatWriter::new(*meta.level())
    }
}

/// 一条事件攒成一行，丢弃时整行写出（logcat 按调用分条，不能逐片写）。
pub struct LogcatWriter {
    level: Level,
    buf: Vec<u8>,
}

impl LogcatWriter {
    fn new(level: Level) -> Self {
        Self {
            level,
            buf: Vec::new(),
        }
    }
}

impl io::Write for LogcatWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for LogcatWriter {
    fn drop(&mut self) {
        if self.buf.is_empty() {
            return;
        }
        let line = String::from_utf8_lossy(&self.buf);
        write_line(self.level, line.trim_end());
    }
}

#[cfg(target_os = "android")]
fn write_line(level: Level, line: &str) {
    use std::ffi::{CString, c_char, c_int};

    #[link(name = "log")]
    unsafe extern "C" {
        fn __android_log_write(prio: c_int, tag: *const c_char, text: *const c_char) -> c_int;
    }

    // android/log.h: VERBOSE=2 DEBUG=3 INFO=4 WARN=5 ERROR=6
    let prio = match level {
        Level::TRACE => 2,
        Level::DEBUG => 3,
        Level::INFO => 4,
        Level::WARN => 5,
        Level::ERROR => 6,
    };
    let (Ok(tag), Ok(text)) = (CString::new(TAG), CString::new(line.replace('\0', " "))) else {
        return;
    };
    // SAFETY: 两个指针都来自存活到调用结束的 CString，liblog 不保留它们。
    unsafe {
        __android_log_write(prio, tag.as_ptr(), text.as_ptr());
    }
}

#[cfg(not(target_os = "android"))]
fn write_line(level: Level, line: &str) {
    eprintln!("{TAG} {level}: {line}");
}
