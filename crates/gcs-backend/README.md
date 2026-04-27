# `ruvector-rulake-gcs` — Parquet-on-GCS BackendAdapter

Implements the first cloud backend per [ADR-155 §M2](../docs/adrs/ADR-155-rulake-datalake-layer.md):
read vectors from Parquet files on Google Cloud Storage, with cache
coherence riding GCS's per-object generation token.

Sibling Cargo package per [ADR-001](../docs/adrs/ADR-001-standalone-repo-strategy.md).

## What v0.1 ships

- `GcsParquetBackend` — `BackendAdapter` trait impl over GCS+Parquet.
- **Cache coherence via GCS object generation** — every reupload bumps
  the generation; ruLake's witness picks up the change automatically
  through the existing `Generation::Num` variant.
- **Cheap `current_bundle()` override** — does a HEAD on the GCS object
  + a Parquet-footer-only schema read instead of the default impl's
  full `pull_vectors`. ADR-004's `rulake://bundle/...` resource
  contract requires this for any backend that backs the resource.
- **Application Default Credentials** — auth is whatever
  `gcloud auth application-default login` (or
  `GOOGLE_APPLICATION_CREDENTIALS=...`) sets up. No bespoke auth code.

## Parquet schema contract (v0.1)

Two required columns:

| Column   | Type                                    | Notes |
|----------|-----------------------------------------|-------|
| `id`     | `INT64` (non-null)                      | Cast to `u64` by bit pattern. |
| `vector` | `LIST<FLOAT32>` or `FixedSizeList<FLOAT32, N>` (non-null) | One row per vector; the list length is the collection's `dim`. |

```sql
-- BigQuery / DuckDB friendly DDL:
CREATE TABLE docs (
  id      INT64    NOT NULL,
  vector  ARRAY<FLOAT64>     -- cast down to FLOAT32 on write
);
```

v0.2 will add: column-name overrides via config, `LIST<DOUBLE>` with
f64→f32 down-cast, partitioned-table fan-out, multi-file collections.

## Usage

```rust
use std::sync::Arc;
use rulake::{cache::Consistency, RuLake, BackendAdapter};
use ruvector_rulake_gcs::{GcsParquetBackend, GcsParquetCollection};

let backend = GcsParquetBackend::open_gcs("gcs-prod", "my-vector-bucket")?;
backend.register(GcsParquetCollection {
    name:    "docs".into(),
    object:  "embeddings/2026-04/docs.parquet".into(),
    dim:     None,    // None → read from Parquet schema on first pull
});

let lake = RuLake::new(20, 42)
    .with_consistency(Consistency::Eventual { ttl_ms: 5_000 });
let dyn_be: Arc<dyn BackendAdapter> = Arc::new(backend);
lake.register_backend(dyn_be)?;

// First call: HEAD + footer + body fetch + RaBitQ prime. ~hundreds of ms.
// Subsequent calls within ttl_ms: cache hit, ~1 ms.
let hits = lake.search_one("gcs-prod", "docs", &query, 10)?;
```

## Build + test

```bash
git clone --recurse-submodules https://github.com/ruvnet/RuLake
cd RuLake/gcs-backend
cargo build --release
cargo test --release          # 4 offline tests against in-memory ObjectStore

# Live test against a real bucket (needs gcloud ADC):
gcloud auth application-default login   # one-time
RULAKE_GCS_LIVE_TEST=1 \
RULAKE_GCS_BUCKET=your-bucket \
RULAKE_GCS_OBJECT=fixtures/docs-100k.parquet \
cargo test --release -- --ignored gcs_live
```

## Acceptance (ADR-155 §M2)

> ingest a 100k-row Parquet file; query latency ≤ 2× the equivalent
> RabitqPlusIndex standalone on the cache-hit path

The v0.1 implementation is wired for this gate; the bench against a
real 100k Parquet file is the v0.1 acceptance milestone (operator-run
since it needs a real GCS bucket + warm-cache measurement).

## What's not (yet) here

Tracked for v0.2+:

- Column-name overrides via `GcsParquetCollection { id_col, vector_col, ... }`.
- `LIST<FLOAT64>` (BigQuery's default) with down-cast to `f32`.
- Partitioned-table fan-out: one collection backed by N Parquet
  objects, planner-level parallel pull.
- `BackendAdapter::supports_pushdown() → true` once BigQuery's
  Vector Search lands as a sibling adapter.

## License

MIT OR Apache-2.0, matching the parent crate.
