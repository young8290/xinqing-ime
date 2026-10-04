//! 心晴：协调器钩子接到 `wind-xinqing-tap` 的端到端测试（A-04～A-06，产品书 18 的 FR-SEN-01～06、
//! FR-OPS-03）。
//!
//! 装一个本机 TCP 的 Tap，测试扮演 Hub 握手并下发 `cfg{collect:true}`，再经 `MessageHandler`
//! 驱动 headless 协调器，断言 Hub 收到的上行事件。Tap 是进程级单例，所以整个文件只有一个测试。
//! 不依赖 build_dev 词库：没有词库时字母键照样进组字缓冲，Esc 照样撤销。

use std::io::ErrorKind;
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use wind_bridge::handler::KeyAction;
use wind_bridge::handler::{FocusData, KeyEventData, MessageHandler};
use wind_config::Config;
use wind_coordinator::Coordinator;
use wind_coordinator::xinqing::{self, HubView};
use wind_ipc::protocol::EVENT_KEY_DOWN;
use wind_ipc::protocol::{MOD_ALT, MOD_CTRL};
use wind_ui_types::MenuCmd;
use wind_xinqing_tap::guard::{GuardState, HubGuard, Launcher};
use wind_xinqing_tap::{
    ByeReason, CompOp, Down, Endpoint, KeyKind, MoodState, PauseBy, Scope, Tap, TapConfig,
};
use xqp::Up;

const VK_A: u32 = 0x41;
const VK_B: u32 = 0x42;
const VK_P: u32 = 0x50;
const VK_RETURN: u32 = 0x0D;
const VK_ESCAPE: u32 = 0x1B;
const PASSWORD_MASK: u64 = 1 << 31;

fn key(k: u32) -> KeyEventData {
    KeyEventData {
        key_code: k,
        scan_code: 0,
        modifiers: 0,
        event_type: EVENT_KEY_DOWN,
        toggles: 0,
        event_seq: 0,
        prev_char: 0,
    }
}

fn focus(pid: u32, name: &str, input_scope_mask: u64) -> FocusData {
    FocusData {
        x: 100,
        y: 100,
        height: 20,
        composition_start_x: 0,
        composition_start_y: 0,
        client_token: u64::from(pid) << 32,
        input_scope_mask,
        disabled: false,
        reason: 0,
        caret_source: wind_ipc::protocol::caret_source::GUI_CARET,
        bundle_id: name.into(),
        window_class: String::new(),
    }
}

struct Count(Arc<AtomicU32>);

impl Launcher for Count {
    fn launch(&mut self) -> std::io::Result<()> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

fn wait_until(mut ok: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !ok() {
        assert!(Instant::now() < deadline, "等待超时");
        std::thread::sleep(Duration::from_millis(20));
    }
}

struct Hub(TcpStream);

impl Hub {
    fn send(&mut self, d: &Down) {
        xqp::write_frame(&mut self.0, d).unwrap();
    }

    /// 下一条非心跳消息。
    fn recv(&mut self) -> Up {
        loop {
            match xqp::read_frame(&mut self.0) {
                Ok(Some(body)) => match xqp::decode_up(&body).unwrap() {
                    Up::Hb { .. } => continue,
                    up => return up,
                },
                Ok(None) => panic!("连接已关闭"),
                Err(xqp::FrameError::Io(e))
                    if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
                {
                    panic!("等待上行消息超时")
                }
                Err(e) => panic!("{e:?}"),
            }
        }
    }

    /// 收到满足条件的消息为止，返回途中跳过的消息（供断言“没有某类消息”）。
    fn until(&mut self, mut ok: impl FnMut(&Up) -> bool) -> (Up, Vec<Up>) {
        let mut skipped = Vec::new();
        loop {
            let up = self.recv();
            if ok(&up) {
                return (up, skipped);
            }
            skipped.push(up);
        }
    }
}

#[test]
fn coordinator_events_reach_hub() {
    let tap = Tap::start(
        TapConfig::new("0.1.0-test", Endpoint::Tcp("127.0.0.1:0".parse().unwrap())),
        Box::new(xinqing::downlink),
    )
    .unwrap();
    assert!(xinqing::install(Arc::clone(&tap)));
    // Hub 守护（A-06）：拉起动作换成计数，连接状态取自 Tap
    let launches = Arc::new(AtomicU32::new(0));
    let probe = Arc::clone(&tap);
    let guard = HubGuard::start(
        Box::new(move || probe.is_linked()),
        Box::new(Count(Arc::clone(&launches))),
        true,
    )
    .unwrap();
    assert!(xinqing::install_guard(Arc::clone(&guard)));
    wait_until(|| launches.load(Ordering::SeqCst) == 1);
    assert_eq!(guard.state(), GuardState::Launching);

    let mut c = Config::default();
    c.input.default.chinese_mode = true;
    let coord = Coordinator::new_headless(c, None);

    let s = TcpStream::connect(tap.local_addr().unwrap()).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let mut hub = Hub(s);
    hub.send(&Down::Hello {
        v: 1,
        hub_ver: "test".into(),
    });
    assert!(matches!(hub.recv(), Up::Hello { .. }));
    hub.send(&Down::Cfg {
        collect: true,
        send_text: false,
        rewrite: false,
        app_blocklist: None,
        app_allowlist: None,
    });

    wait_until(|| guard.state() == GuardState::Connected);
    assert_eq!(launches.load(Ordering::SeqCst), 1, "连上后不再拉起");

    // 下行 mood、badge、pending 记进 hub_view，供工具栏与菜单显示
    hub.send(&Down::Mood {
        state: MoodState::Hesitant,
        offline: false,
    });
    hub.send(&Down::Badge { on: true });
    hub.send(&Down::Pending { count: 2 });
    wait_until(|| xinqing::hub_view().pending == 2);
    assert_eq!(
        xinqing::hub_view(),
        HubView {
            mood: Some(MoodState::Hesitant),
            offline: false,
            badge: true,
            pending: 2,
        }
    );
    let labels = coord.debug_main_menu_labels();
    assert!(labels.iter().any(|l| l == "暂停感知"), "{labels:?}");
    assert!(!labels.iter().any(|l| l.contains("点击重试")));

    // 焦点：进程名来自缓存，闸门打开后先补发当前焦点
    coord.handle_focus_gained(&focus(4242, "Notepad.exe", 0));
    let (up, _) = hub.until(|u| matches!(u, Up::Focus { .. }));
    match up {
        Up::Focus {
            app,
            scope,
            blocked,
            ..
        } => {
            assert_eq!(app.as_deref(), Some("notepad.exe"));
            assert_eq!(scope, Scope::Normal);
            assert!(!blocked);
        }
        _ => unreachable!(),
    }

    // 两个字母：key 先于 comp，第二个键 in_comp
    coord.handle_key_event_policed(&key(VK_A));
    match hub.recv() {
        Up::Key {
            kind, vk, in_comp, ..
        } => {
            assert_eq!(kind, KeyKind::Letter);
            assert_eq!(vk, Some(VK_A as u8));
            assert!(!in_comp);
        }
        other => panic!("{other:?}"),
    }
    match hub.recv() {
        Up::Comp { op, len, .. } => {
            assert_eq!(op, CompOp::Update);
            assert_eq!(len, Some(1));
        }
        other => panic!("{other:?}"),
    }
    coord.handle_key_event_policed(&key(VK_B));
    match hub.recv() {
        Up::Key { in_comp, .. } => assert!(in_comp),
        other => panic!("{other:?}"),
    }
    match hub.recv() {
        Up::Comp { op, len, .. } => {
            assert_eq!(op, CompOp::Update);
            assert_eq!(len, Some(2));
        }
        other => panic!("{other:?}"),
    }

    // Esc 撤销组字
    coord.handle_key_event_policed(&key(VK_ESCAPE));
    match hub.recv() {
        Up::Key { kind, in_comp, .. } => {
            assert_eq!(kind, KeyKind::Esc);
            assert!(in_comp);
        }
        other => panic!("{other:?}"),
    }
    match hub.recv() {
        Up::Comp { op, .. } => assert_eq!(op, CompOp::Cancel),
        other => panic!("{other:?}"),
    }

    // 回车上屏编码：key 之后只有一条 commit（send_text 关着，不带原文）；统计兜底不重报
    coord.handle_key_event_policed(&key(VK_A));
    coord.handle_key_event_policed(&key(VK_B));
    coord.handle_key_event_policed(&key(VK_RETURN));
    let (up, _) = hub.until(|u| {
        matches!(
            u,
            Up::Key {
                kind: KeyKind::Enter,
                ..
            }
        )
    });
    assert!(matches!(up, Up::Key { in_comp: true, .. }));
    match hub.recv() {
        Up::Commit {
            chars, text, src, ..
        } => {
            assert_eq!(chars, 2);
            assert_eq!(text, None);
            assert_eq!(src, "raw_input");
        }
        other => panic!("{other:?}"),
    }

    // 密码框：上报 scope 后闸门关闭，之后的按键不出管道
    coord.handle_focus_gained(&focus(4343, "Login.exe", PASSWORD_MASK));
    match hub.recv() {
        Up::Focus { scope, .. } => assert_eq!(scope, Scope::Password),
        other => panic!("{other:?}"),
    }
    coord.handle_key_event_policed(&key(VK_A));
    coord.handle_key_event_policed(&key(VK_ESCAPE));

    // 回到普通框：焦点之前不应夹着密码框里的按键
    coord.handle_focus_gained(&focus(4242, "Notepad.exe", 0));
    let (_, skipped) = hub.until(|u| matches!(u, Up::Focus { .. }));
    assert!(skipped.is_empty(), "密码框里的事件漏出：{skipped:?}");

    // 无痕快捷键（A-05，FR-SEN-06）：Ctrl+Alt+P 被吃掉，Hub 收到 pause_changed{hotkey}，
    // 之后的按键不出管道
    let mut ctrl_alt_p = key(VK_P);
    ctrl_alt_p.modifiers = MOD_CTRL | MOD_ALT;
    assert!(matches!(
        coord.handle_key_event_policed(&ctrl_alt_p),
        KeyAction::Consumed
    ));
    let (up, _) = hub.until(|u| matches!(u, Up::PauseChanged { .. }));
    assert!(matches!(
        up,
        Up::PauseChanged {
            on: true,
            by: PauseBy::Hotkey,
            ..
        }
    ));
    assert!(tap.paused());
    coord.handle_key_event_policed(&key(VK_A));
    coord.handle_key_event_policed(&key(VK_ESCAPE));

    // 菜单恢复：pause_changed{menu}，随后补发焦点；中间不应夹着无痕期间的按键
    coord.debug_run_menu_cmd(MenuCmd::XinqingTogglePause);
    let (up, skipped) = hub.until(|u| matches!(u, Up::PauseChanged { .. }));
    assert!(skipped.is_empty(), "无痕期间的事件漏出：{skipped:?}");
    assert!(matches!(
        up,
        Up::PauseChanged {
            on: false,
            by: PauseBy::Menu,
            ..
        }
    ));
    assert!(matches!(hub.recv(), Up::Focus { .. }));
    assert!(!tap.paused());

    // Hub 正常退出（发 bye）：守护不重拉，菜单出现“点击重试”，点了才再拉起
    hub.send(&Down::Bye {
        reason: ByeReason::Shutdown,
    });
    wait_until(|| guard.state() == GuardState::HubQuit);
    assert_eq!(xinqing::hub_view(), HubView::default());
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(launches.load(Ordering::SeqCst), 1, "Hub 自己退出不重拉");
    let labels = coord.debug_main_menu_labels();
    assert!(
        labels.iter().any(|l| l == "心晴组件未运行，点击重试"),
        "{labels:?}"
    );
    coord.debug_run_menu_cmd(MenuCmd::XinqingRetryHub);
    wait_until(|| launches.load(Ordering::SeqCst) == 2);

    // 新的 Hub 连上
    let s = TcpStream::connect(tap.local_addr().unwrap()).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let mut hub = Hub(s);
    hub.send(&Down::Hello {
        v: 1,
        hub_ver: "test".into(),
    });
    assert!(matches!(hub.recv(), Up::Hello { .. }));
    wait_until(|| guard.state() == GuardState::Connected);

    // 核心退出：Hub 收到 bye
    xinqing::shutdown();
    let (up, _) = hub.until(|u| matches!(u, Up::Bye { .. }));
    assert!(matches!(up, Up::Bye { .. }));
}
