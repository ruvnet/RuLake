//! `ruqu-mcp` binary — MCP companion server entry point for the ruQu
//! v2 substrate (ADR-008 §6, v0.1).
//!
//! v0.1 wires the Streamable HTTP transport on top of the v0.0.1 tool
//! surface. No tool semantics change: `ruqu_simulate`, `ruqu_verify`,
//! `ruqu_replay`, `ruqu_optimize` (stub), `ruqu_qec_schedule` (stub)
//! are all served unchanged whether the binary is launched in `stdio`
//! or `http` mode. Mirrors `mcp-rvdna`'s CLI surface so an operator
//! script can drive both servers with the same `--bind` /
//! `--audit-file` / `--insecure-allow-no-auth` flags.
//!
//! Capability tiers are plumbed but not enforced in v0.0.1 / v0.1 —
//! ADR-008 §6 Decision 4 reserves the `simulate` / `hardware` /
//! `verify` capability set for v0.2 once mcp-server's `policy` module
//! agrees. The `--capabilities` flag is accepted (so smoke scripts
//! that mirror the rvdna shape don't break) and logged.

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::Context;

use ruvector_rulake_mcp_ruqu::{
    AllowBearerOnPublic, AuditSink, AuthMode, InsecureAllowNoAuth, RuquMcpServer,
};

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let args = parse_args()?;

    if let Some(csv) = &args.capabilities {
        tracing::info!(
            requested = %csv,
            "ruqu-mcp: --capabilities accepted but unused in v0.1 \
             (ADR-008 §6 Decision 4 — capability tiers ship in v0.2)"
        );
    }

    let audit = match &args.audit_file {
        Some(p) => {
            let sink = AuditSink::open_file(p)
                .with_context(|| format!("opening ruqu audit file: {}", p.display()))?;
            tracing::info!(path = %p.display(), "ruqu audit -> JSONL file");
            sink
        }
        None => {
            tracing::info!("ruqu audit -> stderr (no --audit-file set)");
            AuditSink::stderr()
        }
    };

    let server = RuquMcpServer::new(args.backend_id.clone()).with_audit(audit);
    tracing::info!(backend_id = %args.backend_id, "starting ruqu-mcp");

    match args.transport {
        Transport::Stdio => server.serve_stdio().await,
        Transport::Http(http) => {
            ruvector_rulake_mcp_ruqu::http::serve(
                server,
                http.bind,
                AuthMode::None,
                AllowBearerOnPublic(false),
                InsecureAllowNoAuth(http.insecure_allow_no_auth),
            )
            .await
        }
    }
}

#[derive(Debug)]
struct Args {
    transport: Transport,
    capabilities: Option<String>,
    audit_file: Option<PathBuf>,
    backend_id: String,
}

#[derive(Debug)]
enum Transport {
    Stdio,
    Http(HttpArgs),
}

#[derive(Debug)]
struct HttpArgs {
    bind: SocketAddr,
    insecure_allow_no_auth: bool,
}

fn parse_args() -> anyhow::Result<Args> {
    let mut transport_kind: Option<&'static str> = None;
    let mut capabilities: Option<String> = None;
    let mut audit_file: Option<PathBuf> = None;
    let mut http_bind: Option<SocketAddr> = None;
    let mut http_insecure_allow_no_auth = false;
    let mut backend_id: String = "ruqu-sv".to_string();

    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "stdio" => transport_kind = Some("stdio"),
            "http" => transport_kind = Some("http"),
            "--capabilities" => {
                capabilities = Some(it.next().context("--capabilities expects CSV")?);
            }
            "--audit-file" => {
                audit_file = Some(PathBuf::from(
                    it.next().context("--audit-file expects PATH")?,
                ));
            }
            "--backend-id" => {
                backend_id = it.next().context("--backend-id expects STRING")?;
            }
            "--bind" => {
                let s = it.next().context("--bind expects ADDR:PORT")?;
                http_bind = Some(s.parse().with_context(|| format!("--bind {s:?}"))?);
            }
            "--insecure-allow-no-auth" => http_insecure_allow_no_auth = true,
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            other => anyhow::bail!("unknown arg: {other}"),
        }
    }

    let transport = match transport_kind.unwrap_or("stdio") {
        "stdio" => Transport::Stdio,
        "http" => {
            let bind = http_bind.unwrap_or_else(|| "127.0.0.1:7442".parse().unwrap());
            Transport::Http(HttpArgs {
                bind,
                insecure_allow_no_auth: http_insecure_allow_no_auth,
            })
        }
        _ => unreachable!(),
    };

    Ok(Args {
        transport,
        capabilities,
        audit_file,
        backend_id,
    })
}

fn print_help() {
    println!(
        "ruqu-mcp -- ruQu v2 MCP companion server (ADR-008 §6, v0.1)\n\
         \n\
         USAGE:\n    \
             ruqu-mcp stdio [--capabilities CSV] [--audit-file PATH] [--backend-id ID]\n    \
             ruqu-mcp http  [--bind ADDR:PORT] [--capabilities CSV] [...]\n\
         \n\
         COMMON OPTIONS:\n    \
             --audit-file PATH         JSONL audit per ADR-008 §6. Default: stderr.\n    \
             --capabilities CSV        Tier set (accepted but unused in v0.1 — see\n    \
                                       ADR-008 §6 Decision 4; capability tiers ship\n    \
                                       in v0.2 once mcp-server's policy module agrees).\n    \
             --backend-id ID           StateVector backend id (default: ruqu-sv). Flows\n    \
                                       into the witness as the backend handle.\n    \
             -h, --help                Print this help.\n\
         \n\
         HTTP OPTIONS:\n    \
             --bind ADDR:PORT          Bind address. Default 127.0.0.1:7442.\n    \
             --insecure-allow-no-auth  Required to bind --auth none to a non-loopback addr.\n\
         \n\
         v0.1 exposes the same five tools as v0.0.1 — `ruqu_simulate` (publish),\n\
         `ruqu_verify`, `ruqu_replay`, `ruqu_optimize` (stub — v0.2 planner),\n\
         `ruqu_qec_schedule` (stub — v0.2 QEC). Streamable HTTP transport lifts\n\
         the iter 32 CORS layer from mcp-rvdna so the ruLake Console can drive\n\
         this server directly without a CORS proxy."
    );
}

fn init_tracing() {
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};
    let _ = tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(fmt::layer().json().with_writer(std::io::stderr))
        .try_init();
}
