//! 暖心话服务走真实的 HTTP 网关到进程内 mock-ai（C-04）：P-COMFORT 生成、校验、出网日志不含内容、
//! 服务故障时退回本地模板（TC-CMF-03/04 的网关部分）。

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use chrono::Local;
use mock_ai::{COMFORT_REPLIES, Scenario as Mock};
use tokio::sync::mpsc;
use xinqing_hub_core::bus::HubEvent;
use xinqing_hub_core::care::{Comfort, ComfortPort, ComfortService, ComfortSource};
use xinqing_hub_core::domain::comfort::{CareLevel, ComfortPrompt, ComfortTemplates, Style};
use xinqing_hub_core::domain::self_report::SelfWeather;
use xinqing_hub_core::domain::validate::BannedWords;
use xinqing_hub_core::infra::clock::ManualClock;
use xinqing_hub_core::infra::gateway::NetLogEntry;
use xinqing_hub_core::infra::store::{ComfortRecord, Db};
use xinqing_hub_core::infra::templates::TemplateDirs;
use xinqing_hub_gateway::{GatewayConfig, HttpGateway, LlmConfig};

struct Port {
    db: Mutex<Db>,
}

impl ComfortPort for Port {
    fn level(&self) -> CareLevel {
        CareLevel::Normal
    }
    fn style(&self) -> Style {
        Style::Gentle
    }
    fn llm_allowed(&self) -> bool {
        true
    }
    fn recent_texts(&self) -> Vec<String> {
        self.db.lock().unwrap().comfort_recent_texts(10).unwrap()
    }
    fn blocked_templates(&self) -> HashSet<String> {
        HashSet::new()
    }
    fn auto_since(&self, since_ms: i64) -> (u32, Option<i64>) {
        self.db
            .lock()
            .unwrap()
            .comfort_auto_since(since_ms)
            .unwrap()
    }
    fn save(&self, rec: &ComfortRecord<'_>) -> Option<i64> {
        self.db.lock().unwrap().comfort_insert(rec).ok()
    }
    fn show(&self, _c: &Comfort) {}
}

async fn rig(scenario: Mock) -> (ComfortService, mpsc::UnboundedReceiver<NetLogEntry>) {
    let (base, _mock) = mock_ai::serve(scenario).await.unwrap();
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
    let service = ComfortService::new(
        Arc::new(Port {
            db: Mutex::new(Db::open_in_memory().unwrap()),
        }),
        Arc::new(gw),
        clock,
        ComfortTemplates::load(&dirs).unwrap(),
        ComfortPrompt::load(&dirs).unwrap(),
        Arc::new(BannedWords::load(&dirs).unwrap()),
    );
    (service, log)
}

fn rain() -> HubEvent {
    HubEvent::SelfReport {
        weather: SelfWeather::Rain,
        until: 0,
    }
}

#[tokio::test]
async fn comfort_goes_through_the_gateway_and_passes_validation() {
    let (mut s, mut log) = rig(Mock::Normal).await;
    let mut texts = Vec::new();
    for _ in 0..3 {
        let c = s.on_event(rain()).await.unwrap();
        assert_eq!(c.source, ComfortSource::Llm);
        texts.push(c.text);
    }
    // 三句都来自 mock-ai，且互不相同（V5）
    for t in &texts {
        assert!(
            COMFORT_REPLIES.iter().any(|r| r.contains(t.as_str())),
            "{t}"
        );
    }
    assert_eq!(texts.iter().collect::<HashSet<_>>().len(), 3);
    // 出网日志：接口是 llm/comfort，只有元数据
    let mut entries = Vec::new();
    while let Ok(e) = log.try_recv() {
        entries.push(e);
    }
    assert!(
        entries
            .iter()
            .any(|e| e.api == "llm/comfort" && e.status == "ok")
    );
    let dump = format!("{entries:?}");
    assert!(
        !dump.contains("深呼吸") && !dump.contains("low"),
        "出网日志不含内容"
    );
}

#[tokio::test]
async fn upstream_failure_falls_back_to_templates() {
    let (mut s, _log) = rig(Mock::E500).await;
    let c = s.on_event(rain()).await.unwrap();
    assert_eq!(c.source, ComfortSource::Template);
    assert!(!c.text.is_empty());
}
