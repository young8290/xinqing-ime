//! Windows 命名管道服务端（10 第 2.1 节，C-PLT-07）。
//!
//! - 安全描述符只授权当前用户 SID（另加 SYSTEM），并设 `PIPE_REJECT_REMOTE_CLIENTS`。
//!   清风 wind-rpc 的 SDDL 放行 Everyone 与 AppContainer，这里不能照抄：管道里是按键时序。
//! - 用重叠 I/O：同步句柄上的读写会互相排队，读线程等下行时写线程就发不出去。
//! - 第一次创建带 `FILE_FLAG_FIRST_PIPE_INSTANCE`，名字已被别的进程占用时创建失败，防止抢注。

use std::ffi::c_void;
use std::io::{self, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use windows::Win32::Foundation::{
    CloseHandle, ERROR_BROKEN_PIPE, ERROR_IO_PENDING, ERROR_PIPE_CONNECTED,
    ERROR_PIPE_NOT_CONNECTED, HANDLE, HLOCAL, LocalFree, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{
    GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
    TokenUser,
};
use windows::Win32::Storage::FileSystem::{
    FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, PIPE_ACCESS_DUPLEX, ReadFile, WriteFile,
};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows::Win32::System::Threading::{
    CreateEventW, GetCurrentProcess, INFINITE, OpenProcessToken, WaitForSingleObject,
};
use windows::core::{PCWSTR, PWSTR};

use crate::transport::{Conn, ConnRead, Listener};

const BUF_SIZE: u32 = 64 * 1024;

fn win_io(e: windows::core::Error) -> io::Error {
    // HRESULT_FROM_WIN32：低 16 位是 Win32 错误码
    io::Error::from_raw_os_error((e.code().0 as u32 & 0xFFFF) as i32)
}

fn is(e: &windows::core::Error, code: windows::Win32::Foundation::WIN32_ERROR) -> bool {
    e.code() == code.to_hresult()
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

struct Handle(HANDLE);

// SAFETY: 内核句柄可以跨线程使用；关闭只在 Drop 里发生一次。
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: 句柄由本模块创建并独占所有权。
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

fn event() -> io::Result<Handle> {
    // SAFETY: 无名手动复位事件，参数都是常量。
    let h = unsafe { CreateEventW(None, true, false, PCWSTR::null()) }.map_err(win_io)?;
    Ok(Handle(h))
}

/// 当前进程用户的 SID 字符串，如 `S-1-5-21-…`。
fn current_user_sid() -> io::Result<String> {
    // SAFETY: 标准的 OpenProcessToken + GetTokenInformation(TokenUser) 两段式调用；
    // 缓冲按 u64 对齐，TOKEN_USER 的 SID 指针指向缓冲内部，在缓冲释放前转成字符串。
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).map_err(win_io)?;
        let token = Handle(token);
        let mut len = 0u32;
        let _ = GetTokenInformation(token.0, TokenUser, None, 0, &mut len);
        let mut buf = vec![0u64; (len as usize).div_ceil(8)];
        GetTokenInformation(
            token.0,
            TokenUser,
            Some(buf.as_mut_ptr() as *mut c_void),
            len,
            &mut len,
        )
        .map_err(win_io)?;
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut s = PWSTR::null();
        ConvertSidToStringSidW(user.User.Sid, &mut s).map_err(win_io)?;
        let sid = s.to_string();
        let _ = LocalFree(HLOCAL(s.0 as *mut c_void));
        sid.map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }
}

/// `ConvertStringSecurityDescriptorToSecurityDescriptorW` 分配的描述符，Drop 时释放。
struct SecDesc(PSECURITY_DESCRIPTOR);

impl SecDesc {
    fn from_sddl(sddl: &[u16]) -> io::Result<Self> {
        let mut psd = PSECURITY_DESCRIPTOR::default();
        // SAFETY: sddl 以 0 结尾；成功后 psd 由我们负责 LocalFree。
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(sddl.as_ptr()),
                SDDL_REVISION_1,
                &mut psd,
                None,
            )
        }
        .map_err(win_io)?;
        Ok(Self(psd))
    }
}

impl Drop for SecDesc {
    fn drop(&mut self) {
        // SAFETY: 由 ConvertStringSecurityDescriptorToSecurityDescriptorW 分配。
        unsafe {
            let _ = LocalFree(HLOCAL(self.0.0));
        }
    }
}

/// 一个管道实例；关闭只做“取消 + 断开”，句柄等读写两端都放手后才关，避免句柄复用。
struct Pipe {
    h: Handle,
    closed: AtomicBool,
}

impl Pipe {
    fn close(&self) {
        if !self.closed.swap(true, Ordering::SeqCst) {
            // SAFETY: 句柄在 self 存活期间有效。
            unsafe {
                let _ = CancelIoEx(self.h.0, None);
                let _ = DisconnectNamedPipe(self.h.0);
            }
        }
    }
}

/// 管道的一端（读或写），各自一个事件对象，可以在两个线程上同时进行。
struct Side {
    pipe: Arc<Pipe>,
    ev: Handle,
    timeout: Option<Duration>,
}

impl Side {
    fn new(pipe: Arc<Pipe>) -> io::Result<Self> {
        Ok(Self {
            pipe,
            ev: event()?,
            timeout: None,
        })
    }

    /// 发起一次重叠操作并等它结束；超时则取消并等取消完成，OVERLAPPED 不会悬空。
    fn io(
        &mut self,
        start: impl FnOnce(HANDLE, *mut OVERLAPPED) -> windows::core::Result<()>,
    ) -> io::Result<usize> {
        if self.pipe.closed.load(Ordering::SeqCst) {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        let h = self.pipe.h.0;
        let mut ov = OVERLAPPED {
            hEvent: self.ev.0,
            ..Default::default()
        };
        match start(h, &mut ov) {
            Ok(()) => {}
            Err(e) if is(&e, ERROR_IO_PENDING) => {
                let ms = self.timeout.map_or(INFINITE, |t| {
                    t.as_millis().min(u128::from(INFINITE - 1)) as u32
                });
                // SAFETY: ov 与事件在本函数返回前一直有效；超时分支等取消完成后才返回。
                unsafe {
                    if WaitForSingleObject(self.ev.0, ms) == WAIT_TIMEOUT {
                        let _ = CancelIoEx(h, Some(&ov));
                        let mut n = 0u32;
                        let _ = GetOverlappedResult(h, &ov, &mut n, true);
                        return Err(io::ErrorKind::TimedOut.into());
                    }
                }
            }
            Err(e) => return Err(win_io(e)),
        }
        let mut n = 0u32;
        // SAFETY: 同上。
        unsafe { GetOverlappedResult(h, &ov, &mut n, true) }.map_err(win_io)?;
        Ok(n as usize)
    }
}

impl Read for Side {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let r = self.io(|h, ov| {
            // SAFETY: buf 在 io() 返回前有效。
            unsafe { ReadFile(h, Some(buf), None, Some(ov)) }
        });
        match r {
            // 对端关闭视为 EOF
            Err(e)
                if e.raw_os_error() == Some(ERROR_BROKEN_PIPE.0 as i32)
                    || e.raw_os_error() == Some(ERROR_PIPE_NOT_CONNECTED.0 as i32) =>
            {
                Ok(0)
            }
            r => r,
        }
    }
}

impl ConnRead for Side {
    fn set_read_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.timeout = timeout;
        Ok(())
    }
}

impl Write for Side {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.io(|h, ov| {
            // SAFETY: buf 在 io() 返回前有效。
            unsafe { WriteFile(h, Some(buf), None, Some(ov)) }
        })?;
        if n == 0 && !buf.is_empty() {
            return Err(io::ErrorKind::WriteZero.into());
        }
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// 正在等待客户端的实例。`ov` 装箱，地址在挂起的 ConnectNamedPipe 完成前不变。
struct Pending {
    pipe: Arc<Pipe>,
    ev: Handle,
    ov: Box<OVERLAPPED>,
    waiting: bool,
}

// SAFETY: OVERLAPPED 只含指针与事件句柄；Pending 只在接受线程上使用，移交线程时没有挂起的借用。
unsafe impl Send for Pending {}

impl Drop for Pending {
    fn drop(&mut self) {
        if self.waiting {
            // SAFETY: 取消挂起的连接并等它结束，之后才释放 ov。
            unsafe {
                let _ = CancelIoEx(self.pipe.h.0, Some(&*self.ov));
                let mut n = 0u32;
                let _ = GetOverlappedResult(self.pipe.h.0, &*self.ov, &mut n, true);
            }
        }
    }
}

pub(crate) struct PipeServer {
    name: Vec<u16>,
    sddl: Vec<u16>,
    first: bool,
    pending: Option<Pending>,
}

impl PipeServer {
    /// `name` 不含 `\\.\pipe\` 前缀。
    pub fn new(name: &str) -> io::Result<Self> {
        let sid = current_user_sid()?;
        Ok(Self {
            name: wide(&format!(r"\\.\pipe\{name}")),
            sddl: wide(&format!("D:P(A;;GA;;;{sid})(A;;GA;;;SY)")),
            first: true,
            pending: None,
        })
    }

    fn create(&mut self) -> io::Result<Arc<Pipe>> {
        let sd = SecDesc::from_sddl(&self.sddl)?;
        let sa = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd.0.0,
            bInheritHandle: false.into(),
        };
        let mut open_mode = PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED;
        if self.first {
            open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
        }
        // SAFETY: name 以 0 结尾，sa 与 sd 在调用期间有效。
        let h = unsafe {
            CreateNamedPipeW(
                PCWSTR(self.name.as_ptr()),
                open_mode,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                BUF_SIZE,
                BUF_SIZE,
                0,
                Some(&sa),
            )
        };
        if h.is_invalid() {
            return Err(io::Error::last_os_error());
        }
        self.first = false;
        Ok(Arc::new(Pipe {
            h: Handle(h),
            closed: AtomicBool::new(false),
        }))
    }

    /// 新建实例并发起 ConnectNamedPipe；客户端已经连上时返回 `waiting = false`。
    fn listen(&mut self) -> io::Result<Pending> {
        let pipe = self.create()?;
        let ev = event()?;
        let mut ov = Box::new(OVERLAPPED {
            hEvent: ev.0,
            ..Default::default()
        });
        // SAFETY: ov 装箱后地址固定，Pending 的 Drop 保证挂起时先取消再释放。
        let r = unsafe { ConnectNamedPipe(pipe.h.0, Some(&mut *ov)) };
        let waiting = match r {
            Ok(()) => false,
            Err(e) if is(&e, ERROR_PIPE_CONNECTED) => false,
            Err(e) if is(&e, ERROR_IO_PENDING) => true,
            Err(e) => return Err(win_io(e)),
        };
        Ok(Pending {
            pipe,
            ev,
            ov,
            waiting,
        })
    }

    fn conn(pipe: Arc<Pipe>) -> io::Result<Conn> {
        let reader = Side::new(Arc::clone(&pipe))?;
        let writer = Side::new(Arc::clone(&pipe))?;
        Ok(Conn {
            reader: Box::new(reader),
            writer: Box::new(writer),
            closer: Arc::new(move || pipe.close()),
        })
    }
}

impl Listener for PipeServer {
    fn accept(&mut self, wait: Duration) -> io::Result<Option<Conn>> {
        if self.pending.is_none() {
            self.pending = Some(self.listen()?);
        }
        let Some(p) = self.pending.as_mut() else {
            return Ok(None);
        };
        if p.waiting {
            let ms = wait.as_millis().min(u128::from(INFINITE - 1)) as u32;
            // SAFETY: 事件句柄有效。
            let r = unsafe { WaitForSingleObject(p.ev.0, ms) };
            if r != WAIT_OBJECT_0 {
                return Ok(None);
            }
            let mut n = 0u32;
            // SAFETY: 事件已触发，操作已完成。
            let done = unsafe { GetOverlappedResult(p.pipe.h.0, &*p.ov, &mut n, false) };
            p.waiting = false;
            if let Err(e) = done {
                self.pending = None;
                return Err(win_io(e));
            }
        }
        let Some(p) = self.pending.take() else {
            return Ok(None);
        };
        Self::conn(Arc::clone(&p.pipe)).map(Some)
    }

    fn idle(&mut self) {
        self.pending = None;
    }
}
