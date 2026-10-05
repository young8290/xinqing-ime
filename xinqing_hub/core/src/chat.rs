//! AI 对话服务（03 第 2.2 节 `chat` 任务，C-07）与对话里的危机安全（C-08，05 第 4 节）。
//!
//! 发一条消息：写库 → 本地词表（≤ 50 ms，先于大模型，命中立即显示求助卡片）→ 后台任务里**同时**发 Q-CRISIS
//! 与流式调用大模型 → 每段增量推 `chat:delta` → 结束后做 V3 / V4 / V6 校验、写库、推 `chat:done`。
//! 任一通道命中：本会话进入安全模式（P-CHAT-SAFE），写 `safety_log`（只有时间和通道）。
//! 安全模式下大模型不可用时用本地固定回应（FR-SAF-03 第 2 条）。实现上的取舍见 ADR 0018。
//!
//! 与外壳之间只通过 [`ChatPort`]，本模块不依赖 Tauri（ADR 0007）。

use std::collections::HashMap;
use std::ops::Deref;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{Local, TimeZone};
use futures::StreamExt;
use tokio::sync::watch;

use crate::domain::chat::{
    self, ChatCopy, ChatMode, ChatPrompts, CrisisChannel, JEV_CRISIS_THRESHOLD, ReplyCheck,
    SafeMode, StatePoint, Turn,
};
use crate::domain::comfort::Style;
use crate::domain::safety::CrisisLexicon;
use crate::domain::validate::BannedWords;
use crate::infra::clock::Clock;
use crate::infra::gateway::{
    AiError, AiGateway, Answer, CompleteRequest, JudgeRequest, Message, Question, Scenario,
};
use crate::infra::store::{Db, NewMessage, StoreError};

/// 兜底的总时长：网关自己有首字 15 秒、总长 90 秒的超时（FR-CHT-04 第 2 条），这里多留 5 秒以防万一。
const TOTAL_LIMIT: Duration = Duration::from_secs(95);
/// 回复已经推完、Q-CRISIS 还没回来时，最多再等这么久写 `safety_log`。
const JEV_GRACE: Duration = Duration::from_secs(10);

/// 外壳提供给对话服务的能力。
pub trait ChatPort: Send + Sync {
    /// 写连接（外壳的 `DbWriter::lock`，对话里读写交错，都在写连接上做）。每次取用都很短，不跨 `.await` 持有。
    fn db(&self) -> Box<dyn Deref<Target = Db> + '_>;
    /// 设置 `care.style`（FR-CHT-10）。
    fn style(&self) -> Style;
    /// 是否已同意 ④（今日状态摘要发给大模型，FR-CHT-05）。
    fn summary_allowed(&self) -> bool;
    /// 推送 `chat:delta` / `chat:done` / `chat:error`。
    fn emit(&self, ev: ChatEvent);
    /// 危机识别命中：发布 `HubEvent::Safety`、推送 `safety:triggered`，对话窗口没开时打开它（FR-SAF-02）。
    fn safety(&self, session_id: i64);
    /// 运行日志，只会传入不含用户数据的内容（NFR-LOG）。
    fn note(&self, _msg: &str) {}
}

/// 推给对话窗口的事件（10 第 5.2 节）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatEvent {
    Delta {
        request_id: u32,
        text: String,
    },
    /// 回复结束。`text` 是最终写库、该显示的全文：校验替换或截断过时与流式拼出来的不同，界面以它为准。
    /// 用户点了“停止生成”且一个字都还没有时 `message_id` 为 `None`。
    Done {
        request_id: u32,
        session_id: i64,
        message_id: Option<i64>,
        text: String,
        ai_generated: bool,
        stopped: bool,
    },
    /// 没有拿到回复。用户消息已保存，界面显示提示和“重试”（FR-CHT-04 第 2 条）。
    Error {
        request_id: u32,
        session_id: i64,
        reason: ChatFailure,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum ChatFailure {
    /// 超时、模型不可用、网络等：“晴晴有点卡，稍后再试”
    Stuck,
    /// 今日对话额度用完（FR-AIG-07）：“今天聊了很多啦，明天继续？”
    DailyCap,
}

impl ChatFailure {
    fn from_ai(e: &AiError) -> Self {
        match e {
            AiError::BudgetExceeded => ChatFailure::DailyCap,
            _ => ChatFailure::Stuck,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ChatError {
    #[error("消息为空")]
    Empty,
    #[error("消息超过 2000 字")]
    TooLong,
    #[error("这个会话正在回复")]
    Busy,
    #[error("会话不存在")]
    NoSession,
    #[error("没有要重试的消息")]
    NothingToRetry,
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// `chat_send` / `chat_retry` 的返回。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sent {
    pub request_id: u32,
    pub session_id: i64,
    /// 刚写入的用户消息；重试时为 `None`
    pub user_message_id: Option<i64>,
    /// 这次开了新会话（没传会话、或上一条消息已超过 6 小时）
    pub new_session: bool,
    /// 本地词表命中，界面应立即显示求助卡片（`safety:triggered` 也会推）
    pub safety: bool,
    /// 用户明确要求记住的内容，界面先请用户确认，确认后才调 `memory_add`（FR-CHT-07 第 1 条）；
    /// 求助卡片出现时不问
    pub memory_candidate: Option<String>,
}

struct Running {
    session_id: i64,
    stop: watch::Sender<bool>,
}

pub struct ChatService {
    port: Arc<dyn ChatPort>,
    gateway: Arc<dyn AiGateway>,
    clock: Arc<dyn Clock>,
    prompts: ChatPrompts,
    copy: ChatCopy,
    banned: Arc<BannedWords>,
    lex: Arc<CrisisLexicon>,
    next_id: AtomicU32,
    running: Mutex<HashMap<u32, Running>>,
}

impl ChatService {
    pub fn new(
        port: Arc<dyn ChatPort>,
        gateway: Arc<dyn AiGateway>,
        clock: Arc<dyn Clock>,
        prompts: ChatPrompts,
        copy: ChatCopy,
        banned: Arc<BannedWords>,
        lex: Arc<CrisisLexicon>,
    ) -> Self {
        Self {
            port,
            gateway,
            clock,
            prompts,
            copy,
            banned,
            lex,
            next_id: AtomicU32::new(1),
            running: Mutex::new(HashMap::new()),
        }
    }

    fn running(&self) -> std::sync::MutexGuard<'_, HashMap<u32, Running>> {
        self.running.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 发一条消息（`chat_send`）。须在 tokio 运行时里调用：回复在后台任务里生成。
    /// `chat_mode` 不为空时先把（可能是新开的）会话切到这种对话方式，再生成回复（FR-CHT-06，ADR 0022）。
    pub fn send(
        self: &Arc<Self>,
        session_id: Option<i64>,
        text: &str,
        chat_mode: Option<ChatMode>,
    ) -> Result<Sent, ChatError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(ChatError::Empty);
        }
        if text.chars().count() > chat::MAX_INPUT_CHARS {
            return Err(ChatError::TooLong);
        }
        let now = self.clock.now_ms();
        let (session_id, new_session, mode, user_message_id) = {
            let db = self.port.db();
            let current = match session_id {
                Some(id) => Some(db.chat_session(id)?.ok_or(ChatError::NoSession)?),
                None => None,
            };
            let (id, new, mode) = match current {
                Some(s) if chat::continues(Some(s.last_ts), now) => (s.id, false, s.safe_mode),
                _ => (
                    db.chat_session_create(&chat::title_of(text), now)?,
                    true,
                    SafeMode::Off,
                ),
            };
            if self.busy(id) {
                return Err(ChatError::Busy);
            }
            if let Some(m) = chat_mode {
                db.chat_set_mode(id, m)?;
            }
            let mid = db.chat_message_insert(&NewMessage {
                session_id: id,
                role: "user",
                content: text,
                ts: now,
                ai_generated: false,
                model: None,
                prompt_ver: None,
            })?;
            (id, new, mode, mid)
        };
        // 本地词表先于大模型（FR-SAF-01），命中立即显示求助卡片，不等 Jev 和大模型（17 第 2.7 节）
        let local = chat::lexicon_hit(text, &self.lex, mode);
        if local {
            self.enter_safe_mode(session_id, mode);
        }
        let request_id = self.start(session_id, text.to_string(), Some(local));
        Ok(Sent {
            request_id,
            session_id,
            user_message_id: Some(user_message_id),
            new_session,
            safety: local,
            memory_candidate: (!local).then(|| chat::memory_request(text)).flatten(),
        })
    }

    /// 切换会话的对话方式（快捷指令“我只是想吐槽”“帮我理一理”，回到平常用 `Normal`）。
    /// 从下一次回复起生效；正在生成的回复不受影响。
    pub fn set_mode(&self, session_id: i64, mode: ChatMode) -> Result<(), ChatError> {
        if self.port.db().chat_set_mode(session_id, mode)? {
            Ok(())
        } else {
            Err(ChatError::NoSession)
        }
    }

    /// 为会话最后一条用户消息重新生成回复（“重试”，ADR 0018 第 4 条）。不再写用户消息，也不再做危机识别。
    pub fn retry(self: &Arc<Self>, session_id: i64) -> Result<Sent, ChatError> {
        let last = {
            let db = self.port.db();
            db.chat_session(session_id)?.ok_or(ChatError::NoSession)?;
            db.chat_messages(session_id)?
                .into_iter()
                .rev()
                .find(|m| m.role != "system_notice")
        };
        let Some(last) = last.filter(|m| m.role == "user") else {
            return Err(ChatError::NothingToRetry);
        };
        if self.busy(session_id) {
            return Err(ChatError::Busy);
        }
        let request_id = self.start(session_id, last.content, None);
        Ok(Sent {
            request_id,
            session_id,
            user_message_id: None,
            new_session: false,
            safety: false,
            memory_candidate: None,
        })
    }

    /// “停止生成”（FR-CHT-04 第 3 条）：已生成的部分照常保存。
    pub fn stop(&self, request_id: u32) {
        if let Some(r) = self.running().get(&request_id) {
            let _ = r.stop.send(true);
        }
    }

    /// “我说的不是这个意思”（FR-SAF-06）：回到普通模式，本会话词表阈值提高；求助信息仍折叠显示一行。
    pub fn dismiss_safety(&self, session_id: i64) -> Result<(), ChatError> {
        let db = self.port.db();
        let s = db.chat_session(session_id)?.ok_or(ChatError::NoSession)?;
        if s.safe_mode == SafeMode::On {
            db.chat_set_safe_mode(session_id, SafeMode::Dismissed)?;
        }
        Ok(())
    }

    /// 复制一条消息（`chat_copy`）：AI 回复附加“（内容由 AI 生成）”。消息不存在时为 `None`。
    pub fn copy_text(&self, message_id: i64) -> Result<Option<String>, ChatError> {
        let m = self.port.db().chat_message(message_id)?;
        Ok(m.map(|m| self.copy.for_copy(&m.content, m.ai_generated)))
    }

    fn busy(&self, session_id: i64) -> bool {
        self.running().values().any(|r| r.session_id == session_id)
    }

    fn enter_safe_mode(&self, session_id: i64, before: SafeMode) {
        if before != SafeMode::On
            && let Err(e) = self.port.db().chat_set_safe_mode(session_id, SafeMode::On)
        {
            self.port.note(&format!("写入安全模式失败：{e}"));
        }
        self.port.safety(session_id);
    }

    /// `crisis` 是本地词表的结果；`None` 表示不做危机识别（重试）。
    fn start(self: &Arc<Self>, session_id: i64, text: String, crisis: Option<bool>) -> u32 {
        let request_id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (stop, stopped) = watch::channel(false);
        self.running()
            .insert(request_id, Running { session_id, stop });
        let me = self.clone();
        tokio::spawn(async move {
            me.generate(request_id, session_id, text, crisis, stopped)
                .await;
            me.running().remove(&request_id);
        });
        request_id
    }

    /// 本次请求的系统提示词与上下文（FR-CHT-05）。
    fn messages(
        &self,
        session_id: i64,
        safe: bool,
        mode: ChatMode,
    ) -> Result<Vec<Message>, StoreError> {
        let style = self.port.style();
        let summary = if !safe && self.port.summary_allowed() {
            self.today_summary()?
        } else {
            None
        };
        let db = self.port.db();
        let memories = if safe {
            Vec::new()
        } else {
            db.memories_recent(chat::MEMORY_ITEMS)?
        };
        let history: Vec<Turn> = db
            .chat_messages(session_id)?
            .into_iter()
            .filter(|m| m.role == "user" || m.role == "assistant")
            .map(|m| Turn {
                user: m.role == "user",
                content: m.content,
            })
            .collect();
        drop(db);
        let mut out = vec![Message {
            role: "system".into(),
            content: self
                .prompts
                .system(safe, style, mode, summary.as_deref(), &memories),
        }];
        out.extend(chat::trim_context(&history).iter().map(|t| Message {
            role: if t.user { "user" } else { "assistant" }.into(),
            content: t.content.clone(),
        }));
        Ok(out)
    }

    fn today_summary(&self) -> Result<Option<String>, StoreError> {
        let now = self.clock.now();
        let midnight = now
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .and_then(|t| Local.from_local_datetime(&t).earliest())
            .map_or(now.timestamp_millis(), |t| t.timestamp_millis());
        let db = self.port.db();
        let points: Vec<StatePoint> = db
            .shown_states_since(midnight)?
            .into_iter()
            .filter_map(|(ts, state)| {
                Local
                    .timestamp_millis_opt(ts)
                    .single()
                    .map(|local| StatePoint { local, state })
            })
            .collect();
        let typing_min = (db.active_ms_since(midnight)? / 60_000) as u32;
        Ok(chat::today_summary(&points, typing_min))
    }

    async fn judge_crisis(gateway: Arc<dyn AiGateway>, text: String) -> bool {
        let mut state = serde_json::Map::new();
        state.insert("message".into(), text.into());
        let req = JudgeRequest {
            questions: vec![Question::Crisis],
            state,
        };
        match gateway.judge(req).await {
            Ok(r) => matches!(
                r.answers.get(&Question::Crisis),
                Some(Answer::Noul { p }) if *p >= JEV_CRISIS_THRESHOLD
            ),
            // Jev 不可用时只靠本地词表（FR-SAF-04）
            Err(_) => false,
        }
    }

    async fn generate(
        &self,
        request_id: u32,
        session_id: i64,
        text: String,
        crisis: Option<bool>,
        mut stopped: watch::Receiver<bool>,
    ) {
        // 重试不再做危机识别：同一条消息在发送时已经查过。Q-CRISIS 与大模型同时进行，Jev 的结果在流式过程中到达
        let local = crisis.unwrap_or(false);
        let mut jev = Box::pin(Self::judge_crisis(self.gateway.clone(), text));
        let mut jev_hit: Option<bool> = if crisis.is_some() { None } else { Some(false) };

        let session = self.port.db().chat_session(session_id).ok().flatten();
        let safe = session
            .as_ref()
            .is_some_and(|s| s.safe_mode == SafeMode::On);
        let mode = session.map_or(ChatMode::Normal, |s| s.mode);
        let messages = match self.messages(session_id, safe, mode) {
            Ok(m) => m,
            Err(e) => {
                self.port.note(&format!("组装对话上下文失败：{e}"));
                self.port.emit(ChatEvent::Error {
                    request_id,
                    session_id,
                    reason: ChatFailure::Stuck,
                });
                return;
            }
        };
        let prompt_ver = self.prompts.ver(safe, mode);
        let req = CompleteRequest {
            scenario: Scenario::Chat,
            prompt_ver: prompt_ver.clone(),
            messages,
        };

        let deadline = tokio::time::Instant::now() + TOTAL_LIMIT;
        let mut reply = String::new();
        let mut failure: Option<AiError> = None;
        let mut user_stopped = false;
        let opened = tokio::select! {
            r = self.gateway.stream(req) => Some(r),
            _ = stopped.wait_for(|s| *s) => None,
        };
        match opened {
            None => user_stopped = true,
            Some(Err(e)) => failure = Some(e),
            Some(Ok(mut stream)) => loop {
                tokio::select! {
                    biased;
                    _ = stopped.wait_for(|s| *s) => {
                        user_stopped = true;
                        break;
                    }
                    hit = &mut jev, if jev_hit.is_none() => {
                        jev_hit = Some(hit);
                        self.on_jev(session_id, local, hit);
                    }
                    _ = tokio::time::sleep_until(deadline) => {
                        failure = Some(AiError::Timeout);
                        break;
                    }
                    d = stream.next() => match d {
                        Some(Ok(d)) => {
                            if d.text.is_empty() {
                                continue;
                            }
                            reply.push_str(&d.text);
                            self.port.emit(ChatEvent::Delta { request_id, text: d.text });
                        }
                        Some(Err(e)) => {
                            failure = Some(e);
                            break;
                        }
                        None => break,
                    },
                }
            },
        }

        self.finish(Finish {
            request_id,
            session_id,
            reply,
            failure,
            user_stopped,
            safe,
            prompt_ver: &prompt_ver,
        });

        if jev_hit.is_none() {
            let hit = tokio::time::timeout(JEV_GRACE, jev).await.unwrap_or(false);
            self.on_jev(session_id, local, hit);
        }
    }

    /// Q-CRISIS 有结果了：写 `safety_log`，Jev 命中而词表没命中时这时才进入安全模式（ADR 0018 第 1 条）。
    fn on_jev(&self, session_id: i64, local: bool, jev: bool) {
        let Some(channel) = CrisisChannel::combine(local, jev) else {
            return;
        };
        if jev && !local {
            let before = self
                .port
                .db()
                .chat_session(session_id)
                .ok()
                .flatten()
                .map_or(SafeMode::Off, |s| s.safe_mode);
            self.enter_safe_mode(session_id, before);
        }
        if let Err(e) = self
            .port
            .db()
            .safety_log(self.clock.now_ms(), channel.as_str())
        {
            self.port.note(&format!("写入 safety_log 失败：{e}"));
        }
    }

    fn finish(&self, f: Finish<'_>) {
        let Finish {
            request_id,
            session_id,
            reply,
            failure,
            user_stopped,
            safe,
            prompt_ver,
        } = f;
        // 中途出错：已推的半截丢掉，界面收到 error 后移除（ADR 0018 第 5 条）
        let (text, ai_generated) = if user_stopped {
            if reply.trim().is_empty() {
                self.port.emit(ChatEvent::Done {
                    request_id,
                    session_id,
                    message_id: None,
                    text: String::new(),
                    ai_generated: false,
                    stopped: true,
                });
                return;
            }
            self.checked(&reply)
        } else if failure.is_some() || reply.trim().is_empty() {
            if safe {
                // 安全模式下大模型不可用：本地固定回应（FR-SAF-03 第 2 条）
                (self.copy.fallback_reply.clone(), false)
            } else {
                let reason = failure
                    .as_ref()
                    .map_or(ChatFailure::Stuck, ChatFailure::from_ai);
                self.port.emit(ChatEvent::Error {
                    request_id,
                    session_id,
                    reason,
                });
                return;
            }
        } else {
            self.checked(&reply)
        };
        let saved = self.port.db().chat_message_insert(&NewMessage {
            session_id,
            role: "assistant",
            content: &text,
            ts: self.clock.now_ms(),
            ai_generated,
            model: None,
            prompt_ver: ai_generated.then_some(prompt_ver),
        });
        let message_id = match saved {
            Ok(id) => Some(id),
            Err(e) => {
                self.port.note(&format!("保存对话回复失败：{e}"));
                None
            }
        };
        self.port.emit(ChatEvent::Done {
            request_id,
            session_id,
            message_id,
            text,
            ai_generated,
            stopped: user_stopped,
        });
    }

    /// 流式结束后的 V3 / V4 / V6（08 第 5 节）。返回要显示的全文和是否由 AI 生成。
    fn checked(&self, reply: &str) -> (String, bool) {
        match chat::check_reply(reply, &self.banned, &self.lex) {
            ReplyCheck::Ok(t) => (t, true),
            ReplyCheck::Unsafe => {
                self.port
                    .note("对话回复命中危机词表的方法类条目（V6），已换成固定回应");
                (self.copy.fallback_reply.clone(), false)
            }
            ReplyCheck::Banned => {
                self.port.note("对话回复命中禁用词（V4），已换成固定文案");
                (self.copy.replaced.clone(), false)
            }
        }
    }
}

struct Finish<'a> {
    request_id: u32,
    session_id: i64,
    reply: String,
    failure: Option<AiError>,
    user_stopped: bool,
    safe: bool,
    prompt_ver: &'a str,
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::path::PathBuf;

    use async_trait::async_trait;
    use chrono::NaiveDate;
    use futures::stream::{self, BoxStream};

    use super::*;
    use crate::infra::clock::ManualClock;
    use crate::infra::gateway::{CompleteResponse, Delta, GatewayHealth, JudgeResponse};
    use crate::infra::templates::TemplateDirs;

    fn dirs() -> TemplateDirs {
        TemplateDirs::factory_only(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../hub_templates"),
        )
    }

    type Script = Result<Vec<Option<&'static str>>, AiError>;

    /// 每次 `stream` 取一个剧本：`Ok(片段)` 逐段吐出，`Err` 表示打开就失败；片段里的 `None` 表示中途出错。
    #[derive(Default)]
    struct FakeAi {
        scripts: Mutex<VecDeque<Script>>,
        crisis_p: Mutex<Option<f64>>,
        requests: Mutex<Vec<CompleteRequest>>,
        judged: AtomicU32,
        /// 吐出第一段后挂起，直到被停止
        hang: Mutex<bool>,
    }

    #[async_trait]
    impl AiGateway for FakeAi {
        async fn judge(&self, _req: JudgeRequest) -> Result<JudgeResponse, AiError> {
            self.judged.fetch_add(1, Ordering::Relaxed);
            let p = (*self.crisis_p.lock().unwrap()).ok_or(AiError::ModelUnavailable)?;
            Ok(JudgeResponse {
                model: "jev".into(),
                latency_ms: 1,
                answers: [(Question::Crisis, Answer::Noul { p })].into(),
            })
        }
        async fn complete(&self, _req: CompleteRequest) -> Result<CompleteResponse, AiError> {
            Err(AiError::ModelUnavailable)
        }
        async fn stream(
            &self,
            req: CompleteRequest,
        ) -> Result<BoxStream<'static, Result<Delta, AiError>>, AiError> {
            self.requests.lock().unwrap().push(req);
            let script = self
                .scripts
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Err(AiError::ModelUnavailable))?;
            let items: Vec<Result<Delta, AiError>> = script
                .into_iter()
                .map(|p| p.map(|t| Delta { text: t.into() }).ok_or(AiError::Upstream))
                .collect();
            let s = stream::iter(items);
            if *self.hang.lock().unwrap() {
                return Ok(s.chain(stream::pending()).boxed());
            }
            Ok(s.boxed())
        }
        fn health(&self) -> GatewayHealth {
            GatewayHealth::default()
        }
    }

    struct FakePort {
        db: Mutex<Db>,
        summary: bool,
        events: Mutex<Vec<ChatEvent>>,
        safety: Mutex<Vec<i64>>,
    }

    impl ChatPort for FakePort {
        fn db(&self) -> Box<dyn Deref<Target = Db> + '_> {
            Box::new(self.db.lock().unwrap())
        }
        fn style(&self) -> Style {
            Style::Gentle
        }
        fn summary_allowed(&self) -> bool {
            self.summary
        }
        fn emit(&self, ev: ChatEvent) {
            self.events.lock().unwrap().push(ev);
        }
        fn safety(&self, session_id: i64) {
            self.safety.lock().unwrap().push(session_id);
        }
    }

    struct Env {
        svc: Arc<ChatService>,
        ai: Arc<FakeAi>,
        port: Arc<FakePort>,
        clock: Arc<ManualClock>,
    }

    fn env(summary: bool) -> Env {
        let naive = NaiveDate::from_ymd_opt(2026, 3, 10)
            .unwrap()
            .and_hms_opt(15, 0, 0)
            .unwrap();
        let clock = Arc::new(ManualClock::new(
            Local.from_local_datetime(&naive).earliest().unwrap(),
        ));
        let ai = Arc::new(FakeAi::default());
        let port = Arc::new(FakePort {
            db: Mutex::new(Db::open_in_memory().unwrap()),
            summary,
            events: Mutex::new(Vec::new()),
            safety: Mutex::new(Vec::new()),
        });
        let svc = Arc::new(ChatService::new(
            port.clone(),
            ai.clone(),
            clock.clone(),
            ChatPrompts::load(&dirs()).unwrap(),
            ChatCopy::load(&dirs()).unwrap(),
            Arc::new(BannedWords::load(&dirs()).unwrap()),
            Arc::new(CrisisLexicon::load(&dirs()).unwrap()),
        ));
        Env {
            svc,
            ai,
            port,
            clock,
        }
    }

    impl Env {
        fn script(&self, s: Script) {
            self.ai.scripts.lock().unwrap().push_back(s);
        }

        /// 等后台任务做完（含等 Q-CRISIS）。
        async fn settle(&self) {
            for _ in 0..200 {
                if self.svc.running().is_empty() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            panic!("后台任务没有结束");
        }

        fn last_event(&self) -> ChatEvent {
            self.port.events.lock().unwrap().last().cloned().unwrap()
        }

        fn mode(&self, session: i64) -> SafeMode {
            self.port
                .db
                .lock()
                .unwrap()
                .chat_session(session)
                .unwrap()
                .unwrap()
                .safe_mode
        }

        fn safety_log(&self) -> Vec<String> {
            let db = self.port.db.lock().unwrap();
            let mut stmt = db
                .conn()
                .prepare("SELECT channel FROM safety_log ORDER BY id")
                .unwrap();
            stmt.query_map([], |r| r.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        }
    }

    #[tokio::test]
    async fn streams_reply_and_saves_session() {
        let e = env(false);
        *e.ai.crisis_p.lock().unwrap() = Some(0.01);
        e.script(Ok(vec![Some("听起来"), Some("挺累的。")]));
        let sent = e
            .svc
            .send(None, "  今天考试没考好，心里有点难受  ", None)
            .unwrap();
        assert!(sent.new_session && !sent.safety);
        e.settle().await;
        let events = e.port.events.lock().unwrap().clone();
        assert_eq!(
            events[0],
            ChatEvent::Delta {
                request_id: sent.request_id,
                text: "听起来".into()
            }
        );
        let ChatEvent::Done {
            message_id: Some(mid),
            text,
            ai_generated: true,
            stopped: false,
            ..
        } = events.last().unwrap().clone()
        else {
            panic!("{events:?}")
        };
        assert_eq!(text, "听起来挺累的。");
        let db = e.port.db.lock().unwrap();
        let s = db.chat_session(sent.session_id).unwrap().unwrap();
        assert_eq!(s.title, "今天考试没考好，心里有点");
        let msgs = db.chat_messages(sent.session_id).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].content, "今天考试没考好，心里有点难受");
        assert_eq!(msgs[1].id, mid);
        drop(db);
        // 系统提示词是 P-CHAT，最后一条是这次的用户消息
        let req = &e.ai.requests.lock().unwrap()[0];
        assert_eq!(req.scenario, Scenario::Chat);
        assert_eq!(req.prompt_ver, "P-CHAT v1");
        assert_eq!(req.messages[0].role, "system");
        assert!(
            !req.messages[0].content.contains("今天的大致状态"),
            "未同意 ④"
        );
        assert_eq!(req.messages.last().unwrap().role, "user");
        assert!(e.safety_log().is_empty());
    }

    #[tokio::test]
    async fn continues_within_six_hours_then_starts_new() {
        let e = env(true);
        e.script(Ok(vec![Some("嗯嗯。")]));
        e.script(Ok(vec![Some("好呀。")]));
        e.script(Ok(vec![Some("你好。")]));
        let a = e.svc.send(None, "第一句", None).unwrap();
        e.settle().await;
        e.clock.advance_ms(60_000);
        let b = e.svc.send(Some(a.session_id), "第二句", None).unwrap();
        e.settle().await;
        assert_eq!(b.session_id, a.session_id);
        assert!(!b.new_session);
        // 第二次带上了第一轮
        let req = e.ai.requests.lock().unwrap()[1].clone();
        let roles: Vec<&str> = req.messages.iter().map(|m| m.role.as_str()).collect();
        assert_eq!(roles, ["system", "user", "assistant", "user"]);
        e.clock.advance_ms(chat::NEW_SESSION_GAP_MS + 1);
        let c = e.svc.send(Some(a.session_id), "第三句", None).unwrap();
        e.settle().await;
        assert!(c.new_session);
        assert_ne!(c.session_id, a.session_id);
    }

    #[tokio::test]
    async fn explicit_remember_request_is_offered_for_confirmation_not_saved() {
        let e = env(true);
        e.script(Ok(vec![Some("好的。")]));
        let sent = e
            .svc
            .send(None, "帮我记一下：下周三考英语", None)
            .unwrap();
        e.settle().await;
        assert_eq!(sent.memory_candidate.as_deref(), Some("下周三考英语"));
        // 只是给界面去问，记忆表里还没有（FR-CHT-07 第 4 条：不自动保存）
        assert!(e.port.db.lock().unwrap().memories_list().unwrap().is_empty());
        // 求助卡片出现时不问
        let sent = e.svc.send(None, "记住，我真的不想活了", None).unwrap();
        assert!(sent.safety);
        assert_eq!(sent.memory_candidate, None);
    }

    #[tokio::test]
    async fn chat_mode_sticks_to_session_until_switched_back() {
        let e = env(true);
        for _ in 0..3 {
            e.script(Ok(vec![Some("嗯，我在听。")]));
        }
        let a = e
            .svc
            .send(None, "今天被说了一顿", Some(ChatMode::Vent))
            .unwrap();
        e.settle().await;
        e.clock.advance_ms(60_000);
        // 不带 mode 时沿用会话里存的对话方式
        e.svc.send(Some(a.session_id), "就是很委屈", None).unwrap();
        e.settle().await;
        e.svc.set_mode(a.session_id, ChatMode::Normal).unwrap();
        e.clock.advance_ms(60_000);
        e.svc.send(Some(a.session_id), "好多了", None).unwrap();
        e.settle().await;
        let reqs = e.ai.requests.lock().unwrap().clone();
        for req in &reqs[..2] {
            assert_eq!(req.prompt_ver, "P-CHAT v1 + P-CHAT-VENT v1");
            assert!(req.messages[0].content.contains("只倾听"));
        }
        assert_eq!(reqs[2].prompt_ver, "P-CHAT v1");
        assert!(!reqs[2].messages[0].content.contains("只倾听"));
        assert!(matches!(
            e.svc.set_mode(99, ChatMode::Vent),
            Err(ChatError::NoSession)
        ));
    }

    #[tokio::test]
    async fn summary_only_with_consent() {
        let e = env(true);
        {
            let db = e.port.db.lock().unwrap();
            let ts = e.clock.now_ms() - 3_600_000;
            db.conn()
                .execute(
                    "INSERT INTO mood_state (ts, state, shown_state, source) VALUES (?1, 'fluent', 'fluent', 'rule')",
                    [ts],
                )
                .unwrap();
        }
        e.script(Ok(vec![Some("嗯。")]));
        e.svc.send(None, "在吗", None).unwrap();
        e.settle().await;
        let sys = e.ai.requests.lock().unwrap()[0].messages[0].content.clone();
        assert!(sys.contains("用户今天的大致状态：下午平稳"), "{sys}");
    }

    #[tokio::test]
    async fn lexicon_hit_shows_card_at_once_and_uses_safe_prompt() {
        // TC-SAF：本地词表命中 → 立即触发、安全模式、safety_log 只记通道
        let e = env(false);
        *e.ai.crisis_p.lock().unwrap() = Some(0.9);
        e.script(Ok(vec![Some("谢谢你告诉我。你现在安全吗？")]));
        let sent = e.svc.send(None, "我真的不想活了", None).unwrap();
        assert!(sent.safety);
        assert_eq!(e.port.safety.lock().unwrap().as_slice(), [sent.session_id]);
        assert_eq!(e.mode(sent.session_id), SafeMode::On);
        e.settle().await;
        assert_eq!(
            e.ai.requests.lock().unwrap()[0].prompt_ver,
            "P-CHAT-SAFE v1"
        );
        assert_eq!(e.safety_log(), ["both"]);
        // 内容不进 safety_log
        let db = e.port.db.lock().unwrap();
        let cols: i64 = db
            .conn()
            .query_row(
                "SELECT count(*) FROM pragma_table_info('safety_log')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(cols, 3);
    }

    #[tokio::test]
    async fn jev_only_hit_enters_safe_mode_for_next_message() {
        let e = env(false);
        *e.ai.crisis_p.lock().unwrap() = Some(0.6);
        e.script(Ok(vec![Some("我在听。")]));
        e.script(Ok(vec![Some("你现在安全吗？")]));
        let sent = e.svc.send(None, "感觉一切都没有意义", None).unwrap();
        assert!(!sent.safety);
        e.settle().await;
        assert_eq!(e.safety_log(), ["jev"]);
        assert_eq!(e.mode(sent.session_id), SafeMode::On);
        assert_eq!(e.port.safety.lock().unwrap().len(), 1);
        *e.ai.crisis_p.lock().unwrap() = Some(0.1);
        e.svc.send(Some(sent.session_id), "嗯", None).unwrap();
        e.settle().await;
        assert_eq!(
            e.ai.requests.lock().unwrap()[1].prompt_ver,
            "P-CHAT-SAFE v1"
        );
    }

    #[tokio::test]
    async fn safe_mode_offline_uses_fixed_reply() {
        // FR-SAF-03 第 2 条、FR-SAF-04：断网时求助卡片与固定回应照常
        let e = env(false);
        e.script(Err(AiError::Network));
        let sent = e.svc.send(None, "我真的不想活了", None).unwrap();
        e.settle().await;
        let ChatEvent::Done {
            text, ai_generated, ..
        } = e.last_event()
        else {
            panic!()
        };
        assert!(text.contains("12356"));
        assert!(!ai_generated, "固定回应是人工撰写的，不标 AI 生成");
        assert_eq!(e.safety_log(), ["lexicon"], "Jev 不可用时只记词表");
        assert_eq!(e.mode(sent.session_id), SafeMode::On);
    }

    #[tokio::test]
    async fn failure_keeps_user_message_and_retry_regenerates() {
        let e = env(false);
        e.script(Err(AiError::Timeout));
        let sent = e.svc.send(None, "你好", None).unwrap();
        e.settle().await;
        assert_eq!(
            e.last_event(),
            ChatEvent::Error {
                request_id: sent.request_id,
                session_id: sent.session_id,
                reason: ChatFailure::Stuck
            }
        );
        assert_eq!(
            e.port
                .db
                .lock()
                .unwrap()
                .chat_messages(sent.session_id)
                .unwrap()
                .len(),
            1,
            "用户消息保留"
        );
        let judged = e.ai.judged.load(Ordering::Relaxed);
        e.script(Ok(vec![Some("你好呀。")]));
        let again = e.svc.retry(sent.session_id).unwrap();
        assert!(again.user_message_id.is_none());
        e.settle().await;
        assert!(matches!(e.last_event(), ChatEvent::Done { .. }));
        assert_eq!(
            e.ai.judged.load(Ordering::Relaxed),
            judged,
            "重试不再查 Jev"
        );
        let msgs = e
            .port
            .db
            .lock()
            .unwrap()
            .chat_messages(sent.session_id)
            .unwrap();
        assert_eq!(msgs.len(), 2);
        assert!(matches!(
            e.svc.retry(sent.session_id),
            Err(ChatError::NothingToRetry)
        ));
    }

    #[tokio::test]
    async fn mid_stream_error_and_daily_cap() {
        let e = env(false);
        e.script(Ok(vec![Some("半截"), None]));
        e.svc.send(None, "你好", None).unwrap();
        e.settle().await;
        assert!(matches!(
            e.last_event(),
            ChatEvent::Error {
                reason: ChatFailure::Stuck,
                ..
            }
        ));
        e.script(Err(AiError::BudgetExceeded));
        e.svc.send(None, "再来", None).unwrap();
        e.settle().await;
        assert!(matches!(
            e.last_event(),
            ChatEvent::Error {
                reason: ChatFailure::DailyCap,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn unsafe_or_banned_reply_is_replaced() {
        let e = env(false);
        e.script(Ok(vec![Some("可以试试"), Some("割腕")]));
        e.script(Ok(vec![Some("你这是抑郁症")]));
        e.svc.send(None, "随便聊聊", None).unwrap();
        e.settle().await;
        let ChatEvent::Done {
            text, ai_generated, ..
        } = e.last_event()
        else {
            panic!()
        };
        assert!(text.contains("12356") && !ai_generated);
        e.svc.send(None, "再聊聊", None).unwrap();
        e.settle().await;
        let ChatEvent::Done { text, .. } = e.last_event() else {
            panic!()
        };
        assert_eq!(text, ChatCopy::load(&dirs()).unwrap().replaced);
    }

    #[tokio::test]
    async fn stop_keeps_partial_and_busy_session_rejects() {
        let e = env(false);
        *e.ai.hang.lock().unwrap() = true;
        e.script(Ok(vec![Some("我想想")]));
        let sent = e.svc.send(None, "讲个故事", None).unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(matches!(
            e.svc.send(Some(sent.session_id), "快点", None),
            Err(ChatError::Busy)
        ));
        e.svc.stop(sent.request_id);
        e.settle().await;
        let ChatEvent::Done {
            text,
            stopped: true,
            message_id: Some(_),
            ..
        } = e.last_event()
        else {
            panic!("{:?}", e.last_event())
        };
        assert_eq!(text, "我想想");
    }

    #[tokio::test]
    async fn dismiss_raises_threshold_but_keeps_record() {
        let e = env(false);
        e.script(Ok(vec![Some("你现在安全吗？")]));
        let sent = e.svc.send(None, "我真的不想活了", None).unwrap();
        e.settle().await;
        e.svc.dismiss_safety(sent.session_id).unwrap();
        assert_eq!(e.mode(sent.session_id), SafeMode::Dismissed);
        // 阈值提高后，同一句不再触发……
        e.script(Ok(vec![Some("嗯。")]));
        let again = e.svc.send(Some(sent.session_id), "活着好累", None).unwrap();
        assert!(!again.safety);
        e.settle().await;
        assert_eq!(e.ai.requests.lock().unwrap()[1].prompt_ver, "P-CHAT v1");
    }

    #[test]
    fn input_limits() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let e = env(false);
            assert!(matches!(
                e.svc.send(None, "   ", None),
                Err(ChatError::Empty)
            ));
            let long = "字".repeat(chat::MAX_INPUT_CHARS + 1);
            assert!(matches!(
                e.svc.send(None, &long, None),
                Err(ChatError::TooLong)
            ));
            assert!(matches!(
                e.svc.send(Some(99), "你好", None),
                Err(ChatError::NoSession)
            ));
        });
    }
}
