use crate::practice::{Practice, bounded_edit_distance};

fn chars(s: &str) -> Vec<char> {
    s.chars().collect()
}

#[test]
fn bounded_edit_distance_matches_levenshtein_and_caps() {
    assert_eq!(
        bounded_edit_distance(&chars("kitten"), &chars("sitting"), 10),
        3
    );
    assert_eq!(
        bounded_edit_distance(&chars("今天天气好"), &chars("今天气很好"), 10),
        2
    );
    assert_eq!(bounded_edit_distance(&chars(""), &chars("abc"), 10), 3);
    assert_eq!(
        bounded_edit_distance(&chars("abcdef"), &chars("uvwxyz"), 3),
        3
    );
    assert_eq!(bounded_edit_distance(&chars("a"), &chars("abcdefgh"), 4), 4);
    assert_eq!(bounded_edit_distance(&chars("same"), &chars("same"), 1), 0);
}

#[test]
fn practice_scores_speed_and_accuracies() {
    let mut p = Practice::new("今天天气很好", 0);
    p.on_text(1_000, "今天");
    p.on_text(2_000, "今天田"); // 错字
    p.on_text(3_000, "今天"); // 删掉
    p.on_text(4_000, "今天天气很好");
    let s = p.finish(31_000);
    assert_eq!(s.elapsed_ms, 30_000, "从第一次输入算起");
    assert_eq!(s.distance, 0);
    assert_eq!(s.final_accuracy, 1.0);
    // 新增 2 + 1(错) + 4 = 7 个字，6 个同位一致
    assert!((s.process_accuracy - 6.0 / 7.0).abs() < 1e-9);
    assert_eq!(s.effective_cpm, 12.0);
}

#[test]
fn practice_final_accuracy_uses_edit_distance() {
    let mut p = Practice::new("abcd", 0);
    p.on_text(0, "abxd");
    let s = p.finish(60_000);
    assert_eq!(s.distance, 1);
    assert_eq!(s.final_accuracy, 0.75);
    assert_eq!(s.effective_cpm, 3.0);
    let empty = Practice::new("abc", 0).finish(1_000);
    assert_eq!(empty.final_accuracy, 0.0);
    assert_eq!(empty.process_accuracy, 1.0);
}
