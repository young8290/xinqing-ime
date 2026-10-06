//! 日程与待办识别走真实的 HTTP 网关到进程内 mock-ai（C-05、C-06）：L2 Q-PLAN / Q-TODO、L3 P-SCHEDULE / P-TODO、
//! 日期以代码为准、出网的只有那一句、上游故障时按本地规则抽取。

use std::ops::Deref;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use chrono::{Local, TimeZone};
use mock_ai::{Handle, Scenario as Mock};
use xinqing_hub_core::domain::schedule::{
    CandidateKind, ExtractPrompts, FLAG_LOCAL, ScheduleRecognizer,
};
use xinqing_hub_core::domain::validate::BannedWords;
use xinqing_hub_core::infra::clock::ManualClock;
use xinqing_hub_core::infra::store::Db;
use xinqing_hub_core::infra::templates::TemplateDirs;
use xinqing_hub_core::schedule::{Ready, SchedulePort, ScheduleService};
use xinqing_hub_gateway::{GatewayConfig, HttpGateway, JevConfig, LlmConfig};
use zeroize::Zeroizing;

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

impl SchedulePort for Port {
    fn allowed(&self) -> bool {
        true
    }
    fn daily_cap(&self) -> usize {
        50
    }
    fn default_offsets(&self) -> Vec<i64> {
        vec![600]
    }
    fn db(&self) -> Box<dyn Deref<Target = Db> + '_> {
        Box::new(Guard(self.db.lock().unwrap()))
    }
    fn tip(&self, _kind: CandidateKind) {}
    fn detected(&self, _card_id: u32, _kind: CandidateKind) {}
    fn ready(&self, _r: &Ready) {}
}

async fn rig(llm_scenario: Mock) -> (Arc<ScheduleService>, Handle) {
    let (jev_base, _jev) = mock_ai::serve(Mock::Normal).await.unwrap();
    let (llm_base, llm_mock) = mock_ai::serve(llm_scenario).await.unwrap();
    let mut llm = LlmConfig::new(format!("{llm_base}/v1"));
    llm.models = vec![mock_ai::DEFAULT_MODEL.into()];
    // FR-SCH-04 验收用的“今天”：2026-10-03（周六）
    let clock = Arc::new(ManualClock::new(
        Local.with_ymd_and_hms(2026, 10, 3, 10, 0, 0).unwrap(),
    ));
    let gw = HttpGateway::new(
        GatewayConfig {
            jev: Some(JevConfig::new(format!("{jev_base}/v1/systemone"))),
            llm: Some(llm),
            caps: Vec::new(),
        },
        clock.clone(),
    )
    .unwrap();
    let dirs = TemplateDirs::factory_only(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../hub_templates"),
    );
    let service = Arc::new(ScheduleService::new(
        Arc::new(Port {
            db: Mutex::new(Db::open_in_memory().unwrap()),
        }),
        Arc::new(gw),
        clock,
        ScheduleRecognizer::load(&dirs).unwrap(),
        ExtractPrompts::load(&dirs).unwrap(),
        Arc::new(BannedWords::load(&dirs).unwrap()),
    ));
    (service, llm_mock)
}

#[tokio::test]
async fn schedule_goes_through_jev_and_llm() {
    let (s, mock) = rig(Mock::Normal).await;
    let r = s
        .process(
            CandidateKind::Schedule,
            Zeroizing::new("好的，周五下午三点在实验楼开组会".into()),
        )
        .await
        .expect("Q-PLAN 通过");
    let Ready::Schedule { row: Some(row), .. } = r else {
        panic!("{r:?}")
    };
    assert_eq!(row.title.as_deref(), Some("组会"));
    assert_eq!(
        (row.date.as_deref(), row.time.as_deref()),
        (Some("2026-10-09"), Some("15:00")),
        "日期时刻以代码为准"
    );
    assert!(!row.flags.contains(&FLAG_LOCAL.to_owned()));
    let body = mock.last_body().unwrap().to_string();
    assert!(body.contains("句子：好的，周五下午三点在实验楼开组会"));
}

#[tokio::test]
async fn todo_goes_through_jev_and_llm() {
    let (s, _mock) = rig(Mock::Normal).await;
    let r = s
        .process(CandidateKind::Todo, Zeroizing::new("记得明天交材料".into()))
        .await
        .expect("Q-TODO 通过");
    let Ready::Todo { row: Some(t), .. } = r else {
        panic!("{r:?}")
    };
    assert_eq!(t.due_date.as_deref(), Some("2026-10-04"));
    assert_eq!(t.status, "pending");
}

#[tokio::test]
async fn llm_failure_falls_back_to_local_rules() {
    let (s, _mock) = rig(Mock::E500).await;
    let r = s
        .process(
            CandidateKind::Schedule,
            Zeroizing::new("明晚八点前交数据库作业".into()),
        )
        .await
        .unwrap();
    let Ready::Schedule { row: Some(row), .. } = r else {
        panic!("{r:?}")
    };
    assert!(row.flags.contains(&FLAG_LOCAL.to_owned()));
    assert_eq!(row.title.as_deref(), Some("交数据库作业"));
}
