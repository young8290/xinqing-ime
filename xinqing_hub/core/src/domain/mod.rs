//! 领域层：只依赖基础层的 trait，禁止 use tauri / windows（NFR-MNT-03）。

pub mod consent;
pub mod features;
pub mod fusion;
pub mod rules;
pub mod safety;
pub mod settings;
pub mod status;
pub mod validate;
