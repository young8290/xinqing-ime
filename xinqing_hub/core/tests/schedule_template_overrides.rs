use std::path::PathBuf;
use chrono::NaiveDate;
use xinqing_hub_core::domain::schedule::{CandidateKind, ExtractPrompts, ScheduleRecognizer};
use xinqing_hub_core::infra::templates::TemplateDirs;

#[test]
fn schedule_and_todo_prompts_override_independently() {
    let user = std::env::temp_dir().join(format!("xq-extract-{}", std::process::id()));
    std::fs::create_dir_all(user.join("prompts")).unwrap();
    let mut dirs = TemplateDirs { factory: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../hub_templates"), user: Some(user.clone()) };
    for name in ["schedule", "todo"] {
        std::fs::write(user.join(format!("prompts/{name}.md")), "<!-- version: 42 -->\n自定义：{date} {weekday} {sentence}").unwrap();
    }
    for kind in [CandidateKind::Schedule, CandidateKind::Todo] {
        let p = ExtractPrompts::load(&dirs).unwrap();
        assert!(p.ver(kind).ends_with("v42"));
        let out = p.render(kind, "明天开会", NaiveDate::from_ymd_opt(2026, 10, 6).unwrap());
        assert!(out.contains("2026-10-06") && out.contains("周二") && out.contains("明天开会"));
    }
    for invalid in ["", "<!-- version: 0 -->\n{date}{weekday}{sentence}", "<!-- version: 42 -->\n{date}{sentence}"] {
        std::fs::write(user.join("prompts/schedule.md"), invalid).unwrap();
        let p = ExtractPrompts::load(&dirs).unwrap();
        assert_eq!(p.ver(CandidateKind::Schedule), "P-SCHEDULE v1");
        assert_eq!(p.ver(CandidateKind::Todo), "P-TODO v42");
    }
    std::fs::write(user.join("prompts/todo.md"), [0xff]).unwrap();
    assert_eq!(ExtractPrompts::load(&dirs).unwrap().ver(CandidateKind::Todo), "P-TODO v1");
    dirs.factory = user.join("missing");
    assert!(ExtractPrompts::load(&dirs).is_err());
    std::fs::remove_dir_all(user).unwrap();
}

#[test]
fn recognizer_override_checks_bounds_regexes_and_falls_back() {
    let user = std::env::temp_dir().join(format!("xq-patterns-{}", std::process::id()));
    std::fs::create_dir_all(&user).unwrap();
    let mut dirs = TemplateDirs { factory: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../hub_templates"), user: Some(user.clone()) };
    let original: toml::Value = toml::from_str(&std::fs::read_to_string(dirs.factory_path("schedule_patterns.toml")).unwrap()).unwrap();
    let file = user.join("schedule_patterns.toml");
    let mut valid = original.clone();
    valid["daily_cap"] = toml::Value::Integer(3);
    std::fs::write(&file, toml::to_string(&valid).unwrap()).unwrap();
    assert_eq!(ScheduleRecognizer::load(&dirs).unwrap().daily_cap(), 3);
    for case in 0..6 {
        let mut value = valid.clone();
        match case {
            0 => {value["version"] = toml::Value::Integer(0);}
            1 => {value["min_len"] = toml::Value::Integer(0);}
            2 => {value["max_len"] = toml::Value::Integer(1);}
            3 => {value["daily_cap"] = toml::Value::Integer(0);}
            4 => {value["time_of_day"]["pattern"] = toml::Value::String("[".into());}
            _ => {value["event_verb"]["pattern"] = toml::Value::String(" ".into());}
        }
        std::fs::write(&file, toml::to_string(&value).unwrap()).unwrap();
        let r = ScheduleRecognizer::load(&dirs).unwrap();
        assert_eq!(r.daily_cap(), 50);
        assert!(!r.scan("明天下午三点开会", 0).candidates.is_empty());
    }
    std::fs::write(&file, [0xff]).unwrap();
    assert_eq!(ScheduleRecognizer::load(&dirs).unwrap().daily_cap(), 50);
    dirs.factory = user.join("missing");
    assert!(ScheduleRecognizer::load(&dirs).is_err());
    std::fs::remove_dir_all(user).unwrap();
}
