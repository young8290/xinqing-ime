//! 危机词表 Rust 实现与 Python 参考实现（eval/tools/check_crisis.py）对拍，并检查 E-CRISIS 门槛。
//! 重新生成对拍文件：python3 eval/tools/check_crisis.py --golden eval/datasets/e_crisis.golden.json

use std::collections::HashMap;
use std::path::PathBuf;

use serde::Deserialize;
use xinqing_hub_core::domain::safety::{check_local, CrisisLexicon};
use xinqing_hub_core::infra::templates::TemplateDirs;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[derive(Deserialize)]
struct Sample {
    id: String,
    text: String,
    label: u8,
}

#[derive(Deserialize)]
struct Golden {
    score: f64,
    hit: bool,
}

#[test]
fn matches_python_reference_and_meets_thresholds() {
    let lex =
        CrisisLexicon::load(&TemplateDirs::factory_only(repo().join("hub_templates"))).unwrap();
    let data = std::fs::read_to_string(repo().join("eval/datasets/e_crisis.jsonl")).unwrap();
    let golden: HashMap<String, Golden> = serde_json::from_str(
        &std::fs::read_to_string(repo().join("eval/datasets/e_crisis.golden.json")).unwrap(),
    )
    .unwrap();

    let (mut pos, mut tp, mut neg, mut fp) = (0, 0, 0, 0);
    for line in data.lines().filter(|l| !l.trim().is_empty()) {
        let s: Sample = serde_json::from_str(line).unwrap();
        let v = check_local(&s.text, &lex, lex.threshold);
        let g = golden
            .get(&s.id)
            .unwrap_or_else(|| panic!("{} 不在对拍文件中，请重新生成", s.id));
        assert!(
            (v.score - g.score).abs() < 1e-6 && v.hit == g.hit,
            "{}：Rust {:.4}/{} ≠ Python {:.4}/{}",
            s.id,
            v.score,
            v.hit,
            g.score,
            g.hit
        );
        if s.label == 1 {
            pos += 1;
            tp += v.hit as u32;
        } else {
            neg += 1;
            fp += v.hit as u32;
        }
    }
    assert_eq!(tp, pos, "召回率必须 100%");
    assert!((fp as f64) / (neg as f64) < 0.10, "误报率必须 < 10%");
}

#[test]
fn dismiss_raises_threshold_and_method_is_flagged() {
    let lex =
        CrisisLexicon::load(&TemplateDirs::factory_only(repo().join("hub_templates"))).unwrap();
    let v = check_local("我真的不想活了", &lex, lex.threshold);
    assert!(v.hit);
    assert!(!check_local("我真的不想活了", &lex, lex.threshold_after_dismiss).hit);
    assert!(!check_local("今天作业多到要我命，累死了", &lex, lex.threshold).hit);
    assert!(check_local("我在想怎么跳楼", &lex, lex.threshold).has_method());
}

#[test]
fn traditional_and_fullwidth_text_is_normalized() {
    let lex =
        CrisisLexicon::load(&TemplateDirs::factory_only(repo().join("hub_templates"))).unwrap();
    // 繁体写法必须和简体一样命中（判定算法第 1 步“繁转简”）
    for (trad, simp) in [
        ("我想跳樓", "我想跳楼"),
        ("活著沒有意義", "活着没有意义"),
        ("我已經寫好遺書了", "我已经写好遗书了"),
    ] {
        let a = check_local(trad, &lex, lex.threshold);
        let b = check_local(simp, &lex, lex.threshold);
        assert!(a.hit, "{trad} 应命中");
        assert_eq!(a.score, b.score, "{trad} 与 {simp} 得分应相同");
    }
    // 全角空格和全角标点被去掉，不影响匹配
    assert!(check_local("不想　活了！", &lex, lex.threshold).hit);
}
