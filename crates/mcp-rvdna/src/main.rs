//! `rvdna-mcp` binary — MCP companion server entry point for the rvDNA
//! v2 substrate (ADR-007 §6, v0.0.1).
//!
//! v0.0.1 wires no real backend — there are no `.rvdna` files in the
//! tree yet. The binary serves an empty registry by default, useful as
//! a smoke test (`tools/list` returns the read-tier 4 + the optional
//! `rvdna_lineage` if `--capabilities internal`). Operators with a
//! built-up `RvdnaT0Backend` should embed `RvdnaMcpServer` as a
//! library and call `register_collection` before `serve_*`.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;

use ruvector_rulake_mcp_rvdna::{
    AllowBearerOnPublic, AuditSink, AuthMode, CapabilitySet, InsecureAllowNoAuth,
    RvdnaMcpServer,
};
use ruvector_rulake_rvdna::{RvdnaCollection, RvdnaT0Backend};

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let args = parse_args()?;

    let capabilities = match args.capabilities.as_deref() {
        Some(csv) => CapabilitySet::from_csv(csv)?,
        None => CapabilitySet::default(),
    };
    tracing::info!(capabilities = ?capabilities.labels(), "starting rvdna-mcp");

    let audit = match &args.audit_file {
        Some(p) => {
            let sink = AuditSink::open_file(p)
                .with_context(|| format!("opening rvdna audit file: {}", p.display()))?;
            tracing::info!(path = %p.display(), "rvdna audit -> JSONL file");
            sink
        }
        None => {
            tracing::info!("rvdna audit -> stderr (no --audit-file set)");
            AuditSink::stderr()
        }
    };

    let server = RvdnaMcpServer::new()
        .with_capabilities(capabilities)
        .with_audit(audit);

    // --demo-collection: register a deterministic RvdnaT0Backend("demo")
    // with one seeded collection ("demo", 64 k-mer vectors at D=8, PCG32
    // seed 0xDEADBEEF). Lets the public Cloud Run rvdna-mcp answer real
    // rvdna_find / call_variants / translate / score calls instead of
    // refusing every request with RVDNA_UNKNOWN_COLLECTION. Off in
    // production deploys (the witness over a 64-row toy isn't useful).
    if args.demo_collection {
        let backend = Arc::new(RvdnaT0Backend::new("demo"));
        let mut state = 0xDEADBEEFu64;
        let mut next_f32 = || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let bits = (state >> 40) as u32;
            let x = (bits as f32) / (1u32 << 24) as f32;
            x * 2.0 - 1.0
        };
        let coll = RvdnaCollection {
            ids: (0..64u64).collect(),
            vectors: (0..64).map(|_| (0..8).map(|_| next_f32()).collect()).collect(),
            dim: 8,
            generation: 1,
        };
        backend.put_collection("demo", coll);
        server
            .register_collection(backend, "demo", 0xDEADBEEF, 20)
            .context("--demo-collection: register seeded T0 backend")?;
        tracing::info!(
            backend = "demo",
            collection = "demo",
            vectors = 64,
            "rvdna demo collection registered (--demo-collection)"
        );
    }

    match args.transport {
        Transport::Stdio => server.serve_stdio().await,
        Transport::Http(http) => {
            ruvector_rulake_mcp_rvdna::http::serve(
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
    demo_collection: bool,
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
    let mut demo_collection = false;

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
            "--bind" => {
                let s = it.next().context("--bind expects ADDR:PORT")?;
                http_bind = Some(s.parse().with_context(|| format!("--bind {s:?}"))?);
            }
            "--insecure-allow-no-auth" => http_insecure_allow_no_auth = true,
            "--demo-collection" => demo_collection = true,
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
            let bind = http_bind.unwrap_or_else(|| "127.0.0.1:7441".parse().unwrap());
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
        demo_collection,
    })
}

fn print_help() {
    println!(
        "rvdna-mcp -- rvDNA v2 MCP companion server (ADR-007 §6, v0.0.1)\n\
         \n\
         USAGE:\n    \
             rvdna-mcp stdio [--capabilities CSV] [--audit-file PATH]\n    \
             rvdna-mcp http  [--bind ADDR:PORT] [--capabilities CSV] [...]\n\
         \n\
         COMMON OPTIONS:\n    \
             --audit-file PATH         JSONL audit per ADR-007 §6. Default: stderr.\n    \
             --capabilities CSV        Tier set: read|internal|clinical|admin (default: read).\n    \
                                       `read` exposes the 4 read-tier rvdna tools.\n    \
                                       `internal` adds rvdna_lineage.\n    \
             -h, --help                Print this help.\n\
         \n\
         HTTP OPTIONS:\n    \
             --bind ADDR:PORT          Bind address. Default 127.0.0.1:7441.\n    \
             --insecure-allow-no-auth  Required to bind --auth none to a non-loopback addr.\n\
         \n\
         v0.0.1 has zero mutation tools — the rvdna wire surface is read-only by design.\n\
         (See docs/research/security/v0.0-substrates.md R-1.) Operators wire backends in\n\
         at process start via the library API; v0.0.2 lifts JWT/mTLS from mcp-server."
    );
}

fn init_tracing() {
    use tracing_subscriber::{EnvFilter, fmt, prelude::*};
    let _ = tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(fmt::layer().json().with_writer(std::io::stderr))
        .try_init();
}
