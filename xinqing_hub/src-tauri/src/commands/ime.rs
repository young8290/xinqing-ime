//! 输入法设置命令（10 第 5.1 节“输入法设置”组，07 FR-SET-02，ADR 0016）：透传 wind-rpc 的
//! `config.schema` / `config.get` / `config.setItems`。管道读写是阻塞的，放进 `spawn_blocking`，不占主线程。
//!
//! 配置值是任意 JSON。specta 把 `serde_json::Value` 当递归类型内联导出时会栈溢出，所以值字段在
//! TypeScript 里标成 `unknown`（`#[specta(type = Unknown)]`），由前端按 `ime_schema` 的类型自己判断。

use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;
use specta_typescript::Unknown;
use xinqing_hub_core::infra::imeconf::{ImeField, ImeItem, ImeRpc, ImeRpcError, ImeSetResult};

use crate::error::UiError;

/// `ime_config_set` 的一项：点分键名与新值。
#[derive(Debug, Clone, Deserialize, Type)]
pub struct ImeItemInput {
    pub key: String,
    #[specta(type = Unknown)]
    pub value: Value,
}

/// `ime_config_get` 的返回：整份合并后的配置，界面按键名的点分路径取值。
#[derive(Debug, Clone, Serialize, Type)]
pub struct ImeConfig {
    #[specta(type = Unknown)]
    pub values: Value,
}

fn rpc() -> ImeRpc {
    // 开发版 Hub 连开发版核心（管道名带 `_dev`），与 XQP 管道的规则一致
    ImeRpc::from_env(cfg!(debug_assertions))
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce(ImeRpc) -> Result<T, ImeRpcError> + Send + 'static,
) -> Result<T, UiError> {
    tauri::async_runtime::spawn_blocking(move || f(rpc()))
        .await
        .map_err(|e| UiError::internal("ime.join", e))?
        .map_err(UiError::from)
}

/// 全部已登记的输入法配置键与类型，设置页据此生成表单。
#[tauri::command]
#[specta::specta]
pub async fn ime_schema() -> Result<Vec<ImeField>, UiError> {
    blocking(|r| r.schema()).await
}

/// 整份合并后的输入法配置。
#[tauri::command]
#[specta::specta]
pub async fn ime_config_get() -> Result<ImeConfig, UiError> {
    blocking(|r| r.config().map(|values| ImeConfig { values })).await
}

/// 写入若干项：失败项不让整批失败，`skipped` 里带原因，界面显示在对应控件旁（17 第 3.4 节）。
#[tauri::command]
#[specta::specta]
pub async fn ime_config_set(items: Vec<ImeItemInput>) -> Result<ImeSetResult, UiError> {
    let items: Vec<ImeItem> = items
        .into_iter()
        .map(|i| ImeItem {
            key: i.key,
            value: i.value,
        })
        .collect();
    blocking(move |r| r.set_items(&items)).await
}
