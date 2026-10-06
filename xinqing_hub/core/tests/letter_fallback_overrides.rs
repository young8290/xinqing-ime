use std::path::PathBuf;
use xinqing_hub_core::domain::letter::{LetterFallback, WeekFacts, stats};
use xinqing_hub_core::infra::templates::TemplateDirs;

#[test]
fn fallback_override_renders_optional_data_and_rejects_broken_syntax() {
    let user = std::env::temp_dir().join(format!("xq-letter-fallback-{}", std::process::id()));
    std::fs::create_dir_all(&user).unwrap();
    let mut dirs = TemplateDirs { factory: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../hub_templates"), user: Some(user.clone()) };
    let file = user.join("letter_fallback.md");
    let header = "<!-- version: 42 -->\n";
    let custom = "用户兜底 {typing_avg} {?water}喝水 {water} 次{/water} {?rest_rate}休息 {rest_rate} {rest_comment}{/rest_rate} {tip}";
    std::fs::write(&file, format!("{header}{custom}")).unwrap();
    let fallback = LetterFallback::load(&dirs).unwrap();
    let mut s = stats(&WeekFacts::default());
    let out = fallback.render(&s);
    assert!(out.contains("用户兜底") && !out.contains("喝水") && !out.contains("休息"));
    s.water = 3;
    s.rest_rate = Some(80);
    let out = fallback.render(&s);
    assert!(out.contains("喝水 3 次") && out.contains("休息 80%") && !out.contains('{'));
    let factory = LetterFallback::load(&TemplateDirs::factory_only(&dirs.factory)).unwrap().render(&s);
    for invalid in ["", "{unknown}", "{?water}{water}", "{/water}", "{?water}{water}{/rest_rate}", "{?water}{?rest_rate}{rest_rate}{/rest_rate}{/water}", "{water}", "{rest_comment}", "{typing_avg", "typing_avg}", "你可能有点抑郁"] {
        std::fs::write(&file, format!("{header}{invalid}")).unwrap();
        assert_eq!(LetterFallback::load(&dirs).unwrap().render(&s), factory, "case {invalid}");
    }
    std::fs::write(&file, "<!-- version: 0 -->\n自定义").unwrap();
    assert_eq!(LetterFallback::load(&dirs).unwrap().render(&s), factory);
    std::fs::write(&file, [0xff]).unwrap();
    assert_eq!(LetterFallback::load(&dirs).unwrap().render(&s), factory);
    let empty = user.join("factory");
    std::fs::create_dir_all(&empty).unwrap();
    std::fs::copy(dirs.factory_path("banned_words.toml"), empty.join("banned_words.toml")).unwrap();
    dirs.factory = empty;
    assert!(LetterFallback::load(&dirs).is_err());
    std::fs::remove_dir_all(user).unwrap();
}
