//! Hub 设置（10 第 6.2 节、03 第 6 节）：键注册表、默认值与取值范围。
//!
//! 值以 JSON 文本存放在 `settings` 表。读取时缺失或已不合法的值一律回落默认值，
//! 写入时先校验，不合法的值不落库。新增键必须同时写明默认值和取值范围（10 第 6.2 节）。
//!
//! 列表类设置（勿扰应用、安静时段）整份读写：界面上删掉一项就是写入删掉后的整份列表，
//! 出厂值只是默认值，用户清空后不会再合并回来（ADR 0019）。

use serde::{Deserialize, Serialize};

use super::dnd;
use crate::infra::gateway::BudgetKind;
use crate::infra::store::{Db, StoreError};

/// 设置项的值。只有这四种形态，对应 [`Kind`]；以 JSON 文本落库（`true` / `0.8` / `"system"` / `["22:00-07:00"]`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(untagged)]
pub enum SettingValue {
    Bool(bool),
    // 从 JSON 读进来的数不会是 NaN，导出成 `number`（ADR 0027）
    Number(#[cfg_attr(feature = "specta", specta(type = specta_typescript::Number))] f64),
    Text(String),
    // 字符串列表（ADR 0019）
    List(Vec<String>),
}

impl SettingValue {
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            SettingValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            SettingValue::Number(x) => Some(*x),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            SettingValue::Text(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&[String]> {
        match self {
            SettingValue::List(v) => Some(v),
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

impl From<&[&str]> for SettingValue {
    fn from(v: &[&str]) -> Self {
        SettingValue::List(v.iter().map(|s| s.to_string()).collect())
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
    /// 闭区间内的整数（落库仍是 JSON 数字）
    Int {
        min: f64,
        max: f64,
    },
    /// 枚举字符串
    Choice(&'static [&'static str]),
    /// 自由填写的一段文字，格式由 [`SettingItem`] 约束
    Text(SettingItem),
    /// 字符串列表：最多 `max` 项，每项按 [`SettingItem`] 校验，不许重复（忽略 ASCII 大小写）
    List {
        item: SettingItem,
        max: usize,
    },
}

/// 文字类设置的格式。只放已知格式，不收任意文本：设置值会出现在界面和固定文案里（求助卡片），
/// 任意文本绕得过禁用词校验（ADR 0019 第 3 条）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum SettingItem {
    /// 进程名，如 `Zoom.exe`（[`dnd::valid_app`]）
    App,
    /// 时段 `"HH:MM-HH:MM"`，可跨午夜（[`dnd::QuietRange`]）
    TimeRange,
    /// 电话号码：数字、空格与 `+-()`，至少 3 个数字，不超过 [`PHONE_MAX_CHARS`] 字；空串表示没填
    Phone,
    /// 用户研究的匿名编号（FR-DMO-04、12 第 4 节）：ASCII 字母、数字、`-`、`_`，不超过
    /// [`RESEARCH_ID_MAX_CHARS`] 字；空串表示没填
    ResearchId,
}

pub const PHONE_MAX_CHARS: usize = 24;
pub const RESEARCH_ID_MAX_CHARS: usize = 16;

impl SettingItem {
    pub fn accepts(self, s: &str) -> bool {
        match self {
            SettingItem::App => dnd::valid_app(s),
            SettingItem::TimeRange => dnd::QuietRange::parse(s).is_some(),
            SettingItem::Phone => {
                s.is_empty()
                    || (s.chars().count() <= PHONE_MAX_CHARS
                        && s.trim() == s
                        && s.chars().all(|c| {
                            c.is_ascii_digit() || matches!(c, ' ' | '+' | '-' | '(' | ')')
                        })
                        && s.chars().filter(char::is_ascii_digit).count() >= 3)
            }
            SettingItem::ResearchId => {
                s.len() <= RESEARCH_ID_MAX_CHARS
                    && s.chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
            }
        }
    }
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
    // 勿扰应用与安静时段（FR-CMF-01 第 5 条，ADR 0019）：整份列表，默认是出厂的三个会议应用、没有安静时段
    KeySpec {
        key: "care.dnd_apps",
        kind: Kind::List {
            item: SettingItem::App,
            max: 32,
        },
        default: || dnd::DEFAULT_APPS.into(),
    },
    KeySpec {
        key: "care.quiet_hours",
        kind: Kind::List {
            item: SettingItem::TimeRange,
            max: 4,
        },
        default: || SettingValue::List(Vec::new()),
    },
    KeySpec {
        key: "care.style",
        kind: Kind::Choice(&["gentle", "lively", "brief"]),
        default: || "gentle".into(),
    },
    // 休息提醒（FR-RST-09）：四类各自的开关与间隔（分钟），默认全部开启；深夜提醒的起始时刻
    KeySpec {
        key: "rest.eye.enabled",
        kind: Kind::Bool,
        default: || true.into(),
    },
    KeySpec {
        key: "rest.eye.interval",
        kind: Kind::Number {
            min: 15.0,
            max: 60.0,
        },
        default: || 20.0.into(),
    },
    KeySpec {
        key: "rest.water.enabled",
        kind: Kind::Bool,
        default: || true.into(),
    },
    KeySpec {
        key: "rest.water.interval",
        kind: Kind::Number {
            min: 30.0,
            max: 120.0,
        },
        default: || 60.0.into(),
    },
    KeySpec {
        key: "rest.move.enabled",
        kind: Kind::Bool,
        default: || true.into(),
    },
    KeySpec {
        key: "rest.move.interval",
        kind: Kind::Number {
            min: 30.0,
            max: 90.0,
        },
        default: || 50.0.into(),
    },
    KeySpec {
        key: "rest.night.enabled",
        kind: Kind::Bool,
        default: || true.into(),
    },
    KeySpec {
        key: "rest.night.start",
        kind: Kind::Choice(crate::domain::rest::NIGHT_STARTS),
        default: || "23:30".into(),
    },
    // 无痕、英文状态下用系统空闲时间估算使用时长（04 FR-SEN-06 第 4 条、FR-RST-01）
    KeySpec {
        key: "rest.count_when_paused",
        kind: Kind::Bool,
        default: || true.into(),
    },
    // 求助卡片上的学校心理中心电话（FR-SAF-03），默认没填
    KeySpec {
        key: "safety.school_phone",
        kind: Kind::Text(SettingItem::Phone),
        default: || "".into(),
    },
    // 每日预算上限（FR-AIG-07）：10 第 6.2 节的 `ai.daily_caps` 拆成每类一个整数键（ADR 0019 第 4 条），默认值与
    // `BudgetKind::default_cap` 一致；0 表示当天不用这一类
    KeySpec {
        key: "ai.cap.jev",
        kind: Kind::Int {
            min: 0.0,
            max: 20000.0,
        },
        default: || 3000.0.into(),
    },
    KeySpec {
        key: "ai.cap.llm",
        kind: Kind::Int {
            min: 0.0,
            max: 2000.0,
        },
        default: || 200.0.into(),
    },
    KeySpec {
        key: "ai.cap.chat",
        kind: Kind::Int {
            min: 0.0,
            max: 1000.0,
        },
        default: || 100.0.into(),
    },
    KeySpec {
        key: "ai.cap.schedule",
        kind: Kind::Int {
            min: 0.0,
            max: 500.0,
        },
        default: || 50.0.into(),
    },
    KeySpec {
        key: "ai.cap.rewrite",
        kind: Kind::Int {
            min: 0.0,
            max: 1000.0,
        },
        default: || 100.0.into(),
    },
    KeySpec {
        key: "chat.retention_days",
        kind: Kind::Choice(&["30", "90", "365", "permanent"]),
        default: || "90".into(),
    },
    KeySpec {
        key: "dev.mode",
        kind: Kind::Bool,
        default: || false.into(),
    },
    // 演示模式（FR-DMO-03）：开启后下次启动进入演示模式，切换要重启 Hub（ADR 0023 第 1 条、ADR 0025）
    KeySpec {
        key: "dev.demo",
        kind: Kind::Bool,
        default: || false.into(),
    },
    // 研究模式（FR-DMO-04）：先填研究编号，再打开开关；编号为空时开关不起作用（ADR 0025）
    KeySpec {
        key: "research.id",
        kind: Kind::Text(SettingItem::ResearchId),
        default: || "".into(),
    },
    KeySpec {
        key: "research.enabled",
        kind: Kind::Bool,
        default: || false.into(),
    },
];

/// 借用 `settings` 表存放的内部状态键：不是用户设置，不在 [`KEYS`] 里，`settings_get` / `settings_set` 命令读写不到；
/// 导出、导入（FR-DAT-03/07）只处理 [`KEYS`] 里的键，这些键不跨电脑带走。新增内部键都登记在这里。
pub const INTERNAL_KEYS: &[&str] = &[
    crate::domain::comfort_feedback::MUTED_UNTIL_KEY,
    crate::domain::comfort_feedback::REDUCED_TS_KEY,
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

/// `settings_schema` 的一项（ADR 0019 第 9 条）：设置页按它出控件、设范围、写提示，不在前端另抄一份取值范围。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct SettingField {
    pub key: String,
    pub kind: SettingFieldKind,
    pub default: SettingValue,
}

/// 取值范围（[`Kind`] 的可序列化形态），按 `type` 区分。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SettingFieldKind {
    Bool,
    /// 闭区间
    Number {
        min: f64,
        max: f64,
    },
    /// 闭区间内的整数
    Int {
        min: f64,
        max: f64,
    },
    /// 枚举字符串
    Choice {
        options: Vec<String>,
    },
    /// 一段文字，格式见 `item`
    Text {
        item: SettingItem,
    },
    /// 字符串列表：最多 `max_items` 项，每项格式见 `item`，不许重复（忽略 ASCII 大小写）
    List {
        item: SettingItem,
        max_items: u32,
    },
}

impl From<Kind> for SettingFieldKind {
    fn from(k: Kind) -> Self {
        match k {
            Kind::Bool => SettingFieldKind::Bool,
            Kind::Number { min, max } => SettingFieldKind::Number { min, max },
            Kind::Int { min, max } => SettingFieldKind::Int { min, max },
            Kind::Choice(opts) => SettingFieldKind::Choice {
                options: opts.iter().map(|s| s.to_string()).collect(),
            },
            Kind::Text(item) => SettingFieldKind::Text { item },
            Kind::List { item, max } => SettingFieldKind::List {
                item,
                max_items: u32::try_from(max).unwrap_or(u32::MAX),
            },
        }
    }
}

/// 全部设置键的取值范围与默认值，顺序同 [`KEYS`]。
pub fn schema() -> Vec<SettingField> {
    KEYS.iter()
        .map(|s| SettingField {
            key: s.key.to_string(),
            kind: s.kind.into(),
            default: (s.default)(),
        })
        .collect()
}

pub fn spec(key: &str) -> Option<&'static KeySpec> {
    KEYS.iter().find(|s| s.key == key)
}

impl KeySpec {
    pub fn accepts(&self, v: &SettingValue) -> bool {
        match (self.kind, v) {
            (Kind::Bool, SettingValue::Bool(_)) => true,
            (Kind::Number { min, max }, SettingValue::Number(x)) => (min..=max).contains(x),
            (Kind::Int { min, max }, SettingValue::Number(x)) => {
                x.fract() == 0.0 && (min..=max).contains(x)
            }
            (Kind::Choice(opts), SettingValue::Text(s)) => opts.contains(&s.as_str()),
            (Kind::Text(item), SettingValue::Text(s)) => item.accepts(s),
            (Kind::List { item, max }, SettingValue::List(v)) => {
                let mut seen = std::collections::HashSet::new();
                v.len() <= max
                    && v.iter()
                        .all(|s| item.accepts(s) && seen.insert(s.to_ascii_lowercase()))
            }
            _ => false,
        }
    }
}

/// 预算类别对应的设置键（ADR 0019 第 4 条）。
pub fn cap_key(kind: BudgetKind) -> &'static str {
    match kind {
        BudgetKind::Jev => "ai.cap.jev",
        BudgetKind::Llm => "ai.cap.llm",
        BudgetKind::ChatTurn => "ai.cap.chat",
        BudgetKind::SchedulePrefilter => "ai.cap.schedule",
        BudgetKind::Rewrite => "ai.cap.rewrite",
    }
}

/// 读出全部预算上限，交给网关 `set_cap`。读不出来的类别用默认值。
pub fn caps(db: &Db) -> Vec<(BudgetKind, u32)> {
    BudgetKind::ALL
        .iter()
        .map(|&k| {
            let cap = get(db, cap_key(k))
                .ok()
                .and_then(|v| v.as_f64())
                .map_or(k.default_cap(), |x| x as u32);
            (k, cap)
        })
        .collect()
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

    #[test]
    fn internal_keys_are_not_settings() {
        let db = Db::open_in_memory().unwrap();
        for k in INTERNAL_KEYS {
            assert!(spec(k).is_none(), "{k} 同时是设置键");
            assert!(k.contains('.'), "{k} 要带命名空间");
            assert!(matches!(get(&db, k), Err(SettingsError::UnknownKey(_))));
        }
    }

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

    fn list(v: &[&str]) -> SettingValue {
        v.into()
    }

    #[test]
    fn lists_are_whole_values_and_can_be_emptied() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(get(&db, "care.dnd_apps").unwrap(), list(dnd::DEFAULT_APPS));
        assert_eq!(get(&db, "care.quiet_hours").unwrap(), list(&[]));
        // 清空出厂的勿扰应用：存的是空列表，读回来也是空的，不会合并回默认值
        assert!(set(&db, "care.dnd_apps", &list(&[])).unwrap());
        assert_eq!(get(&db, "care.dnd_apps").unwrap(), list(&[]));
        assert_eq!(
            db.settings_get("care.dnd_apps").unwrap().as_deref(),
            Some("[]")
        );
        assert!(
            set(
                &db,
                "care.quiet_hours",
                &list(&["22:30-07:00", "12:00-13:00"])
            )
            .unwrap()
        );
        assert_eq!(
            db.settings_get("care.quiet_hours").unwrap().as_deref(),
            Some(r#"["22:30-07:00","12:00-13:00"]"#)
        );
    }

    #[test]
    fn list_items_are_validated() {
        let db = Db::open_in_memory().unwrap();
        for bad in [
            list(&["Zoom.exe", "zoom.exe"]),
            list(&["C:\\Zoom.exe"]),
            list(&[""]),
            text("Zoom.exe"),
            SettingValue::List(vec!["a.exe".into(); 33]),
        ] {
            assert!(
                matches!(
                    set(&db, "care.dnd_apps", &bad),
                    Err(SettingsError::OutOfRange { .. })
                ),
                "{bad:?}"
            );
        }
        for bad in [
            list(&["22:00-22:00"]),
            list(&["7:00-8:00"]),
            list(&[
                "01:00-02:00",
                "02:00-03:00",
                "03:00-04:00",
                "04:00-05:00",
                "05:00-06:00",
            ]),
        ] {
            assert!(set(&db, "care.quiet_hours", &bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn school_phone_accepts_phone_numbers_only() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(get(&db, "safety.school_phone").unwrap(), text(""));
        for ok in ["010-1234 5678", "+86 (10) 12345678", "12356", ""] {
            assert!(set(&db, "safety.school_phone", &text(ok)).is_ok(), "{ok}");
        }
        for bad in [
            "打这个电话",
            "12",
            " 12356",
            "1234567890123456789012345",
            "12356\n",
        ] {
            assert!(
                set(&db, "safety.school_phone", &text(bad)).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn research_id_is_a_short_anonymous_code() {
        let ok = |s: &str| SettingItem::ResearchId.accepts(s);
        assert!(ok(""), "空串表示没填");
        assert!(ok("P01") && ok("xq-2026_07") && ok("ABCDEFGHIJKLMNOP"));
        assert!(!ok("ABCDEFGHIJKLMNOPQ"), "超过 16 字");
        assert!(
            !ok("张三") && !ok("P 01") && !ok("a@b.com"),
            "不收姓名、空格、邮箱"
        );
        let db = Db::open_in_memory().unwrap();
        assert_eq!(get(&db, "research.id").unwrap(), SettingValue::from(""));
        assert_eq!(
            get(&db, "research.enabled").unwrap(),
            SettingValue::from(false)
        );
        assert_eq!(get(&db, "dev.demo").unwrap(), SettingValue::from(false));
        assert!(set(&db, "research.id", &"张三".into()).is_err());
        assert!(set(&db, "research.id", &"P07".into()).is_ok());
    }

    #[test]
    fn caps_are_integers_and_default_to_the_budget_defaults() {
        let db = Db::open_in_memory().unwrap();
        for k in BudgetKind::ALL {
            let s = spec(cap_key(k)).expect("每个预算类别都有设置键");
            assert_eq!((s.default)(), SettingValue::Number(k.default_cap() as f64));
        }
        assert!(set(&db, "ai.cap.chat", &SettingValue::Number(1.5)).is_err());
        assert!(set(&db, "ai.cap.chat", &SettingValue::Number(-1.0)).is_err());
        set(&db, "ai.cap.chat", &SettingValue::Number(0.0)).unwrap();
        set(&db, "ai.cap.jev", &SettingValue::Number(500.0)).unwrap();
        let caps = caps(&db);
        assert!(caps.contains(&(BudgetKind::ChatTurn, 0)));
        assert!(caps.contains(&(BudgetKind::Jev, 500)));
        assert!(caps.contains(&(BudgetKind::Llm, 200)));
    }

    #[test]
    fn schema_lists_every_key_with_its_range() {
        let fields = schema();
        assert_eq!(fields.len(), KEYS.len());
        let by_key = |k: &str| fields.iter().find(|f| f.key == k).unwrap().clone();
        assert_eq!(
            by_key("ai.cap.chat").kind,
            SettingFieldKind::Int {
                min: 0.0,
                max: 1000.0
            }
        );
        // 前端按这个形状收窄类型
        assert_eq!(
            serde_json::to_value(by_key("care.quiet_hours")).unwrap(),
            serde_json::json!({
                "key": "care.quiet_hours",
                "kind": {"type": "list", "item": "time_range", "max_items": 4},
                "default": []
            })
        );
        assert_eq!(
            serde_json::to_value(by_key("ui.theme").kind).unwrap(),
            serde_json::json!({"type": "choice", "options": ["system", "light", "dark"]})
        );
        assert_eq!(
            serde_json::to_value(by_key("safety.school_phone").kind).unwrap(),
            serde_json::json!({"type": "text", "item": "phone"})
        );
        assert_eq!(
            serde_json::to_value(by_key("widget.visible").kind).unwrap(),
            serde_json::json!({"type": "bool"})
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
