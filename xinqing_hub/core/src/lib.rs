//! 心晴 Hub 核心（领域层 + 平台无关的基础层），结构见产品书 17 第 2 节和 docs/adr/0007。
//!
//! Tauri 外壳（`xinqing_hub/src-tauri`）、Windows 专属基础设施（命名管道、DPAPI、Toast）
//! 依赖本 crate，本 crate 不反向依赖它们，因此可以在 Linux CI 上完整测试。

pub mod bus;
pub mod care;
pub mod chat;
pub mod domain;
pub mod infra;
pub mod pipeline;
pub mod research;
pub mod rest;
pub mod sense;

/// Hub 版本（XQP 下行 hello 的 `hub_ver`）。
pub const HUB_VERSION: &str = env!("CARGO_PKG_VERSION");
