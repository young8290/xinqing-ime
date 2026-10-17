//! 示例插件：拼音出表情。演示 `sdk = 1` 的 ABI（见 moodflow-plugin-host 模块文档）：
//! 导出 `memory`、`mf_alloc`，以及声明了的扩展点 `mf_candidates`、`mf_cmd`；
//! 入参与返回都是 UTF-8 JSON，返回值高 32 位是指针、低 32 位是长度。
//!
//! 构建：`cargo build --release --target wasm32-unknown-unknown`，
//! 再把 `target/wasm32-unknown-unknown/release/pinyin_emoji.wasm` 复制为 `plugin.wasm`。

use serde::{Deserialize, Serialize};

const TABLE: &[(&str, &str, &str)] = &[
    ("xiao", "😀", "笑"),
    ("ku", "😭", "哭"),
    ("zan", "👍", "赞"),
    ("ai", "❤️", "爱"),
    ("huo", "🔥", "火"),
    ("hua", "🌸", "花"),
    ("mao", "🐱", "猫"),
    ("gou", "🐶", "狗"),
];

#[derive(Serialize)]
struct Candidate {
    text: &'static str,
    comment: &'static str,
    weight: i32,
}

#[derive(Deserialize)]
struct CandidatesIn {
    code: String,
}

#[derive(Deserialize)]
struct CmdIn {
    #[allow(dead_code)]
    name: String,
    args: Vec<String>,
}

#[derive(Serialize)]
struct CmdOut {
    value: String,
}

/// 宿主写入参数前向插件要一块内存。插件生命周期短、调用量小，直接泄漏，不做回收。
#[unsafe(no_mangle)]
pub extern "C" fn mf_alloc(len: i32) -> i32 {
    let buf = vec![0u8; len.max(0) as usize].into_boxed_slice();
    Box::leak(buf).as_mut_ptr() as i32
}

fn input<'a>(ptr: i32, len: i32) -> &'a [u8] {
    // SAFETY: 宿主保证 [ptr, ptr+len) 是 mf_alloc 刚分配并写好的内存。
    unsafe { core::slice::from_raw_parts(ptr as *const u8, len as usize) }
}

fn output(bytes: Vec<u8>) -> i64 {
    let len = bytes.len() as i64;
    let ptr = Box::leak(bytes.into_boxed_slice()).as_ptr() as i64;
    (ptr << 32) | len
}

#[unsafe(no_mangle)]
pub extern "C" fn mf_candidates(ptr: i32, len: i32) -> i64 {
    let code = serde_json::from_slice::<CandidatesIn>(input(ptr, len))
        .map(|i| i.code)
        .unwrap_or_default();
    let list: Vec<Candidate> = TABLE
        .iter()
        .filter(|(py, _, _)| !code.is_empty() && py.starts_with(code.as_str()))
        .map(|(py, emoji, comment)| Candidate {
            text: emoji,
            comment,
            // 全拼命中排在前缀命中之前；绝对值放低，不抢正常汉字候选的位置。
            weight: if *py == code { 20 } else { 10 },
        })
        .collect();
    output(serde_json::to_vec(&list).unwrap_or_else(|_| b"[]".to_vec()))
}

#[unsafe(no_mangle)]
pub extern "C" fn mf_cmd(ptr: i32, len: i32) -> i64 {
    let value = serde_json::from_slice::<CmdIn>(input(ptr, len))
        .ok()
        .and_then(|i| i.args.into_iter().next())
        .and_then(|name| {
            TABLE
                .iter()
                .find(|(py, _, comment)| *py == name || *comment == name)
                .map(|(_, emoji, _)| (*emoji).to_owned())
        })
        .unwrap_or_default();
    output(serde_json::to_vec(&CmdOut { value }).unwrap_or_default())
}
