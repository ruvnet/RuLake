//! Operator config — `mcp.toml`. Subset of the full ADR-004 §5 shape;
//! v0.1 only covers what the v0.1 server actually consults.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct McpConfig {
    /// `Fresh` (default), `Eventual { ttl_ms }`, or `Frozen`.
    #[serde(default)]
    pub consistency: ConsistencyConfig,

    /// RaBitQ rerank factor. ADR-155 §M1 gates 100% recall@10 at ≥ 20.
    #[serde(default = "default_rerank")]
    pub rerank_factor: usize,

    /// Haar rotation seed. Same seed across processes → witness shares.
    #[serde(default = "default_seed")]
    pub rotation_seed: u64,

    /// Number of worker threads in the bounded pool. `0` = `cores * 2`
    /// (the ADR-004 §6 default). Capped at 256.
    #[serde(default)]
    pub workers: usize,

    /// Max in-flight tool calls before backpressure. ADR-004 §6 default 64.
    #[serde(default = "default_max_inflight")]
    pub max_inflight: usize,

    /// Backends to register at startup. Order matters for the
    /// planner's tie-break ("first registered wins" in v0.1).
    #[serde(default)]
    pub backends: Vec<BackendConfig>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(tag = "mode", rename_all = "PascalCase")]
pub enum ConsistencyConfig {
    Fresh,
    Eventual { ttl_ms: u64 },
    Frozen,
}

impl Default for ConsistencyConfig {
    fn default() -> Self {
        Self::Fresh
    }
}

impl ConsistencyConfig {
    pub fn into_runtime(self) -> ruvector_rulake::cache::Consistency {
        use ruvector_rulake::cache::Consistency;
        match self {
            Self::Fresh => Consistency::Fresh,
            Self::Eventual { ttl_ms } => Consistency::Eventual { ttl_ms },
            Self::Frozen => Consistency::Frozen,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum BackendConfig {
    /// In-memory backend. v0.1 uses this as the demo path; the MCP
    /// server doesn't populate collections for `local` — that's a
    /// programmatic step the operator does in code (or via `Fs`
    /// loading a `ruvec1` file).
    Local { id: String },
    /// Filesystem-backed `ruvec1` files.
    Fs {
        id: String,
        root: PathBuf,
        #[serde(default)]
        collections: Vec<FsCollection>,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FsCollection {
    pub name: String,
    pub filename: String,
}

fn default_rerank() -> usize {
    20
}

fn default_seed() -> u64 {
    42
}

fn default_max_inflight() -> usize {
    64
}

impl McpConfig {
    pub fn from_toml(s: &str) -> anyhow::Result<Self> {
        Ok(toml::from_str(s)?)
    }

    pub fn from_path(path: impl AsRef<std::path::Path>) -> anyhow::Result<Self> {
        let body = std::fs::read_to_string(path.as_ref())?;
        Self::from_toml(&body)
    }
}
