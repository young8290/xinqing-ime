//! AI 网关（08 第 2 节、17 第 2.5 节）。领域层只依赖 `AiGateway` trait（A-04）。
//!
//! 本模块提供：trait 与类型、熔断器、预算、隐私过滤、离线实现（无密钥时使用）。
//! 真实的 Jev / 大模型 HTTP 客户端在 `xinqing-hub-gateway` crate（领域层 crate 不依赖 reqwest，TC-AIG-06），
//! 开发和测试对接 `tools/mock-ai`。

pub mod breaker;
pub mod budget;
pub mod privacy;

use std::collections::HashMap;

use async_trait::async_trait;
use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};

pub use breaker::Breaker;
pub use budget::{Budget, BudgetKind};
pub use privacy::redact;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
pub enum AiError {
    #[error("超时")]
    Timeout,
    #[error("被限流")]
    RateLimited,
    #[error("模型不可用")]
    ModelUnavailable,
    #[error("请求格式错误（4xx）")]
    BadRequest,
    #[error("上游错误（5xx）")]
    Upstream,
    #[error("网络错误")]
    Network,
    #[error("熔断中")]
    CircuitOpen,
    #[error("今日额度已用完")]
    BudgetExceeded,
    #[error("输出未通过校验")]
    InvalidOutput,
}

impl AiError {
    /// Jev 只对这三类重试 1 次（FR-AIG-02）。
    pub fn retryable(&self) -> bool {
        matches!(
            self,
            AiError::Timeout | AiError::Upstream | AiError::Network
        )
    }
}

/// Jev 问题目录（08 第 3 节）。每个问题声明允许的 state 字段（FR-AIG-05 字段白名单）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Question {
    #[serde(rename = "Q-STATE")]
    State,
    #[serde(rename = "Q-VALENCE")]
    Valence,
    #[serde(rename = "Q-COMFORT")]
    Comfort,
    #[serde(rename = "Q-TEXT-EMO")]
    TextEmo,
    #[serde(rename = "Q-PLAN")]
    Plan,
    #[serde(rename = "Q-TODO")]
    Todo,
    #[serde(rename = "Q-CRISIS")]
    Crisis,
}

/// Q-STATE / Q-VALENCE / Q-COMFORT 允许的字段：FR-STA-02 全部数值特征 + app_cat + local_hints。
pub const STATE_FIELDS: &[&str] = &[
    "n_keys",
    "active_ms",
    "kpm",
    "iki_med",
    "iki_iqr",
    "dwell_med",
    "bs_rate",
    "bs_burst_max",
    "typo_cnt",
    "pause_cnt",
    "abandon",
    "delete_committed",
    "page_flips",
    "cand_pos_mean",
    "session_min",
    "hour",
    "kpm_z",
    "iki_med_z",
    "iki_iqr_z",
    "bs_rate_z",
    "dwell_med_z",
    "app_cat",
    "local_hints",
];

impl Question {
    pub fn allowed_fields(self) -> &'static [&'static str] {
        match self {
            Question::State | Question::Valence | Question::Comfort => STATE_FIELDS,
            Question::TextEmo | Question::Plan | Question::Todo => &["committed_text"],
            Question::Crisis => &["message"],
        }
    }

    /// 是否允许带文字（决定是否需要脱敏、是否需要同意 ③/⑤）。
    pub fn carries_text(self) -> bool {
        !matches!(
            self,
            Question::State | Question::Valence | Question::Comfort
        )
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JudgeRequest {
    pub questions: Vec<Question>,
    pub state: serde_json::Map<String, serde_json::Value>,
}

impl JudgeRequest {
    /// 按全部问题的字段白名单的并集过滤 state，文本字段先脱敏。
    pub fn filtered(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut out = serde_json::Map::new();
        for (k, v) in &self.state {
            if self
                .questions
                .iter()
                .any(|q| q.allowed_fields().contains(&k.as_str()))
            {
                let v = match v {
                    serde_json::Value::String(s) => serde_json::Value::String(redact(s)),
                    other => other.clone(),
                };
                out.insert(k.clone(), v);
            }
        }
        out
    }
}

/// 单个问题的答案：`choice` 类给出各选项概率，`noul` 给出是的概率，`score` 给出 0–4 分。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Answer {
    Choice {
        probs: HashMap<String, f64>,
        choice: String,
    },
    Noul {
        p: f64,
    },
    Score {
        score: f64,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JudgeResponse {
    pub model: String,
    pub latency_ms: u64,
    pub answers: HashMap<Question, Answer>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scenario {
    Comfort,
    Schedule,
    Chat,
    Diary,
    Todo,
    Rewrite,
    Letter,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompleteRequest {
    pub scenario: Scenario,
    pub prompt_ver: String,
    pub messages: Vec<Message>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompleteResponse {
    pub model: String,
    pub text: String,
    pub latency_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Delta {
    pub text: String,
}

/// 一条出网记录（08 FR-AIG-05 第 3 条、09 D-20）：只有时间、接口、模型、字段名、耗时、状态与
/// token 用量，**不含任何请求或响应内容**。网关每发出一个 HTTP 请求写一条，由 Hub 存入 `net_log`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetLogEntry {
    /// Unix 毫秒
    pub ts: i64,
    /// `jev`、`llm/<场景>`、`llm/models`、`llm/test`
    pub api: String,
    pub model: Option<String>,
    /// 出网的字段名，例如 Jev 白名单过滤后的 state 键
    pub fields: Vec<String>,
    pub latency_ms: Option<u64>,
    /// `ok`、HTTP 状态码（如 `401`）或 `timeout` / `network` / `invalid_output` / `cancelled`
    pub status: String,
    pub tokens_in: Option<u32>,
    pub tokens_out: Option<u32>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayHealth {
    pub jev: bool,
    pub llm: bool,
}

impl GatewayHealth {
    pub fn offline(&self) -> bool {
        !self.jev || !self.llm
    }
}

#[async_trait]
pub trait AiGateway: Send + Sync {
    /// Jev 结构化判断；questions 必须来自问题目录。
    async fn judge(&self, req: JudgeRequest) -> Result<JudgeResponse, AiError>;
    /// 大模型一次性生成（暖心话、日程抽取、日记）。
    async fn complete(&self, req: CompleteRequest) -> Result<CompleteResponse, AiError>;
    /// 大模型流式生成（对话）。
    async fn stream(
        &self,
        req: CompleteRequest,
    ) -> Result<BoxStream<'static, Result<Delta, AiError>>, AiError>;
    /// 当前健康状态（小组件“离线模式”角标）。
    fn health(&self) -> GatewayHealth;
}

/// 未配置密钥时使用的离线网关：所有请求都返回 `ModelUnavailable`，
/// 领域服务据此走降级路径（03 第 8 节 L3：本地规则 + 模板）。
#[derive(Debug, Default, Clone, Copy)]
pub struct OfflineGateway;

#[async_trait]
impl AiGateway for OfflineGateway {
    async fn judge(&self, _req: JudgeRequest) -> Result<JudgeResponse, AiError> {
        Err(AiError::ModelUnavailable)
    }

    async fn complete(&self, _req: CompleteRequest) -> Result<CompleteResponse, AiError> {
        Err(AiError::ModelUnavailable)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_question_never_carries_text() {
        let mut state = serde_json::Map::new();
        state.insert("kpm".into(), 210.0.into());
        state.insert("committed_text".into(), "周五开会".into());
        state.insert("app".into(), "WeChat.exe".into());
        let req = JudgeRequest {
            questions: vec![Question::State, Question::Valence, Question::Comfort],
            state,
        };
        let f = req.filtered();
        assert!(f.contains_key("kpm"));
        assert!(
            !f.contains_key("committed_text"),
            "Q-STATE 不得带文本（FR-STA-05）"
        );
        assert!(!f.contains_key("app"), "进程名不出网（09 D-05）");
    }

    #[test]
    fn text_questions_are_redacted() {
        let mut state = serde_json::Map::new();
        state.insert("message".into(), "我的手机号是13812345678".into());
        let req = JudgeRequest {
            questions: vec![Question::Crisis],
            state,
        };
        assert_eq!(req.filtered()["message"], "我的手机号是[手机号]");
    }

    #[tokio::test]
    async fn offline_gateway_degrades() {
        let g = OfflineGateway;
        assert_eq!(
            g.judge(JudgeRequest::default()).await.unwrap_err(),
            AiError::ModelUnavailable
        );
        assert!(g.health().offline());
    }
}
