# ruQu v2 Research Corpus

A four-document set that takes the original "ruQu already speaks witness;
ruLake already cache-shares by witness — bind them" thesis and turns it
into a shippable v2 spec, a concrete integration plan, and a single
Architecture Decision Record that both bind. The corpus does not fork
the v1 simulation backends; it ratifies the ruQu v1 engine at
`vendor/ruvector/crates/ruqu-core/` as a five-faced ruLake substrate,
fixing the gaps that prevented v1's hash-chained witness log from
participating in cross-process, cross-deployment cache reuse.

## 30-second pitch

ruQu v1 (`vendor/ruvector/crates/ruqu-core/`, 26 093 lines across 30
source files) is a Rust quantum execution intelligence engine: five
simulation backends (StateVector, Stabilizer, Clifford+T, TensorNetwork,
Hardware), a cost-model planner that picks among them
(`vendor/ruvector/crates/ruqu-core/src/planner.rs`), an OpenQASM 3.0
exporter, a QEC control plane with surface-code scheduling
(`vendor/ruvector/crates/ruqu-core/src/qec_scheduler.rs`), and — load-
bearing for this corpus — an **already-cryptographic** execution-record
layer at `vendor/ruvector/crates/ruqu-core/src/replay.rs::ExecutionRecord`
plus a hash-chained `WitnessLog` at
`vendor/ruvector/crates/ruqu-core/src/witness.rs`. ruLake is a
witness-anchored vector cache that has shipped four backends (Local,
Fs, GCS, IPFS), an MCP server with eight tools, and a console — all
built around a SHAKE-256 bundle witness at
`src/bundle.rs::compute_witness` that makes any cache entry content-
addressable across deployments.

v2 closes a single missing seam: every ruQu execution emits a ruLake-
compatible bundle JSON, the five simulation engines each register as a
`BackendAdapter` (`src/backend.rs:110`), the cost-model planner becomes
*cache-aware* (`src/lake.rs:127` `cache_stats_by_backend`), and a
sibling `mcp-ruqu` server exposes the quantum verbs through the same
capability-gated tool surface that ruLake's MCP server uses. The result
is the system the v1 README implies but the v1 architecture cannot
reach without external glue: ruQu as the *immutable simulation
artefact*, ruLake as the *adaptive retrieval layer*, ruvector as the
reasoning loop. The "two processes, same circuit, second one returns
the cached witness in <1 ms" validation test from the brief becomes one
of five acceptance gates in ADR-008.

## Files in this directory

| Path                                         | Lines | What it does                                                                       |
|----------------------------------------------|------:|------------------------------------------------------------------------------------|
| `README.md` (this file)                      |  ~150 | Index, persona reading guides, compositional thesis, what's NOT in the corpus      |
| `v2-spec.md`                                 | ~1900 | Canonical ruQu v2 spec — 5 backends as `BackendAdapter`, witness equivalence       |
| `integration-with-rulake.md`                 | ~1000 | Code-level integration: `ruqu-backend/`, `mcp-ruqu/`, Console "Quantum" route hooks |
| `ADR-008-ruqu-as-rulake-substrate.md`        |  ~750 | The decision record that binds the spec and the integration plan together         |

Read order is the order they appear above. Each file cites real paths
into v1 (`vendor/ruvector/crates/ruqu-core/`) and ruLake (`src/`,
`mcp-server/src/`, `docs/adrs/`). No invented APIs.

## The compositional thesis (one paragraph)

ruQu's v1 `ExecutionRecord` (`vendor/ruvector/crates/ruqu-core/src/replay.rs:57`)
already covers `circuit_hash`, `seed`, `backend`, `noise_config`,
`shots`, `software_version`, `timestamp_utc`. Its `WitnessLog`
(`vendor/ruvector/crates/ruqu-core/src/witness.rs:88`) hash-chains
those records into a tamper-evident audit trail using a 32-byte digest
built from `DefaultHasher` four times. ruLake's `RuLakeBundle`
(`src/bundle.rs:113`) holds `data_ref`, `dim`, `rotation_seed`,
`rerank_factor`, `generation`, plus a `rvf_witness` SHAKE-256(32) over
all of them (`src/bundle.rs:362` `compute_witness`). v2 makes these
the *same* witness: the ruQu execution-record fields fold into the
bundle's `data_ref` (the URI `ruqu://<backend>/<circuit_hash>`) and
`generation` (a struct-tag that pins backend id, noise model id,
decoder id, mitigation id, mixed-precision mode, SIMD path), with the
SHAKE-256 digest replacing the v1 `DefaultHasher` chain. Two
identical re-runs of the same circuit on the same backend produce
byte-identical bundles, so ruLake's content-addressed cross-process
dedup (`src/cache.rs::CacheKey`, the witness-keyed compressed entry
table) returns the cached result for free on the second call. No new
code in user space; no fork of ruqu-core; the witness gets *stronger*
(SHAKE vs SipHash); the federation surface (`src/lake.rs:521`
`search_federated`) becomes "ask all five quantum backends for prior
runs of this circuit" by changing zero lines in the federation API.

## Reading guide by persona

### Persona 1: Quantum researcher (reproducibility-first)

You ran a 22-qubit VQE three months ago, you need to defend the result
to a reviewer, and the seed-tied measurements have to match bit-for-bit.
You care about:

1. **The witness equivalence theorem** in `v2-spec.md` §c. v1's
   `ReplayEngine::circuit_hash`
   (`vendor/ruvector/crates/ruqu-core/src/replay.rs:183`) and ruLake's
   `compute_witness` (`src/bundle.rs:362`) become one digest with a
   single domain separator. Same inputs → same hex string, on any
   workstation, in any process.
2. **What goes into the witness** in `v2-spec.md` §i. Decoder choice
   ticks the witness, noise model ticks the witness, the SIMD path
   (scalar / AVX2 / NEON) ticks the witness because float-order-of-ops
   changes amplitudes by ULPs and our reproducibility claim is bit-
   exact. Honest about the implications: SIMD path-portability is
   *not* a property v2 claims.
3. **Where to start in `v2-spec.md`**:
   - §a Status & supersession (which v1 promises hold verbatim)
   - §c Witness equivalence theorem (the field-mapping table)
   - §i QEC + noise + mitigation in the witness
   - §m Open quantum-supremacy + benchmarking (the "is a cache hit a
     valid scientific result?" question, answered honestly)
4. **Where to start in `ADR-008`**: §Decisions 1 (witness mapping), 4
   (SIMD-path documentation as a Consequence), 6 (`--no-cache`
   discipline). §Verification gate G4 (Clifford-only stabilizer-sim ↔
   hardware-sim concordance test).

### Persona 2: Platform engineer wiring ruQu into agents

You're building agentic systems that call quantum simulation as a
sub-routine — VQE for chemistry, Grover for unstructured lookup, QAOA
for combinatorial planning — and you need the calls to be MCP tools
with capability tiers, audit trails, and zero-recompute on repeat. You
care about:

1. **The MCP tool surface** in `v2-spec.md` §h. Five verbs
   (`ruqu_simulate`, `ruqu_verify`, `ruqu_replay`, `ruqu_optimize`,
   `ruqu_qec_schedule`) that mirror the shape of `rulake_query` at
   `mcp-server/src/server.rs:189`. Capability-gated. JSON-schema'd.
   Audit-logged with codes from a six-code refusal vocabulary in §h.
2. **How v2 plugs into the existing MCP plane**: `mcp-ruqu/` is a
   sibling crate of `mcp-server/`, not an extension. The two servers
   share the `mcp-server/src/audit.rs::AuditEntry` schema with disjoint
   code prefixes (`RULAKE_*` vs `RUQU_*`). One log pipeline; two
   tool families. JWT scopes from the same provider — see
   `mcp-server/src/auth.rs::scopes_to_caps` — gate both.
3. **Where to start in `v2-spec.md`**: §g pipeline (the five intent
   verbs), §h MCP tools, §o Migration from v1 (so existing
   `WitnessLog`s can be re-derived as v2 bundles).
4. **Where to start in `integration-with-rulake.md`**: the `mcp-ruqu/`
   section and the Console hooks section (the proposed Quantum route).
5. **Where to start in `ADR-008`**: §Decisions 4 (MCP tool surface)
   and 6 (`--no-cache` discipline); §Verification gate G2
   (cross-process replay sub-1 ms).

### Persona 3: Operator running `mcp-ruqu` in production

You run the service that other teams call. You owe an SLO, an audit
trail, a clear answer to "is this circuit too expensive to run?", and
a precise story for hardware-backend dispatch (because IBM-Heron-Q3
calibrations drift mid-run and your finance team will ask why a
"cached" answer cost $40). You care about:

1. **The `runtime_class` field** in `v2-spec.md` §l. The bundle's
   `runtime_class` distinguishes `simulated` from
   `hardware:<provider>:<device>:<calibration-snapshot>` so a cache
   hit on a hardware-run can be authoritatively attributed.
   Calibration-snapshot drift is a witness-affecting change.
2. **The cache-aware planner** in `v2-spec.md` §e. The v1 cost-model
   planner (`vendor/ruvector/crates/ruqu-core/src/planner.rs:213`
   `plan_execution`) becomes v2's first lookup against
   `lake.cache_stats_by_backend` (`src/lake.rs:127`) — re-runs cost
   zero before they hit any simulator. Operator wins: cache pinning
   and per-backend hit-rate visibility through the existing
   `rulake://stats/by-backend` MCP resource.
3. **The audit shape** in `v2-spec.md` §h. `RUQU_WITNESS_MISMATCH`,
   `RUQU_HARDWARE_REFUSED`, `RUQU_PRECISION_DEGRADED`,
   `RUQU_CACHE_MISS_OVER_BUDGET`, `RUQU_BACKEND_UNAVAILABLE`,
   `RUQU_QEC_DECODER_TIMEOUT` — six refusal codes, mirroring the
   shape of ruLake's `WITNESS_MISMATCH_REFUSED` at
   `mcp-server/src/server.rs:255`-area.
4. **Where to start in `v2-spec.md`**: §e cache-aware planner, §h
   MCP tools, §l Hardware backend + audit chain, §m benchmarking
   policy.
5. **Where to start in `ADR-008`**: §Decisions 3 (cache-aware
   planner), 5 (`runtime_class`), 6 (`--no-cache`). §Verification
   gates G3 (hardware-cache attribution test) and G5 (audit-log
   round-trip parity with ruLake's).

## Compositional fit vs ruLake — what's the same shape, what's not

| Question | ruLake answer | ruQu v2 answer | Same?                |
|----------|---------------|----------------|----------------------|
| What anchors a cache entry? | SHAKE-256(32) over (data_ref, dim, rotation_seed, rerank_factor, generation) — `src/bundle.rs:362` | SHAKE-256(32) over the same fields, where `data_ref = ruqu://<backend>/<circuit_hash>` and `generation` packs (backend_id, noise, decoder, mitigation, precision, simd_path, runtime_class) | Yes — same digest, same domain prefix family |
| Where do bundles live? | `table.rulake.json` sidecar; published via `rulake_publish_bundle` (`mcp-server/src/server.rs:365`) | `circuit.rulake.json` sidecar with the *same* `RuLakeBundle` struct; published via `ruqu_simulate`'s post-run hook | Yes — one struct, two emitters |
| How does federation work? | `lake.search_federated` parallel fan-out across `(backend, collection)` pairs (`src/lake.rs:521`) | Same call, where backends are the five simulation engines and "collections" are sets of cached circuit-witnesses | Yes — one API, two interpretations of what a backend is |
| What's the browser story? | Console at `ui/`, three modes (Demo, WASM-local, Live); `rulake-wasm@2.2.1` does `verifyBundleJson`, `computeWitness`, `searchBruteForceL2` | Same Console; new "Quantum" route uses `ruqu-wasm` (`vendor/ruvector/crates/ruqu-wasm/`) for in-tab simulation, then `rulake-wasm` for witness verification — zero new wasm code | Yes — composition of two existing wasm packages |
| What does an operator stare at? | `rulake://stats`, `rulake://stats/by-backend`, `rulake://bundle/{b}/{c}`, `rulake://audit/tail` | Same four resources; `rulake://stats/by-backend` now shows StateVector / Stabilizer / Clifford+T / TensorNetwork / Hardware as backend ids | Yes — same MCP resources, broader content |

## What's NOT in this corpus

- **A `cargo` build of `ruqu-backend/`.** ADR-008 lands first; the
  scaffold is the next commit AFTER ADR-008 acceptance. See
  `integration-with-rulake.md` §"What v0.1 ships vs v0.2 defers" for
  exactly what the first PR contains.
- **A fork of `ruqu-core`'s simulation engines.** All five backends
  stay where they are (`vendor/ruvector/crates/ruqu-core/src/{state,
  stabilizer, clifford_t, tensor_network, hardware}.rs`). v2 wraps
  them; it does not modify them.
- **A new circuit-IR.** `QuantumCircuit` at
  `vendor/ruvector/crates/ruqu-core/src/circuit.rs` and OpenQASM 3.0
  export at `vendor/ruvector/crates/ruqu-core/src/qasm.rs` remain the
  authoritative IR pair. v2 cache-keys the circuit by its existing
  hash (`vendor/ruvector/crates/ruqu-core/src/replay.rs:183`
  `circuit_hash`) and the QASM is re-emittable from cache without
  recompute.
- **A claim that v2 makes hardware-backend runs "fair benchmarks"
  when served from cache.** The `--no-cache` flag and the `verify`
  intent (`v2-spec.md` §m) explicitly preserve "always-re-run"
  semantics for benchmarking and supremacy claims. We are not in the
  business of laundering cache hits as fresh hardware results.

## What ships next (after this corpus lands)

1. ADR-008 review + acceptance.
2. `ruqu-backend/` v0.0 scaffold — `Cargo.toml`, the five
   `BackendAdapter` impls (one per ruqu-core backend), one passing
   round-trip test that primes a Bell-state circuit on the
   `StateVector` backend and re-reads it from cache via the witness.
3. `mcp-ruqu/` v0.0 — `ruqu_simulate` only, witness-pinned,
   capability-gated by a new `mcp:ruqu:simulate` JWT scope.
4. Console: a 7th sidebar entry (`Quantum`) that surfaces the
   `rulake://bundle/{ruqu_backend}/{circuit_hash}` resource and runs a
   witness-verify round-trip against the existing `node-wasm/`
   `verifyBundleJson` path. Implementation: see
   `integration-with-rulake.md` §Console hooks.

The v0.1 boundary is deliberately tight. Five backends × five intent
verbs × the QEC scheduler is enough surface to be wrong about; we
ship the smallest end-to-end loop first and grow from there.
