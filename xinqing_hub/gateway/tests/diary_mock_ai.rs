//! 情绪日记走真实的 HTTP 网关到进程内 mock-ai（C-10）：发出去的是 P-DIARY、mock 的草稿能过校验、
//! 上游故障时给空白模板。

use std::ops::Deref;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use chrono::{Local, TimeZone};
use mock_ai::{Handle, Scenario as Mock};
use xinqing_hub_core::diary::{DiaryPort, DiaryService};
use xinqing_hub_core::domain::diary::{DiaryCopy, DiaryPrompt};
use xinqing_hub_core::domain::safety::CrisisLexicon;
use xinqing_hub_core::domain::validate::BannedWords;
use xinqing_hub_core::infra::clock::ManualClock;
use xinqing_hub_core::infra::store::{Db, NewMessage};
use xinqing_hub_core::infra::templates::TemplateDirs;
use xinqing_hub_gateway::{GatewayConfig, HttpGateway, LlmConfig};

struct Port {
    db: Mutex<Db>,
}

struct Guard<'a>(MutexGuard<'a, Db>);
impl Deref for Guard<'_> {
    type Target = Db;
    fn deref(&self) -> &Db {
        &self.0
    }
}

impl DiaryPort for Port {
    fn db(&self) -> Box<dyn Deref<Target = Db> + '_> {
        Box::new(Guard(self.db.lock().unwrap()))
    }
    fn summary_allowed(&self) -> bool {
        false
    }
    fn jev_allowed(&self) -> bool {
        false
    }
    fn safety(&self, _session_id: i64) {}
}

async fn rig(scenario: Mock) -> (DiaryService, Arc<Port>, Handle) {
    let (base, mock) = mock_ai::serve(scenario).await.unwrap();
    let mut llm = LlmConfig::new(format!("{base}/v1"));
    llm.models = vec![mock_ai::DEFAULT_MODEL.into()];
    let clock = Arc::new(ManualClock::new(
        Local.with_ymd_and_hms(2026, 10, 6, 21, 0, 0).unwrap(),
    ));
    let gw = HttpGateway::new(
        GatewayConfig {
            jev: None,
            llm: Some(llm),
            caps: Vec::new(),
        },
        clock.clone(),
    )
    .unwrap();
    let dirs = TemplateDirs::factory_only(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../hub_templates"),
    );
    let port = Arc::new(Port {
        db: Mutex::new(Db::open_in_memory().unwrap()),
    });
    let service = DiaryService::new(
        port.clone(),
        Arc::new(gw),
        clock,
        DiaryPrompt::load(&dirs).unwrap(),
        DiaryCopy::load(&dirs).unwrap(),
        Arc::new(CrisisLexicon::load(&dirs).unwrap()),
        Arc::new(BannedWords::load(&dirs).unwrap()),
    );
    (service, port, mock)
}

fn session(port: &Port) -> i64 {
    let db = port.db();
    let id = db.chat_session_create("聊天", 0).unwrap();
    db.chat_message_insert(&NewMessage {
        session_id: id,
        role: "user",
        content: "今天组会被老师说了，有点委屈",
        ts: 1,
        ai_generated: false,
        model: None,
        prompt_ver: None,
    })
    .unwrap();
    id
}

#[tokio::test]
async fn draft_goes_through_the_gateway() {
    let (s, port, mock) = rig(Mock::Normal).await;
    let id = session(&port);
    let d = s.generate(Some(id)).await.unwrap();
    assert!(d.ai_generated, "mock 的草稿能过长度、禁用词、语言校验");
    assert_eq!(d.text, mock_ai::DIARY_REPLY);
    let body = mock.last_body().unwrap().to_string();
    assert!(body.contains("组会被老师说了"));
    assert!(body.contains("今日状态摘要：（无）"), "没同意 ④ 不带摘要");
}

#[tokio::test]
async fn upstream_failure_gives_the_blank_template() {
    let (s, port, _mock) = rig(Mock::E500).await;
    let id = session(&port);
    let d = s.generate(Some(id)).await.unwrap();
    assert!(!d.ai_generated);
    assert!(d.text.starts_with("今天发生了"));
}
