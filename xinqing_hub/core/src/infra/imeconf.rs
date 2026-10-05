//! wind-rpc 客户端：读写输入法配置（10 第 4 节 IF-03、07 FR-SET-02、ADR 0016）。
//!
//! 只用 wind-rpc **已有的公开方法**（`config.schema` / `config.get` / `config.setItems`），不为心晴新增方法。
//! 协议是 4 字节大端长度前缀 + JSON 的请求-响应，每次连接一条请求（与清风 `wind-rpc/src/client.rs` 相同）。
//! 不依赖清风的 `wind-ipc` crate（两个 workspace 分开，ADR 0007）：帧格式在这里重写一份，
//! 协议版本号由测试 `protocol_version_matches_wind_ipc` 与清风那边核对。
//!
//! 调用是阻塞的（管道读写），外壳里放进 `spawn_blocking`。

use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// 与清风 `wind_ipc::rpc::PROTOCOL_VERSION` 一致
pub const PROTOCOL_VERSION: i32 = 1;
/// 与清风 `wind_ipc::rpc::MAX_MESSAGE_SIZE` 一致：整份配置也远小于这个数
pub const MAX_MESSAGE_SIZE: usize = 16 * 1024 * 1024;
/// 环境变量：指定管道或套接字路径（测试、非 Windows 联调）
pub const ENDPOINT_ENV: &str = "XQ_IME_RPC";

#[derive(Debug, thiserror::Error)]
pub enum ImeRpcError {
    /// 管道不存在（输入法核心没有运行）。界面提示“输入法还没启动”，不重试。
    #[error("连不上输入法核心：{0}")]
    Unavailable(io::Error),
    #[error("与输入法核心通信失败：{0}")]
    Io(#[from] io::Error),
    /// 核心返回了 error 字段（例如参数不对）
    #[error("输入法核心返回错误：{0}")]
    Remote(String),
    #[error("输入法核心的响应格式不对：{0}")]
    Protocol(String),
}

/// 配置项的类型（`config.schema` 的 `type`），决定界面用什么控件（07 FR-SET-02）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum ImeFieldKind {
    Bool,
    Int,
    Float,
    String,
    Enum,
    /// 字符串数组（schema 里写作 `string[]`）
    StringList,
    Map,
    Array,
    /// 认不出的类型：界面只读显示 JSON
    Other,
}

impl ImeFieldKind {
    fn from_wire(s: &str) -> Self {
        match s {
            "bool" => Self::Bool,
            "int" => Self::Int,
            "float" => Self::Float,
            "string" => Self::String,
            "enum" => Self::Enum,
            "string[]" => Self::StringList,
            "map" => Self::Map,
            "array" => Self::Array,
            _ => Self::Other,
        }
    }
}

/// `ime_schema` 的一项：已登记的输入法配置键。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ImeField {
    /// 点分键名，例如 `ui.candidate.layout`
    pub key: String,
    pub kind: ImeFieldKind,
    /// 只有 `enum` 有：合法取值
    pub options: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct WireField {
    key: String,
    #[serde(rename = "type")]
    ty: String,
    #[serde(default)]
    options: Option<Vec<String>>,
}

/// 要写入的一项。值是任意 JSON，合法性由核心按注册表校验。
/// 不导出给前端：specta 把 `serde_json::Value` 当递归类型内联会栈溢出，外壳另有 `ImeItemInput`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImeItem {
    pub key: String,
    pub value: Value,
}

/// 被核心跳过的一项（未登记、类型或取值不对……），`reason` 是核心给的原因。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ImeSkipped {
    pub key: String,
    pub reason: String,
}

/// `ime_config_set` 的返回：失败项不让整批失败（`config.setItems` 的语义）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ImeSetResult {
    /// 有改动需要重启输入法才生效（FR-SET-01 的重启横幅）
    pub needs_restart: bool,
    pub applied: u32,
    pub skipped: Vec<ImeSkipped>,
}

/// 核心回的是 camelCase（`needsRestart`）；单独一个只读结构，免得导出的类型分成序列化 / 反序列化两份。
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireSetResult {
    needs_restart: bool,
    applied: u32,
    #[serde(default)]
    skipped: Vec<ImeSkipped>,
}

/// 心晴版控制通道的默认地址（身份改造后的名字，见 `docs/xinqing/identity.md`）。开发版加 `_dev` 后缀。
pub fn default_endpoint(dev: bool) -> String {
    let suffix = if dev { "_dev" } else { "" };
    if cfg!(windows) {
        format!(r"\\.\pipe\xinqing_rpc_ctrl{suffix}")
    } else {
        // 与清风 `wind-rpc/src/server.rs` 的 `unix_endpoint` 同一规则（非 Windows 只用于开发）
        let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
        format!("{dir}/wind_input{suffix}_ctrl.sock")
    }
}

pub struct ImeRpc {
    endpoint: String,
    next_id: AtomicU64,
}

impl ImeRpc {
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            next_id: AtomicU64::new(1),
        }
    }

    /// `XQ_IME_RPC` 优先，否则用默认地址。
    pub fn from_env(dev: bool) -> Self {
        Self::new(std::env::var(ENDPOINT_ENV).unwrap_or_else(|_| default_endpoint(dev)))
    }

    /// 发一条请求、读一条响应。
    pub fn call(&self, method: &str, params: Value) -> Result<Value, ImeRpcError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let req =
            json!({ "version": PROTOCOL_VERSION, "id": id, "method": method, "params": params });
        let mut stream = connect(&self.endpoint).map_err(ImeRpcError::Unavailable)?;
        exchange(&mut stream, &req)
    }

    /// `config.schema`：全部已登记的配置键。
    pub fn schema(&self) -> Result<Vec<ImeField>, ImeRpcError> {
        #[derive(Deserialize)]
        struct Schema {
            fields: Vec<WireField>,
        }
        let s: Schema = parse(self.call("config.schema", json!({}))?)?;
        Ok(s.fields
            .into_iter()
            .map(|f| ImeField {
                kind: ImeFieldKind::from_wire(&f.ty),
                key: f.key,
                options: f.options,
            })
            .collect())
    }

    /// `config.get`：整份合并后的配置。
    pub fn config(&self) -> Result<Value, ImeRpcError> {
        self.call("config.get", json!({}))
    }

    /// `config.setItems`：逐项校验，失败项跳过并说明原因。
    pub fn set_items(&self, items: &[ImeItem]) -> Result<ImeSetResult, ImeRpcError> {
        let w: WireSetResult = parse(self.call("config.setItems", json!({ "items": items }))?)?;
        Ok(ImeSetResult {
            needs_restart: w.needs_restart,
            applied: w.applied,
            skipped: w.skipped,
        })
    }
}

fn parse<T: serde::de::DeserializeOwned>(v: Value) -> Result<T, ImeRpcError> {
    serde_json::from_value(v).map_err(|e| ImeRpcError::Protocol(e.to_string()))
}

#[cfg(windows)]
fn connect(endpoint: &str) -> io::Result<std::fs::File> {
    // 命名管道用普通文件接口打开即可，不需要 windows crate
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(endpoint)
}

#[cfg(unix)]
fn connect(endpoint: &str) -> io::Result<std::os::unix::net::UnixStream> {
    let s = std::os::unix::net::UnixStream::connect(endpoint)?;
    // 核心卡住时不让设置页一直转圈
    s.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
    Ok(s)
}

#[cfg(not(any(unix, windows)))]
fn connect(endpoint: &str) -> io::Result<std::fs::File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        format!("不支持的平台：{endpoint}"),
    ))
}

/// 写一帧请求，读一帧响应，取出 `result`（有 `error` 时报错）。
fn exchange<S: Read + Write>(stream: &mut S, req: &Value) -> Result<Value, ImeRpcError> {
    let payload = serde_json::to_vec(req).map_err(|e| ImeRpcError::Protocol(e.to_string()))?;
    let len = u32::try_from(payload.len()).map_err(|_| ImeRpcError::Protocol("请求过大".into()))?;
    stream.write_all(&len.to_be_bytes())?;
    stream.write_all(&payload)?;
    stream.flush()?;

    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_MESSAGE_SIZE {
        return Err(ImeRpcError::Protocol(format!("响应过大：{len} 字节")));
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf)?;

    #[derive(Deserialize)]
    struct Response {
        #[serde(default)]
        result: Option<Value>,
        #[serde(default)]
        error: Option<String>,
    }
    let resp: Response =
        serde_json::from_slice(&buf).map_err(|e| ImeRpcError::Protocol(e.to_string()))?;
    match resp.error {
        Some(e) => Err(ImeRpcError::Remote(e)),
        None => Ok(resp.result.unwrap_or(Value::Null)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 协议版本号与清风那边逐字核对：两边都改了才会对得上。
    #[test]
    fn protocol_version_matches_wind_ipc() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../wind_input/crates/wind-ipc/src/rpc.rs"
        );
        let src = std::fs::read_to_string(path).expect("找不到清风的 wind-ipc/src/rpc.rs");
        assert!(
            src.contains(&format!(
                "pub const PROTOCOL_VERSION: i32 = {PROTOCOL_VERSION};"
            )),
            "wind-ipc 的 PROTOCOL_VERSION 变了，心晴的 imeconf 要跟着改"
        );
        assert!(src.contains("pub const MAX_MESSAGE_SIZE: usize = 16 * 1024 * 1024;"));
    }

    #[test]
    fn default_endpoint_uses_xinqing_names() {
        if cfg!(windows) {
            assert_eq!(default_endpoint(false), r"\\.\pipe\xinqing_rpc_ctrl");
            assert_eq!(default_endpoint(true), r"\\.\pipe\xinqing_rpc_ctrl_dev");
        } else {
            assert!(default_endpoint(true).ends_with("wind_input_dev_ctrl.sock"));
        }
    }

    #[test]
    fn field_kinds_cover_every_schema_type() {
        let cases = [
            ("bool", ImeFieldKind::Bool),
            ("int", ImeFieldKind::Int),
            ("float", ImeFieldKind::Float),
            ("string", ImeFieldKind::String),
            ("enum", ImeFieldKind::Enum),
            ("string[]", ImeFieldKind::StringList),
            ("map", ImeFieldKind::Map),
            ("array", ImeFieldKind::Array),
            ("struct", ImeFieldKind::Other),
        ];
        for (wire, kind) in cases {
            assert_eq!(ImeFieldKind::from_wire(wire), kind, "{wire}");
        }
    }

    #[test]
    fn set_result_reads_core_camel_case_and_serializes_snake_case() {
        let w: WireSetResult = serde_json::from_value(json!({
            "needsRestart": true, "applied": 1,
            "skipped": [{ "key": "x.y", "reason": "未登记" }]
        }))
        .unwrap();
        assert!(w.needs_restart);
        let r = ImeSetResult {
            needs_restart: w.needs_restart,
            applied: w.applied,
            skipped: w.skipped,
        };
        let out = serde_json::to_value(&r).unwrap();
        assert_eq!(out["needs_restart"], json!(true));
        assert_eq!(out["skipped"][0]["key"], json!("x.y"));
    }

    #[cfg(unix)]
    mod wire {
        use super::*;
        use std::os::unix::net::UnixListener;

        /// 起一个只答一条请求的假核心：返回 (地址, 收到的请求)。
        fn fake_core(reply: Value) -> (String, std::thread::JoinHandle<Value>, tempdir::Dir) {
            let dir = tempdir::Dir::new();
            let path = dir.path.join("ctrl.sock");
            let listener = UnixListener::bind(&path).unwrap();
            let handle = std::thread::spawn(move || {
                let (mut s, _) = listener.accept().unwrap();
                let mut len = [0u8; 4];
                s.read_exact(&mut len).unwrap();
                let mut buf = vec![0u8; u32::from_be_bytes(len) as usize];
                s.read_exact(&mut buf).unwrap();
                let req: Value = serde_json::from_slice(&buf).unwrap();
                let mut resp = reply;
                resp["id"] = req["id"].clone();
                let out = serde_json::to_vec(&resp).unwrap();
                s.write_all(&(out.len() as u32).to_be_bytes()).unwrap();
                s.write_all(&out).unwrap();
                req
            });
            (path.to_string_lossy().into_owned(), handle, dir)
        }

        #[test]
        fn schema_round_trip() {
            let (ep, h, _dir) = fake_core(json!({ "result": { "fields": [
                { "key": "ui.candidate.layout", "type": "enum", "options": ["horizontal", "vertical"] },
                { "key": "ui.candidate.page_size", "type": "int" },
            ]}}));
            let fields = ImeRpc::new(ep).schema().unwrap();
            let req = h.join().unwrap();
            assert_eq!(req["method"], json!("config.schema"));
            assert_eq!(req["version"], json!(PROTOCOL_VERSION));
            assert_eq!(fields[0].kind, ImeFieldKind::Enum);
            assert_eq!(
                fields[0].options.as_deref(),
                Some(&["horizontal".to_string(), "vertical".to_string()][..])
            );
            assert_eq!(fields[1].kind, ImeFieldKind::Int);
            assert_eq!(fields[1].options, None);
        }

        #[test]
        fn set_items_sends_items_and_reports_skipped() {
            let (ep, h, _dir) = fake_core(json!({ "result": {
                "needsRestart": false, "applied": 1,
                "skipped": [{ "key": "ui.nope", "reason": "键 'ui.nope' 未登记" }]
            }}));
            let items = [
                ImeItem {
                    key: "ui.candidate.page_size".into(),
                    value: json!(7),
                },
                ImeItem {
                    key: "ui.nope".into(),
                    value: json!(true),
                },
            ];
            let r = ImeRpc::new(ep).set_items(&items).unwrap();
            let req = h.join().unwrap();
            assert_eq!(req["method"], json!("config.setItems"));
            assert_eq!(
                req["params"]["items"][0],
                json!({ "key": "ui.candidate.page_size", "value": 7 })
            );
            assert_eq!(r.applied, 1);
            assert_eq!(r.skipped[0].key, "ui.nope");
        }

        #[test]
        fn remote_error_is_reported() {
            let (ep, h, _dir) = fake_core(json!({ "error": "invalid_params: items missing" }));
            let err = ImeRpc::new(ep).config().unwrap_err();
            h.join().unwrap();
            assert!(matches!(err, ImeRpcError::Remote(m) if m.contains("items missing")));
        }

        #[test]
        fn missing_core_is_unavailable() {
            let dir = tempdir::Dir::new();
            let ep = dir.path.join("none.sock");
            let err = ImeRpc::new(ep.to_string_lossy()).schema().unwrap_err();
            assert!(matches!(err, ImeRpcError::Unavailable(_)));
        }

        /// 测试用的临时目录，用完删掉（不为此引入 tempfile 依赖）。
        mod tempdir {
            pub struct Dir {
                pub path: std::path::PathBuf,
            }
            impl Dir {
                pub fn new() -> Self {
                    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
                    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let path =
                        std::env::temp_dir().join(format!("xq-imeconf-{}-{n}", std::process::id()));
                    std::fs::create_dir_all(&path).unwrap();
                    Self { path }
                }
            }
            impl Drop for Dir {
                fn drop(&mut self) {
                    let _ = std::fs::remove_dir_all(&self.path);
                }
            }
        }
    }
}
