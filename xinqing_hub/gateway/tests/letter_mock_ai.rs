//! 周信走真实的 HTTP 网关到进程内 mock-ai（C-10）：发出去的只有统计 JSON、mock 的信能过校验、
//! 上游故障时用本地模板。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Local, NaiveDate, TimeZone};
use mock_ai::{Handle, Scenario as Mock};
use xinqing_hub_core::domain::comfort::Style;
use xinqing_hub_core::domain::letter::{DayCounts, LetterFallback, LetterPrompt, WeekFacts};
use xinqing_hub_core::domain::validate::BannedWords;
use xinqing_hub_core::infra::clock::ManualClock;
use xinqing_hub_core::infra::templates::TemplateDirs;
use xinqing_hub_core::letter::{Letter, LetterPort, LetterService};
use xinqing_hub_gateway::{GatewayConfig, HttpGateway, LlmConfig};
use xqp::MoodState;

#[derive(Default)]
struct Port {
    last: Mutex<Option<NaiveDate>>,
}

impl LetterPort for Port {
    fn enabled(&self) -> bool {
        true
    }
    fn llm_allowed(&self) -> bool {
        true
    }
    fn style(&self) -> Style {
        Style::Gentle
    }
    fn last_week(&self) -> Option<NaiveDate> {
        *self.last.lock().unwrap()
    }
    fn mark_week(&self, week: NaiveDate) {
        *self.last.lock().unwrap() = Some(week);
    }
    fn facts(&self, _week: NaiveDate, _now: DateTime<Local>) -> Option<WeekFacts> {
        let day = DayCounts {
            typing_min: 150,
            rests_due: 3,
            rests_done: 2,
            water: 2,
        };
        Some(WeekFacts {
            days: vec![day; 4],
            states: vec![(0, MoodState::Fluent)],
            schedules_done: 4,
            ..Default::default()
        })
    }
    fn save(&self, _l: &Letter) -> Option<i64> {
        Some(1)
    }
    fn show(&self, _id: i64, _l: &Letter) {}
}

async fn rig(scenario: Mock) -> (LetterService, Handle) {
    let (base, mock) = mock_ai::serve(scenario).await.unwrap();
    let mut llm = LlmConfig::new(format!("{base}/v1"));
    llm.models = vec![mock_ai::DEFAULT_MODEL.into()];
    // 2026-10-11 是周日
    let clock = Arc::new(ManualClock::new(
        Local.with_ymd_and_hms(2026, 10, 11, 20, 30, 0).unwrap(),
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
    let service = LetterService::new(
        Arc::new(Port::default()),
        Arc::new(gw),
        clock,
        LetterPrompt::load(&dirs).unwrap(),
        LetterFallback::load(&dirs).unwrap(),
        Arc::new(BannedWords::load(&dirs).unwrap()),
    );
    (service, mock)
}

#[tokio::test]
async fn letter_goes_through_the_gateway_with_only_stats() {
    let (s, mock) = rig(Mock::Normal).await;
    let l = s.on_tick().await.expect("周日 20:30 写这周的信");
    assert!(l.ai_generated, "mock 的信能过长度、禁用词、V9");
    assert_eq!(l.content, mock_ai::LETTER_REPLY);
    let body = mock.last_body().unwrap().to_string();
    assert!(body.contains("本周统计："));
    assert!(body.contains(r#"\"valid_days\":4"#), "{body}");
}

#[tokio::test]
async fn upstream_failure_falls_back_to_the_template() {
    let (s, _mock) = rig(Mock::E500).await;
    let l = s.on_tick().await.unwrap();
    assert!(!l.ai_generated);
    assert!(l.content.contains("2 小时 30 分"), "{}", l.content);
}
