//! 暖心话的反馈命令（C-04 第二部分：FR-CMF-05、FR-CMF-06 第 2 条，ADR 0015 第 9–12 条）。

use tauri::{AppHandle, State};
use tauri_specta::Event;
use xinqing_hub_core::domain::comfort_feedback::{self, ComfortVerdict};

use crate::error::UiError;
use crate::events::{CareReduced, SettingsChanged};
use crate::state::AppState;

/// 一句暖心话上的 👍 有用 / 👎 不合适 / 🔕 今天先别说了。`id` 是 `comfort:new` 带的行号。
/// 👎 的模板句以后不再出现；🔕 让今天剩余时间不再主动关怀（休息提醒不受影响）；
/// 连续 3 天都是 👎 / 🔕 时自动降一档，推送 `care:reduced` 和 `settings:changed`。
/// 这句话已被清理时什么也不做。
#[tauri::command]
#[specta::specta]
pub fn comfort_feedback(
    app: AppHandle,
    state: State<'_, AppState>,
    id: u32,
    verdict: ComfortVerdict,
) -> Result<(), UiError> {
    let out = state.writer().write_sync(|db| {
        comfort_feedback::record(db, i64::from(id), verdict, chrono::Local::now())
    })?;
    if let Some(level) = out.reduced_to {
        let changed = SettingsChanged {
            key: "care.level".into(),
        };
        if let Err(e) = changed.emit(&app) {
            eprintln!("推送 settings:changed 失败：{e}");
        }
        let reduced = CareReduced {
            level: level.as_str().into(),
        };
        if let Err(e) = reduced.emit(&app) {
            eprintln!("推送 care:reduced 失败：{e}");
        }
    }
    Ok(())
}
