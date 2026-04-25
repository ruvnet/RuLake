//! ruLake bundle utilities, sandboxed inside `wasm32-wasi`.
//!
//! Build:
//!   cargo build --target wasm32-wasip1 --release
//!
//! Run with any WASI runtime:
//!   wasmtime ... bundle-toolkit.wasm verify  /path/to/dir
//!   wasmtime ... bundle-toolkit.wasm dump    /path/to/dir
//!   wasmtime ... bundle-toolkit.wasm witness /path/to/dir
//!
//! WASI runtimes sandbox filesystem access — the directory must be
//! mounted explicitly: `wasmtime --dir=/path/to/dir bundle-toolkit.wasm
//! verify /path/to/dir`. Wasmer is similar.

#![deny(unsafe_code)]

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use sha3::{
    digest::{ExtendableOutput, Update, XofReader},
    Shake256,
};

const SIDECAR_FILENAME: &str = "table.rulake.json";
const MAX_JSON_BYTES: usize = 64 * 1024;
const MAX_FIELD_BYTES: usize = 4 * 1024;
const MAX_WITNESS_HEX: usize = 128;
const SUPPORTED_FORMAT_VERSION: u32 = 2;
const GEN_TAG_NUM: u8 = 0x00;
const GEN_TAG_OPAQUE: u8 = 0x01;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
enum Generation {
    Num(u64),
    Opaque(String),
}

impl Generation {
    fn hash_bytes(&self) -> Vec<u8> {
        match self {
            Self::Num(n) => {
                let mut out = Vec::with_capacity(1 + 8);
                out.push(GEN_TAG_NUM);
                out.extend_from_slice(&n.to_le_bytes());
                out
            }
            Self::Opaque(s) => {
                let mut out = Vec::with_capacity(1 + s.len());
                out.push(GEN_TAG_OPAQUE);
                out.extend_from_slice(s.as_bytes());
                out
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RuLakeBundle {
    format_version: u32,
    data_ref: String,
    dim: u64,
    rotation_seed: u64,
    rerank_factor: u64,
    generation: Generation,
    rvf_witness: String,
    #[serde(default)]
    pii_policy: Option<String>,
    #[serde(default)]
    lineage_id: Option<String>,
    #[serde(default)]
    memory_class: Option<String>,
}

fn compute_witness(b: &RuLakeBundle) -> String {
    let mut h = Shake256::default();
    h.update(b"rulake-bundle-witness-v1|");
    h.update(&(b.data_ref.len() as u64).to_le_bytes());
    h.update(b.data_ref.as_bytes());
    h.update(b"|");
    h.update(&b.dim.to_le_bytes());
    h.update(&b.rotation_seed.to_le_bytes());
    h.update(&b.rerank_factor.to_le_bytes());
    h.update(b"|");
    let g = b.generation.hash_bytes();
    h.update(&(g.len() as u64).to_le_bytes());
    h.update(&g);
    let mut reader = h.finalize_xof();
    let mut out = [0u8; 32];
    reader.read(&mut out);
    hex::encode(out)
}

fn load_bundle(dir: &std::path::Path) -> Result<RuLakeBundle, String> {
    let path = dir.join(SIDECAR_FILENAME);
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| format!("read {}: {e}", path.display()))?;
    if raw.len() > MAX_JSON_BYTES {
        return Err(format!("bundle parse: exceeds {MAX_JSON_BYTES} bytes"));
    }
    let b: RuLakeBundle = serde_json::from_str(&raw).map_err(|e| format!("parse: {e}"))?;
    if b.format_version > SUPPORTED_FORMAT_VERSION {
        return Err(format!(
            "format_version={} newer than this binary supports ({SUPPORTED_FORMAT_VERSION})",
            b.format_version
        ));
    }
    if b.data_ref.len() > MAX_FIELD_BYTES {
        return Err(format!("data_ref exceeds {MAX_FIELD_BYTES} bytes"));
    }
    for (name, opt) in [
        ("pii_policy", b.pii_policy.as_deref()),
        ("lineage_id", b.lineage_id.as_deref()),
        ("memory_class", b.memory_class.as_deref()),
    ] {
        if let Some(v) = opt {
            if v.len() > MAX_FIELD_BYTES {
                return Err(format!("{name} exceeds {MAX_FIELD_BYTES} bytes"));
            }
        }
    }
    if b.rvf_witness.len() > MAX_WITNESS_HEX {
        return Err("rvf_witness not a hex-encoded SHAKE-256(32)".to_string());
    }
    Ok(b)
}

#[derive(Parser, Debug)]
#[command(
    name = "bundle-toolkit",
    about = "ruLake bundle utilities (wasm32-wasi)",
    version
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Verify a bundle's witness. Exit 0 on match, 1 on mismatch, 2 on error.
    Verify { dir: PathBuf },
    /// Pretty-print the bundle JSON.
    Dump { dir: PathBuf },
    /// Print only the recomputed witness hex (for shell pipelines).
    Witness { dir: PathBuf },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Verify { dir } => match load_bundle(&dir) {
            Ok(b) => {
                let computed = compute_witness(&b);
                if computed == b.rvf_witness {
                    println!("PASS  {}", dir.join(SIDECAR_FILENAME).display());
                    println!("  witness: {computed}");
                    ExitCode::from(0)
                } else {
                    eprintln!("FAIL  {}", dir.join(SIDECAR_FILENAME).display());
                    eprintln!("  stored:   {}", b.rvf_witness);
                    eprintln!("  computed: {computed}");
                    ExitCode::from(1)
                }
            }
            Err(e) => {
                eprintln!("ERROR  {e}");
                ExitCode::from(2)
            }
        },
        Cmd::Dump { dir } => match load_bundle(&dir) {
            Ok(b) => match serde_json::to_string_pretty(&b) {
                Ok(s) => {
                    println!("{s}");
                    ExitCode::from(0)
                }
                Err(e) => {
                    eprintln!("ERROR  serialize: {e}");
                    ExitCode::from(2)
                }
            },
            Err(e) => {
                eprintln!("ERROR  {e}");
                ExitCode::from(2)
            }
        },
        Cmd::Witness { dir } => match load_bundle(&dir) {
            Ok(b) => {
                println!("{}", compute_witness(&b));
                ExitCode::from(0)
            }
            Err(e) => {
                eprintln!("ERROR  {e}");
                ExitCode::from(2)
            }
        },
    }
}
