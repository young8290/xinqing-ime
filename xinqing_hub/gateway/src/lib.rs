//! 心晴 AI 网关的 HTTP 实现（08 第 2 节、17 第 2.5 节）。
//!
//! [`HttpGateway`] 实现 core 里的 `AiGateway` trait，领域层只认 trait，不依赖本 crate 和 reqwest
//! （TC-AIG-06）。每次请求按 17 第 2.5 节的顺序处理：预算 → 选模型 → 隐私过滤 → 发送 →
//! 分类并更新熔断器与统计、写出网日志 → 重试。健康状态变化时通过 [`HttpGateway::subscribe_health`]
//! 通知 Hub 推送 `gateway:health`。
//!
//! 开发和测试对接 `tools/mock-ai`，不需要任何真实密钥（14 第 3.3 节）。

mod config;
mod error;
mod http;
mod jev;
mod llm;
mod metrics;
mod sse;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::stream::BoxStream;
use tokio::sync::{mpsc, watch};
use xinqing_hub_core::infra::clock::Clock;
use xinqing_hub_core::infra::gateway::{
    AiError, AiGateway, Budget, BudgetKind, CompleteRequest, CompleteResponse, Delta,
    GatewayHealth, JudgeRequest, JudgeResponse, NetLogEntry, Scenario,
};

pub use config::{
    ApiKey, GatewayConfig, JevConfig, LlmConfig, ScenarioParams, DEFAULT_MODELS, DEV_BASE,
};
pub use error::BuildError;
pub use llm::{ModelProbe, ModelStatus};
pub use metrics::ApiMetrics;

use jev::JevClient;
use llm::LlmClient;
use metrics::Shared;

/// 每类请求占用的每日预算（FR-AIG-07）。周信每周 2 次的限制由周信服务自己管。
fn budget_kind(s: Scenario) -> BudgetKind {
    match s {
        Scenario::Chat => BudgetKind::ChatTurn,
        Scenario::Rewrite => BudgetKind::Rewrite,
        _ => BudgetKind::Llm,
    }
}

pub struct HttpGateway {
    jev: Option<JevClient>,
    llm: Option<LlmClient>,
    budget: Mutex<Budget>,
    shared: Arc<Shared>,
    health: watch::Sender<GatewayHealth>,
}

impl HttpGateway {
    pub fn new(cfg: GatewayConfig, clock: Arc<dyn Clock>) -> Result<Self, BuildError> {
        let shared = Arc::new(Shared::new(clock));
        let jev = cfg
            .jev
            .map(|c| JevClient::new(c, shared.clone()))
            .transpose()?;
        let llm = cfg
            .llm
            .map(|c| LlmClient::new(c, shared.clone()))
            .transpose()?;
        let mut budget = Budget::new();
        for (kind, cap) in cfg.caps {
            budget.set_cap(kind, cap);
        }
        let gw = Self {
            jev,
            llm,
            budget: Mutex::new(budget),
            shared,
            health: watch::Sender::new(GatewayHealth::default()),
        };
        gw.publish_health();
        Ok(gw)
    }

    /// 出网日志交给 Hub 写入 `net_log`（09 D-20）。只能设置一次。
    pub fn with_net_log(self, tx: mpsc::UnboundedSender<NetLogEntry>) -> Self {
        self.shared.set_net_log(tx);
        self
    }

    pub fn subscribe_health(&self) -> watch::Receiver<GatewayHealth> {
        self.health.subscribe()
    }

    /// 重新计算健康状态，有变化才通知订阅者。每次请求后自动调用；熔断打开期结束不会自己触发，
    /// Hub 的定时任务（每 30 分钟刷新模型列表时）也会调用。
    pub fn publish_health(&self) {
        let now = self.current_health();
        self.health.send_if_modified(|h| {
            let changed = *h != now;
            *h = now;
            changed
        });
    }

    fn current_health(&self) -> GatewayHealth {
        GatewayHealth {
            jev: self.jev.as_ref().is_some_and(|j| !j.breaker_open()),
            llm: self.llm.as_ref().is_some_and(|l| l.healthy()),
        }
    }

    /// 启动时和之后每 30 分钟调用（FR-AIG-04 第 1 条），返回服务端列出的模型。
    pub async fn refresh_models(&self) -> Result<Vec<String>, AiError> {
        let llm = self.llm.as_ref().ok_or(AiError::ModelUnavailable)?;
        let r = llm.refresh_models().await;
        self.publish_health();
        r
    }

    /// 设置页“测试连接”（FR-AIG-04 第 4 条）。
    pub async fn test_connection(&self) -> Vec<ModelProbe> {
        let Some(llm) = &self.llm else {
            return Vec::new();
        };
        let r = llm.test_connection().await;
        self.publish_health();
        r
    }

    /// 各接口的调用统计（FR-AIG-08），演示者视图和设置页用。
    pub fn metrics(&self) -> Vec<ApiMetrics> {
        self.shared.snapshot(|api| match api.strip_prefix("llm/") {
            Some(model) => self.llm.as_ref().is_some_and(|l| l.breaker_open(model)),
            None => self.jev.as_ref().is_some_and(|j| j.breaker_open()),
        })
    }

    /// 各模型的可用性、熔断、成功率与 P50（FR-AIG-04 第 2 条）。
    pub fn models(&self) -> Vec<ModelStatus> {
        self.llm.as_ref().map(|l| l.models()).unwrap_or_default()
    }

    /// 今日用量（FR-SET-08）。
    pub fn budget_used(&self, kind: BudgetKind) -> u32 {
        let today = self.shared.clock.now().date_naive();
        self.budget.lock().unwrap().used(kind, today)
    }

    /// 设置 `ai.daily_caps` 改动后调用。
    pub fn set_cap(&self, kind: BudgetKind, cap: u32) {
        self.budget.lock().unwrap().set_cap(kind, cap);
    }

    fn take_budget(&self, kind: BudgetKind) -> Result<(), AiError> {
        let today = self.shared.clock.now().date_naive();
        if self.budget.lock().unwrap().try_take(kind, today) {
            Ok(())
        } else {
            Err(AiError::BudgetExceeded)
        }
    }

    fn llm_for(&self, s: Scenario) -> Result<&LlmClient, AiError> {
        let llm = self.llm.as_ref().ok_or(AiError::ModelUnavailable)?;
        self.take_budget(budget_kind(s))?;
        Ok(llm)
    }
}

#[async_trait]
impl AiGateway for HttpGateway {
    async fn judge(&self, req: JudgeRequest) -> Result<JudgeResponse, AiError> {
        let jev = self.jev.as_ref().ok_or(AiError::ModelUnavailable)?;
        if req.questions.is_empty() {
            return Err(AiError::BadRequest);
        }
        self.take_budget(BudgetKind::Jev)?;
        let r = jev.judge(&req).await;
        self.publish_health();
        r
    }

    async fn complete(&self, req: CompleteRequest) -> Result<CompleteResponse, AiError> {
        let r = self.llm_for(req.scenario)?.complete(&req).await;
        self.publish_health();
        r
    }

    async fn stream(
        &self,
        req: CompleteRequest,
    ) -> Result<BoxStream<'static, Result<Delta, AiError>>, AiError> {
        let r = self.llm_for(req.scenario)?.stream(&req).await;
        self.publish_health();
        r
    }

    fn health(&self) -> GatewayHealth {
        self.current_health()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xinqing_hub_core::infra::clock::SystemClock;

    #[tokio::test]
    async fn unconfigured_sides_degrade_without_network() {
        let gw = HttpGateway::new(GatewayConfig::default(), Arc::new(SystemClock)).unwrap();
        assert!(gw.health().offline());
        let req = JudgeRequest {
            questions: vec![xinqing_hub_core::infra::gateway::Question::State],
            ..Default::default()
        };
        assert_eq!(gw.judge(req).await.unwrap_err(), AiError::ModelUnavailable);
        assert_eq!(gw.budget_used(BudgetKind::Jev), 0, "没发请求不占预算");
        assert!(gw.test_connection().await.is_empty());
    }

    #[test]
    fn budget_kinds_follow_fr_aig_07() {
        assert_eq!(budget_kind(Scenario::Chat), BudgetKind::ChatTurn);
        assert_eq!(budget_kind(Scenario::Rewrite), BudgetKind::Rewrite);
        assert_eq!(budget_kind(Scenario::Comfort), BudgetKind::Llm);
        assert_eq!(budget_kind(Scenario::Schedule), BudgetKind::Llm);
    }
}
