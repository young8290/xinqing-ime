use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::*;

const HOST: &str = "0.130.0";

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("mf-registry-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn manifest(id: &str, contributes: &str, caps: &str, platforms: &str) -> String {
    format!(
        r#"
id = "{id}"
name = "{id}"
version = "1.0.0"
sdk = 1
min_host_version = "0.1.0"
type = "wasm"
entry = "plugin.wasm"
platforms = [{platforms}]
license = "MIT"
[contributes]
{contributes}
[capabilities]
{caps}
"#
    )
}

/// 一页内存 + 递增分配器 + 若干函数体。
fn module(imports: &str, body: &str) -> Vec<u8> {
    wat::parse_str(format!(
        r#"(module
  {imports}
  (memory (export "memory") 1)
  (global $heap (mut i32) (i32.const 8192))
  (func (export "mf_alloc") (param $n i32) (result i32)
    (local $p i32)
    (local.set $p (global.get $heap))
    (global.set $heap (i32.add (global.get $heap) (local.get $n)))
    (local.get $p))
  {body}
)"#
    ))
    .unwrap()
}

/// 在 offset 处放常量 JSON，导出函数返回它。
fn returns(export: &str, offset: u32, json: &str) -> String {
    let escaped = json.replace('\\', "\\\\").replace('"', "\\\"");
    format!(
        r#"(data (i32.const {offset}) "{escaped}")
  (func (export "{export}") (param i32 i32) (result i64)
    (i64.or (i64.shl (i64.const {offset}) (i64.const 32)) (i64.const {len})))"#,
        len = json.len()
    )
}

fn install(dir: &Path, id: &str, manifest: &str, wasm: &[u8]) {
    let d = dir.join(id).join("1.0.0");
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("manifest.toml"), manifest).unwrap();
    std::fs::write(d.join("plugin.wasm"), wasm).unwrap();
}

fn opts<'a>(
    plugins: &'a Path,
    data: &'a Path,
    enabled: &'a [String],
    n: Notifier,
) -> LoadOptions<'a> {
    LoadOptions {
        plugins_dir: plugins,
        data_dir: data,
        enabled,
        host_version: HOST,
        platform: Platform::Windows,
        safe_mode: false,
        limits: Limits::default(),
        notifier: n,
    }
}

fn quiet() -> Notifier {
    Arc::new(|_, _| {})
}

const OK: InputContext = InputContext {
    private_field: false,
    secure_desktop: false,
    incognito: false,
    app_blocked: false,
};

fn ids(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn loads_only_enabled_supported_valid_plugins() {
    let root = tmp("load");
    let (plugins, data) = (root.join("plugins"), root.join("data"));
    let cand = returns("mf_candidates", 1024, "[]");
    install(
        &plugins,
        "dev.a",
        &manifest("dev.a", "candidate_provider = {}", "", r#""windows""#),
        &module("", &cand),
    );
    install(
        &plugins,
        "dev.off",
        &manifest("dev.off", "candidate_provider = {}", "", r#""windows""#),
        &module("", &cand),
    );
    install(
        &plugins,
        "dev.android",
        &manifest("dev.android", "candidate_provider = {}", "", r#""android""#),
        &module("", &cand),
    );
    install(
        &plugins,
        "dev.broken",
        &manifest("dev.broken", "candidate_provider = {}", "", r#""windows""#),
        b"not wasm",
    );
    let enabled = ids(&["dev.a", "dev.android", "dev.broken"]);
    let (reg, report) = Registry::load(&opts(&plugins, &data, &enabled, quiet()));
    assert_eq!(report.loaded, ids(&["dev.a"]));
    assert_eq!(report.not_enabled, ids(&["dev.off"]));
    let failed: Vec<&str> = report.failed.iter().map(|(id, _)| id.as_str()).collect();
    assert!(failed.contains(&"dev.android") && failed.contains(&"dev.broken"));
    assert_eq!(reg.status().len(), 1);

    let mut safe = opts(&plugins, &data, &enabled, quiet());
    safe.safe_mode = true;
    assert!(Registry::load(&safe).0.is_empty(), "安全模式不加载任何插件");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn candidates_follow_priority_and_respect_privacy() {
    let root = tmp("cand");
    let (plugins, data) = (root.join("plugins"), root.join("data"));
    install(
        &plugins,
        "dev.low",
        &manifest(
            "dev.low",
            "candidate_provider = { priority = 10 }",
            "",
            r#""windows""#,
        ),
        &module(
            "",
            &returns("mf_candidates", 1024, r#"[{"text":"低","weight":5}]"#),
        ),
    );
    install(
        &plugins,
        "dev.high",
        &manifest(
            "dev.high",
            "candidate_provider = { priority = 90 }",
            "",
            r#""windows""#,
        ),
        &module(
            "",
            &returns(
                "mf_candidates",
                1024,
                r#"[{"text":"高","weight":1},{"text":""}]"#,
            ),
        ),
    );
    let enabled = ids(&["dev.low", "dev.high"]);
    let (mut reg, _) = Registry::load(&opts(&plugins, &data, &enabled, quiet()));
    let got: Vec<(String, String)> = reg
        .candidates(&OK, "ga")
        .into_iter()
        .map(|(id, c)| (id, c.text))
        .collect();
    assert_eq!(
        got,
        vec![
            ("dev.high".into(), "高".into()),
            ("dev.low".into(), "低".into())
        ],
        "高优先级来源在前，空文本候选丢弃"
    );
    assert!(reg.candidates(&OK, "").is_empty());
    let private = InputContext {
        private_field: true,
        ..OK
    };
    assert!(reg.candidates(&private, "ga").is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn failing_plugin_is_skipped_and_eventually_disabled() {
    let root = tmp("fail");
    let (plugins, data) = (root.join("plugins"), root.join("data"));
    install(
        &plugins,
        "dev.loop",
        &manifest("dev.loop", "candidate_provider = {}", "", r#""windows""#),
        &module(
            "",
            r#"(func (export "mf_candidates") (param i32 i32) (result i64) (loop $l (br $l)) (i64.const 0))"#,
        ),
    );
    install(
        &plugins,
        "dev.ok",
        &manifest("dev.ok", "candidate_provider = {}", "", r#""windows""#),
        &module("", &returns("mf_candidates", 1024, r#"[{"text":"好"}]"#)),
    );
    let enabled = ids(&["dev.loop", "dev.ok"]);
    let (mut reg, _) = Registry::load(&opts(&plugins, &data, &enabled, quiet()));
    for _ in 0..3 {
        assert_eq!(reg.candidates(&OK, "h").len(), 1, "坏插件不拖累好插件");
    }
    let st = reg.status();
    assert!(st.iter().find(|s| s.id == "dev.loop").unwrap().disabled);
    assert!(reg.reenable("dev.loop"));
    assert!(
        !reg.status()
            .iter()
            .find(|s| s.id == "dev.loop")
            .unwrap()
            .disabled
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn transforms_chain_and_cmd_routes_by_name() {
    let root = tmp("chain");
    let (plugins, data) = (root.join("plugins"), root.join("data"));
    // 第一个把任何文本变成 "甲"，第二个（优先级低，后执行）变成 "乙"
    install(
        &plugins,
        "dev.t1",
        &manifest(
            "dev.t1",
            "commit_transform = { priority = 80 }",
            "",
            r#""windows""#,
        ),
        &module("", &returns("mf_transform", 1024, r#"{"text":"甲"}"#)),
    );
    install(
        &plugins,
        "dev.t2",
        &manifest(
            "dev.t2",
            "commit_transform = { priority = 20 }\ncmdbar_functions = [\"answer\"]",
            "",
            r#""windows""#,
        ),
        &module(
            "",
            &format!(
                "{}\n{}",
                returns("mf_transform", 1024, r#"{"text":"乙"}"#),
                returns("mf_cmd", 2048, r#"{"value":"42"}"#)
            ),
        ),
    );
    let enabled = ids(&["dev.t1", "dev.t2"]);
    let (mut reg, _) = Registry::load(&opts(&plugins, &data, &enabled, quiet()));
    assert!(reg.has_transforms());
    assert_eq!(reg.transform(&OK, "原文"), "乙");
    let private = InputContext {
        incognito: true,
        ..OK
    };
    assert_eq!(reg.transform(&private, "原文"), "原文");
    assert_eq!(reg.cmd_names(), ids(&["answer"]));
    assert_eq!(reg.cmd("answer", &[]), Some(Ok("42".into())));
    assert_eq!(reg.cmd("nope", &[]), None);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn storage_persists_per_plugin_and_notify_reaches_host() {
    let root = tmp("store");
    let (plugins, data) = (root.join("plugins"), root.join("data"));
    // transform：把入参存到 key "k"，再读回作为结果；顺带 notify 一条。
    let wasm = module(
        r#"(import "mf" "storage_get" (func $get (param i32 i32 i32 i32) (result i32)))
  (import "mf" "storage_set" (func $set (param i32 i32 i32 i32) (result i32)))
  (import "mf" "notify" (func $n (param i32 i32) (result i32)))"#,
        r#"(data (i32.const 512) "k")
  (data (i32.const 600) "hi")
  (func (export "mf_transform") (param $p i32) (param $n i32) (result i64)
    (local $len i32)
    (drop (call $set (i32.const 512) (i32.const 1) (local.get $p) (local.get $n)))
    (drop (call $n (i32.const 600) (i32.const 2)))
    (local.set $len (call $get (i32.const 512) (i32.const 1) (i32.const 32768) (i32.const 4096)))
    (i64.or (i64.shl (i64.const 32768) (i64.const 32)) (i64.extend_i32_u (local.get $len))))"#,
    );
    install(
        &plugins,
        "dev.store",
        &manifest(
            "dev.store",
            "commit_transform = {}",
            "storage = true\nnotify = true",
            r#""windows""#,
        ),
        &wasm,
    );
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    let notifier: Notifier =
        Arc::new(move |id, text| sink.lock().unwrap().push((id.to_owned(), text.to_owned())));
    let enabled = ids(&["dev.store"]);
    let (mut reg, _) = Registry::load(&opts(&plugins, &data, &enabled, notifier));
    assert_eq!(reg.transform(&OK, "你好"), "你好");
    assert_eq!(
        *seen.lock().unwrap(),
        vec![("dev.store".to_string(), "hi".to_string())]
    );
    let saved = std::fs::read_to_string(data.join("dev.store").join("storage.json")).unwrap();
    assert!(saved.contains("\"k\""), "存储落到插件自己的目录");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn file_host_enforces_key_and_size_limits() {
    let root = tmp("limits");
    let mut h = FileHost::new(root.join("p"), quiet());
    assert!(h.storage_set("p", "a", b"1"));
    assert!(!h.storage_set("p", "", b"1"));
    assert!(!h.storage_set("p", &"k".repeat(STORAGE_KEY_MAX + 1), b"1"));
    assert!(!h.storage_set("p", "bin", &[0xff, 0xfe]));
    assert!(!h.storage_set("p", "big", "x".repeat(STORAGE_LIMIT_BYTES).as_bytes()));
    let mut again = FileHost::new(root.join("p"), quiet());
    assert_eq!(again.storage_get("p", "a"), Some(b"1".to_vec()));
    assert_eq!(again.storage_get("p", "big"), None);
    let _ = std::fs::remove_dir_all(&root);
}
