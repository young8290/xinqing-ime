//! 休息提醒服务（03 第 2.2 节 `rest` 任务，B-08）：订阅总线记使用时长，每秒问一次 [`RestEngine`] 该不该提醒。
//!
//! 计时来源（FR-RST-01）：平时用 XQP 的按键、上屏事件；无痕、英文状态、没连上输入法时改为每 10 秒读一次
//! 系统空闲时间（外壳提供，`rest.count_when_paused` 关闭时无痕期间不计）。显示与写 `reminder_log` 交给外壳。
//!
//! 与外壳之间只通过 [`RestPort`]，本模块不依赖 Tauri（ADR 0007）。

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{broadcast, mpsc};
use tokio::time::MissedTickBehavior;
use xqp::{CompOp, MoodState, Up};

use crate::bus::{HubEvent, MoodEvent};
use crate::domain::rest::{Ctx, Due, Input, RestAction, RestConfig, RestEngine, RestKind};
use crate::infra::clock::Clock;

/// 判断间隔。
pub const TICK: Duration = Duration::from_secs(1);
/// 读系统空闲时间、重读设置的间隔。
const SLOW_EVERY_TICKS: u32 = 10;

/// 外壳提供给休息提醒服务的能力。
pub trait RestPort: Send + Sync {
    /// 设置 `rest.*`（FR-RST-09）。
    fn config(&self) -> RestConfig;
    /// 设置 `rest.count_when_paused`。
    fn count_when_paused(&self) -> bool;
    /// 是否连着输入法核心。
    fn linked(&self) -> bool;
    /// 前台全屏等勿扰情形（FR-RST-06 第 3 条）。
    fn dnd(&self) -> bool {
        false
    }
    /// 系统空闲毫秒（Windows `GetLastInputInfo`）；拿不到时为 `None`，此时只靠 XQP 事件计时。
    fn system_idle_ms(&self) -> Option<u64> {
        None
    }
    /// 显示提醒：光标旁气泡 + 小组件卡片（FR-RST-06 第 4 条）。
    fn show(&self, due: &Due);
    /// 写入 `reminder_log`（FR-RST-08）。
    fn log(&self, ts: i64, kind: RestKind, action: RestAction);
    /// 运行日志，只会传入不含用户数据的内容（NFR-LOG）。
    fn note(&self, _msg: &str) {}
}

/// 外壳发给服务的命令。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestCmd {
    /// 用户点了提醒卡片上的按钮。
    Act { kind: RestKind, action: RestAction },
}

pub struct RestService {
    engine: RestEngine,
    port: Arc<dyn RestPort>,
    clock: Arc<dyn Clock>,
    paused: bool,
    /// 输入法处于激活的中文状态：这时核心看得到按键，用 XQP 事件计时
    ime_chinese: bool,
    ticks: u32,
}

impl RestService {
    pub fn new(port: Arc<dyn RestPort>, clock: Arc<dyn Clock>) -> Self {
        let engine = RestEngine::new(port.config());
        Self {
            engine,
            port,
            clock,
            paused: false,
            ime_chinese: true,
            ticks: 0,
        }
    }

    /// 运行到命令通道关闭为止。总线关闭后只处理命令和计时。
    pub async fn run(
        mut self,
        mut bus: broadcast::Receiver<HubEvent>,
        mut cmds: mpsc::Receiver<RestCmd>,
    ) {
        let mut tick = tokio::time::interval(TICK);
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut bus_open = true;
        loop {
            tokio::select! {
                ev = bus.recv(), if bus_open => match ev {
                    Ok(ev) => self.on_event(&ev),
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        self.port.note(&format!("休息提醒跳过了 {n} 条总线事件"));
                    }
                    Err(broadcast::error::RecvError::Closed) => bus_open = false,
                },
                cmd = cmds.recv() => match cmd {
                    Some(c) => self.on_cmd(c),
                    None => return,
                },
                _ = tick.tick() => {
                    self.on_tick();
                }
            }
        }
    }

    /// 现在是否改用系统空闲时间计时。
    fn system_source(&self) -> bool {
        self.paused || !self.ime_chinese || !self.port.linked()
    }

    pub fn on_event(&mut self, ev: &HubEvent) {
        match ev {
            HubEvent::Xqp(up) => self.on_up(up),
            HubEvent::Pause(on) => self.paused = *on,
            HubEvent::Mood(MoodEvent::StateChanged { state, .. }) => {
                self.engine.set_tired(*state == MoodState::Tired);
            }
            _ => {}
        }
    }

    fn on_up(&mut self, up: &Up) {
        if let Up::Ime {
            active, chinese, ..
        } = up
        {
            self.ime_chinese = *active && *chinese;
            return;
        }
        if self.paused {
            // 无痕期间核心不发采集事件；万一收到也不计（FR-SEN-06）
            return;
        }
        let input = match up {
            Up::Key { .. } => Input::Key,
            Up::Commit { .. } => Input::Commit,
            Up::Comp { op, .. } => match op {
                CompOp::Update => Input::CompUpdate,
                CompOp::Cancel | CompOp::Clear | CompOp::Terminated => Input::CompEnd,
            },
            _ => return,
        };
        self.engine.on_input(input, self.clock.now());
    }

    pub fn on_cmd(&mut self, cmd: RestCmd) {
        match cmd {
            RestCmd::Act { kind, action } => {
                let now = self.clock.now();
                self.engine.act(kind, action, now);
                self.port.log(now.timestamp_millis(), kind, action);
            }
        }
    }

    /// 每秒一次；有提醒要显示时返回它。
    pub fn on_tick(&mut self) -> Option<Due> {
        let slow = self.ticks.is_multiple_of(SLOW_EVERY_TICKS);
        self.ticks = self.ticks.wrapping_add(1);
        if slow {
            self.engine.set_config(self.port.config());
        }
        let now = self.clock.now();
        let system = self.system_source();
        let mut idle = None;
        if system {
            idle = self.port.system_idle_ms();
            let counting = !self.paused || self.port.count_when_paused();
            if let (true, true, Some(ms)) = (slow, counting, idle) {
                self.engine.on_system_idle(ms, now);
            }
        }
        let ctx = Ctx {
            dnd: self.port.dnd(),
            system_idle_ms: idle,
        };
        let due = self.engine.poll(now, ctx)?;
        self.port.show(&due);
        Some(due)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use chrono::{Duration as ChronoDuration, Local, NaiveDate, TimeZone};
    use xqp::{KeyKind, KeySrc};

    use super::*;
    use crate::infra::clock::ManualClock;

    #[derive(Default)]
    struct FakePort {
        shown: Mutex<Vec<Due>>,
        logged: Mutex<Vec<(RestKind, RestAction)>>,
        idle: Mutex<Option<u64>>,
        linked: Mutex<bool>,
        cfg: Mutex<RestConfig>,
    }

    impl RestPort for FakePort {
        fn config(&self) -> RestConfig {
            *self.cfg.lock().unwrap()
        }
        fn count_when_paused(&self) -> bool {
            true
        }
        fn linked(&self) -> bool {
            *self.linked.lock().unwrap()
        }
        fn system_idle_ms(&self) -> Option<u64> {
            *self.idle.lock().unwrap()
        }
        fn show(&self, due: &Due) {
            self.shown.lock().unwrap().push(*due);
        }
        fn log(&self, _ts: i64, kind: RestKind, action: RestAction) {
            self.logged.lock().unwrap().push((kind, action));
        }
    }

    fn setup() -> (RestService, Arc<FakePort>, Arc<ManualClock>) {
        let naive = NaiveDate::from_ymd_opt(2026, 3, 10)
            .unwrap()
            .and_hms_opt(10, 0, 0)
            .unwrap();
        let clock = Arc::new(ManualClock::new(
            Local.from_local_datetime(&naive).earliest().unwrap(),
        ));
        let port = Arc::new(FakePort {
            linked: Mutex::new(true),
            ..FakePort::default()
        });
        {
            let mut c = port.cfg.lock().unwrap();
            c.water.enabled = false;
            c.mv.enabled = false;
            c.night_enabled = false;
        }
        let svc = RestService::new(port.clone(), clock.clone());
        (svc, port, clock)
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

    #[test]
    fn keys_drive_the_eye_reminder_and_actions_are_logged() {
        let (mut svc, port, clock) = setup();
        for _ in 0..20 {
            svc.on_event(&key());
            assert_eq!(svc.on_tick(), None);
            clock.advance_ms(60_000);
        }
        // 最后一次按键已过去 1 分钟（5 秒空闲早满足）
        let due = svc.on_tick().expect("护眼到期");
        assert_eq!(due.kind, RestKind::Eye);
        assert_eq!(port.shown.lock().unwrap().len(), 1);
        svc.on_cmd(RestCmd::Act {
            kind: RestKind::Eye,
            action: RestAction::Done,
        });
        assert_eq!(
            port.logged.lock().unwrap().as_slice(),
            &[(RestKind::Eye, RestAction::Done)]
        );
    }

    #[test]
    fn paused_counts_with_system_idle() {
        let (mut svc, port, clock) = setup();
        svc.on_event(&HubEvent::Pause(true));
        *port.idle.lock().unwrap() = Some(1_000);
        // 无痕期间收到的按键不计；系统空闲每 10 秒读一次
        for _ in 0..(20 * 60) {
            svc.on_event(&key());
            assert_eq!(svc.on_tick(), None, "还在操作电脑，不显示");
            clock.advance_ms(1_000);
        }
        *port.idle.lock().unwrap() = Some(6_000);
        clock.advance_ms(ChronoDuration::seconds(1).num_milliseconds());
        assert_eq!(svc.on_tick().map(|d| d.kind), Some(RestKind::Eye));
    }

    #[test]
    fn english_mode_switches_to_system_idle() {
        let (mut svc, port, _clock) = setup();
        assert!(!svc.system_source());
        svc.on_event(&HubEvent::Xqp(Arc::new(Up::Ime {
            ts: 0,
            seq: None,
            active: true,
            chinese: false,
        })));
        assert!(svc.system_source());
        *port.linked.lock().unwrap() = false;
        svc.ime_chinese = true;
        assert!(svc.system_source(), "没连上输入法");
    }
}
