//! Hub 内部事件总线（03 第 2.2 节、10 第 5.3 节）。领域服务之间只通过总线通信。
//!
//! 目前只定义已实现模块用到的事件；其余事件随对应模块加入，名称与 10 第 5.3 节保持一致。

use std::sync::Arc;

use tokio::sync::broadcast;
use xqp::{MoodState, Up};

use crate::domain::features::WindowFeatures;
use crate::domain::fusion::FusionOut;
use crate::domain::rules::Hints;

pub const BUS_CAPACITY: usize = 1024;

#[derive(Debug, Clone)]
pub enum MoodEvent {
    StateChanged { ts: i64, state: MoodState },
    Sample { ts: i64, out: FusionOut },
}

#[derive(Debug, Clone)]
pub enum HubEvent {
    /// 原始上行消息
    Xqp(Arc<Up>),
    /// 特征窗口
    Window(Arc<(WindowFeatures, Hints)>),
    Typo {
        ts: i64,
    },
    Mood(MoodEvent),
    Safety {
        session_id: i64,
    },
    ConsentChanged,
    Pause(bool),
}

pub fn channel() -> (broadcast::Sender<HubEvent>, broadcast::Receiver<HubEvent>) {
    broadcast::channel(BUS_CAPACITY)
}
