use std::path::PathBuf;
use xinqing_hub_core::domain::rewrite::RewritePrompt;
use xinqing_hub_core::infra::templates::TemplateDirs;
use xqp::RewriteStyle;

#[test]
fn rewrite_override_validates_required_fields_and_falls_back() {
    let user = std::env::temp_dir().join(format!("xq-rewrite-{}", std::process::id()));
    std::fs::create_dir_all(user.join("prompts")).unwrap();
    let mut dirs = TemplateDirs { factory: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../hub_templates"), user: Some(user.clone()) };
    let path = user.join("prompts/rewrite.md");
    assert_eq!(RewritePrompt::load(&dirs).unwrap().version, 1);
    std::fs::write(&path, "<!-- version: 42 -->\n<!-- 不出网 -->\n自定义：{style}\n原文：{text}").unwrap();
    let p = RewritePrompt::load(&dirs).unwrap();
    assert_eq!(p.ver(), "P-REWRITE v42");
    let rendered = p.render(RewriteStyle::Gentle, "[号码1]明天见");
    assert!(rendered.contains("[号码1]明天见") && !rendered.contains("<!--"));
    for text in ["", "<!-- version: 0 -->\n{style}{text}", "<!-- version: 42\n{style}{text}", "<!-- version: 42 -->\n{style}", "<!-- version: 42 -->\n{text}", "<!-- version: 42 -->\n<!-- {style}{text} -->"] {
        std::fs::write(&path, text).unwrap();
        assert_eq!(RewritePrompt::load(&dirs).unwrap().version, 1);
    }
    std::fs::write(&path, [0xff]).unwrap();
    assert_eq!(RewritePrompt::load(&dirs).unwrap().version, 1);
    dirs.factory = user.join("missing");
    assert!(RewritePrompt::load(&dirs).is_err());
    std::fs::remove_dir_all(user).unwrap();
}
