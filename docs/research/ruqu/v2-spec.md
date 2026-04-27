# ruQu v2 — Witness-Anchored Quantum Execution Intelligence

## a. Status and supersession

**Proposed (2026-04-27)** — supersedes the v1 `WitnessLog` design at
`vendor/ruvector/crates/ruqu-core/src/witness.rs:88`. v2 lifts the
witness *out* of ruQu's own format (a hash-chained `Vec<WitnessEntry>`
serialised by hand-rolled JSON at
`vendor/ruvector/crates/ruqu-core/src/witness.rs:203`-area) and *into*
a ruLake bundle (`crates/core/src/bundle.rs::RuLakeBundle`, format_version 2). The
v1 `WitnessLog` does not disappear; it becomes a v2-internal append
mode (a chain of bundle witnesses linked by `prev_witness` in a
companion sidecar), preserved for the operators who run continuous-
audit pipelines today.

Sister to:

- ADR-155 (`docs/adrs/ADR-155-rulake-datalake-layer.md`) — the
  cache-first, witness-as-anchor framing this spec consumes.
- ADR-005 (`docs/adrs/sdk/ADR-005-ipfs-backend-and-deploy.md`) —
  the bundle-only IPFS backend pattern v2 reuses for cross-site
  circuit-cache sharing.
- ADR-006 (`docs/adrs/ADR-006-rulake-console-vite-github-pages.md`) —
  the Console architecture v2's "Quantum" route plugs into.
- The parallel rvDNA v2 corpus at `docs/research/rvdna/` and its
  ADR-007 (sibling decision record being authored at the same time;
  both ADR-007 and ADR-008 use the same "X as ruLake substrate"
  shape).

### Promises kept verbatim from v1

The full v1 README at `vendor/ruvector/crates/ruqu-core/README.md`
states ten capabilities. v2 keeps every one of them unchanged.
Quoting (italics ours):

1. *5 Simulation Backends — StateVector (exact, up to 32 qubits),
   Stabilizer (millions of qubits), Clifford+T (moderate T-count),
   TensorNetwork (MPS-based), Hardware (device profiles).* v2 keeps
   these five and registers each one as a `BackendAdapter` (§d).
   No backend is removed; no backend's qubit limit changes; no new
   simulation engine is added in v0.1.
2. *Cost-Model Planner — Automatically routes circuits to the optimal
   backend based on qubit count, gate mix, and T-count.* v2 keeps
   the planner at
   `vendor/ruvector/crates/ruqu-core/src/planner.rs:213`
   `plan_execution`. v2 layers a *cache-aware lookup* on top of it
   (§e): the planner's first action is "is this circuit already in
   the lake?", not "which backend is cheapest?". Cheap-backend
   selection is the second action, fired only on cache miss.
3. *Universal Gate Set — H, X, Y, Z, CNOT, CZ, Toffoli, Rx, Ry, Rz,
   Phase, SWAP, and custom unitaries.* The `Gate` enum at
   `vendor/ruvector/crates/ruqu-core/src/gate.rs` is unchanged.
   Custom unitaries (`Gate::Unitary1Q`) are folded into the witness
   exactly as `circuit_canonical_bytes` does today
   (`vendor/ruvector/crates/ruqu-core/src/replay.rs:212`).
4. *QEC Control Plane — Union-find decoder with O(n·α(n)) amortized
   time, sub-polynomial decoders, QEC scheduling, control theory
   integration.* All four files stay
   (`vendor/ruvector/crates/ruqu-core/src/{decoder,subpoly_decoder,
   qec_scheduler,control_theory}.rs`). v2 changes one thing: the
   active decoder's identifier and parameters become inputs to the
   witness derivation (§i). Two runs that differ only in decoder
   produce different witnesses. This is correct: different decoders
   correct different errors and produce different output
   distributions.
5. *OpenQASM 3.0 — Full circuit export to standard quantum assembly
   format.* `vendor/ruvector/crates/ruqu-core/src/qasm.rs` is
   unchanged. v2 adds: any cached circuit can be re-emitted as QASM
   without re-running the simulation (§g `replay`).
6. *Noise & Mitigation — Depolarizing, amplitude/phase damping,
   custom Kraus operators, zero-noise extrapolation, probabilistic
   error cancellation.* `vendor/ruvector/crates/ruqu-core/src/noise.rs`
   and `mitigation.rs` are unchanged. v2 adds: noise model id and
   mitigation strategy id flow into the witness (§i).
7. *SIMD Acceleration — AVX2/NEON vectorized gate application for
   2-4x speedup.* `vendor/ruvector/crates/ruqu-core/src/simd.rs` is
   unchanged. v2 adds: the *active SIMD path* (`scalar` /
   `avx2` / `neon` / `parallel-rayon`) flows into the witness (§i).
   This is the load-bearing honesty in this spec — see the SIMD
   gotcha discussion in §i.
8. *Multi-Threading — Rayon-based parallelism for large qubit
   counts.* Unchanged. The rayon path is one of the four SIMD-path
   discriminants in §i.
9. *Cryptographic Witnesses — Tamper-evident execution logs for
   reproducibility and verification.* This is the seam v2 widens.
   The v1 `WitnessLog` chain becomes a `prev_witness` link inside a
   ruLake bundle's optional metadata; the *current* witness becomes
   `RuLakeBundle::rvf_witness` (SHAKE-256(32) instead of
   SipHash×4). See §c for the equivalence proof.
10. *Transpiler — Gate decomposition, routing, and hardware-aware
    optimization.* `vendor/ruvector/crates/ruqu-core/src/transpiler.rs`
    is unchanged. v2 adds: the transpiler output is itself a cached
    artefact (§g `optimize`), keyed by the input circuit's hash and
    the transpilation parameters.
11. *Mixed Precision — Configurable f32/f64 simulation for speed vs
    accuracy tradeoff.* `vendor/ruvector/crates/ruqu-core/src/mixed_precision.rs`
    is unchanged. v2 adds: the active precision mode flows into the
    witness (§i).

### What v2 reshapes (and why)

- **The `WitnessLog` evolves from "internal log we serialise
  ourselves" to "stream of `RuLakeBundle`s linked by `prev_witness`".**
  The `to_json` method at
  `vendor/ruvector/crates/ruqu-core/src/witness.rs:203` is replaced
  by `RuLakeBundle::to_json` (`crates/core/src/bundle.rs:203`-area). The
  hash-chain semantic is preserved; only the digest function (SHAKE
  vs SipHash) and the serialisation library (serde_json vs
  hand-rolled) change.
- **The cost-model planner becomes cache-aware** (§e). The v1
  `plan_execution` returns an `ExecutionPlan` with a chosen backend;
  v2 wraps that with `lake.cache_stats_by_backend` so the actual
  call site sees "cache hit" or "miss → backend X" rather than
  "backend X" unconditionally.
- **Hardware backend submissions get a `runtime_class` field** (§l)
  so a cached IBM-Heron-Q3 result is distinguishable from a cached
  StateVector simulation result. v1's `Hardware` backend at
  `vendor/ruvector/crates/ruqu-core/src/hardware.rs` returns
  `AuthenticationFailed` for remote providers in the open-source
  build; v2 keeps that posture but reserves the schema.
- **The browser story unifies on the Console.** v1 ships
  `ruqu-wasm` with a standalone JS API
  (`vendor/ruvector/crates/ruqu-wasm/README.md` Quick Start). v2
  keeps that API for direct integrators, *and* surfaces a
  Console route ("Quantum") that runs the same wasm in-tab and
  composes with `rulake-wasm` for witness verification (§k).

### What v2 explicitly drops or defers

- **The hand-rolled JSON serialiser** at
  `vendor/ruvector/crates/ruqu-core/src/witness.rs:203`-area. v2
  uses `RuLakeBundle::to_json` which goes through `serde_json` and
  benefits from the same length-cap hardening at `crates/core/src/bundle.rs:218`.
  v1's serialiser is preserved as `WitnessLog::to_json_v1_compat()`
  for tools that already parse the v1 shape; emitting v1-shaped
  JSON from v2 is supported through v0.4 and removed in v0.5.
- **The `software_version` field's role as part of the witness.**
  v1 includes the crate version in `ExecutionRecord::software_version`
  and folds it into the hash. v2 demotes this to bundle metadata (a
  `provenance` field outside the witness inputs), because two runs
  on different ruqu-core point releases that produce *bit-identical*
  amplitude vectors should share a cache entry. Operators who need
  per-version pinning add a witness-affecting setting via the
  `runtime_class` field (§l). This is a deliberate weakening of
  v1's "two records with same hash → identical software" property
  and we acknowledge it openly.

## b. Goals and non-goals

### Five goals

1. **Witness ↔ ruLake bundle parity.** A ruQu v2 execution emits a
   bundle that is byte-identical to one emitted by any other ruQu
   v2 instance for the same circuit, on the same backend, with the
   same QEC and noise/mitigation settings, on the same SIMD path.
   "Byte-identical" means `RuLakeBundle::rvf_witness` matches and
   `RuLakeBundle::verify_witness()` (`crates/core/src/bundle.rs:191`) returns
   `true`. This is the "two processes, second one is free"
   property.

2. **Cross-process result reuse via content-addressing.** Once
   process A has run a circuit and cached the result, process B on
   the same machine — or on a different machine sharing the same
   IPFS-backend bundle (ADR-005) — gets the answer in <1 ms via
   cache lookup keyed by `rvf_witness`. No re-simulation. This is
   the validation gate G2 in ADR-008.

3. **Federated cost-model planning.** The v1 planner picks the
   cheapest backend; v2 picks the *highest-cache-hit-rate* backend
   first via `lake.cache_stats_by_backend` (`crates/core/src/lake.rs:127`),
   falling back to the v1 cheapness criterion only on miss. The
   federation layer (`lake.search_federated`, `crates/core/src/lake.rs:521`)
   becomes "ask all five backends, in parallel, for prior runs of
   this circuit" — a one-line API call against the existing
   federation primitive.

4. **Browser-side circuit execution + verification.** A Console
   user opens the "Quantum" route, drops a Bell-state circuit into
   the editor, runs it with `ruqu-wasm`
   (`vendor/ruvector/crates/ruqu-wasm/`), gets the amplitudes plus
   a bundle JSON. The Console then verifies that bundle against a
   remote `mcp-ruqu` server's witness for the same circuit, using
   the existing `rulake-wasm` `verifyBundleJson` path. Both wasm
   modules already exist; v2 composes them.

5. **Zero new code in user space when ruQu users adopt ruLake.**
   The existing v1 entry points
   (`vendor/ruvector/crates/ruqu-core/src/simulator.rs::Simulator::run`,
   `Simulator::run_with_config`, `Simulator::run_shots`) keep their
   signatures. v2 introduces a *parallel* entry point
   `Simulator::run_with_lake(circuit, config, lake)` that returns
   the same `SimulationResult` plus a `RuLakeBundle`; users who
   want the cache-share property opt in by changing the call site,
   and users who don't aren't disrupted.

### Three non-goals

1. **v2 does not replace OpenQASM.** The v1 exporter at
   `vendor/ruvector/crates/ruqu-core/src/qasm.rs` is the
   authoritative interchange format with the rest of the quantum
   ecosystem. v2 cache-keys *circuits* (which can be re-emitted as
   QASM losslessly) but does not invent a "ruQu QASM" dialect.

2. **v2 does not fork the simulation backends.** All five backends
   (StateVector, Stabilizer, Clifford+T, TensorNetwork, Hardware)
   stay in `ruqu-core`. v2 wraps them as `BackendAdapter`
   implementations in a sibling crate `crates/ruqu-backend/` (analogous to
   `crates/gcs-backend/` and `crates/ipfs-backend/` in this repo). If a backend
   needs to evolve, the change lands in `ruqu-core`; v2 inherits
   it via the trait impl.

3. **v2 does not ship a ruQu-specific UI.** The Console at `ui/`
   gets a Quantum route; there is no separate quantum-only web app.
   This is the same discipline ADR-006 applies to all ruLake-
   surface UIs: one Console, multiple routes, shared modes
   (Demo / WASM-local / Live).

## c. The witness equivalence theorem (informal)

**Claim.** ruQu's v1 cryptographic witness
(`vendor/ruvector/crates/ruqu-core/src/replay.rs::ExecutionRecord` +
`vendor/ruvector/crates/ruqu-core/src/witness.rs::WitnessEntry`) and
ruLake's v2 bundle witness (`crates/core/src/bundle.rs::RuLakeBundle::rvf_witness`,
computed via `crates/core/src/bundle.rs:362` `compute_witness`) can be unified
without either side losing semantics.

**Strategy.** We exhibit a bijection between the v1 `ExecutionRecord`
field set and the v2 `RuLakeBundle` field set such that:

(a) every v1 field is recoverable from the v2 bundle (no data is
lost on the v1→v2 direction); and

(b) the v2 `rvf_witness` SHAKE-256(32) covers exactly the same
*entropy* the v1 hash chain covered, plus strict additions (the v1
hash function — `DefaultHasher` SipHash — is replaced by SHAKE, a
cryptographic hash, not a non-cryptographic one).

### Field mapping

| v1 `ExecutionRecord` field | v2 `RuLakeBundle` placement | Notes |
|---|---|---|
| `circuit_hash: [u8; 32]` (`vendor/ruvector/crates/ruqu-core/src/replay.rs:60`) | Encoded into `data_ref: String` as `ruqu://<backend>/<hex(circuit_hash)>` | The bundle's `data_ref` is the URI of the authoritative byte stream; for ruQu v2 the "byte stream" is the canonical circuit bytes recoverable from the hash. The `data_ref` length-prefixed digest covers the same 32 bytes. |
| `seed: u64` (`replay.rs:62`) | Folded into a packed `Generation::Opaque(serde_json::to_string(&RuntimeContext))` | See `RuntimeContext` definition below. |
| `backend: String` (`replay.rs:64`) | Folded into `RuntimeContext.backend_id` (e.g. `"state_vector"`, `"stabilizer"`, `"clifford_t"`, `"tensor_network"`, `"hardware:ibm:heron-q3"`) | The bundle's `data_ref` includes the backend in its prefix (`ruqu://<backend>/...`), so the witness double-covers the backend id. |
| `noise_config: Option<NoiseConfig>` (`replay.rs:66`) | Folded into `RuntimeContext.noise_id`, a deterministic SHAKE-256(8) digest over `(depolarizing_rate, bit_flip_rate, phase_flip_rate, amplitude_damping, phase_damping, custom_kraus_hash)` | The id, not the parameters, goes into the witness. The full parameters live in the bundle's *non-witness* metadata (the bundle is a struct; only the witness-input fields fold into the SHAKE digest). |
| `shots: u32` (`replay.rs:68`) | Folded into `RuntimeContext.shots` | Different shot counts → different witnesses. Two runs at 1024 shots and 2048 shots are distinct cache entries. |
| `software_version: String` (`replay.rs:70`) | Demoted to `RuLakeBundle::lineage_id` (a free-form provenance field; *not* a witness input). | See §a "What v2 explicitly drops" — this is a deliberate semantic relaxation. |
| `timestamp_utc: u64` (`replay.rs:72`) | Demoted to `RuLakeBundle::lineage_id` (free-form provenance) | Same reason: two re-runs at different wall-clock times should share a cache entry if all witness-affecting inputs are equal. |
| `WitnessEntry::sequence: u64` (`witness.rs:73`) | Folded into the *companion* sidecar `chain.rulake.json` (a list of bundle witnesses); the bundle itself does not carry a sequence | The hash-chain is now a sequence of bundles; sequence number is the array index. |
| `WitnessEntry::prev_hash: [u8; 32]` (`witness.rs:75`) | Folded into the companion sidecar's per-entry `prev_witness` field | Bundle witnesses become the chain links. |
| `WitnessEntry::result_hash: [u8; 32]` (`witness.rs:79`) | Folded into `RuLakeBundle::lineage_id` for the bundle that *captures* the result; it is *not* a witness input | The result hash is a *consequence* of the witness, not an input to it. Same circuit + same witness inputs → same result_hash. The chain proof recomputes it on verification (§i). |

### The `RuntimeContext` packing

```rust
// New in ruqu-backend; mirrors the rotation_seed/rerank_factor pattern
// in ruLake's bundle but for the quantum execution domain.
#[derive(Serialize, Deserialize, Debug, Clone, Hash, PartialEq, Eq)]
struct RuntimeContext {
    backend_id: String,        // "state_vector" | "stabilizer" | ...
    noise_id: String,          // SHAKE-256(8) over the full noise model
    decoder_id: String,        // "union_find" | "subpoly:lut-v3" | "none"
    decoder_params_hash: String,
    mitigation_id: String,     // "none" | "zne_3" | "zne_5" | "full_cdr_n50" | ...
    precision_mode: String,    // "f32" | "f64"
    simd_path: String,         // "scalar" | "avx2" | "neon" | "rayon"
    shots: u32,
    seed: u64,
    runtime_class: String,     // "simulated" | "hardware:ibm:heron-q3:cal-2026-04-22T14:30Z"
}
```

The bundle's `Generation::Opaque(serde_json::to_string(&ctx))` carries
this packing into `compute_witness` (`crates/core/src/bundle.rs:362`). The
`Generation::hash_bytes()` variant tag at `crates/core/src/bundle.rs:82` already
prefixes a `0x01` byte for `Opaque`, ensuring this packing cannot
collide with a numeric `Generation::Num` from a non-ruQu backend.

### Why the bijection holds

(a) **v1→v2 lossless.** Every v1 `ExecutionRecord` field is either in
the `RuntimeContext` (backend, noise [via id+params lookup], shots,
seed) or in the bundle's `lineage_id` (software_version, timestamp).
The `circuit_hash` is encoded into `data_ref`. No v1 field is dropped.

(b) **v2 witness covers the v1 entropy.** SHAKE-256(32) over
`(data_ref, dim, rotation_seed, rerank_factor, generation)` —
where `data_ref` includes the circuit hash, `generation` includes the
full `RuntimeContext` JSON, and `rotation_seed` and `rerank_factor`
are inherited from the lake's cache configuration (constants for a
given lake instance) — covers strictly more entropy than the v1
SipHash chain over `circuit_hash || seed || backend || noise_config
|| shots || software_version || timestamp_utc`. The two extra
fields ruLake contributes (`rotation_seed`, `rerank_factor`) are
constants from ruQu's perspective but are *correctly* part of the
witness because two ruLake instances configured with different
rotation seeds will not share cached *result vectors* even if the
circuit is identical (the result vectors are stored as RaBitQ-
compressed codes, and the codes depend on the rotation seed).

(c) **The hash function strengthens, not weakens.** v1 uses
`DefaultHasher`, which is SipHash-1-3 in current Rust. SipHash is
designed for HashMap collision resistance, not cryptographic
collision resistance. v2 uses SHAKE-256(32), which is a NIST-approved
extendable-output function with cryptographic collision resistance.
The v1 chain's tamper-detection property (every entry's `prev_hash`
links to the predecessor) is preserved by the v2 chain sidecar
(every bundle's `prev_witness` field links to the predecessor's
`rvf_witness`), but the per-link strength goes from non-cryptographic
to cryptographic.

The claim is informal because we have not produced a machine-checked
proof; the field mapping, the entropy-covering argument, and the
hash-strength argument together are the closest engineering approach.
The acceptance gate G1 in ADR-008 demands a property test that
exhibits the bijection over 1000 randomly-generated executions.

## d. 5 backends → 5 BackendAdapter impls

The `BackendAdapter` trait at `crates/core/src/backend.rs:110` is four required
methods plus an optional `current_bundle` override and an optional
`supports_pushdown`. Per the trait:

```rust
pub trait BackendAdapter: Send + Sync {
    fn id(&self) -> &str;
    fn list_collections(&self) -> Result<Vec<CollectionId>>;
    fn pull_vectors(&self, collection: &str) -> Result<PulledBatch>;
    fn generation(&self, collection: &str) -> Result<u64>;
    // optional:
    fn current_bundle(&self, collection: &str, rotation_seed: u64,
                      rerank_factor: usize) -> Result<crate::RuLakeBundle> { ... }
    fn supports_pushdown(&self) -> bool { false }
}
```

Each ruQu backend gets one impl. The "collection" notion is what
varies — it's not "tables of vectors" in a SQL sense; it's "registered
circuits whose results live in this engine's cache".

### d.1 StateVector

```rust
pub struct StateVectorBackend {
    sim: Arc<RwLock<ruqu_core::simulator::Simulator>>,
    cache: Arc<RwLock<HashMap<String /* circuit_hash hex */, CachedRun>>>,
    generation: Arc<AtomicU64>,
}

impl BackendAdapter for StateVectorBackend {
    fn id(&self) -> &str { "ruqu-state-vector" }

    fn list_collections(&self) -> Result<Vec<CollectionId>> {
        // A "collection" is a registered circuit_hash. The set is
        // every circuit whose final amplitudes live in this backend's
        // result cache. Empty after process restart unless the
        // operator pre-loaded from a snapshot.
        Ok(self.cache.read().unwrap().keys().cloned().collect())
    }

    fn pull_vectors(&self, collection: &str) -> Result<PulledBatch> {
        // The "vectors" are the final amplitudes (re/im interleaved
        // as f32 pairs), or the measurement-distribution histogram
        // re-encoded as a vector. Operator picks at backend
        // construction time; v0.1 default is amplitudes.
        let cache = self.cache.read().unwrap();
        let cached = cache.get(collection)
            .ok_or_else(|| RuLakeError::InvalidParameter(
                format!("ruqu-state-vector: no cached run for {collection}")))?;
        Ok(PulledBatch {
            collection: collection.into(),
            ids: vec![0],  // one row per circuit, the "result" itself
            vectors: vec![cached.amplitudes_as_f32()],
            dim: cached.amplitudes.len() * 2, // re+im interleaved
            generation: self.generation.load(Ordering::Acquire),
        })
    }

    fn generation(&self, _collection: &str) -> Result<u64> {
        Ok(self.generation.load(Ordering::Acquire))
    }
    // generation() ticks when: a noise-model rev lands, a SIMD-path
    // change is detected, or the operator calls bump_generation()
    // explicitly (e.g. after upgrading ruqu-core).

    fn current_bundle(&self, collection: &str, rotation_seed: u64,
                      rerank_factor: usize) -> Result<RuLakeBundle> {
        let cached = self.cache.read().unwrap()
            .get(collection)
            .ok_or_else(...)?
            .clone();
        // Pack the runtime context per §c.
        let ctx = RuntimeContext {
            backend_id: "state_vector".into(),
            noise_id: cached.noise_id.clone(),
            decoder_id: "none".into(),
            decoder_params_hash: "".into(),
            mitigation_id: cached.mitigation_id.clone(),
            precision_mode: cached.precision_mode.clone(),
            simd_path: simd_path_active(),
            shots: cached.shots,
            seed: cached.seed,
            runtime_class: "simulated".into(),
        };
        Ok(RuLakeBundle::new(
            format!("ruqu://state_vector/{collection}"),
            cached.amplitudes.len() * 2,
            rotation_seed,
            rerank_factor,
            Generation::Opaque(serde_json::to_string(&ctx)?),
        ))
    }
}
```

What ticks `generation`: the witness already covers the SIMD path,
the noise id, the decoder id, the precision mode. The bare
`generation()` integer ticks only when the *backend itself* changes
out from under the cache — a `ruqu-core` point release that altered
gate-application semantics, a calibration update, or an explicit
operator-driven invalidation. In normal operation, the lake's
cache-key is dominated by the `Generation::Opaque(ctx_json)` packing,
not the integer `generation()` return value.

### d.2 Stabilizer

```rust
pub struct StabilizerBackend { ... }

impl BackendAdapter for StabilizerBackend {
    fn id(&self) -> &str { "ruqu-stabilizer" }
    fn list_collections(&self) -> Result<Vec<CollectionId>> {
        // For Clifford circuits with no measurement, the "result"
        // is a stabilizer-tableau snapshot, not amplitudes. The
        // collection is the circuit_hash; pull_vectors returns the
        // tableau rows as bit-packed bytes re-encoded as f32s.
        ...
    }
    fn pull_vectors(&self, collection: &str) -> Result<PulledBatch> {
        // Tableau is 2n × n bits; for n=1000 that's 250 KB.
        // We encode as f32 vector of length (2n*n + 31)/32 by packing
        // 32 tableau bits per f32 word. This is awkward but
        // ruLake's API speaks vectors; we lean into it.
        ...
    }
    fn generation(&self, _: &str) -> Result<u64> { ... }
    // current_bundle: backend_id="stabilizer", decoder_id="none"
    // (Clifford is exact; QEC decoder only enters via the Clifford+T
    // hybrid backend or via the QEC scheduler when the circuit is
    // an encoded surface code).
}
```

What ticks `generation`: same as StateVector — a `ruqu-core` Clifford-
backend revision (e.g. a bug fix in
`vendor/ruvector/crates/ruqu-core/src/stabilizer.rs::apply_cnot`)
or an operator-driven bump.

### d.3 Clifford+T

```rust
pub struct CliffordTBackend { ... }

impl BackendAdapter for CliffordTBackend {
    fn id(&self) -> &str { "ruqu-clifford-t" }
    fn list_collections(&self) -> Result<Vec<CollectionId>> { ... }
    fn pull_vectors(&self, collection: &str) -> Result<PulledBatch> {
        // The result is a sum of stabilizer terms (sparse). v0.1
        // returns the dominant-term tableau plus a coefficient
        // vector of length min(t_count, max_terms_returned). The
        // dim is configurable per cache entry; the bundle's `dim`
        // field captures it.
        ...
    }
    fn generation(&self, _: &str) -> Result<u64> { ... }
    // current_bundle: backend_id="clifford_t", decoder_id="none",
    // and the `RuntimeContext` adds a `t_count` field that the
    // packing serialises into the witness.
}
```

What ticks `generation`: a change in the
`vendor/ruvector/crates/ruqu-core/src/clifford_t.rs` term-truncation
heuristic (today the planner caps at `CT_MAX_T_COUNT = 40`,
`vendor/ruvector/crates/ruqu-core/src/planner.rs:184`); a bump in
the maximum-terms-returned configuration; an operator-driven
invalidation.

### d.4 TensorNetwork

```rust
pub struct TensorNetworkBackend { ... }

impl BackendAdapter for TensorNetworkBackend {
    fn id(&self) -> &str { "ruqu-tensor-network" }
    fn list_collections(&self) -> Result<Vec<CollectionId>> { ... }
    fn pull_vectors(&self, collection: &str) -> Result<PulledBatch> {
        // The result is an MPS — a chain of bond-dimensional
        // tensors. We flatten the entire MPS (n × chi^2 × 2 for
        // complex f32) into a single vector. dim varies with chi;
        // the bundle captures it. Pulls are larger than for
        // StateVector — a 64-qubit MPS at chi=128 is ~2 MB.
        ...
    }
    fn generation(&self, _: &str) -> Result<u64> { ... }
    // current_bundle: backend_id="tensor_network", and the
    // `RuntimeContext` adds bond_dim_max and truncation_threshold
    // — both witness-affecting because different chi caps produce
    // different MPS approximations.
}
```

What ticks `generation`: a change in
`vendor/ruvector/crates/ruqu-core/src/tensor_network.rs` contraction
order, an SVD-implementation revision, an operator bump.

### d.5 Hardware

```rust
pub struct HardwareBackend {
    provider: ruqu_core::hardware::ProviderType, // IbmQuantum, IonQ, ...
    device_name: String,
    cal_snapshot: Arc<RwLock<CalibrationSnapshot>>,
    ...
}

impl BackendAdapter for HardwareBackend {
    fn id(&self) -> &str {
        // Composite id encodes provider+device so the lake
        // can attribute cache entries authoritatively.
        // Examples: "ruqu-hardware:ibm:heron-q3"
        //           "ruqu-hardware:ionq:forte-1"
        Box::leak(format!("ruqu-hardware:{}:{}",
            self.provider.short(), self.device_name).into_boxed_str())
    }
    fn list_collections(&self) -> Result<Vec<CollectionId>> {
        // Only completed jobs whose results we hold. Pending jobs
        // are not collections; they are an out-of-band concern.
        ...
    }
    fn pull_vectors(&self, collection: &str) -> Result<PulledBatch> {
        // Hardware results are measurement histograms (shot counts).
        // We encode as a dense vector of length 2^min(n, 20) where
        // each entry is the shot count for that bitstring. Truncation
        // is signalled in the bundle's lineage_id (provenance), not
        // the witness — two truncated runs of the same circuit at
        // the same shot count produce the same witness.
        ...
    }
    fn generation(&self, _: &str) -> Result<u64> {
        // CRITICAL: a calibration snapshot change ticks this.
        // The runtime_class field in the RuntimeContext also
        // captures the snapshot id; the integer generation is
        // belt-and-braces for backends that are out of band.
        self.cal_snapshot.read().unwrap().monotonic_id()
    }
    fn current_bundle(&self, collection: &str, rotation_seed: u64,
                      rerank_factor: usize) -> Result<RuLakeBundle> {
        // runtime_class is the load-bearing field for this backend.
        let cached = ...;
        let cal = self.cal_snapshot.read().unwrap();
        let ctx = RuntimeContext {
            backend_id: "hardware".into(),
            // ... other fields ...
            runtime_class: format!("hardware:{}:{}:cal-{}",
                self.provider.short(), self.device_name, cal.id_iso8601()),
        };
        Ok(RuLakeBundle::new(
            format!("ruqu://hardware/{}/{}", self.id(), collection),
            cached.histogram.len(),
            rotation_seed,
            rerank_factor,
            Generation::Opaque(serde_json::to_string(&ctx)?),
        ))
    }
}
```

What ticks `generation`: every calibration snapshot change. This is
the field that prevents a cached result from a 14:30 calibration
window being served when the device has since drifted; the
`runtime_class` field in the witness ensures the cache *cannot*
serve cross-calibration even by accident, because the witness will
not match.

### A note on the ruqu-backend crate shape

`crates/ruqu-backend/` is a single Cargo crate that exports all five
adapters behind a feature flag set:

```toml
[features]
default = ["state-vector", "stabilizer"]
state-vector = []
stabilizer = []
clifford-t = []
tensor-network = []
hardware = ["dep:reqwest"] # remote providers need a HTTP client
all = ["state-vector", "stabilizer", "clifford-t",
       "tensor-network", "hardware"]
```

Operators with WASM-target deployments (`ruqu-wasm` builds for
`wasm32-unknown-unknown`) get only `state-vector` and `stabilizer`
by default — the other three pull in dependencies that don't
cleanly cross-compile. See `integration-with-rulake.md`
§"What v0.1 ships vs v0.2 defers" for the feature-flag rationale
in detail.

## e. Cost-model planner as backend chooser

The v1 planner at
`vendor/ruvector/crates/ruqu-core/src/planner.rs:213`
`plan_execution` returns an `ExecutionPlan` with:

- `backend: BackendType` — chosen via the priority rules at
  `vendor/ruvector/crates/ruqu-core/src/planner.rs:476`-area
  (`select_optimal_backend`). Pure Clifford → Stabilizer; small
  → StateVector; mostly Clifford with few non-Clifford → Stabilizer
  with approximate decomposition; moderate T-count → CliffordT;
  large nearest-neighbour → TensorNetwork; fall-through → SV.
- `predicted_memory_bytes: u64`, `predicted_runtime_ms: f64` —
  cost estimates from the cost model at
  `vendor/ruvector/crates/ruqu-core/src/planner.rs:160`-area
  (the `SV_NS_PER_GATE` etc constants).
- `verification_policy`, `mitigation_strategy`,
  `entanglement_budget`, `cost_breakdown`.

v2 wraps this with a *cache pre-pass*. The wrapper:

```rust
// New in ruqu-backend; lives in `planner_cache.rs`.
pub struct CacheAwarePlanner {
    inner: ruqu_core::planner::PlannerConfig,
    lake: Arc<RuLake>,
}

#[derive(Debug, Clone)]
pub enum CacheAwarePlan {
    /// Cache hit on backend `b` — `replay_witness` is the
    /// `rvf_witness` of the cached bundle. Caller fetches the
    /// result via `lake.cache_get_by_witness(...)`.
    Hit { backend_id: String, replay_witness: String },
    /// Cache miss — fall back to the v1 planner's chosen backend.
    /// Caller dispatches the simulation, then writes the resulting
    /// bundle back into the lake via `lake.put_circuit_result(...)`.
    Miss {
        plan: ruqu_core::planner::ExecutionPlan,
        anticipated_witness: String, // computed from inputs ahead of time
    },
}

impl CacheAwarePlanner {
    pub fn plan(&self, circuit: &QuantumCircuit, ctx: &RuntimeContext)
        -> Result<CacheAwarePlan>
    {
        // 1. Compute the anticipated witness from inputs.
        let circuit_hash = ReplayEngine::circuit_hash(circuit);
        let bundle = self.synthetic_bundle(circuit_hash, ctx);
        let witness = bundle.rvf_witness.clone();

        // 2. Federation lookup: ask all five backends in parallel.
        //    Reuses lake.search_federated semantically by witness.
        let stats = self.lake.cache_stats_by_backend();
        let backends_with_hits: Vec<_> = stats.iter()
            .filter(|(_, s)| s.witnesses.contains(&witness))
            .map(|(b, _)| b.clone())
            .collect();
        if let Some(b) = backends_with_hits.into_iter().next() {
            return Ok(CacheAwarePlan::Hit {
                backend_id: b,
                replay_witness: witness,
            });
        }

        // 3. Cache miss: defer to the v1 cost-model planner.
        let plan = ruqu_core::planner::plan_execution(circuit, &self.inner);
        Ok(CacheAwarePlan::Miss {
            plan,
            anticipated_witness: witness,
        })
    }
}
```

`stats.witnesses.contains(&witness)` is the new cache-stats
extension v2 needs from ruLake. It can be implemented in v2 of
ruLake as one method on `PerBackendStats` returning a
`HashSet<String>` of witnesses currently held; ADR-008 §Decision 3
specifies this is a non-breaking ruLake addition.

The cache pre-pass adds one `HashMap` lookup (`cache_stats_by_backend`
is `O(num_backends)`, currently 4-8) plus one `compute_witness` call
(microseconds). For circuits that miss, the wrapper adds <50 µs to
the planner's existing ~10 ms typical runtime — negligible. For
circuits that hit, the wrapper saves whatever the v1 simulation
would have cost — typically tens of milliseconds for SV at 20
qubits, seconds for SV at 25, minutes for hardware-backend
submissions. The asymmetry is the win: cheap on miss, transformative
on hit.

## f. Pipeline §1: encode

`ruqu v2 encode <qasm>` produces a witness-anchored circuit bundle
without running the simulation. Use case: pre-compute witnesses for
1000 circuits to populate a cache or to plan a federation distribution
before paying simulation cost.

Sequence:

1. Parse QASM (`vendor/ruvector/crates/ruqu-core/src/qasm.rs` parser
   — note v1 ships an exporter; v2 needs the matching parser, which
   is on the v0.1 critical path).
2. Optimize: `ruqu_core::optimizer::fuse_gates`
   (`vendor/ruvector/crates/ruqu-core/src/optimizer.rs:46`).
3. Decompose: `ruqu_core::decomposition::decompose`
   (called from the v1 pipeline at
   `vendor/ruvector/crates/ruqu-core/src/pipeline.rs:129`).
4. Transpile: `ruqu_core::transpiler` (target backend per the
   planner output).
5. Assign backend: `CacheAwarePlanner::plan` (§e).
6. Encode for cache key: compute the bundle. If the planner returns
   `Hit`, the bundle exists; if `Miss`, the bundle is the
   `anticipated_witness` form, ready to be filled in by `simulate`.

The CLI returns a JSON shape:

```json
{
  "circuit_hash": "a7b2...",
  "bundle": {
    "format_version": 2,
    "data_ref": "ruqu://state_vector/a7b2...",
    "dim": 32,
    "rotation_seed": 1234,
    "rerank_factor": 20,
    "generation": "{\"backend_id\":\"state_vector\",...}",
    "rvf_witness": "8d3c..."
  },
  "cache_status": "miss",
  "estimated_cost_ms": 12.3
}
```

Two encodes of the same QASM file produce the same JSON. This is the
property that makes encode a cheap pre-pass: an operator can encode
1000 circuits in under a second and immediately know the federation
distribution from the `cache_status` field.

## g. Pipeline §2: run — five intent verbs

Mirrors `rulake_query`'s search/verify/explain/refresh shape (see
`crates/mcp-server/src/server.rs:189`-area for the v1 ruLake intent enum).

### `simulate` (run circuit, witness-pinned)

Run the circuit on the planner-chosen backend, write the result into
the lake, return `(SimulationResult, RuLakeBundle)`.

**Input shape:**
```rust
struct SimulateRequest {
    circuit: QuantumCircuit,        // or qasm: String
    runtime: RuntimeContext,        // see §c packing
    no_cache: bool,                 // §m benchmarking discipline
}
```

**Output shape:**
```rust
struct SimulateResponse {
    result: SimulationResult,       // ruqu_core::simulator::SimulationResult
    bundle: RuLakeBundle,
    cache_action: CacheAction,      // Hit | StoredFresh | NoCacheBypass
    elapsed_ms: f64,
    backend_chosen: String,
}
```

**Cache key derivation:**
```
witness = compute_witness(
    data_ref = "ruqu://" + runtime.backend_id + "/" + hex(circuit_hash),
    dim = result_dim_for(backend_id, num_qubits),
    rotation_seed = lake.rotation_seed,
    rerank_factor = lake.rerank_factor,
    generation = Opaque(serde_json(runtime))
)
```

**Expected p50/p99 (cite v1 benches at
`vendor/ruvector/crates/ruqu-core/benches/quantum_sim.rs`):**

| Backend | Qubits | Cache hit p50 | Cache miss p50 | Cache miss p99 |
|---|---|---|---|---|
| StateVector | 10 | ~30 µs | ~0.3 ms | ~0.5 ms |
| StateVector | 20 | ~30 µs | 250 ms | 400 ms |
| Stabilizer | 50 | ~30 µs | <1 ms | ~2 ms |
| TensorNetwork (chi=64) | 64 | ~30 µs | 50-200 ms | 1 s |
| Hardware (Heron-Q3, 20 qubits, 1024 shots) | 20 | ~30 µs | 30 s + queue | 5 min + queue |

The cache-hit path is dominated by the SHAKE-256 witness compute
(microseconds) plus the `cache_get_by_witness` HashMap lookup
(microseconds). The 30 µs figure is conservative; actual hit-path
in ruLake's RaBitQ-cached search is benchmarked at <50 µs in
`benchmarks/` for k=10 retrieval.

### `verify` (re-run + verify a stored witness)

Recompute the witness from inputs, fetch the stored bundle, recompute
the result-hash on a fresh run, compare.

**Input shape:**
```rust
struct VerifyRequest {
    circuit: QuantumCircuit,
    runtime: RuntimeContext,
    expected_witness: String,
}
```

**Output shape:**
```rust
struct VerifyResponse {
    matches: bool,
    expected_witness: String,
    computed_witness: String,
    result_hash_match: bool,        // true iff bit-exact result reproduction
    elapsed_ms: f64,
}
```

This is the *clinical-grade reproducibility* path (mirrors rvDNA's
clinical profile in `docs/research/rvdna/`). For a quantum-classical
hybrid algorithm — VQE iterating against a classical optimiser — the
quantum sub-step's witness can be re-verified for any given iterate
and the operator can prove the iterate was deterministic.

**Cache key derivation:** identical to `simulate`. The verify path
*does not* serve from cache; it always re-runs to compare.

**Expected p50:** 2× the `simulate` cache-miss p50 (one fetch + one
fresh re-run; the re-run dominates).

### `replay` (zero-cost fetch from cache)

Given a witness, return the cached `(SimulationResult, RuLakeBundle)`.
Fails if the witness is not in the cache — operator decides whether
to fall back to `simulate` or surface an error.

**Input shape:**
```rust
struct ReplayRequest { witness: String }
```

**Output shape:**
```rust
struct ReplayResponse {
    result: Option<SimulationResult>,
    bundle: Option<RuLakeBundle>,
    cache_status: ReplayStatus,    // Found | NotFound
    elapsed_ms: f64,
}
```

**Cache key derivation:** the witness IS the key.

**Expected p50:** ~30 µs (identical to `simulate` cache-hit path
without the witness-recompute step).

### `optimize` (decompose + transpile, cached)

Run the circuit through the v1 optimizer + decomposer + transpiler,
return the equivalent circuit. The mapping `(input_circuit_hash,
optimization_params) → output_circuit` is itself a cache entry.

**Input shape:**
```rust
struct OptimizeRequest {
    circuit: QuantumCircuit,
    target_backend: BackendType,
    optimizer_passes: Vec<String>, // ["fuse_gates", "decompose:max_qubits=25", ...]
}
```

**Output shape:**
```rust
struct OptimizeResponse {
    output_circuit: QuantumCircuit,
    output_qasm: String,
    bundle: RuLakeBundle,           // witness over the optimization itself
    cache_action: CacheAction,
}
```

**Cache key derivation:** witness over the input circuit's hash plus
the `optimizer_passes` list, with `data_ref = "ruqu://optimize/..."`.
Two requests with identical inputs share a cache entry.

**Expected p50:** ~30 µs on hit; ~5-50 ms on miss depending on circuit
size and pass list (cite `optimizer.rs::fuse_gates` is single-pass
linear scan; decomposition + transpilation are the dominant terms).

### `bench` (exercise all 5 backends, return cost-model coefficients)

Run a fixed benchmark suite across all five backends, return the
measured `SV_NS_PER_GATE`, `STAB_NS_PER_GATE`, `TN_NS_PER_GATE`,
`CT_NS_PER_GATE` coefficients (cite v1 constants at
`vendor/ruvector/crates/ruqu-core/src/planner.rs:160`-area).

**Input shape:**
```rust
struct BenchRequest {
    backends: Vec<BackendType>,    // default: all five
    circuit_sizes: Vec<u32>,       // default: [10, 15, 20, 25]
    iterations: u32,               // default: 5
}
```

**Output shape:**
```rust
struct BenchResponse {
    measurements: Vec<BenchSample>,
    suggested_constants: SuggestedConstants, // for plugging into PlannerConfig
    bundle: RuLakeBundle,           // witness over (host_id, backend_revs, ...)
    elapsed_ms: f64,
}
```

**Cache key derivation:** `data_ref = "ruqu://bench/<host_id>/<backend_rev_hashes>"`,
dim = number of (backend, qubit_size) pairs measured. Re-running
`bench` on the same host with the same backend revisions returns the
cached coefficients in <1 ms.

**Expected runtime:** ~10s for the default suite (5 backends × 4
sizes × 5 iterations × ~100 ms average). Cached re-run: ~30 µs.

## h. MCP tools — quantum surface for agents

A new sibling crate `crates/mcp-ruqu/` (mirrors `crates/mcp-server/`'s shape; see
`crates/mcp-server/src/server.rs:189` for the `#[tool_router]` /
`#[tool(name = "...", description = "...")]` macro pattern) exposes
the five intent verbs as five `#[tool]`s.

### Capability tiers

Mirrors `crates/mcp-server/src/auth.rs::Capability` (Read | Publish | Admin |
Internal). v2 adds three new capabilities specific to quantum
execution:

- `simulate` — can call `ruqu_simulate`, `ruqu_optimize`,
  `ruqu_qec_schedule` against any *simulator* backend. Cannot dispatch
  to hardware.
- `hardware` — can call `ruqu_simulate` against a `Hardware` backend
  (incurs real-device cost; gated separately for billing /
  rate-limiting).
- `verify` — can call `ruqu_verify` and `ruqu_replay`. Read-only
  semantically (does not write to cache); independently grantable
  so an audit-only role can replay without being able to schedule
  new simulations.

### The five tools

```rust
// In crates/mcp-ruqu/src/server.rs, mirroring crates/mcp-server/src/server.rs:189
#[tool_router(router = tool_router)]
impl RuQuMcpServer {
    #[tool(
        name = "ruqu_simulate",
        description = "Public ruQu intent: simulate a circuit on the \
                       planner-chosen backend, write result to lake, \
                       return (result, witness). Capability: simulate \
                       (or hardware for backend=hardware:*)."
    )]
    pub async fn ruqu_simulate(...) -> Result<Json<SimulateResponse>, McpError>
    { /* Cap check, dispatch, audit emit. */ }

    #[tool(
        name = "ruqu_verify",
        description = "Re-run a stored witness against a fresh execution \
                       and compare. Capability: verify."
    )]
    pub async fn ruqu_verify(...) -> Result<Json<VerifyResponse>, McpError>
    { ... }

    #[tool(
        name = "ruqu_replay",
        description = "Zero-cost fetch from the lake by witness. \
                       Capability: verify (semantically read-only)."
    )]
    pub async fn ruqu_replay(...) -> Result<Json<ReplayResponse>, McpError>
    { ... }

    #[tool(
        name = "ruqu_optimize",
        description = "Run the v1 optimizer + decomposer + transpiler, \
                       return the equivalent circuit. Capability: simulate."
    )]
    pub async fn ruqu_optimize(...) -> Result<Json<OptimizeResponse>, McpError>
    { ... }

    #[tool(
        name = "ruqu_qec_schedule",
        description = "Generate a surface-code schedule for the given \
                       distance and round count, optionally optimised \
                       via optimize_feed_forward. Capability: simulate."
    )]
    pub async fn ruqu_qec_schedule(...) -> Result<Json<QecScheduleResponse>, McpError>
    {
        // Wraps vendor/ruvector/crates/ruqu-core/src/qec_scheduler.rs:
        //   - generate_surface_code_schedule(distance, rounds)
        //   - optimize_feed_forward(&schedule) (optional)
        //   - schedule_latency(&schedule, gate_time_ns, classical_time_ns)
        // Returns the schedule + critical-path length + latency estimate.
    }
}
```

### JSON schema for `ruqu_simulate` (illustrative)

```json
{
  "type": "object",
  "required": ["qasm"],
  "properties": {
    "qasm": { "type": "string", "description": "OpenQASM 3.0 program" },
    "backend": {
      "type": "string",
      "enum": ["auto", "state_vector", "stabilizer", "clifford_t",
               "tensor_network", "hardware:ibm:heron-q3", ...],
      "default": "auto"
    },
    "shots": { "type": "integer", "minimum": 1, "default": 1024 },
    "seed": { "type": "integer", "default": 42 },
    "noise_model": {
      "type": "object",
      "properties": {
        "depolarizing": { "type": "number" },
        "bit_flip": { "type": "number" },
        "phase_flip": { "type": "number" }
      }
    },
    "decoder": {
      "type": "string",
      "enum": ["none", "union_find", "subpoly:lut-v3"]
    },
    "mitigation": {
      "type": "string",
      "enum": ["none", "measurement_correction", "zne_3", "zne_5",
               "zne_pec", "full_cdr"]
    },
    "precision": { "type": "string", "enum": ["f32", "f64"], "default": "f64" },
    "no_cache": { "type": "boolean", "default": false },
    "lineage_id": { "type": "string", "description": "OpenLineage job id" }
  }
}
```

### Audit codes — six-code refusal vocabulary

Mirrors ruLake's `WITNESS_MISMATCH_REFUSED` shape at
`crates/mcp-server/src/server.rs:255`-area. All codes use the `RUQU_*`
prefix to avoid namespace collision with `RULAKE_*`.

| Code | Trigger | Recoverable? |
|---|---|---|
| `RUQU_WITNESS_MISMATCH` | `ruqu_verify` re-run produced a different result_hash than the stored bundle's. | No — investigate before proceeding. Possible causes: SIMD-path drift, ruqu-core version skew, cosmic ray. |
| `RUQU_HARDWARE_REFUSED` | `ruqu_simulate` requested `hardware:*` backend without the `hardware` capability. | Yes — caller can request fallback to `auto` (which excludes hardware). |
| `RUQU_PRECISION_DEGRADED` | The chosen backend (e.g. TensorNetwork at high entanglement) had to truncate. The bundle's `lineage_id` records the truncation; the witness covers the truncation parameters. | Yes — the result is approximate but reproducible. Caller decides whether the approximation is acceptable. |
| `RUQU_CACHE_MISS_OVER_BUDGET` | The planner predicted the cache-miss cost (`predicted_runtime_ms`) exceeds the request's `time_budget_ms`. | Yes — caller can raise the budget, switch to a smaller circuit, or accept the refusal. |
| `RUQU_BACKEND_UNAVAILABLE` | The requested backend is not registered in this `mcp-ruqu` instance (e.g. `hardware:ionq:*` requested but no IonQ adapter compiled in). | Yes — caller picks a different backend or queries `ruqu_list_backends`. |
| `RUQU_QEC_DECODER_TIMEOUT` | The QEC decoder (union-find or subpoly) exceeded its decode-time budget. The bundle is *not* written to cache; the partial syndrome is returned for diagnostic purposes. | Yes — caller can retry with a different decoder or a higher timeout. |

Every refusal flows through the same `crates/mcp-server/src/audit.rs`
`AuditEntry` shape — `code: Some("RUQU_*".into())`, `outcome:
"refused"`, `policy_decision: PolicyDecision { capability_required:
"simulate" / "hardware" / "verify", capability_granted: <subject's
caps> }`. Operators get a single audit pipeline for both ruLake and
ruQu refusals.

## i. QEC + noise + mitigation in the witness

This section answers a real soundness question:

> If the active decoder changes — say, union-find vs subpoly — the
> result distribution changes, because different decoders correct
> different errors. So the witness MUST cover decoder choice. What
> exactly flows into the v2 witness?

### The witness-affecting setting list

Every one of these flows into `RuntimeContext` and therefore into the
`Generation::Opaque(...)` packing that feeds `compute_witness`:

1. **Backend id** (`backend_id` field). Already covered: ruqu-state-vector
   vs ruqu-stabilizer vs ... vs ruqu-hardware:ibm:heron-q3.

2. **Noise model id** (`noise_id` field) — a SHAKE-256(8) digest over
   the full noise model parameter set:
   - depolarizing rate (`vendor/ruvector/crates/ruqu-core/src/noise.rs`
     `Depolarizing::new(rate)`)
   - amplitude damping rate
   - phase damping rate
   - any custom Kraus operators (their matrices, in canonical order)

   The id, not the parameters, goes into the witness because the
   parameters can be long (Kraus matrices are 2×2 complex per
   single-qubit channel). The id is sufficient because two noise
   models with the same id have the same parameters by construction.

3. **Decoder choice + parameters** (`decoder_id`,
   `decoder_params_hash` fields):
   - `"none"` (no QEC active)
   - `"union_find"` (`vendor/ruvector/crates/ruqu-core/src/decoder.rs`)
   - `"subpoly:lut-v3"` (`vendor/ruvector/crates/ruqu-core/src/subpoly_decoder.rs`,
     where `lut-v3` is the lookup-table revision)

   Two runs that differ only in `decoder_id` produce different witnesses
   — correctly, because the corrected output differs.

4. **Mitigation choice** (`mitigation_id` field):
   - `"none"`
   - `"measurement_correction"` (the v1 `MeasurementCorrectionOnly`
     strategy at `vendor/ruvector/crates/ruqu-core/src/planner.rs:104`-area)
   - `"zne_3"` (ZNE with three scale factors `[1.0, 1.5, 2.0]`)
   - `"zne_5"` (ZNE with five scale factors `[1.0, 1.25, 1.5, 1.75, 2.0]`)
   - `"zne_pec"` (ZNE + measurement correction)
   - `"full_cdr_n50"` (full pipeline with 50 CDR training circuits)

   Two runs that differ only in mitigation id produce different
   witnesses.

5. **Mixed-precision mode** (`precision_mode` field): `"f32"` or
   `"f64"`. The `vendor/ruvector/crates/ruqu-core/src/mixed_precision.rs`
   module is configurable; the active mode flows into the witness.
   Why: `f32` and `f64` runs of the same circuit can produce
   amplitudes that differ by ULPs, which can flip a measurement bit
   when sampling near 50/50 outcomes. The two runs are different
   answers; cache them as different entries.

6. **SIMD path** (`simd_path` field): `"scalar"` / `"avx2"` /
   `"neon"` / `"rayon"`. This is the load-bearing honesty.

### The SIMD-path-affects-witness gotcha

Floating-point addition is not associative. The scalar gate kernel at
`vendor/ruvector/crates/ruqu-core/src/simd.rs:37`
`apply_single_qubit_gate_scalar` walks amplitudes in a strict
sequential order:

```rust
let mut block_start = 0;
while block_start < n {
    for i in block_start..block_start + step {
        let j = i + step;
        let a = amplitudes[i];
        let b = amplitudes[j];
        amplitudes[i] = matrix[0][0] * a + matrix[0][1] * b;
        amplitudes[j] = matrix[1][0] * a + matrix[1][1] * b;
    }
    block_start += step << 1;
}
```

The AVX2 kernel processes four amplitude pairs at a time (the
`std::arch::x86_64::*` path imported at
`vendor/ruvector/crates/ruqu-core/src/simd.rs:17`); the NEON kernel
does the same with two pairs. The rayon path
(`PARALLEL_THRESHOLD = 65_536`,
`vendor/ruvector/crates/ruqu-core/src/simd.rs:25`) splits the
amplitude vector across threads which then sum independently.

Each path has a *different reduction order* over the same multiplications
and additions. For a 25-qubit StateVector run with non-trivial
entanglement, the resulting amplitudes can differ between paths in
the last ULP or two — well below any quantum measurement's noise floor,
but sufficient to make `result_hash_match` fail in a bit-exact verify.

**v2's stance:** the active SIMD path goes into the witness. Two runs
of the same circuit with `simd_path = "avx2"` and `simd_path =
"scalar"` are *different cache entries*, by design.

The implication: a workstation with AVX2 cannot serve a cache hit to
a workstation without AVX2 (or vice versa) for the *exact same
circuit*. They will see two distinct witnesses for what looks like
the same execution.

This is the right answer. The alternative — pretending the SIMD
path doesn't matter — is the kind of soft lie that undermines the
"two witnesses are interchangeable" property that ruLake's bundle
contract is built on. Operators who want cache-sharing across
heterogeneous hardware should pin `simd_path = "scalar"` at the
cost of speed; this is documented in `integration-with-rulake.md`
§"What v0.1 ships vs v0.2 defers".

### What does *not* go into the witness

- **Wall-clock time.** Two runs at different times share a witness
  if all witness-affecting inputs are equal. The timestamp is
  bundle metadata (`lineage_id`), not a witness input.
- **`software_version`** — see §a "What v2 explicitly drops". The
  ruqu-core point release is metadata, not a witness input. Operators
  who want per-version pinning bump the SIMD-path id manually
  (e.g. `simd_path = "avx2-v2.1"`) when a release affects gate
  semantics.
- **The `WitnessLog::sequence` field.** The hash-chain semantic
  moves to a companion sidecar (`chain.rulake.json`), not the bundle.
  Two bundles with the same witness can appear at different
  sequence numbers in different chains and still be cache-shareable.

## j. Federation across backends and sites

ruLake's federation primitive at `crates/core/src/lake.rs:521`
`search_federated` is a parallel rayon fan-out across `(backend,
collection)` pairs:

```rust
pub fn search_federated(
    &self,
    targets: &[(&str, &str)],
    query: &[f32],
    k: usize,
) -> Result<Vec<SearchResult>>
```

For ruQu v2, the `targets` slice becomes a list of `(backend_id,
circuit_witness_hex)` pairs. The federation fans out across the
five simulation engines registered in the lake, asking each:
"do you have a cached run with this witness?" The v1 search
behaviour is unchanged; we're just calling it with a domain-specific
interpretation of what "search by query vector" means (a witness
lookup is degenerate: k=1, query-vector = the bundle's amplitude
encoding, exact-match required).

### Population-scale use case

Eight nodes deployed across two regions, each with a different mix:

- Node 1-3 (us-central1): StateVector + Stabilizer, RAM-sized for
  20-qubit SV runs, used by VQE workloads.
- Node 4-5 (us-central1): TensorNetwork + Clifford+T, used by
  large-circuit nearest-neighbour workloads.
- Node 6 (us-central1): Hardware (IBM Quantum), gated by `hardware`
  capability, used for verification runs.
- Node 7-8 (eu-west1): same shape as 1-3 (HA failover for the EU
  user base).

Use case: "show me all prior simulations of circuit `abc123...` across
all backends in the federation, ranked by witness-verified concordance."

```rust
// On any node. Walks every registered backend across every node in
// the lake's view (the lake's view is the union of locally-registered
// backends plus IPFS-published bundles per ADR-005).
let witness = ReplayEngine::circuit_hash(&circuit);  // 32-byte hash
let candidate = compute_witness(/* per §c */);
let targets: Vec<(&str, &str)> = lake.backend_ids().iter()
    .map(|b| (b.as_str(), witness_hex.as_str()))
    .collect();
let hits = lake.search_federated(&targets, &[/* witness as query */], 1)?;
// hits is a Vec<SearchResult> with one entry per backend that holds
// the cached run. Each SearchResult { backend, collection, id, score }
// gives the operator a per-backend view; "score" here is the
// witness-equality flag (1.0 for exact match, 0.0 otherwise).
```

The federated answer is "5 of 8 nodes hold this run; here's the
per-backend latency to fetch each." Operators use this to:

1. Ask the node closest to the requesting agent for the result —
   minimising network round-trip.
2. Schedule a `verify` against the *furthest-removed* node's stored
   result for a clinical-grade reproducibility check.
3. Detect out-of-band invalidation: a node that *should* have a
   given witness but doesn't suggests cache eviction or a
   subtle generation-tick issue.

### Cross-site sharing via IPFS (ADR-005 read-only path)

Per ADR-005 (`docs/adrs/sdk/ADR-005-ipfs-backend-and-deploy.md`), the
IPFS backend stores `table.rulake.json` bundles addressed by CID.
For ruQu v2, the same path applies: a `circuit.rulake.json` bundle
is published to IPFS, the CID is shared (e.g. via a paper's
supplementary materials or an internal artefact registry), and any
node with kubo can read the bundle, verify the witness, and — *if it
also has the corresponding cached result* — serve replays for that
witness.

The IPFS backend is *bundle-only* (ADR-005 §"What 'the IPFS backend
stores' — sharpening the scope"); the actual amplitude vectors live
on whatever backend serves them. For a quantum circuit on
StateVector, the amplitudes are 2^n complex numbers — a 20-qubit run
is 2^20 × 16 = 16 MiB. We do not put amplitudes on IPFS in v0.1;
the federation pattern is "share the bundle (witness), each node
keeps its own copy of the result, federated `search_federated`
finds the holding nodes."

This is exactly the rvDNA v2 federation pattern (sister corpus); the
bundles are the portable currency, the bytes stay where they are.

## k. WASM circuits in the browser — the edge story

ruqu-wasm (`vendor/ruvector/crates/ruqu-wasm/`, README at
`vendor/ruvector/crates/ruqu-wasm/README.md`) is a working browser
build of the StateVector backend with a 25-qubit cap. v2 composes
this with rulake-wasm (the existing `node-wasm/` crate that powers
the Console's WASM-local mode at `ui/src/lib/wasm-mode.ts`-area —
see ADR-006).

### The composition

```javascript
// Browser-side, in the Console's Quantum route.
import init as ruquInit, { WasmQuantumCircuit, simulate }
    from '@ruvector/ruqu-wasm';
import init as rulakeInit, { computeWitness, verifyBundleJson }
    from '@ruvector/rulake-wasm';

await Promise.all([ruquInit(), rulakeInit()]);

// 1. Build and run the circuit entirely in-tab.
const qc = new WasmQuantumCircuit(2);
qc.h(0);
qc.cnot(0, 1);
const result = simulate(qc);
// result.probabilities = Float64Array

// 2. Construct a v2 bundle for the run, in-tab.
const ctx = {
  backend_id: "state_vector",
  noise_id: "none-default",
  decoder_id: "none",
  decoder_params_hash: "",
  mitigation_id: "none",
  precision_mode: "f64",
  simd_path: "wasm-scalar",  // wasm has no SIMD path discriminant in v0.1
  shots: 0,                  // amplitudes returned, no shots taken
  seed: 0,
  runtime_class: "simulated"
};
const witness = computeWitness({
  data_ref: `ruqu://state_vector/${qc.circuitHashHex()}`,
  dim: result.probabilities.length * 2,
  rotation_seed: 0,           // wasm-local mode uses 0 by convention
  rerank_factor: 0,
  generation: { Opaque: JSON.stringify(ctx) }
});

// 3. Verify against a remote mcp-ruqu server's published witness.
const remoteResp = await fetch(
  '/mcp-ruqu/ruqu_replay',
  { method: 'POST', body: JSON.stringify({ witness }) }
);
const remote = await remoteResp.json();
const witnessMatch = remote.bundle && remote.bundle.rvf_witness === witness;

// 4. If user opts in, federate: ask the lake for all backends that
//    hold this witness.
if (federationConsent) {
  const fedResp = await fetch('/mcp-ruqu/ruqu_federate',
    { method: 'POST', body: JSON.stringify({ witness }) });
  // ...
}
```

Both wasm modules are existing artefacts. v2 ships *zero new wasm
code*. The Console route is composition.

### The "Quantum" route — peer or sub-mode?

The existing Console (`ui/src/components/screens.jsx`-area, six
routes per `docs/adrs/ADR-006-rulake-console-vite-github-pages.md`:
Stats, Playground, Backends, Bundle, Audit, Connect) gets a seventh
entry. The proposal:

**Quantum is a peer of Stats / Playground / etc — not a Playground
sub-mode.** Defence:

- The Quantum surface introduces a circuit composer (a QASM editor
  plus drag-and-drop gate palette). That UI vocabulary doesn't map
  onto Playground's "search-vector + see results" shape. Forcing it
  into Playground would compromise both routes.
- The Bundle viewer route already does what we need for
  *circuit-witness verification*: it shows the bundle JSON, runs
  `verifyBundleJson`, and displays the verification result. v2's
  Quantum route reuses the Bundle viewer for the verify step —
  the new code is the *composer* and the *result viewer*, not
  the bundle UI.
- Audit and Stats are unchanged: the existing screens already
  surface `RUQU_*` audit codes and per-backend hit-rates without
  modification (the resources are typed by string, not by enum;
  see `crates/mcp-server/src/server.rs` resource definitions).

Architecturally: Quantum is a 7th sidebar entry with two screens —
Composer (QASM editor + gate palette + run button) and Result
Viewer (probability histogram + measurement table + bundle viewer
embed). See `integration-with-rulake.md` §Console hooks for the
component decomposition.

The risk we acknowledge: a 7th route adds Console surface, and the
Console's contract per ADR-006 is "small, surveyable UI." We mitigate
by deferring the gate-palette UI to v0.2 — v0.1 ships with a textarea
QASM editor (the same shape as Playground's textarea search-vector
input). Total new UI surface in v0.1: one new route, two screens,
~400 lines of TSX.

## l. Hardware backend + audit chain

When v2 dispatches a circuit to a real quantum device (the `Hardware`
backend at `vendor/ruvector/crates/ruqu-core/src/hardware.rs`), the
witness chain MUST distinguish "simulated on a workstation" from
"actually ran on an IBM Heron-Q3 with calibration snapshot
2026-04-22T14:30Z". Operators need this for:

- **Compliance.** A clinical-grade study that submits a circuit to
  hardware and bases a decision on the result needs proof that the
  result came from the real device, not a re-played simulation that
  happens to share an input shape.
- **Reproducibility.** Two different calibrations of the same device
  produce different distributions (gate fidelities drift). A cached
  result from yesterday's calibration is not the same as a fresh
  result from today's, even though the circuit and shot count are
  identical.
- **Cost attribution.** Hardware runs cost real money. A cached
  hardware-run is *zero cost on second call* — the operator needs
  to be able to prove "this answer cost $40 the first time, zero
  every time after, and the cache hit was authoritative because
  the witness pinned the calibration."

### The `runtime_class` field

The bundle's `RuntimeContext.runtime_class` field is the load-bearing
discriminant. Possible values:

- `"simulated"` — any of the four simulator backends. No further
  qualification (the backend_id distinguishes them).
- `"hardware:<provider>:<device>:<calibration-snapshot>"` —
  authoritative attribution. Examples:
  - `"hardware:ibm:heron-q3:cal-2026-04-22T14:30Z"`
  - `"hardware:ionq:forte-1:cal-2026-04-23T09:00Z"`
  - `"hardware:rigetti:ankaa-9q:cal-2026-04-22T08:00Z"`

The calibration-snapshot id is a UTC ISO 8601 timestamp truncated to
the device's calibration cadence (typically every few hours for
superconducting platforms, daily for ion traps). Two hardware runs
on the same device with the same calibration snapshot can share a
witness (and therefore a cache entry); two runs on the same device
with different snapshots cannot.

### How the hardware backend ticks `generation`

Per §d.5, the hardware backend's `generation()` method returns a
monotonic counter that increments every calibration snapshot change.
This is *belt-and-braces* with the witness — even if the
`runtime_class` field were ever omitted, the integer generation
would force a cache invalidation.

The hardware backend's `current_bundle` (per §d.5) packs the active
calibration snapshot into the runtime_class string before computing
the witness. Two effects:

1. The witness will not match across calibration boundaries —
   `RUQU_WITNESS_MISMATCH` if a verify is attempted across the
   boundary.
2. The bundle's `lineage_id` carries the full calibration metadata
   (T1/T2 per qubit, gate fidelities, readout error rates), but
   *not* in the witness — the lineage_id is provenance, not
   identity.

### The audit chain integration

Every hardware dispatch emits an `AuditEntry`
(`crates/mcp-server/src/audit.rs::AuditEntry` shape) with:

```rust
AuditEntry {
    ts: now_ts(),
    transport: "stdio" | "http",
    principal: <jwt-derived principal>,
    tool: "ruqu_simulate".into(),
    intent: Some("hardware".into()),
    outcome: "ok" | "refused" | "error",
    code: Some("RUQU_HARDWARE_DISPATCH").into(),  // new code
    witness_in: None,                             // no input witness on first dispatch
    witness_out: Some(bundle.rvf_witness.clone()),
    policy_decision: Some(PolicyDecision {
        capability_required: "hardware".into(),
        capability_granted: <subject's caps>,
    }),
    decision: Some(json!({
        "backend_id": "ruqu-hardware:ibm:heron-q3",
        "runtime_class": "hardware:ibm:heron-q3:cal-2026-04-22T14:30Z",
        "shots": 1024,
        "queue_position": 17,
        "estimated_cost_usd": 0.50,
    })),
}
```

The `decision` field is a free-form JSON blob that the audit
pipeline can index for cost reporting. The `witness_out` is the
authoritative cache key for any future replay.

A subsequent `ruqu_replay` for the same witness emits a different
audit row:

```rust
AuditEntry {
    tool: "ruqu_replay".into(),
    intent: Some("verify".into()),
    code: Some("RUQU_REPLAY_HIT".into()),
    witness_in: Some(<the witness>),
    witness_out: Some(<the same witness>),  // round-trip
    decision: Some(json!({
        "cache_action": "hit",
        "original_dispatch_audit_id": "ulid:01HX...",  // backreference
        "elapsed_ms": 0.027,
    })),
}
```

The backreference `original_dispatch_audit_id` is the cost-attribution
load-bearer: the operator can trace any cached hardware result back
to the original dispatch and the dollar cost recorded there.

## m. Open quantum-supremacy and benchmarking — honest discussion

A fair question:

> If v2 caches a run on real hardware and the next call returns the
> cached result, is that still a "fair benchmark"? A "fair scientific
> result"? A "fair quantum-supremacy claim"?

Three different framings, three different answers. We treat them
honestly because the alternative — silent caching of expensive
results — would be malpractice.

### Framing 1: benchmarking

If you are benchmarking quantum hardware, the cache is poison. A
cached re-run is not a fresh execution, no matter how authoritative
the witness. v2 ships `--no-cache` as a first-class flag on
`ruqu_simulate`:

```rust
struct SimulateRequest {
    ...
    no_cache: bool,  // default: false
}
```

When `no_cache: true`:

- The `CacheAwarePlanner` skips the cache pre-pass entirely (§e).
- The simulation runs unconditionally on the planner-chosen backend.
- The result *is* still written to the cache (so the next non-bench
  caller benefits), but the bench result returned to the caller
  comes from this fresh execution.
- The audit row carries `RUQU_NOCACHE_BENCH` as a reason code so
  the operator can audit which calls bypassed cache.

Benchmark suites that need scientific rigour set `no_cache: true`
on every call. A bench harness that *forgets* to set it is a bug
the audit log will surface immediately.

### Framing 2: scientific reproducibility

If you are publishing a result that says "circuit C run on backend B
produced distribution D", the cache is *fine* — the witness *proves*
the inputs were what you say they were, and the result_hash *proves*
the outputs were what you say they were. A cached result is a
*reproduction*, not a fresh experiment.

In this framing, v2 ships `verify` as a first-class verb:

- `ruqu_verify` always re-runs (§g) and compares result_hash.
- A successful verify *is* the reproducibility proof — same inputs,
  same outputs, in two independent executions.

Operators publishing scientific results run `verify` on every claim
they make and include the `expected_witness` in the supplementary
materials. Reviewers run their own `verify` against the same
witness; if it matches, the result is reproducible by the
"identical-execution" definition.

### Framing 3: quantum-supremacy claims

This is the strongest framing, and the most fraught. A quantum-
supremacy claim says "this computation is infeasible on classical
hardware; the result was produced by a real quantum device." A
cached result is *not* a quantum-supremacy demonstration — it's a
classical cache lookup of a previous quantum-device output.

v2's discipline:

- Quantum-supremacy claims must be made about *fresh dispatches*
  with `no_cache: true` and `runtime_class: "hardware:*"`. A claim
  made about a cached result is, by construction, a
  classical-cache claim, not a quantum-supremacy claim.
- The Console's Quantum route surfaces `runtime_class` prominently
  in the result viewer; cached hardware results display a clear
  "served from cache; original dispatch audit id: ..." banner so
  the operator (and any reviewer with Console access) cannot
  mistake a cached result for a fresh one.
- The audit code `RUQU_REPLAY_HIT` (per §l) is the machine-
  readable equivalent: any audit-pipeline filter that wants to
  exclude cached results from a supremacy claim can filter on
  this code.

We are *not* in the business of laundering cache hits as fresh
hardware results. The witness mechanism gives operators the *option*
to rerun on hardware (`no_cache: true`) or to accept the cached
result with full knowledge that it's a cache hit (default).
Operators who confuse the two have a process problem, not a tooling
problem; v2's job is to make the distinction unambiguous in every
artefact (bundle, audit row, Console UI), and we do.

## n. Validation test — five-stage acceptance

Mirrors the rvDNA spec's §l acceptance shape. ADR-008's Verification
section makes these the five gates that block ADR-008 acceptance.

### Stage G1: Witness equivalence proves out

**Test:** Encode 1000 pre-defined circuits (mix of Bell, GHZ, VQE
ansatzes, surface-code distance-3, random circuits at 5/10/15/20
qubits). For each, compute the v1 `ExecutionRecord` hash AND the v2
`RuLakeBundle::rvf_witness`. Verify that:

- Two encodes of the same `(circuit, ctx)` pair produce identical
  v2 witnesses.
- The v1 hash and the v2 witness are *both* deterministic — re-running
  the encode 100 times produces the same pair every time.
- A targeted mutation in any witness-affecting field (per §i) produces
  a different v2 witness, while a mutation in `software_version`
  or `timestamp_utc` does *not* (per §c "What v2 explicitly drops").

**Pass criterion:** 1000/1000 circuits exhibit the bijection; zero
collisions; the SHAKE-256 hexstring is stable across processes and
across machines.

### Stage G2: Cross-process replay sub-1 ms

**Test:** Two processes (P1 and P2) on the same machine, sharing the
same on-disk lake. P1 runs `ruqu_simulate` for circuit C and gets
result R1 with witness W. P2 runs `ruqu_replay` with witness W.

**Pass criterion:** P2's replay returns within 1 ms (p50) and 5 ms
(p99) for 100 trials. The result returned by P2 is bit-identical
to R1 (`result.amplitudes == R1.amplitudes` for SV; equivalent
equality for the other backends).

### Stage G3: Hardware-cache attribution

**Test:** Dispatch a 5-qubit circuit to a `Hardware` backend (the
test mocks the hardware adapter; real-device CI is out of scope).
The mock returns a deterministic histogram with calibration snapshot
`cal-2026-04-22T14:30Z`. Verify:

- The bundle's `runtime_class` field is exactly
  `"hardware:mock:test-device:cal-2026-04-22T14:30Z"`.
- The audit row for the dispatch has `code: "RUQU_HARDWARE_DISPATCH"`
  and the witness in `witness_out`.
- A subsequent replay returns the same witness in `witness_out` and
  carries `code: "RUQU_REPLAY_HIT"` plus `original_dispatch_audit_id`
  in the `decision` JSON.
- A simulated calibration-snapshot bump (mock returns
  `cal-2026-04-22T17:30Z`) changes the witness, and a replay against
  the *old* witness still works (the cache entry is preserved; the
  *new* circuit dispatch creates a new entry).

**Pass criterion:** all four bullets pass for 10 trials.

### Stage G4: Clifford concordance — stabilizer-sim ↔ hardware

**Test:** A pure-Clifford circuit (e.g. a 4-qubit GHZ-state
preparation) is dispatched two ways:

1. To the StabilizerBackend (exact, all-Clifford).
2. To the mock HardwareBackend (which reports the same circuit run
   on an IBM Heron-Q3 mock with negligible noise).

Both runs produce result histograms (Stabilizer needs to add a
measurement step; Hardware does so by definition).

**Pass criterion:** the two histograms agree to within statistical
fluctuation (chi-squared test at p > 0.01) for 10 trials. The two
witnesses are *different* (different backend_id, different
runtime_class) — this is correct; the test is on result agreement,
not witness agreement.

This gate is the soundness check on the Clifford backends: a
witness-anchored cache is only useful if the underlying
backends agree on the actual physics for circuits where they
*should* agree.

### Stage G5: Audit log round-trip parity

**Test:** Run a sequence of ten `ruqu_*` calls (mix of simulate,
verify, replay) against `mcp-ruqu`. In parallel, run a sequence of
ten `rulake_*` calls against `mcp-server`. Assert that:

- Both servers' audit logs share the exact same `AuditEntry` schema
  (cite `crates/mcp-server/src/audit.rs::AuditEntry`).
- The `code` fields use disjoint prefixes (`RUQU_*` vs `RULAKE_*`).
- A single audit pipeline (e.g. a `tail -F` reader merged across
  both files) produces a unified, time-ordered stream that a
  downstream observer can parse uniformly.

**Pass criterion:** schema compatibility, prefix disjointness, and
unified-stream parseability all verified for the 20 audit rows.

### Cross-process dedup as the umbrella property

All five gates together prove the umbrella claim from §b goal 2:
the same circuit, run on the same backend with the same settings,
costs zero on second call. G1 establishes the cache key; G2 measures
the hit-path latency; G3 establishes the hardware-attribution; G4
establishes physics concordance; G5 establishes the audit-pipeline
property that makes cache-share auditable.

## o. Migration from v1

v1 emits a `WitnessLog` whose `to_json()` (`vendor/ruvector/crates/ruqu-core/src/witness.rs:203`)
output is a JSON array of entries, each containing `circuit_hash`,
`seed`, `backend`, `noise_config`, `shots`, `software_version`,
`timestamp_utc`, `result_hash`, `prev_hash`, `entry_hash`,
`sequence`. v2 reads this format via a `migrate` subcommand:

```
$ ruqu-cli migrate --from-v1 path/to/v1-witness.json --to-v2 path/to/v2-bundles/
```

For each v1 entry, the migrate command:

1. Synthesises a `RuntimeContext` per §c. Fields with no v1
   counterpart (`decoder_id`, `mitigation_id`, `precision_mode`,
   `simd_path`, `runtime_class`) are filled with defaults
   (`"none"`, `"none"`, `"f64"`, `"unknown"`, `"simulated"`).
2. Constructs a `RuLakeBundle` with `data_ref =
   "ruqu://<backend>/<hex(circuit_hash)>"` and
   `Generation::Opaque(serde_json::to_string(&ctx))`.
3. Writes one `circuit-<hex(witness)>.rulake.json` per v1 entry to
   the output directory.
4. Emits a `chain.rulake.json` companion that links the bundles in
   sequence, mirroring the v1 hash-chain semantic.

**The "default migration" caveat:** the v2 witnesses produced by
migrate will *not* match witnesses that v2 would produce for fresh
runs of the same circuits. Reason: the v1 record doesn't capture
the SIMD path, the decoder, or the mitigation; the migration fills
them with `"unknown"` / `"none"`, which is a different witness from
e.g. `"avx2"` / `"union_find"`. Operators who care about this need
to manually annotate the migration with the correct settings — the
migrate command surfaces a `--annotate <toml>` flag for this.

Operators who don't migrate: v2 reads v1 `WitnessLog` files
indefinitely (the parse code is in the migrate subcommand's library
form and stays compiled in). v1 logs are *read-only* in v2 — you
cannot append v1-format entries from v2; new entries always use the
v2 bundle shape.

## p. Open questions — 5–7 honest unknowns

### 1. Hardware-backend witness when calibration drifts mid-run

A 60-shot run on IBM Heron-Q3 takes ~30 seconds total wall-clock.
If the device recalibrates *during* the run (which shouldn't happen
but does in pathological cases), the bundle's `runtime_class` would
record the *initial* calibration snapshot, even though some shots
were taken under a later snapshot. The witness would still match a
re-run with the initial snapshot — but the actual measurement
distribution might differ slightly because of the mid-run drift.

**Possible answers:**
- Per-shot calibration recording (expensive, possibly impossible to
  query at the device API level for some providers).
- Fail-fast on detected drift (the hardware backend polls
  calibration snapshots during long runs and aborts on change).
- Accept the drift as bundle-metadata noise (the `lineage_id`
  records every snapshot id seen during the run; the witness uses
  the initial one).

v0.1: option three (record in `lineage_id`, not the witness).
This is a deliberate choice that v0.2 may revisit if we observe
real drift in practice.

### 2. Quantum-classical hybrid algorithms — what witness covers the classical part?

VQE iterates: quantum sub-step → classical optimizer → next quantum
parameters. The quantum sub-step has a clean witness per §c. The
classical optimizer's behaviour (which variant — COBYLA vs SPSA;
which hyperparameters; which random seed for stochastic optimisers)
is *not* captured by ruQu's witness today.

**Possible answers:**
- Treat hybrid algorithms as *out of scope* for the v2 witness —
  each quantum sub-step is individually verifiable, but the
  iterative trajectory is not. (Operators wrap the VQE driver in
  their own witness layer.)
- Extend the witness to include a `classical_context` field
  alongside `runtime`. Two VQE runs that differ in optimiser hyper-
  parameters produce different witnesses.
- Punt to ruqu-algorithms (`vendor/ruvector/crates/ruqu-algorithms/src/vqe.rs`)
  to define its own witness shape that composes with ruqu-core's.

v0.1: option one. v0.2 may pick option three; option two is
unlikely because it would couple ruqu-core to a wide range of
classical optimiser libraries.

### 3. The "is a cached run still scientific?" policy from §m

We've stated v2's stance (cached results are reproductions, not
fresh experiments; supremacy claims need `no_cache: true`). But:

- What about *partial* cache hits? A 100-shot run cached at 50
  shots — is the cached run scientifically equivalent to a fresh
  100-shot run with the cached half + 50 fresh? (No, but the
  shape of the answer is fiddly.)
- What about the "pre-warm by replay" pattern — an operator who
  replays a witness, then *uses the result as if they had just
  fresh-run it*. This is technically allowed by v2 today; we don't
  have a mechanism to prevent it.

**Possible answers:**
- Add a `freshness_required` flag to `ruqu_replay` that, when set,
  refuses to serve from cache if the original dispatch is older
  than a caller-specified TTL.
- Make `ruqu_replay` return a richer status that explicitly
  distinguishes "fresh first call" from "Nth replay" so the
  caller can filter.

v0.1: ship the basic mechanism; v0.2 considers the freshness
extensions based on operator feedback.

### 4. Tier-2 backend storage costs at federation scale

A 25-qubit StateVector amplitude vector is 2^25 × 16 bytes = 512 MiB.
Caching N such vectors locally is cheap; caching N across 8 federated
nodes is 8N. ruLake's RaBitQ compression brings this down by ~32×
(from f32 to 1-bit codes), so federated storage is 8N / 32 = N/4
of the original — manageable, but not free.

**Possible answers:**
- Tiered cache: nodes hold T0 (recent runs, full uncompressed), T1
  (middle-age runs, RaBitQ-compressed), T2 (old runs, witness-only,
  result fetched on demand from the original dispatching node).
- Operator-driven eviction: the lake's existing
  `cache_stats_by_collection` (`crates/core/src/lake.rs:136`) gives operators
  per-collection visibility for manual eviction.
- Federation-aware RaBitQ: the rotation seed could be *backend-
  specific*, so two backends' RaBitQ codes for the same amplitude
  vector are different and can be deduped at the backend boundary.
  (This breaks the cross-backend cache-share for amplitude codes,
  which is mostly fine because amplitude bytes are not the
  dominant federation currency — bundles are.)

v0.1: punt. The cache is per-process; federation shares bundles
(small) not amplitudes (large). v0.2 considers the tiered design
if/when amplitude-bytes federation becomes a use case.

### 5. The OpenQASM-export interplay with witness rotation

If `ruqu_optimize` produces an equivalent circuit (different gate
sequence, same unitary), and we cache the *output* circuit by
witness, but the witness covers the input circuit hash, then two
optimize-then-simulate sequences with different optimizer pass
lists will produce different witnesses for what is *physically* the
same simulation.

**Possible answers:**
- Accept this. The witness covers the *path*, not just the *result*.
  Two paths that produce the same final amplitudes are different
  witnesses; the cache deduplicates on path identity, not result
  identity.
- Add a *result-witness* alongside the path-witness. The path-
  witness keys the optimize cache; the result-witness keys the
  simulation cache. A second `simulate` call on a different-path
  circuit that produces the same amplitudes would hit the result-
  witness cache.

v0.1: option one (path-witness only). v0.2 might add result-witness
if benchmarks show meaningful redundancy.

### 6. What about the parallel ruQu (capital Q) crate at vendor/ruvector/crates/ruQu/?

`ruQu/` is a *separate* crate — a "Classical Nervous System for
Quantum Machines" focused on real-time syndrome processing and
coherence assessment (`vendor/ruvector/crates/ruQu/src/lib.rs:1`).
It's tile-based, three-filter coherence-gate, < 4 µs p99 gate
decision latency. Not the same as ruqu-core.

**Question:** does ruQu (capital Q) also benefit from witness
anchoring? Does its syndrome buffer count as something that should
flow through ruLake?

**Stance:** out of scope for ruQu v2 corpus (this document is about
the ruqu-core / ruqu-algorithms / ruqu-exotic / ruqu-wasm family).
ruQu (capital Q) is a separate compositional question — possibly
worth its own future "ruQu-cap-Q v2" corpus. Flagging here so the
reader who follows up doesn't conflate the two.

### 7. ruqu-exotic and ruqu-algorithms crates — do they need v2 surface?

`ruqu-algorithms` (`vendor/ruvector/crates/ruqu-algorithms/src/`)
ships VQE, Grover, QAOA, Surface Code. `ruqu-exotic`
(`vendor/ruvector/crates/ruqu-exotic/src/`) ships
quantum-classical hybrids (memory decay, interference search,
reasoning QEC, swarm interference).

Both crates *consume* `ruqu-core` and would automatically benefit
from the v2 witness when they call `Simulator::run_with_lake(...)`.
But they may want their own algorithm-level cache keys — a Grover
search of 4 qubits for target 1010 is "the same Grover" regardless
of how many internal simulator iterations it took.

**Stance:** v0.1 ships v2 at the ruqu-core layer only. ruqu-
algorithms and ruqu-exotic continue to call into ruqu-core; their
v2 witnesses are inherited from ruqu-core's. v0.2 may add
algorithm-level wrappers (e.g. `Grover::run_with_lake`) that compute
algorithm-level witnesses. Out of scope here, flagged for follow-up.
