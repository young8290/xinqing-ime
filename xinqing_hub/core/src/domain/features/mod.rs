//! 特征流水线（FR-STA-01～03，17 第 2.3 节）。

pub mod baseline;
pub mod calc;
pub mod typo;
pub mod window;

pub use baseline::{Baseline, Bucket};
pub use calc::{SessionCtx, SessionTracker, WindowFeatures, compute};
pub use typo::TypoDetector;
pub use window::{CutReason, WindowBuf, WindowCutter};
