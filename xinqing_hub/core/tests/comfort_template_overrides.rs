//! C-04：用户暖心话与提示词整体覆盖、校验回退、出厂禁用词表约束。
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use xinqing_hub_core::domain::comfort::{ComfortPrompt, ComfortTemplates, Group, Style};
use xinqing_hub_core::domain::validate::{BannedWords, Scene};
use xinqing_hub_core::infra::templates::TemplateDirs;

static NEXT: AtomicUsize = AtomicUsize::new(0);
const CUSTOM_LINE: &str = "给自己留一点慢慢来的时间";

struct Fixture(TemplateDirs);

impl Fixture {
    fn new() -> Self {
        let user = std::env::temp_dir().join(format!(
            "xq-comfort-overrides-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(user.join("prompts")).unwrap();
        Self(TemplateDirs {
            factory: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../hub_templates"),
            user: Some(user),
        })
    }

    fn write(&self, name: &str, text: &str) {
        std::fs::write(self.0.user.as_ref().unwrap().join(name), text).unwrap();
    }

    fn custom_lines(&self) -> toml::Value {
        let text = std::fs::read_to_string(self.0.factory_path("comfort.toml")).unwrap();
        let mut value: toml::Value = toml::from_str(&text).unwrap();
        value["version"] = toml::Value::Integer(42);
        for line in value["gentle"]["cheer"].as_array_mut().unwrap() {
            line["text"] = toml::Value::String(CUSTOM_LINE.into());
        }
        value
    }

    fn picked(&self) -> String {
        ComfortTemplates::load(&self.0).unwrap()
            .pick(Style::Gentle, Group::Cheer, &[], &HashSet::new(), 0).unwrap().text
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.0.user.as_ref().unwrap());
    }
}

#[test]
fn valid_overrides_replace_factory_files_independently() {
    let f = Fixture::new();
    assert_eq!(ComfortTemplates::load(&f.0).unwrap().version, 1);
    f.write("comfort.toml", &toml::to_string(&f.custom_lines()).unwrap());
    assert_eq!(ComfortTemplates::load(&f.0).unwrap().version, 42);
    assert_eq!(f.picked(), CUSTOM_LINE);
    assert_eq!(ComfortPrompt::load(&f.0).unwrap().version, 1);

    f.write("prompts/comfort.md", "<!-- version: 42 -->\n自定义提示词\n{style_block}\n{summary_json}\n{recent_texts}");
    let prompt = ComfortPrompt::load(&f.0).unwrap();
    assert_eq!(prompt.version, 42);
    assert!(prompt.body().contains("自定义提示词"));
    assert!(!prompt.body().contains("<!--"));
}

#[test]
fn malformed_or_unsafe_user_lines_fall_back_to_factory() {
    let f = Fixture::new();
    f.write("comfort.toml", "version = [");
    assert_eq!(ComfortTemplates::load(&f.0).unwrap().version, 1);

    // TOML 解析成功也要做内容校验；用户词表不能取消出厂禁用词约束。
    f.write("banned_words.toml", "version = 42\n");
    for bad in ["你可能有点抑郁，早点休息吧", "加油", "慢慢来不用着急！！"] {
        let mut value = f.custom_lines();
        value["gentle"]["cheer"][0]["text"] = toml::Value::String(bad.into());
        f.write("comfort.toml", &toml::to_string(&value).unwrap());
        assert_eq!(ComfortTemplates::load(&f.0).unwrap().version, 1);
        assert_ne!(f.picked(), bad);
    }
    let banned = BannedWords::load(&f.0).unwrap();
    assert!(banned.find("抑郁", Scene::Other).is_some());
}

#[test]
fn incomplete_groups_duplicate_ids_and_no_brief_fallback_are_rejected() {
    let f = Fixture::new();
    for case in 0..3 {
        let mut value = f.custom_lines();
        match case {
            0 => { value["lively"].as_table_mut().unwrap().remove("low"); }
            1 => { value["gentle"]["cheer"][0]["id"] = value["gentle"]["cheer"][1]["id"].clone(); }
            _ => {
                for line in value["gentle"]["cheer"].as_array_mut().unwrap() {
                    line["text"] = toml::Value::String("今天可以给自己多留一点慢慢来的时间".into());
                }
            }
        }
        f.write("comfort.toml", &toml::to_string(&value).unwrap());
        assert_eq!(ComfortTemplates::load(&f.0).unwrap().version, 1);
    }
}

#[test]
fn invalid_prompt_falls_back_without_discarding_valid_lines() {
    let f = Fixture::new();
    f.write("comfort.toml", &toml::to_string(&f.custom_lines()).unwrap());
    for bad in ["没有版本号", "<!-- version: 0 -->", "<!-- version: 2 -->\n缺少占位符", "<!-- version: 2\n{style_block}{summary_json}{recent_texts}"] {
        f.write("prompts/comfort.md", bad);
        assert_eq!(ComfortPrompt::load(&f.0).unwrap().version, 1);
        assert_eq!(f.picked(), CUSTOM_LINE);
    }
    // 非 UTF-8 与同名目录也不能让暖心话加载失败。
    let prompt_path = f.0.user.as_ref().unwrap().join("prompts/comfort.md");
    std::fs::write(&prompt_path, [0xff, 0xfe]).unwrap();
    assert_eq!(ComfortPrompt::load(&f.0).unwrap().version, 1);
    std::fs::remove_file(&prompt_path).unwrap();
    std::fs::create_dir(&prompt_path).unwrap();
    assert_eq!(ComfortPrompt::load(&f.0).unwrap().version, 1);
}

#[test]
fn missing_or_broken_factory_is_not_hidden_when_user_is_invalid() {
    let mut f = Fixture::new();
    f.write("prompts/comfort.md", "无效提示词");
    f.0.factory = f.0.user.as_ref().unwrap().join("missing-factory");
    assert!(ComfortPrompt::load(&f.0).is_err());
    assert!(ComfortTemplates::load(&f.0).is_err());
}
