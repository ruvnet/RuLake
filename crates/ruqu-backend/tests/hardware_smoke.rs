//! End-to-end smoke for `RuquHardwareStubBackend` (v0.2).
//! Mirrors `tests/smoke.rs` but on the Hardware (stub) backend.

use std::sync::Arc;

use rulake::backend::BackendAdapter;
use rulake::cache::Consistency;
use rulake::RuLake;
use ruvector_rulake_ruqu::{Circuit, Gate, RuquHardwareStubBackend, RuquStateVectorBackend};

fn bell_circuit() -> Circuit {
    let mut c = Circuit::new("bell-hw", 2);
    c.push(Gate::H { q: 0 });
    c.push(Gate::Cx { control: 0, target: 1 });
    c
}

#[test]
fn hardware_stub_runs_bell_pair() {
    let be = RuquHardwareStubBackend::new("ruqu-hw-fixture", 0.02, 1);
    let bundle = be.execute("bell-pair", &bell_circuit())
        .expect("Hardware stub execute should succeed for Bell pair");

    assert_eq!(be.id(), "ruqu-hw-fixture");
    assert_eq!(be.list_collections().unwrap(), vec!["bell-pair".to_string()]);
    assert_eq!(bundle.memory_class.as_deref(), Some("quantum"));

    let batch = be.pull_vectors("bell-pair").unwrap();
    assert_eq!(batch.collection, "bell-pair");
    assert_eq!(batch.dim, 2); // [re, im] per row
    assert_eq!(batch.ids.len(), 4); // 2-qubit Hilbert space → 4 basis states
    assert_eq!(batch.vectors.len(), 4);
    assert_eq!(batch.generation, 1);

    // Sum of |amplitude|² should be ≈ 1.0 within 1e-3 (the stub re-
    // normalises after the synthetic noise injection).
    let total: f64 = batch
        .vectors
        .iter()
        .map(|v| {
            let re = v[0] as f64;
            let im = v[1] as f64;
            re * re + im * im
        })
        .sum();
    assert!(
        (total - 1.0).abs() < 1e-3,
        "noisy state must remain unit-norm within 1e-3; got total={total}"
    );
}

#[test]
fn hardware_stub_witness_distinct_from_state_vector() {
    let circuit = bell_circuit();
    let sv = RuquStateVectorBackend::new("sv-fixture");
    let hw = RuquHardwareStubBackend::new("hw-fixture", 0.0, 0);
    let bundle_sv = sv.execute("bell", &circuit).unwrap();
    let bundle_hw = hw.execute("bell", &circuit).unwrap();
    // Backend-tag rotation in `data_ref` MUST keep these distinct,
    // even with identical seed and zero noise — that's the v0.2
    // contract per ADR-008 §4 (concordance mode is a v0.3 strip).
    assert_ne!(
        bundle_sv.rvf_witness, bundle_hw.rvf_witness,
        "hardware-stub witness must differ from sv witness — backend tag carries the distinction"
    );
}

#[test]
fn hardware_stub_round_trips_through_rulake_cache() {
    let raw = Arc::new({
        let b = RuquHardwareStubBackend::new("ruqu-hw-fixture", 0.01, 7);
        let _ = b.execute("bell-pair", &bell_circuit()).unwrap();
        b
    });
    let be: Arc<dyn BackendAdapter> = Arc::clone(&raw) as Arc<dyn BackendAdapter>;

    let lake = RuLake::new(20, 42).with_consistency(Consistency::Eventual { ttl_ms: 1_000 });
    lake.register_backend(be).unwrap();

    let q = vec![1.0_f32, 0.0]; // dim=2 query
    let hits1 = lake.search_one("ruqu-hw-fixture", "bell-pair", &q, 3).unwrap();
    assert_eq!(hits1.len(), 3);

    // Cache hit on the second call.
    let hits2 = lake.search_one("ruqu-hw-fixture", "bell-pair", &q, 3).unwrap();
    assert_eq!(hits1, hits2);

    let stats = lake.cache_stats();
    assert_eq!(stats.primes, 1, "second call must hit the cache, not re-prime");
}
