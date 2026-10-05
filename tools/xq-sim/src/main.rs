//! xq-sim：扮演核心服务，按 XQP 协议把 JSONL 脚本回放给 Hub（07 FR-DMO-01，17 第 4 节）。
//!
//! 传输：Windows 上默认创建命名管道 `\\.\pipe\xinqing_tap_dev`；任何平台都可以用 `--tcp`
//! （只监听 127.0.0.1，开发调试用）或 `--stdout`（只输出补好 seq 的 JSONL，不握手）。
//!
//! 握手严格按 10 第 2.3 节：等 Hub 的 hello → 回 hello → 等到 `cfg{collect:true}` 才开始回放。

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::Parser;
use serde::Deserialize;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use xqp::{Down, Up};

#[derive(Parser, Debug)]
#[command(name = "xq-sim", about = "心晴 XQP 事件模拟器")]
struct Args {
    /// 回放脚本（每行一条上行消息，ts 为相对毫秒，seq 可省略）
    #[arg(long)]
    script: Option<PathBuf>,
    /// 固定基线 TOML（格式同 baseline_default.toml）。由 Hub 读取：xq-sim 校验后打印设好
    /// `XQ_SIM_BASELINE` 的 Hub 启动命令（ADR 0021）
    #[arg(long)]
    baseline: Option<PathBuf>,
    /// 模拟的起始本地时刻 HH:MM。由 Hub 读取：打印设好 `XQ_SIM_START_AT` 的 Hub 启动命令
    /// （ADR 0021）。XQP 的 ts 是相对时间，Hub 以收到第一条消息的时刻为起点，平移 ts 没有作用
    #[arg(long)]
    start_at: Option<String>,
    /// 倍速（1 / 5 / 20 …）；0 表示不等待，尽快发完
    #[arg(long, default_value_t = 1.0)]
    speed: f64,
    /// 握手后先暂停，等标准输入的命令再回放（FR-DMO-01 暂停 / 单步）：
    /// `p` 暂停或继续，回车或 `n` 在暂停时发下一条，`c` 继续
    #[arg(long)]
    paused: bool,
    /// 命名管道名（仅 Windows）
    #[arg(long, default_value = xqp::PIPE_NAME_DEV)]
    pipe: String,
    /// 改用 TCP 监听本机端口（任意平台，开发调试用），如 127.0.0.1:18765
    #[arg(long)]
    tcp: Option<String>,
    /// 不握手，只把补好 seq 的消息逐行输出到标准输出
    #[arg(long)]
    stdout: bool,
    /// 只校验脚本（解析、ts 单调、不含 seq 冲突），不回放
    #[arg(long)]
    check: bool,
    /// 监听模式：以客户端身份连接核心（或另一个 xq-sim），把收到的上行消息去掉 text 后写入该文件（FR-DMO-02）
    #[arg(long)]
    listen: Option<PathBuf>,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let args = Args::parse();
    hub_env_hint(&args).await?;
    if let Some(out) = &args.listen {
        return listen(&args, out).await;
    }
    let script = args.script.as_ref().context("需要 --script")?;
    let msgs = load_script(script).await?;
    if args.check {
        println!("{}：{} 条消息，校验通过", script.display(), msgs.len());
        return Ok(());
    }
    if args.stdout {
        let mut seq = 0u32;
        for mut m in msgs {
            if !matches!(m, Up::Hello { .. }) {
                seq += 1;
                m.set_seq(seq);
            }
            println!("{}", serde_json::to_string(&m)?);
        }
        return Ok(());
    }
    if let Some(addr) = &args.tcp {
        if !addr.starts_with("127.0.0.1:") && !addr.starts_with("localhost:") {
            bail!("--tcp 只允许监听本机地址（C-PLT-07）");
        }
        let listener = tokio::net::TcpListener::bind(addr).await?;
        eprintln!("xq-sim：在 {addr} 等待 Hub 连接…");
        let (stream, peer) = listener.accept().await?;
        eprintln!("xq-sim：Hub 已连接 {peer}");
        let (r, w) = stream.into_split();
        return serve(r, w, msgs, args.speed, Gate::from_stdin(args.paused)).await;
    }
    serve_pipe(&args.pipe, msgs, args.speed, args.paused).await
}

#[cfg(windows)]
async fn serve_pipe(name: &str, msgs: Vec<Up>, speed: f64, paused: bool) -> Result<()> {
    use tokio::net::windows::named_pipe::ServerOptions;
    let path = format!(r"\\.\pipe\{name}");
    let server = ServerOptions::new()
        .first_pipe_instance(true)
        .reject_remote_clients(true)
        .create(&path)
        .with_context(|| format!("创建管道 {path} 失败（核心或另一个 xq-sim 是否正在运行？）"))?;
    eprintln!("xq-sim：在 {path} 等待 Hub 连接…");
    server.connect().await?;
    eprintln!("xq-sim：Hub 已连接");
    let (r, w) = tokio::io::split(server);
    serve(r, w, msgs, speed, Gate::from_stdin(paused)).await
}

#[cfg(not(windows))]
async fn serve_pipe(_name: &str, _msgs: Vec<Up>, _speed: f64, _paused: bool) -> Result<()> {
    bail!("命名管道只在 Windows 上可用；请改用 --tcp 127.0.0.1:<端口> 或 --stdout")
}

async fn load_script(path: &PathBuf) -> Result<Vec<Up>> {
    let text = tokio::fs::read_to_string(path)
        .await
        .with_context(|| format!("读取 {}", path.display()))?;
    let mut out = Vec::new();
    let mut last_ts = 0u64;
    for (i, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let m: Up = serde_json::from_str(line)
            .with_context(|| format!("{}:{} 不是合法的 XQP 上行消息", path.display(), i + 1))?;
        if let Some(ts) = m.ts() {
            if ts < last_ts {
                bail!("{}:{} ts 倒退（{ts} < {last_ts}）", path.display(), i + 1);
            }
            last_ts = ts;
        }
        out.push(m);
    }
    Ok(out)
}

#[derive(Debug, Deserialize)]
struct BaselineFile {
    version: u32,
    #[serde(default)]
    calibrated: bool,
    day: std::collections::HashMap<String, MedMad>,
    night: std::collections::HashMap<String, MedMad>,
}

#[derive(Debug, Deserialize)]
struct MedMad {
    med: f64,
    mad: f64,
}

async fn load_baseline(path: &PathBuf) -> Result<BaselineFile> {
    let text = tokio::fs::read_to_string(path)
        .await
        .with_context(|| format!("读取基线 {}", path.display()))?;
    let baseline: BaselineFile =
        toml::from_str(&text).with_context(|| format!("解析基线 {}", path.display()))?;
    if baseline.day.is_empty() || baseline.night.is_empty() {
        bail!("基线必须同时包含 day 和 night 特征");
    }
    if baseline
        .day
        .values()
        .chain(baseline.night.values())
        .any(|v| !v.med.is_finite() || !v.mad.is_finite() || v.mad < 0.0)
    {
        bail!("基线中的 med/mad 必须是有限数值，且 mad 不得为负");
    }
    Ok(baseline)
}

fn check_start_at(value: &str) -> Result<()> {
    let (hour, minute) = value.split_once(':').context("--start-at 格式应为 HH:MM")?;
    let ok = hour.len() == 2
        && minute.len() == 2
        && hour.parse::<u32>().is_ok_and(|h| h < 24)
        && minute.parse::<u32>().is_ok_and(|m| m < 60);
    if !ok {
        bail!("--start-at 应为 00:00 到 23:59");
    }
    Ok(())
}

/// `--baseline` / `--start-at` 要由 Hub 读取（ADR 0021）：校验参数，打印调试构建 Hub 的启动方式。
async fn hub_env_hint(args: &Args) -> Result<()> {
    let mut vars = Vec::new();
    if let Some(path) = &args.baseline {
        let baseline = load_baseline(path).await?;
        let abs = std::path::absolute(path).unwrap_or_else(|_| path.clone());
        eprintln!(
            "xq-sim：基线 v{}（{}）{}",
            baseline.version,
            if baseline.calibrated {
                "已校准"
            } else {
                "未校准"
            },
            abs.display()
        );
        vars.push(("XQ_SIM_BASELINE", abs.display().to_string()));
    }
    if let Some(start_at) = &args.start_at {
        check_start_at(start_at)?;
        vars.push(("XQ_SIM_START_AT", start_at.clone()));
    }
    if vars.is_empty() {
        return Ok(());
    }
    if let Some(addr) = &args.tcp {
        vars.push(("XQ_XQP_TCP", addr.clone()));
    }
    eprintln!("xq-sim：这些选项由 Hub 读取，请用调试构建的 Hub 并设置环境变量后启动：");
    let ps: Vec<String> = vars
        .iter()
        .map(|(k, v)| format!("$env:{k}='{v}'"))
        .collect();
    eprintln!("  PowerShell：{}; pnpm tauri dev", ps.join("; "));
    let sh: Vec<String> = vars.iter().map(|(k, v)| format!("{k}='{v}'")).collect();
    eprintln!("  sh：{} pnpm tauri dev", sh.join(" "));
    Ok(())
}

async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> Result<Option<Vec<u8>>> {
    let mut prefix = [0u8; 4];
    match r.read_exact(&mut prefix).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let len = xqp::frame_len(prefix)?;
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).await?;
    Ok(Some(body))
}

async fn send<W: AsyncWrite + Unpin, T: serde::Serialize>(w: &mut W, msg: &T) -> Result<()> {
    w.write_all(&xqp::encode(msg)?).await?;
    w.flush().await?;
    Ok(())
}

async fn serve<R, W>(mut r: R, mut w: W, msgs: Vec<Up>, speed: f64, mut gate: Gate) -> Result<()>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin,
{
    // 1. 等 Hub hello
    let body = read_frame(&mut r)
        .await?
        .context("Hub 未发送 hello 就断开了")?;
    match xqp::decode_down(&body)? {
        Down::Hello { v, hub_ver } if v == xqp::PROTOCOL_VERSION => {
            eprintln!("xq-sim：Hub hello v{v}（{hub_ver}）");
        }
        Down::Hello { v, .. } => {
            send(
                &mut w,
                &Up::Bye {
                    ts: None,
                    seq: None,
                    reason: xqp::ByeReason::Version,
                },
            )
            .await?;
            bail!("协议主版本不一致：Hub v{v}");
        }
        other => bail!("Hub 第一条消息应为 hello，收到 {other:?}"),
    }
    // 2. 回 hello（脚本自带 hello 时使用脚本里的）
    let hello = msgs
        .iter()
        .find(|m| matches!(m, Up::Hello { .. }))
        .cloned()
        .unwrap_or(Up::Hello {
            v: xqp::PROTOCOL_VERSION,
            ime_ver: "xq-sim".into(),
            session: "xq-sim".into(),
            caps: vec!["core_keys".into()],
        });
    send(&mut w, &hello).await?;
    // 3. 等 cfg{collect:true}
    loop {
        let body = read_frame(&mut r)
            .await?
            .context("Hub 未下发 cfg 就断开了")?;
        let down = xqp::decode_down(&body)?;
        eprintln!("xq-sim：收到 {}", String::from_utf8_lossy(&body));
        if let Down::Cfg { collect: true, .. } = down {
            break;
        }
    }
    // 4. 回放；同时继续读下行消息并打印（tip / mood 等）
    let reader = tokio::spawn(async move {
        while let Ok(Some(body)) = read_frame(&mut r).await {
            eprintln!("xq-sim ← {}", String::from_utf8_lossy(&body));
        }
    });
    let mut seq = 0u32;
    let mut last_ts = 0u64;
    let mut next_hb = 5_000u64;
    gate.announce();
    for mut m in msgs.into_iter().filter(|m| !matches!(m, Up::Hello { .. })) {
        let stepped = gate.wait(&mut w, last_ts, &mut seq).await?;
        let ts = m.ts().unwrap_or(last_ts);
        while ts >= next_hb {
            pace(next_hb.saturating_sub(last_ts), speed).await;
            last_ts = next_hb;
            seq += 1;
            send(
                &mut w,
                &Up::Hb {
                    ts: next_hb,
                    seq: Some(seq),
                    dropped: 0,
                    queue: 0,
                },
            )
            .await?;
            next_hb += 5_000;
        }
        // 单步时不按脚本间隔等待，按一下发一条
        if !stepped {
            pace(ts.saturating_sub(last_ts), speed).await;
        }
        last_ts = ts;
        seq += 1;
        m.set_seq(seq);
        send(&mut w, &m).await?;
    }
    eprintln!("xq-sim：回放完成，共 {seq} 条（含心跳）");
    send(
        &mut w,
        &Up::Bye {
            ts: Some(last_ts),
            seq: Some(seq + 1),
            reason: xqp::ByeReason::Shutdown,
        },
    )
    .await?;
    reader.abort();
    Ok(())
}

/// 回放控制命令（FR-DMO-01 暂停、单步），标准输入每行一个。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cmd {
    /// `p`：暂停或继续
    Toggle,
    /// 回车或 `n`：暂停时发下一条
    Step,
    /// `c`：继续
    Resume,
}

fn parse_cmd(line: &str) -> Option<Cmd> {
    match line.trim() {
        "p" | "pause" => Some(Cmd::Toggle),
        "" | "n" | "next" => Some(Cmd::Step),
        "c" | "continue" => Some(Cmd::Resume),
        _ => None,
    }
}

/// 暂停 / 单步的闸门。暂停期间每 5 秒发一次 ts 不前进的心跳：Hub 收到过心跳后 15 秒没有消息会断开重连。
struct Gate {
    paused: bool,
    steps: u32,
    cmds: tokio::sync::mpsc::UnboundedReceiver<Cmd>,
    closed: bool,
    /// 暂停期间的心跳间隔（与核心一致，5 秒）
    hb_every: Duration,
}

impl Gate {
    fn new(paused: bool, cmds: tokio::sync::mpsc::UnboundedReceiver<Cmd>) -> Self {
        Self {
            paused,
            steps: 0,
            cmds,
            closed: false,
            hb_every: Duration::from_secs(5),
        }
    }

    /// 从标准输入读命令。标准输入关着（例如在脚本里跑）就收不到命令，照常回放。
    fn from_stdin(paused: bool) -> Self {
        use tokio::io::AsyncBufReadExt;
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(async move {
            let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                match parse_cmd(&line) {
                    Some(c) => {
                        if tx.send(c).is_err() {
                            break;
                        }
                    }
                    None => eprintln!(
                        "xq-sim：不认识的命令 {line:?}（p 暂停/继续，回车或 n 单步，c 继续）"
                    ),
                }
            }
        });
        Self::new(paused, rx)
    }

    fn announce(&self) {
        eprintln!("xq-sim：回放控制：p 暂停/继续，暂停时回车或 n 发下一条，c 继续");
        if self.paused {
            eprintln!("xq-sim：已暂停");
        }
    }

    fn apply(&mut self, c: Cmd) {
        match c {
            Cmd::Toggle => self.paused = !self.paused,
            Cmd::Resume => self.paused = false,
            Cmd::Step if self.paused => self.steps += 1,
            Cmd::Step => {}
        }
        if c != Cmd::Step {
            eprintln!(
                "xq-sim：{}",
                if self.paused {
                    "已暂停"
                } else {
                    "继续回放"
                }
            );
        }
    }

    /// 发下一条脚本消息之前调用。暂停时等到继续或单步；返回这一条是不是单步放行的。
    async fn wait<W: AsyncWrite + Unpin>(
        &mut self,
        w: &mut W,
        last_ts: u64,
        seq: &mut u32,
    ) -> Result<bool> {
        loop {
            while let Ok(c) = self.cmds.try_recv() {
                self.apply(c);
            }
            if !self.paused {
                return Ok(false);
            }
            if self.steps > 0 {
                self.steps -= 1;
                return Ok(true);
            }
            if self.closed {
                // 标准输入已关，没人能再发命令：不卡死，接着回放
                eprintln!("xq-sim：标准输入已关闭，继续回放");
                self.paused = false;
                return Ok(false);
            }
            tokio::select! {
                c = self.cmds.recv() => match c {
                    Some(c) => self.apply(c),
                    None => self.closed = true,
                },
                _ = tokio::time::sleep(self.hb_every) => {
                    *seq += 1;
                    let hb = Up::Hb { ts: last_ts, seq: Some(*seq), dropped: 0, queue: 0 };
                    send(w, &hb).await?;
                }
            }
        }
    }
}

async fn pace(delta_ms: u64, speed: f64) {
    if speed > 0.0 && delta_ms > 0 {
        tokio::time::sleep(Duration::from_secs_f64(delta_ms as f64 / 1000.0 / speed)).await;
    }
}

/// 监听模式：以 Hub 身份连接并录制（FR-DMO-02：自动删除 text）。
async fn listen(args: &Args, out: &PathBuf) -> Result<()> {
    let addr = args
        .tcp
        .as_ref()
        .context("监听模式目前需要 --tcp 指定核心地址；Windows 管道客户端在接入核心后补充")?;
    let stream = tokio::net::TcpStream::connect(addr).await?;
    let (mut r, mut w) = stream.into_split();
    send(
        &mut w,
        &Down::Hello {
            v: xqp::PROTOCOL_VERSION,
            hub_ver: "xq-sim-listen".into(),
        },
    )
    .await?;
    send(
        &mut w,
        &Down::Cfg {
            collect: true,
            send_text: false,
            rewrite: false,
            app_blocklist: None,
            app_allowlist: None,
        },
    )
    .await?;
    let mut file = tokio::fs::File::create(out).await?;
    let mut n = 0usize;
    while let Some(body) = read_frame(&mut r).await? {
        let mut up = xqp::decode_up(&body)?;
        up.strip_text();
        file.write_all(serde_json::to_string(&up)?.as_bytes())
            .await?;
        file.write_all(b"\n").await?;
        n += 1;
        if matches!(up, Up::Bye { .. }) {
            break;
        }
    }
    eprintln!("xq-sim：已录制 {n} 条到 {}", out.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_control_commands() {
        assert_eq!(parse_cmd("p"), Some(Cmd::Toggle));
        assert_eq!(parse_cmd(""), Some(Cmd::Step));
        assert_eq!(parse_cmd(" n "), Some(Cmd::Step));
        assert_eq!(parse_cmd("c"), Some(Cmd::Resume));
        assert_eq!(parse_cmd("x"), None);
    }

    #[tokio::test]
    async fn paused_gate_steps_one_at_a_time_and_keeps_link_alive() {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let mut gate = Gate::new(true, rx);
        gate.hb_every = Duration::from_millis(10);
        let mut out = Vec::new();
        let mut seq = 7;
        // 暂停时没有命令：只发 ts 不前进的心跳
        let waited = tokio::time::timeout(
            Duration::from_millis(35),
            gate.wait(&mut out, 1_000, &mut seq),
        )
        .await;
        assert!(waited.is_err(), "暂停时不应放行");
        assert!(seq > 7);
        assert!(!out.is_empty());
        // 单步放行一条，并告诉调用方不要按脚本间隔等
        tx.send(Cmd::Step).unwrap();
        assert!(gate.wait(&mut out, 1_000, &mut seq).await.unwrap());
        // 继续之后不再拦
        tx.send(Cmd::Resume).unwrap();
        assert!(!gate.wait(&mut out, 1_000, &mut seq).await.unwrap());
        // 运行中单步不起作用
        tx.send(Cmd::Step).unwrap();
        assert!(!gate.wait(&mut out, 1_000, &mut seq).await.unwrap());
        assert_eq!(gate.steps, 0);
    }

    #[tokio::test]
    async fn closed_stdin_does_not_hang_a_paused_replay() {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        drop(tx);
        let mut gate = Gate::new(true, rx);
        let mut seq = 0;
        assert!(!gate.wait(&mut Vec::new(), 0, &mut seq).await.unwrap());
        assert!(!gate.paused);
    }
}
