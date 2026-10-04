//! HTTP 结果 → `AiError` 分类（08 FR-AIG-01）。

use reqwest::StatusCode;
use xinqing_hub_core::infra::gateway::AiError;

pub(crate) fn from_reqwest(e: &reqwest::Error) -> AiError {
    if e.is_timeout() {
        AiError::Timeout
    } else if e.is_decode() {
        AiError::InvalidOutput
    } else {
        AiError::Network
    }
}

/// 非 2xx 状态码。404 且错误码是 `model_not_found` 视为模型不可用（换下一个模型），其余 4xx 不重试。
pub(crate) fn from_status(status: StatusCode, body: &str) -> AiError {
    match status.as_u16() {
        429 => AiError::RateLimited,
        404 if body.contains("model_not_found") => AiError::ModelUnavailable,
        400..=499 => AiError::BadRequest,
        _ => AiError::Upstream,
    }
}

/// 出网日志里没有 HTTP 状态码时记的错误类别（不含任何响应内容）。
pub(crate) fn kind_label(e: &AiError) -> &'static str {
    match e {
        AiError::Timeout => "timeout",
        AiError::RateLimited => "rate_limited",
        AiError::ModelUnavailable => "model_unavailable",
        AiError::BadRequest => "bad_request",
        AiError::Upstream => "upstream",
        AiError::Network => "network",
        AiError::CircuitOpen => "circuit_open",
        AiError::BudgetExceeded => "budget_exceeded",
        AiError::InvalidOutput => "invalid_output",
    }
}

/// 创建 HTTP 客户端失败（TLS 后端初始化出错等），只在启动时可能发生。
#[derive(Debug, thiserror::Error)]
#[error("创建 HTTP 客户端失败：{0}")]
pub struct BuildError(#[from] reqwest::Error);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_classification() {
        let s = |c| StatusCode::from_u16(c).unwrap();
        assert_eq!(from_status(s(429), ""), AiError::RateLimited);
        assert_eq!(
            from_status(s(404), r#"{"error":{"code":"model_not_found"}}"#),
            AiError::ModelUnavailable
        );
        assert_eq!(from_status(s(404), "not here"), AiError::BadRequest);
        assert_eq!(from_status(s(422), ""), AiError::BadRequest);
        assert_eq!(from_status(s(401), ""), AiError::BadRequest);
        assert_eq!(from_status(s(500), ""), AiError::Upstream);
        assert_eq!(from_status(s(503), ""), AiError::Upstream);
        assert_eq!(kind_label(&AiError::InvalidOutput), "invalid_output");
    }
}
