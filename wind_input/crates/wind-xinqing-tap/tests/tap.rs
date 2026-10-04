//! 用本机 TCP 扮演 Hub，端到端测试握手、闸门、背压与断线（17 第 1.1 节 tests/）。

use std::io::{ErrorKind, Read};
use std::net::TcpStream;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use wind_xinqing_tap::{
    CandOp, CommitHook, CompOp, Down, Endpoint, FocusHook, KeyHook, KeyKind, OpenTarget, PauseBy,
    RewriteReq, RewriteSource, RewriteStyle, Scope, Tap, TapConfig,
};
use xqp::{ByeReason, MoodState, Up};

const WAIT: Duration = Duration::from_secs(3);

fn start() -> (Arc<Tap>, Arc<Mutex<Vec<Down>>>) {
    let got = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&got);
    let cfg = TapConfig::new("0.1.0-test", Endpoint::Tcp("127.0.0.1:0".parse().unwrap()));
    let tap = Tap::start(cfg, Box::new(move |d| sink.lock().unwrap().push(d))).unwrap();
    (tap, got)
}

struct Hub {
    s: TcpStream,
    got: Arc<Mutex<Vec<Down>>>,
    /// 上行 `seq` 按连接从 1 起连续递增（心跳也占号），每条都检查。
    last_seq: u32,
}

impl Hub {
    fn connect(tap: &Tap, got: &Arc<Mutex<Vec<Down>>>) -> Hub {
        let s = TcpStream::connect(tap.local_addr().unwrap()).unwrap();
        s.set_read_timeout(Some(WAIT)).unwrap();
        Hub {
            s,
            got: Arc::clone(got),
            last_seq: 0,
        }
    }

    fn hello(tap: &Tap, got: &Arc<Mutex<Vec<Down>>>, v: u32) -> Hub {
        let mut hub = Hub::connect(tap, got);
        hub.send(&Down::Hello {
            v,
            hub_ver: "test".into(),
        });
        hub
    }

    /// 握手并下发 `cfg`。
    fn linked(
        tap: &Tap,
        got: &Arc<Mutex<Vec<Down>>>,
        collect: bool,
        send_text: bool,
        rewrite: bool,
    ) -> Hub {
        let mut hub = Hub::hello(tap, got, 1);
        assert!(matches!(hub.recv(), Up::Hello { .. }));
        hub.cfg(collect, send_text, rewrite);
        hub
    }

    /// 下发 `cfg` 并等核心处理完：下行按序处理，回调收到其后的屏障消息即说明 `cfg` 已生效。
    fn cfg(&mut self, collect: bool, send_text: bool, rewrite: bool) {
        self.send(&Down::Cfg {
            collect,
            send_text,
            rewrite,
            app_blocklist: None,
            app_allowlist: None,
        });
        self.barrier();
    }

    fn barrier(&mut self) {
        static NEXT: AtomicU32 = AtomicU32::new(1_000_000);
        let mark = Down::Pending {
            count: NEXT.fetch_add(1, Ordering::SeqCst),
        };
        self.send(&mark);
        let got = Arc::clone(&self.got);
        wait_for(|| got.lock().unwrap().contains(&mark));
    }

    fn send(&mut self, d: &Down) {
        xqp::write_frame(&mut self.s, d).unwrap();
    }

    fn try_recv_any(&mut self) -> Option<Up> {
        match xqp::read_frame(&mut self.s) {
            Ok(Some(body)) => {
                let mut up = xqp::decode_up(&body).unwrap();
                if let Some(seq) = seq_of(&mut up) {
                    assert_eq!(seq, self.last_seq + 1, "seq 应连续：{up:?}");
                    self.last_seq = seq;
                }
                Some(up)
            }
            Ok(None) => None,
            Err(xqp::FrameError::Io(e))
                if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
            {
                panic!("等待上行消息超时")
            }
            Err(_) => None,
        }
    }

    /// 下一条非心跳消息。
    fn recv(&mut self) -> Up {
        loop {
            match self.try_recv_any() {
                Some(Up::Hb { .. }) => continue,
                Some(m) => return m,
                None => panic!("连接已关闭"),
            }
        }
    }

    /// 对端关闭（读到 EOF 或连接被重置）。
    fn closed(&mut self) -> bool {
        let mut b = [0u8; 1];
        loop {
            match self.s.read(&mut b) {
                Ok(0) => return true,
                Ok(_) => continue,
                Err(e) if e.kind() == ErrorKind::ConnectionReset => return true,
                Err(_) => return false,
            }
        }
    }

    /// 发一个必定放行的控制事件作为标记，断言它之前没有别的消息。
    fn expect_nothing_before_marker(&mut self, tap: &Tap) {
        assert!(tap.send_open(OpenTarget::Dashboard));
        match self.recv() {
            Up::Open { target, .. } => assert_eq!(target, OpenTarget::Dashboard),
            other => panic!("闸门应关闭，却收到 {other:?}"),
        }
    }
}

fn seq_of(up: &mut Up) -> Option<u32> {
    let v = serde_json::to_value(&*up).unwrap();
    v.get("seq").and_then(|s| s.as_u64()).map(|s| s as u32)
}

fn key(kind: KeyKind, vk: u8) -> KeyHook {
    KeyHook {
        kind,
        vk,
        in_comp: false,
        modified: false,
    }
}

fn focus(app: &str, scope: Scope) -> FocusHook<'_> {
    FocusHook {
        app: Some(app),
        scope,
    }
}

fn commit(text: &str) -> CommitHook<'_> {
    CommitHook {
        chars: text.chars().count() as u32,
        keystrokes: 4,
        cand_pos: 0,
        src: "candidate",
        text,
    }
}

fn wait_for(mut ok: impl FnMut() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !ok() {
        assert!(Instant::now() < deadline, "等待超时");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn handshake_then_nothing_until_cfg() {
    let (tap, got) = start();
    tap.hook_focus(focus("WeChat.exe", Scope::Normal));
    let mut hub = Hub::hello(&tap, &got, 1);
    match hub.recv() {
        Up::Hello {
            v, session, caps, ..
        } => {
            assert_eq!(v, 1);
            assert_eq!(session, tap.session());
            assert_eq!(caps, ["core_keys"]);
        }
        other => panic!("{other:?}"),
    }
    wait_for(|| tap.is_linked());
    tap.hook_key(key(KeyKind::Letter, b'H'));
    tap.hook_commit(commit("你好"));
    hub.expect_nothing_before_marker(&tap);

    hub.cfg(true, false, false);
    // 闸门打开时补发当前焦点
    match hub.recv() {
        Up::Focus {
            app,
            scope,
            blocked,
            ..
        } => {
            assert_eq!(app.as_deref(), Some("WeChat.exe"));
            assert_eq!(scope, Scope::Normal);
            assert!(!blocked);
        }
        other => panic!("{other:?}"),
    }
    tap.hook_key(key(KeyKind::Letter, 72));
    tap.hook_key(KeyHook {
        modified: true,
        ..key(KeyKind::Letter, b'C')
    });
    tap.hook_comp(CompOp::Update, 1);
    tap.hook_cand(CandOp::Select, Some(2));
    let mut seqs = Vec::new();
    match hub.recv() {
        Up::Key { kind, vk, seq, .. } => {
            assert_eq!((kind, vk), (KeyKind::Letter, Some(72)));
            seqs.push(seq.unwrap());
        }
        other => panic!("{other:?}"),
    }
    match hub.recv() {
        Up::Key { kind, vk, seq, .. } => {
            assert_eq!((kind, vk), (KeyKind::Other, None), "组合键只记 other");
            seqs.push(seq.unwrap());
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        hub.recv(),
        Up::Comp {
            op: CompOp::Update,
            len: Some(1),
            ..
        }
    ));
    assert!(matches!(
        hub.recv(),
        Up::Cand {
            op: CandOp::Select,
            pos: Some(2),
            ..
        }
    ));
    assert_eq!(seqs[1], seqs[0] + 1);
}

#[test]
fn version_mismatch_gets_bye() {
    let (tap, got) = start();
    let mut hub = Hub::hello(&tap, &got, 2);
    assert!(matches!(
        hub.recv(),
        Up::Bye {
            reason: ByeReason::Version,
            ..
        }
    ));
    assert!(hub.closed());
    assert!(!tap.is_linked());
}

#[test]
fn password_box_and_blocklist_close_the_gate() {
    let (tap, got) = start();
    let mut hub = Hub::linked(&tap, &got, true, true, false);

    tap.hook_focus(focus("chrome.exe", Scope::Password));
    assert!(matches!(
        hub.recv(),
        Up::Focus {
            scope: Scope::Password,
            ..
        }
    ));
    tap.hook_focus(focus("chrome.exe", Scope::Password));
    tap.hook_key(key(KeyKind::Letter, 65));
    tap.hook_commit(commit("密码"));
    hub.expect_nothing_before_marker(&tap);

    tap.hook_focus(focus("AliPay.exe", Scope::Normal));
    match hub.recv() {
        Up::Focus { app, blocked, .. } => {
            assert!(blocked);
            assert_eq!(app, None, "命中名单时不带进程名");
        }
        other => panic!("{other:?}"),
    }
    tap.hook_key(key(KeyKind::Letter, 65));
    hub.expect_nothing_before_marker(&tap);

    tap.hook_focus(focus("notepad.exe", Scope::Normal));
    assert!(matches!(hub.recv(), Up::Focus { blocked: false, .. }));
    tap.hook_key(key(KeyKind::Letter, 65));
    assert!(matches!(hub.recv(), Up::Key { .. }));

    tap.hook_focus(focus("xinqing_hub.exe", Scope::Normal));
    assert!(matches!(hub.recv(), Up::Focus { blocked: true, .. }));
    tap.hook_key(key(KeyKind::Letter, 65));
    hub.expect_nothing_before_marker(&tap);
    // 从一个名单应用切到另一个，Hub 看到的焦点没变，不重复发
    tap.hook_focus(focus("AliPay.exe", Scope::Normal));
    hub.expect_nothing_before_marker(&tap);

    tap.hook_focus(focus("notepad.exe", Scope::Normal));
    assert!(matches!(hub.recv(), Up::Focus { blocked: false, .. }));

    // Hub 下发的白名单立即生效
    hub.send(&Down::Cfg {
        collect: true,
        send_text: true,
        rewrite: false,
        app_blocklist: None,
        app_allowlist: Some(vec!["WeChat.exe".into()]),
    });
    hub.barrier();
    assert!(matches!(hub.recv(), Up::Focus { blocked: true, .. }));
    tap.hook_key(key(KeyKind::Letter, 65));
    hub.expect_nothing_before_marker(&tap);
}

#[test]
fn commit_text_follows_send_text() {
    let (tap, got) = start();
    let mut hub = Hub::linked(&tap, &got, true, false, false);
    tap.hook_commit(commit("好的"));
    match hub.recv() {
        Up::Commit { text, chars, .. } => {
            assert_eq!(text, None);
            assert_eq!(chars, 2);
        }
        other => panic!("{other:?}"),
    }
    hub.cfg(true, true, false);
    hub.expect_nothing_before_marker(&tap);
    tap.hook_commit(commit("好的"));
    assert!(matches!(hub.recv(), Up::Commit { text: Some(t), truncated: None, .. } if t == "好的"));
    let long = "字".repeat(600);
    tap.hook_commit(commit(&long));
    match hub.recv() {
        Up::Commit {
            text, truncated, ..
        } => {
            assert_eq!(text.unwrap().chars().count(), 500);
            assert_eq!(truncated, Some(true));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn pause_from_hub_and_from_core() {
    let (tap, got) = start();
    tap.hook_focus(focus("WeChat.exe", Scope::Normal));
    let mut hub = Hub::linked(&tap, &got, true, false, false);
    assert!(matches!(hub.recv(), Up::Focus { .. }));

    hub.send(&Down::Pause { on: true });
    wait_for(|| tap.paused());
    assert!(got.lock().unwrap().contains(&Down::Pause { on: true }));
    tap.hook_key(key(KeyKind::Letter, 65));
    hub.expect_nothing_before_marker(&tap);

    hub.send(&Down::Pause { on: false });
    assert!(matches!(hub.recv(), Up::Focus { .. }), "恢复时补发焦点");
    tap.hook_key(key(KeyKind::Letter, 65));
    assert!(matches!(hub.recv(), Up::Key { .. }));

    tap.set_paused(true, PauseBy::Hotkey);
    assert!(matches!(
        hub.recv(),
        Up::PauseChanged {
            on: true,
            by: PauseBy::Hotkey,
            ..
        }
    ));
    tap.hook_key(key(KeyKind::Letter, 65));
    hub.expect_nothing_before_marker(&tap);
}

#[test]
fn pause_set_before_hub_connects_is_reported_after_hello() {
    let (tap, got) = start();
    // 启动时恢复“记住无痕”、或 Hub 没连上时按了快捷键
    tap.set_paused(true, PauseBy::Hotkey);
    let mut hub = Hub::hello(&tap, &got, 1);
    assert!(matches!(hub.recv(), Up::Hello { .. }));
    assert!(matches!(hub.recv(), Up::PauseChanged { on: true, .. }));
    hub.cfg(true, false, false);
    tap.hook_key(key(KeyKind::Letter, 65));
    hub.expect_nothing_before_marker(&tap);
}

#[test]
fn downlink_messages_reach_the_callback() {
    let (tap, got) = start();
    let mut hub = Hub::linked(&tap, &got, true, false, false);
    let mood = Down::Mood {
        state: MoodState::Hesitant,
        offline: false,
    };
    hub.send(&mood);
    hub.send(&Down::Badge { on: true });
    // 未知消息类型跳过，不断开
    xqp::write_frame(
        &mut hub.s,
        &serde_json::json!({"t": "from_the_future", "x": 1}),
    )
    .unwrap();
    hub.send(&Down::Pending { count: 2 });
    wait_for(|| got.lock().unwrap().contains(&Down::Pending { count: 2 }));
    let got = got.lock().unwrap();
    let pos = got.iter().position(|d| *d == mood).unwrap();
    assert_eq!(got[pos + 1], Down::Badge { on: true });
    assert_eq!(got[pos + 2], Down::Pending { count: 2 });
    assert!(tap.is_linked());
}

#[test]
fn hub_bye_reaches_the_callback_after_unlink() {
    // 回调里看到的连接状态：守护线程据此判断 Hub 退出，必须已经是“未连接”
    let me: Arc<std::sync::OnceLock<Arc<Tap>>> = Arc::default();
    let seen: Arc<Mutex<Option<bool>>> = Arc::default();
    let (me2, seen2) = (Arc::clone(&me), Arc::clone(&seen));
    let got = Arc::new(Mutex::new(Vec::new()));
    let cfg = TapConfig::new("0.1.0-test", Endpoint::Tcp("127.0.0.1:0".parse().unwrap()));
    let tap = Tap::start(
        cfg,
        Box::new(move |d| {
            if matches!(d, Down::Bye { .. }) {
                *seen2.lock().unwrap() = me2.get().map(|t| t.is_linked());
            }
        }),
    )
    .unwrap();
    let _ = me.set(Arc::clone(&tap));
    let mut hub = Hub::hello(&tap, &got, 1);
    assert!(matches!(hub.recv(), Up::Hello { .. }));
    wait_for(|| tap.is_linked());
    hub.send(&Down::Bye {
        reason: ByeReason::Shutdown,
    });
    wait_for(|| seen.lock().unwrap().is_some());
    assert_eq!(*seen.lock().unwrap(), Some(false));
    assert!(hub.closed());
}

#[test]
fn new_client_replaces_old_and_needs_its_own_cfg() {
    let (tap, got) = start();
    let mut old = Hub::linked(&tap, &got, true, false, false);
    tap.hook_key(key(KeyKind::Letter, 65));
    assert!(matches!(old.recv(), Up::Key { .. }));

    let mut new = Hub::hello(&tap, &got, 1);
    assert!(matches!(new.recv(), Up::Hello { .. }));
    assert!(old.closed());
    tap.hook_key(key(KeyKind::Letter, 66));
    new.expect_nothing_before_marker(&tap);
    new.cfg(true, false, false);
    new.expect_nothing_before_marker(&tap);
    tap.hook_key(key(KeyKind::Letter, 67));
    assert!(matches!(new.recv(), Up::Key { vk: Some(67), .. }));
}

#[test]
fn hub_disconnect_revokes_consent() {
    let (tap, got) = start();
    let hub = Hub::linked(&tap, &got, true, true, true);
    wait_for(|| tap.rewrite_enabled());
    drop(hub);
    wait_for(|| !tap.is_linked());
    assert!(!tap.rewrite_enabled());
    assert!(!tap.send_open(OpenTarget::Chat), "没有连接时不入队");
}

#[test]
fn queue_full_drops_and_heartbeat_reports_them() {
    let (tap, got) = start();
    let mut hub = Hub::linked(&tap, &got, true, false, false);
    hub.expect_nothing_before_marker(&tap);
    const N: u32 = 200_000;
    for _ in 0..N {
        tap.hook_key(key(KeyKind::Letter, 65));
    }
    let (mut keys, mut dropped) = (0u32, 0u32);
    hub.s
        .set_read_timeout(Some(Duration::from_secs(8)))
        .unwrap();
    while keys + dropped < N {
        match hub.try_recv_any() {
            Some(Up::Key { .. }) => keys += 1,
            Some(Up::Hb {
                dropped: d, queue, ..
            }) => {
                dropped += d;
                assert!(queue <= 4096);
            }
            Some(other) => panic!("{other:?}"),
            None => panic!("连接已关闭"),
        }
    }
    assert!(dropped > 0, "队列满时应丢弃并计数");
    assert_eq!(keys + dropped, N);
}

#[test]
fn disable_sends_bye_and_stops_accepting() {
    let (tap, got) = start();
    let mut hub = Hub::linked(&tap, &got, true, false, false);
    hub.expect_nothing_before_marker(&tap);
    tap.set_enabled(false);
    assert!(matches!(
        hub.recv(),
        Up::Bye {
            reason: ByeReason::Disabled,
            ..
        }
    ));
    assert!(hub.closed());
    assert!(!tap.send_open(OpenTarget::Chat));

    // 停用期间连得上 TCP 也不握手；重新启用后照常连接
    let mut late = Hub::hello(&tap, &got, 1);
    late.s
        .set_read_timeout(Some(Duration::from_millis(600)))
        .unwrap();
    let mut b = [0u8; 1];
    assert!(
        !matches!(late.s.read(&mut b), Ok(n) if n > 0),
        "停用时不应回复 hello"
    );
    tap.set_enabled(true);
    let mut hub = Hub::linked(&tap, &got, true, false, false);
    hub.expect_nothing_before_marker(&tap);
}

#[test]
fn recent_commits_feed_rewrite() {
    let (tap, got) = start();
    tap.hook_focus(focus("WeChat.exe", Scope::Normal));
    let mut hub = Hub::linked(&tap, &got, true, false, false);
    assert!(matches!(hub.recv(), Up::Focus { .. }));
    tap.hook_commit(commit("不记录"));
    assert!(matches!(hub.recv(), Up::Commit { .. }));
    assert_eq!(tap.take_recent(), None, "未同意改写时不记录");

    hub.cfg(true, false, true);
    hub.expect_nothing_before_marker(&tap);
    tap.hook_commit(commit("今天"));
    tap.hook_commit(commit("好累😀"));
    hub.recv();
    hub.recv();
    let recent = tap.take_recent().unwrap();
    assert_eq!(recent.text, "今天好累😀");
    assert_eq!(recent.utf16_len, 6);
    assert_eq!(tap.take_recent(), None, "取走即清空");

    tap.hook_commit(commit("一半"));
    hub.recv();
    tap.hook_key(key(KeyKind::Nav, 0x25));
    hub.recv();
    assert_eq!(tap.take_recent(), None, "移动光标后清空");

    assert!(tap.send_rewrite_req(RewriteReq {
        req_id: 7,
        source: RewriteSource::Recent,
        text: "今天好累".into(),
        style: RewriteStyle::Gentle,
        replace_len: Some(4),
    }));
    match hub.recv() {
        Up::RewriteReq {
            req_id,
            text,
            replace_len,
            ..
        } => {
            assert_eq!(
                (req_id, text.as_str(), replace_len),
                (7, "今天好累", Some(4))
            );
        }
        other => panic!("{other:?}"),
    }
    let result = Down::RewriteResult {
        req_id: 7,
        style: RewriteStyle::Gentle,
        cands: vec!["今天有点累".into()],
    };
    hub.send(&result);
    wait_for(|| got.lock().unwrap().contains(&result));

    tap.hook_focus(focus("chrome.exe", Scope::Password));
    hub.recv();
    tap.hook_commit(commit("secret"));
    assert_eq!(tap.take_recent(), None, "密码框里不记录");
    assert!(!tap.send_rewrite_req(RewriteReq {
        req_id: 8,
        source: RewriteSource::Clipboard,
        text: "x".into(),
        style: RewriteStyle::Polite,
        replace_len: None,
    }));
}

#[test]
fn shutdown_says_bye() {
    let (tap, got) = start();
    let mut hub = Hub::linked(&tap, &got, false, false, false);
    hub.expect_nothing_before_marker(&tap);
    tap.shutdown();
    assert!(matches!(
        hub.recv(),
        Up::Bye {
            reason: ByeReason::Shutdown,
            ..
        }
    ));
    assert!(hub.closed());
}
