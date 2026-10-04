//! 心晴：温柔改写模式（A-08，产品书 06 FR-RWR-01～04、10 第 3.1 节、17 第 1.6 节）。
//!
//! 按 `xinqing.rewrite_hotkey`（出厂 Ctrl+Alt+R）进入：取“最近上屏”，没有就读剪贴板，
//! 发 `rewrite_req` 后立即返回，不等结果。Hub 的 `rewrite_result` / `rewrite_fail` 从 XQP
//! 读线程进来，在 `state` 锁内更新模式数据并重画候选框；那时模式已退出就丢掉。
//!
//! 模式数据放在 `State::xq_rewrite`（`Some` 即在模式里），不是 `ModeKind` 的变体：
//! `ModeKind` 的分派排在英文透传之后、按编码缓冲工作，而改写针对的是已上屏的文字，
//! 英文态下也要能用。清风另一个热键进入的模态（快捷加词）也是这样单独存的（ADR 0009 第 11 条）。
//!
//! 吃键约定（10 第 3.1 节）：模式期间 `hotkey_session` 为真，C++ 吃键并转发。按键表之外的
//! 键先退出模式、再走正常流程；正常流程若要把键交给宿主，改成
//! `ClearCompositionThenPassThrough` 由 C++ 重放，保证不丢键（NFR-REL-08，见 [`replay_if_exited`]）。

use std::cell::Cell;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use wind_bridge::handler::{KeyAction, KeyEventData};
use wind_config::hotkey;
use wind_ipc::protocol::{EVENT_KEY_DOWN, MOD_ALT, calc_key_hash, MOD_CTRL, MOD_SHIFT, MOD_WIN};
use wind_keys::keymap::{VK_1, VK_ESCAPE, VK_SPACE, VK_TAB};
use wind_ui_types::{CandidateItem, UiCommand};
use wind_xinqing_tap::{
    OpenTarget, RewriteFailReason, RewriteOutcome, RewriteReq, RewriteSource, RewriteStyle,
};

use super::{is_modifier_vk, retry_hub, tap};
use crate::coordinator::{Coordinator, State};

/// 等结果最多 10 秒（FR-RWR-03）。
const WAIT_LIMIT: Duration = Duration::from_secs(10);
/// 失败提示停留 1.5 秒后退出模式。
const FAIL_LINGER: Duration = Duration::from_millis(1500);
/// 每种风格的结果缓存 5 分钟（FR-RWR-02）。
const CACHE_TTL: Duration = Duration::from_secs(300);
/// 剪贴板来源上限（FR-RWR-01）。
const CLIP_MAX_CHARS: usize = 300;
/// 编码区显示原文前 20 个字（FR-RWR-03）。
const PREVIEW_CHARS: usize = 20;
/// 候选折行：每行字数与最多行数（FR-RWR-03“最多 3 行”）。候选框本身不折行，这里拆成几行。
const LINE_CHARS: usize = 18;
const MAX_LINES: usize = 3;
/// 气泡时长（FR-ENT-03 限定 1.5–2.5 秒）。
const TIP_MS: u64 = 2500;

const STYLES: [RewriteStyle; 4] = [
    RewriteStyle::Gentle,
    RewriteStyle::Polite,
    RewriteStyle::Concise,
    RewriteStyle::Structured,
];

static NEXT_REQ: AtomicU32 = AtomicU32::new(1);

thread_local! {
    /// 本次按键让改写模式退出了（见 [`replay_if_exited`]）。
    static EXITED: Cell<bool> = const { Cell::new(false) };
}

pub(crate) fn style_label(s: RewriteStyle) -> &'static str {
    match s {
        RewriteStyle::Gentle => "更温和",
        RewriteStyle::Polite => "更礼貌得体",
        RewriteStyle::Concise => "更简洁",
        RewriteStyle::Structured => "更有条理",
    }
}

fn next_style(s: RewriteStyle) -> RewriteStyle {
    let i = STYLES.iter().position(|x| *x == s).unwrap_or(0);
    STYLES[(i + 1) % STYLES.len()]
}

/// `rewrite_fail.reason` → 提示文字。危机内容不在候选框里说，单独处理。
fn fail_text(r: RewriteFailReason) -> &'static str {
    match r {
        RewriteFailReason::Timeout => "暂时改写不了，稍后再试",
        RewriteFailReason::Invalid => "这段话不太好改，换个说法试试？",
        RewriteFailReason::Budget => "今天的改写次数用完了",
        RewriteFailReason::NoConsent => "要先在心晴里同意“温柔改写”",
        RewriteFailReason::Offline => "晴晴暂时连不上网络，改写不了",
        RewriteFailReason::Crisis => CRISIS_TIP,
    }
}

/// FR-RWR-05 第 3 条：命中危机词表时不改写，改为邀请和晴晴聊聊。
const CRISIS_TIP: &str = "这段话里好像有很重的情绪，要不要和晴晴聊聊？";

#[derive(Debug, Clone, PartialEq, Eq)]
enum Phase {
    /// 等 Hub 的结果。
    Waiting,
    /// 显示 1–3 个候选。
    Showing(Vec<String>),
    /// 显示失败提示，[`FAIL_LINGER`] 后退出。
    Failed(&'static str),
}

/// 改写模式的数据（`State::xq_rewrite`）。原文只在内存里，退出即丢（FR-RWR-07）。
#[derive(Debug)]
pub(crate) struct RewriteMode {
    req_id: u32,
    source: RewriteSource,
    text: String,
    /// 仅“最近上屏”来源：要删掉的原文长度（UTF-16）。
    replace_len: Option<u32>,
    style: RewriteStyle,
    phase: Phase,
    cache: Vec<(RewriteStyle, Instant, Vec<String>)>,
}

impl RewriteMode {
    fn cached(&self, style: RewriteStyle) -> Option<Vec<String>> {
        self.cache
            .iter()
            .find(|(s, at, _)| *s == style && at.elapsed() < CACHE_TTL)
            .map(|(_, _, c)| c.clone())
    }
}

/// 把一段话按 [`LINE_CHARS`] 拆成最多 [`MAX_LINES`] 行，超出部分在最后一行末尾用省略号。
pub(crate) fn wrap_lines(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().filter(|c| *c != '\n' && *c != '\r').collect();
    let mut lines: Vec<String> = chars
        .chunks(LINE_CHARS)
        .take(MAX_LINES)
        .map(|c| c.iter().collect())
        .collect();
    if chars.len() > LINE_CHARS * MAX_LINES
        && let Some(last) = lines.last_mut()
    {
        last.pop();
        last.push('…');
    }
    lines
}

/// 每次按键开头调用的收尾：本次按键让改写模式退出、且正常流程要把键交给宿主时，改成
/// `ClearCompositionThenPassThrough`——这个键已经被 C++ 吃掉了，必须由它重放（NFR-REL-08）。
pub(crate) fn replay_if_exited(data: &KeyEventData, action: KeyAction) -> KeyAction {
    let exited = EXITED.with(|c| c.replace(false));
    if !exited || data.event_type != EVENT_KEY_DOWN {
        return action;
    }
    match action {
        KeyAction::PassThrough | KeyAction::NotHandled => {
            KeyAction::ClearCompositionThenPassThrough
        }
        other => other,
    }
}

impl Coordinator {
    /// 这一键是不是改写快捷键（按下）。按键钩子据此不报这一键，模式里再按一次也不退出。
    pub(crate) fn is_xinqing_rewrite_hotkey(&self, data: &KeyEventData) -> bool {
        if data.event_type != EVENT_KEY_DOWN || tap().is_none() {
            return false;
        }
        let mods = data.modifiers & hotkey::MOD_GENERIC_MASK;
        self.rt()
            .compiled_hotkeys
            .match_key_down(calc_key_hash(mods, data.key_code))
            .is_some_and(|a| a == hotkey::XINQING_REWRITE_ACTION)
    }

    /// 改写快捷键（FR-RWR-01）。没装心晴、总开关关着时返回 `None`，交给常规链路。
    pub(crate) fn xinqing_enter_rewrite(&self) -> Option<KeyAction> {
        let t = tap()?;
        if !self.rt().config.xinqing.enabled {
            return None;
        }
        {
            let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if s.xq_rewrite.is_some() {
                return Some(KeyAction::Consumed);
            }
        }
        if self.has_active_session() {
            self.show_xinqing_tip("先上屏再改写哦", TIP_MS);
            return Some(KeyAction::Consumed);
        }
        if !t.rewrite_enabled() {
            let tip = if !t.is_linked() {
                retry_hub();
                "心晴组件未运行，正在重新启动"
            } else if t.paused() {
                "已暂停感知，暂时不能改写"
            } else if !t.rewrite_consented() {
                // Hub 那边会问一次同意（10 第 2.5 节 cfg.rewrite）
                t.send_open(OpenTarget::Settings);
                "要先在心晴里同意“温柔改写”"
            } else {
                // 密码框、名单里的应用、安全桌面
                "这里不能使用温柔改写"
            };
            self.show_xinqing_tip(tip, TIP_MS);
            return Some(KeyAction::Consumed);
        }
        let (source, text, replace_len) = match t.take_recent() {
            Some(r) => (RewriteSource::Recent, r.text, Some(r.utf16_len)),
            None => {
                // 剪贴板读取最多阻塞几十毫秒，不在 state 锁里读
                let clip = self.host_services().clipboard_get_text().unwrap_or_default();
                let clip = clip.trim();
                if clip.is_empty() {
                    self.show_xinqing_tip("没有找到要改写的文字", TIP_MS);
                    return Some(KeyAction::Consumed);
                }
                let text: String = clip.chars().take(CLIP_MAX_CHARS).collect();
                (RewriteSource::Clipboard, text, None)
            }
        };
        let style = RewriteStyle::Gentle;
        let req_id = NEXT_REQ.fetch_add(1, Ordering::Relaxed);
        if !t.send_rewrite_req(RewriteReq {
            req_id,
            source,
            text: text.clone(),
            style,
            replace_len,
        }) {
            self.show_xinqing_tip(fail_text(RewriteFailReason::Timeout), TIP_MS);
            return Some(KeyAction::Consumed);
        }
        tracing::info!("心晴改写：进入，来源 {source:?}，{} 字", text.chars().count());
        {
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            s.xq_rewrite = Some(RewriteMode {
                req_id,
                source,
                text,
                replace_len,
                style,
                phase: Phase::Waiting,
                cache: Vec::new(),
            });
            self.show_rewrite_panel(&s);
        }
        self.arm_rewrite_timer(req_id, WAIT_LIMIT);
        Some(KeyAction::Consumed)
    }

    /// 改写模式里的按键（FR-RWR-03 按键表）。不在模式里、或这个键让模式退出并该走正常
    /// 流程时返回 `None`。
    pub(crate) fn xinqing_rewrite_key(&self, data: &KeyEventData) -> Option<KeyAction> {
        // 锁外要弹的气泡
        let mut tip = None;
        let action = {
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let rw = s.xq_rewrite.as_mut()?;
            let vk = data.key_code;
            // 单按修饰键：Ctrl+1 要先按 Ctrl，不能因此退出
            if is_modifier_vk(vk) {
                return None;
            }
            // 模式里再按一次快捷键：什么都不做（不能当“表外的键”退出后又被热键段重新进入）
            if self.is_xinqing_rewrite_hotkey(data) {
                return Some(KeyAction::Consumed);
            }
            let mods = data.modifiers & (MOD_CTRL | MOD_ALT | MOD_SHIFT | MOD_WIN);
            // 数字 1–3 选对应候选，空格选第 1 个
            let pick = if vk == VK_SPACE && mods == 0 {
                Some(0)
            } else {
                (VK_1..VK_1 + 3)
                    .contains(&vk)
                    .then(|| (vk - VK_1) as usize)
            };
            match (vk, mods, pick, &rw.phase) {
                (VK_ESCAPE, 0, _, _) => {
                    self.exit_rewrite(&mut s, None, RewriteOutcome::Cancelled);
                    Some(KeyAction::Consumed)
                }
                (VK_TAB, 0, _, _) => {
                    let style = next_style(rw.style);
                    rw.style = style;
                    let req = match rw.cached(style) {
                        Some(c) => {
                            rw.phase = Phase::Showing(c);
                            None
                        }
                        None => {
                            rw.req_id = NEXT_REQ.fetch_add(1, Ordering::Relaxed);
                            rw.phase = Phase::Waiting;
                            Some(RewriteReq {
                                req_id: rw.req_id,
                                source: rw.source,
                                text: rw.text.clone(),
                                style,
                                replace_len: rw.replace_len,
                            })
                        }
                    };
                    if let Some(req) = req {
                        let id = req.req_id;
                        if tap().is_some_and(|t| t.send_rewrite_req(req)) {
                            // 锁外起定时器不必：它只记下时刻，回调里再取锁
                            self.arm_rewrite_timer(id, WAIT_LIMIT);
                        } else if let Some(rw) = s.xq_rewrite.as_mut() {
                            rw.phase = Phase::Failed(fail_text(RewriteFailReason::Timeout));
                            self.arm_rewrite_timer(id, FAIL_LINGER);
                        }
                    }
                    self.show_rewrite_panel(&s);
                    Some(KeyAction::Consumed)
                }
                (_, 0 | MOD_CTRL, Some(i), Phase::Showing(cands)) => {
                    let Some(text) = cands.get(i).cloned() else {
                        // 候选不足 3 个时按了多出来的数字：不做什么
                        return Some(KeyAction::Consumed);
                    };
                    let copy = mods == MOD_CTRL;
                    let (action, outcome, done) = if copy {
                        let _ = self.ui_tx.send(UiCommand::CopyToClipboard(text));
                        (KeyAction::Consumed, RewriteOutcome::Copied, "已复制改写结果")
                    } else if let Some(count) = rw.replace_len {
                        (
                            KeyAction::ReplaceBackward { count, text },
                            RewriteOutcome::Replaced,
                            "已改写，Ctrl+Z 可撤销",
                        )
                    } else {
                        let chinese_mode = s.chinese_mode;
                        (
                            KeyAction::InsertText {
                                text,
                                new_composition: None,
                                mode_changed: false,
                                chinese_mode,
                                has_new_composition: false,
                            },
                            RewriteOutcome::Inserted,
                            "已插入改写结果",
                        )
                    };
                    self.exit_rewrite(&mut s, Some(i as u8), outcome);
                    tip = Some(done);
                    Some(action)
                }
                _ => {
                    // 表外的键：退出模式，键走正常流程（等待中按键也一样，不影响正常打字）
                    self.exit_rewrite(&mut s, None, RewriteOutcome::Cancelled);
                    EXITED.with(|c| c.set(true));
                    None
                }
            }
        };
        if let Some(tip) = tip {
            self.show_xinqing_tip(tip, TIP_MS);
        }
        action
    }

    /// 退出改写模式：清数据、收候选框、发 `rewrite_done`（不含文字）。`hotkey_session`
    /// 由按键出口统一推送；后台线程退出时调用方自己推。
    fn exit_rewrite(&self, s: &mut State, chosen: Option<u8>, outcome: RewriteOutcome) {
        if let Some(rw) = s.xq_rewrite.take() {
            if let Some(t) = tap() {
                t.send_rewrite_done(rw.req_id, chosen, outcome);
            }
            tracing::info!("心晴改写：退出（{outcome:?}）");
            self.notify_ui_hide();
        }
    }

    /// 焦点切换等清理路径：模式还在就当取消（`reset_exclusive_modes` 调）。
    pub(crate) fn xinqing_abort_rewrite(&self, s: &mut State) {
        self.exit_rewrite(s, None, RewriteOutcome::Cancelled);
    }

    /// Hub 的 `rewrite_result`（XQP 读线程）。不是当前请求的结果直接丢弃。
    pub(crate) fn xinqing_rewrite_result(&self, req_id: u32, style: RewriteStyle, cands: Vec<String>) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(rw) = s.xq_rewrite.as_mut() else {
            return;
        };
        let cands: Vec<String> = cands
            .into_iter()
            .filter(|c| !c.trim().is_empty())
            .take(3)
            .collect();
        rw.cache.retain(|(st, _, _)| *st != style);
        if !cands.is_empty() {
            rw.cache.push((style, Instant::now(), cands.clone()));
        }
        if rw.req_id != req_id || rw.phase != Phase::Waiting {
            return;
        }
        if cands.is_empty() {
            rw.phase = Phase::Failed(fail_text(RewriteFailReason::Invalid));
            self.show_rewrite_panel(&s);
            drop(s);
            self.arm_rewrite_timer(req_id, FAIL_LINGER);
            return;
        }
        rw.phase = Phase::Showing(cands);
        self.show_rewrite_panel(&s);
    }

    /// Hub 的 `rewrite_fail`（XQP 读线程）。危机内容直接退出并用气泡邀请聊聊；其余在
    /// 候选框里显示 1.5 秒后退出。
    pub(crate) fn xinqing_rewrite_fail(&self, req_id: u32, reason: RewriteFailReason) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(rw) = s.xq_rewrite.as_mut() else {
            return;
        };
        if rw.req_id != req_id {
            return;
        }
        if reason == RewriteFailReason::Crisis {
            self.exit_rewrite(&mut s, None, RewriteOutcome::Failed);
            drop(s);
            self.push_hotkey_session_if_changed();
            self.show_xinqing_tip(CRISIS_TIP, TIP_MS);
            return;
        }
        rw.phase = Phase::Failed(fail_text(reason));
        self.show_rewrite_panel(&s);
        drop(s);
        self.arm_rewrite_timer(req_id, FAIL_LINGER);
    }

    /// 等待超时（10 秒）→ 显示失败提示；失败提示停留 1.5 秒 → 退出。`req_id` 不对（换了
    /// 风格、模式已退出）就什么都不做。
    fn arm_rewrite_timer(&self, req_id: u32, after: Duration) {
        let Some(weak) = self.self_weak.get().cloned() else {
            return;
        };
        let spawned = std::thread::Builder::new()
            .name("xq-rewrite-timer".into())
            .spawn(move || {
                std::thread::sleep(after);
                if let Some(c) = weak.upgrade() {
                    c.rewrite_timer_fired(req_id);
                }
            });
        if let Err(e) = spawned {
            tracing::warn!("心晴改写：定时线程启动失败：{e}");
        }
    }

    fn rewrite_timer_fired(&self, req_id: u32) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(rw) = s.xq_rewrite.as_mut() else {
            return;
        };
        if rw.req_id != req_id {
            return;
        }
        match rw.phase {
            Phase::Waiting => {
                rw.phase = Phase::Failed(fail_text(RewriteFailReason::Timeout));
                self.show_rewrite_panel(&s);
                drop(s);
                self.arm_rewrite_timer(req_id, FAIL_LINGER);
            }
            Phase::Failed(_) => {
                self.exit_rewrite(&mut s, None, RewriteOutcome::Failed);
                drop(s);
                // 不是按键触发的退出，没有按键出口替它推 hotkey_session
                self.push_hotkey_session_if_changed();
            }
            Phase::Showing(_) => {}
        }
    }

    /// 候选框里的改写面板：标题行带“AI 改写”标识（FR-RWR-06），编码区灰显原文前 20 字，
    /// 下面是“改写中…”、1–3 个候选或失败提示。
    fn show_rewrite_panel(&self, s: &State) {
        let Some(rw) = s.xq_rewrite.as_ref() else {
            return;
        };
        let hint = |text: String, comment: &str| CandidateItem {
            text,
            code: String::new(),
            label: String::new(),
            tooltip: Default::default(),
            comment: comment.to_string().into(),
            comment_above: Default::default(),
            no_index: true,
        };
        let mut rows = vec![hint(
            format!("AI 改写 · {}", style_label(rw.style)),
            "Tab 换风格  Esc 取消",
        )];
        match &rw.phase {
            Phase::Waiting => rows.push(hint("✨ 改写中…".into(), "")),
            Phase::Failed(msg) => rows.push(hint((*msg).into(), "")),
            Phase::Showing(cands) => {
                for (i, c) in cands.iter().enumerate() {
                    for (j, line) in wrap_lines(c).into_iter().enumerate() {
                        let mut item = hint(line, "");
                        if j == 0 {
                            item.no_index = false;
                            item.label = (i + 1).to_string();
                        }
                        rows.push(item);
                    }
                }
                rows.push(hint(String::new(), "数字或空格上屏  Ctrl+数字只复制"));
            }
        }
        let preedit: String = rw.text.chars().take(PREVIEW_CHARS).collect();
        self.sync_candidate_layout(s);
        self.sync_candidate_font(s);
        let (fixed, fixed_x, fixed_y) = self.candidate_fixed_pos();
        let _ = self.ui_tx.send(UiCommand::UpdateCandidates {
            preedit_caret: preedit.len(),
            preedit,
            preedit_host_owned: false,
            mode_label: String::new(),
            candidates: rows,
            selected: usize::MAX,
            hover: -1,
            page: 1,
            total_pages: 1,
            caret_x: s.caret_x,
            caret_y: s.caret_y,
            caret_height: s.caret_height,
            caret_valid: true,
            fixed,
            fixed_x,
            fixed_y,
        });
    }
}

/// 测试与诊断：当前改写面板的阶段文字（`None` = 不在模式里）。
pub(crate) fn debug_phase(s: &State) -> Option<String> {
    s.xq_rewrite.as_ref().map(|rw| match &rw.phase {
        Phase::Waiting => format!("waiting:{}", style_label(rw.style)),
        Phase::Showing(c) => format!("showing:{}:{}", style_label(rw.style), c.join("|")),
        Phase::Failed(m) => format!("failed:{m}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_long_candidates_into_three_lines() {
        assert_eq!(wrap_lines("短句"), vec!["短句".to_string()]);
        let long = "字".repeat(LINE_CHARS * 2 + 1);
        assert_eq!(wrap_lines(&long).len(), 3);
        let too_long = "字".repeat(LINE_CHARS * MAX_LINES + 5);
        let lines = wrap_lines(&too_long);
        assert_eq!(lines.len(), MAX_LINES);
        assert!(lines[2].ends_with('…'));
        assert_eq!(lines[2].chars().count(), LINE_CHARS);
    }

    #[test]
    fn styles_cycle_through_all_four() {
        let mut s = RewriteStyle::Gentle;
        let mut seen = vec![s];
        for _ in 0..3 {
            s = next_style(s);
            seen.push(s);
        }
        assert_eq!(seen, STYLES.to_vec());
        assert_eq!(next_style(RewriteStyle::Structured), RewriteStyle::Gentle);
    }

}
