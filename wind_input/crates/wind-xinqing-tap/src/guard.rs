//! Hub 守护（17 第 1.5 节，FR-OPS-03，任务 A-06）：拉起 Hub，断线 5 秒后重拉，
//! 10 分钟内第 4 次失败就停下，等用户在菜单里点“心晴组件未运行，点击重试”。
//!
//! 状态转换全在 [`Machine`] 里，时间由调用方传入，单元测试不用真等；[`HubGuard`] 只是
//! `xq-hub-guard` 线程外壳：定时取“是否已与 Hub 握手”，需要时调 [`Launcher`]。
//! 线程不碰协调器的 `State` 锁（03 第 2.1 节）。

use std::collections::VecDeque;
use std::io;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::server::lock;

/// 拉起后这么久还没握手，算一次失败（17 第 1.5 节）。
pub const LAUNCH_TIMEOUT: Duration = Duration::from_secs(10);
/// 断开或拉起失败后等这么久再拉（FR-OPS-03）。
pub const RELAUNCH_DELAY: Duration = Duration::from_secs(5);
/// 统计失败次数的时间窗。
pub const FAIL_WINDOW: Duration = Duration::from_secs(600);
/// 时间窗内最多重拉几次；再失败一次就放弃。
pub const MAX_RELAUNCH: usize = 3;
/// Hub 的启动参数（11 第 FR-OPS-03 条）。
pub const HUB_ARGS: &[&str] = &["--background"];

const TICK: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardState {
    /// 不需要 Hub：`xinqing.enabled` 或 `xinqing.hub_autostart` 关着。
    Idle,
    /// 已拉起，等 Hub 握手。
    Launching,
    Connected,
    /// 断开或拉起失败，等 [`RELAUNCH_DELAY`] 后重拉。
    Backoff,
    /// 10 分钟内失败太多次，不再自动拉起。
    GaveUp,
    /// Hub 发了 `bye`，是它自己正常退出（用户在 Hub 里点了退出），不重拉。
    HubQuit,
}

impl GuardState {
    /// 菜单该不该显示“心晴组件未运行，点击重试”。
    pub fn needs_retry(self) -> bool {
        matches!(self, Self::GaveUp | Self::HubQuit)
    }
}

/// 守护状态机。`step` 每次返回是否要拉起 Hub。
#[derive(Debug)]
pub struct Machine {
    state: GuardState,
    since: Instant,
    failures: VecDeque<Instant>,
}

impl Machine {
    pub fn new(now: Instant) -> Self {
        Self {
            state: GuardState::Idle,
            since: now,
            failures: VecDeque::new(),
        }
    }

    pub fn state(&self) -> GuardState {
        self.state
    }

    /// `wanted` = `enabled ∧ hub_autostart`；`linked` = 已与 Hub 握手。返回真时调用方去拉起 Hub。
    pub fn step(&mut self, now: Instant, wanted: bool, linked: bool) -> bool {
        use GuardState::*;
        if !wanted {
            // 关掉再打开时立即拉起（FR-IME-02 第 3 条），之前的失败不再计较
            self.failures.clear();
            self.set(Idle, now);
            return false;
        }
        if linked {
            // 包括用户自己打开了 Hub 的情况：不管原来在等什么，连上了就是连上了
            self.set(Connected, now);
            return false;
        }
        match self.state {
            Idle => {
                self.set(Launching, now);
                true
            }
            Launching if now.saturating_duration_since(self.since) >= LAUNCH_TIMEOUT => {
                self.fail(now);
                false
            }
            Connected => {
                self.fail(now);
                false
            }
            Backoff if now.saturating_duration_since(self.since) >= RELAUNCH_DELAY => {
                self.set(Launching, now);
                true
            }
            Launching | Backoff | GaveUp | HubQuit => false,
        }
    }

    /// Hub 发来 `bye`：它是自己退出的，不按崩溃处理。
    pub fn hub_quit(&mut self, now: Instant) {
        if self.state != GuardState::Idle {
            self.set(GuardState::HubQuit, now);
        }
    }

    /// 菜单“点击重试”：清掉失败记录，下一次 `step` 立即拉起。
    pub fn retry(&mut self, now: Instant) {
        if self.state != GuardState::Connected {
            self.failures.clear();
            self.set(GuardState::Idle, now);
        }
    }

    fn fail(&mut self, now: Instant) {
        while self
            .failures
            .front()
            .is_some_and(|t| now.saturating_duration_since(*t) >= FAIL_WINDOW)
        {
            self.failures.pop_front();
        }
        self.failures.push_back(now);
        if self.failures.len() > MAX_RELAUNCH {
            tracing::warn!(
                "心晴 Hub 10 分钟内断开 {} 次，不再自动拉起",
                self.failures.len()
            );
            self.set(GuardState::GaveUp, now);
        } else {
            self.set(GuardState::Backoff, now);
        }
    }

    fn set(&mut self, state: GuardState, now: Instant) {
        if self.state != state {
            tracing::debug!("Hub 守护：{:?} → {:?}", self.state, state);
            self.state = state;
            self.since = now;
        }
    }
}

/// 拉起 Hub 的方式；测试里换成计数器。
pub trait Launcher: Send {
    fn launch(&mut self) -> io::Result<()>;
}

/// 启动 `xinqing_hub.exe --background`。Hub 自己是单实例（FR-OPS-03），已在运行时
/// 新进程把参数转给老实例后退出，所以这里不用先查进程。
pub struct ExeLauncher {
    pub path: PathBuf,
}

impl Launcher for ExeLauncher {
    fn launch(&mut self) -> io::Result<()> {
        let mut child = Command::new(&self.path)
            .args(HUB_ARGS)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        // 非 Windows 上不回收会留僵尸进程；Windows 上这个线程只是等着，不占什么
        let _ = std::thread::Builder::new()
            .name("xq-hub-reap".into())
            .spawn(move || {
                let _ = child.wait();
            });
        Ok(())
    }
}

/// `xq-hub-guard` 线程。
pub struct HubGuard {
    machine: Mutex<Machine>,
    wanted: AtomicBool,
    stop: AtomicBool,
}

impl HubGuard {
    /// 起线程。`linked` 一般是 `Tap::is_linked`。
    pub fn start(
        linked: Box<dyn Fn() -> bool + Send>,
        mut launcher: Box<dyn Launcher>,
        wanted: bool,
    ) -> io::Result<Arc<Self>> {
        let g = Arc::new(Self {
            machine: Mutex::new(Machine::new(Instant::now())),
            wanted: AtomicBool::new(wanted),
            stop: AtomicBool::new(false),
        });
        let me = Arc::clone(&g);
        std::thread::Builder::new()
            .name("xq-hub-guard".into())
            .spawn(move || {
                while !me.stop.load(Ordering::SeqCst) {
                    let wanted = me.wanted.load(Ordering::SeqCst);
                    let go = lock(&me.machine).step(Instant::now(), wanted, linked());
                    if go {
                        match launcher.launch() {
                            Ok(()) => tracing::info!("已拉起心晴 Hub"),
                            // 不单独处理：10 秒没握手就按一次失败重试，次数用完停下
                            Err(e) => tracing::warn!("拉起心晴 Hub 失败：{e}"),
                        }
                    }
                    std::thread::sleep(TICK);
                }
            })?;
        Ok(g)
    }

    /// `xinqing.enabled ∧ xinqing.hub_autostart`，配置热重载时更新。
    pub fn set_wanted(&self, on: bool) {
        self.wanted.store(on, Ordering::SeqCst);
    }

    pub fn state(&self) -> GuardState {
        lock(&self.machine).state()
    }

    /// Hub 发来 `bye`（正常退出）。
    pub fn hub_quit(&self) {
        lock(&self.machine).hub_quit(Instant::now());
    }

    /// 菜单“心晴组件未运行，点击重试”。
    pub fn retry(&self) {
        lock(&self.machine).retry(Instant::now());
    }

    /// 核心退出：线程在下一个周期结束，不再拉起 Hub（Hub 自己留着，C-PLT-12）。
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use GuardState::*;

    struct T {
        m: Machine,
        t0: Instant,
    }

    impl T {
        fn new() -> Self {
            let t0 = Instant::now();
            Self {
                m: Machine::new(t0),
                t0,
            }
        }
        fn at(&self, secs: u64) -> Instant {
            self.t0 + Duration::from_secs(secs)
        }
        fn step(&mut self, secs: u64, wanted: bool, linked: bool) -> bool {
            let now = self.at(secs);
            self.m.step(now, wanted, linked)
        }
    }

    #[test]
    fn launches_once_and_connects() {
        let mut t = T::new();
        assert!(t.step(0, true, false));
        assert_eq!(t.m.state(), Launching);
        assert!(!t.step(1, true, false), "等握手期间不重复拉起");
        assert!(!t.step(2, true, true));
        assert_eq!(t.m.state(), Connected);
    }

    #[test]
    fn not_wanted_never_launches() {
        let mut t = T::new();
        assert!(!t.step(0, false, false));
        assert!(!t.step(100, false, false));
        assert_eq!(t.m.state(), Idle);
    }

    #[test]
    fn already_running_hub_is_adopted() {
        let mut t = T::new();
        assert!(!t.step(0, true, true));
        assert_eq!(t.m.state(), Connected);
    }

    #[test]
    fn launch_timeout_backs_off_then_relaunches() {
        let mut t = T::new();
        assert!(t.step(0, true, false));
        assert!(!t.step(9, true, false));
        assert!(!t.step(10, true, false));
        assert_eq!(t.m.state(), Backoff);
        assert!(!t.step(14, true, false), "等满 5 秒");
        assert!(t.step(15, true, false));
        assert_eq!(t.m.state(), Launching);
    }

    /// TC-OPS-03：10 分钟内结束 Hub 4 次，前 3 次 5 秒内恢复，第 4 次后停止。
    #[test]
    fn fourth_crash_in_ten_minutes_gives_up() {
        let mut t = T::new();
        assert!(t.step(0, true, false));
        t.step(1, true, true);
        let mut now = 1;
        for _ in 0..3 {
            now += 60;
            assert!(!t.step(now, true, false));
            assert_eq!(t.m.state(), Backoff);
            assert!(t.step(now + 5, true, false), "5 秒后重拉");
            t.step(now + 6, true, true);
            now += 6;
        }
        assert!(!t.step(now + 60, true, false));
        assert_eq!(t.m.state(), GaveUp);
        assert!(t.m.state().needs_retry());
        assert!(!t.step(now + 600, true, false), "放弃后不再自动拉起");

        // 用户手动打开 Hub：照样认
        assert!(!t.step(now + 700, true, true));
        assert_eq!(t.m.state(), Connected);
    }

    #[test]
    fn crashes_spread_over_more_than_ten_minutes_keep_relaunching() {
        let mut t = T::new();
        t.step(0, true, true);
        let mut now = 0;
        for _ in 0..6 {
            now += 300;
            assert!(!t.step(now, true, false));
            assert_eq!(t.m.state(), Backoff);
            assert!(t.step(now + 5, true, false));
            t.step(now + 6, true, true);
        }
    }

    #[test]
    fn retry_clears_failures_and_launches() {
        let mut t = T::new();
        t.step(0, true, true);
        for i in 0..4 {
            t.step(10 + i * 20, true, false);
            t.step(15 + i * 20, true, false);
        }
        assert_eq!(t.m.state(), GaveUp);
        t.m.retry(t.at(200));
        assert!(t.step(200, true, false));
        assert_eq!(t.m.state(), Launching);
    }

    #[test]
    fn hub_quit_is_not_a_crash() {
        let mut t = T::new();
        t.step(0, true, true);
        t.m.hub_quit(t.at(5));
        assert!(!t.step(6, true, false));
        assert!(!t.step(600, true, false));
        assert_eq!(t.m.state(), HubQuit);
        assert!(t.m.state().needs_retry());
        t.m.retry(t.at(700));
        assert!(t.step(700, true, false));
    }

    #[test]
    fn disable_then_enable_relaunches_immediately() {
        let mut t = T::new();
        t.step(0, true, true);
        for i in 0..4 {
            t.step(10 + i * 20, true, false);
            t.step(15 + i * 20, true, false);
        }
        assert_eq!(t.m.state(), GaveUp);
        assert!(!t.step(100, false, false));
        assert_eq!(t.m.state(), Idle);
        assert!(t.step(101, true, false), "重新打开心晴立即拉起");
    }

    struct Count(Arc<Mutex<u32>>);
    impl Launcher for Count {
        fn launch(&mut self) -> io::Result<()> {
            *lock(&self.0) += 1;
            Ok(())
        }
    }

    #[test]
    fn guard_thread_launches_and_stops() {
        let n = Arc::new(Mutex::new(0));
        let linked = Arc::new(AtomicBool::new(false));
        let l2 = Arc::clone(&linked);
        let g = HubGuard::start(
            Box::new(move || l2.load(Ordering::SeqCst)),
            Box::new(Count(Arc::clone(&n))),
            true,
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while *lock(&n) == 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(*lock(&n), 1);
        assert_eq!(g.state(), Launching);
        linked.store(true, Ordering::SeqCst);
        let deadline = Instant::now() + Duration::from_secs(3);
        while g.state() != Connected && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(g.state(), Connected);
        g.set_wanted(false);
        let deadline = Instant::now() + Duration::from_secs(3);
        while g.state() != Idle && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(g.state(), Idle);
        g.stop();
        assert_eq!(*lock(&n), 1);
    }
}
