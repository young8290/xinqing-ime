//! 前端 → 后端命令（10 第 5.1 节）。所有命令返回 `Result<T, UiError>`；
//! 这里只做取参 → 调领域服务 → 推送事件，不写业务规则。

pub mod ai;

use tauri::{AppHandle, State};
use tauri_specta::Event;
use xinqing_hub_core::bus::HubEvent;
use xinqing_hub_core::domain::consent::{self, ConsentItem, ConsentState};
use xinqing_hub_core::domain::explain::Explanation;
use xinqing_hub_core::domain::feedback::{self, FeedbackTarget, Verdict};
use xinqing_hub_core::domain::settings::{self, SettingValue};
use xinqing_hub_core::domain::status::StatusSnapshot;
use xinqing_hub_core::sense::SenseCmd;

use crate::error::UiError;
use crate::events::{SettingsChanged, StatusChanged};
use crate::sensing::Sensing;
use crate::state::AppState;
use crate::windows::{self, WindowTarget};

pub(crate) fn emit_status(app: &AppHandle, snap: StatusSnapshot) {
    if let Err(e) = StatusChanged(snap).emit(app) {
        eprintln!("推送 status:changed 失败：{e}");
    }
}

#[tauri::command]
#[specta::specta]
pub fn get_status(state: State<'_, AppState>) -> Result<StatusSnapshot, UiError> {
    Ok(state.status())
}

/// 状态解释（FR-STA-09）。不带 `mood_state_id` 时是当前显示状态的解释（还没切换过时为 `null`）；
/// 带上时是看板时间线上那个状态点的解释，记录不存在或窗口已清理时为 `null`。
#[tauri::command]
#[specta::specta]
pub fn state_explain(
    state: State<'_, AppState>,
    sensing: State<'_, Sensing>,
    // specta 不导出 i64（前端 number 会丢精度）；状态记录的行号用 u32 足够
    mood_state_id: Option<u32>,
) -> Result<Option<Explanation>, UiError> {
    let Some(id) = mood_state_id else {
        return Ok(state.explanation());
    };
    let Some(baseline) = &sensing.baseline else {
        return Ok(None);
    };
    Ok(state.db().explain_mood_state(i64::from(id), baseline)?)
}

/// 状态“准 / 不准”（FR-STA-07）。不带 `target_id` 时评价的是当前显示状态。
/// 写入 `feedback` 表后，“不准”交给感知任务上调个人阈值；交不过去时（任务没运行或队列满）
/// 也不报错，下次启动会从库里重放。还没有任何状态记录时什么也不做。
#[tauri::command]
#[specta::specta]
pub fn submit_feedback(
    state: State<'_, AppState>,
    sensing: State<'_, Sensing>,
    target: FeedbackTarget,
    target_id: Option<u32>,
    verdict: Verdict,
) -> Result<(), UiError> {
    match target {
        FeedbackTarget::MoodState => {
            let ts = chrono::Utc::now().timestamp_millis();
            let rec = feedback::record(&state.db(), target_id.map(i64::from), verdict, ts)?;
            if let Some(f) = rec
                && f.verdict == Verdict::Unfit
                && sensing
                    .cmds
                    .try_send(SenseCmd::Unfit { state: f.state, ts })
                    .is_err()
            {
                eprintln!("“不准”反馈未能交给感知任务，下次启动时重放");
            }
        }
    }
    Ok(())
}

/// 暂停 / 恢复感知（FR-WGT-06 右键菜单）：进行中的窗口作废，并经 XQP 下发给输入法（FR-SEN-06）。
#[tauri::command]
#[specta::specta]
pub fn pause_set(app: AppHandle, sensing: State<'_, Sensing>, on: bool) -> Result<(), UiError> {
    sensing.pause(&app, on);
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
    if changed && let Err(e) = (SettingsChanged { key }).emit(&app) {
        eprintln!("推送 settings:changed 失败：{e}");
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
    sensing: State<'_, Sensing>,
    item: ConsentItem,
    granted: bool,
) -> Result<ConsentState, UiError> {
    let db = state.db();
    consent::set(&db, item, granted, chrono::Utc::now().timestamp_millis())?;
    let now = ConsentState::load(&db)?;
    // Hub 是同意状态的唯一真相源：每次变化都重新下发 cfg（10 第 2.5 节）
    sensing.xqp.send(consent::xqp_cfg(&now));
    let _ = sensing.bus.send(HubEvent::ConsentChanged);
    Ok(now)
}

/// 必须是 async：同步命令跑在主线程，在 Windows 上同步命令里建窗口会死锁（wry#583）。
#[tauri::command]
#[specta::specta]
pub async fn open_window(app: AppHandle, target: WindowTarget) -> Result<(), UiError> {
    windows::open(&app, target)
}
