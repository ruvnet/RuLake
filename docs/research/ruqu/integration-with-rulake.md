# ruQu v2 — Integration with ruLake

This document goes from `v2-spec.md`'s abstractions to concrete
crate-and-line-of-code integration. Three load-bearing pieces:

1. `crates/ruqu-backend/` — a sibling Cargo crate (analogous to `crates/gcs-backend/`
   and `crates/ipfs-backend/` in this repo) that exposes the five ruqu-core
   simulation engines as `BackendAdapter` implementations
   (`crates/core/src/backend.rs:110`).
2. `crates/mcp-ruqu/` — a sibling MCP server crate (analogous to
   `crates/mcp-server/`) that exposes the five intent verbs from `v2-spec.md`
   §g as `#[tool]`-decorated methods, mirroring the macro pattern at
   `crates/mcp-server/src/server.rs:189`.
3. Console hooks — a 7th sidebar entry (`Quantum`) in `ui/` per
   `docs/adrs/ADR-006-rulake-console-vite-github-pages.md`'s
   route-extension discipline, composing existing `ruqu-wasm` and
   `rulake-wasm` modules.

Each section ends with a "What v0.1 ships vs v0.2 defers" subsection
so the first PR after ADR-008 acceptance has an unambiguous scope.

## 1. `crates/ruqu-backend/` — single crate, five Adapter impls

### 1.1 The "single union-impl with discriminator" vs "five sibling impls" decision

The brief asks us to defend the call. We pick **five sibling impls
inside one crate**. Defence:

**For five sibling impls (chosen):**

- Each backend has materially different state: StateVector caches
  amplitude vectors, Stabilizer caches tableau bit-packings, Clifford+T
  caches sums of stabilizer terms, TensorNetwork caches MPS chains,
  Hardware caches measurement histograms. A union impl would either
  carry all five of these inside one struct (fat) or hide them behind
  a trait object (re-introduces the dispatch the union impl was
  trying to avoid).
- Per-backend `generation()` semantics differ: SV/Stab/Clifford+T/TN
  tick on ruqu-core revision; Hardware ticks on calibration snapshot
  (per `v2-spec.md` §d.5). One impl per backend keeps the per-axis
  semantics straight.
- Feature-flag composition: operators with WASM-target deployments
  want only StateVector + Stabilizer compiled in. The Cargo
  feature-flag pattern `[features] state-vector = [], stabilizer =
  [], ...` works cleanly when each adapter is its own module; a
  union impl would gate the *internals* of one module, which is
  more brittle.

**Against (and why we rejected):**

- A union impl with a `BackendKind` discriminator field would let
  the lake hold a single `Vec<RuQuBackend>` instead of a
  `Vec<Box<dyn BackendAdapter>>`. The runtime cost difference
  (vtable dispatch vs match-on-enum) is zero for our call rates
  (lake calls `pull_vectors` on cache miss only; this is once per
  *prime*, not once per query). Not worth the structural cost.

**Resulting crate layout:**

```
crates/ruqu-backend/
├── Cargo.toml
├── src/
│   ├── lib.rs                  # re-exports, feature-gated mods
│   ├── runtime_context.rs      # the RuntimeContext struct from v2-spec.md §c
│   ├── planner_cache.rs        # the CacheAwarePlanner from §e
│   ├── state_vector.rs         # impl BackendAdapter for StateVectorBackend
│   ├── stabilizer.rs           # impl BackendAdapter for StabilizerBackend
│   ├── clifford_t.rs           # impl BackendAdapter for CliffordTBackend
│   ├── tensor_network.rs       # impl BackendAdapter for TensorNetworkBackend
│   └── hardware.rs             # impl BackendAdapter for HardwareBackend
└── tests/
    ├── round_trip_state_vector.rs   # G2 test (cross-process replay sub-1ms)
    ├── witness_equivalence.rs       # G1 test (1000 circuits)
    ├── hardware_cache_attribution.rs # G3 test (mock device)
    ├── clifford_concordance.rs       # G4 test (stabilizer ↔ mock-hardware)
    └── audit_round_trip.rs           # G5 test (mcp-server compat)
```

### 1.2 `Cargo.toml`

```toml
[package]
name = "ruqu-backend"
version = "0.0.1"
edition = "2021"
description = "ruLake BackendAdapter implementations for the five \
               ruqu-core simulation backends. Sibling crate; not in \
               a workspace, per ADR-001."
license = "MIT OR Apache-2.0"

[dependencies]
rulake = "2.2"  # the crates.io published version; mirrors crates/gcs-backend/Cargo.toml shape
ruqu-core = { path = "../vendor/ruvector/crates/ruqu-core" }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
sha3 = "0.10"   # SHAKE-256 for the runtime-context noise_id digest
hex = "0.4"

# Optional dep gate per backend; reqwest only needed for hardware/.
reqwest = { version = "0.11", optional = true, default-features = false,
            features = ["json", "rustls-tls"] }

[features]
default = ["state-vector", "stabilizer"]
state-vector = []
stabilizer = []
clifford-t = []
tensor-network = []
hardware = ["dep:reqwest"]
all = ["state-vector", "stabilizer", "clifford-t", "tensor-network", "hardware"]

# WASM target: only SV + Stab compile cleanly under wasm32-unknown-unknown
# in v0.1. clifford-t + tensor-network use rayon for parallelism (CT term
# expansion, TN contraction) and need cfg adjustments. hardware needs a
# HTTP client. v0.2 unifies the WASM build.
```

### 1.3 The `BackendAdapter` trait surface — one method group at a time

Per `crates/core/src/backend.rs:110`, the trait has 4 required methods plus 2
optional. We walk through what each method *does* per backend and
where the backend's state lives.

#### `id(&self) -> &str`

Returns the stable identifier the lake uses to address this backend.
Per `v2-spec.md` §d:

```rust
// state_vector.rs
fn id(&self) -> &str { "ruqu-state-vector" }

// stabilizer.rs
fn id(&self) -> &str { "ruqu-stabilizer" }

// clifford_t.rs
fn id(&self) -> &str { "ruqu-clifford-t" }

// tensor_network.rs
fn id(&self) -> &str { "ruqu-tensor-network" }

// hardware.rs — composite id encodes provider:device for unambiguous
// audit attribution per v2-spec.md §l.
fn id(&self) -> &str {
    // Lazy-computed once; held in self.cached_id.
    self.cached_id.as_str()
}
// where self.cached_id was built at construction:
//   format!("ruqu-hardware:{}:{}", provider.short(), device_name)
```

Wired into the lake at registration:

```rust
// At process startup, in the operator's binary.
use rulake::Lake;
use ruqu_backend::{StateVectorBackend, StabilizerBackend};

let lake = Lake::new()?;
lake.register_backend(Arc::new(StateVectorBackend::new()))?;
lake.register_backend(Arc::new(StabilizerBackend::new()))?;
// ...
```

#### `list_collections(&self) -> Result<Vec<CollectionId>>`

Per `v2-spec.md` §d, a "collection" in ruQu's universe is a
*registered circuit-witness*. Each `CollectionId` is the hex-encoded
`circuit_hash` for a circuit whose result lives in this backend's
cache.

```rust
// state_vector.rs
fn list_collections(&self) -> Result<Vec<CollectionId>> {
    Ok(self.results.read().unwrap().keys().cloned().collect())
}
```

The `self.results` field is a `Arc<RwLock<HashMap<String, CachedRun>>>`
where the key is the circuit-hash hex and `CachedRun` holds the
amplitudes, the noise/decoder/mitigation ids, the precision mode,
the SIMD path active at run time, and the simulation result.

For Hardware, `list_collections` only returns *completed* jobs:

```rust
// hardware.rs
fn list_collections(&self) -> Result<Vec<CollectionId>> {
    Ok(self.completed_jobs.read().unwrap()
        .iter()
        .map(|j| j.circuit_hash_hex.clone())
        .collect())
}
```

Pending jobs are an out-of-band concern; the lake doesn't see them.
This matters because a hardware backend with 50 pending jobs and 3
completed shouldn't expose 53 collections — only the 3 the lake can
actually serve from cache.

#### `pull_vectors(&self, collection: &str) -> Result<PulledBatch>`

This is the cache-prime path: the lake calls it once per cache miss
to fetch the full vector batch and compress it into RaBitQ codes.

The return value `PulledBatch` (per `crates/core/src/backend.rs:38`-area) is:

```rust
pub struct PulledBatch {
    pub collection: CollectionId,
    pub ids: Vec<u64>,         // one entry per vector
    pub vectors: Vec<Vec<f32>>, // parallel array; each must be length `dim`
    pub dim: usize,
    pub generation: u64,        // backend's coherence token at pull time
}
```

For ruQu backends, the "vectors" are the simulation result encoded as
a vector of f32. Per-backend encoding decisions:

- **StateVector:** `vectors[0]` is the amplitude vector with re/im
  interleaved. For n=10, `vectors[0].len() == 2 * (1 << 10) == 2048`.
  `dim = 2048`. `ids = vec![0]` (one row, the result).
- **Stabilizer:** `vectors[0]` is the tableau bit-packed into f32
  words (32 bits per word). For n=1000, `vectors[0].len() ==
  ceil(2 * n * n / 32) == 62500`. `dim = 62500`. `ids = vec![0]`.
- **Clifford+T:** `vectors[0..t_count]` is one row per significant
  stabilizer term; each row is the term's tableau (f32-packed) plus
  its complex coefficient prefixed. `dim` is term-tableau-size + 2.
  Truncation is signalled in the bundle's `lineage_id` (per
  `v2-spec.md` §i).
- **TensorNetwork:** `vectors[0..n]` is one row per MPS site tensor;
  each tensor is `(left_bond, phys_dim, right_bond)`-shaped, flattened
  with re/im interleaved. `dim` is the maximum tensor flattened size
  across the chain. Heterogeneous tensor sizes are handled by
  zero-padding to `dim`.
- **Hardware:** `vectors[0]` is the measurement histogram as a dense
  vector indexed by bitstring (0..2^n). For n=20, that's a 1M-entry
  vector. v0.1 caps at n=20 for hardware backends — larger circuits
  return a sparse-encoded histogram (top-K bitstrings only) and the
  `lineage_id` records the truncation. Operators who need full
  fidelity dispatch the run themselves and store the result via
  `lake.put_circuit_result()`.

`pull_vectors` is called *only* on cache miss. The lake's RaBitQ
compression then turns each `Vec<f32>` into a 1-bit code; the
amplitudes are *not* held verbatim in the lake — they're rerank
candidates for any future search.

The implication: a "search" against a ruQu-backend collection is
ANN over amplitude vectors. This is rarely what an operator wants
directly (you usually want the *amplitudes for circuit X*, not
"circuits whose amplitudes are similar to Y"); the more common
access path is `lake.cache_get_by_witness(witness)` which is a
direct-lookup, not a search.

#### `generation(&self, collection: &str) -> Result<u64>`

Returns the backend's coherence token at *now*, so the lake can
invalidate the cache when the token bumps.

Per `v2-spec.md` §d, the per-backend tick rules:

- **StateVector / Stabilizer / Clifford+T / TensorNetwork:** ticks
  on `ruqu-core` point release (manual bump via `bump_generation()`
  after upgrade) or operator-driven invalidation. The integer
  generation is *belt-and-braces* — the witness already covers the
  SIMD path and the precision mode through the `RuntimeContext`
  packing, so these backends rarely need a generation tick in
  practice.
- **Hardware:** ticks on calibration snapshot change. The backend
  polls the device's calibration API on a configurable interval
  (default: 60 s) and increments the integer generation when the
  snapshot id changes. This is the load-bearing tick for
  hardware-cache attribution.

```rust
// hardware.rs
fn generation(&self, _collection: &str) -> Result<u64> {
    let cal = self.cal_snapshot.read().unwrap();
    Ok(cal.monotonic_id())
}
```

`monotonic_id()` is a `u64` counter that increments every time the
backend observes a new calibration snapshot id. Two consecutive
calibrations with the same snapshot id (e.g. the device hadn't
recalibrated yet) return the same monotonic_id.

#### `current_bundle(&self, collection, rotation_seed, rerank_factor) -> Result<RuLakeBundle>`

The default impl (`crates/core/src/backend.rs:125`-area) does a `pull_vectors`
to get the dim, then synthesises a bundle. ruQu backends override
to *avoid* the pull on the bundle-fetch hot path:

```rust
// state_vector.rs
fn current_bundle(
    &self,
    collection: &str,
    rotation_seed: u64,
    rerank_factor: usize,
) -> Result<RuLakeBundle> {
    let cached = self.results.read().unwrap()
        .get(collection)
        .cloned()
        .ok_or_else(|| RuLakeError::InvalidParameter(
            format!("ruqu-state-vector: no cached run for {collection}")))?;

    // Pack the runtime context per v2-spec.md §c.
    let ctx = RuntimeContext {
        backend_id: "state_vector".into(),
        noise_id: cached.noise_id.clone(),
        decoder_id: "none".into(),
        decoder_params_hash: "".into(),
        mitigation_id: cached.mitigation_id.clone(),
        precision_mode: cached.precision_mode.clone(),
        simd_path: cached.simd_path.clone(),
        shots: cached.shots,
        seed: cached.seed,
        runtime_class: "simulated".into(),
    };

    Ok(RuLakeBundle::new(
        format!("ruqu://state_vector/{collection}"),
        cached.amplitudes.len() * 2,  // re+im interleaved
        rotation_seed,
        rerank_factor,
        Generation::Opaque(serde_json::to_string(&ctx)?),
    ))
}
```

The bundle is constructed from cached metadata, no pull required.
This is critical for the MCP server's `rulake://bundle/{b}/{c}`
resource (`crates/mcp-server/src/server.rs` resource registration area)
which needs to be O(1) — operators browse bundles in the Console
and cannot afford a pull per click.

#### `supports_pushdown(&self) -> bool`

All five ruQu backends return `false` in v0.1. Pushdown semantically
means "the backend can run a vector search itself rather than the
lake pulling all vectors and searching locally." For ruQu, the
"vectors" are simulation results, and the search-pushdown
interpretation is "simulate a circuit similar to this query and
return the result" — which is the entire ruQu API, not a pushdown.
The flag stays at the default `false` and v0.2 may revisit if a
similarity-of-circuits use case emerges.

### 1.4 What v0.1 ships vs v0.2 defers

**v0.1 ships:**

- `crates/ruqu-backend/` crate scaffold with the five module files.
- `state_vector.rs` and `stabilizer.rs` fully implemented behind
  `state-vector` and `stabilizer` features (default).
- `runtime_context.rs` with the `RuntimeContext` struct and the
  `noise_id` SHAKE-256(8) helper.
- `planner_cache.rs` with `CacheAwarePlanner` returning `Hit` /
  `Miss` per `v2-spec.md` §e.
- One round-trip integration test against `LocalBackend`-style
  harness (`tests/round_trip_state_vector.rs` for G2).
- One witness-equivalence property test
  (`tests/witness_equivalence.rs` for G1) — 100 circuits in CI,
  runs against 1000 in the nightly bench.
- `Cargo.toml` with the feature-flag matrix above.

**v0.2 defers:**

- `clifford_t.rs`, `tensor_network.rs`, `hardware.rs` adapter
  implementations. The trait impls compile against stub bodies that
  return `RuQuFeatureNotEnabled`.
- The hardware-cache attribution test (G3) — needs a mock device
  scaffold which is non-trivial and stalls v0.1 if pulled in.
- The Clifford concordance test (G4) — depends on Stabilizer (which
  ships in v0.1) but also on the mock-hardware backend (deferred).
- WASM-target build of `ruqu-backend`. v0.1 compiles for `x86_64-*`
  and `aarch64-*` only; the wasm story is deferred to v0.2 once
  feature-flag interactions with `wasm-bindgen` are nailed down.
- Tiered cache (T0/T1/T2 per `v2-spec.md` §p open question 4).

## 2. `crates/mcp-ruqu/` — sibling MCP server

Mirrors `crates/mcp-server/`'s shape exactly. The four key parallels:

1. **`#[tool_router]` + `#[tool(name=..., description=...)]` macro
   pattern** — see `crates/mcp-server/src/server.rs:189`. `mcp-ruqu` uses
   the same `rmcp` crate and the same macro.
2. **`AuditEntry` schema reuse** — `crates/mcp-server/src/audit.rs::AuditEntry`
   is *not* re-defined; `mcp-ruqu` depends on `mcp-server` as a
   library and imports the type directly. Disjoint code prefixes
   (`RUQU_*` vs `RULAKE_*`) prevent collision.
3. **JWT auth re-use** — `crates/mcp-server/src/auth.rs::scopes_to_caps` is
   imported and extended with the three new ruqu-specific
   capabilities (`simulate`, `hardware`, `verify`).
4. **Capability gating via `require_cap(&self.capabilities, ...)`** —
   identical pattern to `crates/mcp-server/src/server.rs:339` (`require_cap`
   call in `rulake_list_collections`).

### 2.1 Crate layout

```
crates/mcp-ruqu/
├── Cargo.toml
├── src/
│   ├── main.rs                # CLI: --capabilities, --transport, --jwt-jwks
│   ├── server.rs              # RuQuMcpServer struct, #[tool_router] block
│   ├── tools.rs               # Request/Response structs for the 5 tools
│   ├── planner_dispatch.rs    # Wires CacheAwarePlanner into the simulate tool
│   ├── audit_codes.rs         # The six RUQU_* codes from v2-spec.md §h
│   └── resources.rs           # MCP resources: ruqu://circuit/{hash}/* etc
└── tests/
    ├── tool_router_smoke.rs
    └── http_e2e.rs            # mirrors crates/mcp-server/tests/http_e2e.rs
```

### 2.2 `Cargo.toml`

```toml
[package]
name = "mcp-ruqu"
version = "0.0.1"
edition = "2021"
description = "MCP server exposing ruqu-backend simulation verbs as \
               capability-gated tools. Sibling crate; not in a \
               workspace, per ADR-001."

[dependencies]
mcp-server = { path = "../mcp-server", default-features = false,
               features = ["audit-only"] } # imports AuditEntry, scopes_to_caps
ruqu-backend = { path = "../ruqu-backend", features = ["state-vector", "stabilizer"] }
ruqu-core = { path = "../vendor/ruvector/crates/ruqu-core" }
rulake = "2.2"
rmcp = "0.x"   # same version as mcp-server
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
tokio = { version = "1", features = ["full"] }
clap = { version = "4", features = ["derive"] }

[features]
# By default mcp-ruqu serves SV + Stab only. Operators with hardware
# adapters in their ruqu-backend build re-enable here.
default = []
hardware = ["crates/ruqu-backend/hardware"]
clifford-t = ["crates/ruqu-backend/clifford-t"]
tensor-network = ["crates/ruqu-backend/tensor-network"]
all-backends = ["hardware", "clifford-t", "tensor-network"]
```

The `mcp-server` dep with `default-features = false` and an `audit-only`
feature is a small refactor of `mcp-server` itself: extract the
`AuditEntry` type and the JWT scope handling into a sub-module gated by
`audit-only` so siblings can depend on it without pulling the entire
ruLake-tool surface. This is a v0.1 prerequisite documented in
`ADR-008` §Decision 4 consequences.

### 2.3 The `RuQuMcpServer` struct + tool_router

```rust
// src/server.rs
use mcp_server::audit::{AuditEntry, AuditSink, PolicyDecision, now_ts};
use mcp_server::auth::{Capability, require_cap};
use rmcp::tool;
use rmcp::tool_router;
use rulake::Lake;
use ruqu_backend::{
    StateVectorBackend, StabilizerBackend,
    planner_cache::{CacheAwarePlanner, CacheAwarePlan},
    runtime_context::RuntimeContext,
};

pub struct RuQuMcpServer {
    lake: Arc<Lake>,
    planner: Arc<CacheAwarePlanner>,
    audit: Arc<dyn AuditSink>,
    capabilities: CapabilitySet,
}

#[tool_router(router = tool_router)]
impl RuQuMcpServer {

    #[tool(
        name = "ruqu_simulate",
        description = "Public ruQu intent: simulate a circuit on the \
                       planner-chosen backend, write result to lake, \
                       return (result, witness). Capability: simulate \
                       (or hardware for backend=hardware:*)."
    )]
    pub async fn ruqu_simulate(
        &self,
        Parameters(req): Parameters<SimulateRequest>,
    ) -> Result<Json<SimulateResponse>, McpError> {
        let start = std::time::Instant::now();
        let backend_str = req.backend.as_deref().unwrap_or("auto");

        // Capability check: hardware backends require the `hardware` cap.
        if backend_str.starts_with("hardware:") {
            require_cap(&self.capabilities, Capability::Custom("hardware"))?;
        } else {
            require_cap(&self.capabilities, Capability::Custom("simulate"))?;
        }

        // Parse QASM (or take Circuit directly).
        let circuit = if let Some(qasm) = &req.qasm {
            ruqu_core::qasm::parse_qasm3(qasm)
                .map_err(|e| McpError::invalid_params(
                    format!("RUQU_QASM_PARSE: {e}"), None))?
        } else if let Some(c) = &req.circuit {
            c.clone()
        } else {
            return Err(McpError::invalid_params(
                "RUQU_QASM_PARSE: neither qasm nor circuit provided", None));
        };

        // Build runtime context per v2-spec.md §c.
        let ctx = RuntimeContext::from_request(&req, &self.simd_path_active());

        // Planner: cache-aware dispatch.
        let plan = if req.no_cache {
            CacheAwarePlan::Miss {
                plan: ruqu_core::planner::plan_execution(&circuit, &Default::default()),
                anticipated_witness: ctx.anticipated_witness(&circuit, &self.lake),
            }
        } else {
            self.planner.plan(&circuit, &ctx)?
        };

        // Dispatch.
        let (result, bundle, cache_action) = match plan {
            CacheAwarePlan::Hit { backend_id, replay_witness } => {
                let cached = self.lake.cache_get_by_witness(&replay_witness)?
                    .ok_or_else(|| McpError::internal_error(
                        format!("RUQU_INTERNAL: cache hit metadata but no entry"), None))?;
                (cached.result, cached.bundle, CacheAction::Hit)
            }
            CacheAwarePlan::Miss { plan, anticipated_witness } => {
                let backend = self.dispatch_target(&plan.backend, backend_str)?;
                let r = self.run_on_backend(&backend, &circuit, &ctx).await?;
                let b = backend.current_bundle(
                    &hex::encode(ReplayEngine::circuit_hash(&circuit)),
                    self.lake.rotation_seed(),
                    self.lake.rerank_factor(),
                )?;
                self.lake.put_circuit_result(&b, &r)?;
                if req.no_cache {
                    (r, b, CacheAction::NoCacheBypass)
                } else {
                    (r, b, CacheAction::StoredFresh)
                }
            }
        };

        let elapsed = start.elapsed().as_secs_f64() * 1000.0;

        // Audit emit.
        self.audit.emit(AuditEntry {
            ts: now_ts(),
            transport: "stdio".into(),
            principal: "stdio:local".into(),
            session: None,
            request_id: None,
            tool: "ruqu_simulate".into(),
            intent: Some(format!("simulate:{}", backend_str)),
            outcome: "ok".into(),
            result_size: Some(/* dim */ bundle.dim as u32),
            trust_level: Some("verified".into()),
            duration_ms: elapsed,
            witness_in: None,
            witness_out: Some(bundle.rvf_witness.clone()),
            code: Some(match cache_action {
                CacheAction::Hit => "RUQU_CACHE_HIT".into(),
                CacheAction::StoredFresh => "RUQU_CACHE_STORED".into(),
                CacheAction::NoCacheBypass => "RUQU_NOCACHE_BENCH".into(),
            }),
            policy_decision: Some(PolicyDecision {
                capability_required: if backend_str.starts_with("hardware:") {
                    "hardware".into() } else { "simulate".into() },
                capability_granted: self.capabilities.labels()
                    .iter().map(|s| s.to_string()).collect(),
            }),
            decision: Some(serde_json::json!({
                "backend_chosen": bundle.runtime_context().backend_id,
                "cache_action": format!("{:?}", cache_action),
                "no_cache": req.no_cache,
            })),
        });

        Ok(Json(SimulateResponse { result, bundle, cache_action, elapsed_ms: elapsed }))
    }

    #[tool(
        name = "ruqu_verify",
        description = "Re-run a stored witness against a fresh execution \
                       and compare. Capability: verify."
    )]
    pub async fn ruqu_verify(...) -> Result<Json<VerifyResponse>, McpError> {
        require_cap(&self.capabilities, Capability::Custom("verify"))?;
        // Fetch stored bundle by witness; re-run circuit; compare result_hash.
        // On mismatch: emit RUQU_WITNESS_MISMATCH audit code per v2-spec.md §h.
        ...
    }

    #[tool(
        name = "ruqu_replay",
        description = "Zero-cost fetch from the lake by witness. \
                       Capability: verify (semantically read-only)."
    )]
    pub async fn ruqu_replay(...) -> Result<Json<ReplayResponse>, McpError> {
        require_cap(&self.capabilities, Capability::Custom("verify"))?;
        // O(1) lake lookup. Return Found or NotFound.
        ...
    }

    #[tool(
        name = "ruqu_optimize",
        description = "Run optimizer + decomposer + transpiler, return \
                       equivalent circuit. Capability: simulate."
    )]
    pub async fn ruqu_optimize(...) -> Result<Json<OptimizeResponse>, McpError> {
        require_cap(&self.capabilities, Capability::Custom("simulate"))?;
        // Wraps ruqu_core::optimizer::fuse_gates + transpiler + decomposition.
        // Cache the (input_circuit_hash, optimizer_passes) -> output mapping.
        ...
    }

    #[tool(
        name = "ruqu_qec_schedule",
        description = "Generate a surface-code schedule for the given \
                       distance and round count. Capability: simulate."
    )]
    pub async fn ruqu_qec_schedule(...) -> Result<Json<QecScheduleResponse>, McpError> {
        require_cap(&self.capabilities, Capability::Custom("simulate"))?;
        // Wraps qec_scheduler::generate_surface_code_schedule + optional
        // optimize_feed_forward + schedule_latency. Cache by (distance,
        // num_rounds, gate_time_ns, classical_time_ns) tuple.
        ...
    }
}
```

The structure mirrors `crates/mcp-server/src/server.rs:189`-area exactly.
The `audit.emit` shape is identical; only the codes change.

### 2.4 MCP resources — `ruqu://circuit/{hash}/{kind}`

In addition to `rulake://stats`, `rulake://stats/by-backend`,
`rulake://bundle/{b}/{c}`, `rulake://audit/tail` (per
`crates/mcp-server/src/server.rs` resource registration area), `mcp-ruqu`
adds:

| Resource URI | Returns | Use case |
|---|---|---|
| `ruqu://circuit/{hex}/qasm` | The OpenQASM 3.0 source for the cached circuit. | Console "show me the QASM" button on a Result Viewer entry. |
| `ruqu://circuit/{hex}/result` | The cached `SimulationResult` JSON (amplitudes, measurements, etc). | Programmatic fetch by agents that have the witness from a prior call. |
| `ruqu://circuit/{hex}/bundle` | The `RuLakeBundle` JSON. | Witness-anchored audit trail. Equivalent to `rulake://bundle/{ruqu-state-vector}/{hex}` but addressable by circuit hash directly. |
| `ruqu://schedule/{distance}/{rounds}` | The `QecSchedule` JSON for the given parameters. | Operators who want to inspect or visualise schedules without re-running `ruqu_qec_schedule`. |

All four are O(1) lookups (or near-O(1) — the QASM resource may
re-emit from the cached circuit if the QASM string itself isn't
cached, which is `O(num_gates)`).

### 2.5 What v0.1 ships vs v0.2 defers

**v0.1 ships:**

- `crates/mcp-ruqu/` crate scaffold.
- `ruqu_simulate` only, against the StateVector backend. No Stabilizer
  yet (it ships in v0.2 once the tableau-pull semantics are settled).
- `RUQU_CACHE_HIT`, `RUQU_CACHE_STORED`, `RUQU_QASM_PARSE`,
  `RUQU_INTERNAL` audit codes. The other six codes from `v2-spec.md`
  §h come online with their respective tools.
- One MCP resource: `ruqu://circuit/{hex}/bundle`. The other three
  defer to v0.2.
- New JWT scope: `mcp:ruqu:simulate` granting the `simulate`
  capability. `mcp:ruqu:hardware` and `mcp:ruqu:verify` defer to
  v0.2 with their tools.
- `audit-only` feature on `mcp-server` — the small refactor that
  makes `AuditEntry` and JWT scope handling depable from a sibling.

**v0.2 defers:**

- `ruqu_verify`, `ruqu_replay`, `ruqu_optimize`, `ruqu_qec_schedule`
  tool implementations.
- Stabilizer backend dispatch in `ruqu_simulate`.
- `hardware:*` backend dispatch + the `hardware` capability.
- The remaining MCP resources.
- HTTP e2e test (mirrors `crates/mcp-server/tests/http_e2e.rs`); v0.1 ships
  a stdio-only smoke test.

## 3. Console hooks — the "Quantum" route

Per `docs/adrs/ADR-006-rulake-console-vite-github-pages.md`, the
Console at `ui/` is a Vite + React app deployed to GitHub Pages with
three modes (Demo / WASM-local / Live). Six routes today: Stats,
Playground, Backends, Bundle, Audit, Connect.

v2 adds **Quantum** as a 7th peer route, defended in `v2-spec.md` §k.

### 3.1 Component decomposition

```
ui/src/
├── components/
│   └── screens.jsx          # the existing 6-route switcher; add Quantum case
├── routes/quantum/
│   ├── QuantumRoute.tsx     # entry, mode-aware (Demo/WASM-local/Live)
│   ├── Composer.tsx         # QASM textarea editor + run button
│   ├── ResultViewer.tsx     # probability histogram + measurements
│   ├── BundleEmbed.tsx      # reuses existing Bundle viewer for verify
│   └── hooks/
│       ├── useRuQuWasm.ts   # lazy-init ruqu-wasm
│       └── useFederation.ts # opt-in remote federation lookup
└── lib/
    └── ruqu-client.ts       # wrapper over fetch('/mcp-ruqu/*')
```

### 3.2 What the user sees

**Composer screen.** A QASM 3.0 textarea (initial content: a Bell-state
example), backend dropdown (`auto` default, plus all backends the
remote `mcp-ruqu` reports through `ruqu_list_backends` — a v0.2 tool;
v0.1 hard-codes the dropdown to `["auto", "state_vector"]`), shots
slider (1-1024 in v0.1), seed input, Run button.

The Run button:

1. In **Demo mode**: shows a pre-canned result (Bell state ~50/50).
   No actual computation.
2. In **WASM-local mode**: calls `ruqu-wasm::simulate`, computes the
   bundle witness with `rulake-wasm::computeWitness`, displays the
   result. Bundle is *not* sent to a remote server.
3. In **Live mode**: POSTs to `/mcp-ruqu/ruqu_simulate`, receives the
   `SimulateResponse`, displays the result and bundle.

**Result Viewer screen.** Shows:

- A probability bar chart over bitstrings (top 16 by probability;
  scroll for the rest).
- The measurement table (qubit, result, probability) for measured
  circuits.
- A `BundleEmbed` collapsible panel that renders the full bundle
  JSON and runs `verifyBundleJson` on it (same wasm path as the
  Bundle route).
- For Live mode: a "Verify against remote" button that POSTs
  `ruqu_verify` and shows the response (matches / mismatch /
  expected vs computed witness).
- For cached results: a banner reading "Served from cache;
  original dispatch on YYYY-MM-DD; original audit id: <ULID>"
  per `v2-spec.md` §m. Operators who need to know this is *not* a
  fresh hardware result cannot miss the banner.

### 3.3 Reuse vs new code

**Reused as-is from existing Console:**

- `ui/src/lib/wasm-mode.ts` (already loads `rulake-wasm` lazily;
  extend to load `ruqu-wasm` in parallel for the Quantum route).
- The existing Bundle viewer component — `BundleEmbed` is a thin
  wrapper that passes the bundle JSON in as a prop.
- The mode switcher (Demo / WASM-local / Live).
- The CSS/layout primitives.

**New in v0.1:**

- `QuantumRoute.tsx` (~80 LOC).
- `Composer.tsx` (~120 LOC) — textarea + dropdown + slider +
  button + state.
- `ResultViewer.tsx` (~150 LOC) — chart + table + cache-served
  banner + verify button.
- `useRuQuWasm.ts` (~30 LOC) — lazy-init.
- `useFederation.ts` (~40 LOC) — optional federation lookup.
- `ruqu-client.ts` (~60 LOC) — typed fetch wrappers.

Total new TSX/TS: ~480 LOC. Within the ADR-006 "small, surveyable
UI" budget; defended in `v2-spec.md` §k.

### 3.4 Sidebar decision

The 7th sidebar entry sits between **Bundle** and **Audit**:

```
[Sidebar order]
1. Stats
2. Playground
3. Backends
4. Bundle
5. Quantum     <-- new
6. Audit
7. Connect
```

Rationale: Quantum produces bundles (so it sits adjacent to Bundle)
and emits audit rows (so it sits adjacent to Audit). The user flow
"compose circuit → run → see result → audit the dispatch" maps
naturally to a left-to-right sidebar walk.

### 3.5 What v0.1 ships vs v0.2 defers

**v0.1 ships:**

- `QuantumRoute.tsx` with Composer + ResultViewer screens.
- Demo + WASM-local modes only. WASM-local uses `ruqu-wasm`'s
  StateVector backend directly.
- Backend dropdown hard-coded to `["auto", "state_vector"]`.
- Bundle embed for in-tab witness verification.
- The cache-served banner (read from response metadata; in WASM-local
  mode this is always "fresh in-tab" since there's no cache).

**v0.2 defers:**

- Live mode — needs `mcp-ruqu` running and a configured endpoint;
  v0.1 ships the route but the Live toggle is disabled with a
  tooltip "configure mcp-ruqu endpoint in Connect first."
- Federation viewer — the `useFederation` hook ships in v0.1 as a
  no-op (returns `[]`); the UI to render federated holders defers
  to v0.2.
- The drag-and-drop gate palette per `v2-spec.md` §k. v0.1 is
  textarea-only.
- Verify button in Live mode (depends on `ruqu_verify` shipping in
  `mcp-ruqu` v0.2).

## 4. End-to-end ship checklist

The PRs that follow ADR-008 acceptance, in order:

1. **PR #1 — `mcp-server` audit-only feature.** Refactor `mcp-server`
   to expose `AuditEntry` and `scopes_to_caps` behind a
   `default-features = false, features = ["audit-only"]` mode.
   No behaviour change to existing `mcp-server` consumers (the
   default still includes the full ruLake-tool surface). ~50 LOC
   diff.

2. **PR #2 — `crates/ruqu-backend/` v0.0 scaffold.** Crate, Cargo.toml,
   feature flags, `runtime_context.rs`, `state_vector.rs`,
   `stabilizer.rs` (impl `BackendAdapter` only — full body deferred
   to PR #4), `tests/round_trip_state_vector.rs` (G2 smoke).
   ~600 LOC.

3. **PR #3 — `crates/mcp-ruqu/` v0.0 scaffold.** Crate, Cargo.toml,
   `server.rs` with `ruqu_simulate` against StateVector,
   `tests/tool_router_smoke.rs`. ~400 LOC.

4. **PR #4 — `state_vector.rs` full body.** The `CachedRun` struct,
   `simd_path_active()` detection, `noise_id` SHAKE-256(8)
   computation, `current_bundle` packing. Witness equivalence test
   (G1) ships here. ~300 LOC.

5. **PR #5 — Console Quantum route v0.1.** `QuantumRoute.tsx`,
   `Composer.tsx`, `ResultViewer.tsx`, lazy-load of `ruqu-wasm`,
   sidebar entry. ~500 LOC.

After these five PRs, v0.1 is shippable. v0.2 work (the deferred
items above) follows operator feedback from v0.1 deployments.

The first PR after ADR-008 acceptance is PR #1 (the `mcp-server`
audit-only refactor) — small, low-risk, unblocks PRs #2-#5.
