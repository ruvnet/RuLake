//! `rulake-mcp` binary — the MCP server entry point.
//!
//! v0.1 supports stdio only. Streamable HTTP, OAuth, and the full
//! capability set land in v0.2 per ADR-004 §Open questions.

use std::path::PathBuf;

use anyhow::Context;
use ruvector_rulake_mcp::{McpConfig, RuLakeMcpServer};

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let args = parse_args()?;

    let config = match args.config {
        Some(path) => McpConfig::from_path(&path)
            .with_context(|| format!("loading config: {}", path.display()))?,
        None => McpConfig::default(),
    };

    let server = RuLakeMcpServer::new(config)?;

    match args.transport {
        Transport::Stdio => server.serve_stdio().await,
    }
}

#[derive(Debug)]
struct Args {
    transport: Transport,
    config: Option<PathBuf>,
}

#[derive(Debug)]
enum Transport {
    Stdio,
}

fn parse_args() -> anyhow::Result<Args> {
    let mut transport = Transport::Stdio;
    let mut config = None;

    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "stdio" => transport = Transport::Stdio,
            "--config" => {
                config = Some(PathBuf::from(
                    it.next().context("--config expects a path")?,
                ));
            }
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            other => anyhow::bail!("unknown arg: {other}"),
        }
    }

    Ok(Args { transport, config })
}

fn print_help() {
    println!(
        "rulake-mcp — ruLake MCP server (ADR-004 v0.1)\n\
         \n\
         USAGE:\n    \
             rulake-mcp [stdio] [--config PATH]\n\
         \n\
         OPTIONS:\n    \
             stdio              Use stdio transport (default; v0.1 only).\n    \
             --config PATH      Load mcp.toml from PATH.\n    \
             -h, --help         Print this help.\n\
         \n\
         Streamable HTTP transport, OAuth/mTLS auth, replay protection,\n\
         and the full capability set (publish, admin) ship in v0.2.\n\
         See docs/adrs/sdk/ADR-004-rulake-mcp-server.md."
    );
}

fn init_tracing() {
    use tracing_subscriber::{EnvFilter, fmt, prelude::*};
    // Default level INFO; respect RUST_LOG when set. JSON to stderr so
    // a log shipper can pick it up without parsing.
    let _ = tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(fmt::layer().json().with_writer(std::io::stderr))
        .try_init();
}
