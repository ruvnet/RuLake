//! `verify_bundle()` micro-bench for `IpfsBackend`.
//!
//! The IPFS backend is the bundle-distribution plane (ADR-005 §1).
//! Two load-bearing paths:
//!
//!   1. **Witness recomputation** — `RuLakeBundle::verify_witness`.
//!      Every fetched bundle runs through this; it's the SHAKE-256
//!      hash of the canonical bundle fields. Pure CPU, no I/O.
//!   2. **JSON round-trip + witness verify** — `to_json` → bytes →
//!      `from_json` (which itself enforces 64 KiB / 4 KiB caps and
//!      then drops back through `verify_witness` in the
//!      `read_from_dir` flow). This is what `fetch_bundle` does
//!      after `cat` returns the bytes from kubo / a gateway.
//!   3. **Cheap-path `current_bundle`** — operator-supplied CID + dim
//!      → no network at all, just construct + witness compute.
//!      ADR-005 §4.
//!
//! What's measured:
//!   - bundle JSON serialization
//!   - bundle JSON deserialization (with the size-cap checks)
//!   - witness recomputation
//!   - `IpfsBackend::current_bundle` cheap path (no HTTP)
//!
//! What's excluded (would require live kubo or a mock HTTP server):
//!   - kubo `/api/v0/cat` round trip
//!   - gateway HTTPS GET
//!   - kubo `/api/v0/add` + `pin/add` (publish_bundle)
//!
//! The mock-kubo path used by the smoke tests pulls in hyper and
//! adds I/O latency; including it here would measure the mock, not
//! the substrate. Verify_bundle in isolation is the right knob.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rulake::backend::BackendAdapter;
use rulake::bundle::{Generation, RuLakeBundle};
use ruvector_rulake_ipfs::{IpfsBackend, IpfsCollection, KuboApiUrl, Mode};

const FAKE_CID: &str = "bafybeifaketestcidforipfsbackendbenchonlyy";

fn make_bundle(dim: usize) -> RuLakeBundle {
    RuLakeBundle::new(
        format!("ipfs://{FAKE_CID}"),
        dim,
        42,
        20,
        Generation::Opaque(FAKE_CID.into()),
    )
}

fn bench_verify_witness(c: &mut Criterion) {
    // SHAKE-256 over the canonical field set; cost is roughly constant
    // in the bundle's "interesting" fields (data_ref, dim, seed,
    // rerank, generation) — `dim` doesn't change the input bytes
    // beyond a few digits. We sweep dim anyway as a sanity check.
    let mut group = c.benchmark_group("ipfs_verify_witness");
    for dim in &[8usize, 128, 1024] {
        let bundle = make_bundle(*dim);
        group.bench_with_input(BenchmarkId::from_parameter(dim), dim, |b, _dim| {
            b.iter(|| {
                let ok = bundle.verify_witness();
                black_box(ok);
            });
        });
    }
    group.finish();
}

fn bench_json_roundtrip(c: &mut Criterion) {
    // `fetch_bundle` does cat→from_json→verify_witness. Skip the cat
    // (network); measure the (de)serialize + witness verify the
    // bundle goes through once it's been pulled.
    let mut group = c.benchmark_group("ipfs_bundle_json_roundtrip");
    for dim in &[8usize, 128, 1024] {
        let bundle = make_bundle(*dim);
        let json = bundle.to_json().expect("to_json");
        let bytes_len = json.len() as u64;
        group.throughput(Throughput::Bytes(bytes_len));
        group.bench_with_input(BenchmarkId::from_parameter(dim), dim, |b, _dim| {
            b.iter(|| {
                // Mimic what fetch_bundle does after `cat`: parse
                // bytes into bundle, then witness-verify. from_json
                // already enforces the 64 KiB / 4 KiB caps.
                let parsed = RuLakeBundle::from_json(black_box(&json)).expect("from_json");
                let ok = parsed.verify_witness();
                black_box((parsed, ok));
            });
        });
    }
    group.finish();
}

fn bench_current_bundle_cheap_path(c: &mut Criterion) {
    // Operator pre-set CID + dim — should bypass kubo entirely. We
    // can point at an unreachable kubo URL because the cheap path
    // never opens a connection.
    let backend = IpfsBackend::new(
        "ipfs-bench",
        Mode::Kubo {
            api: KuboApiUrl("http://127.0.0.1:1".into()),
            api_token: None,
        },
    )
    .expect("backend");
    backend
        .register(IpfsCollection {
            name: "wit".into(),
            cid: Some(FAKE_CID.into()),
            dim: Some(128),
        })
        .expect("register");

    let mut group = c.benchmark_group("ipfs_current_bundle_cheap_path");
    group.bench_function("operator_cid_dim_known", |b| {
        b.iter(|| {
            let bundle = backend
                .current_bundle(black_box("wit"), 42, 20)
                .expect("bundle");
            // Sanity — confirms we didn't accidentally fall into a
            // network path.
            debug_assert!(bundle.verify_witness());
            black_box(bundle);
        });
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_verify_witness,
    bench_json_roundtrip,
    bench_current_bundle_cheap_path
);
criterion_main!(benches);
