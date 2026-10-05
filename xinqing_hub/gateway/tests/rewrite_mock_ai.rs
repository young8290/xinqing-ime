//! 温柔改写走真实的 HTTP 网关到进程内 mock-ai（C-09）：发出去的是占位符、结果换回原文、出网日志不含内容、
//! 上游故障时回 `offline`。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use chrono::Local;
use mock_ai::{Handle, Scenario as Mock};
use tokio::sync::mpsc;
use xinqing_hub_core::domain::rewrite::RewritePrompt;
use xinqing_hub_core::domain::safety::CrisisLexicon;
use xinqing_hub_core::domain::validate::BannedWords;
use xinqing_hub_core::infra::clock::ManualClock;
use xinqing_hub_core::infra::gateway::NetLogEntry;
use xinqing_hub_core::infra::templates::TemplateDirs;
use xinqing_hub_core::rewrite::{RewriteLog, RewritePort, RewriteService};
use xinqing_hub_gateway::{GatewayConfig, HttpGateway, LlmConfig};
use xqp::{Down, RewriteFailReason, RewriteSource, RewriteStyle};

#[derive(Default)]
struct Port {
    logs: Mutex<Vec<RewriteLog>>,
}

impl RewritePort for Port {
    fn allowed(&self) -> bool {
        true
    }
    fn reply(&self, _d: Down) {}
    fn crisis(&self, _ts: i64) {}
    fn log(&self, rec: RewriteLog) {
        self.logs.lock().unwrap().push(rec);
    }
}

async fn rig(
    scenario: Mock,
) -> (
    Arc<RewriteService>,
    Handle,
    mpsc::UnboundedReceiver<NetLogEntry>,
) {
    let (base, mock) = mock_ai::serve(scenario).await.unwrap();
    let mut llm = LlmConfig::new(format!("{base}/v1"));
    llm.models = vec![mock_ai::DEFAULT_MODEL.into()];
    let clock = Arc::new(ManualClock::new(Local::now()));
    let (tx, log) = mpsc::unbounded_channel();
    let gw = HttpGateway::new(
        GatewayConfig {
            jev: None,
            llm: Some(llm),
            caps: Vec::new(),
        },
        clock.clone(),
    )
    .unwrap()
    .with_net_log(tx);
    let dirs = TemplateDirs::factory_only(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../hub_templates"),
    );
    let service = RewriteService::new(
        Arc::new(Port::default()),
        Arc::new(gw),
        clock,
        RewritePrompt::load(&dirs).unwrap(),
        Arc::new(CrisisLexicon::load(&dirs).unwrap()),
        Arc::new(BannedWords::load(&dirs).unwrap()),
    );
    (service, mock, log)
}

const SRC: &str = "周五15:30前把报告发我，急事打13812345678，找@小王也行";

#[tokio::test]
async fn rewrite_goes_through_the_gateway_with_placeholders() {
    let (s, mock, mut log) = rig(Mock::Normal).await;
    let d = s
        .rewrite(1, RewriteSource::Recent, SRC, RewriteStyle::Gentle)
        .await;
    let Down::RewriteResult { cands, style, .. } = d else {
        panic!("{d:?}")
    };
    assert_eq!(style, RewriteStyle::Gentle);
    assert_eq!(cands.len(), 3);
    for c in &cands {
        assert!(c.contains("13812345678"), "号码由代码换回：{c}");
        assert!(c.contains("15:30") && c.contains("@小王"));
    }
    // 发出去的请求里只有占位符
    let body = mock.last_body().unwrap().to_string();
    assert!(body.contains("[号码1]"));
    assert!(!body.contains("13812345678"));
    // 出网日志：接口是 llm/rewrite，只有元数据
    let mut entries = Vec::new();
    while let Ok(e) = log.try_recv() {
        entries.push(e);
    }
    assert!(
        entries
            .iter()
            .any(|e| e.api == "llm/rewrite" && e.status == "ok")
    );
    let dump = format!("{entries:?}");
    assert!(
        !dump.contains("报告") && !dump.contains("号码"),
        "出网日志不含内容"
    );
}

#[tokio::test]
async fn upstream_failure_is_reported_as_offline() {
    let (s, _mock, _log) = rig(Mock::E500).await;
    let d = s
        .rewrite(2, RewriteSource::Clipboard, SRC, RewriteStyle::Polite)
        .await;
    assert_eq!(
        d,
        Down::RewriteFail {
            req_id: 2,
            reason: RewriteFailReason::Offline
        }
    );
}
