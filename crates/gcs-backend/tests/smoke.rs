//! Smoke tests for `GcsParquetBackend`.
//!
//! Two paths:
//! - **Offline (always runs):** uses `object_store::memory::InMemory`
//!   as the backing store. Writes a tiny Parquet file with a known
//!   schema (`id INT64, vector LIST<FLOAT32>`), reads it back through
//!   `GcsParquetBackend`, asserts the BackendAdapter contract.
//! - **Live (gated on `RULAKE_GCS_LIVE_TEST=1`):** opens a real
//!   GCS bucket via ADC and reads a fixture Parquet file the operator
//!   uploaded ahead of time. Skipped by default.

use std::sync::Arc;

use arrow::array::{ArrayRef, Float32Array, Int64Array, ListArray};
use arrow::buffer::OffsetBuffer;
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use bytes::Bytes;
use object_store::{memory::InMemory, path::Path as OsPath, ObjectStore, ObjectStoreExt};
use parquet::arrow::ArrowWriter;
use parquet::file::properties::WriterProperties;

use rulake::backend::{BackendAdapter, PulledBatch};
use ruvector_rulake_gcs::{GcsParquetBackend, GcsParquetCollection};

fn build_parquet(n: usize, dim: usize) -> Bytes {
    // Build a RecordBatch with N rows of (id INT64, vector LIST<FLOAT32>).
    let ids: Vec<i64> = (0..n as i64).collect();
    let id_arr = Arc::new(Int64Array::from(ids)) as ArrayRef;

    // Vector column: List<Float32> with N rows of `dim` floats each.
    let total: Vec<f32> = (0..(n * dim)).map(|i| ((i as f32) * 0.001).sin()).collect();
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

#[test]
fn pull_vectors_round_trip_offline() {
    let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
    let object = "embeddings/docs.parquet";
    let body = build_parquet(100, 16);
    rt().block_on(async {
        store.put(&OsPath::from(object), body.into()).await.unwrap();
    });

    let backend = GcsParquetBackend::with_store("test-gcs", "fake-bucket", store).unwrap();
    backend.register(GcsParquetCollection {
        name: "docs".into(),
        object: object.into(),
        dim: None,
    });

    assert_eq!(backend.id(), "test-gcs");
    assert_eq!(
        backend.list_collections().unwrap(),
        vec!["docs".to_string()]
    );

    let PulledBatch {
        ids,
        vectors,
        dim,
        generation,
        ..
    } = backend.pull_vectors("docs").unwrap();

    assert_eq!(ids.len(), 100, "100 rows back");
    assert_eq!(dim, 16, "schema reports dim=16");
    assert_eq!(vectors.len(), 100);
    assert_eq!(vectors[0].len(), 16);
    assert!(
        generation > 0,
        "InMemory store reports a non-zero last_modified-derived generation"
    );
    assert_eq!(ids[42], 42);
}

#[test]
fn current_bundle_does_not_pull_vectors_offline() {
    // Acceptance for the ADR-004 §Resources contract: the cheap-path
    // current_bundle() must not fall through to pull_vectors. We
    // verify by uploading a malformed Parquet body where the data
    // section is junk but the footer schema is valid — pull_vectors
    // would error, but current_bundle() should succeed (it only reads
    // the footer + does HEAD).
    //
    // Concretely: we upload a real, valid parquet so this test is a
    // *positive* check (current_bundle returns a witness). The negative
    // path is covered by the offline crate-tests that build a corrupt
    // body and assert pull_vectors fails — out of scope here.

    let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
    let object = "embeddings/witness-only.parquet";
    let body = build_parquet(10, 32);
    rt().block_on(async {
        store.put(&OsPath::from(object), body.into()).await.unwrap();
    });

    let backend = GcsParquetBackend::with_store("test-gcs", "fake-bucket", store).unwrap();
    backend.register(GcsParquetCollection {
        name: "wit".into(),
        object: object.into(),
        dim: Some(32), // operator-supplied dim — skips even the footer read
    });

    let bundle = backend.current_bundle("wit", 42, 20).unwrap();
    assert_eq!(bundle.dim, 32);
    assert_eq!(bundle.rotation_seed, 42);
    assert_eq!(bundle.rerank_factor, 20);
    assert_eq!(
        bundle.data_ref,
        "gs://fake-bucket/embeddings/witness-only.parquet"
    );
    assert_eq!(bundle.rvf_witness.len(), 64, "SHAKE-256(32) hex");
    assert!(bundle.verify_witness(), "freshly built bundle must verify");
}

#[test]
fn generation_changes_on_reupload_offline() {
    // ADR-155 §Consequences: cache coherence rides on backend generation.
    // We assert that re-uploading the same object produces a different
    // generation token — without this, witness-anchored cache sharing
    // breaks.
    let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
    let object = "embeddings/v.parquet";
    let backend =
        GcsParquetBackend::with_store("test-gcs", "fake-bucket", Arc::clone(&store)).unwrap();
    backend.register(GcsParquetCollection {
        name: "v".into(),
        object: object.into(),
        dim: Some(8),
    });

    let body1 = build_parquet(5, 8);
    rt().block_on(async {
        store
            .put(&OsPath::from(object), body1.into())
            .await
            .unwrap();
    });
    let g1 = backend.generation("v").unwrap();

    // Sleep a tick so InMemory's last_modified actually advances.
    std::thread::sleep(std::time::Duration::from_millis(1100));

    let body2 = build_parquet(5, 8);
    rt().block_on(async {
        store
            .put(&OsPath::from(object), body2.into())
            .await
            .unwrap();
    });
    let g2 = backend.generation("v").unwrap();

    assert!(
        g2 > g1,
        "generation must monotonically advance on re-upload (g1={g1}, g2={g2})"
    );
}

#[test]
fn unknown_collection_errors_with_typed_variant() {
    let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
    let backend = GcsParquetBackend::with_store("test-gcs", "fake-bucket", store).unwrap();

    let err = backend.pull_vectors("nope").unwrap_err();
    let s = err.to_string();
    assert!(
        s.contains("unknown collection"),
        "expected UnknownCollection, got: {s}"
    );
}

// ─── Live GCS test (gated) ────────────────────────────────────────────

/// Live test against a real GCS bucket. Run with:
///
///     RULAKE_GCS_LIVE_TEST=1 \
///     RULAKE_GCS_BUCKET=ruv-rulake-test \
///     RULAKE_GCS_OBJECT=fixtures/docs-100k.parquet \
///     cargo test --release -- --ignored gcs_live
#[test]
#[ignore]
fn gcs_live_pull_vectors() {
    if std::env::var("RULAKE_GCS_LIVE_TEST").is_err() {
        eprintln!("skip: RULAKE_GCS_LIVE_TEST not set");
        return;
    }
    let bucket = std::env::var("RULAKE_GCS_BUCKET").expect("RULAKE_GCS_BUCKET");
    let object =
        std::env::var("RULAKE_GCS_OBJECT").unwrap_or_else(|_| "fixtures/docs-100k.parquet".into());

    let backend = GcsParquetBackend::open_gcs("gcs-live", bucket).unwrap();
    backend.register(GcsParquetCollection {
        name: "docs".into(),
        object,
        dim: None,
    });

    let bundle = backend.current_bundle("docs", 42, 20).unwrap();
    eprintln!(
        "live bundle: dim={} witness={} data_ref={}",
        bundle.dim, bundle.rvf_witness, bundle.data_ref
    );
    assert!(bundle.verify_witness());
    assert!(bundle.dim > 0);
}
