//! C-10：晚间小结用户模板整体覆盖与失败回退。
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use xinqing_hub_core::domain::evening::{EveningTemplates, Group};
use xinqing_hub_core::infra::templates::TemplateDirs;

static NEXT: AtomicUsize = AtomicUsize::new(0);
const GROUPS: [&str; 7] = [
    "sunny", "hesitant", "low", "agitated", "tired", "mixed", "late",
];
const LINE: &str = "今天先到这里，给自己一点安静的时间。";

struct Fixture(TemplateDirs);
impl Fixture {
    fn new() -> Self {
        let user = std::env::temp_dir().join(format!(
            "xq-evening-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&user).unwrap();
        Self(TemplateDirs {
            factory: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../hub_templates"),
            user: Some(user),
        })
    }
    fn value(&self) -> toml::Value {
        let mut value: toml::Value =
            toml::from_str(&std::fs::read_to_string(self.0.factory_path("evening.toml")).unwrap())
                .unwrap();
        value["version"] = toml::Value::Integer(42);
        for group in GROUPS {
            for line in value[group].as_array_mut().unwrap() {
                line["text"] = toml::Value::String(LINE.into());
            }
        }
        value
    }
    fn write(&self, text: &str) {
        std::fs::write(self.0.user.as_ref().unwrap().join("evening.toml"), text).unwrap();
    }
    fn load(&self) -> EveningTemplates {
        EveningTemplates::load(&self.0).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.0.user.as_ref().unwrap());
    }
}

#[test]
fn valid_user_file_replaces_every_group_and_missing_file_uses_factory() {
    let f = Fixture::new();
    assert_ne!(f.load().pick(Group::Late, 0), LINE);
    f.write(&toml::to_string(&f.value()).unwrap());
    for group in [
        Group::Sunny,
        Group::Hesitant,
        Group::Low,
        Group::Agitated,
        Group::Tired,
        Group::Mixed,
        Group::Late,
    ] {
        assert_eq!(f.load().pick(group, 0), LINE);
    }
}

#[test]
fn invalid_syntax_or_content_falls_back_and_user_banned_words_cannot_disable_checks() {
    let f = Fixture::new();
    std::fs::write(
        f.0.user.as_ref().unwrap().join("banned_words.toml"),
        "version = 42\n",
    )
    .unwrap();
    for case in 0..5 {
        let mut value = f.value();
        match case {
            0 => {
                value["version"] = toml::Value::Integer(0);
            }
            1 => {
                value.as_table_mut().unwrap().remove("late");
            }
            2 => {
                value["low"][0]["text"] = toml::Value::String("你可能有点抑郁".into());
            }
            3 => {
                value["mixed"][0]["text"] = toml::Value::String("   ".into());
            }
            _ => {
                value.as_table_mut().unwrap().remove("version");
            }
        }
        f.write(&toml::to_string(&value).unwrap());
        assert_ne!(f.load().pick(Group::Sunny, 0), LINE);
        assert!(f.load().all().all(|text| !text.contains("抑郁")));
    }
    f.write("version = [");
    assert_ne!(f.load().pick(Group::Sunny, 0), LINE);
    std::fs::write(
        f.0.user.as_ref().unwrap().join("evening.toml"),
        [0xff, 0xfe],
    )
    .unwrap();
    assert_ne!(f.load().pick(Group::Sunny, 0), LINE);
}

#[test]
fn unusable_factory_is_reported_when_override_is_invalid() {
    let mut f = Fixture::new();
    f.write("version = [");
    let factory = f.0.user.as_ref().unwrap().join("factory");
    std::fs::create_dir_all(&factory).unwrap();
    std::fs::copy(
        f.0.factory_path("banned_words.toml"),
        factory.join("banned_words.toml"),
    )
    .unwrap();
    f.0.factory = factory;
    assert!(EveningTemplates::load(&f.0).is_err());
}
