# rvDNA v2 — Integration with ruLake

This document is the code-level companion to `v2-spec.md`. It answers
"how does v2 actually plug into ruLake?" with three concrete
deliverables sketched against the existing trait shapes:

1. `crates/rvdna-backend/` — a sibling Rust crate that implements three
   `BackendAdapter`s (T0/T1/T2) backed by mmap'd `.rvdna` v2 files.
2. `crates/mcp-rvdna/` — a sibling MCP server crate exposing the five
   genomic verbs from the v2 spec (`rvdna_find`, `rvdna_call_variants`,
   `rvdna_translate`, `rvdna_score`, `rvdna_lineage`).
3. **Console hooks** — a 7th sidebar entry (`Genomic`) added to
   `ui/src/components/screens.jsx` that surfaces an `rvdna://bundle/{id}`
   resource and runs witness verification through the existing
   `node-wasm/` pipeline.

Each section ends with a "What v0.1 ships vs v0.2 defers" table so the
sequencing is unambiguous.

All cited paths are real. The code blocks are pseudocode in real Rust
against types that exist (`BackendAdapter` at `crates/core/src/backend.rs:110`,
`#[tool]` macro shape at `crates/mcp-server/src/server.rs:189`, the bundle at
`crates/core/src/bundle.rs:113`, the Console screens at
`ui/src/components/screens.jsx:17`).

---

## Part 1 — `crates/rvdna-backend/`

### 1.1 Crate layout

Sibling of `crates/gcs-backend/` and `crates/ipfs-backend/`. No workspace; mirrors
the discipline from `docs/adrs/ADR-001-standalone-repo-strategy.md`
and the precedent set by `docs/adrs/sdk/ADR-005-ipfs-backend-and-deploy.md`:
each backend is a free-standing crate that depends on `rulake` from
crates.io.

```
crates/rvdna-backend/
├── Cargo.toml
├── README.md
├── src/
│   ├── lib.rs              -- pub use exports for the three tier types
│   ├── file.rs             -- RvdnaV2File: mmap loader + bundle pointer
│   ├── header.rs           -- v2 header + bundle pointer parser
│   ├── sections.rs         -- per-section iterators (k-mer, variant, ...)
│   ├── witness.rs          -- builds the Generation::Opaque payload
│   │                          per v2-spec §d.1, calls
│   │                          rulake::compute_witness equivalent
│   ├── t0.rs               -- RvdnaT0Backend: in-RAM k-mer HNSW
│   ├── t1.rs               -- RvdnaT1Backend: mmap'd warm sections
│   ├── t2.rs               -- RvdnaT2Backend: lazy cold-tier proxy
│   └── error.rs            -- RvdnaBackendError, into RuLakeError
├── tests/
│   ├── round_trip.rs       -- v1 file → migrate → v2 → witness verify
│   ├── t0_register.rs      -- 100-file load test (gate G1)
│   ├── t0_query.rs         -- federated query (gate G3)
│   └── witness_parity.rs   -- v2 file's witness == ruLake bundle witness
└── benches/
    └── v2_acceptance.rs    -- mirrors vendor/ruvector/examples/dna/benches/dna_bench.rs
```

### 1.2 `Cargo.toml` (sketch)

```toml
[package]
name = "rvdna-backend"
version = "0.0.1"
edition = "2021"
description = "ruLake BackendAdapter implementations for rvDNA v2 files."
license = "MIT OR Apache-2.0"
rust-version = "1.77"

[dependencies]
# Core dependencies — ruLake from crates.io (mirrors the gcs-backend
# and ipfs-backend pattern; no workspace, no path deps to vendor/).
rulake = "2.2"

# Memory mapping for tier T1.
memmap2 = "0.9"

# BLAKE3 for section checksums; SHAKE-256 for the witness (parity
# with crates/core/src/bundle.rs::compute_witness).
blake3 = "1.5"
sha3 = "0.10"
hex = "0.4"

# Bundle pointer / header layout — zerocopy avoids reading-byte-by-byte
# parsers, matches the discipline of the rest of ruLake.
zerocopy = { version = "0.7", features = ["derive"] }

# Logging + errors.
thiserror = "1.0"
tracing = "0.1"

# Optional: rayon for tier-T0 parallel cold-prime.
rayon = "1.10"

[dev-dependencies]
criterion = "0.5"
tempfile = "3.10"
# Pull in the v1 crate from the vendored submodule for migration tests.
# This is the ONLY reverse dependency from rvdna-backend on v1.
rvdna = { path = "../vendor/ruvector/examples/dna" }

[[bench]]
name = "v2_acceptance"
harness = false
```

### 1.3 `RvdnaT0Backend` — full sketch

This is the load-bearing impl: every other tier is a refinement.
The trait at `crates/core/src/backend.rs:110` is four required methods plus the
`current_bundle` override that makes cross-deployment cache sharing
free (`crates/core/src/backend.rs:125`).

```rust
// src/t0.rs
use std::sync::Arc;

use rulake::{
    backend::{BackendAdapter, CollectionId, PulledBatch},
    error::{Result, RuLakeError},
    Generation, RuLakeBundle,
};

use crate::file::RvdnaV2File;
use crate::sections::KmerCollection;

/// In-RAM k-mer HNSW backed by a single .rvdna v2 file.
///
/// Tier T0 per `docs/research/rvdna/v2-spec.md` §e.1: this backend
/// is the "hot" tier — k-mer vectors are pulled into RaBitQ-compressed
/// cache on first prime and stay resident for the process lifetime.
pub struct RvdnaT0Backend {
    /// Stable id; `format!("rvdna-t0:{file_id}")`. The file_id is the
    /// BLAKE3 of (creator_version || creation_timestamp ||
    /// first 1 KB of section 0), per v2-spec §d.1.
    id: String,

    /// The mmap'd file. Constructed once at registration time.
    file: Arc<RvdnaV2File>,
}

impl RvdnaT0Backend {
    /// Open a v2 file and register it as a backend.
    ///
    /// Validation up-front (per v2-spec §d.4 verify recipe):
    /// 1. Magic check — rejects v1 files with a clear error.
    /// 2. Bundle pointer parse — fails fast on malformed pointers.
    /// 3. Witness recompute — confirms the file's asserted witness
    ///    matches the recomputed value over the actual section bytes.
    pub fn open<P: AsRef<std::path::Path>>(path: P) -> Result<Self> {
        let file = RvdnaV2File::open_and_verify(path).map_err(|e| {
            RuLakeError::InvalidParameter(format!("rvdna-t0: open: {e}"))
        })?;
        let id = format!("rvdna-t0:{}", file.file_id());
        Ok(Self {
            id,
            file: Arc::new(file),
        })
    }
}

impl BackendAdapter for RvdnaT0Backend {
    fn id(&self) -> &str {
        &self.id
    }

    fn list_collections(&self) -> Result<Vec<CollectionId>> {
        // One collection per gene/region in the §1 k-mer block list.
        // The .rvdna v2 file's metadata sidecar (§7) carries the
        // gene names; if absent (e.g. a synthetic file with anonymous
        // regions), fall back to "block_{i}".
        Ok(self.file.kmer_collection_ids())
    }

    fn pull_vectors(&self, collection: &str) -> Result<PulledBatch> {
        let KmerCollection {
            ids,
            vectors,
            dim,
            generation,
        } = self
            .file
            .pull_kmer_collection(collection)
            .map_err(|e| RuLakeError::InvalidParameter(format!("pull: {e}")))?;
        Ok(PulledBatch {
            collection: collection.to_string(),
            ids,
            vectors,
            dim,
            generation,
        })
    }

    fn generation(&self, collection: &str) -> Result<u64> {
        // T0 is immutable per file. The generation is derived from
        // the bundle pointer's `generation_value` field (§c.4).
        // This is a constant for the lifetime of the file unless
        // STREAMING flag is set — in which case T0 isn't the right
        // tier (streaming files use §6 which is a T1 concern).
        self.file
            .stable_generation(collection)
            .map_err(|e| RuLakeError::InvalidParameter(format!("gen: {e}")))
    }

    /// THE override that closes the witness loop with ruLake.
    ///
    /// Per v2-spec §d.2: the .rvdna v2 file already carries a
    /// pre-computed witness in its bundle pointer. We synthesise a
    /// `RuLakeBundle` whose `compute_witness` output matches that
    /// witness byte-for-byte. This means: when the same .rvdna file
    /// is read by two ruLake instances, both produce the same bundle,
    /// the cache shares the entry across them
    /// (`crates/core/src/cache.rs` "the cross-backend share"), and federated
    /// queries don't re-prime per-instance.
    fn current_bundle(
        &self,
        collection: &str,
        rotation_seed: u64,
        rerank_factor: usize,
    ) -> Result<RuLakeBundle> {
        let dim = self.file.bundle_pointer().dim as usize;
        let opaque = self.file.generation_opaque_for(collection).map_err(|e| {
            RuLakeError::InvalidParameter(format!("gen-opaque: {e}"))
        })?;
        let bundle = RuLakeBundle::new(
            format!("rvdna://{}/{}", self.file.file_id(), collection),
            dim,
            rotation_seed,
            rerank_factor,
            Generation::Opaque(opaque),
        )
        .with_memory_class("genomic");

        // Sanity: the synthesised witness MUST match the file's
        // bundle-pointer witness. If they diverge the file was
        // mis-encoded or tampered — refuse to register it.
        let pointer_witness = self.file.bundle_pointer_witness_hex();
        if bundle.rvf_witness != pointer_witness {
            return Err(RuLakeError::InvalidParameter(format!(
                "rvdna-t0: synthesised witness mismatch with bundle pointer: \
                 synth={} vs file={}",
                bundle.rvf_witness, pointer_witness
            )));
        }
        Ok(bundle)
    }

    fn supports_pushdown(&self) -> bool {
        false
    }
}
```

### 1.4 `RvdnaT1Backend` — sketch (warm tier)

```rust
// src/t1.rs
use rulake::backend::{BackendAdapter, CollectionId, PulledBatch};
use rulake::{Result, RuLakeError, Generation, RuLakeBundle};

/// Warm tier: variant tensor (§3) + protein graphs (§4) +
/// attention COO (§2) + biomarker series (§6 in non-streaming mode).
///
/// Backed by mmap'd file regions. `pull_vectors` returns the variant
/// tensor's per-position likelihoods as a flat vector when the
/// caller queries via the `variants_*` collection name space; the
/// protein and attention tensors get their own collection prefixes.
///
/// This tier accepts `Consistency::Eventual { ttl_ms: 5_000 }` per
/// v2-spec §e.2.
pub struct RvdnaT1Backend {
    id: String,
    file: Arc<crate::file::RvdnaV2File>,
}

impl BackendAdapter for RvdnaT1Backend {
    fn id(&self) -> &str { &self.id }
    fn list_collections(&self) -> Result<Vec<CollectionId>> {
        // "variants_<chr>", "protein_<gene>", "attention_<window>"
        Ok(self.file.t1_collection_ids())
    }
    fn pull_vectors(&self, collection: &str) -> Result<PulledBatch> {
        // mmap window → decode → return as PulledBatch.
        self.file.pull_t1_collection(collection)
    }
    fn generation(&self, collection: &str) -> Result<u64> {
        self.file.stable_generation(collection)
    }
    fn current_bundle(
        &self, collection: &str, seed: u64, rerank: usize,
    ) -> Result<RuLakeBundle> {
        // Same shape as T0; the witness derivation is per v2-spec §d.4.
        let dim = self.file.t1_dim_for(collection)?;
        let opaque = self.file.generation_opaque_for(collection)?;
        Ok(RuLakeBundle::new(
            format!("rvdna://{}/{}", self.file.file_id(), collection),
            dim, seed, rerank,
            Generation::Opaque(opaque),
        ).with_memory_class("genomic"))
    }
}
```

### 1.5 `RvdnaT2Backend` — sketch (cold tier)

```rust
// src/t2.rs

/// Cold tier: raw §0 DNA + §5 epigenomic. Not pulled by default —
/// only when an explicit query asks for raw bases or methylation
/// betas. Backend gates against MAX_T2_DECODE_BYTES_PER_QUERY
/// (v2-spec §e.3) and refuses with RVDNA_T2_BUDGET_REFUSED.
///
/// Wraps an inner `BackendAdapter` (Local, GCS via `crates/gcs-backend/`,
/// or IPFS via `crates/ipfs-backend/`) so cold-tier bytes can live anywhere
/// without v2 needing to know.
pub struct RvdnaT2Backend<Inner: BackendAdapter> {
    id: String,
    inner: Inner,
    file: Arc<crate::file::RvdnaV2File>,
    max_decode_bytes_per_query: usize,
}
```

The T2 backend is the natural composition point with `crates/gcs-backend/`
and `crates/ipfs-backend/`: those crates already implement `BackendAdapter`
for object-storage and IPFS-CID access; T2 wraps them with a §0/§5
decode layer.

### 1.6 What v0.1 ships vs v0.2 defers

| Capability | v0.1 (first PR) | v0.2 (second PR) |
|---|---|---|
| `RvdnaT0Backend` (k-mer HNSW, full BackendAdapter impl) | Ships | – |
| Witness verify on file open | Ships | – |
| `current_bundle()` override matching ruLake's compute_witness | Ships | – |
| Round-trip test against a v1 `.rvdna` from `vendor/ruvector/examples/dna/` (one file, one query) | Ships | – |
| Acceptance gate G1 + G3 (load 100 files, p50 < 10 ms federated) | Ships | – |
| `RvdnaT1Backend` (warm tier, mmap'd variants/protein/attention) | – | Ships |
| `RvdnaT2Backend<Local>` (cold tier, local disk only) | – | Ships |
| `RvdnaT2Backend<GcsParquetBackend>` (cold tier on GCS) | – | Defer to v0.3 |
| `RvdnaT2Backend<IpfsBackend>` (cold tier on IPFS) | – | Defer to v0.3 |
| Streaming-mode read support | – | Defer to v0.3 |
| Multi-sample manifest mode | – | Defer to v0.3 |

---

## Part 2 — `crates/mcp-rvdna/`

### 2.1 Crate layout (mirrors `crates/mcp-server/`)

```
crates/mcp-rvdna/
├── Cargo.toml
├── README.md
├── src/
│   ├── lib.rs              -- pub mod server, pub mod planner
│   ├── main.rs             -- CLI binary; mirrors crates/mcp-server/src/main.rs
│   ├── server.rs           -- RvdnaMcpServer with tool_router (the 5 verbs)
│   ├── planner.rs          -- holds Arc<RuLake> + workers + backend ids
│   ├── auth.rs             -- JWT validator (re-uses scopes_to_caps shape)
│   ├── audit.rs            -- AuditRow emitter (re-uses mcp-server's shape)
│   ├── policy.rs           -- effective_caps (research vs clinical profile)
│   ├── http.rs             -- Streamable HTTP, mirrors crates/mcp-server/src/http.rs
│   └── workers.rs          -- offload-pool, mirrors crates/mcp-server/src/workers.rs
├── tests/
│   ├── tools_smoke.rs      -- roundtrip each of the 5 tools
│   ├── clinical_refusal.rs -- tenant-scope mismatch refusal path
│   └── http_e2e.rs         -- mirrors crates/mcp-server/tests/http_e2e.rs
```

### 2.2 The five `#[tool]`s — sketch

The macro pattern is verbatim from `crates/mcp-server/src/server.rs:189`
(`#[tool_router(router = tool_router)]` on the impl block, `#[tool]`
on each method). The shape:

```rust
// src/server.rs
use std::sync::Arc;

use rmcp::{
    handler::{server::router::tool::ToolRouter, ServerHandler},
    model::{ErrorData as McpError, ServerCapabilities, ServerInfo, *},
    serde_json::json,
    tool, tool_handler, tool_router,
    Json,
};
use rulake::RuLake;

use crate::planner::Planner;
use crate::policy::{effective_caps, Capability, CapabilitySet};

pub struct RvdnaMcpServer {
    pub planner: Arc<Planner>,
    pub capabilities: CapabilitySet,
    tool_router: ToolRouter<Self>,
}

#[tool_router(router = tool_router)]
impl RvdnaMcpServer {
    #[tool(
        name = "rvdna_find",
        description = "Read: k-mer similarity search against a registered .rvdna v2 file. \
                       Returns top-K hits with witness pin. Read-tier — gated by --capabilities read."
    )]
    pub async fn rvdna_find(
        &self,
        rmcp::handler::server::tool::Parameters(args): rmcp::handler::server::tool::Parameters<RvdnaFindArgs>,
    ) -> Result<Json<RvdnaFindResponse>, McpError> {
        require_cap(&self.capabilities, Capability::Read)?;

        let lake = Arc::clone(&self.planner.lake);
        let backend_id = format!("rvdna-t0:{}", args.file_id);
        let collection = args.collection.clone();
        let query = args.query_seq.as_bytes().to_vec();
        let asserted = args.asserted_witness.clone();

        // Witness pin: refuse if the file's actual bundle witness
        // diverges from what the caller asserted.
        let actual_witness = self
            .planner
            .workers
            .submit(move || {
                lake.current_bundle(&(backend_id, collection))
                    .map(|b| b.rvf_witness)
            })
            .await
            .map_err(|e| McpError::internal_error(format!("RULAKE_DEGRADED: {e}"), None))?
            .map_err(|e| McpError::internal_error(format!("RVDNA_INTERNAL: {e}"), None))?;
        if asserted != actual_witness {
            return Err(McpError::invalid_request(
                format!(
                    "RVDNA_WITNESS_MISMATCH_REFUSED: asserted={asserted} actual={actual_witness}"
                ),
                None,
            ));
        }

        // Run the actual k-mer query through the cache.
        let lake = Arc::clone(&self.planner.lake);
        let backend_id = format!("rvdna-t0:{}", args.file_id);
        let collection = args.collection.clone();
        let query_vec = encode_kmer_query(&query, args.k_dim);
        let hits = self
            .planner
            .workers
            .submit(move || lake.search_one(&backend_id, &collection, &query_vec, args.k))
            .await
            .map_err(|e| McpError::internal_error(format!("RULAKE_DEGRADED: {e}"), None))?
            .map_err(|e| McpError::internal_error(format!("RVDNA_INTERNAL: {e}"), None))?;

        Ok(Json(RvdnaFindResponse {
            hits: hits.into_iter().map(|h| RvdnaHit {
                gene: collection.clone(),
                position: h.id,
                score: h.score,
                witness: actual_witness.clone(),
            }).collect(),
        }))
    }

    #[tool(
        name = "rvdna_call_variants",
        description = "Read: return variant calls in a region. Add Clinical capability when \
                       file's pii_policy = phi-strict. Refuses with RVDNA_VARIANT_REFUSED_LOW_DEPTH \
                       when no calls meet min_depth."
    )]
    pub async fn rvdna_call_variants(
        &self,
        rmcp::handler::server::tool::Parameters(args): rmcp::handler::server::tool::Parameters<RvdnaCallVariantsArgs>,
    ) -> Result<Json<RvdnaVariantsResponse>, McpError> {
        // The capability gate is dynamic: the file's profile flags
        // determine whether Clinical is required.
        require_cap(&self.capabilities, Capability::Read)?;
        if self.planner.file_is_phi_strict(&args.file_id) {
            require_cap(&self.capabilities, Capability::Clinical)?;
        }
        // Body: lake.search_one against rvdna-t1:{file_id} / variants_{chr}.
        // ... (omitted — same shape as rvdna_find above)
        unimplemented!("v0.2 deferral; see ship table below")
    }

    #[tool(
        name = "rvdna_translate",
        description = "Read: translate DNA → protein for a region + frame; returns AA sequence + \
                       contact predictions + secondary structure. Refuses with RVDNA_TRANSLATE_NO_ORF \
                       when no ORF is present."
    )]
    pub async fn rvdna_translate(
        &self,
        rmcp::handler::server::tool::Parameters(args): rmcp::handler::server::tool::Parameters<RvdnaTranslateArgs>,
    ) -> Result<Json<RvdnaTranslateResponse>, McpError> {
        require_cap(&self.capabilities, Capability::Read)?;
        unimplemented!("v0.2 deferral")
    }

    #[tool(
        name = "rvdna_score",
        description = "Read: compute a polygenic risk or pharmacogenomic dose score. The \
                       score_id is namespaced (\"prs:cad\", \"cpic:cyp2d6\", ...). Refuses with \
                       RVDNA_SCORE_REFUSED_INSUFFICIENT_COVERAGE when required SNPs are absent."
    )]
    pub async fn rvdna_score(
        &self,
        rmcp::handler::server::tool::Parameters(args): rmcp::handler::server::tool::Parameters<RvdnaScoreArgs>,
    ) -> Result<Json<RvdnaScoreResponse>, McpError> {
        require_cap(&self.capabilities, Capability::Read)?;
        unimplemented!("v0.2 deferral")
    }

    #[tool(
        name = "rvdna_lineage",
        description = "Internal: return the witness chain, model checkpoints, and per-section \
                       BLAKE3 digests for a registered file. Used for audit/replay. \
                       Internal-tier — gated by --capabilities internal."
    )]
    pub async fn rvdna_lineage(
        &self,
        rmcp::handler::server::tool::Parameters(args): rmcp::handler::server::tool::Parameters<RvdnaLineageArgs>,
    ) -> Result<Json<RvdnaLineageResponse>, McpError> {
        require_cap(&self.capabilities, Capability::Internal)?;
        let info = self.planner.file_lineage(&args.file_id).map_err(|e| {
            McpError::internal_error(format!("RVDNA_INTERNAL: {e}"), None)
        })?;
        Ok(Json(info))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for RvdnaMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "rvdna v2 genomic intelligence — five verbs over witness-pinned files. \
                 See docs/research/rvdna/v2-spec.md §h for the full surface.".into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

fn require_cap(caps: &CapabilitySet, required: Capability) -> Result<(), McpError> {
    // v2: combine server-wide caps with per-request JWT scopes.
    // Mirrors `crates/mcp-server/src/server.rs::require_cap`.
    let effective = effective_caps(caps);
    effective
        .require(required)
        .map_err(|refused| McpError::invalid_request(refused.to_string(), None))
}

// ─── Tool arg / response shapes ──────────────────────────────────────

#[derive(Debug, serde::Deserialize, serde::Serialize, rmcp::schemars::JsonSchema)]
pub struct RvdnaFindArgs {
    pub file_id: String,
    pub asserted_witness: String,
    pub collection: String,
    pub query_seq: String,
    pub k_dim: usize,
    pub k: usize,
}

#[derive(Debug, serde::Serialize, serde::Deserialize, rmcp::schemars::JsonSchema)]
pub struct RvdnaHit {
    pub gene: String,
    pub position: u64,
    pub score: f32,
    pub witness: String,
}

#[derive(Debug, serde::Serialize, serde::Deserialize, rmcp::schemars::JsonSchema)]
pub struct RvdnaFindResponse {
    pub hits: Vec<RvdnaHit>,
}

// ... (other tools' arg/response shapes follow the same pattern)
```

### 2.3 Audit row shape (parity with `mcp-server`)

The `mcp-rvdna` server emits `AuditRow` with the exact same fields as
`crates/mcp-server/src/audit.rs`, so a single ingestion pipeline can serve
both. The only thing that changes is the `tool` and `code` strings
(`RVDNA_*` instead of `RULAKE_*`).

```jsonl
{"ts":"2026-04-26T12:34:56.789Z","actor":"agent-pgrx-001","tool":"rvdna_find","args_hash":"blake3:...","outcome":"ok","result_size":10,"trust_level":"read","duration_ms":3,"witness_in":"shake256...","witness_out":"shake256...","code":"RVDNA_FOUND_OK","policy_decision":{"capability_required":"read","capability_granted":"read"}}
{"ts":"2026-04-26T12:34:57.012Z","actor":"agent-pgrx-002","tool":"rvdna_call_variants","args_hash":"blake3:...","outcome":"refused","result_size":0,"trust_level":"clinical","duration_ms":1,"code":"RVDNA_TENANT_SCOPE_REFUSED","policy_decision":{"capability_required":"read+clinical","capability_granted":"read","tenant_match":"none"}}
```

### 2.4 JWT scope mapping

Extend the `scopes_to_caps` function from
`crates/mcp-server/src/auth.rs:294`:

```rust
// crates/mcp-rvdna/src/auth.rs (sketch — same shape as mcp-server's).
pub fn scopes_to_caps(scopes: &HashSet<String>) -> CapabilitySet {
    let mut caps = CapabilitySet::default();
    if scopes.contains("mcp:rvdna:read") {
        caps.add(Capability::Read);
    }
    if scopes.contains("mcp:rvdna:internal") {
        caps.add(Capability::Read);
        caps.add(Capability::Internal);
    }
    if scopes.contains("mcp:rvdna:clinical") {
        caps.add(Capability::Read);
        caps.add(Capability::Clinical);
    }
    if scopes.contains("mcp:rvdna:admin") {
        caps.add(Capability::Read);
        caps.add(Capability::Internal);
        caps.add(Capability::Clinical);
        caps.add(Capability::Admin);
    }
    caps
}
```

The JWT tenant claim (`rvdna_tenant: ["acme-hospital", ...]`) is
read separately at request time and stored in a task-local; the
clinical-profile guard in `rvdna_call_variants` intersects the
file's tenant_ids (from §7 metadata) with this list.

### 2.5 What v0.1 ships vs v0.2 defers

| Capability | v0.1 (first PR) | v0.2 (second PR) |
|---|---|---|
| Crate scaffolding (Cargo.toml, lib.rs, main.rs) | Ships | – |
| `rvdna_find` end-to-end (witness pin + HNSW search) | Ships | – |
| `rvdna_lineage` (read bundle + return JSON) | Ships | – |
| stdio transport + bearer auth | Ships | – |
| Audit-tail JSONL emit | Ships | – |
| `rvdna_call_variants` | – | Ships |
| `rvdna_translate` | – | Ships |
| `rvdna_score` | – | Ships |
| Streamable HTTP transport | – | Ships |
| JWT auth + JWKS hot rotation | – | Ships |
| mTLS | – | Defer to v0.3 |
| Tenant-scoped clinical refusal | – | Ships |
| Capability-gated `tools/list` filter | – | Ships (mirrors `crates/mcp-server/src/server.rs:566`) |

---

## Part 3 — Console hooks

The Console at `ui/` ships today with six sidebar entries (per
`docs/adrs/ADR-006-rulake-console-vite-github-pages.md`). The
question: is `Genomic` a first-class 7th screen, or an ephemeral
panel surfaced from existing screens?

### 3.1 The defence: 7th sidebar entry, first-class

Three reasons:

1. **The `rvdna://` resource is novel.** Existing screens (Browse,
   Bundle, Audit, etc.) all assume the resource shape is
   `rulake://...`. A `.rvdna` file is content-addressed by a different
   URI scheme (`rvdna://blake3:...`) and carries data shapes
   (variants, proteins, biomarker series) that don't fit any existing
   screen's mental model.
2. **Witness verification is the same code path.** The Console already
   ships `verifyBundleJson` from `node-wasm/`
   (`docs/adrs/ADR-006-rulake-console-vite-github-pages.md` line 22).
   Adding `verifyRvdnaWitness` as a sibling export means the Genomic
   screen renders `.rvdna` witnesses with zero new browser-side
   crypto code — only a UI shell.
3. **The agent persona benefits from a one-screen demo.** The `mcp-rvdna`
   tool surface (the five verbs) is the most concrete demonstration
   of "rvDNA + ruLake = perfect compositional fit"; an ephemeral
   panel hidden behind another screen buries the lede.

The cost is a single new entry in `Sidebar`'s nav-items array
(`ui/src/components/screens.jsx:17`) and one new screen module.

### 3.2 What the Genomic screen does

Three sub-views, gated by route param:

#### `Genomic / Files` (default)

A list of registered `rvdna-t0:*` and `rvdna-t1:*` backends, sourced
via the existing `rulake_list_backends` MCP tool
(`crates/mcp-server/src/server.rs:323`) plus a filter that keeps only the
ones with `rvdna-` prefix. For each file:
- file_id (the BLAKE3 prefix; click-to-copy)
- bundle witness (with a green checkmark when `verifyRvdnaWitness`
  succeeds)
- `pii_policy_class` (research / phi-strict)
- model checkpoints (from `rvdna_lineage`)
- collection count (from `rulake_list_collections`)

#### `Genomic / Verify`

A drag-and-drop area for a `.rvdna` file. The browser fetches the
bytes, runs `verifyRvdnaWitness(bytes)` via the WASM module, and
displays:
- magic-byte check (v1 vs v2)
- header parse result
- bundle pointer parse result
- per-section BLAKE3 vs sidecar metadata
- final witness comparison (asserted vs computed)

This is the same pattern as today's Bundle screen
(`ui/src/components/screens.jsx:1114` — "Browser-rendered strip on the
Bundle screen: paste a CID, fetch...") but for full `.rvdna` files,
not just bundle JSONs.

#### `Genomic / Query`

A live mode-only sub-view (Demo / WASM-local fall back to a "Live mode
required" panel). A form that:
- asks for the file_id (selectable from `Genomic / Files`)
- asks for a query sequence (paste a FASTA snippet)
- calls `rvdna_find` over the live MCP transport
- renders hits as a sortable table with the matched witness

The MCP client is the same `RuLakeHttp` from `node/http.mjs:107`
(per `docs/adrs/ADR-006-rulake-console-vite-github-pages.md` line 23),
re-pointed at the `mcp-rvdna` server's URL. No new transport code.

### 3.3 Sidebar entry — the diff

```jsx
// ui/src/components/screens.jsx:17 today (sketch — read the file for actual array)
function Sidebar({ route, setRoute, ... }) {
  const items = [
    { id: 'browse',  label: 'Browse',  icon: '...' },
    { id: 'bundle',  label: 'Bundle',  icon: '...' },
    { id: 'cache',   label: 'Cache',   icon: '...' },
    { id: 'audit',   label: 'Audit',   icon: '...' },
    { id: 'witness', label: 'Witness', icon: '...' },
    { id: 'help',    label: 'Help',    icon: '...' },
  ];
  // ...
}
```

The diff to add Genomic:

```jsx
const items = [
  { id: 'browse',   label: 'Browse',   icon: '...' },
  { id: 'bundle',   label: 'Bundle',   icon: '...' },
  { id: 'cache',    label: 'Cache',    icon: '...' },
  { id: 'audit',    label: 'Audit',    icon: '...' },
  { id: 'witness',  label: 'Witness',  icon: '...' },
  { id: 'genomic',  label: 'Genomic',  icon: '...', // NEW
    badge: liveCounts.rvdnaFiles ?? 0 },
  { id: 'help',     label: 'Help',     icon: '...' },
];
```

The badge count (live `.rvdna` files registered) is sourced the same
way the existing Browse-screen counts are
(`ui/src/components/screens.jsx:18` "Live counts from IndexedDB...").

### 3.4 What v0.1 ships vs v0.2 defers

| Capability | v0.1 (first PR) | v0.2 (second PR) |
|---|---|---|
| Sidebar entry + route registration | Ships | – |
| `Genomic / Files` (read-only list of registered backends) | Ships | – |
| `Genomic / Verify` (drag-drop file → verifyRvdnaWitness) | Ships | – |
| `verifyRvdnaWitness` WASM export in `node-wasm/src/lib.rs` | Ships | – |
| `Genomic / Query` (live mode k-mer search) | – | Ships |
| Variant browser (calls `rvdna_call_variants`) | – | Ships |
| Protein structure preview (calls `rvdna_translate`) | – | Defer to v0.3 |
| Lineage timeline view | – | Ships |
| Demo-mode sample `.rvdna` (mirrors today's "Try sample" button) | – | Ships |

---

## Cross-cutting notes

### A. The 1.02× hit-path tax

`BENCHMARK.md` measured ruLake's cache hit-path overhead at 1.02× the
underlying `RabitqPlusIndex` query. v2's `RvdnaT0Backend::pull_vectors`
returns vectors that ruLake's cache compresses with RaBitQ; subsequent
queries pay the same 1.02× tax. The `rvdna_find` p50 < 10 ms target
in v2-spec §l.3 already accounts for this — the v1 floor of 12 ms is
a full pipeline including encode; v2 is a cache hit, so 10 ms is
generous.

### B. Why no shared workspace

`docs/adrs/ADR-001-standalone-repo-strategy.md` and the precedent set
by `crates/gcs-backend/`, `crates/ipfs-backend/`, and `node/` keep each crate free-
standing. `crates/rvdna-backend/` and `crates/mcp-rvdna/` follow the same rule.
Each `Cargo.toml` lists `rulake = "2.2"` from crates.io. No
`workspace = true` deps.

### C. Where v1 stays

v1 is not modified. `vendor/ruvector/examples/dna/` continues to ship
its own `rvdna` crate. v2 reads v1 files via the migration path
(`v2-spec.md` §m), but v1 readers do not (and cannot) read v2 files —
the magic differs by one byte.

### D. The `mcp-rvdna` <-> `mcp-server` relationship

Two separate servers. They share:
- The audit row shape (`crates/mcp-server/src/audit.rs::AuditRow`).
- The capability mapping pattern (`scopes_to_caps`).
- The `RuLake` instance (process-local; both servers can hold an
  `Arc<RuLake>` to the same lake).

They do NOT share:
- The tool router. Each server has its own. `tools/list` from
  `mcp-server` returns ruLake tools; from `mcp-rvdna` returns the
  five rvDNA tools.
- The crate. `mcp-rvdna` is a sibling, not an extension.

This means an operator runs *two* binaries (one for each server),
both pointing at the same ruLake instance via shared state or both
pointing at the same `--workspace-dir`. The Console talks to either
or both via separate URLs.
