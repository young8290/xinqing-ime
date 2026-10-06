use std::path::PathBuf;
use xinqing_hub_core::domain::reminder::{RemindKind, ReminderCopy, Subject};
use xinqing_hub_core::infra::templates::TemplateDirs;

#[test]
fn reminder_override_keeps_subjects_actions_and_rejects_broken_fields() {
    let user = std::env::temp_dir().join(format!("xq-reminder-copy-{}", std::process::id()));
    std::fs::create_dir_all(&user).unwrap();
    let dirs = TemplateDirs {
        factory: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../hub_templates"),
        user: Some(user.clone()),
    };
    let subject = Subject {
        title: "组会",
        time: "15:00",
        location: Some("实验楼"),
        count: 3,
    };
    let original: toml::Value =
        toml::from_str(&std::fs::read_to_string(dirs.factory_path("ui_copy.toml")).unwrap())
            .unwrap();
    let factory = ReminderCopy::load(&dirs).unwrap();
    let mut value = original.clone();
    value["notify"]["schedule_title"] = toml::Value::String("接下来：{title}".into());
    value["notify"]["digest_body"] = toml::Value::String("今天还有 {n} 项待办".into());
    let file = user.join("ui_copy.toml");
    std::fs::write(&file, toml::to_string(&value).unwrap()).unwrap();
    let custom = ReminderCopy::load(&dirs).unwrap();
    let notice = custom.notice(RemindKind::Schedule, &subject);
    let expected = factory.notice(RemindKind::Schedule, &subject);
    assert_eq!(notice.title, "接下来：组会");
    assert_eq!(notice.body, expected.body);
    assert_eq!(notice.buttons, expected.buttons);
    assert_eq!(
        custom.notice(RemindKind::TodoDigest, &subject).body,
        "今天还有 3 项待办"
    );
    std::fs::write(user.join("banned_words.toml"), "version = 42\n").unwrap();
    for text in [
        "  ",
        "抑郁 {time}{location_suffix}",
        "疲劳 {time}{location_suffix}",
        "{unknown}",
        "{time}",
        "{time}{location_suffix",
        "{time}{location_suffix}}",
    ] {
        let mut invalid = value.clone();
        invalid["notify"]["schedule_body"] = toml::Value::String(text.into());
        std::fs::write(&file, toml::to_string(&invalid).unwrap()).unwrap();
        assert_eq!(ReminderCopy::load(&dirs).unwrap(), factory);
    }
    value["version"] = toml::Value::Integer(0);
    std::fs::write(&file, toml::to_string(&value).unwrap()).unwrap();
    assert_eq!(ReminderCopy::load(&dirs).unwrap(), factory);
    std::fs::write(&file, [0xff]).unwrap();
    assert_eq!(ReminderCopy::load(&dirs).unwrap(), factory);
    std::fs::remove_dir_all(user).unwrap();
}
