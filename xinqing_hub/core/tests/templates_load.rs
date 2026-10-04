//! 仓库中的出厂模板都能被 Hub 加载（模板语法错误应在 CI 中暴露，而不是运行时回退）。

use std::path::PathBuf;

use xinqing_hub_core::domain::validate::{BannedWords, Scene};
use xinqing_hub_core::infra::templates::{AppCat, AppCategories, BaselineDefault, TemplateDirs};

fn dirs() -> TemplateDirs {
    TemplateDirs::factory_only(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../hub_templates"),
    )
}

#[test]
fn factory_templates_load() {
    let d = dirs();
    let b = BaselineDefault::load(&d).unwrap();
    for f in ["kpm", "iki_med", "iki_iqr", "bs_rate", "dwell_med"] {
        assert!(
            b.day.contains_key(f) && b.night.contains_key(f),
            "基线缺少 {f}"
        );
    }
    let apps = AppCategories::load(&d).unwrap();
    assert_eq!(apps.classify("WeChat.exe"), AppCat::Chat);
    assert_eq!(apps.classify("weixin.EXE"), AppCat::Chat);
    assert_eq!(apps.classify("Code.exe"), AppCat::Code);
    assert_eq!(apps.classify("unknown.exe"), AppCat::Other);
}

#[test]
fn banned_words_scenes() {
    let bw = BannedWords::load(&dirs()).unwrap();
    assert!(bw.find("检测到你的情绪不太好", Scene::Other).is_some());
    assert!(bw
        .find("想说的话不急着说完，慢慢来。", Scene::Other)
        .is_none());
    // 对话场景放行“药物”等词（FR-CHT-03），其他场景禁止
    assert!(bw.find("关于药物的问题建议问问医生", Scene::Chat).is_none());
    assert!(bw
        .find("关于药物的问题建议问问医生", Scene::Other)
        .is_some());
    // 豁免文案
    assert!(bw.find_in_copy("safety.card", "心理援助热线").is_none());
    assert!(bw.find_in_copy("tip.x", "你必须休息").is_some());
}
