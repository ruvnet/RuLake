# ADR-008: ruQu v2 — Quantum execution intelligence as a ruLake substrate

## Status

**Proposed (2026-04-27)** — no `ruqu-backend/` or `mcp-ruqu/` crates
yet. This ADR fixes the shape *before* the first PR opens so we don't
relitigate the witness-mapping, the SIMD-path-affects-witness call,
the cache-aware planner refactor, or the runtime_class hardware-
attribution discipline at code review time. Sister ADR to ADR-007
(rvDNA v2 — being authored in parallel under
`docs/research/rvdna/ADR-007-rvdna-as-rulake-substrate.md` per the
parallel rvDNA agent's brief) and to the in-flight ruLake ADR family.

## Date

2026-04-27

## Authors

ruv.io · ruvector engineering. Drafted alongside the v2 spec and
integration plan in `docs/research/ruqu/{v2-spec.md,
integration-with-rulake.md}`. The three documents land together as
one corpus per the operator's brief; ADR-008 is the binding artefact
that pins the spec's witness equivalence claim and the integration
plan's PR sequence to a single decision record.

## Relates To

- **ADR-001** (`docs/adrs/ADR-001-standalone-repo-strategy.md`) —
  sibling crate layout, no root workspace, vendored submodule
  discipline. `ruqu-backend/` and `mcp-ruqu/` slot in next to
  `gcs-backend/`, `ipfs-backend/`, and `mcp-server/` under the same
  rules. No `*.workspace = true`, no shared `Cargo.toml` at the
  repo root.
- **ADR-005** (`docs/adrs/sdk/ADR-005-ipfs-backend-and-deploy.md`) —
  the bundle-only IPFS backend pattern v2 reuses for cross-site
  circuit-witness sharing. `circuit.rulake.json` bundles publish to
  IPFS; the amplitude bytes stay on whatever backend the dispatching
  node ran the simulation on. v2 inherits this pattern verbatim.
- **ADR-006** (`docs/adrs/ADR-006-rulake-console-vite-github-pages.md`)
  — the Console architecture v2's "Quantum" route plugs into. The
  route extension follows ADR-006's "small, surveyable UI" budget;
  ~480 LOC of new TSX/TS in v0.1.
- **ADR-155** (`docs/adrs/ADR-155-rulake-datalake-layer.md`) — the
  cache-first, witness-as-anchor framing this ADR consumes. The M2-M5
  backend roadmap explicitly anticipates content-addressed backends;
  ruqu-backend is the first one whose "content" is a *quantum
  simulation result* rather than a vector dataset.
- **ADR-156** (`docs/adrs/ADR-156-rulake-as-memory-substrate.md`) —
  the memory-class tagging mechanism. v2 uses `memory_class:
  Some("quantum-simulation")` on every emitted bundle so operators
  can filter / route by class. This is opaque to ruLake (per
  ADR-156); semantic to the agent layer.
- **`docs/research/ruqu/v2-spec.md`** — the canonical spec this ADR
  ratifies.
- **`docs/research/ruqu/integration-with-rulake.md`** — the
  code-level integration plan.
- **The parallel `docs/research/rvdna/ADR-007-rvdna-as-rulake-substrate.md`**
  — sibling decision record being authored in parallel by the rvDNA
  agent. Same shape (X as ruLake substrate); different X. Both
  ADR-007 and ADR-008 are first instances of a "third-party data
  format / engine as a ruLake substrate" pattern that ADR-001 +
  ADR-155 implicitly anticipate.
- **v1 ruQu reference points:**
  - `vendor/ruvector/crates/ruqu-core/README.md` — the v1 pitch.
  - `vendor/ruvector/crates/ruqu-core/src/replay.rs::ExecutionRecord`
    (line 57) — the v1 execution-record we fold into the bundle.
  - `vendor/ruvector/crates/ruqu-core/src/witness.rs::WitnessLog`
    (line 88) — the v1 hash-chain we replace with bundle chains.
  - `vendor/ruvector/crates/ruqu-core/src/planner.rs::plan_execution`
    (line 213) — the v1 cost-model planner we wrap with cache-aware
    dispatch.
  - The published v1 ADRs are referenced from the README files at
    `vendor/ruvector/crates/ruqu-{core,algorithms,exotic,wasm}/README.md`
    (ADR-QE-001, ADR-QE-003, ADR-QE-004, ADR-QE-005, ADR-QE-006,
    ADR-QE-007, ADR-QE-008, ADR-QE-012, ADR-QE-014, ADR-QE-015) —
    none of these live in the RuLake repo; they are the upstream
    record for the engine. v2 leaves all of them in force.

---

## Context

ruLake's `BackendAdapter` trait (`src/backend.rs:110`) is what every
data lake plugs into. Today we have `LocalBackend`, `FsBackend`,
`GcsParquetBackend`, and `IpfsBackend`. Each one wraps a *vector
data source* — Parquet on cloud storage, an IPFS-published bundle,
an in-memory test substrate. The shape of the trait is four required
methods (`id`, `list_collections`, `pull_vectors`, `generation`)
plus an optional `current_bundle` override that ruLake's MCP server
relies on for the `rulake://bundle/{b}/{c}` resource.

ruQu is a different shape. ruqu-core
(`vendor/ruvector/crates/ruqu-core/`, 26 093 lines across 30 source
files) is a quantum execution engine: five simulation backends
(StateVector, Stabilizer, Clifford+T, TensorNetwork, Hardware), a
cost-model planner, an OpenQASM 3.0 exporter, a QEC control plane,
SIMD acceleration, multi-threading, mixed precision, noise + error
mitigation, and — load-bearing for this ADR — a *cryptographic*
execution-record layer (`ExecutionRecord` + `WitnessLog`) that hash-
chains every run for tamper-evident reproducibility.

There are five forces pushing ruqu-backend onto the v0.1 list:

1. **The witness already exists.** v1's `WitnessLog`
   (`vendor/ruvector/crates/ruqu-core/src/witness.rs:88`) is a
   tamper-evident chain of `ExecutionRecord` entries with 32-byte
   self-hashes computed via `DefaultHasher` (SipHash) four times.
   ruLake's `RuLakeBundle::rvf_witness` (`src/bundle.rs:362`
   `compute_witness`) is a 32-byte SHAKE-256 over the bundle fields.
   Both are 32-byte digests over a deterministic concatenation of
   execution-affecting inputs. The structural similarity is too
   sharp to ignore; the question is whether the v1 entropy fits
   under the v2 hash without semantic loss. v2-spec.md §c argues it
   does.

2. **The cost-model planner is asking to be cache-aware.** The v1
   planner at `vendor/ruvector/crates/ruqu-core/src/planner.rs:213`
   `plan_execution` predicts memory and runtime per backend and
   picks the cheapest. *Cheapest* is dominated by simulation cost.
   On second call with identical inputs, the *cheapest* backend is
   trivially "the cache" — but v1 has no cache, so the planner
   re-simulates. Adding a cache pre-pass via
   `lake.cache_stats_by_backend` (`src/lake.rs:127`) makes the
   second call free. The change is local to a wrapper struct; no
   v1 planner code is modified.

3. **The federation primitive already does the right thing.**
   `lake.search_federated` (`src/lake.rs:521`) is a parallel
   rayon fan-out across `(backend, collection)` pairs. Apply that
   to "(simulation backend, circuit hash)" pairs and federation
   becomes "ask all five quantum engines, in parallel, for prior
   runs of this circuit." Zero new federation code.

4. **The browser story composes existing wasm.** `ruqu-wasm`
   (`vendor/ruvector/crates/ruqu-wasm/`) ships a 25-qubit
   StateVector backend in 180 KB gzip. `rulake-wasm` (the
   ru-vendor's `node-wasm` crate) ships `verifyBundleJson`,
   `computeWitness`, and `searchBruteForceL2`. v2's Console
   "Quantum" route is the composition; zero new wasm code.

5. **The hardware-cache attribution problem is real.** When v2
   dispatches to a real device, the cache hit on a subsequent
   identical call is *not* a fresh hardware result. Cost
   attribution, compliance, and quantum-supremacy claims all
   depend on operators being able to distinguish "served from
   cache" from "fresh dispatch." v2-spec.md §l + §m specify a
   `runtime_class` field plus a `--no-cache` flag that make the
   distinction unambiguous in every artefact (bundle, audit row,
   Console UI).

The decision is whether to take all five of these forces seriously
in one shot — by ratifying v2 — or to let them accrete one-off as
external glue scripts. The latter has been the path so far; this
ADR ends it.

### Why this ADR matters more than the integration code

The integration code in `integration-with-rulake.md` is a 5-PR
sequence over ~2300 LOC. Big enough to be wrong about; small enough
that the cost of being wrong is "rewrite the affected adapter or
tool." The ADR's job is to lock the *witness mapping*, the
*SIMD-path-affects-witness call*, the *runtime_class field*, and
the *--no-cache discipline* before any of those PRs open — because
those four decisions can't be retrofitted without invalidating
every cache entry written before the change.

---

## Decision

We adopt ruQu v2 as a first-class ruLake substrate per the spec at
`docs/research/ruqu/v2-spec.md` and the integration plan at
`docs/research/ruqu/integration-with-rulake.md`. Six numbered
sub-decisions:

### Decision 1: Witness mapping — the v1 ExecutionRecord folds into the v2 RuLakeBundle

The bijection in v2-spec.md §c is the canonical mapping. The packing
formula:

```
data_ref      = "ruqu://" + backend_id + "/" + hex(circuit_hash)
dim           = (per-backend; see v2-spec.md §d)
rotation_seed = lake.rotation_seed (lake-instance constant)
rerank_factor = lake.rerank_factor (lake-instance constant)
generation    = Generation::Opaque(serde_json(RuntimeContext))

where RuntimeContext = {
  backend_id, noise_id, decoder_id, decoder_params_hash,
  mitigation_id, precision_mode, simd_path, shots, seed, runtime_class
}
```

`compute_witness` (`src/bundle.rs:362`) over those five inputs
produces the v2 `rvf_witness`. The v1 `WitnessLog` chain semantic
moves to a companion `chain.rulake.json` sidecar (one entry per
bundle, each with a `prev_witness` field). The v1 hash function
(`DefaultHasher`/SipHash) is replaced by SHAKE-256 — a strict
upgrade in collision resistance.

**The v1 fields `software_version` and `timestamp_utc` move to bundle
metadata (`lineage_id`) and are NOT witness inputs.** This is a
deliberate semantic relaxation; v1's "two records with same hash →
identical software" property is weaker in v2 (the property becomes
"two bundles with same witness → identical witness-affecting
inputs," and software version is reclassified as not witness-
affecting unless the SIMD-path id implicitly captures the change).

### Decision 2: Five sibling BackendAdapter impls in one ruqu-backend crate

Per `integration-with-rulake.md` §1.1, we ship five separate
`BackendAdapter` implementations (`StateVectorBackend`,
`StabilizerBackend`, `CliffordTBackend`, `TensorNetworkBackend`,
`HardwareBackend`) inside one `ruqu-backend/` crate, gated by
five Cargo feature flags. Operators with WASM-target deployments
get StateVector + Stabilizer by default; the other three are opt-in
behind features that pull in their respective dependencies.

The single union-impl-with-discriminator alternative is rejected;
defence in `integration-with-rulake.md` §1.1.

### Decision 3: Cost-model planner becomes cache-aware (non-breaking ruLake addition)

The v1 cost-model planner (`vendor/ruvector/crates/ruqu-core/src/planner.rs:213`)
gets wrapped by `CacheAwarePlanner` in `ruqu-backend/`. The wrapper:

1. Computes the anticipated witness from inputs.
2. Queries `lake.cache_stats_by_backend()` (`src/lake.rs:127`) for
   any backend currently holding the witness.
3. Returns `Hit { backend_id, witness }` on cache hit.
4. Falls back to the v1 `plan_execution` on miss, returning `Miss
   { plan, anticipated_witness }`.

This requires one non-breaking ruLake addition: `PerBackendStats`
gains a method (or field) returning the set of witnesses currently
held by that backend. Concrete shape:

```rust
// In src/cache.rs, on PerBackendStats:
pub fn witnesses_held(&self) -> &HashSet<String> { &self.witnesses }
```

The set is already maintained internally; this exposes it. v0.1
ships this addition as part of PR #2 (the `ruqu-backend` scaffold)
because the wrapper depends on it.

### Decision 4: Five new MCP tools in a sibling mcp-ruqu crate

Per `integration-with-rulake.md` §2, we ship `mcp-ruqu/` as a
sibling crate of `mcp-server/`. The five tools (`ruqu_simulate`,
`ruqu_verify`, `ruqu_replay`, `ruqu_optimize`, `ruqu_qec_schedule`)
mirror the `#[tool_router]` macro pattern at
`mcp-server/src/server.rs:189`. Three new capabilities (`simulate`,
`hardware`, `verify`) gate them.

The audit pipeline is unified across both servers via a shared
`mcp_server::audit::AuditEntry` (with `mcp-server` exposed under a
new `audit-only` feature so siblings can dep without pulling the
full ruLake-tool surface). Audit codes use disjoint prefixes
(`RUQU_*` vs `RULAKE_*`).

### Decision 5: `runtime_class` field on the bundle for hardware attribution

Hardware-backend bundles carry
`runtime_class = "hardware:<provider>:<device>:<calibration-snapshot>"`
in the `RuntimeContext` packed into the bundle's `Generation::Opaque`.
Simulator-backend bundles carry `runtime_class = "simulated"` (the
backend_id distinguishes the four simulator backends; the
runtime_class distinguishes simulator-vs-hardware).

This is the load-bearing discipline for the cost-attribution and
compliance use cases enumerated in v2-spec.md §l. A cached
hardware-result is unambiguously identified through the bundle, the
audit row, and the Console UI; an operator who mistakes a cached
result for a fresh dispatch has a process problem, not a tooling
problem.

### Decision 6: `--no-cache` flag on `ruqu_simulate` for benchmarking and supremacy claims

Per v2-spec.md §m, benchmarks and quantum-supremacy claims demand
fresh dispatches. The `no_cache: bool` field on `SimulateRequest`
(default `false`):

- When `true`, skips the cache pre-pass entirely and dispatches to
  the backend unconditionally.
- The result IS still written to cache (so the next non-bench
  caller benefits), but the result returned to *this* caller comes
  from the fresh execution.
- The audit row carries `code: "RUQU_NOCACHE_BENCH"` so audit-
  pipeline filters can exclude bench dispatches from cost rollups
  and supremacy-claim rollups.

This is the policy-shaped lever that prevents v2 from accidentally
laundering cached results as fresh experiments. The v2 corpus is
explicit that the cache mechanism gives operators the *option* to
re-dispatch on hardware; operators who confuse cache hits with fresh
dispatches have a process problem, not a tooling problem; v2's
job is to make the distinction unambiguous, and the
`runtime_class` + `no_cache` + `RUQU_REPLAY_HIT` / `RUQU_NOCACHE_BENCH`
codes do that.

---

## Consequences

### Positive

1. **Cross-process replay is free.** Two processes on the same
   machine sharing the same lake see the second-call cost drop to
   ~30 µs (the SHAKE compute + HashMap lookup) regardless of how
   expensive the first call was. For 25-qubit StateVector this is
   8 s → 30 µs (5 orders of magnitude). For hardware dispatches
   this is 30+ s of queue + execution → 30 µs. This is the marquee
   benefit and the gate G2 in §Verification.

2. **Federation across simulation engines becomes a one-line API
   call.** The federation primitive `lake.search_federated`
   (`src/lake.rs:521`) is unchanged. Calling it with the five ruQu
   simulation engines as backends and circuit-hashes as collection
   ids gives "show me all prior runs of this circuit across all
   five engines" for free. Operators get cross-backend visibility
   without writing a federation library.

3. **The Console verifies any quantum-run witness with the same
   wasm code today's bundles use.** The `verifyBundleJson` wasm path
   in `node-wasm/` is unchanged. The Console's Quantum route reuses
   the Bundle viewer for the verify step. ~480 LOC of new TSX in
   v0.1; zero new wasm.

4. **The hardware-attribution story is unambiguous.** `runtime_class`
   in the witness, in the bundle, in the audit row, and in the
   Console UI. Cached hardware results carry a banner; bench-run
   dispatches carry a `RUQU_NOCACHE_BENCH` code. Operators
   debugging cost or compliance issues see the truth at every layer.

5. **The five intent verbs map cleanly onto agentic workflows.**
   Agents that run VQE iterate on `ruqu_simulate` in a loop;
   agents that need replay-once semantics call `ruqu_replay`;
   agents doing reproducibility checks call `ruqu_verify`. The
   surface area is small enough that an agent's prompt can
   enumerate it; capability-gated so a misbehaving agent can't
   bypass the policy layer.

6. **ruQu (the engine) gains audit + cache + federation without
   changing.** All five ruqu-core simulation backends, the
   planner, the QEC control plane, the SIMD layer, the QASM
   exporter — none of these change. v2 wraps them; the wrapping
   is in `ruqu-backend/` and `mcp-ruqu/`, both sibling crates,
   neither in the ruqu-core source tree. If ruqu-core evolves,
   v2 inherits via `Cargo.toml` version bumps.

7. **One audit pipeline for two tool families.** `mcp-server` and
   `mcp-ruqu` share `AuditEntry`; the operator's existing audit
   ingestion pipeline (file watch on `audit.jsonl`, ELK / Loki /
   whatever) works unchanged.

### Negative

1. **The SIMD-path-affects-witness call needs careful documentation.**
   Two workstations with different SIMD support (one AVX2, one
   without) cannot share cache entries for the same circuit, even
   though physically they're computing the same thing. This is the
   right call for bit-exact reproducibility but is *unintuitive*
   to first-time integrators. Mitigation: the v2-spec.md §i
   discussion is explicit; the `RuntimeContext.simd_path` field
   surfaces in every bundle JSON; operators can pin
   `simd_path = "scalar"` for cross-hardware sharing at a speed
   cost. Documentation burden: meaningful but bounded.

2. **The hardware-backend cache policy is non-trivial.** Calibration
   drifts; long-running circuits can span calibration windows; the
   `runtime_class` records the *initial* snapshot. v2-spec.md §p
   open question 1 enumerates the trade-offs. v0.1's choice
   (record drift in `lineage_id`, witness uses the initial
   snapshot) is the simplest and we accept it; v0.2 may revisit.

3. **The migration from v1 produces witnesses that don't match
   fresh v2 runs by default.** v1 `WitnessLog` records lack
   `decoder_id`, `mitigation_id`, `precision_mode`, `simd_path`,
   `runtime_class`. The migrate command fills these with
   `"none"` / `"unknown"` / `"f64"` / `"unknown"` /
   `"simulated"`, which is a different witness from a fresh v2
   run with concrete values. Operators who want their v1 archives
   to be cache-shareable with v2 fresh runs need to manually
   annotate via the migrate command's `--annotate <toml>` flag.
   v2-spec.md §o discusses; documentation burden and operator-
   training cost: real but bounded.

4. **`ruqu-backend` is a non-trivial dep on ruqu-core, which is a
   submodule under `vendor/ruvector/`.** This couples ruLake's
   release cadence to the ruvector submodule's. If ruvector
   evolves a `BackendType` or alters the `Simulator::run` API,
   `ruqu-backend` needs to follow. Mitigation: the submodule is
   pinned per `git submodule status`; a deliberate update is the
   only way the surface changes. The cost is "remember to bump
   the submodule when adopting upstream ruqu-core changes."

5. **The sidebar grows from 6 to 7 routes.** ADR-006's discipline
   is "small, surveyable UI." 7 routes is still small (Stats,
   Playground, Backends, Bundle, Quantum, Audit, Connect — fits in
   one column without scroll on a 1080p display) but is the
   second route addition since ADR-006 landed. v2-spec.md §k
   defends Quantum as a peer; v0.2 may revisit if the route
   count grows further (e.g. if rvDNA's "Genomic" route lands per
   ADR-007 at the same time, the Console hits 8 routes — still
   surveyable, but past a casual glance).

### Neutral

1. **ruQu stays a separate crate family.** ruqu-core, ruqu-
   algorithms, ruqu-exotic, ruqu-wasm continue to live under
   `vendor/ruvector/crates/`. They are not pulled into the ruLake
   workspace (there isn't one per ADR-001); they are dependencies
   of `ruqu-backend/`. This preserves the upstream's release
   independence and the submodule discipline.

2. **The `WitnessLog` v1 hash-chain semantic is preserved through
   a different mechanism.** v1: in-memory `Vec<WitnessEntry>` with
   per-entry `prev_hash`. v2: per-bundle `prev_witness` field in
   a companion `chain.rulake.json` sidecar. The tamper-evidence
   property is preserved (modify any bundle in the chain and the
   chain verification fails), only the storage layer changes.

3. **OpenQASM 3.0 export is unchanged.** v1's
   `vendor/ruvector/crates/ruqu-core/src/qasm.rs` exporter remains
   the authoritative interchange format. v2 does *not* invent a
   "ruQu QASM" dialect; it cache-keys circuits and re-emits QASM
   from cache as needed. Interoperability with the broader quantum
   ecosystem is preserved.

4. **The five v1 simulation backends are unmodified.** Their qubit
   limits, their cost models, their SIMD optimisations, their
   noise models, their precision options — all preserved. v2 is
   strictly *additive* at the simulation layer; the addition is
   the witness anchor and the cache hook.

---

## Verification

Five measurable acceptance gates. ADR-008 is *accepted* when all
five pass in CI. Each maps onto a `tests/*.rs` integration test in
either `ruqu-backend/` or `mcp-ruqu/` per
`integration-with-rulake.md` §1.1 and §2.1.

### Gate G1: Witness equivalence proves out

**Test location:** `ruqu-backend/tests/witness_equivalence.rs`

**Setup:** Generate 1000 random circuits at qubit counts {5, 10, 15,
20} with mixed gate sets (Bell, GHZ, VQE-like ansatzes, surface-code
distance-3, fully random). For each circuit, encode it (compute the
v2 bundle) and compare against the v1 `ExecutionRecord` hash.

**Assertions:**
- Two encodes of the same `(circuit, RuntimeContext)` pair produce
  identical v2 witnesses (1000 trials, 100% pass).
- A targeted mutation in any witness-affecting field (per v2-spec.md
  §i) produces a *different* v2 witness (1000 trials × 6 fields,
  100% pass).
- A targeted mutation in `software_version` or `timestamp_utc`
  produces the *same* v2 witness (per Decision 1, these are not
  witness inputs).
- Zero collisions over 1000 distinct (circuit, ctx) pairs.

**Pass criterion:** all four assertions pass on every trial.

### Gate G2: Cross-process replay sub-1 ms

**Test location:** `ruqu-backend/tests/round_trip_state_vector.rs`

**Setup:** Two test harness threads (representing P1 and P2)
sharing one `Lake` instance. P1 runs `ruqu_simulate` for a 10-qubit
StateVector circuit. P2 then calls `ruqu_replay` with the witness
P1 returned.

**Assertions:**
- P2's replay returns a `SimulationResult` byte-identical to P1's
  result (`amplitudes` agree to bitwise equality).
- P2's replay latency is <1 ms p50 and <5 ms p99 over 100 trials.
- P2's witness matches P1's witness exactly.

**Pass criterion:** all three assertions pass.

### Gate G3: Hardware-cache attribution

**Test location:** `ruqu-backend/tests/hardware_cache_attribution.rs`

**Setup:** A `MockHardwareBackend` (deferred to v0.2 per
`integration-with-rulake.md` §1.4 — G3 ships in v0.2). The mock
returns deterministic histograms with the calibration snapshot
configurable via test setup.

**Assertions:**
- Bundle's `runtime_class` matches
  `"hardware:mock:test-device:cal-<configured-iso8601>"` exactly.
- Audit row carries `code: "RUQU_HARDWARE_DISPATCH"` with the
  witness in `witness_out`.
- Subsequent replay carries `code: "RUQU_REPLAY_HIT"` and
  `original_dispatch_audit_id` in `decision`.
- Calibration-snapshot bump produces a different witness; replay
  against the *old* witness still works (cache entry preserved).

**Pass criterion:** all four assertions pass for 10 trials.

### Gate G4: Clifford concordance — stabilizer-sim ↔ mock-hardware

**Test location:** `ruqu-backend/tests/clifford_concordance.rs`
(deferred to v0.2 since it depends on G3's mock hardware backend).

**Setup:** A 4-qubit GHZ-state preparation circuit dispatched two
ways: (1) StabilizerBackend (exact, all-Clifford), (2)
MockHardwareBackend reporting an IBM-Heron-Q3-like device with
negligible noise.

**Assertions:**
- Two histograms agree to within statistical fluctuation (chi-squared
  test, p > 0.01) for 10 trials.
- Two witnesses are *different* (different backend_id, different
  runtime_class) — concordance is on results, not witnesses.

**Pass criterion:** chi-squared passes for ≥9/10 trials; witness
inequality holds 10/10.

### Gate G5: Audit log round-trip parity with ruLake's

**Test location:** `mcp-ruqu/tests/audit_round_trip.rs`

**Setup:** Test harness runs ten `ruqu_*` calls against `mcp-ruqu`
in parallel with ten `rulake_*` calls against `mcp-server`. Both
servers' audit sinks point at separate JSONL files in a shared
test directory. A test-side audit reader merges both files
chronologically.

**Assertions:**
- Both audit files use the exact same `AuditEntry` JSON schema
  (verified by deserialising every row through `mcp_server::audit::AuditEntry`).
- All `code` fields use disjoint prefixes — every `mcp-ruqu` row
  has `code` starting with `"RUQU_"` (or null); every `mcp-server`
  row has `code` starting with `"RULAKE_"` (or null).
- The merged stream is parseable as a single chronological sequence
  by a downstream observer (mock implementation: a chronologically-
  sorted JSON array).

**Pass criterion:** all three assertions pass for the 20-row
mixed sequence.

### v0.1 acceptance vs full acceptance

ADR-008 is **partially accepted** when G1, G2, and G5 pass —
this corresponds to the v0.1 PR sequence (PR #1-#5 in
`integration-with-rulake.md` §4). G3 and G4 require the
MockHardwareBackend which ships in v0.2; ADR-008 is **fully
accepted** when all five gates pass.

The partial-acceptance state is documented as such; ADR-008's
*Status* moves from `Proposed` → `Partially Accepted` after v0.1
ships and from `Partially Accepted` → `Accepted` after v0.2's
G3 + G4 ship. This is the same ratchet pattern ADR-005 uses for
the v0.1 (bundle-only) → v0.2 (vector-bytes-on-IPFS, deferred)
distinction.

---

## Alternatives Considered

### Alternative A: Embed ruqu-core inside the rulake crate directly

**Shape:** Move `ruqu-core` from `vendor/ruvector/crates/` into
`crates/` of this repo and make it a direct dep of the `rulake`
crate. The five backends register at lake construction time.

**Why rejected:**

- Ownership: ruqu-core is upstream ruvector's; the submodule
  discipline of ADR-001 says "vendored, not owned." Moving it
  into the ruLake repo breaks that discipline and creates a
  divergence-risk for upstream changes.
- Coupling: every ruLake user (including those who don't run
  quantum simulations) would pull in ruqu-core's 26K LOC. That's
  the opposite of the modular sibling-crate pattern this repo
  has used since `gcs-backend/`.
- Releaseing: ruLake releases would block on ruqu-core release
  changes, and vice versa. The sibling-crate pattern decouples
  release cadences.

### Alternative B: Make the witness optional — don't fold ruQu's witness into the bundle

**Shape:** ruQu v2 keeps its own `WitnessLog` and emits it as a
separate file. The bundle is a thin pointer (`data_ref =
"ruqu://...?witness=...")` that doesn't carry the witness inputs.

**Why rejected:**

- The bundle's witness is the cache key. If the witness lives
  outside the bundle, ruLake can't dedup cross-process or cross-
  deployment for the same circuit on the same backend with the
  same settings. The marquee benefit (G2) goes away.
- Two layers of witness (bundle + WitnessLog) means two layers
  of correctness invariants and two layers of audit. The v2-spec.md
  §c bijection argues we can have one layer with no semantic loss;
  the extra layer is pure complexity.
- Operators get a worse experience: the Console's Bundle viewer
  shows the bundle but not the witness inputs; they'd have to
  cross-reference the WitnessLog file separately.

### Alternative C: One BackendAdapter per (engine, circuit-class) pair

**Shape:** Instead of one adapter per simulation engine, ship one
adapter per "kind of circuit": `StateVectorVqeBackend`,
`StateVectorRandomBackend`, `StabilizerCliffordBackend`, etc.

**Why rejected:**

- Cardinality explosion. Five engines × N circuit classes is the
  wrong multiplier. The engine is the natural axis (cost models,
  generation tick rules, SIMD path differ per engine, not per
  circuit class).
- The cost-model planner already routes by circuit class; making
  the adapters circuit-class-aware would duplicate the routing
  logic.
- Operator confusion: "which backend should I register?" becomes
  ambiguous. With five engines, the answer is "all five you have
  the resources for"; with circuit-class adapters, the answer is
  "depends on what your users will simulate," which is impossible
  to know in advance.

### Alternative D: Wait for v3 and bundle this with the rvDNA work

**Shape:** Defer ruqu-backend until rvDNA-backend ships, then bundle
both into a "third-party data formats / engines" v3 release.

**Why rejected:**

- The two are independently valuable; tying them together adds
  scheduling risk for both. The rvDNA agent is on a parallel
  track; ADR-007 + ADR-008 are sister documents but not blocking
  each other.
- ruQu's witness already exists — the integration is uniquely
  cheap because we're not inventing the cryptographic foundation,
  just relocating it. Delaying loses the cheap-integration
  property if ruqu-core revs in the meantime.
- The Console route discussion in v2-spec.md §k benefits from
  shipping early — operator feedback on a 7th route shapes
  whether the 8th (rvDNA's "Genomic") follows the same pattern
  or diverges.

---

## Open follow-ups (post-acceptance)

These are *not* blockers for ADR-008 acceptance; they are what the
acceptance hands off to the v0.2+ work.

1. **Calibration drift mid-run** (v2-spec.md §p Q1) — pick option
   1, 2, or 3. Operator feedback from v0.1 hardware deployments
   informs the choice.
2. **Hybrid algorithm witness** (v2-spec.md §p Q2) — likely punted
   to ruqu-algorithms; a separate ADR may be needed for VQE-style
   trajectory witnesses.
3. **Tier-2 amplitude-bytes federation** (v2-spec.md §p Q4) — only
   becomes urgent if amplitude-bytes-on-IPFS becomes a use case;
   v2-spec.md §o argues bundles-only is sufficient for v0.1.
4. **ruQu (capital Q) crate** at `vendor/ruvector/crates/ruQu/` —
   distinct from the ruqu-* family per v2-spec.md §p Q6. Possibly
   worth its own future ADR; not in scope here.
5. **Algorithm-level wrappers in ruqu-algorithms** (v2-spec.md §p
   Q7) — `Grover::run_with_lake`, `VQE::run_with_lake`, etc. v0.2
   ruqu-backend release may add these as convenience wrappers
   that compose ruqu-algorithms calls with the v2 witness machinery.

---

## References

- `docs/research/ruqu/README.md` — corpus index, persona reading
  guides, what's NOT in the corpus.
- `docs/research/ruqu/v2-spec.md` — canonical ruQu v2 spec, 16
  sections (a-p) including the witness equivalence theorem and the
  five-stage acceptance test.
- `docs/research/ruqu/integration-with-rulake.md` — code-level
  integration: ruqu-backend, mcp-ruqu, Console hooks; the 5-PR ship
  checklist that follows ADR-008 acceptance.
- The parallel `docs/research/rvdna/` corpus and its
  `ADR-007-rvdna-as-rulake-substrate.md` (sibling decision record,
  authored in parallel by the rvDNA agent).
- ADR-001 (`docs/adrs/ADR-001-standalone-repo-strategy.md`) —
  sibling crate layout discipline.
- ADR-005 (`docs/adrs/sdk/ADR-005-ipfs-backend-and-deploy.md`) —
  bundle-only IPFS backend pattern reused for cross-site sharing.
- ADR-006 (`docs/adrs/ADR-006-rulake-console-vite-github-pages.md`) —
  Console architecture; Quantum route extension fits the
  small-surveyable-UI budget.
- ADR-155 (`docs/adrs/ADR-155-rulake-datalake-layer.md`) —
  cache-first, witness-as-anchor framing; M2-M5 backend roadmap
  anticipates content-addressed backends like ruqu-backend.
- ADR-156 (`docs/adrs/ADR-156-rulake-as-memory-substrate.md`) —
  memory-class tagging; v2 emits `memory_class:
  Some("quantum-simulation")` per ADR-156's opaque-string contract.
- The v1 ruqu-core source tree at
  `vendor/ruvector/crates/ruqu-core/src/`, especially:
  - `replay.rs::ExecutionRecord` (line 57) and
    `circuit_canonical_bytes` (line 212) — the canonical encoding
    of a circuit's identity.
  - `witness.rs::WitnessLog::append` (line 107) and
    `verify_chain` (line 141) — the v1 hash-chain semantics that
    v2 preserves through `chain.rulake.json` sidecars.
  - `planner.rs::plan_execution` (line 213) — the v1 cost-model
    planner that v2 wraps with `CacheAwarePlanner`.
  - `qec_scheduler.rs::generate_surface_code_schedule` (line 114)
    and `optimize_feed_forward` (line 376) — the QEC scheduling
    that `ruqu_qec_schedule` exposes.
  - `simd.rs::apply_single_qubit_gate_scalar` (line 37) — the
    scalar gate kernel referenced in v2-spec.md §i for the
    SIMD-path-affects-witness gotcha.
- ruLake source tree:
  - `src/backend.rs::BackendAdapter` (line 110) — the trait the
    five ruQu backends implement.
  - `src/bundle.rs::RuLakeBundle::new` (line 166) and
    `compute_witness` (line 362) — the witness anchor v2 inherits.
  - `src/lake.rs::cache_stats_by_backend` (line 127) — the cache
    pre-pass `CacheAwarePlanner` queries.
  - `src/lake.rs::search_federated` (line 521) — the federation
    primitive ruQu v2 reuses with circuit-witness collection ids.
  - `mcp-server/src/server.rs:189` — the `#[tool_router]` /
    `#[tool]` macro pattern `mcp-ruqu` mirrors.
  - `mcp-server/src/audit.rs::AuditEntry` — the audit schema
    `mcp-ruqu` shares with disjoint code prefixes.
