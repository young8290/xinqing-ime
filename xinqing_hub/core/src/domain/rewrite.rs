//! 温柔改写的领域逻辑（C-09：06 FR-RWR-02/05/07、08 P-REWRITE 与第 5 节 V1/V4/V8）。
//!
//! 这里只有纯函数：可还原占位符 [`mask`]、提示词 [`RewritePrompt`]、结果解析 [`parse`] 与保真校验 [`check`]。
//! 什么时候调网关、回核心、写 `rewrite_log` 在 [`crate::rewrite`]。规格没写清的地方写在 docs/adr/0028。
//!
//! 隐私：原文和结果只在内存里，不落库、不写日志（FR-RWR-07）；手机号、证件号、卡号、邮箱、链接发出前换成
//! `[号码1]` 这类占位符，收到结果后由代码换回去，既不出网又保证原样保留。

use std::sync::OnceLock;

use regex::Regex;
use xqp::RewriteStyle;

use super::validate::{self, BannedWords, Scene};
use crate::infra::templates::{TemplateDirs, TemplateError};

/// 一次最多改写这么多字（FR-RWR-07；核心那边最近上屏 ≤ 200、剪贴板 ≤ 300）。
pub const MAX_INPUT_CHARS: usize = 300;
/// 最多给出几个候选（FR-RWR-03）。
pub const MAX_CANDIDATES: usize = 3;

/// 风格的中文名，填进 P-REWRITE 的 `{style}`（FR-RWR-02）。
pub fn style_label(s: RewriteStyle) -> &'static str {
    match s {
        RewriteStyle::Gentle => "更温和",
        RewriteStyle::Polite => "更礼貌得体",
        RewriteStyle::Concise => "更简洁",
        RewriteStyle::Structured => "更有条理",
    }
}

/// `rewrite_log` 里各列的取值，与 XQP 的写法相同（`recent`、`gentle`、`replaced` 等）。
pub fn source_str(s: xqp::RewriteSource) -> &'static str {
    match s {
        xqp::RewriteSource::Recent => "recent",
        xqp::RewriteSource::Clipboard => "clipboard",
        xqp::RewriteSource::Selection => "selection",
    }
}

pub fn style_str(s: RewriteStyle) -> &'static str {
    match s {
        RewriteStyle::Gentle => "gentle",
        RewriteStyle::Polite => "polite",
        RewriteStyle::Concise => "concise",
        RewriteStyle::Structured => "structured",
    }
}

pub fn outcome_str(o: xqp::RewriteOutcome) -> &'static str {
    match o {
        xqp::RewriteOutcome::Replaced => "replaced",
        xqp::RewriteOutcome::Inserted => "inserted",
        xqp::RewriteOutcome::Copied => "copied",
        xqp::RewriteOutcome::Cancelled => "cancelled",
        xqp::RewriteOutcome::Failed => "failed",
    }
}

struct Rules {
    url: Regex,
    email: Regex,
    digits: Regex,
    placeholder: Regex,
    mention: Regex,
    number: Regex,
}

fn rules() -> &'static Rules {
    static R: OnceLock<Rules> = OnceLock::new();
    R.get_or_init(|| Rules {
        url: Regex::new(r"(?i)(?:https?://|www\.)[^\s，。！？；、）】」]+").unwrap(),
        // 与网关脱敏一致：只认 ASCII，Unicode `\w` 会把紧挨着的汉字也当成用户名
        email: Regex::new(r"[A-Za-z0-9_.+-]+@[A-Za-z0-9-]+\.[A-Za-z0-9.]+").unwrap(),
        digits: Regex::new(r"[0-9]+[Xx]?").unwrap(),
        placeholder: Regex::new(r"\[(?:号码|证件|卡号|邮箱|链接)[0-9]+\]").unwrap(),
        mention: Regex::new(r"@[^\s@，。,.!?！？:：;；、]+").unwrap(),
        // 日期、时刻、金额、百分比、小数：数字连同中间的分隔符算一个整体
        number: Regex::new(r"[0-9]+(?:[.:：/\-][0-9]+)*%?").unwrap(),
    })
}

/// 打了占位符的原文，以及占位符到原文片段的对照表（只在内存里）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Masked {
    pub text: String,
    map: Vec<(String, String)>,
}

impl Masked {
    /// 把结果里的占位符换回原文片段。
    pub fn restore(&self, s: &str) -> String {
        let mut out = s.to_string();
        for (ph, orig) in &self.map {
            out = out.replace(ph.as_str(), orig);
        }
        out
    }
}

/// 发出前把手机号、证件号、卡号、邮箱、链接换成带序号的占位符（FR-RWR-07）。同一片段重复出现用同一个占位符。
/// 判定口径与网关脱敏 `redact` 一致；网关之后还会再脱敏一遍，打过占位符的文字它不会再改动。
pub fn mask(text: &str) -> Masked {
    let r = rules();
    let mut map: Vec<(String, String)> = Vec::new();
    let mut put = |kind: &str, orig: &str| -> String {
        if let Some((ph, _)) = map.iter().find(|(_, o)| o == orig) {
            return ph.clone();
        }
        let n = map
            .iter()
            .filter(|(ph, _)| ph.starts_with(&format!("[{kind}")))
            .count()
            + 1;
        let ph = format!("[{kind}{n}]");
        map.push((ph.clone(), orig.to_string()));
        ph
    };
    // 链接、邮箱先于数字：链接和邮箱里的数字不能被拆开
    let s = r
        .url
        .replace_all(text, |c: &regex::Captures| put("链接", &c[0]));
    let s = r
        .email
        .replace_all(&s, |c: &regex::Captures| put("邮箱", &c[0]));
    let s = r.digits.replace_all(&s, |c: &regex::Captures| {
        let run = &c[0];
        match digits_kind(run) {
            Some(kind) => put(kind, run),
            None => run.to_string(),
        }
    });
    Masked {
        text: s.into_owned(),
        map,
    }
}

/// 按完整数字串的长度分类（与 `infra::gateway::privacy` 相同）。
fn digits_kind(run: &str) -> Option<&'static str> {
    let has_x = run.ends_with(['X', 'x']);
    let n = run.len() - has_x as usize;
    match (n, has_x) {
        (17, true) | (18, false) => Some("证件"),
        (16..=19, false) => Some("卡号"),
        (11, false) if run.starts_with('1') && matches!(run.as_bytes()[1], b'3'..=b'9') => {
            Some("号码")
        }
        _ => None,
    }
}

/// 结果里必须原样出现的片段（V8）：占位符、数字（日期、时刻、金额、百分比都按数字串整体比较）。`@某人` 另见 [`mentions`]。
pub fn keep_tokens(masked: &str) -> Vec<String> {
    let r = rules();
    let mut out: Vec<String> = Vec::new();
    let mut add = |t: &str| {
        if !out.iter().any(|o| o == t) {
            out.push(t.to_string());
        }
    };
    for m in r.placeholder.find_iter(masked) {
        add(m.as_str());
    }
    let without_ph = r.placeholder.replace_all(masked, " ");
    // `@` 后面的名字里的数字（@小王2）随名字一起比较，不单独算
    let without_at = r.mention.replace_all(&without_ph, " ");
    for m in r.number.find_iter(&without_at) {
        add(m.as_str());
    }
    out
}

/// `@某人`（不含 `@`）。名字到空白或标点为止；中文里名字后面常紧跟正文（“找@小王也行”），所以只能取到下一个标点。
pub fn mentions(text: &str) -> Vec<&str> {
    rules()
        .mention
        .find_iter(text)
        .map(|m| &m.as_str()[1..])
        .collect()
}

/// 原文的每个 `@名字` 在候选里都要有一个与它互为前缀的 `@…`：“@小王也行” 与 “@小王” 算同一个人，“@小李” 不算。
fn mentions_kept(cand: &str, src: &str) -> bool {
    let got = mentions(cand);
    mentions(src)
        .iter()
        .all(|m| got.iter().any(|g| m.starts_with(g) || g.starts_with(m)))
}

/// V1：去掉代码块标记、取 JSON 里的 `candidates`，只留非空字符串，去掉首尾空白和重复。
pub fn parse(raw: &str) -> Option<Vec<String>> {
    let v = validate::extract_json(raw)?;
    let arr = v.get("candidates")?.as_array()?;
    let mut out: Vec<String> = Vec::new();
    for c in arr {
        let s = c.as_str()?.trim();
        if !s.is_empty() && !out.iter().any(|o| o == s) {
            out.push(s.to_string());
        }
    }
    Some(out)
}

/// 候选为什么被丢弃（只用于测试和计数，不含文字）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reject {
    /// 少了必须原样保留的片段（V8）
    Missing,
    /// 比原文长太多（FR-RWR-05 第 2 条：≤ 原文的 1.5 倍 + 10 个字）
    TooLong,
    /// 模型带进来的禁用词（V4，原文里本来就有的不算）
    Banned,
    /// 原文没有的辱骂词（V8）
    Abuse,
    /// 和原文一字不差
    Unchanged,
}

/// 候选的长度上限：原文的 1.5 倍 + 10 个字，按字符计。
pub fn max_len(src_chars: usize) -> usize {
    src_chars * 3 / 2 + 10
}

/// 对一条（仍带占位符的）候选做 V4、V8 与长度校验。`src` 是带占位符的原文。
pub fn check(cand: &str, src: &str, banned: &BannedWords) -> Result<(), Reject> {
    if cand == src {
        return Err(Reject::Unchanged);
    }
    if cand.chars().count() > max_len(src.chars().count()) {
        return Err(Reject::TooLong);
    }
    if keep_tokens(src).iter().any(|t| !cand.contains(t.as_str())) || !mentions_kept(cand, src) {
        return Err(Reject::Missing);
    }
    if banned.find_new_abuse(cand, src).is_some() {
        return Err(Reject::Abuse);
    }
    if banned.find_new(cand, src, Scene::Other).is_some() {
        return Err(Reject::Banned);
    }
    Ok(())
}

/// 校验并还原：留下通过的候选（最多 3 个），换回占位符。全部不通过时返回空。
pub fn accept(cands: &[String], masked: &Masked, banned: &BannedWords) -> Vec<String> {
    cands
        .iter()
        .filter(|c| check(c, &masked.text, banned).is_ok())
        .take(MAX_CANDIDATES)
        .map(|c| masked.restore(c))
        .collect()
}

/// P-REWRITE 模板（`hub_templates/prompts/rewrite.md`）。
#[derive(Debug, Clone)]
pub struct RewritePrompt {
    pub version: u32,
    body: String,
}

impl RewritePrompt {
    pub fn load(dirs: &TemplateDirs) -> Result<Self, TemplateError> {
        let (version, body) = crate::infra::templates::load_prompt(dirs, "prompts/rewrite.md", &["{style}", "{text}"])?;
        Ok(Self { version, body })
    }

    /// 首行 `<!-- version: N -->`；所有 `<!--` 开头的行是注释，不发给模型。
    pub fn parse(text: &str) -> Self {
        let p = super::comfort::ComfortPrompt::parse(text);
        Self {
            version: p.version,
            body: p.body().to_string(),
        }
    }

    /// 记录在 `CompleteRequest.prompt_ver`。
    pub fn ver(&self) -> String {
        format!("P-REWRITE v{}", self.version)
    }

    /// `masked` 是打过占位符的原文。
    pub fn render(&self, style: RewriteStyle, masked: &str) -> String {
        self.body
            .replace("{style}", style_label(style))
            .replace("{text}", masked)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn dirs() -> TemplateDirs {
        TemplateDirs::factory_only(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../hub_templates"),
        )
    }

    fn banned() -> BannedWords {
        BannedWords::load(&dirs()).unwrap()
    }

    #[test]
    fn masks_and_restores_private_fragments() {
        let src = "明天打13812345678或者发a.b@example.com，资料在 https://x.cn/p?id=1 里，13812345678 也行";
        let m = mask(src);
        assert_eq!(
            m.text,
            "明天打[号码1]或者发[邮箱1]，资料在 [链接1] 里，[号码1] 也行"
        );
        assert!(!m.text.contains("138"), "号码不出网");
        assert_eq!(m.restore(&m.text), src);
        assert_eq!(m.restore("请打[号码1]"), "请打13812345678");
        // 两个不同的号码各用一个序号
        let m = mask("13812345678 和 13900001111");
        assert_eq!(m.text, "[号码1] 和 [号码2]");
    }

    #[test]
    fn short_numbers_dates_and_amounts_stay_but_must_be_kept() {
        let m = mask("周五15:30前把1200元打给@小王，完成率80%");
        assert_eq!(m.text, "周五15:30前把1200元打给@小王，完成率80%");
        assert_eq!(keep_tokens(&m.text), ["15:30", "1200", "80%"]);
        assert_eq!(mentions(&m.text), ["小王"]);
        assert_eq!(mentions("找@小王也行，@Li 你看下"), ["小王也行", "Li"]);
        assert_eq!(keep_tokens("打[号码1]"), ["[号码1]"]);
    }

    #[test]
    fn parses_candidates_leniently() {
        assert_eq!(
            parse("```json\n{\"candidates\":[\" 甲 \",\"乙\",\"甲\",\"\"]}\n```"),
            Some(vec!["甲".to_string(), "乙".to_string()])
        );
        assert_eq!(parse("{\"candidates\":\"甲\"}"), None);
        assert_eq!(parse("不是 JSON"), None);
    }

    #[test]
    fn fidelity_checks() {
        let b = banned();
        let src = "你怎么还没交报告？周五15:30前必须给我，找@小王也行";
        let ok = "报告还没收到哦，麻烦周五15:30前发我，也可以找@小王";
        assert_eq!(check(ok, src, &b), Ok(()));
        assert_eq!(
            check("报告麻烦尽快发我，也可以找@小王", src, &b),
            Err(Reject::Missing),
            "时刻丢了"
        );
        assert_eq!(
            check("报告周五15:30前发我吧，可以找小王", src, &b),
            Err(Reject::Missing),
            "@ 丢了"
        );
        assert_eq!(
            check("报告还没收到，麻烦周五15:30前发我，或者找@小李", src, &b),
            Err(Reject::Missing),
            "换了人"
        );
        assert_eq!(check(src, src, &b), Err(Reject::Unchanged));
        let long = format!("{ok}{}", "，".repeat(max_len(src.chars().count())));
        assert_eq!(check(&long, src, &b), Err(Reject::TooLong));
        assert_eq!(
            check("你这个白痴，周五15:30前发我，找@小王", src, &b),
            Err(Reject::Abuse)
        );
        assert_eq!(
            check("别焦虑了，周五15:30前发我就行，找@小王", src, &b),
            Ok(()),
            "“焦虑”单独不在禁用词表里"
        );
        assert_eq!(
            check("你可能有点焦虑，周五15:30前发我，找@小王", src, &b),
            Err(Reject::Banned)
        );
    }

    #[test]
    fn users_own_words_are_not_held_against_them() {
        let b = banned();
        // 原文自己带了辱骂词，改写后保留不算“新增”
        let src = "你真是个白痴，又把文件删了";
        assert_eq!(check("你又把文件删了，真是个白痴", src, &b), Ok(()));
    }

    #[test]
    fn accept_drops_bad_ones_and_restores() {
        let b = banned();
        let m = mask("打13812345678确认一下周五的会");
        let cands = vec![
            "麻烦打[号码1]确认一下周五的会".to_string(),
            "确认一下周五的会".to_string(),
            "方便的话打[号码1]，确认下周五的会".to_string(),
        ];
        assert_eq!(
            accept(&cands, &m, &b),
            [
                "麻烦打13812345678确认一下周五的会",
                "方便的话打13812345678，确认下周五的会"
            ]
        );
    }

    #[test]
    fn log_columns_match_the_wire_names() {
        for s in [
            RewriteStyle::Gentle,
            RewriteStyle::Polite,
            RewriteStyle::Concise,
            RewriteStyle::Structured,
        ] {
            assert_eq!(serde_json::to_value(s).unwrap(), style_str(s));
        }
        for s in [
            xqp::RewriteSource::Recent,
            xqp::RewriteSource::Clipboard,
            xqp::RewriteSource::Selection,
        ] {
            assert_eq!(serde_json::to_value(s).unwrap(), source_str(s));
        }
        use xqp::RewriteOutcome::*;
        for o in [Replaced, Inserted, Copied, Cancelled, Failed] {
            assert_eq!(serde_json::to_value(o).unwrap(), outcome_str(o));
        }
    }

    #[test]
    fn prompt_renders_style_and_masked_text() {
        let p = RewritePrompt::load(&dirs()).unwrap();
        assert!(p.version >= 1);
        let s = p.render(RewriteStyle::Polite, "打[号码1]");
        assert!(s.contains("“更礼貌得体”"));
        assert!(s.ends_with("原文：打[号码1]"));
        assert!(!s.contains("<!--"));
    }
}
