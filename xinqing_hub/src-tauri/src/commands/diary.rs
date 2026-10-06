//! 情绪日记命令（10 第 5.1 节“日记”，C-10，FR-DIA-01～03）。参数与返回的形状见 ADR 0031 第 6 条。

use std::sync::Arc;

use serde::Serialize;
use specta::Type;
use tauri::State;
use xinqing_hub_core::diary::{DiaryError, DiaryService, SaveReq};

use crate::diary::Diary;
use crate::error::UiError;
use crate::state::AppState;

/// `diary_generate` 的结果：草稿或空白模板，放进编辑框让用户改。
#[derive(Debug, Clone, Serialize, Type)]
pub struct DiaryDraft {
    /// 保存时原样带回 `diary_save`；空白模板时为 `null`
    pub draft_id: Option<u32>,
    pub text: String,
    /// 为假时是空白模板（大模型不可用、草稿没通过校验、或没有可写的内容）
    pub ai_generated: bool,
}

/// `diary_save` 的结果。
#[derive(Debug, Clone, Serialize, Type)]
pub struct DiarySaved {
    pub id: u32,
    /// `ai_draft` / `ai_edited` / `manual`（FR-DIA-02）
    pub source: String,
    /// 本地词表命中：对话窗口已打开并显示求助卡片（FR-SAF-02 第 2 条）
    pub safety: bool,
}

/// 一篇日记。时间戳是 Unix 毫秒（前端绑定不导出 i64）。
#[derive(Debug, Clone, Serialize, Type)]
pub struct DiaryItem {
    pub id: u32,
    /// 本地日期 `YYYY-MM-DD`
    pub date: String,
    pub content: String,
    /// `ai_draft` 标“AI 生成”，`ai_edited` 标“AI 辅助生成”，`manual` 不标（FR-DIA-02）
    pub source: String,
    #[specta(type = specta_typescript::Number)]
    pub created_ts: f64,
    #[specta(type = specta_typescript::Number)]
    pub updated_ts: f64,
}

impl From<DiaryError> for UiError {
    fn from(e: DiaryError) -> Self {
        match e {
            DiaryError::Empty => UiError::new("diary.empty", "error.generic"),
            DiaryError::TooLong => UiError::new("diary.too_long", "error.generic"),
            DiaryError::NotFound => UiError::new("diary.not_found", "error.generic"),
            DiaryError::Store(e) => e.into(),
        }
    }
}

fn service(diary: &Diary) -> Result<&Arc<DiaryService>, UiError> {
    diary
        .0
        .as_ref()
        .ok_or_else(|| UiError::internal("diary.unavailable", "日记模板加载失败"))
}

/// 生成今天的日记草稿（FR-DIA-01）。从对话窗口“写成情绪日记”进来时带上那段对话的 `session_id`，
/// 看板“写今天的日记”不带。最长约 20 秒；大模型不可用时立即给空白模板。
#[tauri::command]
#[specta::specta]
pub async fn diary_generate(
    diary: State<'_, Diary>,
    session_id: Option<u32>,
) -> Result<DiaryDraft, UiError> {
    let d = service(&diary)?.generate(session_id.map(i64::from)).await?;
    Ok(DiaryDraft {
        draft_id: d.draft_id,
        text: d.text,
        ai_generated: d.ai_generated,
    })
}

/// 保存日记（FR-DIA-02/03）：`id` 为空时新写一篇（记在今天），否则修改那一篇。从草稿来的带上 `draft_id`，
/// 手写的不带。来源由后端判断。返回时已落盘。
#[tauri::command]
#[specta::specta]
pub async fn diary_save(
    diary: State<'_, Diary>,
    id: Option<u32>,
    draft_id: Option<u32>,
    content: String,
) -> Result<DiarySaved, UiError> {
    let s = service(&diary)?.save(SaveReq {
        id: id.map(i64::from),
        draft_id,
        content,
    })?;
    Ok(DiarySaved {
        id: s.id as u32,
        source: s.source.as_str().into(),
        safety: s.safety,
    })
}

/// 全部日记，新的在前（FR-DIA-03）。
#[tauri::command]
#[specta::specta]
pub fn diary_list(state: State<'_, AppState>) -> Result<Vec<DiaryItem>, UiError> {
    Ok(state
        .db()
        .diaries()?
        .into_iter()
        .map(|d| DiaryItem {
            id: d.id as u32,
            date: d.date,
            content: d.content,
            source: d.source,
            created_ts: d.created_ts as f64,
            updated_ts: d.updated_ts as f64,
        })
        .collect())
}

/// 删除一篇（FR-DIA-03）。这篇已被删除时什么也不做。
#[tauri::command]
#[specta::specta]
pub fn diary_delete(diary: State<'_, Diary>, id: u32) -> Result<(), UiError> {
    service(&diary)?.delete(i64::from(id))?;
    Ok(())
}
