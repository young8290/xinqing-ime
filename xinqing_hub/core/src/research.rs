//! 研究模式服务（07 FR-DMO-04，B-10，ADR 0026）：订阅总线看用户是不是在打字，每 10 秒问一次 [`Esm`] 该不该弹自评邀请。
//!
//! 邀请的进度 [`Esm`] 由外壳和服务共用：外壳在 `self_report_set` 里问它这次自评是不是邀请的回答（`source = esm`），
//! 用户点“跳过”时也经它清掉待答状态。与外壳之间只通过 [`ResearchPort`]，本模块不依赖 Tauri（ADR 0007）。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::broadcast;
use tokio::time::MissedTickBehavior;
use xqp::{CompOp, Up};

use crate::bus::HubEvent;
use crate::domain::research::{self, Esm, QUIET_MS};
use crate::infra::clock::Clock;

/// 判断间隔。
pub const TICK: Duration = Duration::from_secs(10);

/// 外壳提供给研究模式服务的能力。
pub trait ResearchPort: Send + Sync {
    /// 设置 `research.id`、`research.enabled`（ADR 0025）。
    fn config(&self) -> (String, bool);
    /// 显示自评邀请卡片（复用“主动报告心情”面板，可跳过）。
    fn invite(&self);
    /// 运行日志，只会传入不含用户数据的内容（NFR-LOG）。
    fn note(&self, _msg: &str) {}
}

pub struct ResearchService {
    esm: Arc<Mutex<Esm>>,
    port: Arc<dyn ResearchPort>,
    clock: Arc<dyn Clock>,
    composing: bool,
    last_input_ms: Option<i64>,
    paused: bool,
}

impl ResearchService {
    pub fn new(esm: Arc<Mutex<Esm>>, port: Arc<dyn ResearchPort>, clock: Arc<dyn Clock>) -> Self {
        Self {
            esm,
            port,
            clock,
            composing: false,
            last_input_ms: None,
            paused: false,
        }
    }

    /// 运行到总线关闭为止。
    pub async fn run(mut self, mut bus: broadcast::Receiver<HubEvent>) {
        let mut tick = tokio::time::interval(TICK);
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                ev = bus.recv() => match ev {
                    Ok(ev) => self.on_event(&ev),
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        self.port.note(&format!("研究模式跳过了 {n} 条总线事件"));
                    }
                    Err(broadcast::error::RecvError::Closed) => return,
                },
                _ = tick.tick() => {
                    self.on_tick();
                }
            }
        }
    }

    pub fn on_event(&mut self, ev: &HubEvent) {
        match ev {
            HubEvent::Pause(on) => self.paused = *on,
            HubEvent::Xqp(up) => match up.as_ref() {
                Up::Key { .. } | Up::Commit { .. } => {
                    self.last_input_ms = Some(self.clock.now_ms());
                    if matches!(up.as_ref(), Up::Commit { .. }) {
                        self.composing = false;
                    }
                }
                Up::Comp { op, .. } => {
                    self.last_input_ms = Some(self.clock.now_ms());
                    self.composing = *op == CompOp::Update;
                }
                _ => {}
            },
            _ => {}
        }
    }

    /// 每 10 秒一次；弹了邀请时返回 `true`。
    pub fn on_tick(&mut self) -> bool {
        let (id, enabled) = self.port.config();
        let mut esm = self.esm.lock().unwrap_or_else(|e| e.into_inner());
        if !research::active(&id, enabled) {
            esm.reset();
            return false;
        }
        let now = self.clock.now();
        let recent = self
            .last_input_ms
            .is_some_and(|t| now.timestamp_millis() - t < QUIET_MS);
        // 无痕期间也当作“在忙”，不打扰；等待期过了就跳过这一次
        let busy = self.composing || recent || self.paused;
        let show = esm.poll(now, &id, busy);
        drop(esm);
        if show {
            self.port.invite();
        }
        show
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Local, NaiveDate, TimeZone};
    use xqp::{KeyKind, KeySrc};

    use super::*;
    use crate::domain::research::plan;
    use crate::infra::clock::ManualClock;

    #[derive(Default)]
    struct FakePort {
        id: Mutex<String>,
        enabled: Mutex<bool>,
        invites: Mutex<u32>,
    }

    impl ResearchPort for FakePort {
        fn config(&self) -> (String, bool) {
            (
                self.id.lock().unwrap().clone(),
                *self.enabled.lock().unwrap(),
            )
        }
        fn invite(&self) {
            *self.invites.lock().unwrap() += 1;
        }
    }

    fn key() -> HubEvent {
        HubEvent::Xqp(Arc::new(Up::Key {
            ts: 0,
            seq: None,
            kind: KeyKind::Letter,
            vk: None,
            in_comp: false,
            src: KeySrc::Core,
            eaten: None,
            del_committed: None,
        }))
    }

    fn setup(
        minute: u32,
    ) -> (
        ResearchService,
        Arc<FakePort>,
        Arc<ManualClock>,
        Arc<Mutex<Esm>>,
    ) {
        let naive = NaiveDate::from_ymd_opt(2026, 10, 5)
            .unwrap()
            .and_hms_opt(minute / 60, minute % 60, 0)
            .unwrap();
        let clock = Arc::new(ManualClock::new(
            Local.from_local_datetime(&naive).earliest().unwrap(),
        ));
        let port = Arc::new(FakePort::default());
        *port.id.lock().unwrap() = "P01".into();
        let esm = Arc::new(Mutex::new(Esm::default()));
        let svc = ResearchService::new(esm.clone(), port.clone(), clock.clone());
        (svc, port, clock, esm)
    }

    #[test]
    fn off_until_both_settings_are_set() {
        let slot = plan(NaiveDate::from_ymd_opt(2026, 10, 5).unwrap(), "P01")[0];
        let (mut svc, port, _clock, _) = setup(slot);
        assert!(!svc.on_tick(), "开关没开");
        *port.enabled.lock().unwrap() = true;
        *port.id.lock().unwrap() = String::new();
        assert!(!svc.on_tick(), "没填编号");
        *port.id.lock().unwrap() = "P01".into();
        assert!(svc.on_tick());
        assert_eq!(*port.invites.lock().unwrap(), 1);
    }

    #[test]
    fn waits_while_typing_then_invites_and_marks_the_answer() {
        let slot = plan(NaiveDate::from_ymd_opt(2026, 10, 5).unwrap(), "P01")[0];
        let (mut svc, port, clock, esm) = setup(slot);
        *port.enabled.lock().unwrap() = true;
        svc.on_event(&key());
        assert!(!svc.on_tick(), "刚按过键");
        clock.advance_ms(QUIET_MS);
        assert!(svc.on_tick(), "停了 10 秒");
        assert!(esm.lock().unwrap().take_answer(clock.now_ms()));
    }

    #[test]
    fn paused_counts_as_busy() {
        let slot = plan(NaiveDate::from_ymd_opt(2026, 10, 5).unwrap(), "P01")[0];
        let (mut svc, port, _clock, _) = setup(slot);
        *port.enabled.lock().unwrap() = true;
        svc.on_event(&HubEvent::Pause(true));
        assert!(!svc.on_tick());
        svc.on_event(&HubEvent::Pause(false));
        assert!(svc.on_tick());
    }
}
