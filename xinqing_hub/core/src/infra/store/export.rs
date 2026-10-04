//! 导出我的数据（FR-DAT-03）：zip 包，每张表一个 JSON 文件、一份字段说明 `README.md` 和 `manifest.json`。
//!
//! - 所有表在同一个读事务里读出，导出的是同一时刻的快照；
//! - 先写 `<目标>.part`，完成后改名，失败时删掉临时文件，不会留下半个 zip；
//! - AI 生成的文字（对话里的晴晴回复、大模型写的暖心话和周信、AI 起草的日记）带 `ai_generated: true`，
//!   正文末尾附加“（内容由 AI 生成）”（DS-COPY-05、FR-CHT-04）；
//! - 只读数据库，AI 密钥在 `secrets.bin`（ADR 0012），不会进导出包；
//! - `manifest.json` 记下产品版本、数据库结构版本、导出时间、每个文件的行数和 SHA-256，供导入（FR-DAT-07）校验。

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local};
use rusqlite::types::ValueRef;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use zip::CompressionMethod;
use zip::write::{SimpleFileOptions, ZipWriter};

use super::{Db, StoreError};

/// 导出包格式名与版本。导入时据此识别；包内结构变化时 +1。
pub const FORMAT: &str = "xinqing-export";
pub const FORMAT_VERSION: u32 = 1;
pub const MANIFEST: &str = "manifest.json";
pub const README: &str = "README.md";

/// 与 `hub_templates/ui_copy.toml` 的 `chat.copy_suffix` 相同（测试核对）。导出不依赖模板加载，所以写在这里。
pub const AI_SUFFIX: &str = "（内容由 AI 生成）";

/// 表 → 数据清单编号与说明（09 第 1 节）。数据库里每张表都必须在这里登记，否则测试失败：
/// 新增迁移时顺手补一行，导出包的说明才不会缺。
pub const TABLES: &[(&str, &str, &str)] = &[
    (
        "window_features",
        "D-06",
        "打字节奏的数值特征，每个输入窗口一行；不含任何文字。保留 30 天",
    ),
    (
        "mood_state",
        "D-07",
        "每个窗口的状态判断结果与实际显示的状态。保留 30 天",
    ),
    (
        "daily_summary",
        "D-08",
        "每日汇总：输入时长、状态分布、休息与专注等。保留 1 年",
    ),
    (
        "baseline",
        "D-09",
        "个人基线（各特征的中位数与离散度），只有统计值",
    ),
    (
        "comfort_log",
        "D-10",
        "晴晴说过的暖心话与你的反馈。保留 90 天",
    ),
    ("chat_session", "D-11", "与晴晴的对话会话"),
    ("chat_message", "D-11", "对话消息"),
    ("schedule", "D-12", "日程（已忽略的只留去重用的哈希）"),
    ("diary", "D-13", "情绪日记"),
    ("memory", "D-14", "你让晴晴记住的事"),
    ("consent", "D-15", "每一项同意与撤回的记录"),
    ("reminder_log", "D-16", "休息提醒记录。保留 90 天"),
    (
        "feedback",
        "D-17",
        "对状态判断的“准 / 不准”反馈。保留 90 天",
    ),
    (
        "safety_log",
        "D-18",
        "安全事件，只有时间和触发通道，不含内容。保留 90 天",
    ),
    (
        "net_log",
        "D-20",
        "最近 200 次出网记录：接口、字段名、耗时，不含内容",
    ),
    ("settings", "—", "设置项"),
    ("self_report", "D-24", "你主动报告的心情与备注。保留 1 年"),
    ("todo", "D-25", "待办"),
    ("letter", "D-26", "晴晴的周信。保留 1 年"),
    (
        "rewrite_log",
        "D-28",
        "温柔改写记录，只有来源、风格和长度，不含文字。保留 90 天",
    ),
    ("focus_session", "D-29", "专注记录。保留 1 年"),
    ("collection", "D-30", "晴天收集与装扮"),
];

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("数据库读取失败：{0}")]
    Sql(#[from] rusqlite::Error),
    #[error("写导出文件失败：{0}")]
    Io(#[from] std::io::Error),
    #[error("写 zip 失败：{0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("序列化失败：{0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: String,
    pub format_version: u32,
    /// 导出时的心晴版本
    pub product_version: String,
    /// 数据库结构版本（`schema_version`）；导入时高于当前版本则拒绝
    pub schema_version: i64,
    /// 导出时刻，Unix 毫秒
    pub exported_at: i64,
    /// 同一时刻的本地时间（RFC 3339），给人看
    pub exported_at_local: String,
    /// 除 `manifest.json` 本身以外的每个文件
    pub files: Vec<FileEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    /// 包内路径，如 `tables/mood_state.json`
    pub path: String,
    /// 表文件的行数；`README.md` 为 `None`
    pub rows: Option<u64>,
    /// 文件内容的 SHA-256，小写十六进制
    pub sha256: String,
}

/// 把整个 Hub 数据库导出到 `dest`（已存在则覆盖）。`product_version` 写进 manifest。
pub fn export(
    db: &Db,
    dest: &Path,
    product_version: &str,
    now: DateTime<Local>,
) -> Result<Manifest, ExportError> {
    let part = part_path(dest);
    let result = write_zip(db, &part, product_version, now);
    match result {
        Ok(manifest) => {
            std::fs::rename(&part, dest)?;
            Ok(manifest)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}

fn part_path(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    dest.with_file_name(name)
}

fn write_zip(
    db: &Db,
    path: &Path,
    product_version: &str,
    now: DateTime<Local>,
) -> Result<Manifest, ExportError> {
    let conn = db.conn();
    // 读事务：所有表来自同一个快照（WAL 下不挡写线程）
    let tx = conn.unchecked_transaction()?;
    let schema_version = db.schema_version()?;
    let tables = table_names(conn)?;

    let mut zip = ZipWriter::new(BufWriter::new(File::create(path)?));
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let mut files = Vec::new();

    for table in &tables {
        let name = format!("tables/{table}.json");
        zip.start_file(name.as_str(), opts)?;
        let mut w = Hashing::new(&mut zip);
        let rows = write_table(conn, table, &mut w)?;
        files.push(FileEntry {
            path: name,
            rows: Some(rows),
            sha256: w.finish(),
        });
    }

    zip.start_file(README, opts)?;
    let mut w = Hashing::new(&mut zip);
    w.write_all(readme(conn, &tables)?.as_bytes())?;
    files.push(FileEntry {
        path: README.to_string(),
        rows: None,
        sha256: w.finish(),
    });

    let manifest = Manifest {
        format: FORMAT.to_string(),
        format_version: FORMAT_VERSION,
        product_version: product_version.to_string(),
        schema_version,
        exported_at: now.timestamp_millis(),
        exported_at_local: now.to_rfc3339(),
        files,
    };
    zip.start_file(MANIFEST, opts)?;
    serde_json::to_writer_pretty(&mut zip, &manifest)?;
    zip.write_all(b"\n")?;

    let mut out = zip.finish()?;
    out.flush()?;
    out.into_inner().map_err(|e| e.into_error())?.sync_all()?;
    drop(tx);
    Ok(manifest)
}

/// 数据库里的用户表（不含 `schema_version` 和 SQLite 内部表），按名字排序。
fn table_names(conn: &rusqlite::Connection) -> Result<Vec<String>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_master WHERE type = 'table'
           AND name NOT LIKE 'sqlite_%' AND name <> 'schema_version' ORDER BY name",
    )?;
    stmt.query_map([], |r| r.get(0))?.collect()
}

/// 写一张表：JSON 数组，一行一个对象（字段名即列名），便于用文本编辑器查看。
fn write_table(
    conn: &rusqlite::Connection,
    table: &str,
    w: &mut impl Write,
) -> Result<u64, ExportError> {
    // 表名来自 sqlite_master，不是外部输入；仍按标识符加引号
    let mut stmt = conn
        .prepare(&format!(
            "SELECT * FROM \"{}\" ORDER BY rowid",
            table.replace('"', "\"\"")
        ))
        .or_else(|_| {
            // 没有 rowid 的表（WITHOUT ROWID）按默认顺序
            conn.prepare(&format!("SELECT * FROM \"{}\"", table.replace('"', "\"\"")))
        })?;
    let cols: Vec<String> = stmt.column_names().iter().map(|c| c.to_string()).collect();
    let mut rows = stmt.query([])?;
    let mut n = 0u64;
    w.write_all(b"[")?;
    while let Some(row) = rows.next()? {
        let mut obj = Map::with_capacity(cols.len() + 1);
        for (i, c) in cols.iter().enumerate() {
            obj.insert(c.clone(), to_json(row.get_ref(i)?));
        }
        mark_ai(table, &mut obj);
        w.write_all(if n == 0 { b"\n" } else { b",\n" })?;
        serde_json::to_writer(&mut *w, &obj)?;
        n += 1;
    }
    w.write_all(if n == 0 { b"]\n" } else { b"\n]\n" })?;
    Ok(n)
}

fn to_json(v: ValueRef<'_>) -> Value {
    match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => Value::from(i),
        ValueRef::Real(f) => serde_json::Number::from_f64(f).map_or(Value::Null, Value::Number),
        ValueRef::Text(t) => Value::String(String::from_utf8_lossy(t).into_owned()),
        // 现有结构里没有二进制列；以后有了，用十六进制文本导出
        ValueRef::Blob(b) => Value::String(b.iter().map(|x| format!("{x:02x}")).collect()),
    }
}

/// AI 生成的文字：加 `ai_generated` 标记，正文末尾附“（内容由 AI 生成）”。
fn mark_ai(table: &str, row: &mut Map<String, Value>) {
    let text_col = match table {
        "chat_message" => "content",
        "comfort_log" => "text",
        "letter" => "content",
        "diary" => "content",
        _ => return,
    };
    let s = |k: &str| row.get(k).and_then(Value::as_str);
    let ai = match table {
        "chat_message" => row.get("ai_generated").and_then(Value::as_i64) == Some(1),
        "comfort_log" | "letter" => s("source") == Some("llm"),
        // ai_edited 是用户改过的 AI 草稿，仍按 AI 生成标注
        "diary" => matches!(s("source"), Some("ai_draft" | "ai_edited")),
        _ => false,
    };
    row.insert("ai_generated".into(), Value::Bool(ai));
    if ai && let Some(Value::String(text)) = row.get_mut(text_col) {
        text.push_str(AI_SUFFIX);
    }
}

/// 字段说明：每张表的数据清单编号、用途和列（名字、类型）。
fn readme(conn: &rusqlite::Connection, tables: &[String]) -> Result<String, rusqlite::Error> {
    let mut s = String::from(
        "# 心晴数据导出\n\n\
         这个文件夹是你在心晴里的全部本地数据，每张表一个 JSON 文件（`tables/`），每行一条记录。\n\n\
         - 时间戳（`ts`、`*_ts`）是 Unix 毫秒；日期是 `YYYY-MM-DD`。\n\
         - `ai_generated: true` 表示这段文字由 AI 生成，正文末尾也附了“（内容由 AI 生成）”。\n\
         - AI 服务的地址和密钥不在导出包里。\n\
         - `manifest.json` 记录了每个文件的 SHA-256，导入时用它检查文件是否完整、是否被改过。\n\
         - 心晴不是医疗产品，这些数据只反映打字节奏的变化，不代表任何健康结论。\n",
    );
    for t in tables {
        let (data, what) = TABLES
            .iter()
            .find(|(n, _, _)| n == t)
            .map_or(("—", "（未登记说明）"), |(_, d, w)| (*d, *w));
        s.push_str(&format!(
            "\n## {t}（{data}）\n\n{what}\n\n| 字段 | 类型 |\n|---|---|\n"
        ));
        let mut stmt = conn.prepare(&format!(
            "SELECT name, type FROM pragma_table_info('{}')",
            t.replace('\'', "''")
        ))?;
        let cols = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for c in cols {
            let (name, ty) = c?;
            s.push_str(&format!("| `{name}` | {ty} |\n"));
        }
        if matches!(
            t.as_str(),
            "chat_message" | "comfort_log" | "letter" | "diary"
        ) {
            s.push_str("| `ai_generated` | 导出时附加：是否由 AI 生成 |\n");
        }
    }
    Ok(s)
}

/// 写入时顺带算 SHA-256。
struct Hashing<W: Write> {
    inner: W,
    hash: Sha256,
}

impl<W: Write> Hashing<W> {
    fn new(inner: W) -> Self {
        Self {
            inner,
            hash: Sha256::new(),
        }
    }

    fn finish(self) -> String {
        self.hash
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }
}

impl<W: Write> Write for Hashing<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.hash.update(&buf[..n]);
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// 计算一段字节的 SHA-256（小写十六进制），导入校验与测试用。
pub fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use chrono::TimeZone;
    use rusqlite::params;

    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("xq-export-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn now() -> DateTime<Local> {
        Local
            .with_ymd_and_hms(2026, 10, 4, 21, 0, 0)
            .earliest()
            .unwrap()
    }

    fn seed(db: &Db) {
        let c = db.conn();
        c.execute(
            "INSERT INTO chat_session (id, title, created_ts) VALUES (1, '晚上聊聊', 1)",
            [],
        )
        .unwrap();
        c.execute(
            "INSERT INTO chat_message (session_id, role, content, ts, ai_generated) VALUES
             (1, 'user', '今天好累', 2, 0), (1, 'assistant', '辛苦啦，先歇一会儿', 3, 1)",
            [],
        )
        .unwrap();
        c.execute(
            "INSERT INTO comfort_log (ts, state, text, source) VALUES
             (4, 'tired', '慢慢来', 'llm'), (5, 'tired', '喝口水吧', 'template')",
            [],
        )
        .unwrap();
        c.execute(
            "INSERT INTO diary (date, content, source, created_ts, updated_ts) VALUES
             ('2026-10-04', '草稿', 'ai_draft', 6, 6), ('2026-10-04', '我自己写的', 'manual', 7, 7)",
            [],
        )
        .unwrap();
        c.execute(
            "INSERT INTO daily_summary (date, typing_min, avg_valence) VALUES ('2026-10-04', 42, 2.5)",
            [],
        )
        .unwrap();
        c.execute(
            "INSERT INTO consent (item, policy_ver, granted, ts) VALUES ('sense', 1, 1, ?1)",
            params![8],
        )
        .unwrap();
    }

    fn read_zip(path: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
        let mut z = zip::ZipArchive::new(File::open(path).unwrap()).unwrap();
        (0..z.len())
            .map(|i| {
                let mut f = z.by_index(i).unwrap();
                let mut buf = Vec::new();
                f.read_to_end(&mut buf).unwrap();
                (f.name().to_string(), buf)
            })
            .collect()
    }

    fn table(files: &std::collections::BTreeMap<String, Vec<u8>>, t: &str) -> Vec<Value> {
        serde_json::from_slice(&files[&format!("tables/{t}.json")]).unwrap()
    }

    #[test]
    fn exports_every_table_with_verifiable_manifest() {
        let db = Db::open_in_memory().unwrap();
        seed(&db);
        let dir = tmp("all");
        let dest = dir.join("心晴数据.zip");
        let m = export(&db, &dest, "0.1.0", now()).unwrap();
        assert!(!part_path(&dest).exists(), "临时文件应已改名");

        let files = read_zip(&dest);
        let manifest: Manifest = serde_json::from_slice(&files[MANIFEST]).unwrap();
        assert_eq!(manifest, m);
        assert_eq!(m.format, FORMAT);
        assert_eq!(m.schema_version, db.schema_version().unwrap());
        assert_eq!(m.exported_at, now().timestamp_millis());
        // 每张表一个文件，manifest 覆盖除自身外的全部文件，哈希与内容一致
        let tables = table_names(db.conn()).unwrap();
        assert_eq!(files.len(), tables.len() + 2);
        for e in &m.files {
            assert_eq!(sha256_hex(&files[&e.path]), e.sha256, "{}", e.path);
        }
        for t in &tables {
            let entry = m
                .files
                .iter()
                .find(|e| e.path == format!("tables/{t}.json"))
                .unwrap();
            assert_eq!(entry.rows, Some(table(&files, t).len() as u64), "{t}");
        }
        assert_eq!(table(&files, "daily_summary")[0]["avg_valence"], 2.5);
        assert_eq!(table(&files, "consent").len(), 1);
        let readme = String::from_utf8(files[README].clone()).unwrap();
        assert!(readme.contains("## mood_state（D-07）"));
        assert!(!readme.contains("未登记说明"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ai_text_is_marked() {
        let db = Db::open_in_memory().unwrap();
        seed(&db);
        let dir = tmp("ai");
        let dest = dir.join("out.zip");
        export(&db, &dest, "0.1.0", now()).unwrap();
        let files = read_zip(&dest);

        let msgs = table(&files, "chat_message");
        assert_eq!(msgs[0]["content"], "今天好累");
        assert_eq!(msgs[0]["ai_generated"], false);
        assert_eq!(msgs[1]["content"], format!("辛苦啦，先歇一会儿{AI_SUFFIX}"));
        assert_eq!(msgs[1]["ai_generated"], true);

        let comfort = table(&files, "comfort_log");
        assert_eq!(comfort[0]["text"], format!("慢慢来{AI_SUFFIX}"));
        assert_eq!(comfort[1]["text"], "喝口水吧");
        assert_eq!(comfort[1]["ai_generated"], false);

        let diary = table(&files, "diary");
        assert_eq!(diary[0]["ai_generated"], true);
        assert_eq!(diary[1]["content"], "我自己写的");
        // 其他表不加标记
        assert!(
            table(&files, "daily_summary")[0]
                .get("ai_generated")
                .is_none()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn every_table_is_described() {
        let db = Db::open_in_memory().unwrap();
        for t in table_names(db.conn()).unwrap() {
            assert!(
                TABLES.iter().any(|(n, _, _)| *n == t),
                "表 {t} 没有在 export::TABLES 登记说明"
            );
        }
    }

    #[test]
    fn failure_leaves_no_partial_file() {
        let db = Db::open_in_memory().unwrap();
        let dir = tmp("fail");
        let dest = dir.join("missing-dir").join("out.zip");
        assert!(export(&db, &dest, "0.1.0", now()).is_err());
        assert!(!dest.exists() && !part_path(&dest).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// TC-PERF-11：1 年数据量导出 ≤ 30 秒。按保留期估的上限：特征窗口和状态各 30 天 × 每天 2000 个，
    /// 对话 90 天 × 每天 60 条，自评与汇总各 1 年。手动运行：
    /// `cargo test --release -p xinqing-hub-core --lib export::tests::one_year_export_is_fast -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn one_year_export_is_fast() {
        let db = Db::open_in_memory().unwrap();
        let c = db.conn();
        let tx = c.unchecked_transaction().unwrap();
        let f = r#"{"n_keys":42,"kpm":180.5,"iki_med":210.0,"iki_iqr":90.0,"bs_rate":0.08,"pause_cnt":1,"session_min":35}"#;
        for i in 0..60_000i64 {
            tx.execute(
                "INSERT INTO window_features (start_ts, end_ts, app_cat, features_json) VALUES (?1, ?1, 'chat', ?2)",
                params![i * 1000, f],
            )
            .unwrap();
            tx.execute(
                "INSERT INTO mood_state (ts, window_id, state, shown_state, source) VALUES (?1, ?2, 'fluent', 'fluent', 'rule')",
                params![i * 1000, i + 1],
            )
            .unwrap();
        }
        tx.execute(
            "INSERT INTO chat_session (id, title, created_ts) VALUES (1, 't', 0)",
            [],
        )
        .unwrap();
        for i in 0..5_400i64 {
            tx.execute(
                "INSERT INTO chat_message (session_id, role, content, ts, ai_generated) VALUES (1, 'assistant', ?1, ?2, 1)",
                params!["今天也辛苦了，慢慢来，晴晴一直在这里陪着你。".repeat(4), i],
            )
            .unwrap();
        }
        for d in 0..365i64 {
            tx.execute(
                "INSERT INTO daily_summary (date, typing_min) VALUES (?1, 60)",
                params![format!("d{d:04}")],
            )
            .unwrap();
            tx.execute(
                "INSERT INTO self_report (ts, weather, note, source) VALUES (?1, 'cloudy', '有点累', 'user')",
                params![d],
            )
            .unwrap();
        }
        tx.commit().unwrap();
        let dir = tmp("perf");
        let t0 = std::time::Instant::now();
        let m = export(&db, &dir.join("out.zip"), "0.1.0", now()).unwrap();
        let took = t0.elapsed();
        let size = std::fs::metadata(dir.join("out.zip")).unwrap().len();
        println!("导出 {} 个文件、{size} 字节，用时 {took:?}", m.files.len());
        assert!(took.as_secs() < 30);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 导出包里的说明是给用户看的文字，同样要过禁用词表（DS-COPY-03）。
    #[test]
    fn readme_passes_banned_words() {
        use crate::domain::validate::{BannedWords, Scene};
        use crate::infra::templates::TemplateDirs;
        let bw = BannedWords::load(&TemplateDirs::factory_only(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../hub_templates"),
        ))
        .unwrap();
        let db = Db::open_in_memory().unwrap();
        let tables = table_names(db.conn()).unwrap();
        let text = readme(db.conn(), &tables).unwrap();
        assert_eq!(bw.find(&text, Scene::Other), None);
    }

    #[test]
    fn suffix_matches_ui_copy() {
        let copy = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../hub_templates/ui_copy.toml"),
        )
        .unwrap();
        assert!(copy.contains(&format!("copy_suffix = \"{AI_SUFFIX}\"")));
    }
}
