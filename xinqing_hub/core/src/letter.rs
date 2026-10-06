//! 周信服务（C-10 第二部分，05 FR-REV-02）：每 [`TICK`] 看一次该不该写信，到了就读那周的统计，
//! 同意 ④ 时用 P-LETTER 让大模型写（校验不过重写一次），否则或失败时用本地兜底模板，存进 `letter` 表并推给界面。
//!
//! 出网的只有统计 JSON（[`WeeklyStats`]）。哪一周、统计口径、校验见 [`crate::domain::letter`]。
//! 与外壳之间只通过 [`LetterPort`]，本模块不依赖 Tauri（ADR 0007）。实现说明见 docs/adr/0030。

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Local, NaiveDate};
use tokio::time::MissedTickBehavior;

use crate::domain::comfort::Style;
use crate::domain::letter::{
    self, LetterFallback, LetterPrompt, MIN_VALID_DAYS, Reject, WeekFacts, WeeklyStats,
};
use crate::domain::validate::BannedWords;
use crate::infra::clock::Clock;
use crate::infra::gateway::{AiGateway, CompleteRequest, Message, Scenario};

/// 判断间隔。演示模式的时钟走 60 倍速，30 秒是演示时间的半小时。
pub const TICK: Duration = Duration::from_secs(30);
/// 大模型写的信校验不过时最多写几次（FR-REV-02：再不过就用模板）。
pub const MAX_TRIES: usize = 2;

/// 写好的一封信（还没存）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Letter {
    pub week_start: NaiveDate,
    pub content: String,
    /// 大模型写的；为假时是本地模板
    pub ai_generated: bool,
    pub model: Option<String>,
    pub prompt_ver: Option<String>,
    pub ts: i64,
}

/// 外壳提供给周信服务的能力。
pub trait LetterPort: Send + Sync {
    /// 设置 `review.letter.enabled`。
    fn enabled(&self) -> bool;
    /// 是否已同意 ④（大模型摘要）；没同意时只用本地模板，什么都不出网。
    fn llm_allowed(&self) -> bool;
    /// 设置 `care.style`。
    fn style(&self) -> Style;
    /// 内部键 `review.letter.last_week`。
    fn last_week(&self) -> Option<NaiveDate>;
    fn mark_week(&self, week: NaiveDate);
    /// 那周到 `now` 为止的统计；读不出来时为 `None`（这次不写，下次再试）。
    fn facts(&self, week: NaiveDate, now: DateTime<Local>) -> Option<WeekFacts>;
    /// 存进 `letter` 表，返回行号；失败时为 `None`。
    fn save(&self, l: &Letter) -> Option<i64>;
    /// 交给界面（`letter:new`）。
    fn show(&self, id: i64, l: &Letter);
    /// 运行日志，只会传入不含用户数据的内容（NFR-LOG）。
    fn note(&self, _msg: &str) {}
}

pub struct LetterService {
    port: Arc<dyn LetterPort>,
    gateway: Arc<dyn AiGateway>,
    clock: Arc<dyn Clock>,
    prompt: LetterPrompt,
    fallback: LetterFallback,
    banned: Arc<BannedWords>,
}

impl LetterService {
    pub fn new(
        port: Arc<dyn LetterPort>,
        gateway: Arc<dyn AiGateway>,
        clock: Arc<dyn Clock>,
        prompt: LetterPrompt,
        fallback: LetterFallback,
        banned: Arc<BannedWords>,
    ) -> Self {
        Self {
            port,
            gateway,
            clock,
            prompt,
            fallback,
            banned,
        }
    }

    pub async fn run(self) {
        let mut tick = tokio::time::interval(TICK);
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            self.on_tick().await;
        }
    }

    /// 看一次；写了信时返回它。
    pub async fn on_tick(&self) -> Option<Letter> {
        if !self.port.enabled() {
            return None;
        }
        let now = self.clock.now();
        let last = self.port.last_week();
        let week = letter::due_week(now, |w| last.is_some_and(|l| l >= w))?;
        let facts = self.port.facts(week, now)?;
        let stats = letter::stats(&facts);
        // 先记下再写：写信或存库失败也不在同一周反复调用大模型
        self.port.mark_week(week);
        if stats.valid_days < MIN_VALID_DAYS {
            self.port.note(&format!(
                "周信：{week} 那周有效天数 {} 天，不写",
                stats.valid_days
            ));
            return None;
        }
        let l = self.write(week, &stats, now).await;
        let id = self.port.save(&l)?;
        self.port.show(id, &l);
        Some(l)
    }

    async fn write(&self, week: NaiveDate, stats: &WeeklyStats, now: DateTime<Local>) -> Letter {
        let ts = now.timestamp_millis();
        if self.port.llm_allowed()
            && let Some((content, model)) = self.ask(stats).await
        {
            return Letter {
                week_start: week,
                content,
                ai_generated: true,
                model: Some(model),
                prompt_ver: Some(self.prompt.ver()),
                ts,
            };
        }
        Letter {
            week_start: week,
            content: self.fallback.render(stats),
            ai_generated: false,
            model: None,
            prompt_ver: None,
            ts,
        }
    }

    /// 让大模型写，校验不过重写一次；网关出错（离线、超时、额度用完）直接放弃。
    async fn ask(&self, stats: &WeeklyStats) -> Option<(String, String)> {
        let json = serde_json::to_string(stats).ok()?;
        let req = CompleteRequest {
            scenario: Scenario::Letter,
            prompt_ver: self.prompt.ver(),
            messages: vec![Message {
                role: "user".into(),
                content: self.prompt.render(self.port.style(), &json),
            }],
        };
        for _ in 0..MAX_TRIES {
            let resp = match self.gateway.complete(req.clone()).await {
                Ok(r) => r,
                Err(e) => {
                    self.port
                        .note(&format!("周信：大模型不可用（{e}），用模板"));
                    return None;
                }
            };
            let text = resp.text.trim();
            match letter::check(text, &json, &self.banned) {
                Ok(()) => return Some((text.to_string(), resp.model)),
                Err(r) => self
                    .port
                    .note(&format!("周信：校验没通过（{}）", reason(&r))),
            }
        }
        None
    }
}

fn reason(r: &Reject) -> &'static str {
    match r {
        Reject::Length => "长度",
        Reject::Banned => "禁用词",
        Reject::Facts => "数字不在统计里",
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Mutex;

    use async_trait::async_trait;
    use chrono::TimeZone;
    use futures::stream::BoxStream;
    use xqp::MoodState;

    use super::*;
    use crate::domain::letter::DayCounts;
    use crate::infra::clock::ManualClock;
    use crate::infra::gateway::{
        AiError, CompleteResponse, Delta, GatewayHealth, JudgeRequest, JudgeResponse,
    };
    use crate::infra::templates::TemplateDirs;

    #[derive(Default)]
    struct FakePort {
        llm: bool,
        valid_days: u32,
        last_week: Mutex<Option<NaiveDate>>,
        saved: Mutex<Vec<Letter>>,
        shown: Mutex<Vec<i64>>,
        reads: Mutex<u32>,
    }

    impl LetterPort for FakePort {
        fn enabled(&self) -> bool {
            true
        }
        fn llm_allowed(&self) -> bool {
            self.llm
        }
        fn style(&self) -> Style {
            Style::Gentle
        }
        fn last_week(&self) -> Option<NaiveDate> {
            *self.last_week.lock().unwrap()
        }
        fn mark_week(&self, week: NaiveDate) {
            *self.last_week.lock().unwrap() = Some(week);
        }
        fn facts(&self, _week: NaiveDate, _now: DateTime<Local>) -> Option<WeekFacts> {
            *self.reads.lock().unwrap() += 1;
            let mut days = vec![DayCounts::default(); 7];
            for d in days.iter_mut().take(self.valid_days as usize) {
                *d = DayCounts {
                    typing_min: 120,
                    rests_due: 2,
                    rests_done: 1,
                    water: 1,
                };
            }
            Some(WeekFacts {
                days,
                states: vec![(0, MoodState::Fluent)],
                ..Default::default()
            })
        }
        fn save(&self, l: &Letter) -> Option<i64> {
            let mut s = self.saved.lock().unwrap();
            s.push(l.clone());
            Some(s.len() as i64)
        }
        fn show(&self, id: i64, _l: &Letter) {
            self.shown.lock().unwrap().push(id);
        }
    }

    /// 依次回 `replies` 里的文字；用完了回离线。
    struct FakeGateway {
        replies: Mutex<Vec<Result<String, AiError>>>,
        calls: Mutex<Vec<CompleteRequest>>,
    }

    #[async_trait]
    impl AiGateway for FakeGateway {
        async fn judge(&self, _req: JudgeRequest) -> Result<JudgeResponse, AiError> {
            Err(AiError::ModelUnavailable)
        }
        async fn complete(&self, req: CompleteRequest) -> Result<CompleteResponse, AiError> {
            assert_eq!(req.scenario, Scenario::Letter);
            self.calls.lock().unwrap().push(req);
            let mut r = self.replies.lock().unwrap();
            if r.is_empty() {
                return Err(AiError::ModelUnavailable);
            }
            r.remove(0).map(|text| CompleteResponse {
                model: "fake".into(),
                text,
                latency_ms: 1,
            })
        }
        async fn stream(
            &self,
            _req: CompleteRequest,
        ) -> Result<BoxStream<'static, Result<Delta, AiError>>, AiError> {
            Err(AiError::ModelUnavailable)
        }
        fn health(&self) -> GatewayHealth {
            GatewayHealth::default()
        }
    }

    fn good_letter() -> String {
        format!(
            "这一周你辛苦啦。有效的 3 天里平均每天打字 2 小时，{}下周可以试试晚上早一点合上电脑。\n晴晴",
            "照顾自己做得挺好。".repeat(14)
        )
    }

    fn rig(
        llm: bool,
        valid_days: u32,
        replies: Vec<Result<String, AiError>>,
    ) -> (
        LetterService,
        Arc<FakePort>,
        Arc<FakeGateway>,
        Arc<ManualClock>,
    ) {
        let port = Arc::new(FakePort {
            llm,
            valid_days,
            ..Default::default()
        });
        let gw = Arc::new(FakeGateway {
            replies: Mutex::new(replies),
            calls: Mutex::default(),
        });
        // 2026-10-11 是周日
        let clock = Arc::new(ManualClock::new(
            Local.with_ymd_and_hms(2026, 10, 11, 19, 0, 0).unwrap(),
        ));
        let dirs = TemplateDirs::factory_only(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../hub_templates"),
        );
        let s = LetterService::new(
            port.clone(),
            gw.clone(),
            clock.clone(),
            LetterPrompt::load(&dirs).unwrap(),
            LetterFallback::load(&dirs).unwrap(),
            Arc::new(BannedWords::load(&dirs).unwrap()),
        );
        (s, port, gw, clock)
    }

    fn monday() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 5).unwrap()
    }

    #[tokio::test]
    async fn sunday_evening_llm_letter_once() {
        let (s, port, gw, clock) = rig(true, 3, vec![Ok(good_letter())]);
        // 19:00 时到期的是上一周；先当它已经处理过
        port.mark_week(monday() - chrono::Duration::days(7));
        assert!(s.on_tick().await.is_none(), "周日 20:00 前不写这周");
        assert_eq!(*port.reads.lock().unwrap(), 0, "没到点不读库");
        clock.advance_ms(60 * 60_000);
        let l = s.on_tick().await.expect("20:00 写");
        assert!(l.ai_generated);
        assert_eq!(l.week_start, monday());
        assert_eq!(l.prompt_ver.as_deref(), Some("P-LETTER v1"));
        let sent = gw.calls.lock().unwrap()[0].messages[0].content.clone();
        assert!(sent.contains("\"valid_days\":3"), "只发统计");
        assert!(s.on_tick().await.is_none(), "同一周不重写");
        assert_eq!(*port.shown.lock().unwrap(), [1]);
    }

    #[tokio::test]
    async fn bad_letters_are_retried_once_then_template() {
        let wrong = good_letter().replace("3 天", "5 天");
        let (s, _port, gw, clock) = rig(true, 3, vec![Ok(wrong.clone()), Ok(wrong)]);
        clock.advance_ms(60 * 60_000);
        let l = s.on_tick().await.unwrap();
        assert_eq!(gw.calls.lock().unwrap().len(), 2);
        assert!(!l.ai_generated);
        assert!(l.content.ends_with("晴晴"));
        assert_eq!(l.prompt_ver, None);

        let (s, _, gw, clock) = rig(true, 3, vec![Ok("太短".into()), Ok(good_letter())]);
        clock.advance_ms(60 * 60_000);
        assert!(s.on_tick().await.unwrap().ai_generated, "第二次通过就用");
        assert_eq!(gw.calls.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn no_consent_or_offline_uses_template_without_retry() {
        let (s, _, gw, clock) = rig(false, 3, vec![Ok(good_letter())]);
        clock.advance_ms(60 * 60_000);
        assert!(!s.on_tick().await.unwrap().ai_generated);
        assert!(gw.calls.lock().unwrap().is_empty(), "没同意 ④ 不出网");

        let (s, _, gw, clock) = rig(true, 3, vec![Err(AiError::BudgetExceeded)]);
        clock.advance_ms(60 * 60_000);
        assert!(!s.on_tick().await.unwrap().ai_generated);
        assert_eq!(gw.calls.lock().unwrap().len(), 1, "网关出错不重试");
    }

    #[tokio::test]
    async fn quiet_weeks_get_nothing_and_are_not_retried() {
        let (s, port, gw, clock) = rig(true, 2, vec![]);
        clock.advance_ms(60 * 60_000);
        assert!(s.on_tick().await.is_none());
        assert_eq!(*port.last_week.lock().unwrap(), Some(monday()));
        assert!(s.on_tick().await.is_none());
        assert_eq!(*port.reads.lock().unwrap(), 1, "跳过的那周不再读");
        assert!(gw.calls.lock().unwrap().is_empty());
        assert!(port.saved.lock().unwrap().is_empty());
    }
}
