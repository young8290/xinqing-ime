//! 状态反馈（FR-STA-07）：小组件悬停状态、看板时间线状态点上的“准 / 不准”。
//!
//! 反馈写入 `feedback` 表（09 D-17），“不准”再交给融合按 FR-STA-06 第 8 条上调个人阈值
//! （30 分钟内对同一状态 3 次 → +0.05，持续 7 天）。上调状态只在内存里，Hub 重启时用
//! [`recent_unfit`] 从库里重放最近 7 天的“不准”恢复。
//!
//! 暖心话的 👍 / 👎 写 `comfort_log.feedback`（FR-CMF-05），不走这里。

use serde::{Deserialize, Serialize};
use xqp::MoodState;

use crate::infra::store::{Db, StoreError};

/// 反馈对象（`feedback.target`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum FeedbackTarget {
    /// 一条状态记录（`mood_state`）
    MoodState,
}

impl FeedbackTarget {
    pub fn as_str(self) -> &'static str {
        match self {
            FeedbackTarget::MoodState => "mood_state",
        }
    }
}

/// “准 / 不准”（`feedback.verdict`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Fit,
    Unfit,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Fit => "fit",
            Verdict::Unfit => "unfit",
        }
    }
}

/// 重启时重放多久以内的“不准”：上调持续 7 天，再加上凑满 3 次的 30 分钟。
pub const REPLAY_MS: i64 = 7 * 24 * 3_600_000 + 30 * 60_000;

/// 一条已记录的状态反馈。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateFeedback {
    pub mood_state_id: i64,
    /// 被评价的显示状态
    pub state: MoodState,
    pub verdict: Verdict,
}

/// 记录一条状态反馈。`mood_state_id` 为 `None` 时评价的是当前显示状态，记到最近一条状态记录上
/// （小组件不知道记录 id）。还没有任何状态记录、或记录不存在时返回 `None`，不写库。
pub fn record(
    db: &Db,
    mood_state_id: Option<i64>,
    verdict: Verdict,
    ts: i64,
) -> Result<Option<StateFeedback>, StoreError> {
    let id = match mood_state_id {
        Some(id) => id,
        None => match db.latest_mood_state_id()? {
            Some(id) => id,
            None => return Ok(None),
        },
    };
    let Some(state) = db.mood_shown_state(id)? else {
        return Ok(None);
    };
    db.insert_feedback(
        ts,
        FeedbackTarget::MoodState.as_str(),
        Some(id),
        verdict.as_str(),
    )?;
    Ok(Some(StateFeedback {
        mood_state_id: id,
        state,
        verdict,
    }))
}

/// `now_ms` 之前 [`REPLAY_MS`] 内的“不准”，按时间先后：(被评价的状态, 时间)。
pub fn recent_unfit(db: &Db, now_ms: i64) -> Result<Vec<(MoodState, i64)>, StoreError> {
    db.unfit_since(
        FeedbackTarget::MoodState.as_str(),
        Verdict::Unfit.as_str(),
        now_ms - REPLAY_MS,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::features::WindowFeatures;
    use crate::domain::fusion::{Fusion, JevVerdict, Source};
    use crate::domain::rules::{Hint, Hints};
    use crate::infra::templates::AppCat;

    fn mood(db: &Db, ts: i64, shown: MoodState) -> i64 {
        let w = db
            .insert_window(
                ts - 1,
                ts,
                AppCat::Chat,
                &WindowFeatures::default(),
                &Hints::default(),
            )
            .unwrap();
        db.insert_mood_state(ts, Some(w), shown, shown, Source::Rule)
            .unwrap()
    }

    #[test]
    fn current_state_feedback_goes_to_latest_record() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(record(&db, None, Verdict::Unfit, 10).unwrap(), None);
        mood(&db, 100, MoodState::Fluent);
        let last = mood(&db, 200, MoodState::Hesitant);
        let f = record(&db, None, Verdict::Unfit, 300).unwrap().unwrap();
        assert_eq!(
            f,
            StateFeedback {
                mood_state_id: last,
                state: MoodState::Hesitant,
                verdict: Verdict::Unfit
            }
        );
        let row: (i64, String, i64, String) = db
            .conn()
            .query_row(
                "SELECT ts, target, target_id, verdict FROM feedback",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(row, (300, "mood_state".into(), last, "unfit".into()));
        // 指定的记录不存在：不写库
        assert_eq!(record(&db, Some(999), Verdict::Fit, 400).unwrap(), None);
    }

    #[test]
    fn restart_replays_recent_unfit() {
        // TC-STA-08：30 分钟内 3 次“不准”上调阈值，重启后从库里恢复
        let db = Db::open_in_memory().unwrap();
        let day = 24 * 3_600_000;
        let old = mood(&db, 1_000, MoodState::Low);
        let id = mood(&db, 10 * day, MoodState::Hesitant);
        // 8 天前的“不准”不再重放
        record(&db, Some(old), Verdict::Unfit, 1_000).unwrap();
        for t in [0, 60_000, 120_000] {
            record(&db, Some(id), Verdict::Unfit, 10 * day + t).unwrap();
        }
        record(&db, Some(id), Verdict::Fit, 10 * day + 1).unwrap();
        let now = 10 * day + 200_000;
        let unfit = recent_unfit(&db, now).unwrap();
        assert_eq!(
            unfit,
            vec![
                (MoodState::Hesitant, 10 * day),
                (MoodState::Hesitant, 10 * day + 60_000),
                (MoodState::Hesitant, 10 * day + 120_000),
            ]
        );

        let mut f = Fusion::new();
        for (s, ts) in unfit {
            f.record_unfit(s, ts);
        }
        // 阈值变为 0.95，0.92 不再立即切换
        let v = JevVerdict {
            probs: [(MoodState::Hesitant, 0.92)].into_iter().collect(),
            choice: MoodState::Hesitant,
            confidence: 0.92,
            valence: 2.0,
            need_comfort: 0.3,
        };
        let o = f.on_window(&Hints(vec![Hint::HesitationHint]), Some(&v), now);
        assert!(!o.changed);
    }

    #[test]
    fn serializes_with_table_values() {
        assert_eq!(
            serde_json::to_value(FeedbackTarget::MoodState).unwrap(),
            "mood_state"
        );
        assert_eq!(serde_json::to_value(Verdict::Unfit).unwrap(), "unfit");
    }
}
