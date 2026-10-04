//! 前端 → 后端命令（10 第 5.1 节）。所有命令返回 `Result<T, UiError>`；
//! 这里只做取参 → 调领域服务 → 推送事件，不写业务规则。

use tauri::{AppHandle, State};
use tauri_specta::Event;
use xinqing_hub_core::domain::consent::{self, ConsentItem, ConsentState};
use xinqing_hub_core::domain::settings::{self, SettingValue};
use xinqing_hub_core::domain::status::StatusSnapshot;

use crate::error::UiError;
use crate::events::{SettingsChanged, StatusChanged};
use crate::state::AppState;
use crate::windows::{self, WindowTarget};

fn emit_status(app: &AppHandle, snap: StatusSnapshot) {
    if let Err(e) = StatusChanged(snap).emit(app) {
        eprintln!("推送 status:changed 失败：{e}");
    }
}

#[tauri::command]
#[specta::specta]
pub fn get_status(state: State<'_, AppState>) -> Result<StatusSnapshot, UiError> {
    Ok(state.status())
}

/// 暂停 / 恢复感知（FR-WGT-06 右键菜单）。连上输入法后还要经 XQP 下发，见 C-02。
#[tauri::command]
#[specta::specta]
pub fn pause_set(app: AppHandle, state: State<'_, AppState>, on: bool) -> Result<(), UiError> {
    let (snap, changed) = state.update_status(|s| StatusSnapshot::set(&mut s.paused, on));
    if changed {
        emit_status(&app, snap);
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn settings_get(state: State<'_, AppState>, key: String) -> Result<SettingValue, UiError> {
    Ok(settings::get(&state.db(), &key)?)
}

#[tauri::command]
#[specta::specta]
pub fn settings_set(
    app: AppHandle,
    state: State<'_, AppState>,
    key: String,
    value: SettingValue,
) -> Result<(), UiError> {
    let changed = settings::set(&state.db(), &key, &value)?;
    if changed {
        if let Err(e) = (SettingsChanged { key }).emit(&app) {
            eprintln!("推送 settings:changed 失败：{e}");
        }
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn consent_get(state: State<'_, AppState>) -> Result<ConsentState, UiError> {
    Ok(ConsentState::load(&state.db())?)
}

/// 记录一项同意或撤回，返回更新后的全部同意状态，界面不用再读一次。
#[tauri::command]
#[specta::specta]
pub fn consent_set(
    state: State<'_, AppState>,
    item: ConsentItem,
    granted: bool,
) -> Result<ConsentState, UiError> {
    let db = state.db();
    consent::set(&db, item, granted, chrono::Utc::now().timestamp_millis())?;
    Ok(ConsentState::load(&db)?)
}

/// 必须是 async：同步命令跑在主线程，在 Windows 上同步命令里建窗口会死锁（wry#583）。
#[tauri::command]
#[specta::specta]
pub async fn open_window(app: AppHandle, target: WindowTarget) -> Result<(), UiError> {
    windows::open(&app, target)
}
