//! 两个客户端共用的一次 HTTP 往返：发请求、读完响应、分类，并给出出网日志用的状态。

use std::time::Duration;

use reqwest::{Client, RequestBuilder, StatusCode};
use serde::de::DeserializeOwned;
use xinqing_hub_core::infra::gateway::AiError;

use crate::config::ApiKey;
use crate::error::{from_reqwest, from_status, kind_label};

/// 一次请求的结果和出网日志状态。
pub(crate) struct Outcome<T> {
    pub result: Result<T, AiError>,
    /// `ok`；拿到非 2xx 响应时是状态码（运维手册按 401 / 404 排障）；其余是错误类别
    pub status: String,
}

impl<T> Outcome<T> {
    pub fn ok(v: T) -> Self {
        Self {
            result: Ok(v),
            status: "ok".into(),
        }
    }

    pub fn err(e: AiError) -> Self {
        Self {
            status: kind_label(&e).into(),
            result: Err(e),
        }
    }

    pub fn http(code: StatusCode, body: &str) -> Self {
        Self {
            result: Err(from_status(code, body)),
            status: code.as_u16().to_string(),
        }
    }

    /// 2xx 但内容不合格（缺字段、空回复）。
    pub fn and_then<U>(self, f: impl FnOnce(T) -> Result<U, AiError>) -> Outcome<U> {
        match self.result.and_then(f) {
            Ok(v) => Outcome::ok(v),
            Err(e) if self.status == "ok" => Outcome::err(e),
            Err(e) => Outcome {
                result: Err(e),
                status: self.status,
            },
        }
    }
}

/// 建 HTTP 客户端。reqwest 用的是 `rustls-no-provider`，第一次建客户端前给进程装上 ring 后端
/// （已经装过时 `install_default` 返回错误，忽略即可）。
pub(crate) fn client(connect_timeout: Duration) -> Result<Client, reqwest::Error> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    Client::builder().connect_timeout(connect_timeout).build()
}

/// 有密钥才加 `Authorization: Bearer`（mock-ai 不需要）；reqwest 会把该请求头标为敏感，调试输出里不显示。
pub(crate) fn authed(req: RequestBuilder, key: Option<&ApiKey>) -> RequestBuilder {
    match key {
        Some(k) => req.bearer_auth(k.expose()),
        None => req,
    }
}

/// 发送并把 2xx 响应体解析成 `T`。错误响应体只用来识别 `model_not_found`，不进日志。
pub(crate) async fn send_json<T: DeserializeOwned>(req: RequestBuilder) -> Outcome<T> {
    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => return Outcome::err(from_reqwest(&e)),
    };
    let code = resp.status();
    if !code.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Outcome::http(code, &body);
    }
    match resp.bytes().await {
        Ok(b) => match serde_json::from_slice(&b) {
            Ok(v) => Outcome::ok(v),
            Err(_) => Outcome::err(AiError::InvalidOutput),
        },
        Err(e) => Outcome::err(from_reqwest(&e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 没装加密后端时 reqwest 会在建客户端时直接 panic；反复建也不能出错。
    #[test]
    fn client_builds_with_ring_backend() {
        client(Duration::from_secs(1)).unwrap();
        client(Duration::from_secs(1)).unwrap();
    }
}
