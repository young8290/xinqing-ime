//! 打字测速：用户显式开始的练习。

use serde::{Deserialize, Serialize};

/// 一次练习的成绩。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Score {
    /// 用时（毫秒），从第一次输入算起；没有输入则从开始算。
    pub elapsed_ms: u64,
    /// 范文字数。
    pub target_chars: usize,
    /// 最终文本字数。
    pub typed_chars: usize,
    /// 与范文的编辑距离（超过上界时为上界）。
    pub distance: usize,
    /// 最终字准：`1 - distance / max(范文字数, 最终字数)`，0..=1。
    pub final_accuracy: f64,
    /// 过程正确率：新增的字中与范文同位一致的比例，0..=1；没有新增为 1。
    pub process_accuracy: f64,
    /// 有效速度（字/分钟）：`(范文字数 - distance)` 按用时折算，不低于 0。
    pub effective_cpm: f64,
}

/// 一次练习。调用方在练习期间暂停日常统计（`moodflow_store::StatCollector::pause`）。
#[derive(Debug, Clone)]
pub struct Practice {
    target: Vec<char>,
    started_ms: u64,
    first_input_ms: Option<u64>,
    last: Vec<char>,
    inserted: u64,
    inserted_correct: u64,
}

impl Practice {
    pub fn new(target: &str, started_ms: u64) -> Self {
        Self {
            target: target.chars().collect(),
            started_ms,
            first_input_ms: None,
            last: Vec::new(),
            inserted: 0,
            inserted_correct: 0,
        }
    }

    pub fn target(&self) -> String {
        self.target.iter().collect()
    }

    /// 输入框内容变了（上屏、删除、粘贴都走这里）。与上一次内容比较，求出新增的字；
    /// 每个新增字按它在新文本里的位置与范文同位比较，计入过程正确率。
    pub fn on_text(&mut self, now_ms: u64, text: &str) {
        let now: Vec<char> = text.chars().collect();
        if now == self.last {
            return;
        }
        self.first_input_ms.get_or_insert(now_ms);
        let prefix = self
            .last
            .iter()
            .zip(&now)
            .take_while(|(a, b)| a == b)
            .count();
        let max_suffix = self.last.len().min(now.len()) - prefix;
        let suffix = self
            .last
            .iter()
            .rev()
            .zip(now.iter().rev())
            .take(max_suffix)
            .take_while(|(a, b)| a == b)
            .count();
        for (i, c) in now.iter().enumerate().take(now.len() - suffix).skip(prefix) {
            self.inserted += 1;
            if self.target.get(i) == Some(c) {
                self.inserted_correct += 1;
            }
        }
        self.last = now;
    }

    /// 结束练习，按最后一次 [`Practice::on_text`] 的内容计分。
    pub fn finish(&self, now_ms: u64) -> Score {
        let start = self.first_input_ms.unwrap_or(self.started_ms);
        let elapsed_ms = now_ms.saturating_sub(start);
        let longest = self.target.len().max(self.last.len());
        let distance = bounded_edit_distance(&self.target, &self.last, longest);
        let final_accuracy = if longest == 0 {
            1.0
        } else {
            1.0 - distance as f64 / longest as f64
        };
        let process_accuracy = if self.inserted == 0 {
            1.0
        } else {
            self.inserted_correct as f64 / self.inserted as f64
        };
        let correct = self.target.len().saturating_sub(distance);
        let effective_cpm = if elapsed_ms == 0 {
            0.0
        } else {
            correct as f64 * 60_000.0 / elapsed_ms as f64
        };
        Score {
            elapsed_ms,
            target_chars: self.target.len(),
            typed_chars: self.last.len(),
            distance,
            final_accuracy,
            process_accuracy,
            effective_cpm,
        }
    }
}

/// 有界编辑距离（Levenshtein，按字比较）：只算对角线两侧 `bound` 宽的带，结果超过 `bound` 时返回 `bound`。
/// 练习文本可长达数千字，带宽限制让最坏情况从平方降到 `O(n·bound)`。
pub fn bounded_edit_distance(a: &[char], b: &[char], bound: usize) -> usize {
    if a.len().abs_diff(b.len()) >= bound {
        return bound;
    }
    let (n, m) = (a.len(), b.len());
    let inf = bound;
    let mut prev = vec![inf; m + 1];
    let mut cur = vec![inf; m + 1];
    for (j, p) in prev.iter_mut().enumerate().take(m.min(bound) + 1) {
        *p = j;
    }
    for i in 1..=n {
        let lo = i.saturating_sub(bound).max(1);
        let hi = (i + bound).min(m);
        cur.fill(inf);
        if i <= bound {
            cur[0] = i;
        }
        let mut row_min = cur[0];
        for j in lo..=hi {
            let sub = prev[j - 1] + usize::from(a[i - 1] != b[j - 1]);
            let del = prev[j] + 1;
            let ins = cur[j - 1] + 1;
            cur[j] = sub.min(del).min(ins).min(inf);
            row_min = row_min.min(cur[j]);
        }
        if row_min >= bound {
            return bound;
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[m].min(bound)
}
