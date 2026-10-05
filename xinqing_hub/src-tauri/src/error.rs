//! 命令错误（10 第 5.1 节）：只含给用户看的文案键和错误码，不含堆栈与技术细节（DS-COPY-06）。
//! 详细原因只写日志。

use serde::Serialize;
use specta::Type;
use xinqing_hub_core::domain::self_report::SelfReportError;
use xinqing_hub_core::domain::settings::SettingsError;
use xinqing_hub_core::infra::imeconf::ImeRpcError;
use xinqing_hub_core::infra::store::StoreError;

#[derive(Debug, Clone, Serialize, Type, thiserror::Error)]
#[error("{code}")]
pub struct UiError {
    /// 稳定的错误码，前端可据此分支；只写日志，不展示给用户
    pub code: String,
    /// `hub_templates/ui_copy.toml` 里的文案键，前端据此显示提示
    pub message_key: String,
}

impl UiError {
    pub fn new(code: &str, message_key: &str) -> Self {
        Self {
            code: code.into(),
            message_key: message_key.into(),
        }
    }

    /// 内部错误：记录原因，给用户一句通用提示。
    pub fn internal(code: &str, cause: impl std::fmt::Display) -> Self {
        eprintln!("[{code}] {cause}");
        Self::new(code, "error.generic")
    }
}

impl From<StoreError> for UiError {
    fn from(e: StoreError) -> Self {
        UiError::internal("store", e)
    }
}

impl From<SettingsError> for UiError {
    fn from(e: SettingsError) -> Self {
        match e {
            SettingsError::UnknownKey(_) => UiError::new("settings.unknown_key", "error.generic"),
            SettingsError::OutOfRange { .. } => {
                UiError::new("settings.out_of_range", "error.generic")
            }
            SettingsError::Store(e) => e.into(),
        }
    }
}

impl From<SelfReportError> for UiError {
    fn from(e: SelfReportError) -> Self {
        match e {
            SelfReportError::NoteTooLong => {
                UiError::new("self_report.note_too_long", "error.generic")
            }
            SelfReportError::Store(e) => e.into(),
        }
    }
}

impl From<tauri::Error> for UiError {
    fn from(e: tauri::Error) -> Self {
        UiError::internal("tauri", e)
    }
}

impl From<ImeRpcError> for UiError {
    fn from(e: ImeRpcError) -> Self {
        match e {
            // 输入法核心没在跑：说清楚，不报技术细节（DS-COPY-06）
            ImeRpcError::Unavailable(cause) => {
                eprintln!("[ime.unavailable] {cause}");
                UiError::new("ime.unavailable", "error.ime_unavailable")
            }
            e => UiError::internal("ime.rpc", e),
        }
    }
}
