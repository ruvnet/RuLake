# RVF-as-BackendAdapter — upstream API gap

**Date:** 2026-04-25
**Status:** blocked on upstream `rvf-runtime` change
**Triggered by:** "for Parquet could we use rvf?" — proposing RVF as
the canonical cloud-backend format alongside (or instead of) Parquet.

## The case for RVF as a ruLake backend format

Strong, on the merits:

1. **Native RaBitQ codes.** RVF segments already carry the 1-bit
   compressed representation. A `RvfBackend::pull_*` path that yielded
   the compressed codes directly would skip the encode step on the
   cache prime path — the dominant cost per
   [`crates/rulake/BENCHMARK.md`](../../vendor/ruvector/crates/rulake/BENCHMARK.md)'s
   "cold-prime time" block.
2. **Native witness chain.** Every RVF file carries a `FileIdentity`
   (`vendor/ruvector/crates/rvf/rvf-types/src/lineage.rs:62`) with
   `file_id`, `parent_id`, `parent_hash`, `lineage_depth`. That's a
   *true* witness anchored on actual content; today's
   `gcs-backend` synthesizes one from `data_ref + dim + seed +
   rerank + generation` in `RuLakeBundle::new`.
3. **Append-only segment model** that aligns naturally with object
   storage — the tail manifest is the one thing that mutates,
   everything else is content-addressed.

## The blocker

`rvf-runtime`'s public surface today (`vendor/ruvector/crates/rvf/rvf-runtime/src/store.rs`)
exposes:

- `RvfStore::create / open / open_readonly` — load segments
- `RvfStore::ingest_batch` — write
- `RvfStore::query(vector, k, options) -> Vec<SearchResult>` — read
- `RvfStore::dimension() / file_identity() / segment_dir() / status()` — metadata
- `RvfStore::compact / delete / close` — lifecycle

What's **missing for the BackendAdapter contract** in
[`src/backend.rs:115`](../../src/backend.rs):

```rust
fn pull_vectors(&self, collection: &str) -> Result<PulledBatch>;
//  PulledBatch { ids: Vec<u64>, vectors: Vec<Vec<f32>>, dim, generation }
```

There's no public way to iterate every (id, vector) pair. The
internal `CowMap::read_vector(id, &File, parent: Option<&File>)`
exists at `cow.rs:108`, but it takes the open `&File` — not reachable
from outside `RvfStore` because the file handle is private.

`SearchResult` only carries `id + distance + retrieval_quality` — no
vector bytes. Even `query(zero_vector, k = u32::MAX)` doesn't get us
the vector content.

The lower layer (`read_path::read_vec_seg_payload(payload: &[u8]) ->
Option<Vec<(u64, Vec<f32>)>>` at line 289) is exactly what we need —
but it's `pub(crate)`. `read_path` itself is `pub mod read_path` but
every function in it is `pub(crate)` (see lines 67, 76, 289, 346).

## Three real paths

| Path | Scope | Tradeoff |
|---|---|---|
| **A.** Upstream PR to `rvf-runtime` adding `RvfStore::read_all_vectors() -> impl Iterator<Item = Result<(u64, Vec<f32>)>>` (or similar) | upstream PR + review | Right answer; unblocks cloud-RVF too. Multi-week, blocks on upstream. |
| **B.** Build our own RVF segment parser in `rvf-backend/` using `rvf-types` (the lower-level format crate) | ~500 LOC of duplicated parsing | Works today without forking `rvf-runtime` (which ADR-001 explicitly forbids). Brittle — drifts when RVF spec evolves. Two parsers for one format. |
| **C.** Extend ruLake's `BackendAdapter` trait with an optional `pull_prebuilt_index(collection) -> Option<Arc<RabitqPlusIndex>>` so the backend can skip the encode step entirely. RVF backend implements that path. | small ruLake change, plus upstream PR for the RVF side | Best long-term shape — captures the actual *value* of RVF (no re-encode), not just the format compatibility. Still blocked on the upstream iterator. |

## Recommendation

**Path A** — file the upstream issue / PR against `ruvnet/RuVector`'s
`rvf-runtime` for a public `read_all_vectors()` (or per-id
`read_vector`) method. Until it lands, ruLake's first cloud backend
remains `gcs-backend/` (Parquet on GCS, shipped today as commit
`c706dc6`).

When the upstream lands, the right shape is **Path A + Path C
together**: a public RVF iterator, plus an extended BackendAdapter
trait method that lets RVF backends hand the cache a pre-built RaBitQ
index. That's the only configuration that actually delivers the
prime-time speedup RVF promises.

For now: don't ship a half-implementation of `rvf-backend/`. The
gcs-backend covers the cross-ecosystem case (anyone with Parquet on
GCS) and the witness story still works (synthesized witness from the
`Generation::Num` of the GCS object). When upstream lands, RVF becomes
the higher-performance native option for ruLake-ecosystem deployments.

## Files touched during the spike (no commits)

- Looked at `vendor/ruvector/crates/rvf/rvf-runtime/src/store.rs`,
  `cow.rs`, `read_path.rs`, `options.rs`
- Looked at `vendor/ruvector/crates/rvf/rvf-types/src/lineage.rs`
  for `FileIdentity` shape
- No source files written (the scaffold was abandoned before any
  code landed).

## Follow-ups

1. Open issue on `ruvnet/RuVector` requesting a public RVF segment
   vector iterator (Path A).
2. Draft an ADR for the `BackendAdapter::pull_prebuilt_index` trait
   extension (Path C).
3. Once both land, scaffold `rvf-backend/` as a sibling crate
   following the `gcs-backend/` pattern.
