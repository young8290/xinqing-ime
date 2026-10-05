//! 出网文本脱敏（FR-AIG-05 第 2 条）：手机号、证件号、卡号、邮箱、网址查询参数。
//! 邮箱规则限定 ASCII 字符：Unicode `\w` 会把紧挨着的汉字也当成用户名。

use std::sync::OnceLock;

use regex::{Captures, Regex};

struct Rules {
    digits: Regex,
    email: Regex,
    query: Regex,
}

fn rules() -> &'static Rules {
    static R: OnceLock<Rules> = OnceLock::new();
    R.get_or_init(|| Rules {
        digits: Regex::new(r"[0-9]+[Xx]?").unwrap(),
        email: Regex::new(r"[A-Za-z0-9_.+-]+@[A-Za-z0-9-]+\.[A-Za-z0-9.]+").unwrap(),
        // 只认紧跟在域名或路径后面的 `?`：单独一个半角问号（“你在哪?我等你”）不是查询参数
        query: Regex::new(
            r"([A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}(?:/[^\s?]*)?)\?[^\s]+",
        )
        .unwrap(),
    })
}

/// 按“完整数字串”的长度分类，避免长串的一部分被短规则命中
/// （Rust 正则没有环视，汉字与数字之间也没有 `\b` 边界）。
fn classify_digits(run: &str) -> Option<&'static str> {
    let has_x = run.ends_with(['X', 'x']);
    let n = run.len() - has_x as usize;
    match (n, has_x) {
        (17, true) | (18, false) => Some("[证件号]"),
        (16..=19, false) => Some("[卡号]"),
        (11, false) if run.starts_with('1') && matches!(run.as_bytes()[1], b'3'..=b'9') => {
            Some("[手机号]")
        }
        _ => None,
    }
}

pub fn redact(text: &str) -> String {
    let r = rules();
    // 邮箱先于数字处理：用户名是手机号的邮箱（13812345678@qq.com）要整体替换
    let s = r.email.replace_all(text, "[邮箱]");
    let s = r.digits.replace_all(&s, |c: &Captures| {
        let run = &c[0];
        classify_digits(run).map_or_else(|| run.to_string(), str::to_string)
    });
    r.query.replace_all(&s, "$1?[已省略]").into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_all_types() {
        assert_eq!(redact("电话13812345678"), "电话[手机号]");
        assert_eq!(redact("身份证11010519491231002X"), "身份证[证件号]");
        assert_eq!(redact("卡号6222021234567890123"), "卡号[卡号]");
        assert_eq!(redact("邮箱a.b@example.com"), "邮箱[邮箱]");
        assert_eq!(redact("发到13812345678@qq.com"), "发到[邮箱]");
        assert_eq!(
            redact("看 https://x.cn/p?id=1&t=2 这个"),
            "看 https://x.cn/p?[已省略] 这个"
        );
        assert_eq!(
            redact("x.cn/p?id=1 和 example.com?a=b"),
            "x.cn/p?[已省略] 和 example.com?[已省略]"
        );
        assert_eq!(
            redact("你在哪?我等你"),
            "你在哪?我等你",
            "半角问号不是查询参数"
        );
        assert_eq!(redact("ok?好的"), "ok?好的");
        assert_eq!(redact("版本1.2?可以吗"), "版本1.2?可以吗");
        assert_eq!(redact("周五下午三点开会"), "周五下午三点开会");
        assert_eq!(redact("12345678901"), "12345678901", "不是 1[3-9] 开头");
        assert_eq!(redact("10月9日15点"), "10月9日15点");
    }
}
