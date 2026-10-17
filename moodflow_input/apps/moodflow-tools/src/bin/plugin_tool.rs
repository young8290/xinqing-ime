//! 插件开发与市场审核工具（docs/design/plugin-system.md §7）。
//!
//! ```text
//! plugin_tool check <插件目录>            校验 manifest.toml；WASM 插件再实际加载一次，检查导入与导出
//! plugin_tool pack  <插件目录> <输出.mfpack>  打包，并打印可贴进 index.json 的 sha256 / size
//! plugin_tool try   <插件目录> <编码>        加载后按候选来源调一次，打印候选（开发调试用）
//! plugin_tool review <index.json> <包目录>   市场审核：包目录里放按 sha256 命名的 `<sha256>.mfpack`，
//!                                         逐个核对哈希、身份、清单、受保护文件，WASM 再实际加载
//! plugin_tool sign  <index.json>          用环境变量 MF_MARKET_SIGNING_KEY（32 字节种子的十六进制）签名，
//!                                         写出 index.json.sig；密钥不打印、不落盘
//! ```
//!
//! `check` 与市场审核 CI、客户端安装时用的是同一套校验（moodflow-plugin-api / moodflow-plugin-host）。

use std::io::Write;
use std::path::Path;

use ed25519_dalek::{Signer, SigningKey};
use moodflow_plugin_api::{Kind, Manifest, SettingsSchema};
use moodflow_plugin_host::{InputContext, Limits, MemoryHost, PluginInstance};
use sha2::{Digest, Sha256};

/// 校验用的宿主版本：产品版本唯一真源 docs/VERSION（编译时读入）。
const HOST_VERSION: &str = include_str!("../../../../../docs/VERSION");

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let r = match (args.first().map(String::as_str), args.get(1), args.get(2)) {
        (Some("check"), Some(dir), None) => check(Path::new(dir)).map(|_| ()),
        (Some("pack"), Some(dir), Some(out)) => pack(Path::new(dir), Path::new(out)),
        (Some("sign"), Some(index), None) => sign(Path::new(index)),
        (Some("try"), Some(dir), Some(code)) => try_candidates(Path::new(dir), code),
        (Some("review"), Some(index), Some(dir)) => review(Path::new(index), Path::new(dir)),
        _ => {
            eprintln!(
                "用法: plugin_tool check <目录> | pack <目录> <输出> | try <目录> <编码> | review <index.json> <包目录> | sign <index.json>"
            );
            std::process::exit(2);
        }
    };
    if let Err(e) = r {
        eprintln!("失败: {e}");
        std::process::exit(1);
    }
}

fn host_version() -> String {
    // 构建脚本经 MOODFLOW_APP_VERSION 注入的版本优先（与其他产物一致），否则用 docs/VERSION。
    std::env::var("MOODFLOW_APP_VERSION").unwrap_or_else(|_| HOST_VERSION.trim().to_owned())
}

fn check(dir: &Path) -> anyhow::Result<Manifest> {
    let text = std::fs::read_to_string(dir.join("manifest.toml"))?;
    let m = Manifest::from_toml(&text, &host_version())
        .map_err(|e| anyhow::anyhow!("manifest.toml: {e}"))?;
    if let Some(s) = &m.contributes.settings {
        let t = std::fs::read_to_string(dir.join(s))?;
        SettingsSchema::from_toml(&t).map_err(|e| anyhow::anyhow!("{s}: {e}"))?;
    }
    if let Kind::Wasm { entry } = &m.kind {
        let wasm = std::fs::read(dir.join(entry))?;
        PluginInstance::load(
            m.clone(),
            &wasm,
            Limits::default(),
            Box::new(MemoryHost::default()),
        )
        .map_err(|e| anyhow::anyhow!("{entry}: {e}"))?;
    }
    println!("通过：{} {}（{}）", m.id, m.version, m.name);
    let caps: Vec<String> = m
        .capabilities
        .list()
        .iter()
        .map(|c| format!("{c:?}"))
        .collect();
    println!(
        "申请的能力：{}",
        if caps.is_empty() {
            "无".into()
        } else {
            caps.join("、")
        }
    );
    Ok(m)
}

fn try_candidates(dir: &Path, code: &str) -> anyhow::Result<()> {
    let m = check(dir)?;
    let Kind::Wasm { entry } = &m.kind else {
        anyhow::bail!("内容包没有候选来源");
    };
    let wasm = std::fs::read(dir.join(entry))?;
    let mut p = PluginInstance::load(
        m.clone(),
        &wasm,
        Limits::default(),
        Box::new(MemoryHost::default()),
    )
    .map_err(|e| anyhow::anyhow!("{entry}: {e}"))?;
    let list = p
        .candidates(&InputContext::default(), code)
        .map_err(|e| anyhow::anyhow!("调用失败：{e}"))?;
    for c in list {
        println!("{}\t{}\t{}", c.weight, c.text, c.comment);
    }
    Ok(())
}

/// 市场审核（docs/design/plugin-system.md §7）：索引里每个未撤回的版本都必须有对应的包，且包能过客户端安装时的同一套检查。
fn review(index: &Path, dir: &Path) -> anyhow::Result<()> {
    let idx: moodflow_market::Index = serde_json::from_slice(&std::fs::read(index)?)?;
    let host = host_version();
    let mut failed = 0usize;
    let mut seen = std::collections::BTreeSet::new();
    for l in &idx.plugins {
        if !seen.insert(l.id.as_str()) {
            println!("✗ {} 在索引里出现了不止一次", l.id);
            failed += 1;
        }
        for r in &l.releases {
            if idx.is_revoked(&l.id, &r.version) {
                continue;
            }
            let label = format!("{} {}", l.id, r.version);
            let path = dir.join(format!("{}.mfpack", r.sha256));
            let checked = std::fs::read(&path)
                .map_err(|e| format!("读不到 {}：{e}", path.display()))
                .and_then(|bytes| {
                    moodflow_market::inspect_package(&bytes, &l.id, r, &host, None)
                        .map_err(|e| format!("{e:?}"))
                })
                .and_then(|plan| {
                    if let Kind::Wasm { entry } = &plan.manifest.kind {
                        let wasm = plan
                            .files
                            .iter()
                            .find(|(n, _)| n == entry)
                            .map(|(_, b)| b.as_slice())
                            .unwrap_or_default();
                        PluginInstance::load(
                            plan.manifest.clone(),
                            wasm,
                            Limits::default(),
                            Box::new(MemoryHost::default()),
                        )
                        .map_err(|e| format!("{entry}: {e}"))?;
                    }
                    Ok(plan.manifest)
                });
            match checked {
                Ok(m) => {
                    let caps: Vec<String> = m
                        .capabilities
                        .list()
                        .iter()
                        .map(|c| format!("{c:?}"))
                        .collect();
                    println!("✓ {label}  能力：{}", caps.join("、"));
                }
                Err(e) => {
                    println!("✗ {label}：{e}");
                    failed += 1;
                }
            }
        }
    }
    if failed > 0 {
        anyhow::bail!("{failed} 项未通过");
    }
    Ok(())
}

fn pack(dir: &Path, out: &Path) -> anyhow::Result<()> {
    let m = check(dir)?;
    let file = std::fs::File::create(out)?;
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let mut stack = vec![dir.to_path_buf()];
    let mut entries = Vec::new();
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d)? {
            let e = e?;
            let ft = e.file_type()?;
            if ft.is_symlink() {
                anyhow::bail!("不允许符号链接：{}", e.path().display());
            }
            if ft.is_dir() {
                stack.push(e.path());
            } else {
                let rel = e
                    .path()
                    .strip_prefix(dir)?
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                entries.push((rel, e.path()));
            }
        }
    }
    entries.sort();
    for (rel, path) in entries {
        zip.start_file(rel.as_str(), opts)?;
        zip.write_all(&std::fs::read(path)?)?;
    }
    zip.finish()?;
    let bytes = std::fs::read(out)?;
    let sha: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    println!(
        "{}",
        serde_json::json!({
            "version": m.version,
            "sha256": sha,
            "size": bytes.len(),
            "min_host_version": m.min_host_version,
            "platforms": m.platforms,
        })
    );
    Ok(())
}

fn sign(index: &Path) -> anyhow::Result<()> {
    let seed_hex = std::env::var("MF_MARKET_SIGNING_KEY")
        .map_err(|_| anyhow::anyhow!("没有设置 MF_MARKET_SIGNING_KEY"))?;
    let seed: [u8; 32] = (0..seed_hex.trim().len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&seed_hex.trim()[i..i + 2], 16))
        .collect::<Result<Vec<u8>, _>>()
        .ok()
        .and_then(|v| v.try_into().ok())
        .ok_or_else(|| anyhow::anyhow!("MF_MARKET_SIGNING_KEY 应为 64 位十六进制"))?;
    let key = SigningKey::from_bytes(&seed);
    let bytes = std::fs::read(index)?;
    // 签名前先确认它是合法索引，避免签出一份客户端读不了的东西。
    serde_json::from_slice::<moodflow_market::Index>(&bytes)?;
    let sig: String = key
        .sign(&bytes)
        .to_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let out = index.with_extension("json.sig");
    std::fs::write(&out, sig)?;
    let pk: String = key
        .verifying_key()
        .to_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    println!("已签名 → {}（公钥 {pk}）", out.display());
    Ok(())
}
