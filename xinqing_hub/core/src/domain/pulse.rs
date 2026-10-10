//! 打字心电图的脉冲（07 FR-DSH-02、FR-DMO-03 演示者视图，docs/adr/0036）：从 XQP `key` 事件里只取
//! “离上一键多久”和“是不是退格”，不含键值、不含任何文字，也不落库。

use serde::Serialize;
use xqp::{KeyKind, KeySrc, Up};

/// 键间间隔的上限：再长的停顿也按这么高的尖峰画，免得一次长停顿把整条波形压平。
pub const MAX_IKI_MS: u32 = 5_000;

/// 一次按键。
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Pulse {
    /// Hub 收到这一键的时刻（Unix 毫秒）
    #[cfg_attr(feature = "specta", specta(type = specta_typescript::Number))]
    pub ts: f64,
    /// 离上一键多久（毫秒，最多 [`MAX_IKI_MS`]）；会话里的第一键为 0
    pub iki_ms: u32,
    /// 退格或删除：成串的在心电图上标红
    pub backspace: bool,
}

/// 把 XQP 上行消息转成脉冲。核心声明了 `tsf_trace` 时只看 tsf 来源，与状态识别同一口径，避免一键算两次。
#[derive(Debug, Default)]
pub struct PulseTracker {
    prefer_tsf: bool,
    last: Option<u64>,
}

impl PulseTracker {
    /// `now_ms` 是 Hub 的当前时刻（XQP 的 `ts` 是会话内的相对毫秒，只用来算间隔）。
    pub fn on_up(&mut self, up: &Up, now_ms: i64) -> Option<Pulse> {
        match up {
            Up::Hello { caps, .. } => {
                self.prefer_tsf = caps.iter().any(|c| c == "tsf_trace");
                self.last = None;
                None
            }
            // 暂停或恢复后重新开始，不把暂停期间画成一个尖峰
            Up::PauseChanged { .. } => {
                self.last = None;
                None
            }
            Up::Key { ts, kind, src, .. } => {
                let want = if self.prefer_tsf {
                    KeySrc::Tsf
                } else {
                    KeySrc::Core
                };
                if *src != want || matches!(kind, KeyKind::Nav | KeyKind::Esc | KeyKind::Other) {
                    return None;
                }
                let iki = self
                    .last
                    .map_or(0, |l| ts.saturating_sub(l).min(u64::from(MAX_IKI_MS)) as u32);
                self.last = Some(*ts);
                Some(Pulse {
                    ts: now_ms as f64,
                    iki_ms: iki,
                    backspace: matches!(kind, KeyKind::Backspace | KeyKind::Delete),
                })
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(ts: u64, kind: KeyKind, src: KeySrc) -> Up {
        Up::Key {
            ts,
            seq: None,
            kind,
            vk: Some(0x41),
            in_comp: false,
            src,
            eaten: None,
            del_committed: None,
        }
    }

    #[test]
    fn intervals_follow_one_source_and_cap_long_pauses() {
        let mut t = PulseTracker::default();
        let a = t.on_up(&key(100, KeyKind::Letter, KeySrc::Core), 1).unwrap();
        assert_eq!((a.iki_ms, a.backspace, a.ts), (0, false, 1.0));
        assert!(t.on_up(&key(150, KeyKind::Letter, KeySrc::Tsf), 2).is_none(), "没声明 tsf_trace 时只看 core");
        assert_eq!(t.on_up(&key(300, KeyKind::Backspace, KeySrc::Core), 3).unwrap().iki_ms, 200);
        assert!(t.on_up(&key(310, KeyKind::Nav, KeySrc::Core), 4).is_none(), "方向键不是打字");
        let long = t.on_up(&key(60_000, KeyKind::Letter, KeySrc::Core), 5).unwrap();
        assert_eq!(long.iki_ms, MAX_IKI_MS);
    }

    #[test]
    fn hello_with_tsf_trace_switches_source_and_pause_resets() {
        let mut t = PulseTracker::default();
        t.on_up(
            &Up::Hello {
                v: 1,
                ime_ver: "x".into(),
                session: "s".into(),
                caps: vec!["tsf_trace".into()],
            },
            0,
        );
        assert!(t.on_up(&key(10, KeyKind::Letter, KeySrc::Core), 0).is_none());
        assert!(t.on_up(&key(10, KeyKind::Letter, KeySrc::Tsf), 0).is_some());
        t.on_up(
            &Up::PauseChanged {
                ts: 20,
                seq: None,
                on: true,
                by: xqp::PauseBy::Hotkey,
            },
            0,
        );
        assert_eq!(t.on_up(&key(9_000, KeyKind::Letter, KeySrc::Tsf), 0).unwrap().iki_ms, 0);
    }
}
