//! 网关配置（14 第 4 节、08 FR-AIG-02/03）。默认值都取自产品书；测试可以把超时改短。

use std::collections::HashMap;
use std::fmt;
use std::time::Duration;

use xinqing_hub_core::infra::gateway::{BudgetKind, Scenario};
use zeroize::Zeroizing;

/// dev 构建下 mock-ai 的默认地址（14 第 3.3 节）。
pub const DEV_BASE: &str = "http://127.0.0.1:18080";
/// 大模型默认优先级列表（08 FR-AIG-03），团队实测后在设置里补充备选。
pub const DEFAULT_MODELS: &[&str] = &["gemini-3.7-flash"];

/// API 密钥。内存中用后清零（FR-AIG-06）；`Debug` 不输出内容，界面只显示末 4 位。
#[derive(Clone)]
pub struct ApiKey(Zeroizing<String>);

impl ApiKey {
    pub fn new(key: impl Into<String>) -> Self {
        Self(Zeroizing::new(key.into()))
    }

    pub(crate) fn expose(&self) -> &str {
        &self.0
    }

    /// 只露出末 4 位，例如 `••••a1b2`（FR-AIG-06）。
    pub fn masked(&self) -> String {
        let chars: Vec<char> = self.0.chars().collect();
        let tail: String = chars[chars.len().saturating_sub(4)..].iter().collect();
        format!("••••{tail}")
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiKey(<已隐藏>)")
    }
}

#[derive(Debug, Clone)]
pub struct JevConfig {
    /// 完整的接口地址，例如 `https://<中转>/v1/systemone`
    pub base_url: String,
    pub api_key: Option<ApiKey>,
    /// 默认 `jev-latest`，稳定后固定为 `jev-1.13.0`
    pub model: String,
    pub connect_timeout: Duration,
    /// 总超时
    pub timeout: Duration,
    /// 重试前的等待
    pub retry_backoff: Duration,
    /// 每分钟最多发出的请求数（含重试）
    pub per_minute: usize,
    /// 同时在途的请求数
    pub max_in_flight: usize,
}

impl JevConfig {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            api_key: None,
            model: "jev-latest".into(),
            connect_timeout: Duration::from_secs(3),
            timeout: Duration::from_secs(5),
            retry_backoff: Duration::from_millis(300),
            per_minute: 60,
            max_in_flight: 2,
        }
    }
}

/// 每个场景的调用参数（08 FR-AIG-03 分场景表）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScenarioParams {
    pub temperature: f32,
    /// 不设过小的值：推理类模型会先消耗思考 token（C-AI-03）
    pub max_tokens: u32,
    /// 总超时
    pub timeout: Duration,
    /// 流式调用等到第一个字的超时
    pub first_token_timeout: Option<Duration>,
}

impl ScenarioParams {
    pub fn default_for(s: Scenario) -> Self {
        let p = |temperature, secs| ScenarioParams {
            temperature,
            max_tokens: 1024,
            timeout: Duration::from_secs(secs),
            first_token_timeout: None,
        };
        match s {
            Scenario::Comfort => p(0.8, 8),
            Scenario::Schedule => p(0.0, 12),
            Scenario::Chat => ScenarioParams {
                max_tokens: 2048,
                first_token_timeout: Some(Duration::from_secs(15)),
                ..p(0.7, 90)
            },
            Scenario::Diary => p(0.6, 20),
            Scenario::Todo => p(0.0, 12),
            Scenario::Rewrite => p(0.7, 10),
            Scenario::Letter => p(0.8, 30),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LlmConfig {
    /// OpenAI 兼容接口的根地址，例如 `https://<中转>/v1`
    pub base_url: String,
    pub api_key: Option<ApiKey>,
    /// 按优先级排列
    pub models: Vec<String>,
    pub connect_timeout: Duration,
    /// 覆盖某些场景的默认参数
    pub overrides: HashMap<Scenario, ScenarioParams>,
}

impl LlmConfig {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            api_key: None,
            models: DEFAULT_MODELS.iter().map(|m| m.to_string()).collect(),
            connect_timeout: Duration::from_secs(3),
            overrides: HashMap::new(),
        }
    }

    pub fn params(&self, s: Scenario) -> ScenarioParams {
        self.overrides
            .get(&s)
            .copied()
            .unwrap_or_else(|| ScenarioParams::default_for(s))
    }

    pub(crate) fn url(&self, path: &str) -> String {
        format!("{}/{path}", self.base_url.trim_end_matches('/'))
    }
}

#[derive(Debug, Clone, Default)]
pub struct GatewayConfig {
    /// 没配置的一侧所有请求都返回 `ModelUnavailable`，领域服务走降级路径
    pub jev: Option<JevConfig>,
    pub llm: Option<LlmConfig>,
    /// 每日预算上限的覆盖值（FR-AIG-07，设置 `ai.daily_caps`）
    pub caps: Vec<(BudgetKind, u32)>,
}

impl GatewayConfig {
    /// dev 构建用：读环境变量 `XQ_JEV_BASE_URL`、`XQ_LLM_BASE_URL`，默认指向本机 mock-ai，不需要密钥
    /// （14 第 3.3 节）。
    pub fn dev_from_env() -> Self {
        let var = |k: &str, default: String| std::env::var(k).unwrap_or(default);
        Self {
            jev: Some(JevConfig::new(var(
                "XQ_JEV_BASE_URL",
                format!("{DEV_BASE}/v1/systemone"),
            ))),
            llm: Some(LlmConfig::new(var(
                "XQ_LLM_BASE_URL",
                format!("{DEV_BASE}/v1"),
            ))),
            caps: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_key_never_leaks_through_debug() {
        let k = ApiKey::new("sk-secret-a1b2");
        assert_eq!(format!("{k:?}"), "ApiKey(<已隐藏>)");
        assert_eq!(k.masked(), "••••a1b2");
        let cfg = JevConfig {
            api_key: Some(k),
            ..JevConfig::new("http://x")
        };
        assert!(!format!("{cfg:?}").contains("secret"));
        assert_eq!(ApiKey::new("ab").masked(), "••••ab");
    }

    #[test]
    fn scenario_defaults_follow_the_product_book() {
        let chat = ScenarioParams::default_for(Scenario::Chat);
        assert_eq!(chat.max_tokens, 2048);
        assert_eq!(chat.first_token_timeout, Some(Duration::from_secs(15)));
        assert_eq!(chat.timeout, Duration::from_secs(90));
        let comfort = ScenarioParams::default_for(Scenario::Comfort);
        assert_eq!((comfort.temperature, comfort.max_tokens), (0.8, 1024));
        assert_eq!(
            ScenarioParams::default_for(Scenario::Schedule).temperature,
            0.0
        );
    }

    #[test]
    fn url_join_tolerates_trailing_slash() {
        assert_eq!(
            LlmConfig::new("http://h/v1/").url("models"),
            "http://h/v1/models"
        );
        assert_eq!(
            LlmConfig::new("http://h/v1").url("chat/completions"),
            "http://h/v1/chat/completions"
        );
    }
}
