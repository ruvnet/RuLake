//! ruLake GCS backend — Parquet-on-GCS `BackendAdapter` (ADR-155 §M2).
//!
//! # What v0.1 ships
//!
//! - `GcsParquetBackend` — implements `BackendAdapter` against
//!   Parquet files on GCS (`gs://bucket/path/to/file.parquet`).
//! - **Generation = GCS object generation number.** Every
//!   `gsutil cp` / `gcloud storage cp` of a Parquet file bumps the
//!   generation; ruLake's witness picks up the change automatically
//!   via the existing `Generation::Num` variant in
//!   `src/bundle.rs:55`.
//! - **Cheap `current_bundle()` override** — the default impl in
//!   `src/backend.rs:131` does a full `pull_vectors` to learn the
//!   `dim`. That's a network read of the entire Parquet file just to
//!   compute the witness. We override to do a HEAD on the GCS object
//!   (~1 RTT, no body) plus a Parquet-footer-only read (a few KiB)
//!   to learn the schema, then synthesize the bundle. ADR-004's
//!   `rulake://bundle/{backend}/{collection}` resource explicitly
//!   forbids the default-impl behaviour.
//!
//! # Parquet schema contract (v0.1)
//!
//! Two required columns, with operator-overridable names:
//!
//! - `id` — non-null `INT64` (interpreted as `u64` — bit pattern
//!   preserved so signed input round-trips through `as u64`).
//! - `vector` — non-null `LIST<FLOAT>` (a list of `f32`s, fixed
//!   length per row; the length is the collection's `dim`).
//!
//! ```sql
//! -- BigQuery / DuckDB friendly DDL:
//! id      INT64    NOT NULL,
//! vector  ARRAY<FLOAT64>   -- cast down to FLOAT (FLOAT32) at write time
//! ```
//!
//! v0.2 will add: column-name overrides via config, `LIST<DOUBLE>`
//! support with f64→f32 down-cast, partitioned-table fan-out.
//!
//! # Auth
//!
//! Application Default Credentials, via `gcloud auth application-default
//! login` or service-account JSON in `GOOGLE_APPLICATION_CREDENTIALS`.
//! The `object_store` crate handles ADC resolution; we don't ship our
//! own auth code.

pub mod backend;

pub use backend::{GcsParquetBackend, GcsParquetCollection};
