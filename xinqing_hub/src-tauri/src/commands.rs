//! 前端 → 后端命令（10 第 5.1 节）。所有命令返回 `Result<T, UiError>`；
//! 这里只做取参 → 调领域服务 → 推送事件，不写业务规则。

pub mod ai;
pub mod chat;
pub mod comfort;
pub mod diary;
pub mod ime;
pub mod letter;
pub mod schedule;

use std::sync::Arc;

use tauri::{AppHandle, Manager, State};
use tauri_specta::Event;
use xinqing_hub_core::HUB_VERSION;
use xinqing_hub_core::bus::HubEvent;
use xinqing_hub_core::domain::consent::{self, ConsentItem, ConsentState};
use xinqing_hub_core::domain::explain::Explanation;
use xinqing_hub_core::domain::features::persist;
use xinqing_hub_core::domain::feedback::{self, FeedbackTarget, Verdict};
use xinqing_hub_core::domain::rest::{RestAction, RestKind};
use xinqing_hub_core::domain::routine::{self, Routine};
use xinqing_hub_core::domain::self_report::{self, ReportSource, SelfReportItem, SelfWeather};
use xinqing_hub_core::domain::settings::{self, SettingValue};
use xinqing_hub_core::domain::status::StatusSnapshot;
use xinqing_hub_core::infra::store::export;
use xinqing_hub_core::sense::SenseCmd;

use crate::error::UiError;
use crate::events::{SelfReportChanged, SettingsChanged, StatusChanged};
use crate::gateway::Ai;
use crate::research::Research;
use crate::rest::Rest;
use crate::sensing::Sensing;
use crate::sim::{self, DemoStatus};
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

/// 导出用户数据（FR-DAT-03），写到前端文件选择器给的 `path`（已存在则覆盖）。
/// 1 年数据量要几秒，放进 `spawn_blocking`，不占主线程。先提交排队中的数据（ADR 0020 第 3 条最多晚 5 秒），
/// 再在只读连接上导出，导出期间不挡写入。
#[tauri::command]
#[specta::specta]
pub async fn data_export(app: AppHandle, path: String) -> Result<(), UiError> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        state.writer().flush();
        export::export(
            &state.db(),
            std::path::Path::new(&path),
            HUB_VERSION,
            chrono::Local::now(),
        )
    })
    .await
    .map_err(|e| UiError::internal("data.export.join", e))?
    .map(|_| ())
    .map_err(|e| UiError::internal("data.export", e))
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
    let Some(baseline) = sensing.baseline() else {
        return Ok(None);
    };
    Ok(state.db().explain_mood_state(i64::from(id), &baseline)?)
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
            let ts = sim::clock().now_ms();
            // 写连接：先提交排队中的状态记录，“记到最近一条”才是界面上显示的那条
            let rec = state
                .writer()
                .write_sync(|db| feedback::record(db, target_id.map(i64::from), verdict, ts))?;
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

/// 主动报告心情（FR-STA-10）。写入 `self_report` 表后交给感知任务：之后 60 分钟显示用户说的状态
/// （“说不上来”不覆盖），并推送 `self_report:changed`。备注只存本地，用完即清零（NFR-PRI-09）。
/// 研究模式的邀请（`research:invite`）弹出后 30 分钟内的自评记为 `source = esm`（FR-DMO-04）。
#[tauri::command]
#[specta::specta]
pub fn self_report_set(
    app: AppHandle,
    state: State<'_, AppState>,
    sensing: State<'_, Sensing>,
    research: State<'_, Research>,
    weather: SelfWeather,
    note: Option<String>,
) -> Result<(), UiError> {
    let note = note.map(zeroize::Zeroizing::new);
    let ts = sim::clock().now_ms();
    let source = if research.take_answer(ts) {
        ReportSource::Esm
    } else {
        ReportSource::User
    };
    let rec = state.writer().write_sync(|db| {
        self_report::record_from(
            db,
            weather,
            note.as_deref().map(String::as_str),
            state.auto_state(),
            ts,
            source,
        )
    })?;
    if sensing
        .cmds
        .try_send(SenseCmd::SelfReport {
            weather,
            ts,
            raise: rec.raise,
        })
        .is_err()
    {
        eprintln!("自评未能交给感知任务，本次不覆盖显示");
    }
    let ev = SelfReportChanged {
        weather,
        until_ts: rec.until_ms as f64,
    };
    if let Err(e) = ev.emit(&app) {
        eprintln!("推送 self_report:changed 失败：{e}");
    }
    Ok(())
}

/// 研究模式的自评邀请点了“跳过”（FR-DMO-04）：之后的自评按用户主动报告记。
#[tauri::command]
#[specta::specta]
pub fn research_dismiss(research: State<'_, Research>) -> Result<(), UiError> {
    research.dismiss();
    Ok(())
}

/// 删除研究期间邀请得到的自评（`source = esm`，FR-DMO-04“研究结束后删除”），返回删了几条。
/// 用户主动的自评不动。界面应先请用户确认。
#[tauri::command]
#[specta::specta]
pub fn research_clear(state: State<'_, AppState>) -> Result<u32, UiError> {
    let n = state.writer().write_sync(self_report::delete_esm)?;
    Ok(u32::try_from(n).unwrap_or(u32::MAX))
}

/// 某天（本地日期 `YYYY-MM-DD`）的自评，按时间先后；看板时间线用实心标记显示。
#[tauri::command]
#[specta::specta]
pub fn self_report_list(
    state: State<'_, AppState>,
    date: String,
) -> Result<Vec<SelfReportItem>, UiError> {
    let date = chrono::NaiveDate::parse_from_str(&date, "%Y-%m-%d")
        .map_err(|_| UiError::new("self_report.bad_date", "error.generic"))?;
    Ok(self_report::list_day(&state.db(), date)?)
}

/// 作息洞察（FR-REV-03，看板周报）：截至最近一个已经结束的晚上共 `days` 晚（1–90）的停止打字时间。
/// 界面一律称“停止打字时间”，并注明只统计这台电脑上的打字、不等于入睡时间（DS-COPY-09）。
#[tauri::command]
#[specta::specta]
pub fn get_routine(state: State<'_, AppState>, days: u32) -> Result<Routine, UiError> {
    Ok(routine::get(&state.db(), days, sim::clock().now())?)
}

/// 是否为演示模式（FR-DMO-03）：是的话界面在标题栏显示“演示模式”。以 `--demo` 启动时为是（ADR 0023）。
#[tauri::command]
#[specta::specta]
pub fn demo_status() -> Result<DemoStatus, UiError> {
    Ok(sim::demo_status())
}

/// 重置基线（设置页“感知”分类，FR-SET-04、FR-STA-03 第 4 条）：清空个人统计值，
/// 之后只用重置以后的窗口，重新进入冷启动（“正在熟悉你的打字习惯”从 0% 开始）。
#[tauri::command]
#[specta::specta]
pub fn baseline_reset(
    state: State<'_, AppState>,
    sensing: State<'_, Sensing>,
) -> Result<(), UiError> {
    if sensing.fixed_baseline {
        // `XQ_SIM_BASELINE` 回放期间不读写真实基线（ADR 0021）
        return Err(UiError::new("baseline.fixed", "error.generic"));
    }
    let now = sim::clock().now_ms();
    let stats = state.writer().write_sync(|db| persist::reset(db, now))?;
    sensing.apply_baseline(&stats);
    if sensing.cmds.try_send(SenseCmd::Baseline(stats)).is_err() {
        eprintln!("重置基线未能交给感知任务，下次启动时生效");
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

/// 全部设置键的类型、取值范围与默认值（ADR 0019 第 9 条），设置页按它出控件，与 `ime_schema` 对称。
#[tauri::command]
#[specta::specta]
pub fn settings_schema() -> Vec<settings::SettingField> {
    settings::schema()
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
    let changed = state
        .writer()
        .write_sync(|db| settings::set(db, &key, &value))?;
    if changed
        && key.starts_with("ai.cap.")
        && let Some(ai) = app.try_state::<Arc<Ai>>()
    {
        ai.apply_caps(&state.db());
    }
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
    let now = state.writer().write_sync(|db| {
        consent::set(db, item, granted, chrono::Utc::now().timestamp_millis())?;
        ConsentState::load(db)
    })?;
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

/// 用户点了休息提醒卡片上的按钮（FR-RST-06 第 2 条）：完成或关闭后该类计时清零，“5 分钟后”顺延；
/// 每次操作记一条 `reminder_log`（FR-RST-08）。服务没在运行时忽略。
#[tauri::command]
#[specta::specta]
pub fn rest_action(
    rest: State<'_, Rest>,
    kind: RestKind,
    action: RestAction,
) -> Result<(), UiError> {
    if rest
        .cmds
        .try_send(xinqing_hub_core::rest::RestCmd::Act { kind, action })
        .is_err()
    {
        eprintln!("休息提醒服务没有在运行，操作未处理");
    }
    Ok(())
}
