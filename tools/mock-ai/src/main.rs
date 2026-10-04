//! mock-ai 命令行入口。接口与场景见 `lib.rs`。

use std::net::SocketAddr;

use anyhow::Result;
use clap::Parser;
use mock_ai::{Handle, Scenario, router};

#[derive(Parser, Debug)]
#[command(name = "mock-ai", about = "心晴 AI 接口模拟服务")]
struct Args {
    #[arg(long, default_value_t = 18080)]
    port: u16,
    #[arg(long, value_enum, default_value_t = Scenario::Normal)]
    scenario: Scenario,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let app = router(Handle::new(args.scenario));
    let addr = SocketAddr::from(([127, 0, 0, 1], args.port));
    eprintln!("mock-ai：http://{addr}（场景 {:?}）", args.scenario);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
