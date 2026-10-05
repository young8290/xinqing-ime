//! 对话命令（10 第 5.1 节“对话”，C-07）与求助卡片上的“我说的不是这个意思”（FR-SAF-06）。
//! 与产品书的差异（`chat_retry`、`chat_delete_all`、`safety_dismiss`）见 ADR 0018 第 4 条。

use std::sync::Arc;

use serde::Serialize;
use specta::Type;
use tauri::State;
use xinqing_hub_core::chat::{ChatError, ChatService, Sent};
use xinqing_hub_core::domain::chat::{ChatMode, SafeMode};

use crate::chat::Chat;
use crate::error::UiError;
use crate::state::AppState;

/// 左侧抽屉的一行（FR-CHT-02 第 3 条）。时间戳是 Unix 毫秒（前端绑定不导出 i64）。
#[derive(Debug, Clone, Serialize, Type)]
pub struct ChatSessionItem {
    pub id: u32,
    pub title: String,
    pub created_ts: f64,
    pub last_ts: f64,
    /// `on` / `dismissed` 时窗口顶部要有求助信息（展开或折叠成一行）
    pub safe_mode: SafeMode,
    /// 快捷指令切换的对话方式（FR-CHT-06，ADR 0021），窗口据此显示“只倾听中”等提示
    pub mode: ChatMode,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct ChatMessageItem {
    pub id: u32,
    /// `user` / `assistant` / `system_notice`
    pub role: String,
    pub content: String,
    pub ts: f64,
    /// 为真时显示 `AI 生成` 标签（FR-CHT-04 第 4 条）
    pub ai_generated: bool,
}

/// `chat_send` / `chat_retry` 的结果；回复经 `chat:*` 事件推送。
#[derive(Debug, Clone, Serialize, Type)]
pub struct ChatSent {
    pub request_id: u32,
    pub session_id: u32,
    pub user_message_id: Option<u32>,
    /// 开了新会话（没传会话，或上一条消息已超过 6 小时）
    pub new_session: bool,
    /// 本地词表命中：立即显示求助卡片
    pub safety: bool,
}

impl From<Sent> for ChatSent {
    fn from(s: Sent) -> Self {
        Self {
            request_id: s.request_id,
            session_id: s.session_id as u32,
            user_message_id: s.user_message_id.map(|id| id as u32),
            new_session: s.new_session,
            safety: s.safety,
        }
    }
}

impl From<ChatError> for UiError {
    fn from(e: ChatError) -> Self {
        match e {
            ChatError::Empty => UiError::new("chat.empty", "error.generic"),
            ChatError::TooLong => UiError::new("chat.too_long", "error.generic"),
            ChatError::Busy => UiError::new("chat.busy", "error.generic"),
            ChatError::NoSession => UiError::new("chat.no_session", "error.generic"),
            ChatError::NothingToRetry => UiError::new("chat.nothing_to_retry", "error.generic"),
            ChatError::Store(e) => e.into(),
        }
    }
}

fn service(chat: &Chat) -> Result<&Arc<ChatService>, UiError> {
    chat.0
        .as_ref()
        .ok_or_else(|| UiError::internal("chat.unavailable", "对话模板加载失败"))
}

#[tauri::command]
#[specta::specta]
pub fn chat_list_sessions(state: State<'_, AppState>) -> Result<Vec<ChatSessionItem>, UiError> {
    Ok(state
        .db()
        .chat_sessions()?
        .into_iter()
        .map(|s| ChatSessionItem {
            id: s.id as u32,
            title: s.title,
            created_ts: s.created_ts as f64,
            last_ts: s.last_ts as f64,
            safe_mode: s.safe_mode,
            mode: s.mode,
        })
        .collect())
}

/// 一个会话的消息，从旧到新。
#[tauri::command]
#[specta::specta]
pub fn chat_get_messages(
    state: State<'_, AppState>,
    session_id: u32,
) -> Result<Vec<ChatMessageItem>, UiError> {
    Ok(state
        .db()
        .chat_messages(i64::from(session_id))?
        .into_iter()
        .map(|m| ChatMessageItem {
            id: m.id as u32,
            role: m.role,
            content: m.content,
            ts: m.ts as f64,
            ai_generated: m.ai_generated,
        })
        .collect())
}

/// 切换会话的对话方式（FR-CHT-06“我只是想吐槽”“帮我理一理”，回到平常用 `normal`），下一次回复起生效。
/// 还没有会话时不调这个，在 `chat_send` 里带上 `mode`。“写成情绪日记”“陪我呼吸”是窗口的本地动作。
#[tauri::command]
#[specta::specta]
pub fn chat_set_mode(
    chat: State<'_, Chat>,
    session_id: u32,
    mode: ChatMode,
) -> Result<(), UiError> {
    Ok(service(&chat)?.set_mode(i64::from(session_id), mode)?)
}

/// 发一条消息（FR-CHT-02/04/09）。`session_id` 为空或上一条消息已超过 6 小时就开新会话。
/// `mode` 不为空时先把会话切到这种对话方式（快捷指令后发的第一句，ADR 0021）。
/// 异步命令：回复在 tokio 运行时里的后台任务中生成。
#[tauri::command]
#[specta::specta]
pub async fn chat_send(
    chat: State<'_, Chat>,
    session_id: Option<u32>,
    text: String,
    mode: Option<ChatMode>,
) -> Result<ChatSent, UiError> {
    Ok(service(&chat)?
        .send(session_id.map(i64::from), &text, mode)?
        .into())
}

/// “重试”：为会话最后一条用户消息重新生成回复（FR-CHT-04 第 2 条）。
#[tauri::command]
#[specta::specta]
pub async fn chat_retry(chat: State<'_, Chat>, session_id: u32) -> Result<ChatSent, UiError> {
    Ok(service(&chat)?.retry(i64::from(session_id))?.into())
}

/// “停止生成”（FR-CHT-04 第 3 条）。已生成的部分照常保存，经 `chat:done`（`stopped = true`）推送。
#[tauri::command]
#[specta::specta]
pub fn chat_stop(chat: State<'_, Chat>, request_id: u32) -> Result<(), UiError> {
    service(&chat)?.stop(request_id);
    Ok(())
}

/// 复制一条消息的文本：AI 回复末尾附加“（内容由 AI 生成）”（FR-CHT-04 第 5 条）。剪贴板由前端写。
#[tauri::command]
#[specta::specta]
pub fn chat_copy(chat: State<'_, Chat>, message_id: u32) -> Result<String, UiError> {
    service(&chat)?
        .copy_text(i64::from(message_id))?
        .ok_or_else(|| UiError::new("chat.no_message", "error.generic"))
}

/// 删除一个会话（FR-CHT-08）。
#[tauri::command]
#[specta::specta]
pub fn chat_delete(state: State<'_, AppState>, session_id: u32) -> Result<(), UiError> {
    state
        .writer()
        .write_sync(|db| db.chat_delete(Some(i64::from(session_id))))?;
    Ok(())
}

/// 删除全部对话（FR-CHT-08）。
#[tauri::command]
#[specta::specta]
pub fn chat_delete_all(state: State<'_, AppState>) -> Result<(), UiError> {
    state.writer().write_sync(|db| db.chat_delete(None))?;
    Ok(())
}

/// 求助卡片上的“我说的不是这个意思”（FR-SAF-06）：本会话回到普通模式、词表阈值提高，求助信息折叠保留。
#[tauri::command]
#[specta::specta]
pub fn safety_dismiss(chat: State<'_, Chat>, session_id: u32) -> Result<(), UiError> {
    service(&chat)?.dismiss_safety(i64::from(session_id))?;
    Ok(())
}
