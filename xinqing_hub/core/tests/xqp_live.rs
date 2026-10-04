//! XQP 客户端与实时感知的端到端测试：进程内的假核心按 10 第 2.3 节握手、回放合成脚本、断线重连。
//! 时间用 tokio 的暂停时钟，几分钟的脚本瞬间跑完，结果与离线回放（`pipeline::replay`）对拍。

use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{Local, TimeZone};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream, ReadHalf, WriteHalf};
use tokio::sync::mpsc;
use tokio::time::Instant;
use xinqing_hub_core::bus;
use xinqing_hub_core::domain::features::Baseline;
use xinqing_hub_core::domain::status::StatusSnapshot;
use xinqing_hub_core::infra::clock::ManualClock;
use xinqing_hub_core::infra::templates::{AppCategories, BaselineDefault, TemplateDirs};
use xinqing_hub_core::infra::xqp::{
    Conn, Connector, Disconnect, LinkEvent, LinkOptions, XqpHandle, XqpLink,
};
use xinqing_hub_core::pipeline::{StatePipeline, replay};
use xinqing_hub_core::sense::{Sense, SensePort, WindowRecord};
use xqp::{ByeReason, Down, KeyKind, KeySrc, MoodState, OpenTarget, Up};

/// 每次连接创建一对内存管道，核心那一端交给测试。
struct DuplexConnector {
    accept: mpsc::UnboundedSender<DuplexStream>,
    /// 接下来这么多次连接直接失败（模拟核心没运行）。
    refuse: Arc<AtomicU32>,
    attempts: Arc<Mutex<Vec<Instant>>>,
}

#[async_trait]
impl Connector for DuplexConnector {
    async fn connect(&self) -> io::Result<Conn> {
        self.attempts.lock().unwrap().push(Instant::now());
        if self
            .refuse
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
        {
            return Err(io::ErrorKind::NotFound.into());
        }
        let (hub, core) = tokio::io::duplex(64 * 1024);
        self.accept
            .send(core)
            .map_err(|_| io::Error::other("核心已退出"))?;
        let (r, w) = tokio::io::split(hub);
        Ok(Conn {
            reader: Box::new(r),
            writer: Box::new(w),
        })
    }
}

struct FakeCore {
    r: ReadHalf<DuplexStream>,
    w: WriteHalf<DuplexStream>,
}

async fn recv_down(r: &mut ReadHalf<DuplexStream>) -> Option<Down> {
    let mut prefix = [0u8; 4];
    r.read_exact(&mut prefix).await.ok()?;
    let mut body = vec![0u8; xqp::frame_len(prefix).unwrap()];
    r.read_exact(&mut body).await.ok()?;
    Some(xqp::decode_down(&body).unwrap())
}

async fn send_up(w: &mut WriteHalf<DuplexStream>, up: &Up) {
    w.write_all(&xqp::encode(up).unwrap()).await.unwrap();
}

impl FakeCore {
    fn new(s: DuplexStream) -> Self {
        let (r, w) = tokio::io::split(s);
        Self { r, w }
    }

    async fn recv(&mut self) -> Option<Down> {
        recv_down(&mut self.r).await
    }

    async fn send(&mut self, up: &Up) {
        send_up(&mut self.w, up).await;
    }

    async fn raw(&mut self, bytes: &[u8]) {
        self.w.write_all(bytes).await.unwrap();
    }

    /// 按 10 第 2.3 节完成握手，返回 Hub 握手后补发的全部下行消息。
    async fn handshake(&mut self, session: &str) -> Vec<Down> {
        assert!(matches!(self.recv().await, Some(Down::Hello { v: 1, .. })));
        self.send(&hello(1, session)).await;
        let mut out = vec![self.recv().await.unwrap()];
        // 补发的状态类消息紧跟 cfg，一次读完（暂停时钟下不会真的等）
        while let Ok(Some(d)) = tokio::time::timeout(Duration::from_millis(10), self.recv()).await {
            out.push(d);
        }
        out
    }
}

fn hello(v: u32, session: &str) -> Up {
    Up::Hello {
        v,
        ime_ver: "fake-core".into(),
        session: session.into(),
        caps: vec!["core_keys".into()],
    }
}

fn cfg(collect: bool) -> Down {
    Down::Cfg {
        collect,
        send_text: false,
        rewrite: false,
        app_blocklist: None,
        app_allowlist: None,
    }
}

fn key(ts: u64) -> Up {
    Up::Key {
        ts,
        seq: None,
        kind: KeyKind::Letter,
        vk: Some(b'A'),
        in_comp: false,
        src: KeySrc::Core,
        eaten: None,
        del_committed: None,
    }
}

struct Rig {
    handle: XqpHandle,
    events: mpsc::Receiver<LinkEvent>,
    cores: mpsc::UnboundedReceiver<DuplexStream>,
    refuse: Arc<AtomicU32>,
    attempts: Arc<Mutex<Vec<Instant>>>,
}

fn start(initial: Down) -> Rig {
    let (accept, cores) = mpsc::unbounded_channel();
    let refuse = Arc::new(AtomicU32::new(0));
    let attempts = Arc::new(Mutex::new(Vec::new()));
    let connector = DuplexConnector {
        accept,
        refuse: refuse.clone(),
        attempts: attempts.clone(),
    };
    let (link, handle, events) = XqpLink::new(Box::new(connector), initial, LinkOptions::default());
    tokio::spawn(link.run());
    Rig {
        handle,
        events,
        cores,
        refuse,
        attempts,
    }
}

impl Rig {
    async fn core(&mut self) -> FakeCore {
        FakeCore::new(self.cores.recv().await.unwrap())
    }
    async fn event(&mut self) -> LinkEvent {
        self.events.recv().await.unwrap()
    }
}

#[tokio::test(start_paused = true)]
async fn handshake_sends_cfg_then_remembered_state() {
    let mut rig = start(cfg(true));
    // 连接前排队的状态类消息在握手后补发；气泡这种一次性消息直接丢弃
    rig.handle.send(Down::Pause { on: true });
    rig.handle.send(Down::Tip {
        text: "识别到日程".into(),
        ms: 1500,
    });
    let mut core = rig.core().await;
    let got = core.handshake("s1").await;
    assert_eq!(got, vec![cfg(true), Down::Pause { on: true }]);
    match rig.event().await {
        LinkEvent::Connected(p) => assert_eq!(p.session, "s1"),
        other => panic!("{other:?}"),
    }
    core.send(&key(10)).await;
    assert_eq!(rig.event().await, LinkEvent::Up(key(10)));

    // 已连接时下发的消息立即送达；同意变化后的 cfg 也一样
    rig.handle.send(cfg(false));
    assert_eq!(core.recv().await, Some(cfg(false)));
}

#[tokio::test(start_paused = true)]
async fn core_never_sends_events_before_hello() {
    let mut rig = start(cfg(true));
    let mut core = rig.core().await;
    assert!(matches!(core.recv().await, Some(Down::Hello { .. })));
    // 核心第一条不是 hello：按非法帧处理，断开后 2 秒重连
    core.send(&key(1)).await;
    let mut core = rig.core().await;
    core.handshake("s1").await;
    assert!(matches!(rig.event().await, LinkEvent::Connected(_)));
    let at = rig.attempts.lock().unwrap().clone();
    assert_eq!(at.len(), 2);
    assert!(at[1] - at[0] >= Duration::from_secs(2));
}

#[tokio::test(start_paused = true)]
async fn bad_frame_disconnects_and_reconnects_with_state_replayed() {
    let mut rig = start(cfg(true));
    let mut core = rig.core().await;
    core.handshake("s1").await;
    assert!(matches!(rig.event().await, LinkEvent::Connected(_)));
    rig.handle.send(Down::Mood {
        state: MoodState::Tired,
        offline: true,
    });
    assert!(matches!(core.recv().await, Some(Down::Mood { .. })));

    // 长度超过 64 KiB 的帧
    core.raw(&((xqp::MAX_FRAME as u32 + 1).to_be_bytes())).await;
    assert!(matches!(
        rig.event().await,
        LinkEvent::Disconnected(Disconnect::BadFrame(_))
    ));
    let mut core = rig.core().await;
    let got = core.handshake("s1").await;
    assert_eq!(
        got,
        vec![
            cfg(true),
            Down::Mood {
                state: MoodState::Tired,
                offline: true
            }
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn silent_core_is_dropped_after_idle_timeout() {
    let mut rig = start(cfg(true));
    let mut core = rig.core().await;
    core.handshake("s1").await;
    let t0 = Instant::now();
    assert!(matches!(rig.event().await, LinkEvent::Connected(_)));
    // 心跳正常时不断开
    for i in 1..=4 {
        tokio::time::sleep(Duration::from_secs(5)).await;
        core.send(&Up::Hb {
            ts: i * 5_000,
            seq: None,
            dropped: 0,
            queue: 0,
        })
        .await;
        assert!(matches!(rig.event().await, LinkEvent::Up(Up::Hb { .. })));
    }
    assert_eq!(rig.event().await, LinkEvent::Disconnected(Disconnect::Idle));
    assert!(Instant::now() - t0 >= Duration::from_secs(35));
}

#[tokio::test(start_paused = true)]
async fn core_without_heartbeats_is_not_dropped() {
    // 核心在收到 cfg{collect:true} 之前可能什么都不发：没见过心跳就不按空闲断开
    let mut rig = start(cfg(false));
    let mut core = rig.core().await;
    core.handshake("s1").await;
    assert!(matches!(rig.event().await, LinkEvent::Connected(_)));
    tokio::time::sleep(Duration::from_secs(120)).await;
    core.send(&key(120_000)).await;
    assert_eq!(rig.event().await, LinkEvent::Up(key(120_000)));
}

#[tokio::test(start_paused = true)]
async fn version_mismatch_retries_slowly_and_core_bye_is_reported() {
    let mut rig = start(cfg(true));
    let mut core = rig.core().await;
    assert!(matches!(core.recv().await, Some(Down::Hello { .. })));
    core.send(&hello(2, "s1")).await;
    let mut core = rig.core().await;
    {
        let at = rig.attempts.lock().unwrap();
        assert!(
            at[1] - at[0] >= Duration::from_secs(30),
            "主版本不一致时放慢重试"
        );
    }
    core.handshake("s2").await;
    assert!(matches!(rig.event().await, LinkEvent::Connected(_)));
    core.send(&Up::Bye {
        ts: Some(1),
        seq: None,
        reason: ByeReason::Disabled,
    })
    .await;
    assert_eq!(
        rig.event().await,
        LinkEvent::Disconnected(Disconnect::Bye(ByeReason::Disabled))
    );
}

#[tokio::test(start_paused = true)]
async fn core_not_running_is_retried_every_two_seconds_silently() {
    let mut rig = start(cfg(false));
    rig.refuse.store(3, Ordering::SeqCst);
    let mut core = rig.core().await;
    core.handshake("s1").await;
    // 连接失败不报告事件，第一条就是 Connected
    assert!(matches!(rig.event().await, LinkEvent::Connected(_)));
    let at = rig.attempts.lock().unwrap().clone();
    assert_eq!(at.len(), 4);
    for w in at.windows(2) {
        assert!(w[1] - w[0] >= Duration::from_secs(2));
    }
}

#[tokio::test(start_paused = true)]
async fn dropping_every_handle_says_bye() {
    let mut rig = start(cfg(true));
    let mut core = rig.core().await;
    core.handshake("s1").await;
    assert!(matches!(rig.event().await, LinkEvent::Connected(_)));
    drop(rig.handle);
    assert_eq!(
        core.recv().await,
        Some(Down::Bye {
            reason: ByeReason::Shutdown
        })
    );
}

// ---- 实时感知：假核心回放合成脚本，结果与离线回放一致 ----

#[derive(Default)]
struct Port {
    status: Mutex<StatusSnapshot>,
    changes: Mutex<Vec<MoodState>>,
    windows: Mutex<usize>,
    opened: Mutex<Vec<OpenTarget>>,
}

impl SensePort for Port {
    fn update_status(&self, f: &mut dyn FnMut(&mut StatusSnapshot) -> bool) {
        f(&mut self.status.lock().unwrap());
    }
    fn open(&self, target: OpenTarget) {
        self.opened.lock().unwrap().push(target);
    }
    fn save_window(&self, rec: &WindowRecord<'_>) {
        *self.windows.lock().unwrap() += 1;
        if rec.fusion.changed {
            self.changes.lock().unwrap().push(rec.fusion.shown);
        }
    }
    fn note(&self, _msg: &str) {}
}

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn pipeline() -> StatePipeline {
    let dirs = TemplateDirs::factory_only(repo().join("hub_templates"));
    let base = Baseline::from_defaults(&BaselineDefault::load(&dirs).unwrap());
    let start = Local.with_ymd_and_hms(2026, 10, 5, 14, 0, 0).unwrap();
    StatePipeline::new(base, AppCategories::load(&dirs).unwrap(), start)
}

fn script(name: &str) -> Vec<Up> {
    std::fs::read_to_string(repo().join(format!("tools/xq-sim/scripts/{name}.jsonl")))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

/// 与 xq-sim 相同的节奏：按 ts 等待，每 5 秒插一条心跳。
async fn play(w: &mut WriteHalf<DuplexStream>, msgs: &[Up]) {
    let mut last = 0u64;
    let mut next_hb = 5_000u64;
    for m in msgs.iter().filter(|m| !matches!(m, Up::Hello { .. })) {
        let ts = m.ts().unwrap_or(last);
        while ts >= next_hb {
            tokio::time::sleep(Duration::from_millis(next_hb - last)).await;
            last = next_hb;
            let hb = Up::Hb {
                ts: next_hb,
                seq: None,
                dropped: 0,
                queue: 0,
            };
            send_up(w, &hb).await;
            next_hb += 5_000;
        }
        tokio::time::sleep(Duration::from_millis(ts - last)).await;
        last = ts;
        send_up(w, m).await;
    }
}

/// 假核心按真实节奏回放脚本，Hub 实时识别；显示状态的变化序列、窗口数都应与离线回放一致。
/// 返回状态变化序列。
async fn live_matches_offline(name: &str) -> Vec<MoodState> {
    let msgs = script(name);
    let offline = replay(&mut pipeline(), &msgs, |_| None);
    let expected: Vec<MoodState> = offline.changes().into_iter().map(|(_, s)| s).collect();

    let mut rig = start(cfg(true));
    let port = Arc::new(Port::default());
    let (bus_tx, _bus_rx) = bus::channel();
    let clock = Arc::new(ManualClock::new(Local::now()));
    let sense = Sense::new(pipeline(), port.clone(), rig.handle.clone(), bus_tx, clock);
    let (cmd_tx, cmd_rx) = mpsc::channel(4);
    let events = std::mem::replace(&mut rig.events, mpsc::channel(1).1);
    let sensing = tokio::spawn(sense.run(events, cmd_rx));

    let mut core = rig.core().await;
    core.handshake("live").await;
    let FakeCore { mut r, mut w } = core;
    // 一边回放一边收集 Hub 下发的天气
    let moods = tokio::spawn(async move {
        let mut got = Vec::new();
        while let Some(d) = recv_down(&mut r).await {
            if let Down::Mood { state, .. } = d {
                got.push(state);
            }
        }
        got
    });
    play(&mut w, &msgs).await;
    let open = Up::Open {
        ts: 400_000,
        seq: None,
        target: OpenTarget::Dashboard,
    };
    send_up(&mut w, &open).await;
    // 让最后一个窗口按停顿结束
    tokio::time::sleep(Duration::from_secs(10)).await;

    assert!(port.status.lock().unwrap().connected);
    assert_eq!(*port.changes.lock().unwrap(), expected);
    assert_eq!(*port.windows.lock().unwrap(), offline.windows.len());
    assert_eq!(*port.opened.lock().unwrap(), vec![OpenTarget::Dashboard]);

    // Hub 退出：感知任务结束，客户端向核心告别
    drop(cmd_tx);
    sensing.await.unwrap();
    drop(rig.handle);
    let sent = moods.await.unwrap();
    assert_eq!(sent, expected, "每次显示状态变化都下发给核心");
    expected
}

#[tokio::test(start_paused = true)]
async fn live_hesitant_script_switches_like_offline_replay() {
    let changes = live_matches_offline("hesitant").await;
    assert!(changes.contains(&MoodState::Hesitant), "{changes:?}");
}

#[tokio::test(start_paused = true)]
async fn live_fluent_script_never_switches() {
    assert!(live_matches_offline("fluent").await.is_empty());
}

#[tokio::test(start_paused = true)]
async fn live_typo_burst_script_matches_offline_replay() {
    live_matches_offline("typo_burst").await;
}
