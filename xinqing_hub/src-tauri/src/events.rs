//! 后端 → 前端事件（10 第 5.2 节）。事件名与产品书一致，载荷类型自动导出到 TypeScript。

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri_specta::Event;
use xinqing_hub_core::care::ComfortSource;
use xinqing_hub_core::domain::self_report::SelfWeather;
use xinqing_hub_core::domain::status::StatusSnapshot;
use xinqing_hub_core::infra::gateway::GatewayHealth;

/// `status:changed`：载荷同 `get_status`。
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[tauri_specta(event_name = "status:changed")]
pub struct StatusChanged(pub StatusSnapshot);

/// `settings:changed`：只带键名，窗口自行重新读取。
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[tauri_specta(event_name = "settings:changed")]
pub struct SettingsChanged {
    pub key: String,
}

/// `self_report:changed`：用户刚自评（FR-STA-10）。到 `until_ts` 之前小组件显示“你说的：…”；
/// “说不上来”时 `until_ts` 就是自评时刻，即不覆盖显示。
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[tauri_specta(event_name = "self_report:changed")]
pub struct SelfReportChanged {
    pub weather: SelfWeather,
    /// Unix 毫秒（前端绑定不导出 i64，毫秒时间戳在 f64 中是精确的）
    pub until_ts: f64,
}

/// `gateway:health`：Jev 与大模型两侧是否可用（`{jev, llm}`），任一侧不可用时小组件显示“离线”角标。
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[tauri_specta(event_name = "gateway:health")]
pub struct GatewayHealthChanged(pub GatewayHealth);

/// `comfort:new`：晴晴说了一句暖心话（FR-CMF-04）。小组件一句话区显示；`ai_generated` 时句尾加 `AI 生成` 标签，
/// 模板句不加。
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[tauri_specta(event_name = "comfort:new")]
pub struct ComfortNew {
    /// `comfort_log` 的行号，反馈（FR-CMF-05）时用
    pub id: u32,
    pub text: String,
    pub source: ComfortSource,
    pub ai_generated: bool,
}
