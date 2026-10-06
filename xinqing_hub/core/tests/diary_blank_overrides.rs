use std::path::PathBuf;
use xinqing_hub_core::domain::diary::DiaryCopy;
use xinqing_hub_core::infra::templates::TemplateDirs;

#[test]
fn blank_copy_overrides_without_changing_safety_title_and_invalid_copy_falls_back() {
    let user = std::env::temp_dir().join(format!("xq-diary-blank-{}", std::process::id()));
    std::fs::create_dir_all(&user).unwrap();
    let dirs = TemplateDirs { factory: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../hub_templates"), user: Some(user.clone()) };
    let expected = DiaryCopy::load(&dirs).unwrap();
    let file = user.join("ui_copy.toml");
    std::fs::write(&file, "version = 42\n[diary]\nblank = '今天的心情：我的感受：明天的小期待：'\nsafety_session = '不应覆盖'\n").unwrap();
    let actual = DiaryCopy::load(&dirs).unwrap();
    assert_eq!(actual.blank, "今天的心情：我的感受：明天的小期待：");
    assert_eq!(actual.safety_session, expected.safety_session);
    std::fs::write(user.join("banned_words.toml"), "version = 42\n").unwrap();
    for invalid in ["version = [", "version = 42", "version = 0\n[diary]\nblank = '日记'", "[diary]\nblank = '日记'", "version = 42\n[diary]\nblank = '  '", "version = 42\n[diary]\nblank = '抑郁'"] {
        std::fs::write(&file, invalid).unwrap();
        let actual = DiaryCopy::load(&dirs).unwrap();
        assert_eq!(actual.blank, expected.blank);
        assert_eq!(actual.safety_session, expected.safety_session);
    }
    std::fs::write(&file, [0xff]).unwrap();
    assert_eq!(DiaryCopy::load(&dirs).unwrap().blank, expected.blank);
    std::fs::remove_dir_all(user).unwrap();
}
