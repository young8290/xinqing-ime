//! 日程与待办命令（10 第 5.1 节“日程”“待办”“提醒”，C-05、C-06）。参数与返回的形状见 ADR 0032 第 8 条。
//! 用户确认类数据，一律 `write_sync`（ADR 0020）。识别出来的卡片经 `schedule:ready` / `todo:ready` 推送，
//! 卡片上的“添加 / 修改 / 忽略”调这里。

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;
use xinqing_hub_core::domain::agenda::{self, Conflict, Slot};
use xinqing_hub_core::domain::reminder::{self, ACTION_OK, ACTION_SNOOZE_5, ACTION_SNOOZE_10};
use xinqing_hub_core::domain::schedule::{ScheduleDraft, TodoDraft};
use xinqing_hub_core::domain::settings;
use xinqing_hub_core::infra::store::{Db, ScheduleRow, StoreError, TodoRow};
use xinqing_hub_core::reminder::ReminderCmd;

use crate::error::UiError;
use crate::events::ReminderMissed;
use crate::schedule::{MissedStash, Reminders, ScheduleCopy};
use crate::state::AppState;

/// 一条日程（09 D-12）。时间戳是 Unix 毫秒（前端绑定不导出 i64）。
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ScheduleItem {
    pub id: u32,
    /// 已忽略的只留哈希，标题等为 `null`
    pub title: Option<String>,
    /// `YYYY-MM-DD`
    pub date: Option<String>,
    /// `HH:MM`；全天日程为 `null`
    pub time: Option<String>,
    pub end_time: Option<String>,
    pub all_day: bool,
    pub location: Option<String>,
    pub is_deadline: bool,
    /// 提前几秒提醒（FR-SCH-07）；全天日程固定当天 09:00，为空数组
    pub remind_offsets: Vec<u32>,
    /// `pending` 待确认 / `added` 已添加 / `ignored` 已忽略
    pub status: String,
    /// `ai`（卡片显示 `AI 识别`）/ `manual`
    pub source: String,
    /// `adjusted` 日期以代码为准、`maybe_past` 时间可能已过、`confirm_date` 请确认日期、`need_time` 请补充时刻、
    /// `local` 本地规则抽取（请确认信息）、`maybe_dup` 可能已经添加过
    pub flags: Vec<String>,
    #[specta(type = specta_typescript::Number)]
    pub created_ts: f64,
}

impl From<ScheduleRow> for ScheduleItem {
    fn from(r: ScheduleRow) -> Self {
        Self {
            id: r.id as u32,
            title: r.title,
            date: r.date,
            time: r.time,
            end_time: r.end_time,
            all_day: r.all_day,
            location: r.location,
            is_deadline: r.is_deadline,
            remind_offsets: r.remind_offsets.into_iter().map(|o| o as u32).collect(),
            status: r.status,
            source: r.source,
            flags: r.flags,
            created_ts: r.created_ts as f64,
        }
    }
}

/// 一件待办（09 D-25）。
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct TodoItem {
    pub id: u32,
    pub title: Option<String>,
    pub due_date: Option<String>,
    /// `pending` 待确认 / `open` 未完成 / `done` 已完成 / `archived` 已归档 / `ignored` 已忽略
    pub status: String,
    pub source: String,
    #[specta(type = specta_typescript::Number)]
    pub created_ts: f64,
    #[specta(type = Option<specta_typescript::Number>)]
    pub done_ts: Option<f64>,
}

impl From<TodoRow> for TodoItem {
    fn from(r: TodoRow) -> Self {
        Self {
            id: r.id as u32,
            title: r.title,
            due_date: r.due_date,
            status: r.status,
            source: r.source,
            created_ts: r.created_ts as f64,
            done_ts: r.done_ts.map(|t| t as f64),
        }
    }
}

/// 与之时间重叠的已添加日程（FR-SCH-11）：卡片上“⚠ 与「班会」时间重叠（15:00–16:00）”。
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ConflictItem {
    pub id: u32,
    pub title: String,
    /// 全天日程为 `null`
    pub start: Option<String>,
    pub end: Option<String>,
}

impl From<Conflict> for ConflictItem {
    fn from(c: Conflict) -> Self {
        Self {
            id: c.id as u32,
            title: c.title,
            start: c.start,
            end: c.end,
        }
    }
}

/// 新建或修改日程时填的字段（卡片上的“修改”、日程页的新建与编辑）。
#[derive(Debug, Clone, Deserialize, Type)]
pub struct ScheduleInput {
    /// ≤ 12 字
    pub title: String,
    pub date: Option<String>,
    pub time: Option<String>,
    pub end_time: Option<String>,
    pub all_day: bool,
    pub location: Option<String>,
    pub is_deadline: bool,
    /// 提前几秒提醒；为空时按类型给默认值（有时刻的用设置 `sch.default_offsets`，截止类 1 天 + 2 小时，全天当天 09:00）
    pub remind_offsets: Option<Vec<u32>>,
}

/// 新建或修改日程的结果：行号与时间重叠的日程（只提示，不阻止，FR-SCH-11 第 3 条）。
#[derive(Debug, Clone, Serialize, Type)]
pub struct ScheduleSaved {
    pub id: u32,
    pub conflicts: Vec<ConflictItem>,
}

/// 新建或修改待办时填的字段。
#[derive(Debug, Clone, Deserialize, Type)]
pub struct TodoInput {
    /// ≤ 16 字
    pub title: String,
    pub due_date: Option<String>,
}

fn not_found(what: &str) -> UiError {
    UiError::new(&format!("{what}.not_found"), "error.generic")
}

fn store(e: StoreError) -> UiError {
    match e {
        StoreError::InvalidSchedule => UiError::new("schedule.invalid", "error.generic"),
        StoreError::InvalidTodo => UiError::new("todo.invalid", "error.generic"),
        e => e.into(),
    }
}

fn ok_or(found: bool, what: &str) -> Result<(), UiError> {
    if found { Ok(()) } else { Err(not_found(what)) }
}

fn trimmed(s: Option<String>) -> Option<String> {
    s.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty())
}

fn draft_of(db: &Db, input: ScheduleInput, source: &str) -> ScheduleDraft {
    let time = if input.all_day {
        None
    } else {
        trimmed(input.time)
    };
    let all_day = time.is_none();
    let remind_offsets = match input.remind_offsets {
        Some(o) => o.into_iter().map(i64::from).collect(),
        None => {
            let timed = settings::get(db, "sch.default_offsets")
                .ok()
                .and_then(|v| v.as_str().map(reminder::offsets_of_setting))
                .unwrap_or_else(|| reminder::DEFAULT_OFFSETS.to_vec());
            reminder::default_offsets(input.is_deadline, all_day, &timed)
        }
    };
    ScheduleDraft {
        title: input.title.trim().to_owned(),
        date: trimmed(input.date),
        end_time: if all_day {
            None
        } else {
            trimmed(input.end_time)
        },
        time,
        all_day,
        location: trimmed(input.location),
        is_deadline: input.is_deadline,
        remind_offsets,
        source: source.into(),
        flags: Vec::new(),
    }
}

fn conflicts_of(
    db: &Db,
    draft: &ScheduleDraft,
    id: Option<i64>,
) -> Result<Vec<ConflictItem>, StoreError> {
    let same_day = match draft.date.as_deref() {
        Some(d) => db.schedules_added_on(d)?,
        None => Vec::new(),
    };
    Ok(agenda::conflicts(&Slot::of_draft(draft), &same_day, id)
        .into_iter()
        .map(Into::into)
        .collect())
}

/// 某个状态的日程（FR-SCH-09）：`pending` / `added` / `ignored`（只有数量有意义）。“已过期”是已添加里日期已过的，界面自己分。
#[tauri::command]
#[specta::specta]
pub fn schedule_list(
    state: State<'_, AppState>,
    status: String,
) -> Result<Vec<ScheduleItem>, UiError> {
    Ok(state
        .db()
        .schedules_by_status(&status)
        .map_err(store)?
        .into_iter()
        .map(Into::into)
        .collect())
}

/// 卡片或待确认列表里的“添加”（FR-SCH-05）：待确认 → 已添加，按 FR-SCH-07 开始提醒。
#[tauri::command]
#[specta::specta]
pub fn schedule_confirm(state: State<'_, AppState>, id: u32) -> Result<(), UiError> {
    let found = state
        .writer()
        .write_sync(|db| db.schedule_confirm(i64::from(id)))
        .map_err(store)?;
    ok_or(found, "schedule")
}

/// “忽略”（FR-SCH-05）：只留去重哈希，标题等立即删除。
#[tauri::command]
#[specta::specta]
pub fn schedule_ignore(state: State<'_, AppState>, id: u32) -> Result<(), UiError> {
    let found = state
        .writer()
        .write_sync(|db| db.schedule_ignore(i64::from(id)))
        .map_err(store)?;
    ok_or(found, "schedule")
}

/// 手动新建（FR-SCH-09）：直接是已添加，不带 `AI 识别`。返回与之时间重叠的日程（FR-SCH-11）。
#[tauri::command]
#[specta::specta]
pub fn schedule_create(
    state: State<'_, AppState>,
    input: ScheduleInput,
) -> Result<ScheduleSaved, UiError> {
    let now = chrono::Local::now().timestamp_millis();
    state
        .writer()
        .write_sync(|db| {
            let draft = draft_of(db, input, "manual");
            let conflicts = conflicts_of(db, &draft, None)?;
            let (id, _) = db.schedule_create(&draft, "added", now)?;
            Ok(ScheduleSaved {
                id: id as u32,
                conflicts,
            })
        })
        .map_err(store)
}

/// 修改（卡片上的“修改”、日程页编辑，FR-SCH-05、FR-SCH-09）。来源不变，标记清空（用户改过就以用户为准）。
#[tauri::command]
#[specta::specta]
pub fn schedule_update(
    state: State<'_, AppState>,
    id: u32,
    input: ScheduleInput,
) -> Result<ScheduleSaved, UiError> {
    let id = i64::from(id);
    state.writer().write_sync(|db| {
        let row = db
            .schedule_get(id)
            .map_err(store)?
            .ok_or_else(|| not_found("schedule"))?;
        let draft = draft_of(db, input, &row.source);
        let conflicts = conflicts_of(db, &draft, Some(id)).map_err(store)?;
        ok_or(db.schedule_update(id, &draft).map_err(store)?, "schedule")?;
        Ok(ScheduleSaved {
            id: id as u32,
            conflicts,
        })
    })
}

/// 删除（FR-SCH-09）。
#[tauri::command]
#[specta::specta]
pub fn schedule_delete(state: State<'_, AppState>, id: u32) -> Result<(), UiError> {
    let found = state
        .writer()
        .write_sync(|db| db.schedule_delete(i64::from(id)))
        .map_err(store)?;
    ok_or(found, "schedule")
}

/// 加入我的日历（FR-SCH-08）：把这些日程写成 RFC 5545 `.ics` 放进临时目录，Windows 上用系统默认程序打开。
/// 返回文件路径。没有日期的日程跳过。
#[tauri::command]
#[specta::specta]
pub fn schedule_export_ics(
    state: State<'_, AppState>,
    copy: State<'_, ScheduleCopy>,
    ids: Vec<u32>,
) -> Result<String, UiError> {
    let rows = {
        let db = state.db();
        let mut rows = Vec::new();
        for id in ids {
            if let Some(r) = db.schedule_get(i64::from(id)).map_err(store)?
                && r.status == "added"
            {
                rows.push(r);
            }
        }
        rows
    };
    let text = agenda::ics(&rows, &copy.ics_description, chrono::Utc::now());
    let path = std::env::temp_dir().join(format!(
        "xinqing-{}.ics",
        chrono::Local::now().format("%Y%m%d-%H%M%S")
    ));
    std::fs::write(&path, text).map_err(|e| UiError::internal("schedule.ics", e))?;
    #[cfg(windows)]
    if let Err(e) = std::process::Command::new("cmd")
        .args(["/C", "start", ""])
        .arg(&path)
        .spawn()
    {
        eprintln!("打开日历文件失败：{e}");
    }
    Ok(path.to_string_lossy().into_owned())
}

/// 提醒卡片上的操作（FR-SCH-07）：`ok` 知道了、`snooze_5` / `snooze_10` 5 / 10 分钟后再提醒。`id` 是 `reminder:due` 带的。
#[tauri::command]
#[specta::specta]
pub fn reminder_action(
    reminders: State<'_, Reminders>,
    id: u32,
    action: String,
) -> Result<(), UiError> {
    if ![ACTION_OK, ACTION_SNOOZE_5, ACTION_SNOOZE_10].contains(&action.as_str()) {
        return Err(UiError::new("reminder.bad_action", "error.generic"));
    }
    if reminders
        .cmds
        .try_send(ReminderCmd::Act { id, action })
        .is_err()
    {
        eprintln!("提醒服务没有在运行，操作未处理");
    }
    Ok(())
}

/// 取走 Hub 启动时错过的提醒（FR-SCH-07“错过的提醒”卡片）：只给一次，取过就清空；没有时为 `null`。
/// 卡片层打开时、收到 `reminder:missed` 时都调它，同一批不会显示两次（ADR 0036）。
#[tauri::command]
#[specta::specta]
pub fn reminder_missed_take(stash: State<'_, MissedStash>) -> Option<ReminderMissed> {
    stash.0.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// 某个状态的待办（FR-SCH-13）：`pending` / `open` / `done` / `archived`。
#[tauri::command]
#[specta::specta]
pub fn todo_list(state: State<'_, AppState>, status: String) -> Result<Vec<TodoItem>, UiError> {
    Ok(state
        .db()
        .todos_by_status(&status)
        .map_err(store)?
        .into_iter()
        .map(Into::into)
        .collect())
}

/// 卡片上的“加入待办”（FR-SCH-13）：待确认 → 未完成。
#[tauri::command]
#[specta::specta]
pub fn todo_confirm(state: State<'_, AppState>, id: u32) -> Result<(), UiError> {
    let found = state
        .writer()
        .write_sync(|db| db.todo_confirm(i64::from(id)))
        .map_err(store)?;
    ok_or(found, "todo")
}

/// “忽略”：只留去重哈希。
#[tauri::command]
#[specta::specta]
pub fn todo_ignore(state: State<'_, AppState>, id: u32) -> Result<(), UiError> {
    let found = state
        .writer()
        .write_sync(|db| db.todo_ignore(i64::from(id)))
        .map_err(store)?;
    ok_or(found, "todo")
}

/// 手动新建（FR-SCH-13）：直接是未完成，不带 `AI 识别`。返回行号。
#[tauri::command]
#[specta::specta]
pub fn todo_create(state: State<'_, AppState>, input: TodoInput) -> Result<u32, UiError> {
    let draft = TodoDraft {
        title: input.title.trim().to_owned(),
        due_date: trimmed(input.due_date),
        source: "manual".into(),
    };
    let now = chrono::Local::now().timestamp_millis();
    let (id, _) = state
        .writer()
        .write_sync(|db| db.todo_create(&draft, "open", now))
        .map_err(store)?;
    Ok(id as u32)
}

/// 修改待确认或未完成的待办。
#[tauri::command]
#[specta::specta]
pub fn todo_update(state: State<'_, AppState>, id: u32, input: TodoInput) -> Result<(), UiError> {
    let id = i64::from(id);
    state.writer().write_sync(|db| {
        let row = db
            .todo_get(id)
            .map_err(store)?
            .ok_or_else(|| not_found("todo"))?;
        let draft = TodoDraft {
            title: input.title.trim().to_owned(),
            due_date: trimmed(input.due_date),
            source: row.source,
        };
        ok_or(db.todo_update(id, &draft).map_err(store)?, "todo")
    })
}

/// 勾选完成（FR-SCH-13）：7 天后归档、30 天后删除（数据保留期，`retention`）。
#[tauri::command]
#[specta::specta]
pub fn todo_complete(state: State<'_, AppState>, id: u32) -> Result<(), UiError> {
    let now = chrono::Local::now().timestamp_millis();
    let found = state
        .writer()
        .write_sync(|db| db.todo_mark_done(i64::from(id), now))
        .map_err(store)?;
    ok_or(found, "todo")
}

/// 删除。
#[tauri::command]
#[specta::specta]
pub fn todo_delete(state: State<'_, AppState>, id: u32) -> Result<(), UiError> {
    let found = state
        .writer()
        .write_sync(|db| db.todo_delete(i64::from(id)))
        .map_err(store)?;
    ok_or(found, "todo")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(title: &str, time: Option<&str>) -> ScheduleInput {
        ScheduleInput {
            title: title.into(),
            date: Some("2026-10-09".into()),
            time: time.map(Into::into),
            end_time: None,
            all_day: time.is_none(),
            location: Some("  ".into()),
            is_deadline: false,
            remind_offsets: None,
        }
    }

    #[test]
    fn drafts_get_default_offsets_and_conflicts() {
        let db = Db::open_in_memory().unwrap();
        let d = draft_of(&db, input("班会", Some("15:00")), "manual");
        assert_eq!(
            (d.remind_offsets.clone(), d.location.clone()),
            (vec![600], None)
        );
        db.schedule_create(&d, "added", 0).unwrap();
        let all_day = draft_of(&db, input("运动会", None), "manual");
        assert!(all_day.all_day && all_day.remind_offsets.is_empty());
        let mut ddl = input("交报告", Some("23:59"));
        ddl.is_deadline = true;
        assert_eq!(
            draft_of(&db, ddl, "manual").remind_offsets,
            vec![86_400, 7_200]
        );
        let c = conflicts_of(
            &db,
            &draft_of(&db, input("组会", Some("15:30")), "manual"),
            None,
        )
        .unwrap();
        assert_eq!(
            c.iter().map(|c| c.title.as_str()).collect::<Vec<_>>(),
            ["班会"]
        );
        // 设置改成提前 30 分钟
        settings::set(
            &db,
            "sch.default_offsets",
            &settings::SettingValue::from("30"),
        )
        .unwrap();
        assert_eq!(
            draft_of(&db, input("晚课", Some("19:00")), "manual").remind_offsets,
            vec![1_800]
        );
    }
}
