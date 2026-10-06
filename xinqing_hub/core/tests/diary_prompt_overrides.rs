//! C-10：用户日记提示词覆盖、启动校验与出厂安全约束。
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use xinqing_hub_core::domain::diary::{DiaryCopy, DiaryPrompt, Reject, check_draft};
use xinqing_hub_core::domain::validate::BannedWords;
use xinqing_hub_core::infra::templates::TemplateDirs;

static NEXT: AtomicUsize = AtomicUsize::new(0);
const PROMPT: &str = "<!-- version: 42 -->\n<!-- 自定义注释不出网 -->\n自定义日记\n摘要：{summary}\n要点：{chat_digest}";
struct Fixture(TemplateDirs);
impl Fixture {
    fn new() -> Self {
        let user = std::env::temp_dir().join(format!("xq-diary-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(user.join("prompts")).unwrap();
        Self(TemplateDirs { factory: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../hub_templates"), user: Some(user) })
    }
    fn write(&self, text: &str) { std::fs::write(self.0.user.as_ref().unwrap().join("prompts/diary.md"), text).unwrap(); }
    fn load(&self) -> DiaryPrompt { DiaryPrompt::load(&self.0).unwrap() }
}
impl Drop for Fixture {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(self.0.user.as_ref().unwrap()); }
}

#[test]
fn valid_override_renders_optional_inputs_and_missing_file_uses_factory() {
    let f = Fixture::new();
    assert_eq!(f.load().version, 1);
    f.write(PROMPT);
    let p = f.load();
    assert_eq!(p.ver(), "P-DIARY v42");
    for (summary, digest) in [(Some("今日统计"), Some("用户要点")), (None, Some("用户要点")), (Some("今日统计"), None), (None, None)] {
        let rendered = p.render(summary, digest);
        assert!(rendered.contains("自定义日记"));
        assert!(rendered.contains(summary.unwrap_or("（无）")));
        assert!(rendered.contains(digest.unwrap_or("（无）")));
        assert!(!rendered.contains('{') && !rendered.contains("<!--"));
    }
}

#[test]
fn invalid_headers_or_missing_body_placeholders_fall_back() {
    let f = Fixture::new();
    let factory = f.load().render(Some("统计"), Some("要点"));
    for text in ["", "{summary}{chat_digest}", "<!-- version: 0 -->\n{summary}{chat_digest}", "<!-- version: -1 -->\n{summary}{chat_digest}", "<!-- version: abc -->\n{summary}{chat_digest}", "<!-- version: 42\n{summary}{chat_digest}", "<!-- version: 42 -->\n<!-- {summary}{chat_digest} -->", "<!-- version: 42 -->\n摘要：{summary}", "<!-- version: 42 -->\n要点：{chat_digest}"] {
        f.write(text);
        assert_eq!(f.load().version, 1);
        assert_eq!(f.load().render(Some("统计"), Some("要点")), factory);
    }
    std::fs::write(f.0.user.as_ref().unwrap().join("prompts/diary.md"), [0xff, 0xfe]).unwrap();
    assert_eq!(f.load().version, 1);
}

#[test]
fn custom_prompt_cannot_override_factory_copy_or_disable_draft_checks() {
    let f = Fixture::new();
    f.write(PROMPT);
    std::fs::write(f.0.user.as_ref().unwrap().join("banned_words.toml"), "version = 42\n").unwrap();
    std::fs::write(f.0.user.as_ref().unwrap().join("ui_copy.toml"), "[diary]\nblank = '自定义空白'\nsafety_session = '自定义标题'\n").unwrap();
    let factory = TemplateDirs::factory_only(&f.0.factory);
    let actual = DiaryCopy::load(&f.0).unwrap();
    let expected = DiaryCopy::load(&factory).unwrap();
    assert_eq!(actual.blank, expected.blank);
    assert_eq!(actual.safety_session, expected.safety_session);
    let banned = BannedWords::load(&f.0).unwrap();
    assert_eq!(check_draft("很短", &banned), Err(Reject::Length));
    assert_eq!(check_draft(&format!("{}抑郁", "今天我给自己留了一点安静的时间。".repeat(3)), &banned), Err(Reject::Banned));
    assert_eq!(check_draft(&"today I spent some time resting. ".repeat(3), &banned), Err(Reject::Language));
    assert!(check_draft(&"今天我给自己留了一点安静的时间。".repeat(3), &banned).is_ok());
}

#[test]
fn unusable_factory_is_reported_after_invalid_override() {
    let mut f = Fixture::new();
    f.write("invalid");
    f.0.factory = f.0.user.as_ref().unwrap().join("missing-factory");
    assert!(DiaryPrompt::load(&f.0).is_err());
}
