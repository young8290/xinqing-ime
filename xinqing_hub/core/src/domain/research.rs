//! 研究模式（07 FR-DMO-04，B-10，ADR 0026）：每天 09:00–22:00 之间随机 3 个时刻、用户不在打字时，
//! 弹出自评邀请（复用“主动报告心情”面板，FR-STA-10）；邀请后答的自评记 `source = esm`。
//!
//! 只在设置了研究编号（`research.id`）且打开开关（`research.enabled`）时生效（ADR 0025）。
//! 每天的 3 个时刻由日期和研究编号确定：同一天重启 Hub 时刻不变，不需要落库。

use chrono::{DateTime, Datelike, Local, NaiveDate, Timelike};

/// 每天的邀请次数。
pub const INVITES_PER_DAY: usize = 3;
/// 邀请时段 [09:00, 22:00)，以当天 0 点起算的分钟数。
pub const START_MIN: u32 = 9 * 60;
pub const END_MIN: u32 = 22 * 60;
/// 同一天两次邀请至少相隔多久（分钟），免得一下午连着弹。
pub const MIN_GAP_MIN: u32 = 90;
/// 到点后一直在打字，最多再等多久（分钟）；过了就跳过这一次，不补。
pub const WAIT_MIN: u32 = 30;
/// 最近这么久内有过输入就算“正在打字”，先不弹（毫秒）。
pub const QUIET_MS: i64 = 10_000;
/// 邀请弹出后多久内答的自评算这次邀请的回答（毫秒）。
pub const ANSWER_MS: i64 = 30 * 60_000;

/// 研究模式是否生效：编号非空且开关打开。
pub fn active(id: &str, enabled: bool) -> bool {
    enabled && !id.trim().is_empty()
}

/// splitmix64：给定种子的可复现伪随机数。
fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// `date` 这天的 3 个邀请时刻（当天 0 点起的分钟数，升序），相邻至少 [`MIN_GAP_MIN`] 分钟。
pub fn plan(date: NaiveDate, id: &str) -> [u32; INVITES_PER_DAY] {
    let mut seed = u64::from(date.num_days_from_ce() as u32);
    for b in id.bytes() {
        seed = seed.wrapping_mul(131).wrapping_add(u64::from(b));
    }
    let span = u64::from(END_MIN - START_MIN);
    for _ in 0..64 {
        let mut slots = [0u32; INVITES_PER_DAY];
        for s in &mut slots {
            *s = START_MIN + (next(&mut seed) % span) as u32;
        }
        slots.sort_unstable();
        if slots.windows(2).all(|w| w[1] - w[0] >= MIN_GAP_MIN) {
            return slots;
        }
    }
    // 极少数种子 64 次都抽不出合格的组合：退回均匀分布
    let step = (END_MIN - START_MIN) / INVITES_PER_DAY as u32;
    std::array::from_fn(|i| START_MIN + step / 2 + step * i as u32)
}

/// 一天里邀请的进度。只在内存里：重启后已过等待期的时刻视为跳过，还在等待期内的照常弹。
#[derive(Debug, Default)]
pub struct Esm {
    date: Option<NaiveDate>,
    slots: [u32; INVITES_PER_DAY],
    done: [bool; INVITES_PER_DAY],
    /// 当前邀请的回答截止时刻（Unix 毫秒）；没有待答的邀请时为 `None`
    answer_until: Option<i64>,
}

impl Esm {
    /// 现在该不该弹邀请。`typing` 为用户正在打字（组字中，或 [`QUIET_MS`] 内有过输入）。
    /// 返回 `true` 时调用方显示邀请，这一次随即记为已完成。
    pub fn poll(&mut self, local: DateTime<Local>, id: &str, typing: bool) -> bool {
        let today = local.date_naive();
        if self.date != Some(today) {
            self.date = Some(today);
            self.slots = plan(today, id);
            self.done = [false; INVITES_PER_DAY];
        }
        let minute = local.hour() * 60 + local.minute();
        let mut show = false;
        for (slot, done) in self.slots.iter().zip(self.done.iter_mut()) {
            if *done || minute < *slot {
                continue;
            }
            if minute >= slot + WAIT_MIN {
                // 一直没等到空档（或 Hub 当时没运行），这一次跳过
                *done = true;
            } else if !typing && !show {
                *done = true;
                show = true;
            }
        }
        if show {
            self.answer_until = Some(local.timestamp_millis() + ANSWER_MS);
        }
        show
    }

    /// 自评时调用：有待答的邀请且没过截止时刻，就把这次自评算作邀请的回答（`source = esm`），并清掉待答状态。
    pub fn take_answer(&mut self, now_ms: i64) -> bool {
        match self.answer_until.take() {
            Some(until) => now_ms <= until,
            None => false,
        }
    }

    /// 用户点了“跳过”。
    pub fn dismiss(&mut self) {
        self.answer_until = None;
    }

    /// 研究模式关掉时清空进度。
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone};

    use super::*;

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, d).unwrap()
    }

    fn at(d: u32, minute: u32) -> DateTime<Local> {
        let naive = day(d).and_hms_opt(minute / 60, minute % 60, 0).unwrap();
        Local.from_local_datetime(&naive).earliest().unwrap()
    }

    #[test]
    fn needs_both_id_and_switch() {
        assert!(active("P01", true));
        assert!(!active("P01", false));
        assert!(!active("", true));
        assert!(!active("  ", true));
    }

    #[test]
    fn three_spaced_slots_within_hours_and_stable_per_day() {
        for d in 1..=31 {
            for id in ["P01", "P02", "xq-2026_07"] {
                let s = plan(day(d), id);
                assert!(s.iter().all(|m| (START_MIN..END_MIN).contains(m)), "{s:?}");
                assert!(s.windows(2).all(|w| w[1] - w[0] >= MIN_GAP_MIN), "{s:?}");
                assert_eq!(s, plan(day(d), id), "同一天同一编号时刻不变");
            }
        }
        assert_ne!(plan(day(5), "P01"), plan(day(6), "P01"), "每天不同");
        assert_ne!(plan(day(5), "P01"), plan(day(5), "P02"), "每个参与者不同");
    }

    #[test]
    fn walks_a_day_waiting_out_typing_and_skipping_stale_slots() {
        let id = "P01";
        let slots = plan(day(5), id);
        let mut e = Esm::default();
        assert!(!e.poll(at(5, START_MIN - 1), id, false), "09:00 前不弹");

        // 第 1 次：到点时在打字，等到空档再弹
        assert!(!e.poll(at(5, slots[0]), id, true));
        assert!(e.poll(at(5, slots[0] + 5), id, false));
        assert!(!e.poll(at(5, slots[0] + 6), id, false), "同一次只弹一回");

        // 第 2 次：一直在打字超过等待期，跳过
        for m in slots[1]..slots[1] + WAIT_MIN {
            assert!(!e.poll(at(5, m), id, true));
        }
        assert!(
            !e.poll(at(5, slots[1] + WAIT_MIN), id, false),
            "过了等待期不补"
        );

        // 第 3 次照常
        assert!(e.poll(at(5, slots[2]), id, false));
        assert!(!e.poll(at(5, END_MIN + 60), id, false));

        // 第二天重新排
        let next = plan(day(6), id);
        assert!(e.poll(at(6, next[0]), id, false));
    }

    #[test]
    fn restart_late_in_the_day_skips_past_slots() {
        let id = "P01";
        let slots = plan(day(5), id);
        let mut e = Esm::default();
        // 第 1 次早已过了等待期：重启后不补；第 2 次还没到
        assert!(!e.poll(at(5, slots[0] + WAIT_MIN + 1), id, false));
        assert!(e.poll(at(5, slots[1]), id, false));
    }

    #[test]
    fn answer_window() {
        let id = "P01";
        let slots = plan(day(5), id);
        let mut e = Esm::default();
        let t = at(5, slots[0]);
        assert!(!e.take_answer(t.timestamp_millis()), "没有邀请时是普通自评");
        assert!(e.poll(t, id, false));
        let late = t + Duration::milliseconds(ANSWER_MS + 1);
        assert!(
            !e.take_answer(late.timestamp_millis()),
            "过了截止时刻算普通自评"
        );

        let t2 = at(5, slots[1]);
        assert!(e.poll(t2, id, false));
        assert!(e.take_answer(t2.timestamp_millis() + 60_000));
        assert!(
            !e.take_answer(t2.timestamp_millis() + 120_000),
            "一次邀请只算一条"
        );

        let t3 = at(5, slots[2]);
        assert!(e.poll(t3, id, false));
        e.dismiss();
        assert!(
            !e.take_answer(t3.timestamp_millis()),
            "跳过之后再自评是普通自评"
        );
    }
}
