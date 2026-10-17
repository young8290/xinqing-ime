//! moodflow-typing-practice：打字测速（纯逻辑）。
//!
//! 口径取自 XSIME（见 `docs/design/xsime-feature-port-plan.md` §3.3）：用户显式开始的练习，
//! 计分为有效速度（按最终字准折算的每分钟字数）、过程正确率（每次新增的字是否与范文同位一致）、
//! 最终字准（与范文的有界编辑距离）。
//!
//! # 边界
//!
//! - **日常统计不在这里**：它早已在 `moodflow-store::stats` / `stat_collector`（字符分类、时段、
//!   码长、选重、活跃时长、速度模型 v2）。练习期间调用方用 `StatCollector::pause` 暂停日常统计，
//!   练习结束 `resume`——练习的输入不计入日常统计（XSIME 口径）。
//! - 范文与练习文本只在内存里，本 crate 不落盘、不打日志。
//! - 时间是毫秒时间戳，由调用方给。

mod practice;

pub use practice::{Practice, Score, bounded_edit_distance};

#[cfg(test)]
mod tests;
