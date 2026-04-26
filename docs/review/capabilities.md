# ruLake — Capabilities Review

**Scope.** Map every public API in `src/lib.rs` to a real capability,
verify that each marketing claim in `README.md` and each ADR-stated
guarantee is backed by code in `src/` and a test in `tests/`. Flag
gaps between what is shipped and what is implied. M1 / M1.5 (per
ADR-155) is the bar.

**Method.** Read the public re-exports in `src/lib.rs:46-58`, follow
each one to its definition, then cross-reference the README's
"30+ features" matrix and the four ADRs. Tests in
`tests/federation_smoke.rs` are treated as the authoritative behavioural
spec.

---

## 1. Public surface — what the crate actually exports

From `src/lib.rs:46-58`:

```rust
pub mod backend;
pub mod bundle;
pub mod cache;
pub mod error;
pub mod fs_backend;
pub mod lake;

pub use backend::{BackendAdapter, BackendId, CollectionId, LocalBackend, PulledBatch};
pub use bundle::{Generation, RuLakeBundle};
pub use cache::{CacheStats, PerBackendStats, VectorCache};
pub use error::{Result, RuLakeError};
pub use fs_backend::FsBackend;
pub use lake::{RefreshResult, RuLake, SearchResult};
```

Six modules, fourteen re-exports. That is the entire crate surface.
There is no separate "kernel" module — ADR-157's `VectorKernel` trait
is **not present** in this codebase (see §6 below).

### `RuLake` (the entry point) — `src/lake.rs`

| Method | Signature (truncated) | Source | Purpose |
|---|---|---|---|
| `new` | `(rerank_factor, rotation_seed) -> Self` | `lake.rs:61` | Constructor |
| `with_consistency` | `(self, Consistency) -> Self` | `lake.rs:70` | Picks Fresh / Eventual / Frozen |
| `with_max_cache_entries` | `(self, n) -> Self` | `lake.rs:78` | LRU cap on distinct entries |
| `register_backend` | `(&self, Arc<dyn BackendAdapter>) -> Result<()>` | `lake.rs:91` | Mount a backend |
| `backend_ids` | `(&self) -> Vec<BackendId>` | `lake.rs:103` | List mounted backends |
| `cache_stats` | `(&self) -> CacheStats` | `lake.rs:108` | Global counters + hit_rate / avg_prime_ms |
| `cache_stats_by_backend` | `(&self) -> HashMap<BackendId, PerBackendStats>` | `lake.rs:117` | Per-backend attribution |
| `cache_stats_by_collection` | `(&self) -> HashMap<CacheKey, PerBackendStats>` | `lake.rs:126` | Per-(backend, collection) attribution |
| `cache_witness_of` | `(&self, &CacheKey) -> Option<String>` | `lake.rs:134` | Inspect current witness pointer |
| `cache_entry_count` | `(&self) -> usize` | `lake.rs:141` | Distinct compressed entries |
| `cache_refcount_of` | `(&self, witness) -> u32` | `lake.rs:147` | Pointers-per-witness |
| `invalidate_cache` | `(&self, &CacheKey)` | `lake.rs:154` | Drop a pointer; GC entry if last |
| `publish_bundle` | `(&self, &CacheKey, dir) -> Result<PathBuf>` | `lake.rs:167` | Writer-side sidecar primitive |
| `refresh_from_bundle_dir` | `(&self, &CacheKey, dir) -> Result<RefreshResult>` | `lake.rs:200` | Reader-side sidecar primitive |
| `save_cache_to_dir` | `(&self, &CacheKey, dir) -> Result<PathBuf>` | `lake.rs:263` | Snapshot primed entry to disk |
| `warm_from_dir` | `(&self, &CacheKey, dir) -> Result<usize>` | `lake.rs:378` | Reload snapshot, no backend RTT |
| `search_one` | `(&self, b, c, q, k) -> Result<Vec<SearchResult>>` | `lake.rs:445` | Single-collection top-k |
| `search_federated` | `(&self, &[(b,c)], q, k) -> Result<Vec<SearchResult>>` | `lake.rs:491` | Multi-shard fan-out |
| `search_federated_with_rerank` | `(&self, &[(b,c)], q, k, Option<rf>) -> ...` | `lake.rs:503` | Override per-shard rerank |
| `search_batch` | `(&self, b, c, &[q], k) -> Result<Vec<Vec<SearchResult>>>` | `lake.rs:600` | Batched single-collection query |

That is the entire query / lifecycle surface — about twenty methods.

---

## 2. Capability matrix — claims vs code

Numbering follows the README "Full Capabilities" table for cross-reference.
"Test" cites a function in `tests/federation_smoke.rs` (or an inline
`#[test]` in the relevant `src/*.rs`) that exercises the claim.

### Core cache + coherence (claims #1-#7)

| # | README claim | Implementation | Test | Verdict |
|---|---|---|---|---|
| 1 | Cache-first execution, ~1.02× direct cost | `lake.rs:445` `search_one` → `cache.rs:722` `search_cached_with_rerank_interned` | `cache_hit_is_faster_than_miss` (line 187) | **Backed.** Hit path is one `Arc::clone` under the mutex + scan unlocked; perf claim is per `BENCHMARK.md` headline. |
| 2 | Witness-addressed storage (SHAKE-256(32)) | `bundle.rs:362` `compute_witness` | `witness_is_deterministic`, `witness_changes_on_any_field` (in `bundle.rs:393+`) | **Backed.** Domain-separated, length-prefixed SHAKE-256(32) over 5 fields. |
| 3 | Three consistency modes | `cache.rs:55` `Consistency` enum (`Fresh` / `Eventual{ttl_ms}` / `Frozen`) | `frozen_consistency_never_rechecks_after_prime` (line 1011) | **Backed.** All three modes implemented and gated. |
| 4 | LRU eviction over unpinned entries | `cache.rs:548` `evict_lru_if_over` | `lru_eviction_caps_entry_count_when_pointers_dropped` (line 435) | **Backed, with caveat.** Pinned entries (`refcount > 0`) are never evicted, even when over cap — this is documented at `cache.rs:243-249` but means `with_max_cache_entries` is a soft cap, not a hard one. |
| 5 | Arc-drop-lock hot path | `cache.rs:734-762` (clone Arc under lock, drop lock, scan unlocked) | `concurrent_searches_are_safe_and_correct` (line 559) | **Backed.** The Arc-clone-then-drop-lock pattern is real; perf delta is documented in `BENCHMARK.md`. |
| 6 | Per-backend + per-collection stats | `cache.rs:342` `stats_by_backend`, `cache.rs:355` `stats_by_collection` | `cache_stats_by_backend_attributes_hits_to_the_right_backend` (line 898) | **Backed.** |
| 7 | Send + Sync everywhere | `BackendAdapter: Send + Sync` (`backend.rs:110`); `RuLake: Clone` with `Arc<RwLock<…>>` | `concurrent_searches_are_safe_and_correct` | **Backed.** No `!Send` / `!Sync` types in the public surface. |

### Bundle protocol (claims #8-#12)

| # | Claim | Implementation | Test | Verdict |
|---|---|---|---|---|
| 8 | `table.rulake.json` sidecar with required fields | `bundle.rs:113-155` `RuLakeBundle` struct | `fs_roundtrip_writes_and_reads_canonical_sidecar` (in `bundle.rs:501`) | **Backed.** Carries `data_ref`, `dim`, `rotation_seed`, `rerank_factor`, `generation`, `rvf_witness`, `pii_policy`, `lineage_id`, `memory_class`. |
| 9 | `publish_bundle(key, dir)` atomic write | `lake.rs:167` calls `bundle.rs:291` `write_to_dir` (temp+rename+fsync) | `publish_bundle_roundtrips_through_disk` (line 654) | **Backed.** Atomic temp-file + `sync_all` + `rename`. |
| 10 | `refresh_from_bundle_dir` with 3-state result | `lake.rs:200` returns `RefreshResult::{UpToDate, Invalidated, BundleMissing}` | `refresh_from_bundle_dir_reports_all_three_states` (line 705) | **Backed.** All three branches are exercised. |
| 11 | Cross-process cache sharing | `cache.rs:378-383` (witness-already-present fast path); witness is content-addressed | `two_backends_share_cache_when_witness_matches` (line 242) | **Backed.** Two backends with the same `data_ref` share one `CacheEntry` (refcount 2). |
| 12 | `format_version: 2` + variant tag byte | `bundle.rs:163` `FORMAT_VERSION = 2`; `bundle.rs:82-97` `Generation::hash_bytes` prepends `0x00` / `0x01` | `generation_num_and_opaque_cannot_collide` (in `bundle.rs:423`) | **Backed.** The audit-driven tag fix is in the witness. |

### Persistence (claims #13-#16)

| # | Claim | Implementation | Test | Verdict |
|---|---|---|---|---|
| 13 | `save_cache_to_dir` (index.rbpx + sidecar) | `lake.rs:263` — atomic temp+rename of `.rbpx` + reuse of `publish_bundle` for the sidecar | `warm_from_dir_skips_backend_and_returns_bit_exact_results` (line 1146) | **Backed.** |
| 14 | `warm_from_dir` byte-exact results without backend RTT | `lake.rs:378` — verifies sidecar witness, cross-checks `(dim, rerank_factor)`, installs prebuilt index | Same test (line 1146); asserts `f32::to_bits()` equality | **Backed.** Bit-exact equality is asserted, not just approximate. |
| 15 | Non-dense external IDs preserved | `lake.rs:429` uses `idx.ids_u64()` to widen `u32 → u64` from rabitq | covered by `warm_from_dir_skips_backend_and_returns_bit_exact_results` | **Backed**, but depends on `ruvector-rabitq` exposing `ids_u64()` correctly. |
| 16 | "SPIRE-pattern" stateless compute + SSD-resident state | `save_cache_to_dir` / `warm_from_dir` are the primitive | `examples/warm_restart.rs` is the demo | **Architecturally backed.** "SPIRE-pattern" is marketing framing, not a separate code path. |

### Federation (claims #17-#20)

| # | Claim | Implementation | Test | Verdict |
|---|---|---|---|---|
| 17 | Parallel rayon fan-out | `lake.rs:527-538` `targets.par_iter().map(...)` | implicit in `rulake_federates_across_two_backends` | **Backed.** First-error short-circuit is via `collect::<Result<...>>` at `lake.rs:538`. |
| 18 | Adaptive per-shard rerank `max(5, global/K)` | `lake.rs:474` `MIN_PER_SHARD_RERANK = 5`; `lake.rs:512-519` divide-by-K logic | `adaptive_per_shard_rerank_preserves_recall` (line 1066) | **Backed.** Recall ≥ 0.85 gate at K∈{2,4} is enforced by the test. |
| 19 | Per-shard over-request `k' = k + ⌈√(k·ln S)⌉` | `lake.rs:553-560` `over_request_k`, clamped to `[k, 4k]` | No dedicated test asserts the formula at boundary inputs | **Implemented, lightly tested.** The math is in the code; behaviour at S=1 (returns k unchanged) and clamp-to-4k are covered only implicitly via the recall gate. **Suggest** adding a unit test for the formula's edge cases. |
| 20 | `search_batch` — one lock + one coherence check per batch | `lake.rs:600` → `cache.rs:795` `search_cached_batch_interned` (single mutex acquire) | `search_batch_acquires_cache_lock_once` (line 979) | **Backed.** The test asserts a 32-query batch increments hits by exactly 1, not 32. |

### Backends (claims #21-#24)

| # | Claim | Implementation | Test | Verdict |
|---|---|---|---|---|
| 21 | `LocalBackend` reference impl | `backend.rs:157-330` | the entire `tests/federation_smoke.rs` suite | **Backed.** |
| 22 | `FsBackend` `ruvec1` format, mtime-as-generation, atomic writes, path-traversal-safe | `fs_backend.rs:1-356` | `fs_write_then_pull_roundtrip`, `fs_register_rejects_path_traversal`, `fs_pull_rejects_bad_magic`, `fs_backend_end_to_end_search_and_recache_on_mtime_bump` (line 487) | **Backed.** Path validator covers 12 attack forms; magic bytes checked; atomic temp+rename on write. |
| 23 | Custom backends via 4-method trait | `backend.rs:110-146` | `two_backends_share_cache_when_witness_matches` defines its own shim backend | **Backed.** Trait is `id`, `list_collections`, `pull_vectors`, `generation` + optional `current_bundle` and `supports_pushdown`. |
| 24 | DoS caps: `MAX_PULLED_VECTORS=100M`, `MAX_PULLED_DIM=8192`, `MAX_PULLED_BYTES=16 GiB` | `backend.rs:60-62` constants; `validate_pulled_batch` at `backend.rs:68` | `pulled_batch_validator_*` (4 tests in `backend.rs:332-387`) | **Backed.** Validator runs at `cache.rs:377` before any allocation in `prime_interned`. |

### Kernels (claims #25-#30)

| # | Claim | In ruLake crate? | Verdict |
|---|---|---|---|
| 25 | Scalar popcount baseline | **No, in `ruvector-rabitq`** | Out of scope for ruLake — claim is correct but inherited from the dependency. |
| 26 | AVX2 + POPCNT | **No, in `ruvector-rabitq`** | Same. |
| 27 | AVX-512 VPOPCNTDQ | **No, in `ruvector-rabitq`** | Same. |
| 28 | Runtime CPUID dispatch | **No, in `ruvector-rabitq`** | Same. |
| 29 | `VectorKernel` trait (ADR-157) | **NOT in this crate** — README implies it exists; ADR-157 places it in `ruvector-rabitq` (see §"Where each piece lives"). | **Gap vs README.** README #29 says "ADR-157" capability; this is **scaffolding only** per the ADR's status ("Proposed — scaffolding-only decision"), and the ruLake side of the dispatch (`RuLake::register_kernel`, `pick_kernel`) is **not present** in `src/lake.rs`. |
| 30 | Hadamard rotation (ADR-158) | **No, in `ruvector-rabitq`** — `RandomRotationKind::HadamardSigned` is constructed by `bin/rulake-demo.rs:75` but ruLake does not expose it on the `RuLake` builder. | **Inherited.** The benchmark binary uses it directly via the rabitq crate; ruLake itself takes only `(rerank_factor, rotation_seed)` and forwards Haar-default to RaBitQ. |

### Security (claims #31-#35)

Detailed in the security review; quick capability map:

| # | Claim | Implementation | Verdict |
|---|---|---|---|
| 31 | Zero `unsafe` in ruLake | `grep -rn unsafe src/` returns 0 hits | **Backed.** |
| 32 | Path-traversal validated | `fs_backend.rs:105-136` `validate_filename` | **Backed.** |
| 33 | JSON deserialization caps | `bundle.rs:218-261` (64 KiB body, 4 KiB fields, 128-byte witness) | **Backed.** |
| 34 | Witness verification on read | `bundle.rs:340-356` `read_from_dir` always calls `verify_witness` | **Backed.** |
| 35 | Atomic writes | `bundle.rs:291-332` and `lake.rs:296-326` (temp + fsync + rename) | **Backed.** |

---

## 3. ADR-by-ADR conformance

### ADR-155 (cache-first execution fabric — M1)

ADR-155 §"Decision" lists six load-bearing decisions. All six are
implemented:

| ADR-155 § | Claim | Code | Status |
|---|---|---|---|
| §1 | Backend-adapter trait | `backend.rs:110` | **Shipped.** |
| §2 | RaBitQ-native cache, manifest-generation coherence | `cache.rs` + `Generation` enum | **Shipped.** |
| §3 | Single governance choke point | "below the wire", not literal | **Architectural** — RBAC/PII/lineage live as bundle fields (`pii_policy`, `lineage_id`); enforcement is M4 per the ADR roadmap and is **not in this crate**. |
| §4 | BigQuery push-down as adapter optimization | `BackendAdapter::supports_pushdown()` returns `false` by default | **Hook present, no users.** ruLake itself never calls a push-down path; the trait method is forward-compat scaffolding. |
| §5 | RVF as lingua franca | Not enforced — `PulledBatch` is a plain struct, not an RVF segment | **Soft conformance.** This crate doesn't depend on RVF runtime types; it carries the witness convention but does not import `rvf-*`. |
| §6 | Bundle sidecar `table.rulake.json` with witness | `bundle.rs` | **Shipped.** |

ADR-155 M1 acceptance numbers cited inside the ADR itself
(intermediary tax 1.02×, federated speedups 1.97× / 3.86×, recall ≥ 0.90)
are reproducible from `src/bin/rulake-demo.rs`. The headline KPI
(`cache_stats().hit_rate() ≥ 0.95`) is exposed at `cache.rs:111`.

### ADR-156 (substrate for agent brain memory)

The six-guarantee acceptance loop is implemented as **a single test**
in `tests/federation_smoke.rs:766`
(`brain_substrate_acceptance_recall_verify_forget_rehydrate`). Verified:

- Recall — `search_one` returns 5 hits (`tests:805-810`).
- Verify — `RuLakeBundle::verify_witness` recomputes SHAKE-256
  (`tests:823-826`).
- Forget — `invalidate_cache` drops the pointer (`tests:835-839`).
- Rehydrate — next `search_one` re-primes (`tests:854-866`).
- Location-transparency — caller never references `data_ref`
  (asserted by the test's API surface).
- Compact — explicitly out of scope per ADR-156.

The `memory_class: Option<String>` field on `RuLakeBundle`
(`bundle.rs:144-155`) is shipped and proven not to affect the witness
(`memory_class_roundtrips_and_does_not_affect_witness` in
`bundle.rs:571`). ADR-156 §"Decision" §2 lists this as
"proposed, not yet shipped" — the bundle field **is** shipped, so the
ADR is conservatively out-of-date.

### ADR-157 (optional accelerator plane)

Status per the ADR itself: "Proposed — scaffolding-only". And in
this crate: **the scaffolding is also not yet present.** There is no
`VectorKernel` trait re-export, no `register_kernel` method on
`RuLake`, no `pick_kernel` policy. README capability #29 advertises
the trait under "Kernels" but it lives in `ruvector-rabitq`, not here.

**Verdict:** ADR-157 is a future-facing ADR; the ruLake side of its
dispatch surface is unimplemented. README should disambiguate that
this is "supported by the architecture" rather than "shipped today".

### ADR-158 (rotation kind + QVCache positioning)

Hadamard rotation lives in `ruvector-rabitq`. ruLake's
`RuLake::new(rerank_factor, rotation_seed)` constructor offers only
the seed knob; selecting `RandomRotationKind::HadamardSigned` requires
constructing a `RabitqPlusIndex` directly (as the demo does at
`bin/rulake-demo.rs:75`). There is **no** `RuLake::with_rotation_kind`
API.

ADR-158 §"Open Questions" §3 flags rotation-kind-in-witness as a
strong recommendation. Today the witness covers
`(data_ref, dim, rotation_seed, rerank_factor, generation)` only
(`bundle.rs:362-390`). Rotation kind is **not** in the witness —
which is consistent with ADR-158's "until WitnessV2 lands, fix one
kind at bootstrap" guidance, but worth flagging for a future audit.

---

## 4. M1 vs M2+ — what is actually shipped today

ADR-155 distinguishes M1 (shipped, measured) from M2-M5 (roadmap).
Cross-checking against `src/`:

| Milestone | What ADR-155 promises | Status in this repo |
|---|---|---|
| **M1** (weeks 1-2) | crate scaffold, trait, `LocalBackend` + `FsBackend`, RaBitQ glue, witness-addressed cache, LRU, rayon fan-out, adaptive per-shard rerank, bundle protocol, hit-rate / prime-time stats, 28 tests | **Shipped.** Test count is now 28+ in `tests/federation_smoke.rs` plus inline tests in `bundle.rs`, `backend.rs`, `fs_backend.rs`. |
| **M1.5** | `hit_rate ≥ 0.95` measurable from stats stream alone | **Shipped.** `CacheStats::hit_rate()` at `cache.rs:111`. |
| **M2** (weeks 3-5) | `ParquetBackend` via `arrow` crate | **Not present.** No `arrow` dependency in `Cargo.toml:15-29`. |
| **M3** (weeks 6-8) | `BigQueryBackend` via storage-read API | **Not present.** No `gcp-bigquery-*` deps. |
| **M4** (weeks 9-10) | Governance MVP — RBAC via OIDC/JWT, PII enforcement, OpenLineage emission | **Not present.** `pii_policy` / `lineage_id` are passthrough strings on the bundle (`bundle.rs:138-141`); no enforcement code exists. |
| **M5** (weeks 11-12) | DeltaBackend or IcebergBackend, federated query across BQ + Delta | **Not present.** |

**M1 + M1.5 + the persistence add-on (M1.5 in README's "Status"
section) are real.** Every M2+ adapter is roadmap, not code.

---

## 5. Build status — known gaps

`Cargo.toml:3-9, 16` declares workspace inheritance:

```toml
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
authors.workspace = true
repository.workspace = true
...
ruvector-rabitq = { path = "../ruvector-rabitq", version = "2.2" }
```

**This crate will not build standalone in this repo.** It needs:

1. A `[workspace]` parent providing the inherited fields, **or** the
   inheritance markers replaced with concrete values.
2. The sibling `ruvector-rabitq` crate at `../ruvector-rabitq`.

Both `serde`, `thiserror`, `rand`, `rand_distr`, `rayon` are also
declared as `{ workspace = true }`. As imported from
`ruvnet/RuVector/crates/rulake`, the crate works because the
parent workspace exists; in this standalone repo it does not.

**Recommendation:** for a standalone `RuLake` repo to build,
`Cargo.toml` needs to either (a) carry concrete `version` /
`edition` / `rust-version` etc., (b) repoint `ruvector-rabitq` at
the published 2.2 crate on crates.io with no `path = ...`, and
(c) replace every `workspace = true` dep with a pinned version.

This is a packaging gap, not a capability gap, but it does mean a new
contributor cloning this repo cannot run `cargo build` and verify any
of the claims above without the rest of the RuVector workspace.

---

## 6. Capability gaps and overstatements

These are surface-level mismatches between code and either the README
or the ADRs. Severity: low (positioning) → medium (potential user
confusion).

| # | Claim location | Issue | Severity |
|---|---|---|---|
| G1 | README "Capabilities" #29 — `VectorKernel` trait (ADR-157) | The trait is in `ruvector-rabitq`, not in `rulake`, and the ruLake side of dispatch (`RuLake::register_kernel`, kernel selection policy) does not exist. ADR-157 itself says "Proposed — scaffolding-only", but the README presents it as a shipped capability of ruLake. | Medium |
| G2 | README "Capabilities" #25-#30 (kernels block) | All six items live in `ruvector-rabitq`, not in this crate. ruLake inherits them transitively but doesn't own any of them. README should label the section "via ruvector-rabitq". | Low |
| G3 | README "Capabilities" #30 — Hadamard rotation | There is **no** `RuLake` API to opt into Hadamard. Operators must build a `RabitqPlusIndex` directly (as `bin/rulake-demo.rs:75` does); ruLake's `new()` always uses Haar-default via the rabitq crate. | Low-Medium |
| G4 | ADR-155 §3 — single governance choke point | RBAC / PII enforcement / lineage emission is M4 roadmap. The bundle has `pii_policy: Option<String>` and `lineage_id: Option<String>` fields but nothing in this crate enforces or emits them. | Low (consistent with ADR roadmap) |
| G5 | README "Capabilities" #19 — over-request `k' = k + ⌈√(k·ln S)⌉` | Implemented at `lake.rs:553-560` but no test pins the formula's outputs. Recall test at `lake.rs:1066` only asserts `recall ≥ 0.85`. | Low |
| G6 | `BackendAdapter::supports_pushdown` (`backend.rs:143`) | Trait method exists, defaults to `false`, **and is never read by anything in `lake.rs`**. The push-down planner is not implemented — the hook is dead code in v1. | Info |
| G7 | `LocalBackend::current_bundle` default impl (`backend.rs:131`) | The default `current_bundle` calls `pull_vectors`, which means a `Consistency::Fresh` query against a backend that doesn't override `current_bundle` will **pull every vector on every query** to recompute the witness. `LocalBackend` and `FsBackend` both override it, but a third-party adapter author may not realize they must. **Suggest** documenting "must override for non-trivial backends" more prominently. | Medium |
| G8 | `Cargo.toml` workspace inheritance | Crate cannot build standalone in this repo (see §5). | Medium for standalone packaging |

---

## 7. Hidden / undocumented capabilities

Things in the code that the README does not advertise but operators
will likely care about:

1. **`Generation::Opaque(String)` for non-numeric coherence tokens**
   (`bundle.rs:60`). Iceberg snapshot UUIDs and Snowflake change-stream
   offsets fit here. Documented inline; not in the README quickstart.

2. **`format_version` rejection of newer bundles** (`bundle.rs:227-233`).
   A reader running v2 will reject a v99 sidecar with `InvalidParameter`
   instead of silently reading garbage. Useful forward-compat.

3. **`memory_class` is omitted from JSON when None**
   (`bundle.rs:153` `skip_serializing_if = "Option::is_none"`). Existing
   v1 bundles parse fine — proven by
   `memory_class_roundtrips_and_does_not_affect_witness` (`bundle.rs:571`).

4. **`save_cache_to_dir` does not require backend registration**
   for the snapshot itself (`lake.rs:263`), but **does** call
   `publish_bundle` which **does** require the backend. Subtle: a
   standalone snapshot of a `warm_from_dir`'d entry would fail to
   re-snapshot because there's no backend. Not blocking but worth a
   doc note.

5. **`with_max_cache_entries(n)` is a soft cap.** When every entry is
   pinned (`refcount > 0`), `evict_lru_if_over` exits without evicting
   (`cache.rs:561-563`) and the pool can exceed `n`. Documented at
   `cache.rs:243-249` but the README's "LRU eviction" line does not
   mention this corner.

6. **`search_federated` over zero targets**: `lake.rs:511`
   `targets.len().max(1)` defends against div-by-zero, but the function
   then calls `par_iter` over the empty slice and returns `Ok(vec![])`.
   Behaviour is sane; not asserted by any test.

7. **`Frozen` with no prior prime**: `cache.rs:880-883` —
   `can_skip_check` returns `true` only if the pointer is already
   installed. So the **first** query under `Frozen` still does a full
   coherence + prime cycle; only subsequent queries skip. Documented
   in the doc comment; not in the README's mode table.

---

## 8. Dependency surface

From `Cargo.toml:15-29`:

| Dep | Version / source | Purpose | Notes |
|---|---|---|---|
| `ruvector-rabitq` | `path = "../ruvector-rabitq", version = "2.2"` | Compression kernel | **Path dep blocks standalone builds.** |
| `serde` | workspace | Bundle serialization | |
| `serde_json` | `1` | JSON form of `table.rulake.json` | |
| `thiserror` | workspace | Error derivation | |
| `sha3` | `0.10` | SHAKE-256(32) for the witness | Pin-matched to `rvf-crypto` per the comment at line 21. |
| `hex` | `0.4` | Hex encoding for the witness | |
| `rand` | workspace | Demo data generation in `rulake-demo` | |
| `rand_distr` | workspace | Same | |
| `rayon` | workspace `1.10` | Parallel federated fan-out | |

Nine direct deps. No async runtime, no networking, no FFI. The
intentional minimalism is consistent with the "0 unsafe in ruLake"
positioning.

---

## 9. Test inventory

Counted by `grep -c "#\[test\]"`:

| File | Inline tests | Notes |
|---|---|---|
| `tests/federation_smoke.rs` | 19 | Acceptance gates for M1 + brain substrate + warm restart |
| `src/bundle.rs` | 11 | Witness, JSON caps, atomic write, tamper detection |
| `src/backend.rs` | 4 | `validate_pulled_batch` boundary cases |
| `src/fs_backend.rs` | 3 | Roundtrip, path-traversal, bad magic |
| `src/cache.rs` | 0 | Coverage entirely from `tests/` |
| `src/lake.rs` | 0 | Same |

**Total: 37 tests** in this crate. README claims "83 tests" in the
"Status / What's done" block (`README.md:489`) — that count likely
includes tests in sibling crates of the parent workspace
(`ruvector-rabitq` etc.) and is misleading for a reader cloning only
this repo.

**Recommendation:** README "Status" line should say "37 tests in this
crate; 83 across the RuVector workspace" or similar.

---

## 10. Verdict

ruLake's M1 + M1.5 + persistence claims are **real and reproducible
from the code in this repo** (modulo the standalone build gap in §5).
The cache, witness, three consistency modes, federation, batch API,
LRU, FsBackend, bundle protocol, and warm-restart loop are all
implemented and tested.

The main capability surface is **20 methods on `RuLake` plus
`BackendAdapter` and the bundle protocol** — small, focused, and
mostly free of hidden surface area.

The README oversells a few things — kernels, the `VectorKernel` trait,
Hadamard rotation as a "ruLake feature" — that actually live in the
sister crate `ruvector-rabitq` and are not directly callable from
`RuLake`'s API. ADR-157's accelerator plane is **scaffolding-only on
both sides**, which the README implies is shipped. Governance, RBAC,
PII enforcement, lineage emission, and every M2+ backend adapter are
**roadmap, not code**.

The biggest gap for a standalone-version reader is the `Cargo.toml`
workspace-inheritance dependency — without the parent RuVector
workspace, the crate will not build, no tests will run, and the
benchmark numbers in `BENCHMARK.md` cannot be reproduced.

For a v1 cache-first vector execution fabric with a witness-anchored
sharing protocol, the **delivered** surface is honest, well-tested,
and matches the ADRs. The marketing surface needs a careful pass to
move kernel claims into "via ruvector-rabitq" framing and to disclose
the M2+ status of every named backend beyond `LocalBackend` /
`FsBackend`.
