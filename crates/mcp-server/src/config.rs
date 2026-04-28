//! Operator config — `mcp.toml`. Subset of the full ADR-004 §5 shape;
//! v0.1 only covers what the v0.1 server actually consults.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
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
    /// Number of rayon worker threads. Defaults to `num_cpus::get_physical()`
    /// when omitted from config — `0` was the prior `#[serde(default)]` value
    /// and caused the live demo to ship with `workers=0 max_inflight=64`,
    /// which buffers up to 64 calls then DEGRADEs because no workers consume.
    /// See https://github.com/ruvnet/RuLake/issues — discovered via
    /// agent-side capability map showing rulake_list_collections DEGRADED
    /// while rulake_list_backends (static lookup, no pool) succeeded.
    #[serde(default = "default_workers")]
    pub workers: usize,

    /// Max in-flight tool calls before backpressure. ADR-004 §6 default 64.
    #[serde(default = "default_max_inflight")]
    pub max_inflight: usize,

    /// Backends to register at startup. Order matters for the
    /// planner's tie-break ("first registered wins" in v0.1).
    #[serde(default)]
    pub backends: Vec<BackendConfig>,

    /// RBAC allow-list (ADR-004 §5). Empty = no per-collection
    /// restriction (only the capability-tier gate fires). When
    /// populated, every (backend, collection, capability) tuple
    /// must match at least one block.
    #[serde(default, rename = "allow")]
    pub allow: Vec<AllowBlock>,

    /// When true, register a deterministic in-memory `LocalBackend`
    /// named `"demo"` with one seeded collection (`memory`, 100
    /// vectors at D=8, PCG32 seed = 0xDEADBEEF) and add an
    /// allow-block granting `read,publish` against
    /// `(backend="demo", collection=".*")`. Lets the public Cloud
    /// Run demo answer real `rulake_query` / `rulake_list_collections`
    /// / `rulake_publish_bundle` calls instead of refusing on an
    /// empty allowlist.
    ///
    /// Off in production. Enable via `--demo-backend` on the CLI or
    /// `demo_backend = true` in mcp.toml.
    #[serde(default)]
    pub demo_backend: bool,
}

impl Default for McpConfig {
    /// Hand-written `Default` (rather than `#[derive(Default)]`) because
    /// `#[serde(default = "default_workers")]` only fires during
    /// deserialization. A no-config deploy (e.g. `rulake-mcp http
    /// --bind ... --auth none` with no `--config` flag) calls
    /// `McpConfig::default()`, which would otherwise inherit
    /// `usize::default() = 0` for `workers` and DEGRADE the worker pool.
    /// Caught live on https://rulake-mcp.ruv.io/ via agent-side
    /// capability map: rulake_list_collections returned RULAKE_DEGRADED
    /// even after the serde-default fix shipped.
    fn default() -> Self {
        Self {
            consistency: ConsistencyConfig::default(),
            rerank_factor: default_rerank(),
            rotation_seed: default_seed(),
            workers: default_workers(),
            max_inflight: default_max_inflight(),
            backends: Vec::new(),
            allow: Vec::new(),
            demo_backend: false,
        }
    }
}

/// One [[allow]] block in mcp.toml.
///
/// ```toml
/// [[allow]]
/// backend    = "lake-prod"
/// collection = "docs.*"      # regex
/// caps       = ["read"]
/// ```
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AllowBlock {
    /// Backend id (exact match — backends are operator-controlled and
    /// finite, regex is overkill).
    pub backend: String,
    /// Collection regex anchored implicitly (we wrap in `^…$` at
    /// compile time so `docs.*` matches `docs`, `docs.public`, but
    /// NOT `secret-docs.public`).
    pub collection: String,
    /// Capabilities granted by this block. `read` is always implied.
    /// `["read", "publish"]` lets a principal call publish-tier tools
    /// against this (backend, collection) pair.
    #[serde(default)]
    pub caps: Vec<String>,
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
    pub fn into_runtime(self) -> rulake::cache::Consistency {
        use rulake::cache::Consistency;
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

/// Default rayon worker count — `available_parallelism()` clamped to `[2, 16]`.
/// The clamp keeps small-host (1-core CI) installs from getting `workers=1`
/// and big-host (64-core server) installs from spinning a worker per core
/// when the bounded `max_inflight=64` channel can't feed them all anyway.
/// Operators wanting a different sizing pass `--workers N` on the CLI or
/// set `workers = N` in config.toml.
fn default_workers() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(2, 16)
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
