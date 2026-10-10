//! 暖心话服务（03 第 2.2 节 `comfort` 任务，C-04）：订阅总线，在需要的时候说一句话。
//!
//! - 主动关怀：每个窗口（`MoodEvent::Sample`）按 FR-CMF-01 判断，受频率档位、冷却、每日上限、无痕和勿扰约束
//!   （前台全屏由外壳探测；勿扰应用、安静时段读设置，ADR 0019）；
//! - 自评回应：负面自评（`HubEvent::SelfReport`）立即回应，**不受**冷却和每日上限限制（FR-STA-10 第 2 条）。
//!
//! 生成：已同意 ④ 时用 P-COMFORT 调大模型（超时 8 秒，在网关），按 08 第 5 节校验，不通过重新生成 1 次，
//! 仍不通过或调用失败就用本地模板；未同意 ④ 直接用模板（FR-CMF-02/03）。结果写 `comfort_log` 后交给外壳显示。
//!
//! 与外壳之间只通过 [`ComfortPort`]，本模块不依赖 Tauri（ADR 0007）。

use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use chrono::{Local, TimeZone};
use tokio::sync::broadcast;
use xqp::{MoodState, Up};

use crate::bus::{HubEvent, MoodEvent};
use crate::domain::comfort::{
    self, CareLevel, ComfortPrompt, ComfortTemplates, Gate, Group, Style, Summary, Trigger,
};
use crate::domain::dnd;
use crate::domain::validate::BannedWords;
use crate::infra::clock::Clock;
use crate::infra::gateway::{AiGateway, CompleteRequest, Message, Scenario};
use crate::infra::store::ComfortRecord;

/// 第一次生成已经用了这么久时不再重新生成，直接用模板：触发到显示 ≤ 10 秒（NFR-PERF-05）。
const RETRY_BUDGET: Duration = Duration::from_secs(5);
/// 效价走向看最近几个窗口。
const VALENCE_WINDOWS: usize = 5;

/// 暖心话从哪里来：大模型生成的要标 `AI 生成`，模板句不标（FR-CMF-04 第 2 条、FR-CMF-03 第 3 条）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum ComfortSource {
    Llm,
    Template,
}

impl ComfortSource {
    pub fn as_str(self) -> &'static str {
        match self {
            ComfortSource::Llm => "llm",
            ComfortSource::Template => "template",
        }
    }
}

/// 为什么说这句话。也随 `comfort:new` 交给界面：自评后的回应在小组件上带“和晴晴聊聊”（FR-STA-10 第 2 条，ADR 0036）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum ComfortTrigger {
    /// 主动关怀（FR-CMF-01）
    Auto,
    /// 负面自评后的回应（FR-STA-10 第 2 条）
    SelfReport,
}

impl ComfortTrigger {
    pub fn as_str(self) -> &'static str {
        match self {
            ComfortTrigger::Auto => "auto",
            ComfortTrigger::SelfReport => "self_report",
        }
    }
}

/// 已写库、要显示的一句暖心话（`comfort:new` 的内容）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comfort {
    pub id: i64,
    pub ts: i64,
    pub text: String,
    pub source: ComfortSource,
    pub trigger: ComfortTrigger,
}

/// 外壳提供给暖心话服务的能力。
pub trait ComfortPort: Send + Sync {
    /// 设置 `care.level`、`care.style`。
    fn level(&self) -> CareLevel;
    fn style(&self) -> Style;
    /// 是否已同意 ④（状态摘要发给大模型）。
    fn llm_allowed(&self) -> bool;
    /// 外壳探测到的勿扰情形（前台全屏等，FR-CMF-01 第 5 条）。勿扰应用、安静时段由服务自己判断。
    fn dnd(&self) -> bool {
        false
    }
    /// 设置 `care.dnd_apps`：勿扰应用的进程名，服务按焦点事件里的前台应用比较（忽略大小写）。
    fn dnd_apps(&self) -> Vec<String> {
        dnd::DEFAULT_APPS.iter().map(|s| s.to_string()).collect()
    }
    /// 设置 `care.quiet_hours`：安静时段 `"HH:MM-HH:MM"`，按本地时间判断。
    fn quiet_hours(&self) -> Vec<String> {
        Vec::new()
    }
    /// 用户点了 🔕 之后，主动关怀停到什么时候（Unix 毫秒，FR-CMF-05）。
    fn muted_until(&self) -> Option<i64> {
        None
    }
    /// 最近 10 条暖心话（从旧到新）。
    fn recent_texts(&self) -> Vec<String>;
    /// 用户标记过“不合适”的模板句 id。
    fn blocked_templates(&self) -> HashSet<String>;
    /// `since_ms` 之后主动关怀的次数，以及最近一次主动关怀的时间。
    fn auto_since(&self, since_ms: i64) -> (u32, Option<i64>);
    /// 写入 `comfort_log`，返回行号；失败时返回 `None`（这句话照样显示）。
    fn save(&self, rec: &ComfortRecord<'_>) -> Option<i64>;
    /// 交给界面显示（`comfort:new`），小组件隐藏时点亮工具栏小圆点（FR-CMF-04 第 4 条）。
    fn show(&self, c: &Comfort);
    /// 运行日志，只会传入不含用户数据的内容（NFR-LOG）。
    fn note(&self, _msg: &str) {}
}

pub struct ComfortService {
    port: Arc<dyn ComfortPort>,
    gateway: Arc<dyn AiGateway>,
    clock: Arc<dyn Clock>,
    templates: ComfortTemplates,
    prompt: ComfortPrompt,
    banned: Arc<BannedWords>,
    trigger: Trigger,
    paused: bool,
    /// 当前前台应用（只用于勿扰判断，不落库、不出网）
    app: Option<String>,
    session_min: f64,
    valences: VecDeque<f64>,
}

impl ComfortService {
    pub fn new(
        port: Arc<dyn ComfortPort>,
        gateway: Arc<dyn AiGateway>,
        clock: Arc<dyn Clock>,
        templates: ComfortTemplates,
        prompt: ComfortPrompt,
        banned: Arc<BannedWords>,
    ) -> Self {
        Self {
            port,
            gateway,
            clock,
            templates,
            prompt,
            banned,
            trigger: Trigger::default(),
            paused: false,
            app: None,
            session_min: 0.0,
            valences: VecDeque::new(),
        }
    }

    /// 运行到总线关闭为止。处理慢了丢掉的旧事件不补（只影响持续窗口的计数）。
    pub async fn run(mut self, mut bus: broadcast::Receiver<HubEvent>) {
        loop {
            match bus.recv().await {
                Ok(ev) => {
                    self.on_event(ev).await;
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    self.port.note(&format!("暖心话服务跳过了 {n} 条总线事件"));
                }
                Err(broadcast::error::RecvError::Closed) => return,
            }
        }
    }

    /// 处理一条总线事件；说了一句话时返回它。
    pub async fn on_event(&mut self, ev: HubEvent) -> Option<Comfort> {
        match ev {
            HubEvent::Xqp(up) => {
                if let Up::Focus { app, .. } = up.as_ref() {
                    self.app = app.clone();
                }
                None
            }
            HubEvent::Window(w) => {
                self.session_min = w.0.session_min;
                None
            }
            HubEvent::Pause(on) => {
                self.paused = on;
                if on {
                    self.trigger.reset();
                }
                None
            }
            HubEvent::Mood(MoodEvent::Sample {
                ts,
                out,
                need_comfort,
                valence,
            }) => {
                if let Some(v) = valence {
                    if self.valences.len() == VALENCE_WINDOWS {
                        self.valences.pop_front();
                    }
                    self.valences.push_back(v);
                }
                let gate = self.gate(ts);
                let wanted = self
                    .trigger
                    .on_sample(ts, out.shown, need_comfort, gate)
                    .ok()?;
                Some(
                    self.say(
                        wanted.state,
                        wanted.duration_min,
                        wanted.at_ms,
                        ComfortTrigger::Auto,
                    )
                    .await,
                )
            }
            HubEvent::SelfReport { weather, .. } => {
                let state = comfort::self_report_state(weather)?;
                let ts = self.clock.now_ms();
                Some(self.say(state, 0, ts, ComfortTrigger::SelfReport).await)
            }
            _ => None,
        }
    }

    fn gate(&self, ts: i64) -> Gate {
        let (today, last_ms) = self.port.auto_since(local_midnight_ms(ts));
        Gate {
            level: self.port.level(),
            today,
            last_ms,
            paused: self.paused,
            dnd: self.dnd(ts),
            muted: self.port.muted_until().is_some_and(|until| ts < until),
        }
    }

    /// 勿扰情形（FR-CMF-01 第 5 条）：外壳探测的全屏等、前台是勿扰应用、处于安静时段。
    fn dnd(&self, ts: i64) -> bool {
        if self.port.dnd() {
            return true;
        }
        if let Some(app) = self.app.as_deref()
            && dnd::is_dnd_app(app, &self.port.dnd_apps())
        {
            return true;
        }
        Local
            .timestamp_millis_opt(ts)
            .single()
            .is_some_and(|t| dnd::in_quiet_hours(&self.port.quiet_hours(), t.time()))
    }

    /// 生成、写库、显示一句话。
    async fn say(
        &mut self,
        state: MoodState,
        duration_min: u32,
        ts: i64,
        trigger: ComfortTrigger,
    ) -> Comfort {
        let style = self.port.style();
        let recent = self.port.recent_texts();
        let summary = Summary {
            state,
            duration_min,
            local_time: Local
                .timestamp_millis_opt(ts)
                .single()
                .unwrap_or_else(|| self.clock.now()),
            session_min: self.session_min.max(0.0) as u32,
            valence_trend: comfort::valence_trend(self.valences.make_contiguous()),
        };
        let llm = if self.port.llm_allowed() {
            self.generate(style, &summary, &recent).await
        } else {
            None
        };
        let prompt_ver = self.prompt.ver();
        let (text, source, template_id, model) = match llm {
            Some((text, model)) => (text, ComfortSource::Llm, None, Some(model)),
            None => {
                let group = Group::pick(state, summary.time_of_day());
                let blocked = self.port.blocked_templates();
                match self
                    .templates
                    .pick(style, group, &recent, &blocked, ts as u64)
                {
                    Some(line) => (line.text, ComfortSource::Template, Some(line.id), None),
                    // 模板文件被改坏到一句都没有：说一句固定的话，不让用户等不到回应
                    None => (
                        "我在这儿，慢慢来。".to_string(),
                        ComfortSource::Template,
                        None,
                        None,
                    ),
                }
            }
        };
        let rec = ComfortRecord {
            ts,
            state: comfort::state_str(state),
            text: &text,
            source: source.as_str(),
            template_id: template_id.as_deref(),
            model: model.as_deref(),
            prompt_ver: (source == ComfortSource::Llm).then_some(prompt_ver.as_str()),
            trigger: trigger.as_str(),
        };
        let id = self.port.save(&rec).unwrap_or(0);
        let c = Comfort {
            id,
            ts,
            text,
            source,
            trigger,
        };
        self.port.show(&c);
        c
    }

    /// P-COMFORT：校验不通过时重新生成 1 次；调用失败（网关已经换过模型）或仍不通过返回 `None`。
    async fn generate(
        &self,
        style: Style,
        summary: &Summary,
        recent: &[String],
    ) -> Option<(String, String)> {
        let req = CompleteRequest {
            scenario: Scenario::Comfort,
            prompt_ver: self.prompt.ver(),
            messages: vec![Message {
                role: "user".into(),
                content: self.prompt.render(style, summary, recent),
            }],
        };
        let started = tokio::time::Instant::now();
        for attempt in 0..2 {
            if attempt == 1 && started.elapsed() > RETRY_BUDGET {
                break;
            }
            let resp = match self.gateway.complete(req.clone()).await {
                Ok(r) => r,
                Err(e) => {
                    self.port.note(&format!("暖心话生成失败（{e}），改用模板"));
                    return None;
                }
            };
            match comfort::validate(&resp.text, &self.banned, recent) {
                Ok(text) => return Some((text, resp.model)),
                Err(why) => self.port.note(&format!(
                    "暖心话未通过校验（{why:?}），第 {} 次",
                    attempt + 1
                )),
            }
        }
        None
    }
}

/// `ts` 所在本地日期的 00:00（Unix 毫秒）。每日上限按自然日计（FR-CMF-06）。
pub fn local_midnight_ms(ts: i64) -> i64 {
    Local
        .timestamp_millis_opt(ts)
        .single()
        .and_then(|t| {
            t.date_naive()
                .and_hms_opt(0, 0, 0)
                .and_then(|m| Local.from_local_datetime(&m).earliest())
        })
        .map(|m| m.timestamp_millis())
        .unwrap_or(ts)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Mutex;

    use async_trait::async_trait;
    use futures::stream::BoxStream;

    use super::*;
    use crate::domain::fusion::{FusionOut, Source};
    use crate::domain::self_report::SelfWeather;
    use crate::domain::settings;
    use crate::infra::clock::ManualClock;
    use crate::infra::gateway::{
        AiError, CompleteResponse, Delta, GatewayHealth, JudgeRequest, JudgeResponse,
    };
    use crate::infra::store::Db;
    use crate::infra::templates::TemplateDirs;

    /// 按顺序吐出预设的回复；用完后返回 `ModelUnavailable`。
    #[derive(Default)]
    struct FakeLlm {
        replies: Mutex<VecDeque<Result<String, AiError>>>,
        calls: Mutex<Vec<CompleteRequest>>,
    }

    impl FakeLlm {
        fn with(replies: &[Result<&str, AiError>]) -> Arc<Self> {
            let me = Self::default();
            *me.replies.lock().unwrap() = replies
                .iter()
                .map(|r| r.clone().map(str::to_string))
                .collect();
            Arc::new(me)
        }
    }

    #[async_trait]
    impl AiGateway for FakeLlm {
        async fn judge(&self, _req: JudgeRequest) -> Result<JudgeResponse, AiError> {
            Err(AiError::ModelUnavailable)
        }
        async fn complete(&self, req: CompleteRequest) -> Result<CompleteResponse, AiError> {
            self.calls.lock().unwrap().push(req);
            let next = self
                .replies
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Err(AiError::ModelUnavailable))?;
            Ok(CompleteResponse {
                model: "m1".into(),
                text: next,
                latency_ms: 10,
            })
        }
        async fn stream(
            &self,
            _req: CompleteRequest,
        ) -> Result<BoxStream<'static, Result<Delta, AiError>>, AiError> {
            Err(AiError::ModelUnavailable)
        }
        fn health(&self) -> GatewayHealth {
            GatewayHealth::default()
        }
    }

    /// 用内存数据库实现的端口，和外壳的写法一致。
    struct FakePort {
        db: Mutex<Db>,
        level: Mutex<CareLevel>,
        llm: bool,
        dnd: Mutex<bool>,
        muted: Mutex<Option<i64>>,
        shown: Mutex<Vec<Comfort>>,
    }

    impl FakePort {
        fn new(llm: bool) -> Arc<Self> {
            Arc::new(Self {
                db: Mutex::new(Db::open_in_memory().unwrap()),
                level: Mutex::new(CareLevel::Normal),
                llm,
                dnd: Mutex::new(false),
                muted: Mutex::new(None),
                shown: Mutex::new(Vec::new()),
            })
        }

        fn list(&self, key: &str) -> Vec<String> {
            settings::get(&self.db.lock().unwrap(), key)
                .unwrap()
                .as_list()
                .unwrap()
                .to_vec()
        }

        fn set(&self, key: &str, v: &[&str]) {
            settings::set(&self.db.lock().unwrap(), key, &v.into()).unwrap();
        }
    }

    impl ComfortPort for FakePort {
        fn level(&self) -> CareLevel {
            *self.level.lock().unwrap()
        }
        fn style(&self) -> Style {
            Style::Gentle
        }
        fn llm_allowed(&self) -> bool {
            self.llm
        }
        fn dnd(&self) -> bool {
            *self.dnd.lock().unwrap()
        }
        fn dnd_apps(&self) -> Vec<String> {
            self.list("care.dnd_apps")
        }
        fn quiet_hours(&self) -> Vec<String> {
            self.list("care.quiet_hours")
        }
        fn muted_until(&self) -> Option<i64> {
            *self.muted.lock().unwrap()
        }
        fn recent_texts(&self) -> Vec<String> {
            self.db.lock().unwrap().comfort_recent_texts(10).unwrap()
        }
        fn blocked_templates(&self) -> HashSet<String> {
            self.db
                .lock()
                .unwrap()
                .comfort_blocked_templates()
                .unwrap()
                .into_iter()
                .collect()
        }
        fn auto_since(&self, since_ms: i64) -> (u32, Option<i64>) {
            self.db
                .lock()
                .unwrap()
                .comfort_auto_since(since_ms)
                .unwrap()
        }
        fn save(&self, rec: &ComfortRecord<'_>) -> Option<i64> {
            self.db.lock().unwrap().comfort_insert(rec).ok()
        }
        fn show(&self, c: &Comfort) {
            self.shown.lock().unwrap().push(c.clone());
        }
    }

    const MIN: i64 = 60_000;

    fn start_ms() -> i64 {
        Local
            .with_ymd_and_hms(2026, 10, 5, 14, 0, 0)
            .unwrap()
            .timestamp_millis()
    }

    fn service(port: Arc<FakePort>, llm: Arc<FakeLlm>) -> ComfortService {
        let dirs = TemplateDirs::factory_only(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../hub_templates"),
        );
        let clock = Arc::new(ManualClock::new(
            Local.timestamp_millis_opt(start_ms()).unwrap(),
        ));
        ComfortService::new(
            port,
            llm,
            clock,
            ComfortTemplates::load(&dirs).unwrap(),
            ComfortPrompt::load(&dirs).unwrap(),
            Arc::new(BannedWords::load(&dirs).unwrap()),
        )
    }

    fn sample(ts: i64, shown: MoodState, need: Option<f64>) -> HubEvent {
        HubEvent::Mood(MoodEvent::Sample {
            ts,
            out: FusionOut {
                shown,
                changed: false,
                conflict: false,
                source: Source::Jev,
                cand: shown,
            },
            need_comfort: need,
            valence: need.map(|_| 1.0),
        })
    }

    /// 每分钟一个低落窗口，返回说出的话。
    async fn low_for(s: &mut ComfortService, from: i64, minutes: i64) -> Vec<Comfort> {
        let mut out = Vec::new();
        for m in 0..minutes {
            if let Some(c) = s
                .on_event(sample(from + m * MIN, MoodState::Low, Some(0.9)))
                .await
            {
                out.push(c);
            }
        }
        out
    }

    #[tokio::test]
    async fn tc_cmf_01_first_care_then_cooldown() {
        let port = FakePort::new(false);
        let mut s = service(port.clone(), FakeLlm::with(&[]));
        let t0 = start_ms();
        let said = low_for(&mut s, t0, 60).await;
        // 第 2 个窗口触发；适中档冷却 30 分钟，60 分钟内一共 2 次
        assert_eq!(said.len(), 2);
        assert_eq!(said[0].ts, t0 + MIN);
        assert_eq!(said[1].ts, t0 + 31 * MIN);
        assert!(said.iter().all(|c| c.source == ComfortSource::Template));
        assert_eq!(port.shown.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn degraded_mode_never_cares_proactively() {
        let port = FakePort::new(true);
        let mut s = service(port, FakeLlm::with(&[]));
        for m in 0..10 {
            assert!(
                s.on_event(sample(start_ms() + m * MIN, MoodState::Low, None))
                    .await
                    .is_none()
            );
        }
    }

    #[tokio::test]
    async fn tc_cmf_02_dnd_app_and_fullscreen_block() {
        let port = FakePort::new(false);
        let mut s = service(port.clone(), FakeLlm::with(&[]));
        s.on_event(HubEvent::Xqp(Arc::new(Up::Focus {
            ts: 0,
            seq: None,
            app: Some("WeMeetApp.exe".into()),
            scope: xqp::Scope::Normal,
            blocked: false,
        })))
        .await;
        assert!(low_for(&mut s, start_ms(), 5).await.is_empty());
        s.app = Some("WeChat.exe".into());
        *port.dnd.lock().unwrap() = true;
        assert!(low_for(&mut s, start_ms() + 10 * MIN, 5).await.is_empty());
        *port.dnd.lock().unwrap() = false;
        assert_eq!(low_for(&mut s, start_ms() + 20 * MIN, 2).await.len(), 1);
    }

    #[tokio::test]
    async fn user_dnd_apps_and_quiet_hours_block() {
        let port = FakePort::new(false);
        let mut s = service(port.clone(), FakeLlm::with(&[]));
        // 用户删光出厂的勿扰应用、加上自己的：腾讯会议不再拦，OBS 拦
        port.set("care.dnd_apps", &["obs64.exe"]);
        s.app = Some("OBS64.EXE".into());
        assert!(low_for(&mut s, start_ms(), 5).await.is_empty());
        s.app = Some("WeMeetApp.exe".into());
        // 14:00 起，安静时段 14:10-14:30
        port.set("care.quiet_hours", &["14:10-14:30"]);
        assert!(low_for(&mut s, start_ms() + 10 * MIN, 20).await.is_empty());
        assert_eq!(low_for(&mut s, start_ms() + 30 * MIN, 2).await.len(), 1);
    }

    #[tokio::test]
    async fn mute_stops_proactive_care_but_not_self_report_replies() {
        let port = FakePort::new(false);
        *port.muted.lock().unwrap() = Some(start_ms() + 60 * MIN);
        let mut s = service(port, FakeLlm::with(&[]));
        assert!(low_for(&mut s, start_ms(), 60).await.is_empty());
        assert!(
            s.on_event(HubEvent::SelfReport {
                weather: SelfWeather::Rain,
                until: 0,
            })
            .await
            .is_some(),
            "🔕 只停主动关怀"
        );
        assert_eq!(low_for(&mut s, start_ms() + 60 * MIN, 1).await.len(), 1);
    }

    #[tokio::test]
    async fn pause_blocks_and_resets_the_streak() {
        let port = FakePort::new(false);
        let mut s = service(port, FakeLlm::with(&[]));
        s.on_event(HubEvent::Pause(true)).await;
        assert!(low_for(&mut s, start_ms(), 3).await.is_empty());
        s.on_event(HubEvent::Pause(false)).await;
        let said = low_for(&mut s, start_ms() + 10 * MIN, 2).await;
        assert_eq!(said.len(), 1, "恢复后重新数满 2 个窗口");
    }

    #[tokio::test]
    async fn daily_cap_counts_only_proactive_care() {
        let port = FakePort::new(false);
        *port.level.lock().unwrap() = CareLevel::Less;
        let mut s = service(port.clone(), FakeLlm::with(&[]));
        // 自评回应不受限制也不占名额
        for _ in 0..3 {
            assert!(
                s.on_event(HubEvent::SelfReport {
                    weather: SelfWeather::Rain,
                    until: 0,
                })
                .await
                .is_some_and(|c| c.trigger == ComfortTrigger::SelfReport)
            );
        }
        // “少一些”：每天 2 次、冷却 60 分钟
        let said = low_for(&mut s, start_ms(), 240).await;
        assert_eq!(said.len(), 2);
        assert_eq!(said[1].ts - said[0].ts, 60 * MIN);
        // 关闭：不再主动关怀，但自评照样回应
        *port.level.lock().unwrap() = CareLevel::Off;
        assert!(
            low_for(&mut s, start_ms() + 24 * 60 * MIN, 120)
                .await
                .is_empty()
        );
        assert!(
            s.on_event(HubEvent::SelfReport {
                weather: SelfWeather::Night,
                until: 0,
            })
            .await
            .is_some()
        );
        assert!(
            s.on_event(HubEvent::SelfReport {
                weather: SelfWeather::Sunny,
                until: 0,
            })
            .await
            .is_none(),
            "正面自评不回应"
        );
    }

    #[tokio::test]
    async fn tc_cmf_04_llm_sentence_is_marked_and_logged() {
        let port = FakePort::new(true);
        let llm = FakeLlm::with(&[Ok(r#"{"text":"先停十秒，深呼吸一下？","kind":"rest"}"#)]);
        let mut s = service(port.clone(), llm.clone());
        let said = low_for(&mut s, start_ms(), 2).await;
        assert_eq!(said[0].source, ComfortSource::Llm);
        assert_eq!(said[0].text, "先停十秒，深呼吸一下？");
        let calls = llm.calls.lock().unwrap();
        assert_eq!(calls[0].scenario, Scenario::Comfort);
        assert_eq!(calls[0].prompt_ver, "P-COMFORT v1");
        // 摘要里只有统计值
        let prompt = &calls[0].messages[0].content;
        assert!(prompt.contains("\"state\":\"low\""));
        assert!(prompt.contains("\"duration_min\":1"));
        let row: (String, Option<String>, Option<String>, String) = port
            .db
            .lock()
            .unwrap()
            .conn()
            .query_row(
                "SELECT source, model, prompt_ver, trigger FROM comfort_log",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(
            row,
            (
                "llm".into(),
                Some("m1".into()),
                Some("P-COMFORT v1".into()),
                "auto".into()
            )
        );
    }

    #[tokio::test]
    async fn tc_cmf_05_banned_output_is_regenerated_then_falls_back() {
        let port = FakePort::new(true);
        let banned = r#"{"text":"你可能有点抑郁，早点休息吧","kind":"comfort"}"#;
        // 第一次命中禁用词，第二次通过
        let llm = FakeLlm::with(&[
            Ok(banned),
            Ok(r#"{"text":"今天辛苦啦，喝口水歇一歇。","kind":"rest"}"#),
        ]);
        let mut s = service(port.clone(), llm.clone());
        let said = low_for(&mut s, start_ms(), 2).await;
        assert_eq!(said[0].source, ComfortSource::Llm);
        assert_eq!(llm.calls.lock().unwrap().len(), 2);
        // 两次都不通过：用模板，界面上不出现禁用词
        let llm = FakeLlm::with(&[Ok(banned), Ok(banned)]);
        let mut s = service(port, llm.clone());
        let said = low_for(&mut s, start_ms() + 120 * MIN, 2).await;
        assert_eq!(said[0].source, ComfortSource::Template);
        assert!(!said[0].text.contains("抑郁"));
        assert_eq!(llm.calls.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn tc_cmf_03_offline_uses_templates_without_repeats() {
        let port = FakePort::new(true);
        // 网关全部失败（断网）
        let mut s = service(port.clone(), FakeLlm::with(&[]));
        let mut texts = Vec::new();
        for i in 0..20 {
            let c = s
                .on_event(HubEvent::SelfReport {
                    weather: SelfWeather::Storm,
                    until: i,
                })
                .await
                .unwrap();
            assert_eq!(c.source, ComfortSource::Template);
            texts.push(c.text);
        }
        for (i, t) in texts.iter().enumerate() {
            let window = &texts[i.saturating_sub(10)..i];
            assert!(!window.contains(t), "第 {i} 句与前 10 句重复：{t}");
        }
    }

    #[tokio::test]
    async fn without_consent_the_model_is_never_called() {
        let port = FakePort::new(false);
        let llm = FakeLlm::with(&[Ok(r#"{"text":"先停十秒，深呼吸一下？","kind":"rest"}"#)]);
        let mut s = service(port, llm.clone());
        let said = low_for(&mut s, start_ms(), 2).await;
        assert_eq!(said[0].source, ComfortSource::Template);
        assert!(llm.calls.lock().unwrap().is_empty(), "未同意 ④ 不出网");
    }

    #[test]
    fn midnight_is_local() {
        let t = Local.with_ymd_and_hms(2026, 10, 5, 14, 30, 0).unwrap();
        let m = Local.with_ymd_and_hms(2026, 10, 5, 0, 0, 0).unwrap();
        assert_eq!(
            local_midnight_ms(t.timestamp_millis()),
            m.timestamp_millis()
        );
    }
}
