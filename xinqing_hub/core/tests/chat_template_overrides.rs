use std::path::PathBuf;
use xinqing_hub_core::domain::chat::{ChatMode, ChatPrompts};
use xinqing_hub_core::domain::comfort::Style;
use xinqing_hub_core::infra::templates::TemplateDirs;

#[test]
fn ordinary_chat_and_modes_override_independently_while_safe_prompt_stays_factory() {
    let user = std::env::temp_dir().join(format!("xq-chat-overrides-{}", std::process::id()));
    std::fs::create_dir_all(user.join("prompts")).unwrap();
    let mut dirs = TemplateDirs { factory: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../hub_templates"), user: Some(user.clone()) };
    let factory = ChatPrompts::load(&dirs).unwrap();
    let safe = factory.system(true, Style::Gentle, ChatMode::Normal, None, &[]);
    for (name, body) in [("chat.md", "自定义对话\n{style_block}\n{today_summary_block}\n{memory_block}"), ("chat_vent.md", "自定义吐槽"), ("chat_organize.md", "自定义理一理"), ("chat_safe.md", "危险自定义不应生效")] {
        std::fs::write(user.join("prompts").join(name), format!("<!-- version: 42 -->\n{body}")).unwrap();
    }
    let p = ChatPrompts::load(&dirs).unwrap();
    assert_eq!(p.ver(false, ChatMode::Vent), "P-CHAT v42 + P-CHAT-VENT v42");
    let rendered = p.system(false, Style::Gentle, ChatMode::Organize, Some("今日统计"), &["记忆事项".into()]);
    assert!(rendered.contains("今日统计") && rendered.contains("记忆事项") && rendered.contains("自定义理一理"));
    assert!(!rendered.contains("{memory_block}"));
    assert_eq!(p.system(true, Style::Lively, ChatMode::Organize, Some("统计"), &["记忆".into()]), safe);
    assert_eq!(p.ver(true, ChatMode::Vent), "P-CHAT-SAFE v1");
    for invalid in ["", "<!-- version: 0 -->\n{style_block}{today_summary_block}{memory_block}", "<!-- version: 42 -->\n{style_block}{today_summary_block}"] {
        std::fs::write(user.join("prompts/chat.md"), invalid).unwrap();
        assert_eq!(ChatPrompts::load(&dirs).unwrap().ver(false, ChatMode::Vent), "P-CHAT v1 + P-CHAT-VENT v42");
    }
    std::fs::write(user.join("prompts/chat_vent.md"), [0xff]).unwrap();
    assert_eq!(ChatPrompts::load(&dirs).unwrap().ver(false, ChatMode::Vent), "P-CHAT v1 + P-CHAT-VENT v1");
    dirs.factory = user.join("missing");
    assert!(ChatPrompts::load(&dirs).is_err());
    std::fs::remove_dir_all(user).unwrap();
}
