//! 单独同意项（09 第 2.1 节、07 FR-ONB-04）。
//!
//! 每一项独立记录、默认不同意；隐私说明版本升级后全部视为未同意，需要重新征得同意（FR-ONB-01）。
//! Hub 是同意状态的唯一真相源，连接输入法时经 XQP `cfg` 下发（10 第 6.1 节）。

use serde::{Deserialize, Serialize};

use crate::infra::store::{Db, StoreError};

/// 当前隐私说明版本（21 第 1 节，`policy_ver = 1`）。正文改动时 +1。
pub const POLICY_VER: i64 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum ConsentItem {
    /// ① 感知打字节奏，在本机识别状态（使用心晴的前提）
    Sense,
    /// ② 数值特征发送给第三方 AI 服务做状态判断
    JevFeatures,
    /// ③ 日程与待办识别（发送单句文字）
    Schedule,
    /// ④ 状态摘要发给大模型（暖心话、对话、周信）
    LlmSummary,
    /// ⑤ 增强模式（发送上屏文字判断情绪）
    Enhanced,
    /// ⑥ 温柔改写（按快捷键时发送那段文字）
    Rewrite,
}

impl ConsentItem {
    pub const ALL: [ConsentItem; 6] = [
        ConsentItem::Sense,
        ConsentItem::JevFeatures,
        ConsentItem::Schedule,
        ConsentItem::LlmSummary,
        ConsentItem::Enhanced,
        ConsentItem::Rewrite,
    ];

    /// 写入 `consent.item` 的字符串，已发布后不得改名。
    pub fn as_str(self) -> &'static str {
        match self {
            ConsentItem::Sense => "sense",
            ConsentItem::JevFeatures => "jev_features",
            ConsentItem::Schedule => "schedule",
            ConsentItem::LlmSummary => "llm_summary",
            ConsentItem::Enhanced => "enhanced",
            ConsentItem::Rewrite => "rewrite",
        }
    }
}

/// `consent_get` 的返回值：每一项在当前隐私说明版本下是否已同意。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ConsentState {
    pub policy_ver: u32,
    pub items: Vec<ConsentEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ConsentEntry {
    pub item: ConsentItem,
    pub granted: bool,
}

impl ConsentState {
    pub fn load(db: &Db) -> Result<Self, StoreError> {
        let items = ConsentItem::ALL
            .iter()
            .map(|&item| {
                Ok(ConsentEntry {
                    item,
                    granted: db.consent_granted(item.as_str(), POLICY_VER)?,
                })
            })
            .collect::<Result<Vec<_>, StoreError>>()?;
        Ok(Self {
            policy_ver: POLICY_VER as u32,
            items,
        })
    }

    pub fn granted(&self, item: ConsentItem) -> bool {
        self.items.iter().any(|e| e.item == item && e.granted)
    }

    /// 需要打开首次引导：首次运行，或隐私说明升级后“感知”一项不再有效（FR-ONB-01）。
    pub fn needs_onboarding(&self) -> bool {
        !self.granted(ConsentItem::Sense)
    }
}

/// 记录一次同意或撤回。撤回“感知”时，依赖它的其余各项一并视为撤回（09 第 2.1 节“撤回后”列）。
pub fn set(db: &Db, item: ConsentItem, granted: bool, ts: i64) -> Result<(), StoreError> {
    db.consent_set(item.as_str(), POLICY_VER, granted, ts)?;
    if item == ConsentItem::Sense && !granted {
        for other in ConsentItem::ALL
            .into_iter()
            .filter(|&i| i != ConsentItem::Sense)
        {
            if db.consent_granted(other.as_str(), POLICY_VER)? {
                db.consent_set(other.as_str(), POLICY_VER, false, ts)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_granted_by_default() {
        let db = Db::open_in_memory().unwrap();
        let s = ConsentState::load(&db).unwrap();
        assert_eq!(s.items.len(), 6);
        assert!(s.items.iter().all(|e| !e.granted));
        assert!(s.needs_onboarding());
    }

    #[test]
    fn grant_and_withdraw() {
        let db = Db::open_in_memory().unwrap();
        set(&db, ConsentItem::Sense, true, 1).unwrap();
        set(&db, ConsentItem::JevFeatures, true, 2).unwrap();
        let s = ConsentState::load(&db).unwrap();
        assert!(s.granted(ConsentItem::Sense) && s.granted(ConsentItem::JevFeatures));
        assert!(!s.granted(ConsentItem::Schedule));
        assert!(!s.needs_onboarding());

        set(&db, ConsentItem::JevFeatures, false, 3).unwrap();
        assert!(!ConsentState::load(&db)
            .unwrap()
            .granted(ConsentItem::JevFeatures));
    }

    #[test]
    fn withdrawing_sense_withdraws_everything() {
        let db = Db::open_in_memory().unwrap();
        for (i, item) in ConsentItem::ALL.into_iter().enumerate() {
            set(&db, item, true, i as i64).unwrap();
        }
        set(&db, ConsentItem::Sense, false, 100).unwrap();
        let s = ConsentState::load(&db).unwrap();
        assert!(s.items.iter().all(|e| !e.granted));
    }

    #[test]
    fn old_policy_version_does_not_count() {
        let db = Db::open_in_memory().unwrap();
        db.consent_set("sense", POLICY_VER - 1, true, 1).unwrap();
        assert!(ConsentState::load(&db).unwrap().needs_onboarding());
    }
}
