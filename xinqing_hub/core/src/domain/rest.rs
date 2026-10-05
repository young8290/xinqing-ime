//! 休息提醒（06 FR-RST-01～07，17 第 2.8 节）：使用时长计时、四类提醒的到期判断、显示时机与优先级。
//!
//! 纯状态机，不读时钟、不碰数据库：调用方把“现在”（Unix 毫秒 + 本地时间）和输入活动喂进来，
//! 每次 [`RestEngine::poll`] 问一次“现在该不该提醒、提醒哪一类”。服务（`crate::rest`）负责接总线、读设置、显示和写库。
//!
//! - **活跃分钟**：这一分钟内有输入活动；**连续使用**：一串活跃分钟，中间不活跃的间隔 < 2 分钟（FR-RST-01）；
//! - 护眼、活动按“自上次清零以来的连续使用分钟”计，喝水按累计使用分钟计，深夜按深夜时段内的连续使用分钟计；
//! - 到期后不立即显示，等时机：上屏后空闲 3 秒、没有上屏时空闲 5 秒、不在组字中、不在勿扰（FR-RST-06）；
//! - 同类每小时最多 1 次；同时到期只显示优先级最高的一个，其余顺延 5 分钟（深夜 > 活动 > 护眼 > 喝水）。

use chrono::{DateTime, Duration, Local, NaiveTime, TimeZone, Timelike};
use serde::{Deserialize, Serialize};

/// 不活跃满这么多分钟，连续计时清零。
const GAP_RESET_MIN: i64 = 2;
/// 深夜时段内连续使用超过这么多分钟才提醒。
const NIGHT_CONT_MIN: u32 = 10;
/// 深夜提醒每晚最多次数、两次之间的最小间隔。
const NIGHT_MAX: u32 = 2;
const NIGHT_GAP_MS: i64 = 60 * 60_000;
/// 深夜时段到早上 6 点为止（与 05 FR-REV 的“当晚”口径一致）。
const NIGHT_END_MIN: u32 = 6 * 60;
/// 同类提醒的最小间隔。
const SAME_KIND_GAP_MS: i64 = 60 * 60_000;
/// 同时到期时其余顺延、“5 分钟后”的推迟时长。
const DEFER_MS: i64 = 5 * 60_000;
/// 上屏后、没有上屏时分别要空闲多久才显示。
const IDLE_AFTER_COMMIT_MS: i64 = 3_000;
const IDLE_MS: i64 = 5_000;
/// 系统空闲时间小于这个值，就当这 10 秒内有输入（每 10 秒读一次）。
pub const SYSTEM_IDLE_ACTIVE_MS: u64 = 10_000;
/// 疲劳联动：距上次护眼清零至少这么多分钟才提前提醒（FR-RST-07）。
const TIRED_EYE_MIN: u32 = 10;

/// 深夜提醒可选的起始时刻（22:00–01:00，每半小时一档）。
pub const NIGHT_STARTS: &[&str] = &[
    "22:00", "22:30", "23:00", "23:30", "00:00", "00:30", "01:00",
];

/// 四类提醒，声明顺序即优先级（高 → 低）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum RestKind {
    Night,
    Move,
    Eye,
    Water,
}

impl RestKind {
    pub const ALL: [RestKind; 4] = [
        RestKind::Night,
        RestKind::Move,
        RestKind::Eye,
        RestKind::Water,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            RestKind::Night => "night",
            RestKind::Move => "move",
            RestKind::Eye => "eye",
            RestKind::Water => "water",
        }
    }

    /// 光标旁气泡的短文案（≤ 16 字，10 第 2.5 节；与 `ui_copy.toml` 的 `tip.rest_*` 一致）。
    pub fn tip(self) -> &'static str {
        match self {
            RestKind::Night => "不早了，早点休息",
            RestKind::Move => "起来走两步吧",
            RestKind::Eye => "看看远处吧",
            RestKind::Water => "喝口水吧",
        }
    }
}

/// 用户对提醒卡片的操作（FR-RST-06 第 2 条），也是 `reminder_log.action` 的取值。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum RestAction {
    /// 已完成（喝水卡片上叫“喝了”）
    Done,
    /// 5 分钟后
    Later,
    /// 今天不再提醒
    TodayOff,
}

impl RestAction {
    pub fn as_str(self) -> &'static str {
        match self {
            RestAction::Done => "done",
            RestAction::Later => "later",
            RestAction::TodayOff => "today_off",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KindCfg {
    pub enabled: bool,
    pub interval_min: u32,
}

/// 设置 `rest.*`（FR-RST-09）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RestConfig {
    pub eye: KindCfg,
    pub water: KindCfg,
    pub mv: KindCfg,
    pub night_enabled: bool,
    /// 深夜提醒起始时刻，当天的第几分钟
    pub night_start_min: u32,
}

impl Default for RestConfig {
    fn default() -> Self {
        Self {
            eye: KindCfg {
                enabled: true,
                interval_min: 20,
            },
            water: KindCfg {
                enabled: true,
                interval_min: 60,
            },
            mv: KindCfg {
                enabled: true,
                interval_min: 50,
            },
            night_enabled: true,
            night_start_min: 23 * 60 + 30,
        }
    }
}

/// `"23:30"` → 1410。格式不对时返回 `None`。
pub fn parse_hhmm(s: &str) -> Option<u32> {
    let t = NaiveTime::parse_from_str(s, "%H:%M").ok()?;
    Some(t.hour() * 60 + t.minute())
}

/// 某一时刻是否在深夜时段（起始时刻到次日 06:00）。
pub fn in_night(minute_of_day: u32, start_min: u32) -> bool {
    if start_min >= 12 * 60 {
        minute_of_day >= start_min || minute_of_day < NIGHT_END_MIN
    } else {
        (start_min..NIGHT_END_MIN).contains(&minute_of_day)
    }
}

/// 输入活动，来自 XQP 上行（只看“有没有在打字”和组字状态，不看内容）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    Key,
    CompUpdate,
    /// 组字结束（取消、清空、被打断）
    CompEnd,
    Commit,
}

/// 一次要显示的提醒。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Due {
    pub kind: RestKind,
    /// 疲劳联动提前触发的护眼提醒，卡片换文案“打了很久啦，眼睛也累了吧”（FR-RST-07）
    pub tired: bool,
}

/// 显示时机的外部条件。
#[derive(Debug, Clone, Copy, Default)]
pub struct Ctx {
    /// 前台全屏、勿扰应用、系统专注助手（FR-RST-06 第 3 条）
    pub dnd: bool,
    /// 用系统空闲时间计时的时候（无痕、英文状态、没连上输入法）给出当前空闲毫秒，此时没有组字信息
    pub system_idle_ms: Option<u64>,
}

/// 活跃分钟记录（FR-RST-01）。分钟按 Unix 时间切（`ms / 60000`）。
#[derive(Debug, Default)]
struct Activity {
    last_min: Option<i64>,
    run_start_min: i64,
}

impl Activity {
    /// 记一次输入活动。进入新的一分钟时返回 `Some(是否与上一串连续)`，同一分钟内重复的返回 `None`。
    fn mark(&mut self, ms: i64) -> Option<bool> {
        let m = ms.div_euclid(60_000);
        if self.last_min.is_some_and(|l| m <= l) {
            return None;
        }
        let continued = self.last_min.is_some_and(|l| m - l - 1 < GAP_RESET_MIN);
        if !continued {
            self.run_start_min = m;
        }
        self.last_min = Some(m);
        Some(continued)
    }

    /// 现在是否还在“连续使用”中（不活跃还没满 2 分钟）。
    fn in_run(&self, ms: i64) -> bool {
        let m = ms.div_euclid(60_000);
        self.last_min.is_some_and(|l| m - l - 1 < GAP_RESET_MIN)
    }

    /// 连续使用分钟数（到最近一个活跃分钟为止）；已清零时为 0。
    fn continuous_min(&self, ms: i64) -> u32 {
        match self.last_min {
            Some(l) if self.in_run(ms) => (l - self.run_start_min + 1) as u32,
            _ => 0,
        }
    }
}

#[derive(Debug, Default)]
pub struct RestEngine {
    cfg: RestConfig,
    activity: Activity,
    /// 自上次清零以来的连续使用分钟（护眼、活动）、累计使用分钟（喝水）、深夜时段内的连续使用分钟
    eye_min: u32,
    move_min: u32,
    water_min: u32,
    night_min: u32,
    /// 最近一个活跃分钟是否在深夜时段
    last_in_night: bool,
    /// 各类最近一次显示的时刻、顺延到的时刻、“今天不再提醒”到的时刻（Unix 毫秒），下标同 [`RestKind::ALL`]
    last_shown: [Option<i64>; 4],
    deferred_until: [i64; 4],
    off_until: [i64; 4],
    /// 当晚（以早上 6 点分界）已提醒的深夜次数
    night_shown: (Option<chrono::NaiveDate>, u32),
    composing: bool,
    last_input_ms: Option<i64>,
    after_commit: bool,
    tired: bool,
}

fn idx(kind: RestKind) -> usize {
    RestKind::ALL.iter().position(|k| *k == kind).unwrap()
}

/// “当晚”的日期：早上 6 点前算前一天。
fn night_date(local: DateTime<Local>) -> chrono::NaiveDate {
    (local - Duration::minutes(i64::from(NIGHT_END_MIN))).date_naive()
}

/// 下一个本地 `hh:00` 时刻（Unix 毫秒）。夏令时跳过的时刻取最早的合法时刻。
fn next_local_hour(local: DateTime<Local>, hour: u32) -> i64 {
    let today = local.date_naive().and_hms_opt(hour, 0, 0).unwrap();
    let target = if local.naive_local() < today {
        today
    } else {
        today + Duration::days(1)
    };
    Local
        .from_local_datetime(&target)
        .earliest()
        .map_or(local.timestamp_millis() + 86_400_000, |t| {
            t.timestamp_millis()
        })
}

impl RestEngine {
    pub fn new(cfg: RestConfig) -> Self {
        Self {
            cfg,
            ..Self::default()
        }
    }

    pub fn set_config(&mut self, cfg: RestConfig) {
        self.cfg = cfg;
    }

    pub fn config(&self) -> RestConfig {
        self.cfg
    }

    /// 当前连续使用分钟数（FR-RST-01）。
    pub fn continuous_min(&self, now_ms: i64) -> u32 {
        self.activity.continuous_min(now_ms)
    }

    /// XQP 上行带来的输入活动。
    pub fn on_input(&mut self, input: Input, local: DateTime<Local>) {
        let ms = local.timestamp_millis();
        self.last_input_ms = Some(ms);
        match input {
            Input::Key => {
                self.after_commit = false;
                self.active(local);
            }
            Input::CompUpdate => {
                self.composing = true;
                self.after_commit = false;
            }
            Input::CompEnd => self.composing = false,
            Input::Commit => {
                self.composing = false;
                self.after_commit = true;
                self.active(local);
            }
        }
    }

    /// 用系统空闲时间计时（每 10 秒一次）：空闲小于 10 秒就算这段时间在用电脑。
    pub fn on_system_idle(&mut self, idle_ms: u64, local: DateTime<Local>) {
        // 换了来源，之前记的组字状态不再可信
        self.composing = false;
        if idle_ms < SYSTEM_IDLE_ACTIVE_MS {
            self.active(local);
        }
    }

    /// 显示状态是否为 `tired`（FR-RST-07）。
    pub fn set_tired(&mut self, tired: bool) {
        self.tired = tired;
    }

    fn active(&mut self, local: DateTime<Local>) {
        let ms = local.timestamp_millis();
        let Some(continued) = self.activity.mark(ms) else {
            return;
        };
        let minute = local.hour() * 60 + local.minute();
        let night = in_night(minute, self.cfg.night_start_min);
        if continued {
            self.eye_min += 1;
            self.move_min += 1;
        } else {
            self.eye_min = 1;
            self.move_min = 1;
        }
        self.water_min += 1;
        self.night_min = match (night, continued && self.last_in_night) {
            (true, true) => self.night_min + 1,
            (true, false) => 1,
            (false, _) => 0,
        };
        self.last_in_night = night;
    }

    /// 某类提醒的条件是否满足（不看时机和频率）。`tired_early` 表示护眼是疲劳联动提前满足的。
    fn condition(&self, kind: RestKind, now_ms: i64, local: DateTime<Local>) -> Option<bool> {
        let in_run = self.activity.in_run(now_ms);
        match kind {
            RestKind::Eye => {
                let c = self.cfg.eye;
                let eye = if in_run { self.eye_min } else { 0 };
                if !c.enabled {
                    None
                } else if eye >= c.interval_min {
                    Some(false)
                } else if self.tired && eye >= TIRED_EYE_MIN {
                    Some(true)
                } else {
                    None
                }
            }
            RestKind::Move => {
                let c = self.cfg.mv;
                (c.enabled && in_run && self.move_min >= c.interval_min).then_some(false)
            }
            RestKind::Water => {
                let c = self.cfg.water;
                (c.enabled && self.water_min >= c.interval_min).then_some(false)
            }
            RestKind::Night => {
                let minute = local.hour() * 60 + local.minute();
                let tonight = night_date(local);
                let shown = match self.night_shown {
                    (Some(d), n) if d == tonight => n,
                    _ => 0,
                };
                let spaced = self.last_shown[idx(RestKind::Night)]
                    .is_none_or(|t| now_ms - t >= NIGHT_GAP_MS);
                (self.cfg.night_enabled
                    && in_run
                    && in_night(minute, self.cfg.night_start_min)
                    && self.night_min > NIGHT_CONT_MIN
                    && shown < NIGHT_MAX
                    && spaced)
                    .then_some(false)
            }
        }
    }

    /// 时机是否合适（FR-RST-06 第 1、3 条）。
    fn timing_ok(&self, now_ms: i64, ctx: Ctx) -> bool {
        if ctx.dnd {
            return false;
        }
        if let Some(idle) = ctx.system_idle_ms {
            return idle >= IDLE_MS as u64;
        }
        if self.composing {
            return false;
        }
        let need = if self.after_commit {
            IDLE_AFTER_COMMIT_MS
        } else {
            IDLE_MS
        };
        self.last_input_ms.is_none_or(|t| now_ms - t >= need)
    }

    /// 现在该显示哪一个提醒。返回 `Some` 时已记为“已显示”，同时到期的其余几类顺延 5 分钟。
    pub fn poll(&mut self, local: DateTime<Local>, ctx: Ctx) -> Option<Due> {
        let now_ms = local.timestamp_millis();
        let due: Vec<(RestKind, bool)> = RestKind::ALL
            .into_iter()
            .filter(|k| {
                let i = idx(*k);
                now_ms >= self.deferred_until[i]
                    && now_ms >= self.off_until[i]
                    && self.last_shown[i].is_none_or(|t| now_ms - t >= SAME_KIND_GAP_MS)
            })
            .filter_map(|k| self.condition(k, now_ms, local).map(|t| (k, t)))
            .collect();
        let &(kind, tired) = due.first()?;
        if !self.timing_ok(now_ms, ctx) {
            return None;
        }
        for (other, _) in &due[1..] {
            self.deferred_until[idx(*other)] = now_ms + DEFER_MS;
        }
        self.last_shown[idx(kind)] = Some(now_ms);
        if kind == RestKind::Night {
            let tonight = night_date(local);
            self.night_shown = match self.night_shown {
                (Some(d), n) if d == tonight => (Some(d), n + 1),
                _ => (Some(tonight), 1),
            };
        }
        Some(Due { kind, tired })
    }

    /// 用户点了提醒卡片上的按钮（FR-RST-02～06）。完成或关闭后该类计时清零。
    pub fn act(&mut self, kind: RestKind, action: RestAction, local: DateTime<Local>) {
        let now_ms = local.timestamp_millis();
        let i = idx(kind);
        match action {
            RestAction::Done => self.reset(kind),
            RestAction::Later => {
                self.deferred_until[i] = now_ms + DEFER_MS;
                // “5 分钟后”是用户要的，不受“同类每小时 1 次”限制
                self.last_shown[i] = None;
            }
            RestAction::TodayOff => {
                self.reset(kind);
                // 深夜提醒的“今天”到早上 6 点，其余到午夜
                let hour = if kind == RestKind::Night {
                    NIGHT_END_MIN / 60
                } else {
                    0
                };
                self.off_until[i] = next_local_hour(local, hour);
            }
        }
    }

    fn reset(&mut self, kind: RestKind) {
        match kind {
            RestKind::Eye => self.eye_min = 0,
            RestKind::Move => self.move_min = 0,
            RestKind::Water => self.water_min = 0,
            RestKind::Night => self.night_min = 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn at(h: u32, m: u32) -> DateTime<Local> {
        let naive = NaiveDate::from_ymd_opt(2026, 3, 10)
            .unwrap()
            .and_hms_opt(h, m, 0)
            .unwrap();
        Local.from_local_datetime(&naive).earliest().unwrap()
    }

    /// 从 `start` 起连续打字 `minutes` 分钟（每分钟两次按键、最后一次上屏），返回结束时刻。
    fn type_for(e: &mut RestEngine, start: DateTime<Local>, minutes: i64) -> DateTime<Local> {
        let mut t = start;
        for _ in 0..minutes {
            e.on_input(Input::Key, t);
            e.on_input(Input::Commit, t + Duration::seconds(20));
            t += Duration::minutes(1);
        }
        t - Duration::seconds(40)
    }

    fn idle(t: DateTime<Local>, secs: i64) -> DateTime<Local> {
        t + Duration::seconds(secs)
    }

    fn only(cfg_kind: RestKind) -> RestConfig {
        let mut c = RestConfig::default();
        c.eye.enabled = cfg_kind == RestKind::Eye;
        c.water.enabled = cfg_kind == RestKind::Water;
        c.mv.enabled = cfg_kind == RestKind::Move;
        c.night_enabled = cfg_kind == RestKind::Night;
        c
    }

    #[test]
    fn continuous_survives_a_one_minute_pause_and_resets_after_three() {
        // TC-RST-01：25 分钟输入（中间停 1 分钟）连续计时 25；再停 3 分钟清零
        let mut e = RestEngine::new(RestConfig::default());
        let t = type_for(&mut e, at(10, 0), 12);
        let t = type_for(&mut e, t + Duration::minutes(1) + Duration::seconds(40), 12);
        assert_eq!(e.continuous_min(t.timestamp_millis()), 25);
        let later = t + Duration::minutes(3) + Duration::seconds(30);
        assert_eq!(e.continuous_min(later.timestamp_millis()), 0);
    }

    #[test]
    fn eye_reminder_after_twenty_minutes_waits_for_idle_after_commit() {
        let mut e = RestEngine::new(only(RestKind::Eye));
        let t = type_for(&mut e, at(10, 0), 19);
        assert_eq!(e.poll(idle(t, 10), Ctx::default()), None, "19 分钟还没到");
        let t = type_for(&mut e, t + Duration::seconds(40), 1);
        // 上屏后不到 3 秒不显示
        assert_eq!(e.poll(idle(t, 2), Ctx::default()), None);
        assert_eq!(
            e.poll(idle(t, 3), Ctx::default()),
            Some(Due {
                kind: RestKind::Eye,
                tired: false
            })
        );
    }

    #[test]
    fn never_shown_while_composing() {
        let mut e = RestEngine::new(only(RestKind::Eye));
        let t = type_for(&mut e, at(10, 0), 20);
        e.on_input(Input::CompUpdate, idle(t, 1));
        assert_eq!(e.poll(idle(t, 30), Ctx::default()), None, "组字中");
        e.on_input(Input::CompEnd, idle(t, 31));
        // 没有上屏的组字结束：要空闲 5 秒
        assert_eq!(e.poll(idle(t, 34), Ctx::default()), None);
        assert!(e.poll(idle(t, 36), Ctx::default()).is_some());
    }

    #[test]
    fn same_kind_at_most_once_an_hour_until_acted_on() {
        let mut e = RestEngine::new(only(RestKind::Eye));
        let t = type_for(&mut e, at(10, 0), 20);
        assert!(e.poll(idle(t, 5), Ctx::default()).is_some());
        // 没理会这张卡片，继续打字：一小时内不再弹
        let t = type_for(&mut e, idle(t, 20), 30);
        assert_eq!(e.poll(idle(t, 5), Ctx::default()), None);
        let t = type_for(&mut e, idle(t, 20), 31);
        assert!(e.poll(idle(t, 5), Ctx::default()).is_some(), "过了一小时");
    }

    #[test]
    fn done_resets_the_timer() {
        let mut e = RestEngine::new(only(RestKind::Eye));
        let t = type_for(&mut e, at(10, 0), 20);
        assert!(e.poll(idle(t, 5), Ctx::default()).is_some());
        e.act(RestKind::Eye, RestAction::Done, idle(t, 30));
        // 清零后重新数 20 分钟；同类间隔一小时的限制同样要满足
        let t = type_for(&mut e, idle(t, 40), 20);
        assert_eq!(e.poll(idle(t, 5), Ctx::default()), None);
    }

    #[test]
    fn later_comes_back_after_five_minutes() {
        let mut e = RestEngine::new(only(RestKind::Eye));
        let t = type_for(&mut e, at(10, 0), 20);
        assert!(e.poll(idle(t, 5), Ctx::default()).is_some());
        e.act(RestKind::Eye, RestAction::Later, idle(t, 6));
        let t2 = type_for(&mut e, idle(t, 20), 4);
        assert_eq!(e.poll(idle(t2, 5), Ctx::default()), None, "还没到 5 分钟");
        let t3 = type_for(&mut e, idle(t2, 20), 2);
        assert!(e.poll(idle(t3, 5), Ctx::default()).is_some());
    }

    #[test]
    fn today_off_holds_until_midnight() {
        let mut c = only(RestKind::Water);
        c.water.interval_min = 30;
        let mut e = RestEngine::new(c);
        let t = type_for(&mut e, at(21, 0), 30);
        assert!(e.poll(idle(t, 5), Ctx::default()).is_some());
        e.act(RestKind::Water, RestAction::TodayOff, idle(t, 6));
        let t = type_for(&mut e, idle(t, 20), 60);
        assert_eq!(e.poll(idle(t, 5), Ctx::default()), None, "今天不再提醒");
        // 跨过午夜之后恢复
        let t = type_for(&mut e, idle(t, 20), 100);
        assert!(t.date_naive() > at(0, 0).date_naive());
        assert!(e.poll(idle(t, 5), Ctx::default()).is_some(), "第二天恢复");
    }

    #[test]
    fn water_counts_cumulative_minutes_across_breaks() {
        let mut e = RestEngine::new(only(RestKind::Water));
        let t = type_for(&mut e, at(9, 0), 30);
        // 歇 10 分钟，连续计时清零但累计不清零
        let t = type_for(&mut e, t + Duration::minutes(10), 30);
        assert!(e.poll(idle(t, 5), Ctx::default()).is_some());
    }

    #[test]
    fn priority_and_deferral_when_several_are_due() {
        // TC-RST-03：同时到期只显示最高优先级，其余顺延 5 分钟
        let mut c = RestConfig {
            night_enabled: false,
            ..RestConfig::default()
        };
        c.eye.interval_min = 50;
        c.water.interval_min = 50;
        let mut e = RestEngine::new(c);
        let t = type_for(&mut e, at(10, 0), 50);
        let first = e.poll(idle(t, 5), Ctx::default()).unwrap();
        assert_eq!(first.kind, RestKind::Move);
        assert_eq!(e.poll(idle(t, 10), Ctx::default()), None, "其余顺延");
        let t2 = type_for(&mut e, idle(t, 20), 6);
        assert_eq!(
            e.poll(idle(t2, 5), Ctx::default()).map(|d| d.kind),
            Some(RestKind::Eye)
        );
        // 喝水再顺延 5 分钟
        assert_eq!(e.poll(idle(t2, 6), Ctx::default()), None);
        let t3 = type_for(&mut e, idle(t2, 20), 6);
        assert_eq!(
            e.poll(idle(t3, 5), Ctx::default()).map(|d| d.kind),
            Some(RestKind::Water)
        );
    }

    #[test]
    fn dnd_postpones_and_then_shows_once() {
        let mut e = RestEngine::new(only(RestKind::Eye));
        let t = type_for(&mut e, at(10, 0), 45);
        let dnd = Ctx {
            dnd: true,
            ..Ctx::default()
        };
        assert_eq!(e.poll(idle(t, 5), dnd), None);
        assert!(
            e.poll(idle(t, 6), Ctx::default()).is_some(),
            "退出勿扰后补一次"
        );
        assert_eq!(e.poll(idle(t, 7), Ctx::default()), None, "只补一次");
    }

    #[test]
    fn night_reminder_twice_at_most_an_hour_apart() {
        // TC-RST-08：23:35 起连续 15 分钟出现深夜提醒；同一晚间隔 ≥ 60 分钟，最多 2 次
        let mut e = RestEngine::new(only(RestKind::Night));
        let t = type_for(&mut e, at(23, 35), 10);
        assert_eq!(e.poll(idle(t, 5), Ctx::default()), None, "只有 10 分钟");
        let t = type_for(&mut e, idle(t, 20), 1);
        assert_eq!(
            e.poll(idle(t, 5), Ctx::default()).map(|d| d.kind),
            Some(RestKind::Night)
        );
        e.act(RestKind::Night, RestAction::Done, idle(t, 6));
        let t = type_for(&mut e, idle(t, 20), 30);
        assert_eq!(e.poll(idle(t, 5), Ctx::default()), None, "间隔不足 60 分钟");
        let t = type_for(&mut e, idle(t, 20), 31);
        assert!(e.poll(idle(t, 5), Ctx::default()).is_some(), "第二次");
        e.act(RestKind::Night, RestAction::Done, idle(t, 6));
        let t = type_for(&mut e, idle(t, 20), 75);
        assert_eq!(e.poll(idle(t, 5), Ctx::default()), None, "每晚最多 2 次");
    }

    #[test]
    fn night_window_bounds() {
        assert!(in_night(23 * 60 + 30, 23 * 60 + 30));
        assert!(in_night(2 * 60, 23 * 60 + 30));
        assert!(!in_night(6 * 60, 23 * 60 + 30));
        assert!(!in_night(23 * 60, 23 * 60 + 30));
        assert!(in_night(30, 30));
        assert!(
            !in_night(23 * 60 + 50, 30),
            "从 00:30 开始时前一晚 23:50 不算"
        );
        for s in NIGHT_STARTS {
            assert!(parse_hhmm(s).is_some(), "{s}");
        }
        assert_eq!(parse_hhmm("23:30"), Some(1410));
        assert_eq!(parse_hhmm("abc"), None);
    }

    #[test]
    fn tired_brings_the_eye_reminder_forward() {
        // TC-RST-09：距上次护眼 12 分钟时状态变为 tired → 提前提醒，换文案
        let mut e = RestEngine::new(only(RestKind::Eye));
        let t = type_for(&mut e, at(10, 0), 12);
        assert_eq!(e.poll(idle(t, 5), Ctx::default()), None);
        e.set_tired(true);
        assert_eq!(
            e.poll(idle(t, 6), Ctx::default()),
            Some(Due {
                kind: RestKind::Eye,
                tired: true
            })
        );
        // 不到 10 分钟时疲劳也不提前
        let mut e = RestEngine::new(only(RestKind::Eye));
        e.set_tired(true);
        let t = type_for(&mut e, at(10, 0), 9);
        assert_eq!(e.poll(idle(t, 5), Ctx::default()), None);
    }

    #[test]
    fn system_idle_source_counts_usage_and_gates_on_idle() {
        // TC-RST-04：无痕时用系统空闲时间计时，护眼照常
        let mut e = RestEngine::new(only(RestKind::Eye));
        let mut t = at(10, 0);
        for _ in 0..(20 * 6) {
            e.on_system_idle(2_000, t);
            t += Duration::seconds(10);
        }
        let busy = Ctx {
            system_idle_ms: Some(1_000),
            ..Ctx::default()
        };
        assert_eq!(e.poll(t, busy), None, "还在操作");
        let quiet = Ctx {
            system_idle_ms: Some(6_000),
            ..Ctx::default()
        };
        assert!(e.poll(t, quiet).is_some());
    }

    #[test]
    fn disabled_kinds_never_fire() {
        let mut c = RestConfig {
            night_enabled: false,
            ..RestConfig::default()
        };
        c.eye.enabled = false;
        c.mv.enabled = false;
        c.water.enabled = false;
        let mut e = RestEngine::new(c);
        let t = type_for(&mut e, at(23, 0), 130);
        assert_eq!(e.poll(idle(t, 5), Ctx::default()), None);
    }

    #[test]
    fn tips_fit_the_bubble() {
        for k in RestKind::ALL {
            assert!(k.tip().chars().count() <= 16, "{k:?}");
        }
    }
}
