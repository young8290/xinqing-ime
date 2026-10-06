//! 情绪日记服务（C-10 第三部分，05 FR-DIA-01～03）与日记里的危机识别（FR-SAF-01/02）。
//!
//! 生成草稿：今日状态摘要（同意 ④ 时）+ 本次对话里用户说的话 → P-DIARY（网关 20 秒）→ V3 / V4 / V7 校验 → 草稿；
//! 大模型不可用、没通过校验、对话里有危机表达时给空白模板。草稿只在内存里，保存时才落库。
//! 保存：同步写 `diary`，按 FR-DIA-02 定来源；本地词表（先于一切出网）命中时立即开一段安全模式的对话并打开对话窗口，
//! 同意 ② 时另在后台发 Q-CRISIS，结果回来后写 `safety_log`（只有时间和通道）。
//!
//! 与外壳之间只通过 [`DiaryPort`]，本模块不依赖 Tauri（ADR 0007）。实现说明见 docs/adr/0031。

use std::collections::HashMap;
use std::ops::Deref;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use crate::chat::{ChatService, today_summary};
use crate::domain::chat::{self, CrisisChannel, SafeMode};
use crate::domain::diary::{self, DiaryCopy, DiaryPrompt, Source};
use crate::domain::safety::CrisisLexicon;
use crate::domain::validate::BannedWords;
use crate::infra::clock::Clock;
use crate::infra::gateway::{AiGateway, CompleteRequest, Message, Scenario};
use crate::infra::store::{Db, StoreError};

/// 草稿在内存里最多留这么久、这么多份（保存时用来判断改没改过）。
const DRAFT_TTL_MS: i64 = 24 * 60 * 60 * 1000;
const DRAFT_KEEP: usize = 20;

/// 外壳提供给日记服务的能力。
pub trait DiaryPort: Send + Sync {
    /// 写连接（外壳的 `DbWriter::lock`）。每次取用都很短，不跨 `.await` 持有。
    fn db(&self) -> Box<dyn Deref<Target = Db> + '_>;
    /// 是否已同意 ④（今日状态摘要发给大模型）。
    fn summary_allowed(&self) -> bool;
    /// 是否已同意 ②：日记内容只有这时才发给 Jev 做 Q-CRISIS（FR-SAF-01 第 2 条）。
    fn jev_allowed(&self) -> bool;
    /// 危机识别命中：发布 `HubEvent::Safety`、推送 `safety:triggered`，打开对话窗口（FR-SAF-02 第 2 条）。
    fn safety(&self, session_id: i64);
    /// 运行日志，只会传入不含用户数据的内容（NFR-LOG）。
    fn note(&self, _msg: &str) {}
}

/// `diary_generate` 的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    /// 保存时带回来；空白模板时为 `None`
    pub draft_id: Option<u32>,
    pub text: String,
    /// 为假时是空白模板
    pub ai_generated: bool,
}

/// `diary_save` 的参数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveReq {
    /// 改已有的一篇时是它的行号
    pub id: Option<i64>,
    /// 新写的一篇是从哪份草稿来的
    pub draft_id: Option<u32>,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Saved {
    pub id: i64,
    pub source: Source,
    /// 本地词表命中，界面应提示已打开求助卡片（对话窗口已打开）
    pub safety: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum DiaryError {
    #[error("日记是空的")]
    Empty,
    #[error("日记超过 5000 字")]
    TooLong,
    #[error("这篇日记不存在")]
    NotFound,
    #[error(transparent)]
    Store(#[from] StoreError),
}

pub struct DiaryService {
    port: Arc<dyn DiaryPort>,
    gateway: Arc<dyn AiGateway>,
    clock: Arc<dyn Clock>,
    prompt: DiaryPrompt,
    copy: DiaryCopy,
    lex: Arc<CrisisLexicon>,
    banned: Arc<BannedWords>,
    /// `draft_id → (生成时刻, 草稿原文)`
    drafts: Mutex<HashMap<u32, (i64, String)>>,
    next_draft: AtomicU32,
    #[cfg(test)]
    last_check: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl DiaryService {
    pub fn new(
        port: Arc<dyn DiaryPort>,
        gateway: Arc<dyn AiGateway>,
        clock: Arc<dyn Clock>,
        prompt: DiaryPrompt,
        copy: DiaryCopy,
        lex: Arc<CrisisLexicon>,
        banned: Arc<BannedWords>,
    ) -> Self {
        Self {
            port,
            gateway,
            clock,
            prompt,
            copy,
            lex,
            banned,
            drafts: Mutex::default(),
            next_draft: AtomicU32::new(1),
            #[cfg(test)]
            last_check: Mutex::default(),
        }
    }

    fn blank(&self) -> Draft {
        Draft {
            draft_id: None,
            text: self.copy.blank.clone(),
            ai_generated: false,
        }
    }

    /// 生成草稿（FR-DIA-01）。`session_id` 是从对话窗口“写成情绪日记”进来时的那段对话。
    pub async fn generate(&self, session_id: Option<i64>) -> Result<Draft, DiaryError> {
        let now = self.clock.now();
        let summary_allowed = self.port.summary_allowed();
        let (summary, digest) = {
            let db = self.port.db();
            let summary = if summary_allowed {
                today_summary(&db, now)?
            } else {
                None
            };
            let digest = match session_id {
                None => None,
                Some(id) => {
                    let s = db.chat_session(id)?.ok_or(DiaryError::NotFound)?;
                    // 安全模式下的对话不拿去写日记（P-DIARY 不是安全提示词）
                    if s.safe_mode == SafeMode::On {
                        return Ok(self.blank());
                    }
                    let said: Vec<String> = db
                        .chat_messages(id)?
                        .into_iter()
                        .filter(|m| m.role == "user")
                        .map(|m| m.content)
                        .collect();
                    diary::digest(&said)
                }
            };
            (summary, digest)
        };
        // 对话要点里有危机表达时一个字都不出网
        if digest
            .as_deref()
            .is_some_and(|d| chat::lexicon_hit(d, &self.lex, SafeMode::Off))
        {
            return Ok(self.blank());
        }
        // 没有任何可写的内容时不调用大模型（P-DIARY 要求“只写信息中出现过的内容”）
        if summary.is_none() && digest.is_none() {
            return Ok(self.blank());
        }
        let req = CompleteRequest {
            scenario: Scenario::Diary,
            prompt_ver: self.prompt.ver(),
            messages: vec![Message {
                role: "user".into(),
                content: self.prompt.render(summary.as_deref(), digest.as_deref()),
            }],
        };
        let text = match self.gateway.complete(req).await {
            Ok(r) => r.text,
            Err(e) => {
                self.port
                    .note(&format!("日记草稿：大模型不可用（{e}），给空白模板"));
                return Ok(self.blank());
            }
        };
        let text = match diary::check_draft(&text, &self.banned) {
            Ok(t) => t,
            Err(r) => {
                self.port.note(&format!(
                    "日记草稿：校验没通过（{}），给空白模板",
                    r.as_str()
                ));
                return Ok(self.blank());
            }
        };
        let id = self.next_draft.fetch_add(1, Ordering::Relaxed);
        let mut drafts = self.drafts.lock().unwrap_or_else(|e| e.into_inner());
        let now_ms = now.timestamp_millis();
        drafts.retain(|_, (ts, _)| now_ms - *ts < DRAFT_TTL_MS);
        if drafts.len() >= DRAFT_KEEP
            && let Some(oldest) = drafts
                .iter()
                .min_by_key(|(_, (ts, _))| *ts)
                .map(|(k, _)| *k)
        {
            drafts.remove(&oldest);
        }
        drafts.insert(id, (now_ms, text.clone()));
        Ok(Draft {
            draft_id: Some(id),
            text,
            ai_generated: true,
        })
    }

    /// 保存（新写或修改，FR-DIA-02/03）。返回时已落盘；Q-CRISIS 在后台进行。
    pub fn save(self: &Arc<Self>, req: SaveReq) -> Result<Saved, DiaryError> {
        let content = req.content.trim();
        if content.is_empty() {
            return Err(DiaryError::Empty);
        }
        if content.chars().count() > diary::MAX_CHARS {
            return Err(DiaryError::TooLong);
        }
        let now = self.clock.now();
        let ts = now.timestamp_millis();
        // 本地词表先于一切（≤ 50 ms，FR-SAF-01）
        let local = chat::lexicon_hit(content, &self.lex, SafeMode::Off);
        let draft = req.draft_id.map(|d| {
            self.drafts
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&d)
                .map(|(_, t)| t)
        });
        let (id, source) = {
            let db = self.port.db();
            match req.id {
                Some(id) => {
                    let old = db.diary_get(id)?.ok_or(DiaryError::NotFound)?;
                    let source = Source::parse(&old.source).after_edit(&old.content, content);
                    db.diary_update(id, content, source.as_str(), ts)?;
                    (id, source)
                }
                None => {
                    let source = Source::of_new(draft.as_ref().map(|d| d.as_deref()), content);
                    let date = now.format("%Y-%m-%d").to_string();
                    (
                        db.diary_insert(&date, content, source.as_str(), ts)?,
                        source,
                    )
                }
            }
        };
        let session = local.then(|| self.open_safety()).flatten();
        if self.port.jev_allowed() {
            let me = Arc::clone(self);
            let text = content.to_string();
            let _handle = tokio::spawn(async move {
                let jev = ChatService::judge_crisis(me.gateway.clone(), text).await;
                me.crisis_done(local, jev, session);
            });
            #[cfg(test)]
            {
                *self.last_check.lock().unwrap() = Some(_handle);
            }
        } else {
            self.crisis_done(local, false, session);
        }
        Ok(Saved {
            id,
            source,
            safety: local,
        })
    }

    /// 开一段安全模式的对话并让外壳打开对话窗口显示求助卡片（FR-SAF-02 第 2 条、FR-SAF-03）。
    fn open_safety(&self) -> Option<i64> {
        let ts = self.clock.now_ms();
        let opened = {
            let db = self.port.db();
            db.chat_session_create(&self.copy.safety_session, ts)
                .and_then(|id| db.chat_set_safe_mode(id, SafeMode::On).map(|()| id))
        };
        match opened {
            Ok(id) => {
                self.port.safety(id);
                Some(id)
            }
            Err(e) => {
                self.port
                    .note(&format!("日记触发求助卡片时开对话失败：{e}"));
                None
            }
        }
    }

    /// 两个通道都有结果了：Jev 命中而词表没命中时这时才打开求助卡片；写 `safety_log`。
    fn crisis_done(&self, local: bool, jev: bool, session: Option<i64>) {
        let Some(channel) = CrisisChannel::combine(local, jev) else {
            return;
        };
        if session.is_none() && !local {
            self.open_safety();
        }
        if let Err(e) = self
            .port
            .db()
            .safety_log(self.clock.now_ms(), channel.as_str())
        {
            self.port.note(&format!("写入 safety_log 失败：{e}"));
        }
    }

    /// 删除一篇；没有这篇时返回 `false`。
    pub fn delete(&self, id: i64) -> Result<bool, DiaryError> {
        Ok(self.port.db().diary_delete(id)?)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use async_trait::async_trait;
    use chrono::{Local, TimeZone};
    use futures::stream::BoxStream;
    use xqp::MoodState;

    use super::*;
    use crate::domain::fusion::Source as FusionSource;
    use crate::infra::clock::ManualClock;
    use crate::infra::gateway::{
        AiError, Answer, CompleteResponse, Delta, GatewayHealth, JudgeRequest, JudgeResponse,
        Question,
    };
    use crate::infra::store::NewMessage;
    use crate::infra::templates::TemplateDirs;

    const GOOD: &str = "今天上午还挺顺的，下午开始有点累，晚上和晴晴聊了几句，说了说考试的事，心里松快了一些。明天想早点起来，先把最难的那一科看一遍。";

    struct Port {
        db: Mutex<Db>,
        summary: bool,
        jev: bool,
        safety: Mutex<Vec<i64>>,
    }

    struct Guard<'a>(std::sync::MutexGuard<'a, Db>);
    impl Deref for Guard<'_> {
        type Target = Db;
        fn deref(&self) -> &Db {
            &self.0
        }
    }

    impl DiaryPort for Port {
        fn db(&self) -> Box<dyn Deref<Target = Db> + '_> {
            Box::new(Guard(self.db.lock().unwrap()))
        }
        fn summary_allowed(&self) -> bool {
            self.summary
        }
        fn jev_allowed(&self) -> bool {
            self.jev
        }
        fn safety(&self, session_id: i64) {
            self.safety.lock().unwrap().push(session_id);
        }
    }

    struct Gateway {
        reply: Result<String, AiError>,
        crisis_p: f64,
        prompts: Mutex<Vec<String>>,
        judged: Mutex<u32>,
    }

    #[async_trait]
    impl AiGateway for Gateway {
        async fn judge(&self, req: JudgeRequest) -> Result<JudgeResponse, AiError> {
            assert_eq!(req.questions, [Question::Crisis]);
            *self.judged.lock().unwrap() += 1;
            Ok(JudgeResponse {
                model: "jev".into(),
                latency_ms: 1,
                answers: [(Question::Crisis, Answer::Noul { p: self.crisis_p })].into(),
            })
        }
        async fn complete(&self, req: CompleteRequest) -> Result<CompleteResponse, AiError> {
            assert_eq!(req.scenario, Scenario::Diary);
            assert_eq!(req.prompt_ver, "P-DIARY v1");
            self.prompts
                .lock()
                .unwrap()
                .push(req.messages[0].content.clone());
            self.reply.clone().map(|text| CompleteResponse {
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
        reply: Result<String, AiError>,
        summary: bool,
        jev: bool,
        crisis_p: f64,
    ) -> (Arc<DiaryService>, Arc<Port>, Arc<Gateway>) {
        let dirs = TemplateDirs::factory_only(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../hub_templates"),
        );
        let port = Arc::new(Port {
            db: Mutex::new(Db::open_in_memory().unwrap()),
            summary,
            jev,
            safety: Mutex::default(),
        });
        let gw = Arc::new(Gateway {
            reply,
            crisis_p,
            prompts: Mutex::default(),
            judged: Mutex::default(),
        });
        let clock = Arc::new(ManualClock::new(
            Local.with_ymd_and_hms(2026, 10, 6, 21, 0, 0).unwrap(),
        ));
        let s = Arc::new(DiaryService::new(
            port.clone(),
            gw.clone(),
            clock,
            DiaryPrompt::load(&dirs).unwrap(),
            DiaryCopy::load(&dirs).unwrap(),
            Arc::new(CrisisLexicon::load(&dirs).unwrap()),
            Arc::new(BannedWords::load(&dirs).unwrap()),
        ));
        (s, port, gw)
    }

    fn chat(port: &Port, said: &[&str]) -> i64 {
        let db = port.db();
        let id = db.chat_session_create("聊天", 0).unwrap();
        for (i, t) in said.iter().enumerate() {
            db.chat_message_insert(&NewMessage {
                session_id: id,
                role: if i % 2 == 0 { "user" } else { "assistant" },
                content: t,
                ts: i as i64,
                ai_generated: i % 2 == 1,
                model: None,
                prompt_ver: None,
            })
            .unwrap();
        }
        id
    }

    fn mood(port: &Port) {
        let ts = Local
            .with_ymd_and_hms(2026, 10, 6, 9, 0, 0)
            .unwrap()
            .timestamp_millis();
        port.db()
            .insert_mood_state(
                ts,
                None,
                MoodState::Tired,
                MoodState::Tired,
                FusionSource::Rule,
            )
            .unwrap();
    }

    #[tokio::test]
    async fn draft_from_summary_and_what_the_user_said() {
        let (s, port, gw) = rig(Ok(format!("  {GOOD}\n")), true, false, 0.0);
        mood(&port);
        let id = chat(&port, &["考试没考好", "抱抱你，想说说吗", "嗯，有点难过"]);
        let d = s.generate(Some(id)).await.unwrap();
        assert!(d.ai_generated && d.draft_id.is_some());
        assert_eq!(d.text, GOOD);
        let sent = gw.prompts.lock().unwrap()[0].clone();
        assert!(sent.contains("今日状态摘要：") && !sent.contains("今日状态摘要：（无）"));
        assert!(
            sent.contains("考试没考好；嗯，有点难过"),
            "只取用户的话：{sent}"
        );
        assert!(!sent.contains("抱抱你"));

        // 保存未修改的草稿 → ai_draft；再改 → ai_edited
        let saved = s
            .save(SaveReq {
                id: None,
                draft_id: d.draft_id,
                content: d.text.clone(),
            })
            .unwrap();
        assert_eq!(saved.source, Source::AiDraft);
        let edited = s
            .save(SaveReq {
                id: Some(saved.id),
                draft_id: None,
                content: format!("{GOOD}还想去散散步。"),
            })
            .unwrap();
        assert_eq!(edited.source, Source::AiEdited);
        assert_eq!(port.db().diaries().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn blank_template_when_nothing_to_write_or_ai_fails() {
        // 没同意 ④、也不是从对话进来：不出网
        let (s, _port, gw) = rig(Ok(GOOD.into()), false, false, 0.0);
        let d = s.generate(None).await.unwrap();
        assert!(!d.ai_generated && d.draft_id.is_none());
        assert!(d.text.starts_with("今天发生了"));
        assert!(gw.prompts.lock().unwrap().is_empty());

        let (s, port, _) = rig(Err(AiError::Timeout), true, false, 0.0);
        mood(&port);
        assert!(
            !s.generate(None).await.unwrap().ai_generated,
            "断网给空白模板"
        );

        let (s, port, _) = rig(Ok("我可能是抑郁了。".repeat(5)), true, false, 0.0);
        mood(&port);
        assert!(
            !s.generate(None).await.unwrap().ai_generated,
            "禁用词不通过"
        );
    }

    #[tokio::test]
    async fn crisis_conversations_are_not_sent() {
        let (s, port, gw) = rig(Ok(GOOD.into()), true, false, 0.0);
        let id = chat(&port, &["我不想活了"]);
        assert!(!s.generate(Some(id)).await.unwrap().ai_generated);
        port.db().chat_set_safe_mode(id, SafeMode::On).unwrap();
        assert!(!s.generate(Some(id)).await.unwrap().ai_generated);
        assert!(gw.prompts.lock().unwrap().is_empty(), "一个字都不出网");
    }

    #[tokio::test]
    async fn manual_diary_with_crisis_opens_the_help_card() {
        let (s, port, gw) = rig(Ok(GOOD.into()), false, false, 0.0);
        let saved = s
            .save(SaveReq {
                id: None,
                draft_id: None,
                content: "今天真的撑不下去了，我不想活了".into(),
            })
            .unwrap();
        assert!(saved.safety);
        assert_eq!(saved.source, Source::Manual);
        let opened = port.safety.lock().unwrap().clone();
        assert_eq!(opened.len(), 1);
        let session = port.db().chat_session(opened[0]).unwrap().unwrap();
        assert_eq!(session.safe_mode, SafeMode::On);
        assert_eq!(session.title, "关于今天的日记", "标题不含日记内容");
        assert_eq!(*gw.judged.lock().unwrap(), 0, "没同意 ② 不发 Jev");
        let channels: Vec<String> = port
            .db()
            .conn()
            .prepare("SELECT channel FROM safety_log")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(channels, ["lexicon"]);
        assert_eq!(port.db().diaries().unwrap().len(), 1, "日记照常保存");
    }

    #[tokio::test]
    async fn jev_only_hit_opens_the_card_later() {
        let (s, port, gw) = rig(Ok(GOOD.into()), false, true, 0.9);
        let saved = s
            .save(SaveReq {
                id: None,
                draft_id: None,
                content: "今天什么都不想做，觉得一切都没意义".into(),
            })
            .unwrap();
        assert!(!saved.safety);
        let check = s.last_check.lock().unwrap().take().unwrap();
        check.await.unwrap();
        assert_eq!(*gw.judged.lock().unwrap(), 1);
        assert_eq!(port.safety.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn bad_input_and_unknown_drafts() {
        let (s, _port, _) = rig(Ok(GOOD.into()), false, false, 0.0);
        let save = |id, draft_id, content: &str| {
            s.save(SaveReq {
                id,
                draft_id,
                content: content.into(),
            })
        };
        assert!(matches!(save(None, None, "  "), Err(DiaryError::Empty)));
        assert!(matches!(
            save(None, None, &"字".repeat(5001)),
            Err(DiaryError::TooLong)
        ));
        assert!(matches!(
            save(Some(99), None, "x"),
            Err(DiaryError::NotFound)
        ));
        // Hub 重启后草稿不在内存里：按“修改过”标
        assert_eq!(save(None, Some(42), GOOD).unwrap().source, Source::AiEdited);
        let id = save(None, None, "手写的").unwrap().id;
        assert!(s.delete(id).unwrap());
        assert!(!s.delete(id).unwrap());
    }
}
