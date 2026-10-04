//! 显示状态快照（10 第 5.1 节 `get_status`、第 5.2 节 `status:changed`）。
//!
//! 前端不自行推断状态（17 第 3.2 节），只显示这里给出的快照；快照的唯一写入方是 Hub 后端。

use serde::{Deserialize, Serialize};
use xqp::MoodState;

/// 情绪天气（04 第 3.1 节、07 DS-COLOR）。`Wind` 只用于打错字的瞬时动画，不作为持续状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum Weather {
    Sunny,
    Wind,
    Cloudy,
    Rain,
    Storm,
    Night,
}

impl Weather {
    /// 状态 → 天气；`Unknown` 没有对应天气，调用方保持上一状态（04 第 3.1 节）。
    pub fn for_state(s: MoodState) -> Option<Self> {
        match s {
            MoodState::Fluent => Some(Weather::Sunny),
            MoodState::Hesitant => Some(Weather::Cloudy),
            MoodState::Low => Some(Weather::Rain),
            MoodState::Agitated => Some(Weather::Storm),
            MoodState::Tired => Some(Weather::Night),
            MoodState::Unknown => None,
        }
    }
}

/// `get_status` 的返回值，也是 `status:changed` 事件的载荷。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct StatusSnapshot {
    pub state: MoodState,
    pub weather: Weather,
    /// 当前状态的可能性 0–1；冷启动或只有本地规则时为 `None`，界面不显示百分比。
    pub prob: Option<f64>,
    /// AI 网关降级为离线模式（FR-AIG-05），小组件显示“离线”角标。
    pub offline: bool,
    /// 无痕 / 密码框 / 黑名单应用，或用户暂停感知。
    pub paused: bool,
    /// 与输入法核心的 XQP 连接是否已握手。
    pub connected: bool,
    /// 基线建立进度 0–100（FR-STA-03 冷启动）。
    pub baseline_progress: u8,
}

impl Default for StatusSnapshot {
    fn default() -> Self {
        Self {
            state: MoodState::Fluent,
            weather: Weather::Sunny,
            prob: None,
            offline: true,
            paused: false,
            connected: false,
            baseline_progress: 0,
        }
    }
}

impl StatusSnapshot {
    /// 融合结果更新显示状态。返回快照是否有变化（决定是否推送 `status:changed`）。
    pub fn apply_mood(&mut self, state: MoodState, prob: Option<f64>) -> bool {
        let Some(weather) = Weather::for_state(state) else {
            return false;
        };
        let prob = prob.map(|p| p.clamp(0.0, 1.0));
        let changed = self.state != state || self.weather != weather || self.prob != prob;
        self.state = state;
        self.weather = weather;
        self.prob = prob;
        changed
    }

    /// 修改一个布尔或进度字段，返回是否有变化。
    pub fn set<T: PartialEq>(field: &mut T, v: T) -> bool {
        if *field == v {
            false
        } else {
            *field = v;
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_display_state_has_weather_except_unknown() {
        assert_eq!(Weather::for_state(MoodState::Fluent), Some(Weather::Sunny));
        assert_eq!(
            Weather::for_state(MoodState::Hesitant),
            Some(Weather::Cloudy)
        );
        assert_eq!(Weather::for_state(MoodState::Low), Some(Weather::Rain));
        assert_eq!(
            Weather::for_state(MoodState::Agitated),
            Some(Weather::Storm)
        );
        assert_eq!(Weather::for_state(MoodState::Tired), Some(Weather::Night));
        assert_eq!(Weather::for_state(MoodState::Unknown), None);
    }

    #[test]
    fn unknown_keeps_previous_state() {
        let mut s = StatusSnapshot::default();
        assert!(s.apply_mood(MoodState::Hesitant, Some(0.8)));
        assert!(!s.apply_mood(MoodState::Unknown, Some(0.9)));
        assert_eq!(s.state, MoodState::Hesitant);
        assert_eq!(s.weather, Weather::Cloudy);
        assert_eq!(s.prob, Some(0.8));
    }

    #[test]
    fn same_state_same_prob_is_not_a_change() {
        let mut s = StatusSnapshot::default();
        assert!(!s.apply_mood(MoodState::Fluent, None));
        assert!(s.apply_mood(MoodState::Fluent, Some(1.4)));
        assert_eq!(s.prob, Some(1.0));
        assert!(StatusSnapshot::set(&mut s.paused, true));
        assert!(!StatusSnapshot::set(&mut s.paused, true));
    }

    #[test]
    fn serializes_with_snake_case_names() {
        let v = serde_json::to_value(StatusSnapshot::default()).unwrap();
        assert_eq!(v["state"], "fluent");
        assert_eq!(v["weather"], "sunny");
        assert_eq!(v["baseline_progress"], 0);
    }
}
