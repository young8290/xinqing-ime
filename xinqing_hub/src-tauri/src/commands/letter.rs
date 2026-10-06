//! 周信命令（C-10，FR-REV-02，ADR 0030 第 7 条）：列出、标记已读、删除。

use serde::Serialize;
use specta::Type;
use tauri::State;

use crate::error::UiError;
use crate::state::AppState;

/// 一封周信。时间戳是 Unix 毫秒（前端绑定不导出 i64）。
#[derive(Debug, Clone, Serialize, Type)]
pub struct LetterItem {
    pub id: u32,
    /// 那周的周一，`YYYY-MM-DD`
    pub week_start: String,
    pub content: String,
    /// 为真时信末显示 `AI 生成` 标签；本地模板写的不加
    pub ai_generated: bool,
    pub read: bool,
    #[specta(type = specta_typescript::Number)]
    pub created_ts: f64,
}

/// 全部周信，新的在前（保留 1 年）。
#[tauri::command]
#[specta::specta]
pub fn letters_list(state: State<'_, AppState>) -> Result<Vec<LetterItem>, UiError> {
    Ok(state
        .db()
        .letters()?
        .into_iter()
        .map(|l| LetterItem {
            id: l.id as u32,
            week_start: l.week_start,
            content: l.content,
            ai_generated: l.source == "llm",
            read: l.read,
            created_ts: l.created_ts as f64,
        })
        .collect())
}

/// 标记已读。这封已被删除或清理时什么也不做。
#[tauri::command]
#[specta::specta]
pub fn letter_read(state: State<'_, AppState>, id: u32) -> Result<(), UiError> {
    state
        .writer()
        .write_sync(|db| db.letter_mark_read(i64::from(id)))?;
    Ok(())
}

/// 删除一封（删掉的不会重写）。这封已被删除或清理时什么也不做。
#[tauri::command]
#[specta::specta]
pub fn letter_delete(state: State<'_, AppState>, id: u32) -> Result<(), UiError> {
    state
        .writer()
        .write_sync(|db| db.letter_delete(i64::from(id)))?;
    Ok(())
}
