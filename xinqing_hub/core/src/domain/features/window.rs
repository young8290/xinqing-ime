//! 输入窗口切分（FR-STA-01）。
//!
//! 所有时间都用 XQP 会话时间（毫秒），因此回放结果与倍速无关、可复现。
//! 窗口里只存时间戳、按键类别、组字与候选事件，**不存任何文字**。
//!
//! 对规格的一处解释（已写入 docs/adr/0008）：第 2 条“停顿 ≥ 2 秒结束窗口”只在**不处于组字中**
//! 时生效；否则 FR-STA-02 的 `pause_cnt`（组字中 > 3 秒的停顿）永远为 0，R2 也无法由停顿触发。

use xqp::{CandOp, CompOp, KeyKind, KeySrc, Up};

/// 窗口内的一条事件（不含文字）。
#[derive(Debug, Clone, PartialEq)]
pub enum WinEvent {
    Key {
        ts: u64,
        kind: KeyKind,
        vk: Option<u8>,
        in_comp: bool,
        src: KeySrc,
        del_committed: Option<bool>,
    },
    KeyUp {
        ts: u64,
        vk: Option<u8>,
    },
    Comp {
        ts: u64,
        op: CompOp,
        len: Option<u16>,
    },
    Cand {
        ts: u64,
        op: CandOp,
        pos: Option<u32>,
    },
    Commit {
        ts: u64,
        chars: u32,
        cand_pos: i32,
    },
}

impl WinEvent {
    pub fn ts(&self) -> u64 {
        match self {
            WinEvent::Key { ts, .. }
            | WinEvent::KeyUp { ts, .. }
            | WinEvent::Comp { ts, .. }
            | WinEvent::Cand { ts, .. }
            | WinEvent::Commit { ts, .. } => *ts,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct WindowBuf {
    pub events: Vec<WinEvent>,
}

impl WindowBuf {
    pub fn start_ts(&self) -> u64 {
        self.events.first().map(WinEvent::ts).unwrap_or(0)
    }

    pub fn end_ts(&self) -> u64 {
        self.events.last().map(WinEvent::ts).unwrap_or(0)
    }

    /// 特征计算使用的按键来源：窗口里有 tsf 按键（FR-SEN-08）时只用 tsf，否则用 core。
    pub fn key_src(&self) -> KeySrc {
        if self.events.iter().any(|e| {
            matches!(
                e,
                WinEvent::Key {
                    src: KeySrc::Tsf,
                    ..
                }
            )
        }) {
            KeySrc::Tsf
        } else {
            KeySrc::Core
        }
    }

    /// 按键数（不含修饰键组合，即 kind = other），只数 `key_src()` 选定的来源，
    /// 与 `calc::compute` 的 `n_keys` 一致，避免 core + tsf 重复计数让小窗口越过 5 键门槛。
    pub fn n_keys(&self) -> usize {
        let want = self.key_src();
        self.events
            .iter()
            .filter(|e| matches!(e, WinEvent::Key { kind, src, .. } if *kind != KeyKind::Other && *src == want))
            .count()
    }
}

/// 窗口为何结束（日志与测试用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CutReason {
    AfterCommit,
    Pause,
    MaxDuration,
    FocusChange,
    ImeInactive,
}

pub const COMMIT_IDLE_MS: u64 = 1_000;
pub const PAUSE_MS: u64 = 2_000;
pub const MAX_WINDOW_MS: u64 = 30_000;
pub const MIN_KEYS: usize = 5;
pub const SMALL_MERGE_MS: u64 = 60_000;

#[derive(Debug, Default)]
pub struct WindowCutter {
    cur: Option<WindowBuf>,
    pending_small: Option<WindowBuf>,
    last_key_ts: Option<u64>,
    commit_at: Option<u64>,
    in_comp: bool,
    last_app: Option<String>,
    /// 最近一次结束原因，供调试。
    pub last_cut: Option<CutReason>,
}

impl WindowCutter {
    pub fn new() -> Self {
        Self::default()
    }

    /// 每条上行事件调用一次；返回已结束且可分析的窗口（0 或 1 个）。
    pub fn push(&mut self, ev: &Up) -> Option<WindowBuf> {
        match ev {
            Up::Key {
                ts,
                kind,
                vk,
                in_comp,
                src,
                del_committed,
                ..
            } => {
                let mut out = None;
                if self.cur.is_some() {
                    out = self.cut_before_key(*ts);
                }
                let w = self.cur.get_or_insert_with(WindowBuf::default);
                w.events.push(WinEvent::Key {
                    ts: *ts,
                    kind: *kind,
                    vk: *vk,
                    in_comp: *in_comp,
                    src: *src,
                    del_committed: *del_committed,
                });
                self.last_key_ts = Some(*ts);
                self.commit_at = None;
                out
            }
            Up::KeyUp { ts, vk, .. } => {
                if let Some(w) = &mut self.cur {
                    w.events.push(WinEvent::KeyUp { ts: *ts, vk: *vk });
                }
                None
            }
            Up::Comp { ts, op, len, .. } => {
                self.in_comp = matches!(op, CompOp::Update) && len.unwrap_or(1) > 0;
                if let Some(w) = &mut self.cur {
                    w.events.push(WinEvent::Comp {
                        ts: *ts,
                        op: *op,
                        len: *len,
                    });
                }
                None
            }
            Up::Cand { ts, op, pos, .. } => {
                if let Some(w) = &mut self.cur {
                    w.events.push(WinEvent::Cand {
                        ts: *ts,
                        op: *op,
                        pos: *pos,
                    });
                }
                None
            }
            Up::Commit {
                ts,
                chars,
                cand_pos,
                ..
            } => {
                self.in_comp = false;
                if let Some(w) = &mut self.cur {
                    w.events.push(WinEvent::Commit {
                        ts: *ts,
                        chars: *chars,
                        cand_pos: *cand_pos,
                    });
                    self.commit_at = Some(*ts);
                }
                None
            }
            Up::Focus { app, blocked, .. } => {
                let app = if *blocked { None } else { app.clone() };
                let changed = app != self.last_app;
                self.last_app = app;
                if changed {
                    self.in_comp = false;
                    return self.close(CutReason::FocusChange);
                }
                None
            }
            Up::Ime { active, .. } => {
                if !*active {
                    self.in_comp = false;
                    return self.close(CutReason::ImeInactive);
                }
                None
            }
            Up::PauseChanged { on: true, .. } => {
                self.discard();
                None
            }
            _ => None,
        }
    }

    /// 定时调用（Hub 中每 250 ms），`now` 为当前会话时间。
    pub fn tick(&mut self, now: u64) -> Option<WindowBuf> {
        if let Some(p) = &self.pending_small {
            if self.cur.is_none() && now.saturating_sub(p.end_ts()) > SMALL_MERGE_MS {
                self.pending_small = None;
            }
        }
        self.cur.as_ref()?;
        if let Some(c) = self.commit_at {
            if now.saturating_sub(c) >= COMMIT_IDLE_MS {
                return self.close(CutReason::AfterCommit);
            }
        }
        if let Some(k) = self.last_key_ts {
            if !self.in_comp && now.saturating_sub(k) >= PAUSE_MS {
                return self.close(CutReason::Pause);
            }
        }
        if now.saturating_sub(self.cur.as_ref()?.start_ts()) >= MAX_WINDOW_MS {
            return self.close(CutReason::MaxDuration);
        }
        None
    }

    /// 无痕模式开启：当前窗口作废，不计算（FR-SEN-06 第 1 条）。
    pub fn discard(&mut self) {
        self.cur = None;
        self.pending_small = None;
        self.commit_at = None;
        self.last_key_ts = None;
        self.in_comp = false;
    }

    /// 是否正在组字（供窗口结束判定和测试）。
    pub fn in_comp(&self) -> bool {
        self.in_comp
    }

    fn cut_before_key(&mut self, ts: u64) -> Option<WindowBuf> {
        if let Some(c) = self.commit_at {
            if ts.saturating_sub(c) >= COMMIT_IDLE_MS {
                return self.close(CutReason::AfterCommit);
            }
        }
        if let Some(k) = self.last_key_ts {
            if !self.in_comp && ts.saturating_sub(k) >= PAUSE_MS {
                return self.close(CutReason::Pause);
            }
        }
        let start = self.cur.as_ref().map(WindowBuf::start_ts).unwrap_or(ts);
        if ts.saturating_sub(start) >= MAX_WINDOW_MS {
            return self.close(CutReason::MaxDuration);
        }
        None
    }

    fn close(&mut self, reason: CutReason) -> Option<WindowBuf> {
        let w = self.cur.take()?;
        self.commit_at = None;
        self.last_cut = Some(reason);
        let w = match self.pending_small.take() {
            Some(mut p) if w.start_ts().saturating_sub(p.end_ts()) <= SMALL_MERGE_MS => {
                p.events.extend(w.events);
                p
            }
            _ => w,
        };
        if w.n_keys() < MIN_KEYS {
            self.pending_small = Some(w);
            return None;
        }
        Some(w)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(ts: u64) -> Up {
        Up::Key {
            ts,
            seq: None,
            kind: KeyKind::Letter,
            vk: Some(b'A'),
            in_comp: false,
            src: KeySrc::Core,
            eaten: None,
            del_committed: None,
        }
    }

    fn commit(ts: u64) -> Up {
        Up::Commit {
            ts,
            seq: None,
            chars: 2,
            keystrokes: 6,
            cand_pos: 0,
            src: "candidate".into(),
            text: None,
            truncated: None,
        }
    }

    fn comp_update(ts: u64) -> Up {
        Up::Comp {
            ts,
            seq: None,
            op: CompOp::Update,
            len: Some(2),
        }
    }

    #[test]
    fn commit_then_idle_one_second_ends_window() {
        let mut c = WindowCutter::new();
        for i in 0..6 {
            assert!(c.push(&key(i * 150)).is_none());
        }
        c.push(&commit(900));
        assert!(c.tick(1500).is_none());
        let w = c.tick(1900).expect("上屏后 1 秒应结束窗口");
        assert_eq!(w.n_keys(), 6);
        assert_eq!(c.last_cut, Some(CutReason::AfterCommit));
    }

    #[test]
    fn enter_right_after_commit_stays_in_window() {
        let mut c = WindowCutter::new();
        for i in 0..6 {
            c.push(&key(i * 150));
        }
        c.push(&commit(900));
        assert!(c.push(&key(1200)).is_none());
        let w = c.tick(3300).unwrap();
        assert_eq!(w.n_keys(), 7);
    }

    #[test]
    fn pause_outside_composition_ends_window() {
        let mut c = WindowCutter::new();
        for i in 0..6 {
            c.push(&key(i * 150));
        }
        let w = c.push(&key(750 + 2_500)).expect("停顿 ≥ 2 秒应结束窗口");
        assert_eq!(w.n_keys(), 6);
        assert_eq!(c.last_cut, Some(CutReason::Pause));
    }

    #[test]
    fn pause_inside_composition_does_not_end_window() {
        let mut c = WindowCutter::new();
        for i in 0..6 {
            c.push(&key(i * 150));
        }
        c.push(&comp_update(760));
        assert!(c.tick(5_000).is_none());
        assert!(c.push(&key(5_000)).is_none());
    }

    #[test]
    fn thirty_seconds_caps_window() {
        let mut c = WindowCutter::new();
        let mut out = None;
        let mut ts = 0;
        c.push(&comp_update(0));
        while out.is_none() {
            out = c.push(&key(ts));
            ts += 300;
        }
        assert_eq!(c.last_cut, Some(CutReason::MaxDuration));
        assert!(out.unwrap().end_ts() < MAX_WINDOW_MS);
    }

    #[test]
    fn small_window_merges_into_next_within_60s() {
        let mut c = WindowCutter::new();
        for i in 0..3 {
            c.push(&key(i * 150));
        }
        assert!(c.tick(3_000).is_none(), "3 键窗口不单独输出");
        for i in 0..5 {
            c.push(&key(10_000 + i * 150));
        }
        let w = c.tick(13_000).unwrap();
        assert_eq!(w.n_keys(), 8);
    }

    #[test]
    fn small_window_dropped_after_60s() {
        let mut c = WindowCutter::new();
        for i in 0..3 {
            c.push(&key(i * 150));
        }
        c.tick(3_000);
        c.tick(70_000);
        for i in 0..5 {
            c.push(&key(80_000 + i * 150));
        }
        assert_eq!(c.tick(83_000).unwrap().n_keys(), 5);
    }

    #[test]
    fn core_and_tsf_copies_of_same_keys_count_once() {
        let mut c = WindowCutter::new();
        for i in 0..3 {
            c.push(&key(i * 150));
            c.push(&Up::Key {
                ts: i * 150,
                seq: None,
                kind: KeyKind::Letter,
                vk: Some(b'A'),
                in_comp: false,
                src: KeySrc::Tsf,
                eaten: Some(true),
                del_committed: Some(false),
            });
        }
        assert!(
            c.tick(3_000).is_none(),
            "3 个物理按键（core + tsf 各 3 条）仍是小窗口"
        );
    }

    #[test]
    fn pause_mode_discards_current_window() {
        let mut c = WindowCutter::new();
        for i in 0..8 {
            c.push(&key(i * 150));
        }
        c.push(&Up::PauseChanged {
            ts: 1300,
            seq: None,
            on: true,
            by: xqp::PauseBy::Hotkey,
        });
        assert!(c.tick(10_000).is_none());
    }
}
