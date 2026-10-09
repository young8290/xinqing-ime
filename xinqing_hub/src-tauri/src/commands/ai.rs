//! AI 服务命令（10 第 5.1 节“AI”组，FR-SET-08、FR-ONB-05，ADR 0012）。
//!
//! 密钥只进不出：`secrets_set` 收明文密钥后立即放进 `Zeroizing`，任何命令都只返回末 4 位。

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;
use xinqing_hub_core::infra::gateway::BudgetKind;
use xinqing_hub_gateway::{
    AiSecrets, ApiKey, HttpGateway, JevSecret, LlmSecret, MaskedSecrets, MaskedSide, SecretsError,
};

use crate::error::UiError;
use crate::gateway::{Ai, Source};
use crate::secrets::SecretsStoreError;
use crate::state::AppState;

/// 当前 AI 配置从哪里来。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum AiConfigSource {
    /// 设置页保存的
    Saved,
    /// 开发版读到了仓库根目录的 secrets.toml
    DevFile,
    /// 开发版连本机 mock-ai
    DevMock,
    /// 没有配置，离线模式
    None,
}

/// 设置页“AI 服务”展示的内容：地址、模型和密钥末 4 位，不含密钥本身。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct AiConfigView {
    pub source: AiConfigSource,
    pub jev: Option<JevView>,
    pub llm: Option<LlmView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct JevView {
    pub base_url: String,
    /// 例如 `••••a1b2`；没有密钥时为 `null`
    pub key_tail: Option<String>,
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct LlmView {
    pub base_url: String,
    pub key_tail: Option<String>,
    /// 按优先级排列
    pub models: Vec<String>,
}

/// 设置页保存的内容。某一侧为 `null` 表示清除这一侧；`api_key` 为 `null` 或空字符串表示沿用已保存的密钥。
#[derive(Debug, Clone, Deserialize, Type)]
pub struct AiConfigInput {
    pub jev: Option<JevInput>,
    pub llm: Option<LlmInput>,
}

#[derive(Debug, Clone, Deserialize, Type)]
pub struct JevInput {
    pub base_url: String,
    pub api_key: Option<String>,
    /// 为 `null` 时用 `jev-latest`
    pub model: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Type)]
pub struct LlmInput {
    pub base_url: String,
    pub api_key: Option<String>,
    /// 按优先级排列；为空时用默认模型
    pub models: Vec<String>,
}

/// “测试连接”里一个模型的结果（FR-AIG-04 第 4 条）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct ModelProbeView {
    pub model: String,
    pub ok: bool,
    pub latency_ms: u32,
    /// `ok`、HTTP 状态码或 `timeout` / `network` 等错误类别
    pub status: String,
}

/// 今日用量的一行（FR-AIG-07、FR-SET-08）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct UsageRow {
    pub kind: BudgetKind,
    pub used: u32,
    pub cap: u32,
}

/// 演示者视图与设置页读取的内存指标；不含地址、密钥、请求或响应正文。
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
pub struct GatewayMetricsView {
    pub apis: Vec<ApiMetricsView>,
    pub models: Vec<ModelStatusView>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
pub struct ApiMetricsView {
    pub api: String,
    #[specta(type = specta_typescript::Number)]
    pub calls: u64,
    #[specta(type = specta_typescript::Number)]
    pub ok: u64,
    /// 成功次数 / 调用次数（0～1）；没有调用时为空。
    pub success_rate: Option<f64>,
    #[specta(type = Option<specta_typescript::Number>)]
    pub p50_ms: Option<u64>,
    #[specta(type = Option<specta_typescript::Number>)]
    pub p95_ms: Option<u64>,
    pub breaker_open: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
pub struct ModelStatusView {
    pub model: String,
    pub available: bool,
    pub breaker_open: bool,
    /// 最近 20 次调用的成功率（0～1）；没有调用时为空。
    pub success_rate: Option<f64>,
    #[specta(type = Option<specta_typescript::Number>)]
    pub p50_ms: Option<u64>,
}

/// 设置页展示的出网记录；只包含接口、模型、字段名和计量信息，不含请求或响应正文。
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
pub struct NetLogView {
    /// Unix 毫秒；前端用 number 展示即可，时间不会超过 JavaScript 安全整数范围。
    #[specta(type = specta_typescript::Number)]
    pub ts: f64,
    pub api: String,
    pub model: Option<String>,
    pub fields: Vec<String>,
    pub latency_ms: Option<u32>,
    pub status: String,
    pub tokens_in: Option<u32>,
    pub tokens_out: Option<u32>,
}

/// 网关计数的几类。日程识别按 L2 确认的次数计（每句命中初筛的话发一次 Q-PLAN 或 Q-TODO，ADR 0032）；
/// 周信每周最多 2 次，计在“大模型”里，不单列（ADR 0030）。
const USAGE_KINDS: [BudgetKind; 5] = [
    BudgetKind::Jev,
    BudgetKind::Llm,
    BudgetKind::ChatTurn,
    BudgetKind::SchedulePrefilter,
    BudgetKind::Rewrite,
];

#[tauri::command]
#[specta::specta]
pub fn ai_config_get(ai: State<'_, Arc<Ai>>) -> Result<AiConfigView, UiError> {
    let (source, masked) = ai.view();
    Ok(view(source, masked))
}

/// 保存 AI 服务地址与密钥（FR-AIG-06）并立即换用新配置，返回保存后的样子。
#[tauri::command]
#[specta::specta]
pub async fn secrets_set(
    ai: State<'_, Arc<Ai>>,
    config: AiConfigInput,
) -> Result<AiConfigView, UiError> {
    ai.save(config.into_secrets())?;
    let (source, masked) = ai.view();
    Ok(view(source, masked))
}

/// 逐个模型发一条最小请求，返回可用性和延迟（FR-AIG-04 第 4 条）。没配置大模型时为空列表。
#[tauri::command]
#[specta::specta]
pub async fn ai_test_connection(ai: State<'_, Arc<Ai>>) -> Result<Vec<ModelProbeView>, UiError> {
    let probes = ai.gateway().test_connection().await;
    Ok(probes
        .into_iter()
        .map(|p| ModelProbeView {
            model: p.model,
            ok: p.ok,
            latency_ms: u32::try_from(p.latency_ms).unwrap_or(u32::MAX),
            status: p.status,
        })
        .collect())
}

#[tauri::command]
#[specta::specta]
pub fn ai_usage_today(ai: State<'_, Arc<Ai>>) -> Result<Vec<UsageRow>, UiError> {
    let gw = ai.gateway();
    Ok(USAGE_KINDS
        .into_iter()
        .map(|kind| UsageRow {
            kind,
            used: gw.budget_used(kind),
            cap: gw.budget_cap(kind),
        })
        .collect())
}

/// 读取当前网关的内存指标（FR-AIG-08），不发请求、不扣预算、不落盘。
/// 接口按名称排序，模型按配置优先级排列；Hub 重启或更换网关配置后统计清空。
#[tauri::command]
#[specta::specta]
pub fn ai_gateway_metrics(ai: State<'_, Arc<Ai>>) -> Result<GatewayMetricsView, UiError> {
    Ok(metrics_view(&ai.gateway()))
}

fn metrics_view(gw: &HttpGateway) -> GatewayMetricsView {
    GatewayMetricsView {
        apis: gw
            .metrics()
            .into_iter()
            .map(|m| ApiMetricsView {
                api: m.api,
                calls: m.calls,
                ok: m.ok,
                success_rate: (m.calls > 0).then(|| m.ok as f64 / m.calls as f64),
                p50_ms: m.p50_ms,
                p95_ms: m.p95_ms,
                breaker_open: m.breaker_open,
            })
            .collect(),
        models: gw
            .models()
            .into_iter()
            .map(|m| ModelStatusView {
                model: m.model,
                available: m.available,
                breaker_open: m.breaker_open,
                success_rate: m.success_rate,
                p50_ms: m.p50_ms,
            })
            .collect(),
    }
}

/// 返回设置页需要的最近出网记录（FR-SET-09、09 D-20）。数据库本身只保留最近 200 条，
/// 界面再取最近 20 条；记录不含用户输入、提示词或模型回复。
#[tauri::command]
#[specta::specta]
pub fn ai_net_log_recent(state: State<'_, AppState>) -> Result<Vec<NetLogView>, UiError> {
    Ok(state
        .db()
        .net_log_recent(20)?
        .into_iter()
        .map(|entry| NetLogView {
            ts: entry.ts as f64,
            api: entry.api,
            model: entry.model,
            fields: entry.fields,
            latency_ms: entry.latency_ms.and_then(|n| u32::try_from(n).ok()),
            status: entry.status,
            tokens_in: entry.tokens_in,
            tokens_out: entry.tokens_out,
        })
        .collect())
}

impl AiConfigInput {
    fn into_secrets(self) -> AiSecrets {
        let key = |k: Option<String>| k.filter(|k| !k.trim().is_empty()).map(ApiKey::new);
        AiSecrets {
            jev: self.jev.map(|j| JevSecret {
                base_url: j.base_url.trim().to_string(),
                api_key: key(j.api_key),
                model: j.model.filter(|m| !m.trim().is_empty()),
            }),
            llm: self.llm.map(|l| LlmSecret {
                base_url: l.base_url.trim().to_string(),
                api_key: key(l.api_key),
                models: l.models,
            }),
        }
    }
}

fn view(source: Source, m: MaskedSecrets) -> AiConfigView {
    let first = |s: &MaskedSide| s.models.first().cloned().unwrap_or_default();
    AiConfigView {
        source: match source {
            Source::Saved => AiConfigSource::Saved,
            Source::DevFile => AiConfigSource::DevFile,
            Source::DevMock => AiConfigSource::DevMock,
            Source::None => AiConfigSource::None,
        },
        jev: m.jev.as_ref().map(|j| JevView {
            base_url: j.base_url.clone(),
            key_tail: j.key_tail.clone(),
            model: first(j),
        }),
        llm: m.llm.map(|l| LlmView {
            base_url: l.base_url,
            key_tail: l.key_tail,
            models: l.models,
        }),
    }
}

impl From<SecretsStoreError> for UiError {
    fn from(e: SecretsStoreError) -> Self {
        match e {
            SecretsStoreError::Invalid(SecretsError::BadUrl(_) | SecretsError::EmptyModel(_)) => {
                UiError::new("ai.config_invalid", "error.ai_config_invalid")
            }
            SecretsStoreError::Unsupported => {
                UiError::new("ai.secrets_unsupported", "error.ai_secrets_unsupported")
            }
            other => UiError::internal("ai.secrets", other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xinqing_hub_core::infra::clock::SystemClock;
    use xinqing_hub_core::infra::gateway::{AiGateway, CompleteRequest, Message, Scenario};
    use xinqing_hub_gateway::{GatewayConfig, LlmConfig};

    #[test]
    fn offline_metrics_are_empty() {
        let gw = HttpGateway::new(GatewayConfig::default(), Arc::new(SystemClock)).unwrap();
        let v = metrics_view(&gw);
        assert!(v.apis.is_empty());
        assert!(v.models.is_empty());
    }

    #[tokio::test]
    async fn metrics_snapshot_exposes_stats_without_sending_requests_or_private_content() {
        let (base, mock) = mock_ai::serve(mock_ai::Scenario::Normal).await.unwrap();
        let mut llm = LlmConfig::new(format!("{base}/v1"));
        llm.models = vec![mock_ai::DEFAULT_MODEL.into()];
        llm.api_key = Some(ApiKey::new("test-metrics-private-key"));
        let gw = HttpGateway::new(
            GatewayConfig {
                jev: None,
                llm: Some(llm),
                caps: Vec::new(),
            },
            Arc::new(SystemClock),
        )
        .unwrap();
        let unused = metrics_view(&gw);
        assert!(unused.apis.is_empty());
        assert_eq!(unused.models[0].success_rate, None);
        assert_eq!(unused.models[0].p50_ms, None);
        assert_eq!(mock.chat(), 0);
        assert_eq!(mock.models(), 0);

        let request = || CompleteRequest {
            scenario: Scenario::Chat,
            prompt_ver: "metrics-test".into(),
            messages: vec![Message {
                role: "user".into(),
                content: "private-metrics-input".into(),
            }],
        };
        gw.complete(request()).await.unwrap();
        mock.set_scenario(mock_ai::Scenario::E422);
        for _ in 0..3 {
            assert!(gw.complete(request()).await.is_err());
        }
        let used = gw.budget_used(BudgetKind::ChatTurn);
        let before = (mock.chat(), mock.models(), mock.jev());
        let v = metrics_view(&gw);
        assert_eq!(v.apis.len(), 1);
        assert_eq!(v.apis[0].calls, 4);
        assert_eq!(v.apis[0].ok, 1);
        assert_eq!(v.apis[0].success_rate, Some(0.25));
        assert!(v.apis[0].p50_ms.is_some());
        assert_eq!(v.apis[0].p50_ms, v.apis[0].p95_ms);
        assert!(v.apis[0].breaker_open);
        assert!(v.models[0].breaker_open);
        assert_eq!(v.models[0].success_rate, Some(0.25));
        assert_eq!((mock.chat(), mock.models(), mock.jev()), before);
        assert_eq!(gw.budget_used(BudgetKind::ChatTurn), used);
        let json = serde_json::to_string(&v).unwrap();
        for private in [
            "test-metrics-private-key",
            "private-metrics-input",
            mock_ai::REPLY,
            &base,
        ] {
            assert!(!json.contains(private));
        }
        // 新网关的内存统计不继承；换配置只继承今日预算。
        let fresh = HttpGateway::new(GatewayConfig::default(), Arc::new(SystemClock))
            .unwrap()
            .inherit_budget(&gw);
        assert!(metrics_view(&fresh).apis.is_empty());
        assert_eq!(fresh.budget_used(BudgetKind::ChatTurn), used);
    }

    #[test]
    fn blank_key_means_keep_the_saved_one() {
        let input = AiConfigInput {
            jev: None,
            llm: Some(LlmInput {
                base_url: " https://llm.example/v1 ".into(),
                api_key: Some("  ".into()),
                models: vec!["m1".into()],
            }),
        };
        let s = input.into_secrets();
        let llm = s.llm.as_ref().unwrap();
        assert!(llm.api_key.is_none());
        assert_eq!(llm.base_url, "https://llm.example/v1");
        assert!(s.jev.is_none());
    }

    #[test]
    fn view_shows_only_the_key_tail() {
        let s = AiConfigInput {
            jev: Some(JevInput {
                base_url: "https://jev.example".into(),
                api_key: Some("jev-secret-0001".into()),
                model: None,
            }),
            llm: None,
        }
        .into_secrets();
        let v = view(Source::Saved, s.masked());
        let jev = v.jev.as_ref().unwrap();
        assert_eq!(jev.key_tail.as_deref(), Some("••••0001"));
        assert_eq!(jev.model, "jev-latest");
        assert!(!serde_json::to_string(&v).unwrap().contains("secret"));
    }

    #[test]
    fn invalid_config_maps_to_a_user_message() {
        let e: UiError = SecretsStoreError::Invalid(SecretsError::BadUrl("Jev")).into();
        assert_eq!(e.message_key, "error.ai_config_invalid");
        let e: UiError = SecretsStoreError::Unsupported.into();
        assert_eq!(e.code, "ai.secrets_unsupported");
    }
}
