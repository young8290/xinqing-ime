//! 大模型客户端（08 FR-AIG-03 / 04）：OpenAI 兼容 `/chat/completions`，对话走 SSE 流式。
//! 失败时换下一个模型重试 1 次（不是同一模型重试）；每个模型独立熔断（连续 3 次失败 → 5 分钟）；
//! 每个模型记最近 20 次调用的成功率与 P50，暖心话选可用且 P50 最低的模型，其余按优先级。

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bytes::Bytes;
use futures::StreamExt;
use futures::stream::{self, BoxStream};
use reqwest::{Client, RequestBuilder};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use xinqing_hub_core::infra::gateway::{
    AiError, Breaker, CompleteRequest, CompleteResponse, Delta, NetLogEntry, Scenario, redact,
};

use crate::config::LlmConfig;
use crate::error::{from_reqwest, kind_label};
use crate::http::{Outcome, authed, client, send_json};
use crate::jev::breaker_blocking;
use crate::metrics::{Shared, percentile};
use crate::sse::SseParser;

/// 每个模型保留的最近调用数（FR-AIG-04 第 2 条）。
const RECENT: usize = 20;
/// 模型列表与“测试连接”的超时。
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// 设置页显示的单个模型状态（FR-AIG-04）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModelStatus {
    pub model: String,
    /// 最近一次 `/models` 是否列出了它
    pub available: bool,
    pub breaker_open: bool,
    /// 最近 20 次调用的成功率；还没调用过时为空
    pub success_rate: Option<f64>,
    pub p50_ms: Option<u64>,
}

/// “测试连接”的单个模型结果（FR-AIG-04 第 4 条）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModelProbe {
    pub model: String,
    pub ok: bool,
    pub latency_ms: u64,
    /// 与出网日志相同：`ok`、HTTP 状态码或错误类别
    pub status: String,
}

#[derive(Debug)]
struct ModelState {
    name: String,
    available: bool,
    breaker: Breaker,
    /// (是否成功, 延迟毫秒)；流式调用记的是首字延迟
    recent: VecDeque<(bool, u64)>,
}

impl ModelState {
    fn p50(&self) -> Option<u64> {
        percentile(
            self.recent.iter().filter(|(ok, _)| *ok).map(|&(_, ms)| ms),
            0.5,
        )
    }

    fn selectable(&self, now_ms: i64) -> bool {
        self.available && !breaker_blocking(&self.breaker, now_ms)
    }
}

/// 全部模型的状态。流式调用结束时还要回写，所以放在 `Arc` 里。
#[derive(Clone)]
struct ModelBook(Arc<Mutex<Vec<ModelState>>>);

impl ModelBook {
    fn new(names: &[String]) -> Self {
        Self(Arc::new(Mutex::new(
            names
                .iter()
                .map(|n| ModelState {
                    name: n.clone(),
                    available: true,
                    breaker: Breaker::llm(),
                    recent: VecDeque::new(),
                })
                .collect(),
        )))
    }

    fn with<R>(&self, name: &str, f: impl FnOnce(&mut ModelState) -> R) -> Option<R> {
        self.0
            .lock()
            .unwrap()
            .iter_mut()
            .find(|m| m.name == name)
            .map(f)
    }

    /// 按场景排好的候选模型（17 第 2.5 节第 2 步）。
    fn candidates(&self, s: Scenario, now_ms: i64) -> Result<Vec<String>, AiError> {
        let g = self.0.lock().unwrap();
        let mut picked: Vec<&ModelState> = g.iter().filter(|m| m.selectable(now_ms)).collect();
        if picked.is_empty() {
            return Err(if g.iter().any(|m| m.available) {
                AiError::CircuitOpen
            } else {
                AiError::ModelUnavailable
            });
        }
        if s == Scenario::Comfort {
            // 稳定排序：没有样本的排在后面，P50 相同按优先级
            picked.sort_by_key(|m| m.p50().unwrap_or(u64::MAX));
        }
        Ok(picked.into_iter().map(|m| m.name.clone()).collect())
    }

    fn try_acquire(&self, name: &str, now_ms: i64) -> bool {
        self.with(name, |m| m.breaker.try_acquire(now_ms))
            .unwrap_or(false)
    }

    /// 更新熔断器；`model_not_found` 同时把模型标为不可用，等下次 `/models` 再恢复。
    fn settle_breaker(&self, name: &str, err: Option<&AiError>, now_ms: i64) {
        self.with(name, |m| match err {
            None => m.breaker.on_success(),
            Some(e) => {
                if *e == AiError::ModelUnavailable {
                    m.available = false;
                }
                m.breaker.on_failure(now_ms);
            }
        });
    }

    fn sample(&self, name: &str, ok: bool, latency_ms: u64) {
        self.with(name, |m| {
            if m.recent.len() == RECENT {
                m.recent.pop_front();
            }
            m.recent.push_back((ok, latency_ms));
        });
    }

    fn set_available(&self, name: &str, available: bool) {
        self.with(name, |m| m.available = available);
    }

    fn status(&self, now_ms: i64) -> Vec<ModelStatus> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .map(|m| ModelStatus {
                model: m.name.clone(),
                available: m.available,
                breaker_open: breaker_blocking(&m.breaker, now_ms),
                success_rate: (!m.recent.is_empty()).then(|| {
                    m.recent.iter().filter(|(ok, _)| *ok).count() as f64 / m.recent.len() as f64
                }),
                p50_ms: m.p50(),
            })
            .collect()
    }
}

fn scenario_name(s: Scenario) -> &'static str {
    match s {
        Scenario::Comfort => "comfort",
        Scenario::Schedule => "schedule",
        Scenario::Chat => "chat",
        Scenario::Diary => "diary",
        Scenario::Todo => "todo",
        Scenario::Rewrite => "rewrite",
        Scenario::Letter => "letter",
    }
}

#[derive(Debug, Serialize)]
struct WireMessage {
    role: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct ChatBody<'a> {
    model: &'a str,
    messages: &'a [WireMessage],
    temperature: f32,
    max_tokens: u32,
    stream: bool,
}

#[derive(Deserialize)]
struct ChatResponse {
    model: Option<String>,
    #[serde(default)]
    choices: Vec<ChatChoice>,
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[derive(Deserialize)]
struct ChatMessage {
    content: Option<String>,
}

#[derive(Deserialize)]
struct Usage {
    prompt_tokens: Option<u32>,
    completion_tokens: Option<u32>,
}

#[derive(Deserialize)]
struct ModelList {
    data: Vec<ModelId>,
}

#[derive(Deserialize)]
struct ModelId {
    id: String,
}

/// 所有出网文本先脱敏（FR-AIG-05 第 2 条）；改写的可还原占位符由领域层在调用前处理。
fn redact_messages(req: &CompleteRequest) -> Vec<WireMessage> {
    req.messages
        .iter()
        .map(|m| WireMessage {
            role: m.role.clone(),
            content: redact(&m.content),
        })
        .collect()
}

/// 出网日志的字段名：大模型请求只有一个 `messages`。
fn llm_fields() -> Vec<String> {
    vec!["messages".into()]
}

pub(crate) struct LlmClient {
    http: Client,
    cfg: LlmConfig,
    book: ModelBook,
    shared: Arc<Shared>,
}

impl LlmClient {
    pub fn new(cfg: LlmConfig, shared: Arc<Shared>) -> Result<Self, reqwest::Error> {
        let http = client(cfg.connect_timeout)?;
        Ok(Self {
            http,
            book: ModelBook::new(&cfg.models),
            cfg,
            shared,
        })
    }

    fn post(&self, path: &str) -> RequestBuilder {
        authed(
            self.http.post(self.cfg.url(path)),
            self.cfg.api_key.as_ref(),
        )
    }

    /// 至少有一个模型可用且没在熔断。
    pub fn healthy(&self) -> bool {
        self.book
            .candidates(Scenario::Chat, self.shared.now_ms())
            .is_ok()
    }

    pub fn models(&self) -> Vec<ModelStatus> {
        self.book.status(self.shared.now_ms())
    }

    pub fn breaker_open(&self, model: &str) -> bool {
        let now = self.shared.now_ms();
        self.book
            .with(model, |m| breaker_blocking(&m.breaker, now))
            .unwrap_or(false)
    }

    fn log(&self, api: String, model: Option<String>, latency_ms: u64, status: String) {
        self.shared.log(NetLogEntry {
            ts: self.shared.now_ms(),
            fields: if api.starts_with("llm/models") {
                Vec::new()
            } else {
                llm_fields()
            },
            api,
            model,
            latency_ms: Some(latency_ms),
            status,
            tokens_in: None,
            tokens_out: None,
        });
    }

    pub async fn complete(&self, req: &CompleteRequest) -> Result<CompleteResponse, AiError> {
        let params = self.cfg.params(req.scenario);
        let candidates = self.book.candidates(req.scenario, self.shared.now_ms())?;
        let messages = redact_messages(req);
        let api = format!("llm/{}", scenario_name(req.scenario));

        let mut last = None;
        let mut tried = 0;
        for model in candidates {
            if tried == 2 {
                break;
            }
            if !self.book.try_acquire(&model, self.shared.now_ms()) {
                continue;
            }
            tried += 1;
            let body = ChatBody {
                model: &model,
                messages: &messages,
                temperature: params.temperature,
                max_tokens: params.max_tokens,
                stream: false,
            };
            let start = Instant::now();
            let out = send_json::<ChatResponse>(
                self.post("chat/completions")
                    .timeout(params.timeout)
                    .json(&body),
            )
            .await;
            let latency_ms = start.elapsed().as_millis() as u64;
            let usage = out
                .result
                .as_ref()
                .ok()
                .and_then(|r| r.usage.as_ref())
                .map(|u| (u.prompt_tokens, u.completion_tokens));
            let out = out.and_then(|r| {
                let text = r
                    .choices
                    .into_iter()
                    .next()
                    .and_then(|c| c.message.content)
                    .filter(|t| !t.trim().is_empty())
                    .ok_or(AiError::InvalidOutput)?;
                Ok((r.model.unwrap_or_else(|| model.clone()), text))
            });

            self.book
                .settle_breaker(&model, out.result.as_ref().err(), self.shared.now_ms());
            self.book.sample(&model, out.result.is_ok(), latency_ms);
            self.shared
                .record(&format!("llm/{model}"), out.result.is_ok(), latency_ms);
            let (tokens_in, tokens_out) = usage.unwrap_or_default();
            self.shared.log(NetLogEntry {
                ts: self.shared.now_ms(),
                api: api.clone(),
                model: Some(model.clone()),
                fields: llm_fields(),
                latency_ms: Some(latency_ms),
                status: out.status,
                tokens_in,
                tokens_out,
            });

            match out.result {
                Ok((model, text)) => {
                    return Ok(CompleteResponse {
                        model,
                        text,
                        latency_ms,
                    });
                }
                // 请求本身有问题，换模型也一样
                Err(AiError::BadRequest) => return Err(AiError::BadRequest),
                Err(e) => last = Some(e),
            }
        }
        Err(last.unwrap_or(AiError::CircuitOpen))
    }

    /// 流式生成。等到第一个字才返回，首字超时或失败时还能换模型；之后的错误作为流里的一项给出。
    pub async fn stream(
        &self,
        req: &CompleteRequest,
    ) -> Result<BoxStream<'static, Result<Delta, AiError>>, AiError> {
        let params = self.cfg.params(req.scenario);
        let first_timeout = params
            .first_token_timeout
            .unwrap_or(params.timeout)
            .min(params.timeout);
        let candidates = self.book.candidates(req.scenario, self.shared.now_ms())?;
        let messages = redact_messages(req);
        let api = format!("llm/{}", scenario_name(req.scenario));

        let mut last = None;
        let mut tried = 0;
        for model in candidates {
            if tried == 2 {
                break;
            }
            if !self.book.try_acquire(&model, self.shared.now_ms()) {
                continue;
            }
            tried += 1;
            let body = ChatBody {
                model: &model,
                messages: &messages,
                temperature: params.temperature,
                max_tokens: params.max_tokens,
                stream: true,
            };
            let start = Instant::now();
            let begun = tokio::time::Instant::now();
            let request = self.post("chat/completions").json(&body);
            let opened = tokio::time::timeout_at(begun + first_timeout, open_stream(request))
                .await
                .unwrap_or_else(|_| Err(Outcome::err(AiError::Timeout)));
            let first_ms = start.elapsed().as_millis() as u64;
            let now = self.shared.now_ms();

            match opened {
                Ok((reader, first)) => {
                    // 出了第一个字就算这个模型能用：结束半开状态
                    self.book.settle_breaker(&model, None, now);
                    let tail = StreamTail {
                        book: self.book.clone(),
                        shared: self.shared.clone(),
                        model: model.clone(),
                        api: api.clone(),
                        start,
                        first_ms,
                        done: false,
                    };
                    let live = Live {
                        reader,
                        tail,
                        deadline: begun + params.timeout,
                        first: Some(first),
                        ended: false,
                    };
                    return Ok(live.into_stream());
                }
                Err(out) => {
                    let e = out.result.err().unwrap_or(AiError::InvalidOutput);
                    self.book.settle_breaker(&model, Some(&e), now);
                    self.book.sample(&model, false, first_ms);
                    self.shared.record(&format!("llm/{model}"), false, first_ms);
                    self.log(api.clone(), Some(model.clone()), first_ms, out.status);
                    if e == AiError::BadRequest {
                        return Err(e);
                    }
                    last = Some(e);
                }
            }
        }
        Err(last.unwrap_or(AiError::CircuitOpen))
    }

    /// `GET /models`，把列表里没有的模型标为不可用（FR-AIG-04 第 1 条）。请求失败时保持原状。
    pub async fn refresh_models(&self) -> Result<Vec<String>, AiError> {
        let start = Instant::now();
        let request = authed(
            self.http.get(self.cfg.url("models")).timeout(PROBE_TIMEOUT),
            self.cfg.api_key.as_ref(),
        );
        let out = send_json::<ModelList>(request).await;
        self.log(
            "llm/models".into(),
            None,
            start.elapsed().as_millis() as u64,
            out.status,
        );
        let ids: Vec<String> = out.result?.data.into_iter().map(|m| m.id).collect();
        for name in &self.cfg.models {
            self.book.set_available(name, ids.contains(name));
        }
        Ok(ids)
    }

    /// 设置页“测试连接”：逐个模型发一条最小请求（FR-AIG-04 第 4 条）。不动熔断器，只更新可用标记。
    pub async fn test_connection(&self) -> Vec<ModelProbe> {
        let messages = [WireMessage {
            role: "user".into(),
            content: "ping".into(),
        }];
        let mut out = Vec::new();
        for model in &self.cfg.models {
            let body = ChatBody {
                model,
                messages: &messages,
                temperature: 0.0,
                max_tokens: 64,
                stream: false,
            };
            let start = Instant::now();
            let r = send_json::<Value>(
                self.post("chat/completions")
                    .timeout(PROBE_TIMEOUT)
                    .json(&body),
            )
            .await;
            let latency_ms = start.elapsed().as_millis() as u64;
            match &r.result {
                Ok(_) => self.book.set_available(model, true),
                Err(AiError::ModelUnavailable) => self.book.set_available(model, false),
                Err(_) => {}
            }
            self.log(
                "llm/test".into(),
                Some(model.clone()),
                latency_ms,
                r.status.clone(),
            );
            out.push(ModelProbe {
                model: model.clone(),
                ok: r.result.is_ok(),
                latency_ms,
                status: r.status,
            });
        }
        out
    }
}

enum Item {
    Delta(String),
    Done,
    Fail(AiError),
}

/// 一个 SSE 事件的 `data`。只取 `choices[0].delta.content`；只有角色或结束原因的事件跳过。
fn parse_event(data: &str) -> Option<Item> {
    if data.trim() == "[DONE]" {
        return Some(Item::Done);
    }
    let v: Value = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(_) => return Some(Item::Fail(AiError::InvalidOutput)),
    };
    if v.get("error").is_some() {
        return Some(Item::Fail(AiError::Upstream));
    }
    v.pointer("/choices/0/delta/content")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .map(|t| Item::Delta(t.to_string()))
}

struct Reader {
    bytes: BoxStream<'static, reqwest::Result<Bytes>>,
    parser: SseParser,
    queue: VecDeque<Item>,
}

impl Reader {
    /// 下一个事件。可以被超时取消：已解析的事件留在队列里，不会丢。
    async fn next(&mut self) -> Item {
        loop {
            if let Some(item) = self.queue.pop_front() {
                return item;
            }
            match self.bytes.next().await {
                None => return Item::Fail(AiError::Network),
                Some(Err(e)) => return Item::Fail(from_reqwest(&e)),
                Some(Ok(chunk)) => self.queue.extend(
                    self.parser
                        .push(&chunk)
                        .iter()
                        .filter_map(|d| parse_event(d)),
                ),
            }
        }
    }
}

/// 发出流式请求并读到第一个字。
async fn open_stream(request: RequestBuilder) -> Result<(Reader, String), Outcome<()>> {
    let resp = request
        .send()
        .await
        .map_err(|e| Outcome::err(from_reqwest(&e)))?;
    let code = resp.status();
    if !code.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(Outcome::http(code, &body));
    }
    let mut reader = Reader {
        bytes: resp.bytes_stream().boxed(),
        parser: SseParser::default(),
        queue: VecDeque::new(),
    };
    match reader.next().await {
        Item::Delta(t) => Ok((reader, t)),
        // 一个字都没有就结束，按输出不合格换模型
        Item::Done => Err(Outcome::err(AiError::InvalidOutput)),
        Item::Fail(e) => Err(Outcome::err(e)),
    }
}

/// 流式调用的收尾：结束、出错或被调用方丢弃时各记一次统计和出网日志。
struct StreamTail {
    book: ModelBook,
    shared: Arc<Shared>,
    model: String,
    api: String,
    start: Instant,
    first_ms: u64,
    done: bool,
}

impl StreamTail {
    fn finish(&mut self, err: Option<&AiError>) {
        self.done = true;
        if let Some(e) = err {
            self.book
                .settle_breaker(&self.model, Some(e), self.shared.now_ms());
        }
        let status = err.map_or("ok", kind_label);
        self.write(err.is_none(), status);
    }

    fn write(&self, ok: bool, status: &str) {
        self.book.sample(&self.model, ok, self.first_ms);
        self.shared
            .record(&format!("llm/{}", self.model), ok, self.first_ms);
        self.shared.log(NetLogEntry {
            ts: self.shared.now_ms(),
            api: self.api.clone(),
            model: Some(self.model.clone()),
            fields: llm_fields(),
            latency_ms: Some(self.start.elapsed().as_millis() as u64),
            status: status.into(),
            tokens_in: None,
            tokens_out: None,
        });
    }
}

impl Drop for StreamTail {
    /// 用户中途关掉对话：模型本身没出错，按成功计。
    fn drop(&mut self) {
        if !self.done {
            self.done = true;
            self.write(true, "cancelled");
        }
    }
}

struct Live {
    reader: Reader,
    tail: StreamTail,
    deadline: tokio::time::Instant,
    first: Option<String>,
    ended: bool,
}

impl Live {
    fn into_stream(self) -> BoxStream<'static, Result<Delta, AiError>> {
        stream::unfold(self, |mut st| async move {
            if let Some(text) = st.first.take() {
                return Some((Ok(Delta { text }), st));
            }
            if st.ended {
                return None;
            }
            let item = tokio::time::timeout_at(st.deadline, st.reader.next())
                .await
                .unwrap_or(Item::Fail(AiError::Timeout));
            match item {
                Item::Delta(text) => Some((Ok(Delta { text }), st)),
                Item::Done => {
                    st.tail.finish(None);
                    None
                }
                Item::Fail(e) => {
                    st.tail.finish(Some(&e));
                    st.ended = true;
                    Some((Err(e), st))
                }
            }
        })
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xinqing_hub_core::infra::gateway::Message;

    #[test]
    fn comfort_prefers_lowest_p50_others_keep_priority() {
        let names: Vec<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
        let book = ModelBook::new(&names);
        book.sample("a", true, 900);
        book.sample("b", true, 200);
        book.sample("b", false, 8000);
        assert_eq!(
            book.candidates(Scenario::Comfort, 0).unwrap(),
            ["b", "a", "c"],
            "失败不计入 P50；没有样本的排最后"
        );
        assert_eq!(book.candidates(Scenario::Chat, 0).unwrap(), ["a", "b", "c"]);
    }

    #[test]
    fn unavailable_and_open_models_are_skipped() {
        let names: Vec<String> = ["a", "b"].iter().map(|s| s.to_string()).collect();
        let book = ModelBook::new(&names);
        book.settle_breaker("a", Some(&AiError::ModelUnavailable), 0);
        assert_eq!(book.candidates(Scenario::Chat, 0).unwrap(), ["b"]);
        for _ in 0..3 {
            book.settle_breaker("b", Some(&AiError::Upstream), 0);
        }
        assert_eq!(
            book.candidates(Scenario::Chat, 0).unwrap_err(),
            AiError::CircuitOpen
        );
        assert_eq!(
            book.candidates(Scenario::Chat, 5 * 60_000).unwrap(),
            ["b"],
            "打开期过后可以试探"
        );
        book.set_available("b", false);
        assert_eq!(
            book.candidates(Scenario::Chat, 5 * 60_000).unwrap_err(),
            AiError::ModelUnavailable
        );
    }

    #[test]
    fn recent_window_keeps_twenty_calls() {
        let book = ModelBook::new(&["a".to_string()]);
        for i in 0..25 {
            book.sample("a", i % 5 != 0, 100);
        }
        let s = &book.status(0)[0];
        assert_eq!(s.success_rate, Some(16.0 / 20.0));
        assert_eq!(s.p50_ms, Some(100));
    }

    #[test]
    fn messages_are_redacted_before_leaving() {
        let req = CompleteRequest {
            scenario: Scenario::Chat,
            prompt_ver: "P-CHAT-v1".into(),
            messages: vec![Message {
                role: "user".into(),
                content: "加我微信，电话13812345678".into(),
            }],
        };
        let wire = redact_messages(&req);
        assert_eq!(wire[0].content, "加我微信，电话[手机号]");
    }

    #[test]
    fn sse_events() {
        let delta = r#"{"choices":[{"index":0,"delta":{"content":"嗨"}}]}"#;
        assert!(matches!(parse_event(delta), Some(Item::Delta(t)) if t == "嗨"));
        let role_only = r#"{"choices":[{"index":0,"delta":{"role":"assistant"}}]}"#;
        assert!(parse_event(role_only).is_none());
        assert!(matches!(parse_event("[DONE]"), Some(Item::Done)));
        assert!(matches!(
            parse_event(r#"{"error":{"message":"x"}}"#),
            Some(Item::Fail(AiError::Upstream))
        ));
        assert!(matches!(
            parse_event("not json"),
            Some(Item::Fail(AiError::InvalidOutput))
        ));
    }
}
