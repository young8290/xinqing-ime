//! 晚间小结服务（C-10，05 FR-REV-01）：订阅总线记最近一次输入，每 [`TICK`] 看一次该不该出小结，
//! 到了就组装统计和一句本地模板交给外壳显示（`review:evening`）。何时出、选哪句见 [`crate::domain::evening`]。
//!
//! 与外壳之间只通过 [`EveningPort`]，本模块不依赖 Tauri（ADR 0007）。实现说明见 docs/adr/0029。

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Local, NaiveDate, NaiveTime};
use tokio::sync::broadcast;
use tokio::time::MissedTickBehavior;
use xqp::Up;

use crate::bus::HubEvent;
use crate::domain::evening::{self, Check, DayFacts, EveningSummary, EveningTemplates};
use crate::infra::clock::Clock;

/// 判断间隔。演示模式的时钟走 60 倍速，5 秒就是演示时间的 5 分钟。
pub const TICK: Duration = Duration::from_secs(5);

/// 外壳提供给晚间小结服务的能力。
pub trait EveningPort: Send + Sync {
    /// 设置 `review.evening.enabled`、`review.evening.time`。
    fn enabled(&self) -> bool;
    fn at(&self) -> NaiveTime;
    /// 内部键 `review.evening.shown_on`。
    fn shown_on(&self) -> Option<NaiveDate>;
    fn mark_shown(&self, day: NaiveDate);
    /// 某一天到 `now` 为止的统计；读不出来时为 `None`（这次不出，下次再试）。
    fn facts(&self, day: NaiveDate, now: DateTime<Local>) -> Option<DayFacts>;
    /// 交给界面（`review:evening`）。
    fn show(&self, s: &EveningSummary);
    /// 运行日志，只会传入不含用户数据的内容（NFR-LOG）。
    fn note(&self, _msg: &str) {}
}

pub struct EveningService {
    port: Arc<dyn EveningPort>,
    clock: Arc<dyn Clock>,
    templates: EveningTemplates,
    /// 最近一次按键或上屏（Hub 的时钟，Unix 毫秒）
    last_input_ms: Option<i64>,
}

impl EveningService {
    pub fn new(
        port: Arc<dyn EveningPort>,
        clock: Arc<dyn Clock>,
        templates: EveningTemplates,
    ) -> Self {
        Self {
            port,
            clock,
            templates,
            last_input_ms: None,
        }
    }

    pub async fn run(mut self, mut bus: broadcast::Receiver<HubEvent>) {
        let mut tick = tokio::time::interval(TICK);
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut bus_open = true;
        loop {
            tokio::select! {
                ev = bus.recv(), if bus_open => match ev {
                    Ok(ev) => self.on_event(&ev),
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => bus_open = false,
                },
                _ = tick.tick() => {
                    self.on_tick();
                }
            }
        }
    }

    pub fn on_event(&mut self, ev: &HubEvent) {
        if let HubEvent::Xqp(up) = ev
            && matches!(up.as_ref(), Up::Key { .. } | Up::Commit { .. })
        {
            self.last_input_ms = Some(self.clock.now_ms());
        }
    }

    /// 看一次；出了小结时返回它。先用不读库的条件筛，到点且人在时才读当天统计。
    pub fn on_tick(&mut self) -> Option<EveningSummary> {
        let now = self.clock.now();
        let mut c = Check {
            now,
            enabled: self.port.enabled(),
            at: self.port.at(),
            shown_on: self.port.shown_on(),
            typing_min: u32::MAX,
            last_input_ms: self.last_input_ms,
        };
        let day = evening::due(&c)?;
        let facts = self.port.facts(day, now)?;
        c.typing_min = facts.typing_min;
        evening::due(&c)?;
        // 先记下再显示：显示失败也不在同一天反复出
        self.port.mark_shown(day);
        let s = evening::summarize(day, &facts, now, &self.templates);
        self.port.show(&s);
        Some(s)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Mutex;

    use chrono::TimeZone;
    use xqp::{KeyKind, KeySrc, MoodState};

    use super::*;
    use crate::infra::clock::ManualClock;
    use crate::infra::templates::TemplateDirs;

    #[derive(Default)]
    struct FakePort {
        shown_on: Mutex<Option<NaiveDate>>,
        typing_min: Mutex<u32>,
        shown: Mutex<Vec<EveningSummary>>,
        reads: Mutex<u32>,
    }

    impl EveningPort for FakePort {
        fn enabled(&self) -> bool {
            true
        }
        fn at(&self) -> NaiveTime {
            evening::parse_time("22:30")
        }
        fn shown_on(&self) -> Option<NaiveDate> {
            *self.shown_on.lock().unwrap()
        }
        fn mark_shown(&self, day: NaiveDate) {
            *self.shown_on.lock().unwrap() = Some(day);
        }
        fn facts(&self, _day: NaiveDate, _now: DateTime<Local>) -> Option<DayFacts> {
            *self.reads.lock().unwrap() += 1;
            Some(DayFacts {
                typing_min: *self.typing_min.lock().unwrap(),
                water: 2,
                states: vec![(0, MoodState::Tired)],
                ..Default::default()
            })
        }
        fn show(&self, s: &EveningSummary) {
            self.shown.lock().unwrap().push(s.clone());
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

    fn rig(typing_min: u32) -> (EveningService, Arc<FakePort>, Arc<ManualClock>) {
        let port = Arc::new(FakePort::default());
        *port.typing_min.lock().unwrap() = typing_min;
        let clock = Arc::new(ManualClock::new(
            Local.with_ymd_and_hms(2026, 10, 5, 22, 0, 0).unwrap(),
        ));
        let dirs = TemplateDirs::factory_only(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../hub_templates"),
        );
        let s = EveningService::new(
            port.clone(),
            clock.clone(),
            EveningTemplates::load(&dirs).unwrap(),
        );
        (s, port, clock)
    }

    #[test]
    fn shows_once_at_the_set_time_when_the_user_is_there() {
        let (mut s, port, clock) = rig(45);
        s.on_event(&key());
        assert!(s.on_tick().is_none(), "22:00 没到点");
        assert_eq!(*port.reads.lock().unwrap(), 0, "没到点不读库");
        clock.advance_ms(31 * 60_000);
        assert!(s.on_tick().is_none(), "到点了，但半小时没输入");
        s.on_event(&key());
        let shown = s.on_tick().expect("回来打字就出");
        assert_eq!(shown.date, "2026-10-05");
        assert_eq!(shown.water, 2);
        assert!(!shown.line.is_empty());
        assert!(s.on_tick().is_none(), "同一天不重复");
        assert_eq!(port.shown.lock().unwrap().len(), 1);
    }

    #[test]
    fn quiet_days_get_nothing() {
        let (mut s, port, clock) = rig(20);
        clock.advance_ms(40 * 60_000);
        s.on_event(&key());
        assert!(s.on_tick().is_none());
        assert!(port.shown.lock().unwrap().is_empty());
        assert_eq!(*port.shown_on.lock().unwrap(), None, "没出就不记");
    }
}
