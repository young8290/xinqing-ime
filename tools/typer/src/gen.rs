//! 生成节奏脚本：`perf`（TC-PERF-01 匀速长打）与 `r1`（TC-STA-04 打错字召回 / 精确率）。
//! 同一种子总是生成同一份脚本，便于两次测量（心晴开 / 关）用完全相同的输入。

use anyhow::{Result, ensure};

use crate::r1;
use crate::script::{Key, Step};

/// 常用拼音音节（都 ≥ 2 个字母，保证每个词里有可以注入手误的非首字母位置）
const SYLLABLES: &[&str] = &[
    "ni", "hao", "wo", "men", "shi", "de", "zai", "you", "ta", "zhe", "ge", "bu", "le", "ren",
    "jiu", "dao", "shang", "xia", "lai", "qu", "shuo", "hui", "neng", "xiang", "kan", "zhi", "dou",
    "mei", "hen", "ye", "guo", "jia", "xue", "sheng", "gong", "zuo", "jin", "tian", "ming", "hou",
    "peng", "xi", "huan", "chi", "fan", "shui", "jiao", "mang", "lei", "kai", "xin", "qing", "wan",
    "shang", "zao", "dian", "hua", "yao", "kuai", "man", "tai", "zhen",
];

/// splitmix64：够用的可复现随机数，不为此引入 rand
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// [0, n)
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    /// [lo, hi]
    pub fn between(&mut self, lo: u32, hi: u32) -> u32 {
        lo + self.below((hi - lo + 1) as usize) as u32
    }

    fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// 截断在 [lo, hi] 的正态分布（Box–Muller）
    pub fn gauss(&mut self, mean: f64, sd: f64, lo: u32, hi: u32) -> u32 {
        let u1 = self.unit().max(f64::MIN_POSITIVE);
        let z = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * self.unit()).cos();
        ((mean + sd * z).round() as i64).clamp(lo as i64, hi as i64) as u32
    }

    pub fn pick<T: Copy>(&mut self, xs: &[T]) -> T {
        xs[self.below(xs.len())]
    }

    pub fn shuffle<T>(&mut self, xs: &mut [T]) {
        for i in (1..xs.len()).rev() {
            xs.swap(i, self.below(i + 1));
        }
    }
}

fn letters(word: &str) -> impl Iterator<Item = Key> + '_ {
    word.bytes().map(Key::Letter)
}

fn push(steps: &mut Vec<Step>, t: &mut u64, wait: u32, key: Key, label: Option<&str>) {
    *t += u64::from(wait);
    steps.push(Step {
        wait_ms: wait,
        key,
        label: label.map(str::to_owned),
    });
}

fn step(wait_ms: u32, key: Key) -> Step {
    Step {
        wait_ms,
        key,
        label: None,
    }
}

/// TC-PERF-01：`keys` 个键匀速发出，每秒 `rate` 键；内容是“拼音音节 + 空格上屏”循环。
pub fn perf(keys: usize, rate: f64, seed: u64) -> Result<Vec<Step>> {
    ensure!(keys > 0, "键数必须大于 0");
    ensure!(rate > 0.0 && rate <= 100.0, "速率须在 (0, 100] 键/秒之间");
    let mut rng = Rng::new(seed);
    let mut keys_out: Vec<Key> = Vec::with_capacity(keys);
    while keys_out.len() < keys {
        keys_out.extend(letters(rng.pick(SYLLABLES)));
        keys_out.push(Key::Space);
    }
    keys_out.truncate(keys);
    // 计划时刻按浮点累加再取整，速率除不尽时也不会累计漂移
    let interval = 1000.0 / rate;
    let mut prev = 0u64;
    Ok(keys_out
        .into_iter()
        .enumerate()
        .map(|(i, k)| {
            let t = (i as f64 * interval).round() as u64;
            let w = (t - prev) as u32;
            prev = t;
            step(w, k)
        })
        .collect())
}

/// 一次退格修改的样式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Edit {
    /// 相邻键打错 → 600 ms 内退格 → 800 ms 内改对：R1 应命中
    Typo,
    /// 打对了又删掉重打同一个字母（Y = X，不算相邻）
    Same,
    /// 相邻键打错，但过了 600 ms 才退格
    SlowBackspace,
    /// 相邻键打错、及时退格，但过了 800 ms 才改对
    LateRetype,
    /// 打成不相邻的键，及时退格改对
    Far,
}

impl Edit {
    const NEGATIVES: [Edit; 4] = [Edit::Same, Edit::SlowBackspace, Edit::LateRetype, Edit::Far];

    fn label(self) -> &'static str {
        match self {
            Edit::Typo => "typo",
            Edit::Same => "bs_same",
            Edit::SlowBackspace => "bs_slow",
            Edit::LateRetype => "bs_late",
            Edit::Far => "bs_far",
        }
    }
}

/// R1 两次命中之间至少隔这么久（Hub 1 秒去重，再留 100 ms 余量）
const TYPO_GAP_MS: u64 = r1::DEDUP_MS + 100;

/// TC-STA-04：`words` 个词里，按种子选定的位置注入 `typos` 处相邻键手误、`backspaces` 处非手误退格。
/// 改对的那一键带标注（`typo` 或 `bs_*`），评测时与 Hub 的 `typo` 事件对时间即可算精确率和召回率。
pub fn r1(words: usize, typos: usize, backspaces: usize, seed: u64) -> Result<Vec<Step>> {
    ensure!(words > 0, "词数必须大于 0");
    ensure!(
        typos + backspaces <= words,
        "每个词最多注入一处修改：手误数 + 退格数不能超过词数"
    );
    let mut rng = Rng::new(seed);
    let mut plan = vec![None; words];
    let mut idx: Vec<usize> = (0..words).collect();
    rng.shuffle(&mut idx);
    for (n, &w) in idx.iter().take(typos + backspaces).enumerate() {
        plan[w] = Some(if n < typos {
            Edit::Typo
        } else {
            Edit::NEGATIVES[(n - typos) % Edit::NEGATIVES.len()]
        });
    }

    let mut steps = Vec::new();
    let mut t = 0u64; // 当前计划时刻
    let mut last_typo: Option<u64> = None;
    for (w, edit) in plan.into_iter().enumerate() {
        let syllables = rng.between(1, 3) as usize;
        let word: Vec<u8> = (0..syllables)
            .flat_map(|_| rng.pick(SYLLABLES).bytes())
            .collect();
        let at = edit.map(|_| rng.between(1, word.len() as u32 - 1) as usize);
        for (i, &y) in word.iter().enumerate() {
            let mut iki = if i == 0 {
                if w == 0 {
                    0
                } else {
                    rng.gauss(260.0, 60.0, 120, 600)
                }
            } else {
                rng.gauss(160.0, 35.0, 70, 400)
            };
            let Some(e) = edit.filter(|_| at == Some(i)) else {
                push(&mut steps, &mut t, iki, Key::Letter(y), None);
                continue;
            };
            let (x, w1, w2) = match e {
                Edit::Typo => (
                    rng.pick(&r1::neighbors(y)),
                    rng.between(150, 400),
                    rng.between(150, 500),
                ),
                Edit::Same => (y, rng.between(150, 400), rng.between(150, 500)),
                Edit::SlowBackspace => (
                    rng.pick(&r1::neighbors(y)),
                    rng.between(700, 1000),
                    rng.between(150, 500),
                ),
                Edit::LateRetype => (
                    rng.pick(&r1::neighbors(y)),
                    rng.between(150, 400),
                    rng.between(900, 1200),
                ),
                Edit::Far => {
                    let far: Vec<u8> = (b'a'..=b'z')
                        .filter(|&c| c != y && !r1::adjacent(c, y))
                        .collect();
                    (rng.pick(&far), rng.between(150, 400), rng.between(150, 500))
                }
            };
            if e == Edit::Typo {
                // 离上一次手误太近会被 Hub 的 1 秒去重吞掉：把打错那一键往后推
                let hit_at = t + u64::from(iki + w1 + w2);
                if let Some(prev) = last_typo.filter(|&p| hit_at < p + TYPO_GAP_MS) {
                    iki += (prev + TYPO_GAP_MS - hit_at) as u32;
                }
            }
            push(&mut steps, &mut t, iki, Key::Letter(x), None);
            push(&mut steps, &mut t, w1, Key::Backspace, None);
            push(&mut steps, &mut t, w2, Key::Letter(y), Some(e.label()));
            if e == Edit::Typo {
                last_typo = Some(t);
            }
        }
        let space = rng.gauss(220.0, 40.0, 100, 500);
        push(&mut steps, &mut t, space, Key::Space, None);
    }
    Ok(steps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script;

    fn labelled(steps: &[Step], f: impl Fn(&str) -> bool) -> Vec<usize> {
        (0..steps.len())
            .filter(|&i| steps[i].label.as_deref().is_some_and(&f))
            .collect()
    }

    #[test]
    fn perf_is_uniform_and_exact() {
        let steps = perf(10_000, 8.0, 1).unwrap();
        assert_eq!(steps.len(), 10_000);
        assert_eq!(steps[0].wait_ms, 0);
        assert!(steps[1..].iter().all(|s| s.wait_ms == 125));
        // 除不尽的速率：总时长仍按速率算，不漂移
        let t = script::timeline(&perf(301, 7.0, 1).unwrap());
        assert_eq!(
            *t.last().unwrap(),
            (300.0_f64 * 1000.0 / 7.0).round() as u64
        );
    }

    #[test]
    fn r1_labels_match_the_rule_exactly() {
        for seed in 0..20 {
            let steps = r1(200, 40, 20, seed).unwrap();
            let times = script::timeline(&steps);
            let typos = labelled(&steps, |l| l == "typo");
            let negatives = labelled(&steps, |l| l.starts_with("bs_"));
            assert_eq!(typos.len(), 40);
            assert_eq!(negatives.len(), 20);
            assert_eq!(
                r1::hits(&steps, &times),
                typos,
                "种子 {seed}：R1 命中应恰好是标了 typo 的键"
            );
            assert_eq!(steps.iter().filter(|s| s.key == Key::Space).count(), 200);
        }
    }

    #[test]
    fn r1_is_reproducible_and_covers_every_negative() {
        let a = r1(200, 40, 20, 7).unwrap();
        assert_eq!(a, r1(200, 40, 20, 7).unwrap());
        assert_ne!(a, r1(200, 40, 20, 8).unwrap());
        for l in ["bs_same", "bs_slow", "bs_late", "bs_far"] {
            assert_eq!(labelled(&a, |x| x == l).len(), 5, "{l}");
        }
        assert!(r1(10, 8, 3, 1).is_err());
    }
}
