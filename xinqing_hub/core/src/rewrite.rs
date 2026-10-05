//! 温柔改写服务（C-09，06 FR-RWR-05/07、08 P-REWRITE）：处理核心的 `rewrite_req`，回 `rewrite_result` / `rewrite_fail`；
//! 收到 `rewrite_done` 时写 `rewrite_log`。
//!
//! 一次请求：同意 ⑥ → 本地危机词表 → 可还原占位符 → P-REWRITE（网关 10 秒） → V1 解析、V4/V8 校验 → 换回占位符 → 回核心。
//! 原文与结果只在内存里，不落库、不写日志；`rewrite_log` 只有来源、风格、选了第几个、长度和结果（FR-RWR-04）。
//! 与外壳之间只通过 [`RewritePort`]，本模块不依赖 Tauri（ADR 0007）。实现说明见 docs/adr/0028。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::broadcast;
use xqp::{Down, RewriteFailReason, RewriteOutcome, RewriteSource, RewriteStyle, Up};
use zeroize::Zeroizing;

use crate::bus::HubEvent;
use crate::domain::rewrite::{self, RewritePrompt};
use crate::domain::safety::{self, CrisisLexicon};
use crate::domain::validate::BannedWords;
use crate::infra::clock::Clock;
use crate::infra::gateway::{AiError, AiGateway, CompleteRequest, Message, Scenario};

/// 核心等 10 秒就放弃（FR-RWR-03）；这边提前一点给出 `timeout`，免得结果在核心放弃之后才到。
pub const TIMEOUT: Duration = Duration::from_millis(9_500);
/// 没等到 `rewrite_done` 的请求最多记这么久（核心退出改写模式时一定会发，这里只防丢）。
const PENDING_TTL_MS: i64 = 10 * 60_000;

/// `rewrite_log` 的一行（D-28），不含任何文字。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewriteLog {
    pub ts: i64,
    pub source: RewriteSource,
    pub style: RewriteStyle,
    /// 选了第几个（1–3）；没选是 `None`
    pub chosen: Option<u8>,
    /// 原文与所选结果的字数
    pub len_in: u32,
    pub len_out: Option<u32>,
    pub outcome: RewriteOutcome,
}

/// 外壳提供给改写服务的能力。
pub trait RewritePort: Send + Sync {
    /// 是否已同意 ⑥（温柔改写）。核心那边也查，这里再查一次，同意状态以 Hub 为准。
    fn allowed(&self) -> bool;
    /// 回给核心（`rewrite_result` / `rewrite_fail`）。
    fn reply(&self, d: Down);
    /// 原文命中本地危机词表（FR-RWR-05 第 3 条）：记 `safety_log`，让界面给出求助入口（ADR 0028 第 4 条）。
    fn crisis(&self, ts: i64);
    /// 写 `rewrite_log`。
    fn log(&self, rec: RewriteLog);
    /// 运行日志，只会传入不含用户数据的内容（NFR-LOG）。
    fn note(&self, _msg: &str) {}
}

/// 一次请求在等 `rewrite_done` 时记下的东西（不含文字）。
#[derive(Debug, Clone)]
struct Pending {
    at_ms: i64,
    source: RewriteSource,
    style: RewriteStyle,
    len_in: u32,
    /// 回给核心的各候选字数，`rewrite_done.chosen` 按下标取
    lens: Vec<u32>,
}

pub struct RewriteService {
    port: Arc<dyn RewritePort>,
    gateway: Arc<dyn AiGateway>,
    clock: Arc<dyn Clock>,
    prompt: RewritePrompt,
    lexicon: Arc<CrisisLexicon>,
    banned: Arc<BannedWords>,
    pending: Mutex<HashMap<u32, Pending>>,
}

impl RewriteService {
    pub fn new(
        port: Arc<dyn RewritePort>,
        gateway: Arc<dyn AiGateway>,
        clock: Arc<dyn Clock>,
        prompt: RewritePrompt,
        lexicon: Arc<CrisisLexicon>,
        banned: Arc<BannedWords>,
    ) -> Arc<Self> {
        Arc::new(Self {
            port,
            gateway,
            clock,
            prompt,
            lexicon,
            banned,
            pending: Mutex::new(HashMap::new()),
        })
    }

    /// 订阅总线，处理 `rewrite_req` / `rewrite_done`。每个请求一个任务，互不等待。
    pub async fn run(self: Arc<Self>, mut bus: broadcast::Receiver<HubEvent>) {
        loop {
            match bus.recv().await {
                Ok(HubEvent::Xqp(up)) => self.on_up(&up),
                Ok(_) => {}
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    self.port.note(&format!("改写服务跳过了 {n} 条总线事件"));
                }
                Err(broadcast::error::RecvError::Closed) => return,
            }
        }
    }

    pub fn on_up(self: &Arc<Self>, up: &Up) {
        match up {
            Up::RewriteReq {
                req_id,
                source,
                text,
                style,
                ..
            } => {
                let (req_id, source, style) = (*req_id, *source, *style);
                let text = Zeroizing::new(text.clone());
                let me = self.clone();
                tokio::spawn(async move {
                    let d = me.rewrite(req_id, source, &text, style).await;
                    me.port.reply(d);
                });
            }
            Up::RewriteDone {
                req_id,
                chosen,
                outcome,
                ..
            } => self.done(*req_id, *chosen, *outcome),
            _ => {}
        }
    }

    /// 处理一次请求，返回要回给核心的消息。
    pub async fn rewrite(
        &self,
        req_id: u32,
        source: RewriteSource,
        text: &str,
        style: RewriteStyle,
    ) -> Down {
        let now = self.clock.now_ms();
        let len_in = u32::try_from(text.chars().count()).unwrap_or(u32::MAX);
        self.remember(
            req_id,
            Pending {
                at_ms: now,
                source,
                style,
                len_in,
                lens: Vec::new(),
            },
        );
        let fail = |reason| Down::RewriteFail { req_id, reason };
        if !self.port.allowed() {
            return fail(RewriteFailReason::NoConsent);
        }
        if text.trim().is_empty() || text.chars().count() > rewrite::MAX_INPUT_CHARS {
            return fail(RewriteFailReason::Invalid);
        }
        // 危机内容不改写（FR-RWR-05 第 3 条），不出网
        if safety::check_local(text, &self.lexicon, self.lexicon.threshold).hit {
            self.port.crisis(now);
            return fail(RewriteFailReason::Crisis);
        }
        let masked = rewrite::mask(text);
        let req = CompleteRequest {
            scenario: Scenario::Rewrite,
            prompt_ver: self.prompt.ver(),
            messages: vec![Message {
                role: "user".into(),
                content: self.prompt.render(style, &masked.text),
            }],
        };
        let raw = match tokio::time::timeout(TIMEOUT, self.gateway.complete(req)).await {
            Err(_) | Ok(Err(AiError::Timeout)) => return fail(RewriteFailReason::Timeout),
            Ok(Err(AiError::BudgetExceeded)) => return fail(RewriteFailReason::Budget),
            Ok(Err(_)) => return fail(RewriteFailReason::Offline),
            Ok(Ok(resp)) => Zeroizing::new(resp.text),
        };
        let Some(cands) = rewrite::parse(&raw) else {
            return fail(RewriteFailReason::Invalid);
        };
        let cands = rewrite::accept(&cands, &masked, &self.banned);
        if cands.is_empty() {
            return fail(RewriteFailReason::Invalid);
        }
        let lens = cands
            .iter()
            .map(|c| u32::try_from(c.chars().count()).unwrap_or(u32::MAX))
            .collect();
        if let Some(p) = self.pending().get_mut(&req_id) {
            p.lens = lens;
        }
        Down::RewriteResult {
            req_id,
            style,
            cands,
        }
    }

    /// 核心退出改写模式：写一行 `rewrite_log`。没记着这个请求（例如 Hub 重启过）就不写。
    fn done(&self, req_id: u32, chosen: Option<u8>, outcome: RewriteOutcome) {
        let Some(p) = self.pending().remove(&req_id) else {
            return;
        };
        // 协议里 `chosen` 从 0 数，`rewrite_log` 记“第几个”
        let len_out = chosen.and_then(|i| p.lens.get(usize::from(i)).copied());
        self.port.log(RewriteLog {
            ts: self.clock.now_ms(),
            source: p.source,
            style: p.style,
            chosen: chosen.map(|i| i.saturating_add(1)),
            len_in: p.len_in,
            len_out,
            outcome,
        });
    }

    fn remember(&self, req_id: u32, p: Pending) {
        let mut pending = self.pending();
        pending.retain(|_, q| p.at_ms - q.at_ms < PENDING_TTL_MS);
        pending.insert(req_id, p);
    }

    fn pending(&self) -> std::sync::MutexGuard<'_, HashMap<u32, Pending>> {
        self.pending.lock().unwrap_or_else(|e| e.into_inner())
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
    use crate::infra::gateway::{
        CompleteResponse, Delta, GatewayHealth, JudgeRequest, JudgeResponse,
    };
    use crate::infra::templates::TemplateDirs;

    fn dirs() -> TemplateDirs {
        TemplateDirs::factory_only(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../hub_templates"),
        )
    }

    /// 回固定文字的网关，并记下发出去的提示词。
    struct FakeLlm {
        reply: Mutex<Result<String, AiError>>,
        sent: Mutex<Vec<String>>,
        delay: Duration,
    }

    impl FakeLlm {
        fn new(reply: Result<&str, AiError>) -> Arc<Self> {
            Arc::new(Self {
                reply: Mutex::new(reply.map(str::to_string)),
                sent: Mutex::new(Vec::new()),
                delay: Duration::ZERO,
            })
        }
    }

    #[async_trait]
    impl AiGateway for FakeLlm {
        async fn judge(&self, _req: JudgeRequest) -> Result<JudgeResponse, AiError> {
            Err(AiError::ModelUnavailable)
        }
        async fn complete(&self, req: CompleteRequest) -> Result<CompleteResponse, AiError> {
            assert_eq!(req.scenario, Scenario::Rewrite);
            self.sent
                .lock()
                .unwrap()
                .push(req.messages[0].content.clone());
            tokio::time::sleep(self.delay).await;
            self.reply
                .lock()
                .unwrap()
                .clone()
                .map(|text| CompleteResponse {
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

    #[derive(Default)]
    struct FakePort {
        denied: bool,
        crises: Mutex<Vec<i64>>,
        logs: Mutex<Vec<RewriteLog>>,
        replies: Mutex<Vec<Down>>,
    }

    impl RewritePort for FakePort {
        fn allowed(&self) -> bool {
            !self.denied
        }
        fn reply(&self, d: Down) {
            self.replies.lock().unwrap().push(d);
        }
        fn crisis(&self, ts: i64) {
            self.crises.lock().unwrap().push(ts);
        }
        fn log(&self, rec: RewriteLog) {
            self.logs.lock().unwrap().push(rec);
        }
    }

    fn service(port: Arc<FakePort>, llm: Arc<FakeLlm>) -> Arc<RewriteService> {
        RewriteService::new(
            port,
            llm,
            Arc::new(ManualClock::new(Local.timestamp_millis_opt(1_000).unwrap())),
            RewritePrompt::load(&dirs()).unwrap(),
            Arc::new(CrisisLexicon::load(&dirs()).unwrap()),
            Arc::new(BannedWords::load(&dirs()).unwrap()),
        )
    }

    const SRC: &str = "你怎么还不回消息？打13812345678找我";

    #[tokio::test]
    async fn rewrites_with_placeholders_and_logs_lengths_only() {
        let llm = FakeLlm::new(Ok(
            r#"{"candidates":["看到消息麻烦回我一下，可以打[号码1]找我","方便时回个消息吧，打[号码1]也行","回消息"]}"#,
        ));
        let port = Arc::new(FakePort::default());
        let s = service(port.clone(), llm.clone());
        let d = s
            .rewrite(7, RewriteSource::Recent, SRC, RewriteStyle::Gentle)
            .await;
        let Down::RewriteResult { req_id, cands, .. } = d else {
            panic!("{d:?}")
        };
        assert_eq!(req_id, 7);
        // 第三个丢了号码，被 V8 丢弃；号码由代码换回
        assert_eq!(
            cands,
            [
                "看到消息麻烦回我一下，可以打13812345678找我",
                "方便时回个消息吧，打13812345678也行"
            ]
        );
        let sent = llm.sent.lock().unwrap()[0].clone();
        assert!(sent.contains("打[号码1]找我"), "发出去的是占位符");
        assert!(!sent.contains("13812345678"));
        assert!(sent.contains("“更温和”"));

        s.on_up(&Up::RewriteDone {
            ts: 0,
            seq: None,
            req_id: 7,
            chosen: Some(1),
            outcome: RewriteOutcome::Replaced,
        });
        let logs = port.logs.lock().unwrap().clone();
        assert_eq!(
            logs,
            [RewriteLog {
                ts: 1_000,
                source: RewriteSource::Recent,
                style: RewriteStyle::Gentle,
                chosen: Some(2),
                len_in: SRC.chars().count() as u32,
                len_out: Some(cands[1].chars().count() as u32),
                outcome: RewriteOutcome::Replaced,
            }]
        );
    }

    #[tokio::test]
    async fn failures_map_to_reasons() {
        let port = Arc::new(FakePort {
            denied: true,
            ..Default::default()
        });
        let ok = FakeLlm::new(Ok(r#"{"candidates":["甲"]}"#));
        let reason = |d: Down| match d {
            Down::RewriteFail { reason, .. } => reason,
            other => panic!("{other:?}"),
        };
        let s = service(port, ok.clone());
        let d = s
            .rewrite(1, RewriteSource::Recent, SRC, RewriteStyle::Gentle)
            .await;
        assert_eq!(reason(d), RewriteFailReason::NoConsent);
        assert!(ok.sent.lock().unwrap().is_empty(), "没同意不出网");

        let cases = [
            (Err(AiError::BudgetExceeded), RewriteFailReason::Budget),
            (Err(AiError::Timeout), RewriteFailReason::Timeout),
            (Err(AiError::Upstream), RewriteFailReason::Offline),
            (Ok("不是 JSON"), RewriteFailReason::Invalid),
            (
                Ok(r#"{"candidates":["回消息"]}"#),
                RewriteFailReason::Invalid,
            ),
        ];
        for (reply, want) in cases {
            let s = service(Arc::new(FakePort::default()), FakeLlm::new(reply));
            let d = s
                .rewrite(2, RewriteSource::Clipboard, SRC, RewriteStyle::Concise)
                .await;
            assert_eq!(reason(d), want);
        }
        let s = service(Arc::new(FakePort::default()), ok);
        let long = "啊".repeat(rewrite::MAX_INPUT_CHARS + 1);
        let d = s
            .rewrite(3, RewriteSource::Clipboard, &long, RewriteStyle::Gentle)
            .await;
        assert_eq!(reason(d), RewriteFailReason::Invalid);
    }

    #[tokio::test]
    async fn crisis_text_is_never_sent_and_invites_help() {
        let llm = FakeLlm::new(Ok(r#"{"candidates":["甲"]}"#));
        let port = Arc::new(FakePort::default());
        let s = service(port.clone(), llm.clone());
        let d = s
            .rewrite(
                4,
                RewriteSource::Recent,
                "我真的活不下去了，别再找我",
                RewriteStyle::Gentle,
            )
            .await;
        assert_eq!(
            d,
            Down::RewriteFail {
                req_id: 4,
                reason: RewriteFailReason::Crisis
            }
        );
        assert!(llm.sent.lock().unwrap().is_empty(), "危机内容不出网");
        assert_eq!(*port.crises.lock().unwrap(), [1_000]);
    }

    #[tokio::test(start_paused = true)]
    async fn gives_up_before_the_core_does() {
        let llm = Arc::new(FakeLlm {
            reply: Mutex::new(Ok(r#"{"candidates":["甲"]}"#.into())),
            sent: Mutex::new(Vec::new()),
            delay: Duration::from_secs(30),
        });
        let s = service(Arc::new(FakePort::default()), llm);
        let d = s
            .rewrite(5, RewriteSource::Recent, SRC, RewriteStyle::Gentle)
            .await;
        assert_eq!(
            d,
            Down::RewriteFail {
                req_id: 5,
                reason: RewriteFailReason::Timeout
            }
        );
    }

    #[tokio::test]
    async fn cancelled_and_failed_requests_are_logged_without_output() {
        let port = Arc::new(FakePort::default());
        let s = service(port.clone(), FakeLlm::new(Err(AiError::Upstream)));
        s.rewrite(6, RewriteSource::Clipboard, SRC, RewriteStyle::Polite)
            .await;
        s.on_up(&Up::RewriteDone {
            ts: 0,
            seq: None,
            req_id: 6,
            chosen: None,
            outcome: RewriteOutcome::Failed,
        });
        // 不认识的请求不写
        s.on_up(&Up::RewriteDone {
            ts: 0,
            seq: None,
            req_id: 99,
            chosen: None,
            outcome: RewriteOutcome::Cancelled,
        });
        let logs = port.logs.lock().unwrap().clone();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].chosen, None);
        assert_eq!(logs[0].len_out, None);
        assert_eq!(logs[0].outcome, RewriteOutcome::Failed);
        assert_eq!(logs[0].style, RewriteStyle::Polite);
    }
}
