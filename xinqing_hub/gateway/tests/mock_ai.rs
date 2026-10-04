//! 用进程内的 mock-ai 跑网关的各种正常与故障场景（18 TC-AIG-01～05）。
//! 熔断与限速按 `ManualClock` 计时，超时用缩短后的配置，整个文件几秒内跑完。

use std::sync::Arc;
use std::time::Duration;

use chrono::Local;
use futures::StreamExt;
use mock_ai::{Handle, Scenario as Mock, BACKUP_MODEL, DEFAULT_MODEL, REPLY};
use tokio::sync::mpsc;
use xinqing_hub_core::infra::clock::ManualClock;
use xinqing_hub_core::infra::gateway::{
    AiError, AiGateway, Answer, BudgetKind, CompleteRequest, JudgeRequest, Message, NetLogEntry,
    Question, Scenario,
};
use xinqing_hub_gateway::{
    ApiKey, GatewayConfig, HttpGateway, JevConfig, LlmConfig, ScenarioParams,
};

struct Rig {
    gw: HttpGateway,
    mock: Handle,
    clock: Arc<ManualClock>,
    log: mpsc::UnboundedReceiver<NetLogEntry>,
}

impl Rig {
    fn drain(&mut self) -> Vec<NetLogEntry> {
        let mut out = Vec::new();
        while let Ok(e) = self.log.try_recv() {
            out.push(e);
        }
        out
    }
}

async fn rig_with(scenario: Mock, tweak: impl FnOnce(&mut GatewayConfig)) -> Rig {
    let (base, mock) = mock_ai::serve(scenario).await.unwrap();
    let mut jev = JevConfig::new(format!("{base}/v1/systemone"));
    jev.retry_backoff = Duration::from_millis(10);
    let mut llm = LlmConfig::new(format!("{base}/v1"));
    llm.models = vec![DEFAULT_MODEL.into(), BACKUP_MODEL.into()];
    let mut cfg = GatewayConfig {
        jev: Some(jev),
        llm: Some(llm),
        caps: Vec::new(),
    };
    tweak(&mut cfg);
    let clock = Arc::new(ManualClock::new(Local::now()));
    let (tx, log) = mpsc::unbounded_channel();
    let gw = HttpGateway::new(cfg, clock.clone())
        .unwrap()
        .with_net_log(tx);
    Rig {
        gw,
        mock,
        clock,
        log,
    }
}

async fn rig(scenario: Mock) -> Rig {
    rig_with(scenario, |_| {}).await
}

fn state_request() -> JudgeRequest {
    let mut state = serde_json::Map::new();
    state.insert("kpm".into(), 96.0.into());
    state.insert("local_hints".into(), "hesitation_hint".into());
    state.insert("committed_text".into(), "周五下午三点交报告".into());
    JudgeRequest {
        questions: vec![Question::State, Question::Comfort],
        state,
    }
}

fn chat(scenario: Scenario, text: &str) -> CompleteRequest {
    CompleteRequest {
        scenario,
        prompt_ver: "test-v1".into(),
        messages: vec![Message {
            role: "user".into(),
            content: text.into(),
        }],
    }
}

/// TC-AIG-03（Jev 一侧）：只发白名单字段，出网日志不记内容。
#[tokio::test]
async fn judge_sends_whitelisted_fields_only() {
    let mut r = rig(Mock::Normal).await;
    let resp = r.gw.judge(state_request()).await.unwrap();
    assert_eq!(resp.model, "jev-mock");
    assert!(
        matches!(&resp.answers[&Question::State], Answer::Choice { choice, .. } if choice == "hesitant")
    );
    assert!(matches!(
        resp.answers[&Question::Comfort],
        Answer::Noul { .. }
    ));
    assert_eq!(r.mock.jev(), 1);
    let sent = r.mock.last_body().unwrap();
    assert_eq!(
        sent["questions"],
        serde_json::json!(["Q-STATE", "Q-COMFORT"])
    );
    assert!(sent["state"].get("committed_text").is_none());
    assert_eq!(r.mock.last_auth(), None, "没配密钥就不带认证头");

    let log = r.drain();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].api, "jev");
    assert_eq!(log[0].status, "ok");
    assert_eq!(log[0].fields, ["kpm", "local_hints"], "Q-STATE 不带文本");
    let json = serde_json::to_string(&log).unwrap();
    assert!(
        !json.contains("报告") && !json.contains("hesitation"),
        "出网日志不记内容"
    );
}

#[tokio::test]
async fn judge_retries_5xx_once() {
    let mut r = rig(Mock::E500).await;
    assert_eq!(
        r.gw.judge(state_request()).await.unwrap_err(),
        AiError::Upstream
    );
    assert_eq!(r.mock.jev(), 2);
    let status: Vec<String> = r.drain().into_iter().map(|e| e.status).collect();
    assert_eq!(status, ["500", "500"]);
}

#[tokio::test]
async fn judge_does_not_retry_422() {
    let mut r = rig(Mock::E422).await;
    assert_eq!(
        r.gw.judge(state_request()).await.unwrap_err(),
        AiError::BadRequest
    );
    assert_eq!(r.mock.jev(), 1);
    assert_eq!(r.drain()[0].status, "422");
}

#[tokio::test]
async fn judge_times_out_and_retries() {
    let r = rig_with(Mock::Slow, |c| {
        c.jev.as_mut().unwrap().timeout = Duration::from_millis(200)
    })
    .await;
    assert_eq!(
        r.gw.judge(state_request()).await.unwrap_err(),
        AiError::Timeout
    );
    assert_eq!(r.mock.jev(), 2);
}

/// TC-AIG-01；顺带看 TC-AIG-08 的指标。
#[tokio::test]
async fn jev_breaker_opens_and_recovers() {
    let r = rig(Mock::E500).await;
    let mut health = r.gw.subscribe_health();
    assert!(health.borrow_and_update().jev);

    // 2 + 2 + 1：第 5 次失败打开熔断，第三次调用的重试被拦下，报告真实原因
    for _ in 0..3 {
        assert_eq!(
            r.gw.judge(state_request()).await.unwrap_err(),
            AiError::Upstream
        );
    }
    assert_eq!(r.mock.jev(), 5);
    assert_eq!(
        r.gw.judge(state_request()).await.unwrap_err(),
        AiError::CircuitOpen
    );
    assert_eq!(r.mock.jev(), 5, "熔断期间不发请求");
    assert!(health.has_changed().unwrap());
    assert!(!health.borrow_and_update().jev);
    assert!(r.gw.health().offline());

    // 60 秒后半开，放 1 个试探请求；服务恢复后试探成功，熔断关闭
    r.mock.set_scenario(Mock::Normal);
    r.clock.advance_ms(60_000);
    r.gw.judge(state_request()).await.unwrap();
    assert_eq!(r.mock.jev(), 6);
    assert!(health.borrow_and_update().jev);
    let jev = r.gw.metrics().into_iter().find(|m| m.api == "jev").unwrap();
    assert_eq!((jev.calls, jev.ok, jev.breaker_open), (6, 1, false));
}

/// TC-AIG-04：超出每日上限后不再发请求，领域层据此改用本地规则。
#[tokio::test]
async fn budget_exhaustion_sends_nothing() {
    let r = rig_with(Mock::Normal, |c| c.caps = vec![(BudgetKind::Jev, 1)]).await;
    r.gw.judge(state_request()).await.unwrap();
    assert_eq!(
        r.gw.judge(state_request()).await.unwrap_err(),
        AiError::BudgetExceeded
    );
    assert_eq!(r.mock.jev(), 1);
    assert_eq!(r.gw.budget_used(BudgetKind::Jev), 1);
}

#[tokio::test]
async fn jev_rate_limit_per_minute() {
    let r = rig_with(Mock::Normal, |c| c.jev.as_mut().unwrap().per_minute = 2).await;
    r.gw.judge(state_request()).await.unwrap();
    r.gw.judge(state_request()).await.unwrap();
    assert_eq!(
        r.gw.judge(state_request()).await.unwrap_err(),
        AiError::RateLimited
    );
    assert_eq!(r.mock.jev(), 2);
    r.clock.advance_ms(60_000);
    r.gw.judge(state_request()).await.unwrap();
    assert_eq!(r.mock.jev(), 3);
}

/// TC-AIG-03：出网文本里的手机号被替换为 `[手机号]`；出网日志只有 token 用量。
#[tokio::test]
async fn complete_records_tokens_but_no_content() {
    let mut r = rig(Mock::Normal).await;
    let resp =
        r.gw.complete(chat(Scenario::Comfort, "我的手机号13812345678"))
            .await
            .unwrap();
    assert_eq!(resp.text, REPLY);
    assert_eq!(resp.model, DEFAULT_MODEL);
    let sent = r.mock.last_body().unwrap();
    assert_eq!(sent["messages"][0]["content"], "我的手机号[手机号]");
    assert_eq!(sent["temperature"], 0.8);
    assert_eq!(sent["max_tokens"], 1024);
    let log = r.drain();
    assert_eq!(log.len(), 1);
    let e = &log[0];
    assert_eq!(e.api, "llm/comfort");
    assert_eq!((e.tokens_in, e.tokens_out), (Some(10), Some(16)));
    let json = serde_json::to_string(&log).unwrap();
    assert!(!json.contains("1381234") && !json.contains(REPLY));
    assert_eq!(r.gw.budget_used(BudgetKind::Llm), 1);
}

/// TC-AIG-02：优先级第一的模型 `model_not_found` → 换下一个模型，并标为不可用。
#[tokio::test]
async fn model_not_found_falls_back_to_next_model() {
    let mut r = rig(Mock::ModelNotFound).await;
    let resp =
        r.gw.complete(chat(Scenario::Schedule, "明天三点开会"))
            .await
            .unwrap();
    assert_eq!(resp.model, BACKUP_MODEL);
    assert_eq!(r.mock.chat(), 2);
    let status: Vec<String> = r.drain().into_iter().map(|e| e.status).collect();
    assert_eq!(status, ["404", "ok"]);
    let models = r.gw.models();
    assert!(!models[0].available, "model_not_found 的模型标为不可用");
    assert!(models[1].available);

    // 之后直接用备选模型，不再撞 404
    r.gw.complete(chat(Scenario::Schedule, "后天交作业"))
        .await
        .unwrap();
    assert_eq!(r.mock.chat(), 3);
}

/// TC-AIG-07：`/models` 没列出的模型标为不可用。
#[tokio::test]
async fn refresh_models_marks_missing_models() {
    let r = rig(Mock::ModelNotFound).await;
    let ids = r.gw.refresh_models().await.unwrap();
    assert!(ids.contains(&BACKUP_MODEL.to_string()));
    assert_eq!(r.mock.models(), 1);
    let models = r.gw.models();
    assert_eq!((models[0].available, models[1].available), (false, true));
    r.gw.complete(chat(Scenario::Comfort, "累了"))
        .await
        .unwrap();
    assert_eq!(r.mock.chat(), 1, "跳过不可用的模型");
    assert!(!r.gw.health().offline());
}

#[tokio::test]
async fn llm_5xx_switches_model_once() {
    let r = rig(Mock::E500).await;
    assert_eq!(
        r.gw.complete(chat(Scenario::Diary, "今天"))
            .await
            .unwrap_err(),
        AiError::Upstream
    );
    assert_eq!(r.mock.chat(), 2, "换下一个模型重试 1 次");
    let m = r.gw.models();
    assert_eq!(m[0].success_rate, Some(0.0));
}

#[tokio::test]
async fn llm_breaker_opens_per_model() {
    let r = rig(Mock::E500).await;
    for _ in 0..3 {
        let _ = r.gw.complete(chat(Scenario::Diary, "今天")).await;
    }
    assert_eq!(r.mock.chat(), 6);
    assert_eq!(
        r.gw.complete(chat(Scenario::Diary, "今天"))
            .await
            .unwrap_err(),
        AiError::CircuitOpen
    );
    assert_eq!(r.mock.chat(), 6);
    assert!(!r.gw.health().llm);
    r.clock.advance_ms(5 * 60_000);
    assert!(r.gw.health().llm, "5 分钟后可以试探");
}

#[tokio::test]
async fn stream_concatenates_to_reply() {
    let mut r = rig(Mock::Normal).await;
    let s = r.gw.stream(chat(Scenario::Chat, "睡不着")).await.unwrap();
    let parts: Vec<_> = s.collect().await;
    let text: String = parts.into_iter().map(|d| d.unwrap().text).collect();
    assert_eq!(text, REPLY);
    let log = r.drain();
    assert_eq!(log.len(), 1);
    assert_eq!(
        (log[0].api.as_str(), log[0].status.as_str()),
        ("llm/chat", "ok")
    );
    assert_eq!(r.gw.budget_used(BudgetKind::ChatTurn), 1);
}

#[tokio::test]
async fn stream_cut_yields_error_item() {
    let mut r = rig(Mock::StreamCut).await;
    let s = r.gw.stream(chat(Scenario::Chat, "睡不着")).await.unwrap();
    let parts: Vec<_> = s.collect().await;
    let (last, deltas) = parts.split_last().unwrap();
    assert!(!deltas.is_empty() && deltas.iter().all(Result::is_ok));
    assert_eq!(last.as_ref().unwrap_err(), &AiError::Network);
    assert_eq!(r.drain()[0].status, "network");
    assert_eq!(r.gw.models()[0].success_rate, Some(0.0));
}

#[tokio::test]
async fn stream_dropped_early_is_logged_as_cancelled() {
    let mut r = rig(Mock::Normal).await;
    let mut s = r.gw.stream(chat(Scenario::Chat, "在吗")).await.unwrap();
    assert!(s.next().await.unwrap().is_ok());
    drop(s);
    let log = r.drain();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].status, "cancelled");
}

#[tokio::test]
async fn stream_first_token_timeout_switches_model() {
    let mut r = rig_with(Mock::Slow, |c| {
        let llm = c.llm.as_mut().unwrap();
        let mut p = ScenarioParams::default_for(Scenario::Chat);
        p.first_token_timeout = Some(Duration::from_millis(200));
        llm.overrides.insert(Scenario::Chat, p);
    })
    .await;
    assert_eq!(
        r.gw.stream(chat(Scenario::Chat, "在吗")).await.err(),
        Some(AiError::Timeout)
    );
    assert_eq!(r.mock.chat(), 2);
    let status: Vec<String> = r.drain().into_iter().map(|e| e.status).collect();
    assert_eq!(status, ["timeout", "timeout"]);
}

/// TC-AIG-07：“测试连接”逐个模型显示可用性和延迟。
#[tokio::test]
async fn test_connection_probes_every_model() {
    let r = rig(Mock::ModelNotFound).await;
    let probes = r.gw.test_connection().await;
    assert_eq!(probes.len(), 2);
    assert_eq!((probes[0].ok, probes[0].status.as_str()), (false, "404"));
    assert!(probes[1].ok);
    assert!(!r.gw.models()[0].available);
}

/// TC-AIG-05（网关部分）：配置了密钥时按 Bearer 发出，但出网日志和调试输出里都找不到它。
#[tokio::test]
async fn api_key_goes_only_into_the_auth_header() {
    const KEY: &str = "sk-test-not-real-7f3a";
    let mut r = rig_with(Mock::Normal, |c| {
        c.jev.as_mut().unwrap().api_key = Some(ApiKey::new(KEY));
        c.llm.as_mut().unwrap().api_key = Some(ApiKey::new(KEY));
    })
    .await;
    r.gw.judge(state_request()).await.unwrap();
    assert_eq!(
        r.mock.last_auth().as_deref(),
        Some(format!("Bearer {KEY}").as_str())
    );
    r.gw.complete(chat(Scenario::Todo, "记得买牛奶"))
        .await
        .unwrap();
    assert_eq!(
        r.mock.last_auth().as_deref(),
        Some(format!("Bearer {KEY}").as_str())
    );
    let log = serde_json::to_string(&r.drain()).unwrap();
    assert!(!log.contains(KEY) && !log.contains("7f3a"));
    let metrics = format!("{:?}{:?}", r.gw.metrics(), r.gw.models());
    assert!(!metrics.contains(KEY));
}
