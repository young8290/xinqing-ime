//! Jev 客户端（08 FR-AIG-02）：一个窗口的全部问题一次请求；连接 3 秒、总 5 秒；
//! 只对超时、5xx、网络错误重试 1 次；在途 ≤ 2；连续 5 次失败熔断 60 秒；每分钟 ≤ 60 次。
//!
//! 线上请求格式产品书还没给出，这里与 mock-ai 用同一暂定结构（questions + state → answers），
//! 拿到 Jev 接口文档后只改本文件的 `Wire*` 两个类型。

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio::sync::Semaphore;
use xinqing_hub_core::infra::gateway::breaker::BreakerState;
use xinqing_hub_core::infra::gateway::{
    AiError, Answer, Breaker, JudgeRequest, JudgeResponse, NetLogEntry, Question,
};

use crate::config::JevConfig;
use crate::http::{authed, client, send_json};
use crate::metrics::Shared;

const MINUTE_MS: i64 = 60_000;

#[derive(Serialize)]
struct WireRequest<'a> {
    model: &'a str,
    questions: &'a [Question],
    state: &'a Map<String, Value>,
}

#[derive(Deserialize)]
struct WireResponse {
    model: String,
    answers: HashMap<Question, Answer>,
}

pub(crate) struct JevClient {
    http: Client,
    cfg: JevConfig,
    breaker: Mutex<Breaker>,
    /// 最近一分钟内发出请求的时间（Unix 毫秒），限速用
    sent: Mutex<VecDeque<i64>>,
    in_flight: Semaphore,
    shared: Arc<Shared>,
}

/// 熔断器是否处于打开期（打开期已过、等待试探时不算）。
pub(crate) fn breaker_blocking(b: &Breaker, now_ms: i64) -> bool {
    match b.state() {
        BreakerState::Open { until_ms } => now_ms < until_ms,
        BreakerState::HalfOpen { probing } => probing,
        BreakerState::Closed { .. } => false,
    }
}

impl JevClient {
    pub fn new(cfg: JevConfig, shared: Arc<Shared>) -> Result<Self, reqwest::Error> {
        let http = client(cfg.connect_timeout)?;
        Ok(Self {
            http,
            breaker: Mutex::new(Breaker::jev()),
            sent: Mutex::new(VecDeque::new()),
            in_flight: Semaphore::new(cfg.max_in_flight.max(1)),
            cfg,
            shared,
        })
    }

    pub fn breaker_open(&self) -> bool {
        breaker_blocking(&self.breaker.lock().unwrap(), self.shared.now_ms())
    }

    /// 发请求前的放行检查：先限速再问熔断器。顺序不能反：熔断器半开时 `try_acquire`
    /// 会占掉唯一的试探名额，若之后又被限速拦下，就再也没有请求去结束半开状态。
    fn admit(&self) -> Result<(), AiError> {
        let now = self.shared.now_ms();
        let mut sent = self.sent.lock().unwrap();
        while sent.front().is_some_and(|&t| now - t >= MINUTE_MS) {
            sent.pop_front();
        }
        if sent.len() >= self.cfg.per_minute {
            return Err(AiError::RateLimited);
        }
        if !self.breaker.lock().unwrap().try_acquire(now) {
            return Err(AiError::CircuitOpen);
        }
        sent.push_back(now);
        Ok(())
    }

    pub async fn judge(&self, req: &JudgeRequest) -> Result<JudgeResponse, AiError> {
        let state = req.filtered();
        let fields: Vec<String> = state.keys().cloned().collect();
        let wire = WireRequest {
            model: &self.cfg.model,
            questions: &req.questions,
            state: &state,
        };
        let _permit = self.in_flight.acquire().await.expect("信号量不会被关闭");

        let mut previous = None;
        for attempt in 0..2 {
            if attempt > 0 {
                tokio::time::sleep(self.cfg.retry_backoff).await;
            }
            if let Err(e) = self.admit() {
                // 重试被限速或熔断拦下时，报告第一次失败的真实原因
                return Err(previous.unwrap_or(e));
            }
            let start = Instant::now();
            let request = self
                .http
                .post(&self.cfg.base_url)
                .timeout(self.cfg.timeout)
                .json(&wire);
            let out = send_json::<WireResponse>(authed(request, self.cfg.api_key.as_ref()))
                .await
                .and_then(|w| validate(w, &req.questions));
            let latency_ms = start.elapsed().as_millis() as u64;

            let now = self.shared.now_ms();
            {
                let mut b = self.breaker.lock().unwrap();
                match out.result {
                    Ok(_) => b.on_success(),
                    Err(_) => b.on_failure(now),
                }
            }
            self.shared.record("jev", out.result.is_ok(), latency_ms);
            let model = match &out.result {
                Ok(w) => w.model.clone(),
                Err(_) => self.cfg.model.clone(),
            };
            self.shared.log(NetLogEntry {
                ts: now,
                api: "jev".into(),
                model: Some(model),
                fields: fields.clone(),
                latency_ms: Some(latency_ms),
                status: out.status,
                tokens_in: None,
                tokens_out: None,
            });

            match out.result {
                Ok(w) => {
                    return Ok(JudgeResponse {
                        model: w.model,
                        latency_ms,
                        answers: w.answers,
                    })
                }
                Err(e) if attempt == 0 && e.retryable() => previous = Some(e),
                Err(e) => return Err(e),
            }
        }
        Err(previous.unwrap_or(AiError::Network))
    }
}

/// 每个问到的问题都要有答案；多出来的答案丢掉。
fn validate(mut w: WireResponse, asked: &[Question]) -> Result<WireResponse, AiError> {
    if asked.iter().any(|q| !w.answers.contains_key(q)) {
        return Err(AiError::InvalidOutput);
    }
    w.answers.retain(|q, _| asked.contains(q));
    Ok(w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_request_carries_only_filtered_state() {
        let mut state = Map::new();
        state.insert("kpm".into(), 180.0.into());
        state.insert("committed_text".into(), "明天三点开会".into());
        let req = JudgeRequest {
            questions: vec![Question::State],
            state,
        };
        let filtered = req.filtered();
        let wire = WireRequest {
            model: "jev-latest",
            questions: &req.questions,
            state: &filtered,
        };
        let v = serde_json::to_value(&wire).unwrap();
        assert_eq!(
            v,
            serde_json::json!({"model": "jev-latest", "questions": ["Q-STATE"], "state": {"kpm": 180.0}})
        );
    }

    #[test]
    fn missing_answers_are_invalid_output() {
        let w: WireResponse = serde_json::from_value(serde_json::json!({
            "model": "jev-mock",
            "answers": {"Q-COMFORT": {"kind": "noul", "p": 0.4}, "Q-CRISIS": {"kind": "noul", "p": 0.1}}
        }))
        .unwrap();
        let w = validate(w, &[Question::Comfort]).unwrap();
        assert_eq!(w.answers.len(), 1, "没问的问题不要");
        assert!(matches!(
            validate(w, &[Question::Comfort, Question::State]),
            Err(AiError::InvalidOutput)
        ));
    }

    #[test]
    fn breaker_blocking_ignores_expired_open() {
        let mut b = Breaker::new(1, 1000);
        assert!(!breaker_blocking(&b, 0));
        b.on_failure(0);
        assert!(breaker_blocking(&b, 999));
        assert!(!breaker_blocking(&b, 1000), "打开期已过，等着试探");
        assert!(b.try_acquire(1000));
        assert!(breaker_blocking(&b, 1001), "试探在途时不再放行");
    }
}
