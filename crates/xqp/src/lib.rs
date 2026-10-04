//! 心晴 XQP 协议 v1（产品书 10 第 2 节）。
//!
//! - 消息类型与 `protocol/xqp.schema.json` 一一对应，字段名保持一致；
//! - 帧格式：`u32` 大端长度（不含这 4 字节）+ UTF-8 JSON，单帧上限 64 KiB；
//! - 接口变更只允许新增可选字段或新增消息类型（10 开头的变更规则）。

use std::io::{self, Read, Write};

use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;
/// 单帧上限（不含长度前缀）。
pub const MAX_FRAME: usize = 64 * 1024;

/// 正式版管道名（10 第 2.1 节）。
pub const PIPE_NAME: &str = "xinqing_tap";
/// dev 构建管道名。
pub const PIPE_NAME_DEV: &str = "xinqing_tap_dev";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyKind {
    Letter,
    Digit,
    Space,
    Enter,
    Backspace,
    Delete,
    Punct,
    Nav,
    Esc,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeySrc {
    Core,
    Tsf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompOp {
    Update,
    Cancel,
    Clear,
    Terminated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandOp {
    Page,
    Select,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Normal,
    Password,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PauseBy {
    Menu,
    Hotkey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenTarget {
    Chat,
    Dashboard,
    Schedule,
    Settings,
    Onboarding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RewriteSource {
    Recent,
    Clipboard,
    Selection,
}

/// FR-RWR-02 的四种风格。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RewriteStyle {
    Gentle,
    Polite,
    Concise,
    Structured,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RewriteOutcome {
    Replaced,
    Inserted,
    Copied,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RewriteFailReason {
    Timeout,
    Invalid,
    Budget,
    Crisis,
    NoConsent,
    Offline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ByeReason {
    Version,
    Disabled,
    Shutdown,
}

/// 显示状态（04 第 3.1 节；`typo` 是瞬时事件，不作为显示状态下发）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum MoodState {
    Fluent,
    Hesitant,
    Low,
    Agitated,
    Tired,
    Unknown,
}

impl MoodState {
    pub fn as_str(self) -> &'static str {
        match self {
            MoodState::Fluent => "fluent",
            MoodState::Hesitant => "hesitant",
            MoodState::Low => "low",
            MoodState::Agitated => "agitated",
            MoodState::Tired => "tired",
            MoodState::Unknown => "unknown",
        }
    }
}

/// 上行消息（核心 → Hub）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Up {
    Hello {
        v: u32,
        ime_ver: String,
        session: String,
        caps: Vec<String>,
    },
    Key {
        ts: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        seq: Option<u32>,
        kind: KeyKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        vk: Option<u8>,
        in_comp: bool,
        src: KeySrc,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        eaten: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        del_committed: Option<bool>,
    },
    KeyUp {
        ts: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        seq: Option<u32>,
        kind: KeyKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        vk: Option<u8>,
    },
    Comp {
        ts: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        seq: Option<u32>,
        op: CompOp,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        len: Option<u16>,
    },
    Cand {
        ts: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        seq: Option<u32>,
        op: CandOp,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pos: Option<u32>,
    },
    Commit {
        ts: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        seq: Option<u32>,
        chars: u32,
        keystrokes: u32,
        cand_pos: i32,
        src: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        truncated: Option<bool>,
    },
    Focus {
        ts: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        seq: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        app: Option<String>,
        scope: Scope,
        blocked: bool,
    },
    Ime {
        ts: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        seq: Option<u32>,
        active: bool,
        chinese: bool,
    },
    PauseChanged {
        ts: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        seq: Option<u32>,
        on: bool,
        by: PauseBy,
    },
    Open {
        ts: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        seq: Option<u32>,
        target: OpenTarget,
    },
    Hb {
        ts: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        seq: Option<u32>,
        dropped: u32,
        queue: u32,
    },
    RewriteReq {
        ts: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        seq: Option<u32>,
        req_id: u32,
        source: RewriteSource,
        text: String,
        style: RewriteStyle,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        replace_len: Option<u32>,
    },
    RewriteDone {
        ts: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        seq: Option<u32>,
        req_id: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        chosen: Option<u8>,
        outcome: RewriteOutcome,
    },
    Bye {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ts: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        seq: Option<u32>,
        reason: ByeReason,
    },
}

impl Up {
    /// 会话时间（毫秒）；`hello` 没有时间戳。
    pub fn ts(&self) -> Option<u64> {
        match self {
            Up::Hello { .. } => None,
            Up::Bye { ts, .. } => *ts,
            Up::Key { ts, .. }
            | Up::KeyUp { ts, .. }
            | Up::Comp { ts, .. }
            | Up::Cand { ts, .. }
            | Up::Commit { ts, .. }
            | Up::Focus { ts, .. }
            | Up::Ime { ts, .. }
            | Up::PauseChanged { ts, .. }
            | Up::Open { ts, .. }
            | Up::Hb { ts, .. }
            | Up::RewriteReq { ts, .. }
            | Up::RewriteDone { ts, .. } => Some(*ts),
        }
    }

    /// 设置上行序号（`hello` 无序号，调用无效果）。
    pub fn set_seq(&mut self, value: u32) {
        match self {
            Up::Hello { .. } => {}
            Up::Key { seq, .. }
            | Up::KeyUp { seq, .. }
            | Up::Comp { seq, .. }
            | Up::Cand { seq, .. }
            | Up::Commit { seq, .. }
            | Up::Focus { seq, .. }
            | Up::Ime { seq, .. }
            | Up::PauseChanged { seq, .. }
            | Up::Open { seq, .. }
            | Up::Hb { seq, .. }
            | Up::RewriteReq { seq, .. }
            | Up::RewriteDone { seq, .. }
            | Up::Bye { seq, .. } => *seq = Some(value),
        }
    }

    /// 平移会话时间（xq-sim 的 `--start-at` 等场景使用）。
    pub fn set_ts(&mut self, value: u64) {
        match self {
            Up::Hello { .. } => {}
            Up::Bye { ts, .. } => *ts = Some(value),
            Up::Key { ts, .. }
            | Up::KeyUp { ts, .. }
            | Up::Comp { ts, .. }
            | Up::Cand { ts, .. }
            | Up::Commit { ts, .. }
            | Up::Focus { ts, .. }
            | Up::Ime { ts, .. }
            | Up::PauseChanged { ts, .. }
            | Up::Open { ts, .. }
            | Up::Hb { ts, .. }
            | Up::RewriteReq { ts, .. }
            | Up::RewriteDone { ts, .. } => *ts = value,
        }
    }

    /// 去掉所有文字字段（FR-DMO-02 会话录制自动删除 `text`）。
    pub fn strip_text(&mut self) {
        match self {
            Up::Commit {
                text, truncated, ..
            } => {
                *text = None;
                *truncated = None;
            }
            Up::RewriteReq { text, .. } => text.clear(),
            _ => {}
        }
    }
}

/// 下行消息（Hub → 核心）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Down {
    Hello {
        v: u32,
        hub_ver: String,
    },
    Cfg {
        collect: bool,
        send_text: bool,
        rewrite: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        app_blocklist: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        app_allowlist: Option<Vec<String>>,
    },
    Pause {
        on: bool,
    },
    Mood {
        state: MoodState,
        offline: bool,
    },
    Badge {
        on: bool,
    },
    Tip {
        text: String,
        ms: u32,
    },
    Pending {
        count: u32,
    },
    RewriteResult {
        req_id: u32,
        style: RewriteStyle,
        cands: Vec<String>,
    },
    RewriteFail {
        req_id: u32,
        reason: RewriteFailReason,
    },
    Bye {
        reason: ByeReason,
    },
}

impl Down {
    /// 发送前校验 10 第 2.5 节的取值约束（tip ≤ 16 字、ms ∈ [1500, 2500]、候选 1–3 个）。
    pub fn validate(&self) -> Result<(), FrameError> {
        match self {
            Down::Tip { text, ms } if text.chars().count() > 16 || !(1500..=2500).contains(ms) => {
                Err(FrameError::Invalid("tip"))
            }
            Down::RewriteResult { cands, .. } if cands.is_empty() || cands.len() > 3 => {
                Err(FrameError::Invalid("rewrite_result"))
            }
            _ => Ok(()),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("帧长度 {0} 超过 64 KiB 上限")]
    TooLarge(usize),
    #[error("JSON 无法解析：{0}")]
    Json(#[from] serde_json::Error),
    #[error("IO：{0}")]
    Io(#[from] io::Error),
    #[error("字段取值不合法：{0}")]
    Invalid(&'static str),
}

/// 编码一帧：4 字节大端长度 + JSON。
pub fn encode<T: Serialize>(msg: &T) -> Result<Vec<u8>, FrameError> {
    let body = serde_json::to_vec(msg)?;
    if body.len() > MAX_FRAME {
        return Err(FrameError::TooLarge(body.len()));
    }
    let mut out = Vec::with_capacity(4 + body.len());
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

pub fn write_frame<W: Write, T: Serialize>(w: &mut W, msg: &T) -> Result<(), FrameError> {
    w.write_all(&encode(msg)?)?;
    Ok(())
}

/// 解析长度前缀；超限时调用方必须关闭连接（10 第 2.1 节“非法帧”）。
pub fn frame_len(prefix: [u8; 4]) -> Result<usize, FrameError> {
    let len = u32::from_be_bytes(prefix) as usize;
    if len > MAX_FRAME {
        return Err(FrameError::TooLarge(len));
    }
    Ok(len)
}

/// 读取一帧原始 JSON；对端正常关闭（读到 EOF 且没有半帧）时返回 `Ok(None)`。
pub fn read_frame<R: Read>(r: &mut R) -> Result<Option<Vec<u8>>, FrameError> {
    let mut prefix = [0u8; 4];
    match r.read_exact(&mut prefix) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let len = frame_len(prefix)?;
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    Ok(Some(body))
}

pub fn decode_up(body: &[u8]) -> Result<Up, FrameError> {
    Ok(serde_json::from_slice(body)?)
}

pub fn decode_down(body: &[u8]) -> Result<Down, FrameError> {
    Ok(serde_json::from_slice(body)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_session_from_spec_round_trips() {
        // 10 第 2.6 节示例会话
        let lines = [
            r#"{"t":"hello","v":1,"ime_ver":"1.0.0","session":"9f1c","caps":["core_keys"]}"#,
            r#"{"t":"focus","ts":1203,"seq":1,"app":"WeChat.exe","scope":"normal","blocked":false}"#,
            r#"{"t":"key","ts":1850,"seq":2,"kind":"letter","vk":72,"in_comp":false,"src":"core"}"#,
            r#"{"t":"comp","ts":1851,"seq":3,"op":"update","len":1}"#,
            r#"{"t":"commit","ts":9420,"seq":41,"chars":16,"keystrokes":31,"cand_pos":0,"src":"candidate","text":"好的，周五下午三点在实验楼开组会"}"#,
            r#"{"t":"hb","ts":10000,"seq":42,"dropped":0,"queue":0}"#,
        ];
        for line in lines {
            let up: Up = serde_json::from_str(line).unwrap();
            let back = serde_json::to_string(&up).unwrap();
            let again: Up = serde_json::from_str(&back).unwrap();
            assert_eq!(up, again);
        }
        let downs = [
            r#"{"t":"hello","v":1,"hub_ver":"1.0.0"}"#,
            r#"{"t":"cfg","collect":true,"send_text":true,"rewrite":false}"#,
            r#"{"t":"tip","text":"📅 识别到日程","ms":1500}"#,
        ];
        for line in downs {
            let d: Down = serde_json::from_str(line).unwrap();
            d.validate().unwrap();
        }
    }

    #[test]
    fn frame_round_trip_and_eof() {
        let msg = Up::Hb {
            ts: 5000,
            seq: Some(7),
            dropped: 3,
            queue: 0,
        };
        let mut buf = encode(&msg).unwrap();
        buf.extend(encode(&msg).unwrap());
        let mut cur = io::Cursor::new(buf);
        for _ in 0..2 {
            let body = read_frame(&mut cur).unwrap().unwrap();
            assert_eq!(decode_up(&body).unwrap(), msg);
        }
        assert!(read_frame(&mut cur).unwrap().is_none());
    }

    #[test]
    fn oversized_frame_is_rejected() {
        let prefix = ((MAX_FRAME + 1) as u32).to_be_bytes();
        assert!(matches!(frame_len(prefix), Err(FrameError::TooLarge(_))));
    }

    #[test]
    fn strip_text_removes_commit_text() {
        let mut m: Up = serde_json::from_str(
            r#"{"t":"commit","ts":1,"chars":2,"keystrokes":4,"cand_pos":0,"src":"candidate","text":"你好"}"#,
        )
        .unwrap();
        m.strip_text();
        assert!(!serde_json::to_string(&m).unwrap().contains("text"));
    }

    #[test]
    fn tip_limits_are_enforced() {
        let long = Down::Tip {
            text: "一二三四五六七八九十一二三四五六七".into(),
            ms: 1500,
        };
        assert!(long.validate().is_err());
        let fast = Down::Tip {
            text: "好".into(),
            ms: 800,
        };
        assert!(fast.validate().is_err());
    }
}
