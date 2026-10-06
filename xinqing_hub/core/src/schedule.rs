//! 日程与待办识别服务（C-05、C-06，06 FR-SCH-01～06、FR-SCH-10～13）。
//!
//! 上屏文字（只有同意 ③ 时核心才发 `commit.text`）在内存里拼成句子 → L1 本地初筛（[`ScheduleRecognizer`]）→
//! L2 Jev 确认（Q-PLAN / Q-TODO，阈值 0.80）→ 光标旁气泡与“识别中…”卡片 → L3 大模型抽取（P-SCHEDULE / P-TODO，
//! 校验不过重来一次，失败、超时或说“没有”时按本地规则抽取）→ 去重、冲突、相似 → 存为待确认 → “卡片就绪”。
//!
//! 原句只在内存里（`Zeroizing`），出网的只有命中初筛的单句，不落库、不写日志（FR-SCH-10）。
//! 与外壳之间只通过 [`SchedulePort`]，本模块不依赖 Tauri（ADR 0007）。实现说明见 docs/adr/0032。

use std::ops::Deref;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::NaiveDate;
use tokio::sync::broadcast;
use tokio::time::MissedTickBehavior;
use xqp::Up;
use zeroize::Zeroizing;

use crate::bus::HubEvent;
use crate::domain::agenda::{self, Conflict, Slot};
use crate::domain::reminder;
use crate::domain::schedule::{
    self, CandidateKind, ExtractPrompts, FLAG_MAYBE_DUP, ScheduleDraft, ScheduleRecognizer,
    TodoDraft,
};
use crate::domain::validate::BannedWords;
use crate::infra::clock::Clock;
use crate::infra::gateway::{
    AiGateway, Answer, CompleteRequest, JudgeRequest, Message, Question, Scenario,
};
use crate::infra::store::{Db, ScheduleRow, StoreError, TodoRow};

/// L2 阈值（FR-SCH-02、FR-SCH-12）。
pub const JEV_THRESHOLD: f64 = 0.80;
/// 句子缓冲：上屏后这么久没有新的上屏，就把缓冲里剩下的当作一句话（聊天里常常不打句号就发出去）。
pub const IDLE_FLUSH: Duration = Duration::from_secs(3);
/// 缓冲最多这么多字，超出时丢掉最早的（初筛只看 ≤ 120 字的句子，FR-SCH-01 第 1 条）。
pub const BUFFER_MAX_CHARS: usize = 240;
/// 判断间隔。
pub const TICK: Duration = Duration::from_secs(1);

const TERMINATORS: [char; 8] = ['。', '！', '？', '；', '\n', '!', '?', ';'];

/// 外壳提供给识别服务的能力。
pub trait SchedulePort: Send + Sync {
    /// 是否已同意 ③（日程与待办识别）。核心只在同意时发 `commit.text`，这里再查一次，以 Hub 为准。
    fn allowed(&self) -> bool;
    /// 每日初筛命中上限（设置 `ai.cap.schedule`，FR-SCH-01 第 4 条）。
    fn daily_cap(&self) -> usize;
    /// 有时刻的日程默认提前几秒提醒（设置 `sch.default_offsets`，FR-SCH-07）。
    fn default_offsets(&self) -> Vec<i64>;
    /// 写连接（外壳的 `DbWriter::lock`）。每次取用都很短，不跨 `.await` 持有。
    fn db(&self) -> Box<dyn Deref<Target = Db> + '_>;
    /// L2 通过：光标旁气泡“📅 识别到日程”/“✅ 识别到待办”（FR-SCH-05 第 1 条，经 XQP `tip`）。
    fn tip(&self, kind: CandidateKind);
    /// 卡片先显示“识别中…”（`schedule:detected` / `todo:detected`）。
    fn detected(&self, card_id: u32, kind: CandidateKind);
    /// 抽取完成（`schedule:ready` / `todo:ready`）。
    fn ready(&self, r: &Ready);
    /// 运行日志，只会传入不含用户数据的内容（NFR-LOG）。
    fn note(&self, _msg: &str) {}
}

/// 卡片就绪时交给界面的内容。`row` 为 `None` 表示 24 小时内已经提示过同一件事（FR-SCH-06），卡片直接收起。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ready {
    Schedule {
        card_id: u32,
        row: Option<ScheduleRow>,
        /// 与已添加日程的时间重叠（FR-SCH-11）
        conflicts: Vec<Conflict>,
    },
    Todo {
        card_id: u32,
        row: Option<TodoRow>,
    },
}

pub struct ScheduleService {
    port: Arc<dyn SchedulePort>,
    gateway: Arc<dyn AiGateway>,
    clock: Arc<dyn Clock>,
    recognizer: ScheduleRecognizer,
    prompts: ExtractPrompts,
    banned: Arc<BannedWords>,
    next_card: AtomicU32,
    state: Mutex<Buffer>,
}

/// 还没成句的上屏文字与当天的初筛命中数。
#[derive(Default)]
struct Buffer {
    text: Zeroizing<String>,
    last_commit_ms: i64,
    day: Option<NaiveDate>,
    accepted: usize,
}

impl ScheduleService {
    pub fn new(
        port: Arc<dyn SchedulePort>,
        gateway: Arc<dyn AiGateway>,
        clock: Arc<dyn Clock>,
        recognizer: ScheduleRecognizer,
        prompts: ExtractPrompts,
        banned: Arc<BannedWords>,
    ) -> Self {
        Self {
            port,
            gateway,
            clock,
            recognizer,
            prompts,
            banned,
            next_card: AtomicU32::new(1),
            state: Mutex::default(),
        }
    }

    pub async fn run(self: Arc<Self>, mut bus: broadcast::Receiver<HubEvent>) {
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
                _ = tick.tick() => self.on_tick(),
            }
        }
    }

    /// 处理一条总线事件：上屏文字进缓冲，成句的拿去初筛；切换窗口时缓冲里剩下的也算一句。
    pub fn on_event(self: &Arc<Self>, ev: &HubEvent) {
        let HubEvent::Xqp(up) = ev else { return };
        match up.as_ref() {
            Up::Commit {
                text: Some(text), ..
            } => {
                if !self.port.allowed() {
                    self.clear();
                    return;
                }
                let sentences = self.push(text);
                self.screen(&sentences);
            }
            Up::Focus { .. } => {
                let rest = self.take_rest();
                self.screen(&rest);
            }
            _ => {}
        }
    }

    /// 上屏后停了一会儿：缓冲里剩下的当作一句。
    pub fn on_tick(self: &Arc<Self>) {
        let idle = {
            let st = self.state.lock().unwrap_or_else(|e| e.into_inner());
            !st.text.is_empty()
                && self.clock.now_ms() - st.last_commit_ms >= IDLE_FLUSH.as_millis() as i64
        };
        if idle {
            let rest = self.take_rest();
            self.screen(&rest);
        }
    }

    fn clear(&self) {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        st.text = Zeroizing::default();
    }

    /// 追加上屏文字，切出已经成句的部分（带句末标点）。
    fn push(&self, text: &str) -> Vec<Zeroizing<String>> {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        st.last_commit_ms = self.clock.now_ms();
        st.text.push_str(text);
        let n = st.text.chars().count();
        if n > BUFFER_MAX_CHARS {
            let keep: String = st.text.chars().skip(n - BUFFER_MAX_CHARS).collect();
            st.text = Zeroizing::new(keep);
        }
        let Some(cut) = st.text.rfind(TERMINATORS) else {
            return Vec::new();
        };
        let end = cut + st.text[cut..].chars().next().map_or(0, char::len_utf8);
        let done = Zeroizing::new(st.text[..end].to_string());
        st.text = Zeroizing::new(st.text[end..].to_string());
        vec![done]
    }

    fn take_rest(&self) -> Vec<Zeroizing<String>> {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if st.text.trim().is_empty() {
            st.text = Zeroizing::default();
            return Vec::new();
        }
        vec![std::mem::take(&mut st.text)]
    }

    /// L1 初筛；命中的每一句各开一个任务做 L2、L3。
    fn screen(self: &Arc<Self>, texts: &[Zeroizing<String>]) {
        for text in texts {
            let candidates = {
                let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
                let today = self.clock.now().date_naive();
                if st.day != Some(today) {
                    st.day = Some(today);
                    st.accepted = 0;
                }
                let cap = self.port.daily_cap().min(self.recognizer.daily_cap());
                let r = self.recognizer.scan_capped(text, st.accepted, cap);
                st.accepted += r.candidates.len();
                if r.skipped_by_daily_cap > 0 {
                    self.port
                        .note("日程识别：今天的初筛命中已到上限，其余不再识别");
                }
                r.candidates
            };
            for c in candidates {
                let me = Arc::clone(self);
                let sentence = Zeroizing::new(c.text);
                tokio::spawn(async move { me.process(c.kind, sentence).await });
            }
        }
    }

    /// 一句命中初筛的话：L2 → 卡片 → L3 → 存为待确认 → 卡片就绪。返回就绪的内容（L2 没通过时为 `None`）。
    pub async fn process(&self, kind: CandidateKind, sentence: Zeroizing<String>) -> Option<Ready> {
        if !self.confirm(kind, &sentence).await {
            return None;
        }
        let card_id = self.next_card.fetch_add(1, Ordering::Relaxed);
        self.port.tip(kind);
        self.port.detected(card_id, kind);
        let ready = match kind {
            CandidateKind::Schedule => {
                let draft = self.extract_schedule(&sentence).await;
                drop(sentence);
                self.save_schedule(card_id, draft)
            }
            CandidateKind::Todo => {
                let draft = self.extract_todo(&sentence).await;
                drop(sentence);
                self.save_todo(card_id, draft)
            }
        };
        let ready = match ready {
            Ok(r) => r,
            Err(e) => {
                self.port.note(&format!("日程识别：保存失败（{e}）"));
                // 卡片已经在“识别中…”，告诉界面收起
                match kind {
                    CandidateKind::Schedule => Ready::Schedule {
                        card_id,
                        row: None,
                        conflicts: Vec::new(),
                    },
                    CandidateKind::Todo => Ready::Todo { card_id, row: None },
                }
            }
        };
        self.port.ready(&ready);
        Some(ready)
    }

    /// L2：Q-PLAN / Q-TODO ≥ 0.80。Jev 不可用时不弹卡片（初筛本来就宽，没有确认就弹会很吵，ADR 0032）。
    async fn confirm(&self, kind: CandidateKind, sentence: &str) -> bool {
        let question = match kind {
            CandidateKind::Schedule => Question::Plan,
            CandidateKind::Todo => Question::Todo,
        };
        let mut state = serde_json::Map::new();
        state.insert("committed_text".into(), sentence.into());
        let req = JudgeRequest {
            questions: vec![question],
            state,
        };
        match self.gateway.judge(req).await {
            Ok(r) => matches!(
                r.answers.get(&question),
                Some(Answer::Noul { p }) if *p >= JEV_THRESHOLD
            ),
            Err(e) => {
                self.port
                    .note(&format!("日程识别：Jev 不可用（{e}），这句不提示"));
                false
            }
        }
    }

    async fn ask(&self, kind: CandidateKind, sentence: &str, today: NaiveDate) -> Option<String> {
        let scenario = match kind {
            CandidateKind::Schedule => Scenario::Schedule,
            CandidateKind::Todo => Scenario::Todo,
        };
        let req = CompleteRequest {
            scenario,
            prompt_ver: self.prompts.ver(kind),
            messages: vec![Message {
                role: "user".into(),
                content: self.prompts.render(kind, sentence, today),
            }],
        };
        match self.gateway.complete(req).await {
            Ok(r) => Some(r.text),
            Err(e) => {
                self.port
                    .note(&format!("日程识别：大模型不可用（{e}），按本地规则抽取"));
                None
            }
        }
    }

    /// L3：校验不过重来一次（08 第 5 节），失败、超时或说“没有日程”时按本地规则（FR-SCH-03 第 2 条）。
    async fn extract_schedule(&self, sentence: &str) -> ScheduleDraft {
        let now = self.clock.now().naive_local();
        for _ in 0..2 {
            let Some(raw) = self
                .ask(CandidateKind::Schedule, sentence, now.date())
                .await
            else {
                break;
            };
            match schedule::validate_schedule_json(&raw, sentence, now, &self.banned) {
                Ok(Some(d)) => return d,
                Ok(None) => break,
                Err(e) => self
                    .port
                    .note(&format!("日程识别：抽取结果没通过校验（{e}）")),
            }
        }
        self.recognizer.local_schedule(sentence, now, &self.banned)
    }

    async fn extract_todo(&self, sentence: &str) -> TodoDraft {
        let now = self.clock.now().naive_local();
        for _ in 0..2 {
            let Some(raw) = self.ask(CandidateKind::Todo, sentence, now.date()).await else {
                break;
            };
            match schedule::validate_todo_json(&raw, sentence, now, &self.banned) {
                Ok(Some(d)) => return d,
                Ok(None) => break,
                Err(e) => self
                    .port
                    .note(&format!("待办识别：抽取结果没通过校验（{e}）")),
            }
        }
        self.recognizer.local_todo(sentence)
    }

    fn save_schedule(&self, card_id: u32, mut draft: ScheduleDraft) -> Result<Ready, StoreError> {
        draft.remind_offsets = reminder::default_offsets(
            draft.is_deadline,
            draft.all_day,
            &self.port.default_offsets(),
        );
        let db = self.port.db();
        let same_day = match draft.date.as_deref() {
            Some(d) => db.schedules_added_on(d)?,
            None => Vec::new(),
        };
        if agenda::maybe_duplicate(&draft, &same_day, None) {
            draft.flags.push(FLAG_MAYBE_DUP.to_owned());
        }
        let (id, inserted) = db.schedule_create(&draft, "pending", self.clock.now_ms())?;
        if !inserted {
            return Ok(Ready::Schedule {
                card_id,
                row: None,
                conflicts: Vec::new(),
            });
        }
        let conflicts = agenda::conflicts(&Slot::of_draft(&draft), &same_day, None);
        Ok(Ready::Schedule {
            card_id,
            row: db.schedule_get(id)?,
            conflicts,
        })
    }

    fn save_todo(&self, card_id: u32, draft: TodoDraft) -> Result<Ready, StoreError> {
        let db = self.port.db();
        let (id, inserted) = db.todo_create(&draft, "pending", self.clock.now_ms())?;
        Ok(Ready::Todo {
            card_id,
            row: if inserted { db.todo_get(id)? } else { None },
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use async_trait::async_trait;
    use chrono::{Local, TimeZone};
    use futures::stream::BoxStream;

    use super::*;
    use crate::infra::clock::ManualClock;
    use crate::infra::gateway::{AiError, CompleteResponse, Delta, GatewayHealth, JudgeResponse};
    use crate::infra::templates::TemplateDirs;

    struct Port {
        db: Mutex<Db>,
        allowed: bool,
        tips: Mutex<Vec<CandidateKind>>,
        detected: Mutex<Vec<u32>>,
        ready: Mutex<Vec<Ready>>,
    }

    struct Guard<'a>(std::sync::MutexGuard<'a, Db>);
    impl Deref for Guard<'_> {
        type Target = Db;
        fn deref(&self) -> &Db {
            &self.0
        }
    }

    impl SchedulePort for Port {
        fn allowed(&self) -> bool {
            self.allowed
        }
        fn daily_cap(&self) -> usize {
            50
        }
        fn default_offsets(&self) -> Vec<i64> {
            vec![600]
        }
        fn db(&self) -> Box<dyn Deref<Target = Db> + '_> {
            Box::new(Guard(self.db.lock().unwrap()))
        }
        fn tip(&self, kind: CandidateKind) {
            self.tips.lock().unwrap().push(kind);
        }
        fn detected(&self, card_id: u32, _kind: CandidateKind) {
            self.detected.lock().unwrap().push(card_id);
        }
        fn ready(&self, r: &Ready) {
            self.ready.lock().unwrap().push(r.clone());
        }
    }

    struct Gateway {
        p: f64,
        replies: Mutex<Vec<Result<String, AiError>>>,
        sent: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl AiGateway for Gateway {
        async fn judge(&self, req: JudgeRequest) -> Result<JudgeResponse, AiError> {
            let q = req.questions[0];
            assert!(req.state.contains_key("committed_text"));
            Ok(JudgeResponse {
                model: "jev".into(),
                latency_ms: 1,
                answers: [(q, Answer::Noul { p: self.p })].into(),
            })
        }
        async fn complete(&self, req: CompleteRequest) -> Result<CompleteResponse, AiError> {
            self.sent
                .lock()
                .unwrap()
                .push(req.messages[0].content.clone());
            let mut r = self.replies.lock().unwrap();
            if r.is_empty() {
                return Err(AiError::Timeout);
            }
            r.remove(0).map(|text| CompleteResponse {
                model: "m".into(),
                text,
                latency_ms: 1,
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

    fn rig(
        p: f64,
        replies: Vec<Result<String, AiError>>,
    ) -> (
        Arc<ScheduleService>,
        Arc<Port>,
        Arc<Gateway>,
        Arc<ManualClock>,
    ) {
        let dirs = TemplateDirs::factory_only(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../hub_templates"),
        );
        let port = Arc::new(Port {
            db: Mutex::new(Db::open_in_memory().unwrap()),
            allowed: true,
            tips: Mutex::default(),
            detected: Mutex::default(),
            ready: Mutex::default(),
        });
        let gw = Arc::new(Gateway {
            p,
            replies: Mutex::new(replies),
            sent: Mutex::default(),
        });
        // FR-SCH-04 验收用的“今天”：2026-10-03（周六）上午 10 点
        let clock = Arc::new(ManualClock::new(
            Local.with_ymd_and_hms(2026, 10, 3, 10, 0, 0).unwrap(),
        ));
        let s = Arc::new(ScheduleService::new(
            port.clone(),
            gw.clone(),
            clock.clone(),
            ScheduleRecognizer::load(&dirs).unwrap(),
            ExtractPrompts::load(&dirs).unwrap(),
            Arc::new(BannedWords::load(&dirs).unwrap()),
        ));
        (s, port, gw, clock)
    }

    const GROUP: &str = r#"{"has_event":true,"title":"组会","date":"2026-10-09","time":"15:00","end_time":null,"all_day":false,"location":"实验楼","is_deadline":false}"#;

    fn row_of(r: &Ready) -> ScheduleRow {
        match r {
            Ready::Schedule { row: Some(row), .. } => row.clone(),
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn schedule_goes_through_l2_and_l3() {
        let (s, port, gw, _) = rig(0.95, vec![Ok(GROUP.into())]);
        let r = s
            .process(
                CandidateKind::Schedule,
                Zeroizing::new("好的，周五下午三点在实验楼开组会".into()),
            )
            .await
            .unwrap();
        let row = row_of(&r);
        assert_eq!(
            (
                row.title.as_deref(),
                row.date.as_deref(),
                row.time.as_deref()
            ),
            (Some("组会"), Some("2026-10-09"), Some("15:00"))
        );
        assert_eq!(
            (row.status.as_str(), row.remind_offsets.clone()),
            ("pending", vec![600])
        );
        assert_eq!(*port.tips.lock().unwrap(), [CandidateKind::Schedule]);
        assert_eq!(*port.detected.lock().unwrap(), [1]);
        assert!(gw.sent.lock().unwrap()[0].contains("今天是 2026-10-03（周六）"));
        // 同一件事 24 小时内再来一次：卡片收起，不再提示
        gw.replies.lock().unwrap().push(Ok(GROUP.into()));
        let again = s
            .process(
                CandidateKind::Schedule,
                Zeroizing::new("周五下午三点开组会".into()),
            )
            .await
            .unwrap();
        assert!(matches!(again, Ready::Schedule { row: None, .. }));
    }

    #[tokio::test]
    async fn below_threshold_shows_nothing() {
        let (s, port, gw, _) = rig(0.5, vec![Ok(GROUP.into())]);
        assert!(
            s.process(CandidateKind::Schedule, Zeroizing::new("周五开组会".into()))
                .await
                .is_none()
        );
        assert!(port.tips.lock().unwrap().is_empty() && port.ready.lock().unwrap().is_empty());
        assert!(gw.sent.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn bad_output_is_retried_then_local_rules() {
        let (s, _, gw, _) = rig(0.95, vec![Ok("不是 JSON".into()), Ok("还不是".into())]);
        let r = s
            .process(
                CandidateKind::Schedule,
                Zeroizing::new("明晚八点前交数据库作业".into()),
            )
            .await
            .unwrap();
        let row = row_of(&r);
        assert_eq!(gw.sent.lock().unwrap().len(), 2);
        assert!(row.flags.contains(&schedule::FLAG_LOCAL.to_owned()));
        assert_eq!(
            (row.title.as_deref(), row.time.as_deref(), row.is_deadline),
            (Some("交数据库作业"), Some("20:00"), true)
        );
        assert_eq!(
            row.remind_offsets,
            vec![86_400, 7_200],
            "截止类默认提前 1 天和 2 小时"
        );

        // 超时（网关出错）不重试，直接本地
        let (s, _, gw, _) = rig(0.95, vec![]);
        let r = s
            .process(
                CandidateKind::Schedule,
                Zeroizing::new("下周二和室友去看电影".into()),
            )
            .await
            .unwrap();
        assert_eq!(gw.sent.lock().unwrap().len(), 1);
        assert!(row_of(&r).all_day);
    }

    #[tokio::test]
    async fn conflicts_and_similar_titles_are_flagged() {
        let (s, port, _, _) = rig(
            0.95,
            vec![Ok(GROUP
                .replace("组会", "实验室组会")
                .replace("15:00", "15:30"))],
        );
        let existing = ScheduleDraft {
            title: "实验室组会议".into(),
            date: Some("2026-10-09".into()),
            time: Some("15:00".into()),
            end_time: Some("16:00".into()),
            all_day: false,
            location: None,
            is_deadline: false,
            remind_offsets: vec![600],
            source: "manual".into(),
            flags: vec![],
        };
        port.db().schedule_create(&existing, "added", 0).unwrap();
        let r = s
            .process(
                CandidateKind::Schedule,
                Zeroizing::new("周五下午三点半在实验室组会".into()),
            )
            .await
            .unwrap();
        let Ready::Schedule {
            row: Some(row),
            conflicts,
            ..
        } = r
        else {
            panic!()
        };
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].title, "实验室组会议");
        assert!(row.flags.contains(&FLAG_MAYBE_DUP.to_owned()));
    }

    #[tokio::test]
    async fn todos_are_pending_until_confirmed() {
        let (s, port, _, _) = rig(
            0.9,
            vec![Ok(
                r#"{"is_todo":true,"title":"打印简历","due_date":null}"#.into()
            )],
        );
        let r = s
            .process(
                CandidateKind::Todo,
                Zeroizing::new("记得明天打印简历".into()),
            )
            .await
            .unwrap();
        let Ready::Todo { row: Some(t), .. } = r else {
            panic!()
        };
        assert_eq!(
            (t.title.as_deref(), t.due_date.as_deref(), t.status.as_str()),
            (Some("打印简历"), Some("2026-10-04"), "pending")
        );
        assert_eq!(*port.tips.lock().unwrap(), [CandidateKind::Todo]);
    }

    #[tokio::test]
    async fn commits_are_buffered_into_sentences() {
        let (s, port, _, clock) = rig(0.95, vec![Ok(GROUP.into())]);
        let commit = |t: &str| {
            HubEvent::Xqp(Arc::new(Up::Commit {
                ts: 0,
                seq: None,
                chars: 1,
                keystrokes: 1,
                cand_pos: 0,
                src: "core".into(),
                text: Some(t.into()),
                truncated: None,
            }))
        };
        for piece in ["好的，", "周五下午", "三点", "在实验楼", "开组会"] {
            s.on_event(&commit(piece));
        }
        s.on_tick();
        assert!(port.tips.lock().unwrap().is_empty(), "还没停够 3 秒");
        clock.advance_ms(3_000);
        s.on_tick();
        for _ in 0..50 {
            if !port.ready.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(port.ready.lock().unwrap().len(), 1);
        assert!(s.state.lock().unwrap().text.is_empty());
        // 句末标点立刻成句，后面的留在缓冲里
        assert_eq!(s.push("明天开会。后").len(), 1);
        assert_eq!(&**s.state.lock().unwrap().text, "后");
    }
}
