//! 对话服务走真实的 HTTP 网关到进程内 mock-ai（C-07、C-08）：SSE 流式回复写库、同时问 Q-CRISIS、
//! 出网日志不含内容；流中途断开时给出“晴晴有点卡”且不保存半截回复。

use std::ops::Deref;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Local;
use mock_ai::{DEFAULT_MODEL, REPLY, Scenario as Mock};
use tokio::sync::mpsc;
use xinqing_hub_core::chat::{ChatEvent, ChatFailure, ChatPort, ChatService};
use xinqing_hub_core::domain::chat::{ChatCopy, ChatPrompts};
use xinqing_hub_core::domain::comfort::Style;
use xinqing_hub_core::domain::safety::CrisisLexicon;
use xinqing_hub_core::domain::validate::BannedWords;
use xinqing_hub_core::infra::clock::ManualClock;
use xinqing_hub_core::infra::gateway::NetLogEntry;
use xinqing_hub_core::infra::store::Db;
use xinqing_hub_core::infra::templates::TemplateDirs;
use xinqing_hub_gateway::{GatewayConfig, HttpGateway, JevConfig, LlmConfig};

struct Port {
    db: Mutex<Db>,
    events: mpsc::UnboundedSender<ChatEvent>,
}

impl ChatPort for Port {
    fn db(&self) -> Box<dyn Deref<Target = Db> + '_> {
        Box::new(self.db.lock().unwrap())
    }
    fn style(&self) -> Style {
        Style::Gentle
    }
    fn summary_allowed(&self) -> bool {
        false
    }
    fn emit(&self, ev: ChatEvent) {
        let _ = self.events.send(ev);
    }
    fn safety(&self, _session_id: i64) {}
}

struct Rig {
    service: Arc<ChatService>,
    port: Arc<Port>,
    events: mpsc::UnboundedReceiver<ChatEvent>,
    log: mpsc::UnboundedReceiver<NetLogEntry>,
}

async fn rig(scenario: Mock) -> Rig {
    let (base, _mock) = mock_ai::serve(scenario).await.unwrap();
    let mut llm = LlmConfig::new(format!("{base}/v1"));
    llm.models = vec![DEFAULT_MODEL.into()];
    let mut jev = JevConfig::new(format!("{base}/v1/systemone"));
    jev.retry_backoff = Duration::from_millis(10);
    let clock = Arc::new(ManualClock::new(Local::now()));
    let (tx, log) = mpsc::unbounded_channel();
    let gw = HttpGateway::new(
        GatewayConfig {
            jev: Some(jev),
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
    let (etx, events) = mpsc::unbounded_channel();
    let port = Arc::new(Port {
        db: Mutex::new(Db::open_in_memory().unwrap()),
        events: etx,
    });
    let service = Arc::new(ChatService::new(
        port.clone(),
        Arc::new(gw),
        clock,
        ChatPrompts::load(&dirs).unwrap(),
        ChatCopy::load(&dirs).unwrap(),
        Arc::new(BannedWords::load(&dirs).unwrap()),
        Arc::new(CrisisLexicon::load(&dirs).unwrap()),
    ));
    Rig {
        service,
        port,
        events,
        log,
    }
}

/// 收事件直到回复结束（done 或 error）。
async fn until_end(events: &mut mpsc::UnboundedReceiver<ChatEvent>) -> (String, ChatEvent) {
    let mut streamed = String::new();
    loop {
        let ev = tokio::time::timeout(Duration::from_secs(10), events.recv())
            .await
            .expect("10 秒内应结束")
            .unwrap();
        match ev {
            ChatEvent::Delta { text, .. } => streamed.push_str(&text),
            end => return (streamed, end),
        }
    }
}

#[tokio::test]
async fn streams_reply_through_http_gateway() {
    let mut r = rig(Mock::Normal).await;
    let sent = r.service.send(None, "今天有点累").unwrap();
    let (streamed, end) = until_end(&mut r.events).await;
    assert_eq!(streamed, REPLY);
    let ChatEvent::Done {
        text, ai_generated, ..
    } = end
    else {
        panic!("{end:?}")
    };
    assert_eq!(text, REPLY);
    assert!(ai_generated);
    let msgs = r
        .port
        .db
        .lock()
        .unwrap()
        .chat_messages(sent.session_id)
        .unwrap();
    assert_eq!(msgs.len(), 2);
    // 等 Q-CRISIS 也回来
    tokio::time::sleep(Duration::from_millis(200)).await;
    let mut apis = Vec::new();
    while let Ok(e) = r.log.try_recv() {
        assert!(
            !format!("{e:?}").contains("今天有点累"),
            "出网日志不含内容（D-20）"
        );
        apis.push(e.api);
    }
    assert!(apis.iter().any(|a| a == "jev"), "{apis:?}");
    assert!(apis.iter().any(|a| a.starts_with("llm/")), "{apis:?}");
}

#[tokio::test]
async fn cut_stream_reports_stuck_and_keeps_user_message() {
    let mut r = rig(Mock::StreamCut).await;
    let sent = r.service.send(None, "你好").unwrap();
    let (_, end) = until_end(&mut r.events).await;
    assert!(
        matches!(
            end,
            ChatEvent::Error {
                reason: ChatFailure::Stuck,
                ..
            }
        ),
        "{end:?}"
    );
    let msgs = r
        .port
        .db
        .lock()
        .unwrap()
        .chat_messages(sent.session_id)
        .unwrap();
    assert_eq!(msgs.len(), 1, "只有用户消息");
}
