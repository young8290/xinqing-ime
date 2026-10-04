//! 后端 → 前端事件（10 第 5.2 节）。事件名与产品书一致，载荷类型自动导出到 TypeScript。

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri_specta::Event;
use xinqing_hub_core::domain::status::StatusSnapshot;

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
