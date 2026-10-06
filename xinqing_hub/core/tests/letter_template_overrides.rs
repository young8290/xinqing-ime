//! C-10：周信提示词与配套文案的独立覆盖、校验和回退。
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use xinqing_hub_core::domain::comfort::Style;
use xinqing_hub_core::domain::letter::{LetterFallback, LetterPrompt, WeekFacts, stats};
use xinqing_hub_core::infra::templates::TemplateDirs;

static NEXT: AtomicUsize = AtomicUsize::new(0);
const LINE: &str = "给自己留一点安静的时间。";
const PROMPT: &str = "<!-- version: 42 -->\n自定义周信\n{style_block}\n本周统计：{weekly_stats_json}";
struct Fixture(TemplateDirs);
impl Fixture {
    fn new() -> Self {
        let user = std::env::temp_dir().join(format!("xq-letter-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(user.join("prompts")).unwrap();
        Self(TemplateDirs { factory: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../hub_templates"), user: Some(user) })
    }
    fn write(&self, file: &str, text: &str) {
        std::fs::write(self.0.user.as_ref().unwrap().join(file), text).unwrap();
    }
    fn tips(&self) -> toml::Value {
        let mut value: toml::Value = toml::from_str(&std::fs::read_to_string(self.0.factory_path("letter_tips.toml")).unwrap()).unwrap();
        value["version"] = toml::Value::Integer(42);
        for group in ["high", "mid", "low"] { value["rest_comment"][group] = toml::Value::String(LINE.into()); }
        for tip in value["tip"].as_array_mut().unwrap() { tip["text"] = toml::Value::String(LINE.into()); }
        value
    }
    fn copy(&self) -> Vec<String> { LetterFallback::load(&self.0).unwrap().all_copy() }
}
impl Drop for Fixture {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(self.0.user.as_ref().unwrap()); }
}

#[test]
fn valid_files_override_independently_and_render_statistics() {
    let f = Fixture::new();
    let factory_copy = f.copy();
    assert_eq!(LetterPrompt::load(&f.0).unwrap().version, 1);
    f.write("prompts/letter.md", PROMPT);
    let p = LetterPrompt::load(&f.0).unwrap();
    assert_eq!(p.ver(), "P-LETTER v42");
    let rendered = p.render(Style::Gentle, "{\"valid_days\":3}");
    assert!(rendered.contains("自定义周信") && rendered.contains("\"valid_days\":3"));
    assert!(!rendered.contains("{style_block}") && !rendered.contains("{weekly_stats_json}"));
    assert_eq!(f.copy(), factory_copy);
    f.write("letter_tips.toml", &toml::to_string(&f.tips()).unwrap());
    let copy = f.copy();
    assert_eq!(copy[0], LINE);
    assert_eq!(copy[3], factory_copy[3]);
    assert!(copy.iter().enumerate().all(|(i, s)| i == 3 || s == LINE));
    let fallback = LetterFallback::load(&f.0).unwrap();
    let mut s = stats(&WeekFacts::default());
    for rate in [None, Some(10), Some(50), Some(90)] {
        s.rest_rate = rate;
        let body = fallback.render(&s);
        assert!(body.contains(LINE));
        assert!(!body.contains('{'));
    }
}

#[test]
fn invalid_prompts_fall_back_without_discarding_valid_tips() {
    let f = Fixture::new();
    f.write("letter_tips.toml", &toml::to_string(&f.tips()).unwrap());
    for text in ["", "<!-- version: 0 -->\n{style_block}{weekly_stats_json}", "<!-- version: 42\n{style_block}{weekly_stats_json}", "{style_block}{weekly_stats_json}", "<!-- version: 42 -->\n{weekly_stats_json}", "<!-- version: 42 -->\n{style_block}"] {
        f.write("prompts/letter.md", text);
        assert_eq!(LetterPrompt::load(&f.0).unwrap().version, 1);
        assert_eq!(f.copy()[0], LINE);
    }
    std::fs::write(f.0.user.as_ref().unwrap().join("prompts/letter.md"), [0xff]).unwrap();
    assert_eq!(LetterPrompt::load(&f.0).unwrap().version, 1);
}

#[test]
fn invalid_tips_fall_back_without_discarding_valid_prompt_or_factory_bans() {
    let f = Fixture::new();
    let factory = f.copy();
    f.write("prompts/letter.md", PROMPT);
    f.write("banned_words.toml", "version = 42\n");
    for case in 0..9 {
        let mut value = f.tips();
        match case {
            0 => { value["version"] = toml::Value::Integer(0); }
            1 => { value.as_table_mut().unwrap().remove("version"); }
            2 => { value["tip"].as_array_mut().unwrap().pop(); }
            3 => { value["tip"][0]["when"] = toml::Value::String("default".into()); }
            4 => { value["tip"][0]["when"] = toml::Value::String("unknown".into()); }
            5 => { value["rest_comment"]["mid"] = toml::Value::String("  ".into()); }
            6 => { value["tip"][0]["text"] = toml::Value::String("  ".into()); }
            7 => { value["rest_comment"]["high"] = toml::Value::String("你可能有点抑郁".into()); }
            _ => { value["tip"][0]["text"] = toml::Value::String("你可能有点抑郁".into()); }
        }
        f.write("letter_tips.toml", &toml::to_string(&value).unwrap());
        assert_eq!(f.copy(), factory);
        assert_eq!(LetterPrompt::load(&f.0).unwrap().version, 42);
    }
    f.write("letter_tips.toml", "version = [");
    assert_eq!(f.copy(), factory);
    std::fs::write(f.0.user.as_ref().unwrap().join("letter_tips.toml"), [0xff]).unwrap();
    assert_eq!(f.copy(), factory);
}

#[test]
fn invalid_override_and_unavailable_factory_are_reported() {
    let mut f = Fixture::new();
    let factory = f.0.user.as_ref().unwrap().join("factory");
    std::fs::create_dir_all(&factory).unwrap();
    for name in ["banned_words.toml", "letter_fallback.md"] {
        std::fs::copy(f.0.factory_path(name), factory.join(name)).unwrap();
    }
    f.0.factory = factory;
    f.write("letter_tips.toml", "version = [");
    f.write("prompts/letter.md", "invalid");
    assert!(LetterFallback::load(&f.0).is_err());
    assert!(LetterPrompt::load(&f.0).is_err());
}
