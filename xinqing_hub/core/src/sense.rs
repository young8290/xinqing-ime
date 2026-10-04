//! 实时感知（03 第 2.2 节 `feature_pipeline` 任务）：把 XQP 客户端收到的事件喂给状态识别流水线，
//! 更新显示状态、写库、发布总线事件，并把显示状态下发给输入法（工具栏天气按钮，FR-ENT-02）。
//!
//! 与外壳之间只通过 [`SensePort`]（状态快照、打开窗口、写库、日志），本模块不依赖 Tauri（ADR 0007）。
//!
//! 时间：流水线用核心的会话时间（毫秒）。收到新会话的第一条带 `ts` 的事件时，用“现在 − ts”
//! 定出会话起点对应的本地时间；之后每 250 ms 的 tick 用“最近一条事件的 ts + 收到它以来经过的时间”
//! 推算当前会话时间（17 第 2.3 节）。
//!
//! Jev 判断（FR-STA-05）随 AI 网关接入外壳后加入；在此之前按 FR-STA-08 只用本地规则。

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{broadcast, mpsc};
use tokio::time::{Instant, MissedTickBehavior};
use xqp::{Down, MoodState, OpenTarget, RewriteFailReason, Scope, Up};

use crate::bus::{HubEvent, MoodEvent};
use crate::domain::explain::{self, Evidence, ExplainSource, Explanation};
use crate::domain::features::WindowFeatures;
use crate::domain::fusion::{FusionOut, Source};
use crate::domain::rules::Hints;
use crate::domain::status::StatusSnapshot;
use crate::infra::clock::Clock;
use crate::infra::xqp::{LinkEvent, XqpHandle};
use crate::pipeline::{PipelineOut, StatePipeline, WindowOut};

/// tick 间隔（17 第 2.3 节）。
pub const TICK: Duration = Duration::from_millis(250);

/// 外壳提供给感知任务的能力。
pub trait SensePort: Send + Sync {
    /// 修改状态快照；`f` 返回是否有变化，有变化时由外壳推送 `status:changed`。
    fn update_status(&self, f: &mut dyn FnMut(&mut StatusSnapshot) -> bool);
    /// 核心请求打开 Hub 的某个窗口（语言栏菜单、工具栏按钮，FR-ENT-01/02）。
    fn open(&self, target: OpenTarget);
    /// 保存一个特征窗口及其融合结果（D-06、D-07）。
    fn save_window(&self, rec: &WindowRecord<'_>);
    /// 运行日志。只会传入连接状态、计数这类不含用户数据的内容（NFR-LOG）。
    fn note(&self, msg: &str);
    /// 显示状态切换了，附上本次的状态解释（FR-STA-09），外壳缓存到下一次切换，供 `state_explain` 返回。
    fn explained(&self, _e: &Explanation) {}
}

/// 一个已结束的窗口，时间已换算成 Unix 毫秒。
#[derive(Debug)]
pub struct WindowRecord<'a> {
    pub start_ms: i64,
    pub end_ms: i64,
    pub window: &'a WindowOut,
    pub fusion: &'a FusionOut,
}

/// 外壳发给感知任务的命令。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SenseCmd {
    /// 用户在 Hub 里暂停 / 恢复感知（FR-WGT-06 右键菜单），同时经 XQP 下发给核心。
    Pause(bool),
    /// 用户对某个显示状态点了“不准”（FR-STA-07），`ts` 为 Unix 毫秒；已由外壳写入 `feedback` 表。
    Unfit { state: MoodState, ts: i64 },
}

pub struct Sense {
    pipeline: StatePipeline,
    port: Arc<dyn SensePort>,
    xqp: XqpHandle,
    bus: broadcast::Sender<HubEvent>,
    clock: Arc<dyn Clock>,
    /// 当前核心会话（握手 `hello.session`）。
    session: Option<String>,
    /// 新会话还没收到带 `ts` 的事件，会话起点未定。
    needs_base: bool,
    /// 最近一条带 `ts` 的事件及收到它的时刻，用来推算当前会话时间。
    anchor: Option<(u64, Instant)>,
    /// 用户开启了无痕（Hub 右键菜单或输入法菜单、快捷键）。
    user_paused: bool,
    /// 当前输入框是密码框、禁用输入或黑名单应用（FR-SEN-05），小组件显示“闭眼”。
    gate_closed: bool,
    /// 上一个窗口的特征和规则提示，状态解释要看“触发窗口和上一个窗口”（FR-STA-09）。
    prev_window: Option<(WindowFeatures, Hints)>,
    /// 最近一次切换的状态解释，缓存到下一次切换。
    explanation: Option<Explanation>,
}

impl Sense {
    pub fn new(
        pipeline: StatePipeline,
        port: Arc<dyn SensePort>,
        xqp: XqpHandle,
        bus: broadcast::Sender<HubEvent>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            pipeline,
            port,
            xqp,
            bus,
            clock,
            session: None,
            needs_base: true,
            anchor: None,
            user_paused: false,
            gate_closed: false,
            prev_window: None,
            explanation: None,
        }
    }

    /// 最近一次状态切换的解释；还没切换过时为 `None`。
    pub fn explanation(&self) -> Option<&Explanation> {
        self.explanation.as_ref()
    }

    /// 运行到 `cmds` 的发送端全部丢弃为止。`link` 关闭（没有连接核心）后只处理命令。
    pub async fn run(
        mut self,
        mut link: mpsc::Receiver<LinkEvent>,
        mut cmds: mpsc::Receiver<SenseCmd>,
    ) {
        let mut tick = tokio::time::interval(TICK);
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut link_open = true;
        loop {
            tokio::select! {
                ev = link.recv(), if link_open => match ev {
                    Some(ev) => self.on_link(ev, Instant::now()),
                    None => link_open = false,
                },
                cmd = cmds.recv() => match cmd {
                    Some(c) => self.on_cmd(c),
                    None => return,
                },
                now = tick.tick() => self.on_tick(now),
            }
        }
    }

    pub fn on_link(&mut self, ev: LinkEvent, now: Instant) {
        match ev {
            LinkEvent::Connected(peer) => {
                if self.session.as_deref() == Some(peer.session.as_str()) {
                    // 同一会话重连：断开期间核心没发事件，进行中的窗口不完整
                    self.pipeline.discard();
                } else {
                    self.session = Some(peer.session.clone());
                    self.needs_base = true;
                    self.anchor = None;
                    self.prev_window = None;
                    self.pipeline.reset_session(self.clock.now());
                }
                self.pipeline.push(&peer.to_hello());
                self.gate_closed = false;
                self.port
                    .note(&format!("XQP 已连接（核心 {}）", peer.ime_ver));
                let paused = self.paused();
                self.port.update_status(&mut |s| {
                    StatusSnapshot::set(&mut s.connected, true)
                        | StatusSnapshot::set(&mut s.paused, paused)
                });
            }
            LinkEvent::Disconnected(d) => {
                self.pipeline.discard();
                self.gate_closed = false;
                self.port.note(&format!("XQP 已断开（{}）", d.label()));
                let paused = self.paused();
                self.port.update_status(&mut |s| {
                    StatusSnapshot::set(&mut s.connected, false)
                        | StatusSnapshot::set(&mut s.paused, paused)
                });
            }
            LinkEvent::Up(up) => self.on_up(up, now),
        }
    }

    pub fn on_cmd(&mut self, cmd: SenseCmd) {
        match cmd {
            SenseCmd::Pause(on) => {
                self.apply_pause(on);
                self.xqp.send(Down::Pause { on });
            }
            SenseCmd::Unfit { state, ts } => self.pipeline.fusion_mut().record_unfit(state, ts),
        }
    }

    pub fn on_tick(&mut self, now: Instant) {
        if self.user_paused {
            return;
        }
        let Some((ts, at)) = self.anchor else {
            return;
        };
        let now_ts = ts + now.saturating_duration_since(at).as_millis() as u64;
        let outs = self.pipeline.tick(now_ts);
        self.handle(outs);
    }

    fn paused(&self) -> bool {
        self.user_paused || self.gate_closed
    }

    fn refresh_paused(&self) {
        let paused = self.paused();
        self.port
            .update_status(&mut |s| StatusSnapshot::set(&mut s.paused, paused));
    }

    fn apply_pause(&mut self, on: bool) {
        self.user_paused = on;
        if on {
            self.pipeline.discard();
        }
        let _ = self.bus.send(HubEvent::Pause(on));
        self.refresh_paused();
    }

    fn on_up(&mut self, up: Up, now: Instant) {
        if let Some(ts) = up.ts() {
            if self.needs_base {
                let base = self.clock.now() - chrono::Duration::milliseconds(ts as i64);
                self.pipeline.set_base_local(base);
                self.needs_base = false;
            }
            self.anchor = Some((ts, now));
        }
        match &up {
            Up::Focus { scope, blocked, .. } => {
                self.gate_closed = *blocked || *scope != Scope::Normal;
                self.refresh_paused();
            }
            Up::PauseChanged { on, .. } => {
                self.apply_pause(*on);
                // 核心已经切换了，回发一次只是让重连后的补发状态与它一致
                self.xqp.send(Down::Pause { on: *on });
            }
            Up::Open { target, .. } => self.port.open(*target),
            Up::Hb { dropped, .. } if *dropped > 0 => {
                self.port
                    .note(&format!("核心队列满，丢弃了 {dropped} 条事件（FR-SEN-07）"));
            }
            Up::RewriteReq { req_id, .. } => {
                // 改写服务（C-09）接入前直接告诉核心不可用，避免候选框一直等待
                self.xqp.send(Down::RewriteFail {
                    req_id: *req_id,
                    reason: RewriteFailReason::Offline,
                });
            }
            _ => {}
        }
        let up = Arc::new(up);
        let _ = self.bus.send(HubEvent::Xqp(up.clone()));
        if self.user_paused {
            // 无痕期间核心不发采集事件；万一收到也不计算（FR-SEN-06）
            return;
        }
        let outs = self.pipeline.push(&up);
        self.handle(outs);
    }

    fn handle(&mut self, outs: Vec<PipelineOut>) {
        for o in outs {
            match o {
                PipelineOut::Typo { ts } => {
                    let ts = self.pipeline.unix_ms(ts);
                    let _ = self.bus.send(HubEvent::Typo { ts });
                }
                PipelineOut::Window(w) => self.on_window(*w),
            }
        }
    }

    fn on_window(&mut self, w: WindowOut) {
        let fusion = self.pipeline.fuse(&w, None);
        let rec = WindowRecord {
            start_ms: self.pipeline.unix_ms(w.start_ts),
            end_ms: self.pipeline.unix_ms(w.end_ts),
            window: &w,
            fusion: &fusion,
        };
        self.port.save_window(&rec);
        let ts = rec.end_ms;
        let _ = self.bus.send(HubEvent::Window(Arc::new((
            w.features.clone(),
            w.hints.clone(),
        ))));
        let _ = self.bus.send(HubEvent::Mood(MoodEvent::Sample {
            ts,
            out: fusion.clone(),
        }));
        let progress = self.pipeline.baseline().progress_pct();
        let mut offline = true;
        self.port.update_status(&mut |s| {
            offline = s.offline;
            s.apply_mood(fusion.shown, None)
                | StatusSnapshot::set(&mut s.baseline_progress, progress)
        });
        if fusion.changed {
            // 可能性随 Jev 接入后给出；只有本地规则时界面不显示百分比
            let source = match fusion.source {
                Source::Jev => ExplainSource::Jev,
                Source::Rule => ExplainSource::Rule,
            };
            let e = explain::build(
                fusion.shown,
                None,
                source,
                Evidence {
                    features: &w.features,
                    hints: &w.hints,
                },
                self.prev_window
                    .as_ref()
                    .map(|(features, hints)| Evidence { features, hints }),
                self.pipeline.baseline(),
            );
            self.port.explained(&e);
            self.explanation = Some(e);
            let _ = self.bus.send(HubEvent::Mood(MoodEvent::StateChanged {
                ts,
                state: fusion.shown,
            }));
            self.xqp.send(Down::Mood {
                state: fusion.shown,
                offline,
            });
        }
        self.prev_window = Some((w.features, w.hints));
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Mutex;

    use chrono::{Local, TimeZone};
    use xqp::{ByeReason, KeyKind, KeySrc};

    use super::*;
    use crate::domain::features::Baseline;
    use crate::infra::clock::ManualClock;
    use crate::infra::templates::{AppCategories, BaselineDefault, TemplateDirs};
    use crate::infra::xqp::{Disconnect, PeerInfo};

    #[derive(Default)]
    struct FakePort {
        status: Mutex<StatusSnapshot>,
        pushes: Mutex<u32>,
        opened: Mutex<Vec<OpenTarget>>,
        saved: Mutex<Vec<(i64, i64)>>,
        notes: Mutex<Vec<String>>,
        explained: Mutex<Vec<Explanation>>,
    }

    impl SensePort for FakePort {
        fn update_status(&self, f: &mut dyn FnMut(&mut StatusSnapshot) -> bool) {
            if f(&mut self.status.lock().unwrap()) {
                *self.pushes.lock().unwrap() += 1;
            }
        }
        fn open(&self, target: OpenTarget) {
            self.opened.lock().unwrap().push(target);
        }
        fn save_window(&self, rec: &WindowRecord<'_>) {
            self.saved.lock().unwrap().push((rec.start_ms, rec.end_ms));
        }
        fn note(&self, msg: &str) {
            self.notes.lock().unwrap().push(msg.to_string());
        }
        fn explained(&self, e: &Explanation) {
            self.explained.lock().unwrap().push(e.clone());
        }
    }

    struct Rig {
        sense: Sense,
        port: Arc<FakePort>,
        down: mpsc::Receiver<Down>,
        bus: broadcast::Receiver<HubEvent>,
        t0: Instant,
    }

    fn rig() -> Rig {
        let dirs = TemplateDirs::factory_only(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../hub_templates"),
        );
        let base = Baseline::from_defaults(&BaselineDefault::load(&dirs).unwrap());
        let start = Local.with_ymd_and_hms(2026, 10, 5, 14, 0, 0).unwrap();
        let pipeline = StatePipeline::new(base, AppCategories::load(&dirs).unwrap(), start);
        let port = Arc::new(FakePort::default());
        let (xqp, down) = XqpHandle::pair();
        let (bus_tx, bus) = crate::bus::channel();
        let clock = Arc::new(ManualClock::new(start));
        let sense = Sense::new(pipeline, port.clone(), xqp, bus_tx, clock);
        Rig {
            sense,
            port,
            down,
            bus,
            t0: Instant::now(),
        }
    }

    fn connected(session: &str) -> LinkEvent {
        LinkEvent::Connected(PeerInfo {
            ime_ver: "test".into(),
            session: session.into(),
            caps: vec!["core_keys".into()],
        })
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

    fn focus(ts: u64, scope: Scope) -> Up {
        Up::Focus {
            ts,
            seq: None,
            app: None,
            scope,
            blocked: false,
        }
    }

    fn downs(rx: &mut mpsc::Receiver<Down>) -> Vec<Down> {
        std::iter::from_fn(|| rx.try_recv().ok()).collect()
    }

    impl Rig {
        fn up(&mut self, ev: Up) {
            let at = self.t0 + Duration::from_millis(ev.ts().unwrap_or(0));
            self.sense.on_link(LinkEvent::Up(ev), at);
        }
        fn status(&self) -> StatusSnapshot {
            self.port.status.lock().unwrap().clone()
        }
    }

    #[test]
    fn connection_state_reaches_the_status() {
        let mut r = rig();
        r.sense.on_link(connected("s1"), r.t0);
        assert!(r.status().connected);
        r.sense
            .on_link(LinkEvent::Disconnected(Disconnect::Idle), r.t0);
        assert!(!r.status().connected);
        let notes = r.port.notes.lock().unwrap();
        assert!(notes.iter().any(|n| n.contains("idle")));
    }

    #[test]
    fn typing_produces_windows_with_unix_times() {
        let mut r = rig();
        r.sense.on_link(connected("s1"), r.t0);
        for i in 0..12 {
            r.up(key(1_000 + i * 150));
        }
        // 停顿超过 2 秒后由 tick 结束窗口
        let last = 1_000 + 11 * 150;
        r.sense.on_tick(r.t0 + Duration::from_millis(last + 2_100));
        let saved = r.port.saved.lock().unwrap().clone();
        assert_eq!(saved.len(), 1);
        // 会话起点 = 收到第一条事件时的“现在” − 它的 ts
        let base = Local
            .with_ymd_and_hms(2026, 10, 5, 14, 0, 0)
            .unwrap()
            .timestamp_millis()
            - 1_000;
        assert_eq!(saved[0], (base + 1_000, base + last as i64));
        let mut kinds = Vec::new();
        while let Ok(ev) = r.bus.try_recv() {
            kinds.push(match ev {
                HubEvent::Xqp(_) => "xqp",
                HubEvent::Window(_) => "window",
                HubEvent::Mood(MoodEvent::Sample { .. }) => "sample",
                _ => "other",
            });
        }
        assert!(kinds.contains(&"window") && kinds.contains(&"sample"));
    }

    #[test]
    fn state_change_comes_with_explanation() {
        // 回放合成的犹豫脚本（只有本地规则），每次切换都带解释（FR-STA-09、KPI-10）
        let mut r = rig();
        r.sense.on_link(connected("s1"), r.t0);
        let text = std::fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../tools/xq-sim/scripts/hesitant.jsonl"),
        )
        .unwrap();
        let mut changes = Vec::new();
        // 边回放边读总线，脚本事件多于总线容量
        let mut drain = |bus: &mut broadcast::Receiver<HubEvent>| {
            while let Ok(ev) = bus.try_recv() {
                if let HubEvent::Mood(MoodEvent::StateChanged { state, .. }) = ev {
                    changes.push(state);
                }
            }
        };
        let mut last = 0;
        for line in text.lines() {
            let ev: Up = serde_json::from_str(line).unwrap();
            if let Some(ts) = ev.ts() {
                for t in (last..ts).step_by(250).skip(1) {
                    r.sense.on_tick(r.t0 + Duration::from_millis(t));
                }
                last = ts;
            }
            r.up(ev);
            drain(&mut r.bus);
        }
        r.sense.on_tick(r.t0 + Duration::from_millis(last + 5_000));
        drain(&mut r.bus);

        let explained = r.port.explained.lock().unwrap().clone();
        assert!(changes.contains(&MoodState::Hesitant), "{changes:?}");
        assert_eq!(
            explained.iter().map(|e| e.state).collect::<Vec<_>>(),
            changes
        );
        let hesitant = explained
            .iter()
            .find(|e| e.state == MoodState::Hesitant)
            .unwrap();
        assert!(!hesitant.signals.is_empty());
        assert_eq!(hesitant.source, ExplainSource::Rule);
        assert_eq!(hesitant.prob, None);
        assert_eq!(r.sense.explanation(), explained.last());
    }

    #[test]
    fn unfit_feedback_reaches_fusion() {
        // FR-STA-07：外壳写库后把“不准”交给感知任务，30 分钟内 3 次 → 阈值 +0.05
        let mut r = rig();
        for t in [0, 60_000, 120_000] {
            r.sense.on_cmd(SenseCmd::Unfit {
                state: MoodState::Low,
                ts: t,
            });
        }
        let f = r.sense.pipeline.fusion_mut();
        assert!((f.bump(MoodState::Low, 130_000) - 0.05).abs() < 1e-9);
        assert_eq!(f.bump(MoodState::Hesitant, 130_000), 0.0);
    }

    #[test]
    fn pause_from_hub_discards_window_and_is_sent_to_core() {
        let mut r = rig();
        r.sense.on_link(connected("s1"), r.t0);
        for i in 0..8 {
            r.up(key(100 + i * 150));
        }
        r.sense.on_cmd(SenseCmd::Pause(true));
        assert!(r.status().paused);
        assert_eq!(downs(&mut r.down), vec![Down::Pause { on: true }]);
        r.sense.on_tick(r.t0 + Duration::from_secs(10));
        assert!(r.port.saved.lock().unwrap().is_empty(), "无痕后窗口作废");
        r.sense.on_cmd(SenseCmd::Pause(false));
        assert!(!r.status().paused);
    }

    #[test]
    fn pause_from_core_updates_status_and_sticky_state() {
        let mut r = rig();
        r.sense.on_link(connected("s1"), r.t0);
        r.up(Up::PauseChanged {
            ts: 50,
            seq: None,
            on: true,
            by: xqp::PauseBy::Hotkey,
        });
        assert!(r.status().paused);
        assert_eq!(downs(&mut r.down), vec![Down::Pause { on: true }]);
    }

    #[test]
    fn password_field_shows_paused_until_focus_moves() {
        let mut r = rig();
        r.sense.on_link(connected("s1"), r.t0);
        r.up(focus(10, Scope::Password));
        assert!(r.status().paused);
        r.up(focus(20, Scope::Normal));
        assert!(!r.status().paused);
        r.up(Up::Focus {
            ts: 30,
            seq: None,
            app: None,
            scope: Scope::Normal,
            blocked: true,
        });
        assert!(r.status().paused);
        r.sense.on_link(
            LinkEvent::Disconnected(Disconnect::Bye(ByeReason::Shutdown)),
            r.t0,
        );
        assert!(!r.status().paused, "断开后不再显示闭眼");
    }

    #[test]
    fn open_requests_and_rewrite_fallback() {
        let mut r = rig();
        r.sense.on_link(connected("s1"), r.t0);
        r.up(Up::Open {
            ts: 5,
            seq: None,
            target: OpenTarget::Chat,
        });
        assert_eq!(*r.port.opened.lock().unwrap(), vec![OpenTarget::Chat]);
        r.up(Up::RewriteReq {
            ts: 6,
            seq: None,
            req_id: 7,
            source: xqp::RewriteSource::Recent,
            text: "原文".into(),
            style: xqp::RewriteStyle::Gentle,
            replace_len: Some(2),
        });
        assert_eq!(
            downs(&mut r.down),
            vec![Down::RewriteFail {
                req_id: 7,
                reason: RewriteFailReason::Offline
            }]
        );
    }

    #[test]
    fn new_session_restarts_session_time() {
        let mut r = rig();
        r.sense.on_link(connected("s1"), r.t0);
        for i in 0..6 {
            r.up(key(500_000 + i * 150));
        }
        // 核心重启：会话时间从 0 开始，进行中的窗口作废，不会拿新旧时间拼成一个窗口
        r.sense.on_link(
            LinkEvent::Disconnected(Disconnect::Closed),
            r.t0 + Duration::from_secs(1),
        );
        r.sense
            .on_link(connected("s2"), r.t0 + Duration::from_secs(2));
        for i in 0..6 {
            r.up(key(100 + i * 150));
        }
        r.sense
            .on_tick(r.t0 + Duration::from_millis(100 + 5 * 150 + 2_100));
        let saved = r.port.saved.lock().unwrap().clone();
        assert_eq!(saved.len(), 1);
        assert!(saved[0].1 - saved[0].0 < 1_000);
    }
}
