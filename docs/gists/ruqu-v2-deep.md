# ruQu v2 — A Deep Introduction

## TL;DR

ruQu v2 is a witness-anchored execution layer on top of a five-backend Rust quantum simulator (StateVector, Stabilizer, Clifford+T, TensorNetwork, Hardware), recasting every circuit run as a content-addressable artefact instead of a one-shot computation. It folds the v1 `ExecutionRecord` and hash-chained `WitnessLog` into a `RuLakeBundle`, so that two processes asking for the same circuit on the same backend with the same noise / decoder / mitigation / SIMD path get the second answer for free out of the lake's cache. ruLake is a natural host because the bundle witness, the backend trait, and the federation primitive were all designed for exactly this kind of "expensive thing computed once, reused everywhere" pattern.

## Introduction

Quantum execution has, historically, been a one-shot affair. You write a circuit, you pick a simulator (or a hardware queue), you wait, you get amplitudes or histograms back, and the next time someone asks the same question you wait again. The reasons are mostly cultural — in the early years there was no settled identity for a "circuit" beyond its source text, and no convention for binding noise, decoder, and SIMD path into something you could name and look up. Tools that did track execution metadata (Qiskit's job records, Cirq's experiments) treated it as audit, not as cache key.

ruQu v1 (`vendor/ruvector/crates/ruqu-core/`, 26,093 lines across 30 source files) took the audit-trail story further than most. Its `ExecutionRecord` (`vendor/ruvector/crates/ruqu-core/src/replay.rs:57`) covers `circuit_hash`, `seed`, `backend`, `noise_config`, `shots`, `software_version`, `timestamp_utc`. Its `WitnessLog` (`vendor/ruvector/crates/ruqu-core/src/witness.rs:88`) hash-chains those records with a 32-byte digest computed via `DefaultHasher` (SipHash, four rounds). The chain is tamper-evident; modify any link and verification fails. What v1 did not have was a way to *use* this record as a cache key across processes — the digest function was non-cryptographic (SipHash is designed for HashMap collision resistance, not for cryptographic identity), and the `WitnessLog` lived in process memory, serialised by hand-rolled JSON to a sidecar that no other system knew how to interpret.

Meanwhile, ruLake had been shipping a different problem's solution. Its `RuLakeBundle` (`crates/core/src/bundle.rs:113`) anchors a vector cache entry with a SHAKE-256(32) witness over `(data_ref, dim, rotation_seed, rerank_factor, generation)`, computed by `compute_witness` (`crates/core/src/bundle.rs:362`). Two ruLake instances reading the same Parquet file derive the same witness independently and share the cache. The bundle has been ratified as a memory substrate (ADR-156), distributed over IPFS (ADR-005), and exposed through an MCP server with eight tools.

ADR-008 spotted the structural overlap (`docs/adrs/ADR-008-ruqu-as-rulake-substrate.md:106`-area): both designs produce a 32-byte digest over a deterministic concatenation of execution-affecting inputs. The v1 entropy fits inside the v2 hash; the only thing missing is the bridge. v2 builds it. The v1 `ExecutionRecord` field set folds into a `RuLakeBundle`, the SipHash chain becomes a SHAKE-256 chain (a strict upgrade in collision resistance), and the per-execution metadata that used to live in v1's `WitnessLog` lives in a `chain.rulake.json` companion sidecar with a `prev_witness` link per entry.

A precomputed, witness-anchored representation matters here for a different reason than it does in the data-lake setting. Quantum simulation is *expensive in human-perceptible ways*: a 25-qubit StateVector circuit can take 8 seconds to simulate exactly, a TensorNetwork contraction at modest bond dimension takes seconds-to-minutes, a hardware dispatch takes 30+ seconds of queue plus execution and costs real money. The marginal cost of the *second* call to the same circuit on the same backend with the same settings should not be the same as the first. With v2 it isn't: the second call is the SHAKE compute (microseconds) plus a HashMap lookup (microseconds), which is five orders of magnitude under the original on the StateVector path and arbitrarily larger on the hardware path.

The honest part is what v2 is *not* claiming. It is not claiming that a cached hardware result is a fresh quantum-supremacy demonstration. It is not claiming SIMD-path-portable bit-exact results. It is not absorbing the `ruqu-core` simulation engines or replacing OpenQASM. The witness is honest about every input that affects bit-exact output (including the SIMD path, including the calibration snapshot for hardware), and the audit codes (`RUQU_REPLAY_HIT`, `RUQU_NOCACHE_BENCH`, `RUQU_HARDWARE_DISPATCH`) make the distinction between "served from cache" and "fresh dispatch" unambiguous in every artefact.

## The execution model

The five simulation backends are unchanged from v1 in their physics; v2 wraps each one in a `BackendAdapter` impl (`docs/research/ruqu/v2-spec.md:341`-area). The choice of backend is what determines the right pick for a given circuit, and the v1 cost-model planner already encodes the routing rules (`vendor/ruvector/crates/ruqu-core/src/planner.rs:213`). The table below is the operator-facing cheat sheet:

| Backend | What it does | Right pick when | Cache-key shape |
|---|---|---|---|
| StateVector | Exact dense simulation, up to ~25–32 qubits | Small N, mixed gate set, you need exact amplitudes | `data_ref = ruqu://state_vector/<circuit_hash>`, `dim = 2 * 2^n` |
| Stabilizer | Tableau simulation, millions of qubits | All-Clifford circuits (H, S, CNOT) — exact, polynomial | `data_ref = ruqu://stabilizer/<circuit_hash>`, dim = packed tableau bits |
| Clifford+T | Stabilizer + T-gate term decomposition | Mostly Clifford with moderate T-count (planner caps at 40) | dim = min(t_count, max_terms_returned), `RuntimeContext.t_count` in witness |
| TensorNetwork | MPS-based contraction at bounded bond dimension | Large nearest-neighbour circuits, chi-bounded approximation OK | dim varies with chi, `bond_dim_max` and `truncation_threshold` in witness |
| Hardware | Real-device dispatch (IBM Quantum, IonQ, …) | You actually want device output, with calibration provenance | dim = histogram bin count, `runtime_class = hardware:<provider>:<device>:<cal-snapshot>` |

The "collection" notion in the trait is repurposed: a collection is a registered `circuit_hash`, and the "vectors" returned by `pull_vectors` are the result encoded in whatever shape that backend produces (interleaved real/imaginary amplitudes for StateVector, packed tableau bits for Stabilizer, flattened MPS tensors for TensorNetwork, histogram counts for Hardware). The trait does not know about quantum mechanics; it knows about IDs, vectors, dimensions, and generations.

The `Generation::Opaque` payload that flows into the witness is a `RuntimeContext` JSON serialised with serde (`docs/research/ruqu/v2-spec.md:255`-area):

```rust
struct RuntimeContext {
    backend_id: String,        // "state_vector" | "stabilizer" | ...
    noise_id: String,          // SHAKE-256(8) over the full noise model
    decoder_id: String,        // "union_find" | "subpoly:lut-v3" | "none"
    decoder_params_hash: String,
    mitigation_id: String,     // "none" | "zne_3" | "full_cdr_n50" | ...
    precision_mode: String,    // "f32" | "f64"
    simd_path: String,         // "scalar" | "avx2" | "neon" | "rayon"
    shots: u32, seed: u64,
    runtime_class: String,     // "simulated" | "hardware:ibm:heron-q3:cal-2026-04-22T14:30Z"
}
```

That packing is what makes the witness honest about everything that can perturb a bit-exact result. It is also what makes the witness *unintuitive* to first-time integrators — two workstations with different SIMD support cannot share cache entries for the same circuit, because the floating-point ordering differs and the witness rotates. The remediation is documented and operator-visible: pin `simd_path = "scalar"` to share across hardware at a speed cost, or accept per-host cache shards.

The cost-model planner (`vendor/ruvector/crates/ruqu-core/src/planner.rs:213`) becomes cache-aware in v2 by way of a wrapper, `CacheAwarePlanner` (`docs/research/ruqu/v2-spec.md:632`-area). The wrapper computes the anticipated witness from inputs, queries `lake.cache_stats_by_backend()` (`crates/core/src/lake.rs:127`) for any backend currently holding it, and returns `Hit { backend_id, witness }` on hit or `Miss { plan, anticipated_witness }` on miss. The first call pays the v1 simulation cost; subsequent calls pay microseconds.

## Capabilities

**Cross-process replay via witness.** Two processes on the same machine sharing one ruLake see the second-call cost drop to ~30 µs regardless of how expensive the first call was. ADR-008's gate G2 (`docs/adrs/ADR-008-ruqu-as-rulake-substrate.md:469`-area) sets the bar at p50 < 1 ms, p99 < 5 ms over 100 trials, with bit-identical amplitude reproduction. A 25-qubit StateVector circuit goes from 8 seconds to 30 µs (five orders of magnitude); a hardware dispatch goes from 30+ seconds of queue plus billable cost to a single SHAKE compute and a HashMap lookup. The replay cost is independent of the original execution cost — that is the whole point of content-addressing.

**Zero-recompute on witness match.** When a caller hands `ruqu_replay` a witness, the lake either has the result or it doesn't; on a hit, the response carries the cached `(SimulationResult, RuLakeBundle)` and an audit row tagged `RUQU_REPLAY_HIT` with `original_dispatch_audit_id` in its decision JSON for cost attribution. On a miss, the caller decides: fall back to `simulate`, or surface the miss to its own caller. This is the path agentic systems iterate on for VQE-style trajectories where the same parameter combinations recur frequently.

**Multi-backend equivalence checking.** The fourth acceptance gate (G4, "Clifford concordance") dispatches the same all-Clifford circuit two ways: through the StabilizerBackend (exact tableau) and through the HardwareBackend (real device, or a mock with negligible noise). The two histograms must agree to within statistical fluctuation by chi-squared test, *and* the two witnesses must differ (different `backend_id`, different `runtime_class`). Concordance is on results, not on witnesses — the witness rotates with backend identity by construction. This is the soundness check that says "a witness-anchored cache is only useful if the underlying backends agree on the physics for circuits where they should agree."

**Worked example.** Imagine you have an agent that runs VQE for a small chemistry problem. The ansatz has two parameters; the optimiser explores ~200 settings before converging. Without ruQu v2, every parameter setting is a fresh StateVector simulation (perhaps 50 ms each); the run takes ~10 seconds. With ruQu v2, the *first* trajectory pays full cost; subsequent runs of the *same* optimiser on the *same* ansatz with overlapping parameter visits pay 30 µs per cache hit. If a related agent five minutes later visits the same parameter region (because the optimiser landscape is unchanged), the cache hits compound. The benefit is not "make VQE faster" — it is "make repeated VQE-shaped queries free, so the agentic loop above the simulator can ask exploratory questions cheaply." For benchmarking or supremacy claims, the same agent passes `no_cache: true` on `ruqu_simulate` to bypass the cache pre-pass entirely; the result is still written for the next non-bench caller, but this caller's response comes from a fresh execution and the audit row carries `RUQU_NOCACHE_BENCH` so cost rollups exclude it.

The federation surface follows from the same primitive that powers rvDNA and the GCS / IPFS backends. `lake.search_federated` (`crates/core/src/lake.rs:521`) over `(simulation_backend, circuit_hash)` pairs becomes "ask all five quantum engines, in parallel, for prior runs of this circuit." Zero new federation code; zero new MCP tool.

## Trust chain

The trust chain rests on the same `compute_witness` function rvDNA uses, with a different `Generation` payload. Walking through it:

`RuLakeBundle::new` (`crates/core/src/bundle.rs:166`) constructs the bundle, computing the witness from `(data_ref, dim, rotation_seed, rerank_factor, generation)`. The witness is SHAKE-256(32), domain-prefixed by `rulake-bundle-witness-v1|`, length-prefixed at every variable-length input. The `Generation` variant tag at `crates/core/src/bundle.rs:82` distinguishes `Num` from `Opaque` so a numeric generation cannot collide with a JSON-shaped one.

For ruQu, the inputs are these:

- `data_ref = "ruqu://" + backend_id + "/" + hex(circuit_hash)`. The backend identity is part of the URI, so `state_vector` and `stabilizer` over the same circuit get distinct witnesses.
- `dim` is whatever shape the backend's result vector takes (2^n for StateVector amplitudes, packed bits for Stabilizer, etc.).
- `rotation_seed` and `rerank_factor` are inherited from the lake instance; constants from ruQu's perspective.
- `generation = Generation::Opaque(serde_json(RuntimeContext))`. The full noise / decoder / mitigation / SIMD / runtime-class packing.

Two replicas converge if and only if all of these match. They diverge cleanly when any one moves: a noise model bump, a decoder swap, a calibration snapshot, a SIMD path change. The reference impl asserts this property at `crates/ruqu-backend/src/witness.rs:42` and the test below it.

The hash function strengthens, not weakens, relative to v1. SipHash (the v1 `DefaultHasher`) is designed for HashMap collision resistance; SHAKE-256(32) is a NIST-approved extendable-output function with cryptographic collision resistance. The v1 hash-chain semantic — every entry's `prev_hash` links to the predecessor — is preserved by the v2 chain sidecar (every bundle's `prev_witness` field links to the predecessor's `rvf_witness`), but the per-link strength moves from non-cryptographic to cryptographic. ADR-008's gate G1 ("witness equivalence proves out") makes this a tested property: 1000 random circuits, 100% of two-encode pairs identical, 100% of mutation-of-witness-affecting-fields different, zero collisions.

The deliberate semantic relaxation is `software_version` and `timestamp_utc`. v1 folded them into the hash; v2 demotes them to bundle metadata (`lineage_id`). The justification is that two ruqu-core point releases producing bit-identical amplitudes should share a cache entry; operators who need per-version pinning add a witness-affecting setting through the `runtime_class` field instead. ADR-008 §Decision 1 makes this trade-off explicit and audit-visible.

## Reference implementation status

The crate `ruvector-rulake-ruqu` v0.0.1 lives at `crates/ruqu-backend/`. What it ships today:

- `RuquStateVectorBackend`, the `BackendAdapter` impl for small-N exact simulation (`crates/ruqu-backend/src/lib.rs:79`-area). v0.0.1 caps at 16 qubits to keep RAM bounded; the production backend handles up to 25.
- A mini circuit IR — `Circuit`, `Gate { H, X, Y, Z, S, T, CNOT, RZ }` — at `crates/ruqu-backend/src/circuit.rs`. Enough to run Bell pairs end-to-end and prove the witness path; the OpenQASM 3.0 frontend is roadmapped for v0.0.2.
- `simulate` and complex-number type `C` at `crates/ruqu-backend/src/state_vector.rs`. Exact dense simulation against the mini IR.
- The `witness` module at `crates/ruqu-backend/src/witness.rs:42`. Same construction path as any other lake backend (`RuLakeBundle::new`), tagging `memory_class = "quantum"`. Backend tag flows into `data_ref` so two backends over the same circuit get distinct witnesses.
- Two Criterion benches scaffolded for eventual acceptance: `simulate` and `execute_and_pull`.

What v0.0.1 does *not* ship, and which is roadmapped per ADR-008 §Decision 2 and the integration plan:

- The four other simulation backends (Stabilizer, Clifford+T, TensorNetwork, Hardware). Each is a separate `BackendAdapter` impl gated by a Cargo feature. `state-vector` and `stabilizer` are intended as the WASM-default pair; `clifford-t`, `tensor-network`, `hardware` pull in heavier dependencies.
- The `CacheAwarePlanner` wrapper around v1's `plan_execution`, plus the non-breaking ruLake addition that `PerBackendStats` exposes its `witnesses_held()` set (ADR-008 §Decision 3).
- `crates/mcp-ruqu/`, the sibling MCP server with the five intent verbs (`ruqu_simulate`, `ruqu_verify`, `ruqu_replay`, `ruqu_optimize`, `ruqu_qec_schedule`). PR 2 in the integration plan.
- The `chain.rulake.json` companion sidecar (the v1 `WitnessLog` semantic preserved as `prev_witness` links between bundles). PR 3.
- The Console's seventh sidebar entry (`Quantum`) composing `ruqu-wasm` (in-tab simulation) with `rulake-wasm` (`verifyBundleJson` for witness verification). PR 4.
- The `MockHardwareBackend` that lets gates G3 (hardware-cache attribution) and G4 (Clifford concordance) run in CI without a real device.
- Migration from v1's `WitnessLog` JSON via a `ruqu-cli migrate --from-v1` subcommand.

The five acceptance gates are itemised in ADR-008 §Verification: G1 witness equivalence (1000 random circuits, bijection, zero collisions), G2 cross-process replay (<1 ms p50), G3 hardware-cache attribution (`runtime_class` correctness, `RUQU_REPLAY_HIT` audit code, calibration-snapshot rotation), G4 Clifford concordance (chi-squared on results, witness inequality), G5 audit log round-trip parity with ruLake (shared `AuditEntry` schema, disjoint code prefixes). v0.1 partial-acceptance is G1 + G2 + G5; full acceptance is all five after the mock hardware backend lands in v0.2.

## Composition with ruLake

The pattern is the same one rvDNA uses, applied to a different domain.

**`BackendAdapter` is the contract.** Five `ruqu-*` adapters (one per simulation engine) implement the same trait at `crates/core/src/backend.rs:110` that `LocalBackend`, GCS, IPFS, and `RvdnaT0Backend` implement. From ruLake's perspective, a registered ruQu backend is just another source of vectors, and a "circuit_hash" is just a collection name.

**The cache witness is shared.** Every ruQu backend's `current_bundle` constructs a `RuLakeBundle` through `RuLakeBundle::new`, which calls `compute_witness`. Two ruLake instances running the same circuit on the same backend with the same `RuntimeContext` derive the same witness independently and share the cache through `crates/core/src/cache.rs::install_prebuilt_interned`. Cross-deployment via IPFS works the same way — a `circuit.rulake.json` published to IPFS at a CID is verifiable end-to-end without ever shipping the amplitude bytes.

**Federation is fan-out.** `RuLake::search_federated` over `(simulation_backend, circuit_hash)` pairs gives "ask all five engines for prior runs of this circuit" in one call. The `mcp-server` audit pipeline (`crates/mcp-server/src/audit.rs::AuditEntry`) is shared with `mcp-ruqu`; disjoint `RUQU_*` vs `RULAKE_*` code prefixes let one log stream serve both servers, and a shared `audit-only` Cargo feature on `mcp-server` lets siblings depend on the schema without pulling the full ruLake-tool surface.

A substrate, in this framing, satisfies the trait, produces witnesses through the canonical recipe, and rides the federation primitive without inventing a new one. v2's job is not to invent federation, audit, or a cache layer; it is to be a substrate that ruLake can host. ADR-008 §Compatibility section confirms every existing ruLake artefact (the trait, the bundle struct, the witness function, the GCS / IPFS backends, the `mcp-server` audit row, the Console crypto) is unchanged.

## Open questions

The ADR is honest about several genuine unknowns (`docs/research/ruqu/v2-spec.md:1763`-area). Calibration drift mid-run is the largest: a 30-second Heron-Q3 dispatch can span a calibration window, and the bundle's `runtime_class` records only the *initial* snapshot; v0.1 chose to record drift in `lineage_id` and use the initial snapshot for the witness, but v0.2 may revisit if real drift becomes operationally common. Hybrid algorithm witnesses (VQE / QAOA where a classical optimiser drives the loop) are likely to be punted to `ruqu-algorithms` rather than coupled into ruqu-core. Tier-2 amplitude-bytes federation (when do amplitude vectors, not just bundles, need to cross node boundaries?) is deferred until use cases demand it. The "is a cached run still scientific?" policy is settled for the cache vs. supremacy distinction but unclear on partial-shot caches. The `ruqu-cap-Q` crate at `vendor/ruvector/crates/ruQu/` (note the capital Q — different from `ruqu-core`) is flagged as out of scope; whether its real-time syndrome buffer benefits from witness anchoring is a separate compositional question worth its own future ADR.

## References

- ADR-008: `/home/ruvultra/projects/RuLake/docs/adrs/ADR-008-ruqu-as-rulake-substrate.md`
- v2 spec: `/home/ruvultra/projects/RuLake/docs/research/ruqu/v2-spec.md`
- Integration plan: `/home/ruvultra/projects/RuLake/docs/research/ruqu/integration-with-rulake.md`
- Corpus README: `/home/ruvultra/projects/RuLake/docs/research/ruqu/README.md`
- Reference implementation (v0.0.1): `/home/ruvultra/projects/RuLake/crates/ruqu-backend/`
- ruLake bundle: `/home/ruvultra/projects/RuLake/crates/core/src/bundle.rs:113` (`RuLakeBundle`), `/home/ruvultra/projects/RuLake/crates/core/src/bundle.rs:166` (`RuLakeBundle::new`), `/home/ruvultra/projects/RuLake/crates/core/src/bundle.rs:362` (`compute_witness`)
- Backend trait: `/home/ruvultra/projects/RuLake/crates/core/src/backend.rs:110`
- Federation primitive: `crates/core/src/lake.rs:521` (`search_federated`); cache stats: `crates/core/src/lake.rs:127` (`cache_stats_by_backend`)
- v1 ruqu-core source: `vendor/ruvector/crates/ruqu-core/`
  - `src/replay.rs::ExecutionRecord` (line 57), `circuit_canonical_bytes` (line 212)
  - `src/witness.rs::WitnessLog` (line 88), `append` (line 107), `verify_chain` (line 141)
  - `src/planner.rs::plan_execution` (line 213), `select_optimal_backend` (line 476)
  - `src/qec_scheduler.rs::generate_surface_code_schedule` (line 114)
  - `src/simd.rs::apply_single_qubit_gate_scalar` (line 37) — the SIMD gotcha reference
- v1 benchmark suite (cited for cost-model coefficients in v2-spec §g `bench`): `vendor/ruvector/crates/ruqu-core/benches/quantum_sim.rs`
- Browser composition target: `vendor/ruvector/crates/ruqu-wasm/` (in-tab StateVector at 25 qubits, 180 KB gzip)
- IPFS bundle distribution pattern reused for cross-site circuit sharing: `/home/ruvultra/projects/RuLake/docs/adrs/sdk/ADR-005-ipfs-backend-and-deploy.md`
- Console architecture the Quantum route plugs into: `/home/ruvultra/projects/RuLake/docs/adrs/ADR-006-rulake-console-vite-github-pages.md`
- Memory-class framing: `/home/ruvultra/projects/RuLake/docs/adrs/ADR-156-rulake-as-memory-substrate.md` (v2 emits `memory_class = "quantum-simulation"`)
- Simulation references — the v2 spec cites StateVector / Stabilizer / Clifford+T / TensorNetwork / Hardware as engine families inherited verbatim from v1; the v1 README at `vendor/ruvector/crates/ruqu-core/README.md` is the upstream record. The v1 ADR family (ADR-QE-001 through ADR-QE-015) is referenced from the README and lives upstream rather than in this repo.
