//! AI 服务命令（10 第 5.1 节“AI”组，FR-SET-08、FR-ONB-05，ADR 0012）。
//!
//! 密钥只进不出：`secrets_set` 收明文密钥后立即放进 `Zeroizing`，任何命令都只返回末 4 位。

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;
use xinqing_hub_core::infra::gateway::BudgetKind;
use xinqing_hub_gateway::{
    AiSecrets, ApiKey, JevSecret, LlmSecret, MaskedSecrets, MaskedSide, SecretsError,
};

use crate::error::UiError;
use crate::gateway::{Ai, Source};
use crate::secrets::SecretsStoreError;

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

/// 网关自己计数的几类；日程初筛（C-05）和周信（C-10）的次数由各自的服务计，接入后再加进来。
const USAGE_KINDS: [BudgetKind; 4] = [
    BudgetKind::Jev,
    BudgetKind::Llm,
    BudgetKind::ChatTurn,
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
