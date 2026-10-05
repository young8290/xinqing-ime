//! 领域层：只依赖基础层的 trait，禁止 use tauri / windows（NFR-MNT-03）。

pub mod comfort;
pub mod comfort_feedback;
pub mod consent;
pub mod explain;
pub mod features;
pub mod feedback;
pub mod fusion;
pub mod rest;
pub mod retention;
pub mod rules;
pub mod safety;
pub mod schedule;
pub mod self_report;
pub mod settings;
pub mod status;
pub mod validate;
