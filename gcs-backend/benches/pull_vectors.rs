//! `pull_vectors()` micro-bench for `GcsParquetBackend`.
//!
//! Targets the load-bearing read path: HEAD on the (in-memory)
//! object store, ParquetRecordBatchStream open, full row decode into
//! `(Vec<u64>, Vec<Vec<f32>>)`. The GCS *network* fetch is
//! deliberately excluded — we use `object_store::memory::InMemory`
//! so the bench measures the Parquet decode + arrow→PulledBatch
//! conversion that runs once per (cold) cache prime.
//!
//! What's measured (per ADR-155 §M2 the hot path is cache, not pull):
//!   - footer read + schema resolve
//!   - one ParquetRecordBatchStream pass
//!   - `extract_vector_row` for every row
//!   - dim-vs-MAX_PULLED_DIM check + cache write
//!
//! What's excluded:
//!   - GCS HTTPS round-trip + ADC auth (would need a live bucket)
//!   - the writer cost (we build the Parquet body once outside the
//!     measured loop)

use std::sync::Arc;

use arrow::array::{ArrayRef, Float32Array, Int64Array, ListArray};
use arrow::buffer::OffsetBuffer;
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use bytes::Bytes;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use object_store::{memory::InMemory, path::Path as OsPath, ObjectStore, ObjectStoreExt};
use parquet::arrow::ArrowWriter;
use parquet::file::properties::WriterProperties;

use rulake::backend::BackendAdapter;
use ruvector_rulake_gcs::{GcsParquetBackend, GcsParquetCollection};

/// Build a synthetic Parquet body with `n` rows of `dim`-wide
/// `LIST<FLOAT32>` vectors. Mirrors the schema used in
/// `tests/smoke.rs::build_parquet`.
fn build_parquet(n: usize, dim: usize) -> Bytes {
    let ids: Vec<i64> = (0..n as i64).collect();
    let id_arr = Arc::new(Int64Array::from(ids)) as ArrayRef;

    let total: Vec<f32> = (0..(n * dim))
        .map(|i| ((i as f32) * 0.001).sin())
        .collect();
    let values = Float32Array::from(total);
    let offsets: Vec<i32> = (0..=n).map(|i| (i * dim) as i32).collect();
    let offset_buffer = OffsetBuffer::new(offsets.into());
    let field = Arc::new(Field::new("item", DataType::Float32, false));
    let list = ListArray::try_new(field.clone(), offset_buffer, Arc::new(values), None).unwrap();
    let vec_arr = Arc::new(list) as ArrayRef;

    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("vector", DataType::List(field), false),
    ]));
    let batch = RecordBatch::try_new(schema.clone(), vec![id_arr, vec_arr]).unwrap();

    let mut buf: Vec<u8> = Vec::new();
    let mut writer =
        ArrowWriter::try_new(&mut buf, schema, Some(WriterProperties::builder().build())).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();
    buf.into()
}

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// Stand up a fresh backend with a single registered collection
/// pointing at a Parquet body uploaded into an in-memory object
/// store. Returns the backend (cache warm-free) and the collection
/// name.
fn setup(n: usize, dim: usize) -> (GcsParquetBackend, &'static str) {
    let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
    let object = "embeddings/bench.parquet";
    let body = build_parquet(n, dim);
    rt().block_on(async {
        store.put(&OsPath::from(object), body.into()).await.unwrap();
    });
    let backend = GcsParquetBackend::with_store("bench-gcs", "fake-bucket", store).unwrap();
    backend.register(GcsParquetCollection {
        name: "docs".into(),
        object: object.into(),
        dim: None,
    });
    (backend, "docs")
}

fn bench_pull_vectors(c: &mut Criterion) {
    let mut group = c.benchmark_group("gcs_pull_vectors_in_memory");

    // Three sizes: tiny / small / medium. Vector body ≈ n * dim * 4 B.
    //   100  x 16   ≈   6.4 KiB
    //   1000 x 64   ≈ 256.0 KiB
    //   1000 x 384  ≈   1.5 MiB
    for &(n, dim) in &[(100usize, 16usize), (1000, 64), (1000, 384)] {
        let bytes_estimate = (n * dim * 4) as u64;
        group.throughput(Throughput::Bytes(bytes_estimate));
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("n={n}_dim={dim}")),
            &(n, dim),
            |b, &(n, dim)| {
                let (backend, name) = setup(n, dim);
                b.iter(|| {
                    // Each iter is a fresh pull — the BackendAdapter
                    // contract has the cache layer above us; pull is
                    // intentionally stateless from the adapter side.
                    let batch = backend.pull_vectors(black_box(name)).expect("pull");
                    black_box(batch);
                });
            },
        );
    }
    group.finish();
}

fn bench_current_bundle_cheap_path(c: &mut Criterion) {
    // ADR-004 §Resources contract — the `current_bundle()` override
    // must NOT call into pull_vectors. With the operator-supplied
    // `dim`, this should be one HEAD on the (in-memory) store plus
    // bundle-witness compute. The whole call collapses to ~single-
    // digit microseconds against InMemory.
    let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
    let object = "embeddings/wit.parquet";
    let body = build_parquet(10, 32);
    rt().block_on(async {
        store.put(&OsPath::from(object), body.into()).await.unwrap();
    });
    let backend = GcsParquetBackend::with_store("bench-gcs", "fake-bucket", store).unwrap();
    backend.register(GcsParquetCollection {
        name: "wit".into(),
        object: object.into(),
        dim: Some(32), // skip even the footer read
    });

    let mut group = c.benchmark_group("gcs_current_bundle_cheap_path");
    group.bench_function("witness_only_dim_known", |b| {
        b.iter(|| {
            let bundle = backend
                .current_bundle(black_box("wit"), 42, 20)
                .expect("bundle");
            black_box(bundle);
        });
    });
    group.finish();
}

criterion_group!(benches, bench_pull_vectors, bench_current_bundle_cheap_path);
criterion_main!(benches);
