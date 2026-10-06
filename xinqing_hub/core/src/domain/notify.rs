//! 系统通知的文案（07 FR-NTF-01、16 D-09）。
//!
//! 系统通知只用在三处：日程提醒、错过的提醒、小组件隐藏时的休息提醒；暖心话、情绪状态、求助卡片**不走**系统通知
//! （DS-COPY-08）。文案来自 `ui_copy.toml` 的 `[notify]` 与 `[rest]`，和小组件卡片同一份，界面与通知说法一致。
//! 目前接上的是休息提醒；日程提醒、错过的提醒等日程模块的事件到位后加在这里。

use serde::Deserialize;

use crate::domain::rest::RestKind;
use crate::infra::templates::{TemplateDirs, TemplateError, read_toml};

/// 一条要弹的系统通知。按钮的 `action` 是点了之后交回的参数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub title: String,
    pub body: String,
    pub buttons: Vec<NoticeButton>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoticeButton {
    pub label: String,
    pub action: &'static str,
}

/// 休息提醒通知上“知道了”交回的参数：等同卡片上的“已完成”（该类计时清零，FR-RST-06 第 2 条）
pub const ACTION_OK: &str = "done";
/// “5 分钟后”交回的参数：等同卡片上的“5 分钟后”
pub const ACTION_SNOOZE: &str = "later";

/// 系统通知用到的文案。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotifyCopy {
    rest_title: String,
    rest_body: String,
    btn_ok: String,
    btn_snooze: String,
    eye: String,
    eye_tired: String,
    water: String,
    mv: String,
    night: String,
}

#[derive(Debug, Deserialize)]
struct RawCopy {
    notify: RawNotify,
    rest: RawRest,
}

#[derive(Debug, Deserialize)]
struct RawNotify {
    rest_title: String,
    rest_body: String,
    btn_ok: String,
    btn_snooze: String,
}

#[derive(Debug, Deserialize)]
struct RawRest {
    eye: String,
    eye_tired: String,
    water: String,
    #[serde(rename = "move")]
    mv: String,
    night: String,
}

impl NotifyCopy {
    /// 只读出厂版本，和小组件卡片（前端由同一文件生成）保持一致。
    pub fn load(dirs: &TemplateDirs) -> Result<Self, TemplateError> {
        let raw: RawCopy = read_toml(&dirs.factory_path("ui_copy.toml"))?;
        Ok(Self {
            rest_title: raw.notify.rest_title,
            rest_body: raw.notify.rest_body,
            btn_ok: raw.notify.btn_ok,
            btn_snooze: raw.notify.btn_snooze,
            eye: raw.rest.eye,
            eye_tired: raw.rest.eye_tired,
            water: raw.rest.water,
            mv: raw.rest.mv,
            night: raw.rest.night,
        })
    }

    /// 小组件隐藏时的休息提醒（FR-NTF-01）：标题“休息一下”，正文是卡片上的那句话，按钮“知道了”/“5 分钟后”。
    /// `tired` 时护眼换成“打了很久啦，眼睛也累了吧”（FR-RST-07），与卡片相同。
    pub fn rest(&self, kind: RestKind, tired: bool) -> Notice {
        let text = match kind {
            RestKind::Eye if tired => &self.eye_tired,
            RestKind::Eye => &self.eye,
            RestKind::Water => &self.water,
            RestKind::Move => &self.mv,
            RestKind::Night => &self.night,
        };
        Notice {
            title: self.rest_title.clone(),
            body: self.rest_body.replace("{text}", text),
            buttons: vec![
                NoticeButton {
                    label: self.btn_ok.clone(),
                    action: ACTION_OK,
                },
                NoticeButton {
                    label: self.btn_snooze.clone(),
                    action: ACTION_SNOOZE,
                },
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::domain::rest::RestAction;
    use crate::domain::validate::{BannedWords, Scene};

    fn dirs() -> TemplateDirs {
        TemplateDirs::factory_only(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../hub_templates"),
        )
    }

    #[test]
    fn rest_notice_uses_card_copy_and_two_buttons() {
        let copy = NotifyCopy::load(&dirs()).unwrap();
        let n = copy.rest(RestKind::Eye, false);
        assert_eq!(n.title, "休息一下");
        assert_eq!(n.body, "看看 6 米外的地方，休息 20 秒");
        assert_eq!(
            n.buttons
                .iter()
                .map(|b| (b.label.as_str(), b.action))
                .collect::<Vec<_>>(),
            [("知道了", "done"), ("5 分钟后", "later")]
        );
        assert_eq!(
            copy.rest(RestKind::Eye, true).body,
            "打了很久啦，眼睛也累了吧"
        );
        assert_eq!(copy.rest(RestKind::Move, false).body, "站起来走两步吧");
        // tired 只影响护眼
        assert_eq!(
            copy.rest(RestKind::Water, true).body,
            copy.rest(RestKind::Water, false).body
        );
    }

    #[test]
    fn button_actions_are_rest_actions() {
        // 通知按钮交回的参数必须能直接当作 rest_action 的取值
        for a in [ACTION_OK, ACTION_SNOOZE] {
            let parsed: RestAction = serde_json::from_value(serde_json::json!(a)).unwrap();
            assert_eq!(parsed.as_str(), a);
        }
    }

    #[test]
    fn rest_notices_pass_banned_words() {
        // DS-COPY-08：系统通知不出现情绪状态词；禁用词表同时管住诊断、监控等说法
        let banned = BannedWords::load(&dirs()).unwrap();
        let copy = NotifyCopy::load(&dirs()).unwrap();
        for kind in RestKind::ALL {
            for tired in [false, true] {
                let n = copy.rest(kind, tired);
                for text in [&n.title, &n.body] {
                    assert!(banned.find(text, Scene::Other).is_none(), "{text}");
                }
            }
        }
    }
}
