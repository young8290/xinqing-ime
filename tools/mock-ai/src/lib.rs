//! mock-ai：本地模拟 Jev 与 OpenAI 兼容大模型接口（12 第 1.2 节、14 第 3.3 节、17 第 4 节）。
//!
//! 只监听 127.0.0.1，不需要任何真实密钥。场景用启动参数 `--scenario` 指定，
//! 单个请求可以用请求头 `X-Mock-Scenario` 覆盖。网关的集成测试直接调用 [`serve`] 在进程内起服务。
//!
//! 注意：产品书没有给出 Jev 的线上请求/响应格式，这里的 `/v1/systemone` 采用与
//! `xinqing-hub-core` 网关类型一致的结构（questions + state → answers）。拿到 Jev 接口文档后，
//! 由 C 在网关的 JevClient 中做格式转换，本工具同步调整。

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use clap::ValueEnum;
use futures::Stream;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Scenario {
    /// 正常响应
    Normal,
    /// 慢响应（6 秒，超过 Jev 5 秒总超时）
    Slow,
    /// 不响应（60 秒）
    Timeout,
    /// 返回 500
    #[value(name = "500")]
    E500,
    /// 返回 422（请求格式错误，不应重试）
    #[value(name = "422")]
    E422,
    /// 优先级第一的模型不存在：/models 不含它，用它请求 chat 返回 404 model_not_found；
    /// 其他模型照常可用（TC-AIG-02 自动换下一个模型）
    #[value(name = "model_not_found")]
    ModelNotFound,
    /// SSE 流中途断开，没有 [DONE]
    #[value(name = "stream_cut")]
    StreamCut,
}

impl Scenario {
    fn from_header(h: &HeaderMap) -> Option<Self> {
        let v = h.get("x-mock-scenario")?.to_str().ok()?;
        Scenario::from_str(v, true).ok()
    }
}

/// 进程内 mock 的控制柄：各接口收到的请求数（测试用来断言“重试了几次”“熔断期间没有发请求”）、
/// 最近一次请求的内容与 `Authorization` 头（测试脱敏和密钥），以及运行中切换场景（测试熔断恢复）。
#[derive(Debug, Clone)]
pub struct Handle {
    jev: Arc<AtomicUsize>,
    chat: Arc<AtomicUsize>,
    models: Arc<AtomicUsize>,
    scenario: Arc<Mutex<Scenario>>,
    last_body: Arc<Mutex<Option<Value>>>,
    last_auth: Arc<Mutex<Option<String>>>,
}

impl Handle {
    pub fn new(scenario: Scenario) -> Self {
        Self {
            jev: Arc::default(),
            chat: Arc::default(),
            models: Arc::default(),
            scenario: Arc::new(Mutex::new(scenario)),
            last_body: Arc::default(),
            last_auth: Arc::default(),
        }
    }

    pub fn jev(&self) -> usize {
        self.jev.load(Ordering::SeqCst)
    }

    pub fn chat(&self) -> usize {
        self.chat.load(Ordering::SeqCst)
    }

    pub fn models(&self) -> usize {
        self.models.load(Ordering::SeqCst)
    }

    pub fn set_scenario(&self, s: Scenario) {
        *self.scenario.lock().unwrap() = s;
    }

    /// 最近一次 Jev 或 chat 请求的 JSON 请求体。
    pub fn last_body(&self) -> Option<Value> {
        self.last_body.lock().unwrap().clone()
    }

    /// 最近一次 Jev 或 chat 请求的 `Authorization` 头。
    pub fn last_auth(&self) -> Option<String> {
        self.last_auth.lock().unwrap().clone()
    }

    fn scenario(&self) -> Scenario {
        *self.scenario.lock().unwrap()
    }

    /// 记下请求体和认证头，再按需要的结构解析。
    fn record<T: serde::de::DeserializeOwned>(
        &self,
        h: &HeaderMap,
        body: Value,
    ) -> Result<T, serde_json::Error> {
        *self.last_auth.lock().unwrap() = h
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        *self.last_body.lock().unwrap() = Some(body.clone());
        serde_json::from_value(body)
    }
}

/// 默认（优先级第一）的模型名，与产品书 08 FR-AIG-03 的默认值一致。
pub const DEFAULT_MODEL: &str = "gemini-3.7-flash";
/// `/models` 列出的备选模型。
pub const BACKUP_MODEL: &str = "backup-model";
/// 对话与一次性生成的固定回复。
pub const REPLY: &str = "我在这儿呢，想说什么都可以慢慢说。";
/// P-COMFORT 的回复（08 第 4 节的一行 JSON），按调用次数轮换，连续几次也能过 V5 去重。
pub const COMFORT_REPLIES: [&str; 3] = [
    r#"{"text":"先停十秒，深呼吸一下？","kind":"rest"}"#,
    r#"{"text":"今天辛苦啦，喝口水歇一歇。","kind":"comfort"}"#,
    r#"{"text":"慢慢来，你已经做得很好了。","kind":"cheer"}"#,
];

/// P-LETTER 的回复：一封不含数字的周信（V9 只要求出现的数字都来自统计），长度、禁用词都能过校验。
pub const LETTER_REPLY: &str = "你好呀，\n\n这一周你在电脑前忙了不少事情，辛苦了。能看出来你在认真对待手头的每一件事，有累的时候，也有顺手的时候，这些都很正常。忙的日子里还记得停下来喝口水、伸个懒腰，这份照顾自己的心意很难得。\n\n这周有几个晚上好像收工得有点晚，白天的节奏也偏紧一些，身体大概在悄悄提醒你放慢一点。不用急着改变什么，先留意到就已经很好了。\n\n下周可以试试：找一个晚上，比平时早半小时合上电脑，做一件让自己放松的小事。\n\n晴晴";

/// P-DIARY 的回复：一篇第一人称的日记草稿，长度、禁用词都能过校验。
pub const DIARY_REPLY: &str = "今天过得有点累，上午还算顺利，下午开始状态慢慢往下走。晚上和晴晴说了几句话，把心里的事讲出来以后轻松了一些。很多事情一下子做不完，也没关系。明天想早点起床，先做一件最简单的小事。";

/// P-SCHEDULE 的回复：标题取句中认得的事件词（认不出时“日程”），日期时刻留空让 Hub 按原句计算（FR-SCH-04）。
pub fn schedule_reply(sentence: &str) -> String {
    const EVENTS: [&str; 8] = [
        "组会",
        "开会",
        "面试",
        "考试",
        "上课",
        "看电影",
        "聚餐",
        "答辩",
    ];
    let title = EVENTS
        .iter()
        .find(|e| sentence.contains(*e))
        .copied()
        .unwrap_or("日程");
    serde_json::json!({
        "has_event": true, "title": title, "date": null, "time": null, "end_time": null,
        "all_day": false, "location": null, "is_deadline": sentence.contains("交"),
    })
    .to_string()
}

/// P-TODO 的回复：去掉“记得 / 别忘了”后取前 8 个字作标题，截止日期留空让 Hub 计算。
pub fn todo_reply(sentence: &str) -> String {
    let rest = sentence
        .trim_start_matches("记得")
        .trim_start_matches("别忘了");
    let title: String = rest.chars().take(8).collect();
    serde_json::json!({"is_todo": true, "title": title, "due_date": null}).to_string()
}

/// P-REWRITE 的回复：在原文（提示词最后一行“原文：”之后，已是占位符形式）前后加几个客气字，
/// 原文里的数字、`@某人`、占位符都原样保留，能过 V8。
pub fn rewrite_reply(original: &str) -> String {
    let cands = [
        format!("你好，{original}"),
        format!("麻烦看一下：{original}"),
        format!("{original}，谢谢"),
    ];
    serde_json::json!({ "candidates": cands }).to_string()
}

/// 请求的是 P-COMFORT（提示词要求输出 `{"text":...,"kind":"comfort|rest|cheer"}`）或 P-REWRITE（`{"candidates":...}`）
/// 时回 JSON，P-SCHEDULE / P-TODO 回抽取结果，P-LETTER（“本周统计：”）回一封信，P-DIARY（“情绪日记草稿”）回一篇草稿，
/// 否则回固定的一句话。
fn reply_for(req: &ChatReq, n: usize) -> String {
    let has = |marker: &str| req.messages.iter().any(|m| m.content.contains(marker));
    if has(r#""kind":"comfort|rest|cheer""#) {
        COMFORT_REPLIES[n % COMFORT_REPLIES.len()].to_string()
    } else if has(r#"{"candidates":"#) {
        let original = req
            .messages
            .iter()
            .rev()
            .find_map(|m| m.content.rsplit_once("原文：").map(|(_, t)| t.trim()))
            .unwrap_or_default();
        rewrite_reply(original)
    } else if has(r#""has_event""#) || has(r#""is_todo""#) {
        let sentence = req
            .messages
            .iter()
            .rev()
            .find_map(|m| m.content.rsplit_once("句子：").map(|(_, t)| t.trim()))
            .unwrap_or_default();
        if has(r#""has_event""#) {
            schedule_reply(sentence)
        } else {
            todo_reply(sentence)
        }
    } else if has("本周统计：") {
        LETTER_REPLY.to_string()
    } else if has("情绪日记草稿") {
        DIARY_REPLY.to_string()
    } else {
        REPLY.to_string()
    }
}

pub fn router(handle: Handle) -> Router {
    Router::new()
        .route("/v1/systemone", post(jev))
        .route("/v1/chat/completions", post(chat))
        .route("/v1/models", get(models))
        .with_state(handle)
}

/// 在 127.0.0.1 的随机端口上起服务，返回 `http://127.0.0.1:<port>` 和控制柄。
pub async fn serve(scenario: Scenario) -> std::io::Result<(String, Handle)> {
    let handle = Handle::new(scenario);
    let app = router(handle.clone());
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0))).await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok((format!("http://{addr}"), handle))
}

fn pick(st: &Handle, h: &HeaderMap) -> Scenario {
    Scenario::from_header(h).unwrap_or_else(|| st.scenario())
}

/// 请求体结构不对：返回 422。
fn invalid(e: serde_json::Error) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({"error": {"message": e.to_string()}})),
    )
        .into_response()
}

/// 公共故障注入；返回 `Some` 表示直接用该响应结束。
async fn inject(s: Scenario) -> Option<Response> {
    match s {
        Scenario::Slow => {
            tokio::time::sleep(Duration::from_secs(6)).await;
            None
        }
        Scenario::Timeout => {
            tokio::time::sleep(Duration::from_secs(60)).await;
            Some(StatusCode::GATEWAY_TIMEOUT.into_response())
        }
        Scenario::E500 => Some(
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": {"message": "mock upstream error"}})),
            )
                .into_response(),
        ),
        Scenario::E422 => Some(
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({"error": {"message": "mock invalid request"}})),
            )
                .into_response(),
        ),
        _ => None,
    }
}

#[derive(Debug, Deserialize)]
struct JevReq {
    #[serde(default)]
    questions: Vec<String>,
    #[serde(default)]
    state: serde_json::Map<String, Value>,
}

/// 根据本地提示构造可复现的 Q-STATE 概率，便于回放脚本得到确定的状态序列。
fn state_answer(state: &serde_json::Map<String, Value>) -> Value {
    let hints = state
        .get("local_hints")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let (choice, p) = if hints.contains("hesitation_hint") {
        ("hesitant", 0.85)
    } else if hints.contains("agitation_hint") {
        ("agitated", 0.82)
    } else if hints.contains("fatigue_hint") || hints.contains("late_night") {
        ("tired", 0.80)
    } else if hints.contains("low_hint") {
        ("low", 0.84)
    } else {
        ("fluent", 0.88)
    };
    let rest = (1.0 - p) / 4.0;
    let mut probs = serde_json::Map::new();
    for s in ["fluent", "hesitant", "low", "agitated", "tired"] {
        probs.insert(s.into(), json!(if s == choice { p } else { rest }));
    }
    json!({"kind": "choice", "probs": probs, "choice": choice})
}

fn text_of(state: &serde_json::Map<String, Value>, key: &str) -> String {
    state
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

async fn jev(State(st): State<Handle>, h: HeaderMap, Json(body): Json<Value>) -> Response {
    st.jev.fetch_add(1, Ordering::SeqCst);
    let req: JevReq = match st.record(&h, body) {
        Ok(r) => r,
        Err(e) => return invalid(e),
    };
    let s = pick(&st, &h);
    if let Some(r) = inject(s).await {
        return r;
    }
    let mut answers = serde_json::Map::new();
    for q in &req.questions {
        let a = match q.as_str() {
            "Q-STATE" => state_answer(&req.state),
            "Q-VALENCE" => json!({"kind": "score", "score": 2.0}),
            "Q-COMFORT" => json!({"kind": "noul", "p": 0.4}),
            "Q-TEXT-EMO" => {
                json!({"kind": "choice", "probs": {"neutral": 0.8}, "choice": "neutral"})
            }
            "Q-PLAN" => {
                let t = text_of(&req.state, "committed_text");
                let yes = ["点", "周", "明天", "后天", "号"]
                    .iter()
                    .any(|k| t.contains(k));
                json!({"kind": "noul", "p": if yes { 0.95 } else { 0.05 }})
            }
            "Q-TODO" => {
                let t = text_of(&req.state, "committed_text");
                let yes = ["记得", "要去", "得去", "交", "买"]
                    .iter()
                    .any(|k| t.contains(k));
                json!({"kind": "noul", "p": if yes { 0.9 } else { 0.05 }})
            }
            "Q-CRISIS" => json!({"kind": "noul", "p": 0.02}),
            _ => {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(json!({"error": {"message": format!("unknown question {q}")}})),
                )
                    .into_response();
            }
        };
        answers.insert(q.clone(), a);
    }
    Json(json!({"model": "jev-mock", "answers": answers})).into_response()
}

#[derive(Debug, Deserialize)]
struct ChatReq {
    model: Option<String>,
    #[serde(default)]
    stream: bool,
    #[serde(default)]
    messages: Vec<ChatMsg>,
}

#[derive(Debug, Deserialize)]
struct ChatMsg {
    #[serde(default)]
    content: String,
}

async fn chat(State(st): State<Handle>, h: HeaderMap, Json(body): Json<Value>) -> Response {
    let n = st.chat.fetch_add(1, Ordering::SeqCst);
    let req: ChatReq = match st.record(&h, body) {
        Ok(r) => r,
        Err(e) => return invalid(e),
    };
    let s = pick(&st, &h);
    let model = req.model.clone().unwrap_or_else(|| DEFAULT_MODEL.into());
    if s == Scenario::ModelNotFound && model == DEFAULT_MODEL {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": {"code": "model_not_found", "message": format!("model {model} not found")}})),
        )
            .into_response();
    }
    if let Some(r) = inject(s).await {
        return r;
    }
    if req.stream {
        return Sse::new(sse_stream(model, s == Scenario::StreamCut)).into_response();
    }
    Json(json!({
        "id": "mock-1",
        "object": "chat.completion",
        "model": model,
        "choices": [{"index": 0, "message": {"role": "assistant", "content": reply_for(&req, n)}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 10, "completion_tokens": 16, "total_tokens": 26}
    }))
    .into_response()
}

fn sse_stream(
    model: String,
    cut: bool,
) -> impl Stream<Item = Result<Event, std::convert::Infallible>> {
    async_stream::stream! {
        let chars: Vec<char> = REPLY.chars().collect();
        let stop = if cut { chars.len() / 2 } else { chars.len() };
        for c in &chars[..stop] {
            tokio::time::sleep(Duration::from_millis(30)).await;
            let chunk = json!({
                "id": "mock-1", "object": "chat.completion.chunk", "model": model,
                "choices": [{"index": 0, "delta": {"content": c.to_string()}, "finish_reason": null}]
            });
            yield Ok(Event::default().data(chunk.to_string()));
        }
        if !cut {
            yield Ok(Event::default().data("[DONE]"));
        }
    }
}

async fn models(State(st): State<Handle>, h: HeaderMap) -> Response {
    st.models.fetch_add(1, Ordering::SeqCst);
    let s = pick(&st, &h);
    let ids: Vec<&str> = if s == Scenario::ModelNotFound {
        vec![BACKUP_MODEL, "some-other-model"]
    } else {
        vec![DEFAULT_MODEL, BACKUP_MODEL]
    };
    let data: Vec<Value> = ids
        .into_iter()
        .map(|id| json!({"id": id, "object": "model"}))
        .collect();
    Json(json!({"object": "list", "data": data})).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_answer_follows_hints() {
        let mut s = serde_json::Map::new();
        s.insert("local_hints".into(), json!("hesitation_hint"));
        assert_eq!(state_answer(&s)["choice"], "hesitant");
        assert_eq!(state_answer(&serde_json::Map::new())["choice"], "fluent");
    }

    #[test]
    fn scenario_header_parsing() {
        let mut h = HeaderMap::new();
        h.insert("x-mock-scenario", "stream_cut".parse().unwrap());
        assert_eq!(Scenario::from_header(&h), Some(Scenario::StreamCut));
        h.insert("x-mock-scenario", "500".parse().unwrap());
        assert_eq!(Scenario::from_header(&h), Some(Scenario::E500));
    }
}
