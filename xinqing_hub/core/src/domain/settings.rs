//! Hub 设置（10 第 6.2 节、03 第 6 节）：键注册表、默认值与取值范围。
//!
//! 值以 JSON 文本存放在 `settings` 表。读取时缺失或已不合法的值一律回落默认值，
//! 写入时先校验，不合法的值不落库。新增键必须同时写明默认值和取值范围（10 第 6.2 节）。

use serde::{Deserialize, Serialize};

use crate::infra::store::{Db, StoreError};

/// 设置项的值。只有这三种形态，对应 [`Kind`]；以 JSON 文本落库（`true` / `0.8` / `"system"`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(untagged)]
pub enum SettingValue {
    Bool(bool),
    Number(f64),
    Text(String),
}

impl SettingValue {
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            SettingValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            SettingValue::Text(s) => Some(s),
            _ => None,
        }
    }
}

impl From<bool> for SettingValue {
    fn from(b: bool) -> Self {
        SettingValue::Bool(b)
    }
}

impl From<f64> for SettingValue {
    fn from(x: f64) -> Self {
        SettingValue::Number(x)
    }
}

impl From<&str> for SettingValue {
    fn from(s: &str) -> Self {
        SettingValue::Text(s.into())
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Kind {
    Bool,
    /// 闭区间
    Number {
        min: f64,
        max: f64,
    },
    /// 枚举字符串
    Choice(&'static [&'static str]),
}

#[derive(Debug, Clone, Copy)]
pub struct KeySpec {
    pub key: &'static str,
    pub kind: Kind,
    pub default: fn() -> SettingValue,
}

/// 已登记的键。外壳只读写这里列出的键，其余一律拒绝。
pub const KEYS: &[KeySpec] = &[
    KeySpec {
        key: "ui.theme",
        kind: Kind::Choice(&["system", "light", "dark"]),
        default: || "system".into(),
    },
    KeySpec {
        key: "widget.visible",
        kind: Kind::Bool,
        default: || true.into(),
    },
    KeySpec {
        key: "widget.opacity",
        kind: Kind::Number { min: 0.7, max: 1.0 },
        default: || 1.0.into(),
    },
    KeySpec {
        key: "widget.topmost",
        kind: Kind::Bool,
        default: || true.into(),
    },
    KeySpec {
        key: "widget.autohide",
        kind: Kind::Bool,
        default: || false.into(),
    },
    // 主动关怀频率（FR-CMF-06）：多一些 / 适中 / 少一些 / 关闭
    KeySpec {
        key: "care.level",
        kind: Kind::Choice(&["more", "normal", "less", "off"]),
        default: || "normal".into(),
    },
    KeySpec {
        key: "care.style",
        kind: Kind::Choice(&["gentle", "lively", "brief"]),
        default: || "gentle".into(),
    },
    KeySpec {
        key: "dev.mode",
        kind: Kind::Bool,
        default: || false.into(),
    },
];

/// 借用 `settings` 表存放的内部状态键：不是用户设置，不在 [`KEYS`] 里，`settings_get` / `settings_set`
/// 命令读写不到；导出 / 导入设置时只处理 [`KEYS`]，不带这些键（换一台电脑导入时不该带过来）。
pub const INTERNAL_KEYS: &[&str] = &[
    // 上次“重置基线”的时间（Unix 毫秒，B-04，ADR 0014）
    crate::domain::features::persist::RESET_KEY,
];

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("未登记的设置项：{0}")]
    UnknownKey(String),
    #[error("设置项 {key} 的值不在取值范围内")]
    OutOfRange { key: String },
    #[error(transparent)]
    Store(#[from] StoreError),
}

pub fn spec(key: &str) -> Option<&'static KeySpec> {
    KEYS.iter().find(|s| s.key == key)
}

impl KeySpec {
    pub fn accepts(&self, v: &SettingValue) -> bool {
        match (self.kind, v) {
            (Kind::Bool, SettingValue::Bool(_)) => true,
            (Kind::Number { min, max }, SettingValue::Number(x)) => (min..=max).contains(x),
            (Kind::Choice(opts), SettingValue::Text(s)) => opts.contains(&s.as_str()),
            _ => false,
        }
    }
}

pub fn get(db: &Db, key: &str) -> Result<SettingValue, SettingsError> {
    let spec = spec(key).ok_or_else(|| SettingsError::UnknownKey(key.into()))?;
    let stored = db
        .settings_get(key)?
        .and_then(|s| serde_json::from_str::<SettingValue>(&s).ok())
        .filter(|v| spec.accepts(v));
    Ok(stored.unwrap_or_else(spec.default))
}

/// 写入一个设置项。返回值是否真的改变（决定是否推送 `settings:changed`）。
pub fn set(db: &Db, key: &str, value: &SettingValue) -> Result<bool, SettingsError> {
    let spec = spec(key).ok_or_else(|| SettingsError::UnknownKey(key.into()))?;
    if !spec.accepts(value) {
        return Err(SettingsError::OutOfRange { key: key.into() });
    }
    if get(db, key)? == *value {
        return Ok(false);
    }
    let text = serde_json::to_string(value).expect("设置值总能序列化");
    db.settings_set(key, &text)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> SettingValue {
        SettingValue::Text(s.into())
    }

    #[test]
    fn every_default_is_in_range() {
        for s in KEYS {
            assert!(
                s.accepts(&(s.default)()),
                "{} 的默认值不在取值范围内",
                s.key
            );
        }
    }

    #[test]
    fn keys_are_unique_and_namespaced() {
        let mut seen = std::collections::HashSet::new();
        for s in KEYS {
            assert!(seen.insert(s.key), "重复的键 {}", s.key);
            assert!(s.key.contains('.'), "{} 不是“分类.名称”格式", s.key);
        }
    }

    #[test]
    fn internal_keys_are_not_user_settings() {
        for k in INTERNAL_KEYS {
            assert!(spec(k).is_none(), "{k} 不应出现在 KEYS 里");
            assert!(k.contains('.'), "{k} 要带命名空间");
        }
        let db = Db::open_in_memory().unwrap();
        assert!(matches!(
            get(&db, INTERNAL_KEYS[0]),
            Err(SettingsError::UnknownKey(_))
        ));
    }

    #[test]
    fn missing_value_falls_back_to_default() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(
            get(&db, "widget.opacity").unwrap(),
            SettingValue::Number(1.0)
        );
        assert_eq!(get(&db, "ui.theme").unwrap(), text("system"));
    }

    #[test]
    fn set_validates_and_reports_changes() {
        let db = Db::open_in_memory().unwrap();
        assert!(set(&db, "widget.opacity", &SettingValue::Number(0.8)).unwrap());
        assert!(!set(&db, "widget.opacity", &SettingValue::Number(0.8)).unwrap());
        assert_eq!(
            get(&db, "widget.opacity").unwrap(),
            SettingValue::Number(0.8)
        );
        assert!(matches!(
            set(&db, "widget.opacity", &SettingValue::Number(0.5)),
            Err(SettingsError::OutOfRange { .. })
        ));
        assert!(matches!(
            set(&db, "ui.theme", &text("blue")),
            Err(SettingsError::OutOfRange { .. })
        ));
        assert!(matches!(
            set(&db, "no.such", &SettingValue::Bool(true)),
            Err(SettingsError::UnknownKey(_))
        ));
    }

    #[test]
    fn stored_as_plain_json() {
        let db = Db::open_in_memory().unwrap();
        set(&db, "ui.theme", &text("dark")).unwrap();
        set(&db, "widget.topmost", &SettingValue::Bool(false)).unwrap();
        assert_eq!(
            db.settings_get("ui.theme").unwrap().as_deref(),
            Some("\"dark\"")
        );
        assert_eq!(
            db.settings_get("widget.topmost").unwrap().as_deref(),
            Some("false")
        );
        db.settings_set("widget.opacity", "1").unwrap();
        assert_eq!(
            get(&db, "widget.opacity").unwrap(),
            SettingValue::Number(1.0)
        );
    }

    #[test]
    fn corrupted_stored_value_falls_back_to_default() {
        let db = Db::open_in_memory().unwrap();
        db.settings_set("widget.visible", "\"yes\"").unwrap();
        assert_eq!(
            get(&db, "widget.visible").unwrap(),
            SettingValue::Bool(true)
        );
        db.settings_set("care.style", "not json").unwrap();
        assert_eq!(get(&db, "care.style").unwrap(), text("gentle"));
    }
}
