//! End-to-end smoke for `RuquStateVectorBackend`. Confirms the
//! BackendAdapter contract holds and a Bell-pair execution round-
//! trips through ruLake with a stable witness.

use std::sync::Arc;

use rulake::backend::BackendAdapter;
use rulake::cache::Consistency;
use rulake::RuLake;
use ruvector_rulake_ruqu::{Circuit, Gate, RuquStateVectorBackend};

fn bell_circuit() -> Circuit {
    let mut c = Circuit::new("bell", 2);
    c.push(Gate::H { q: 0 });
    c.push(Gate::Cx { control: 0, target: 1 });
    c
}

#[test]
fn statevector_backend_adapter_contract_holds() {
    let be = RuquStateVectorBackend::new("ruqu-fixture");
    let bundle = be.execute("bell-pair", &bell_circuit()).unwrap();

    assert_eq!(be.id(), "ruqu-fixture");
    assert_eq!(be.list_collections().unwrap(), vec!["bell-pair".to_string()]);
    assert_eq!(bundle.memory_class.as_deref(), Some("quantum"));

    let batch = be.pull_vectors("bell-pair").unwrap();
    assert_eq!(batch.collection, "bell-pair");
    assert_eq!(batch.dim, 2);
    assert_eq!(batch.ids.len(), 4); // 2-qubit Hilbert space
    assert_eq!(batch.vectors.len(), 4);
    assert_eq!(batch.generation, 1);

    // |00⟩ and |11⟩ should each carry amplitude ≈ 1/√2 ≈ 0.7071.
    let amp = std::f32::consts::FRAC_1_SQRT_2;
    assert!((batch.vectors[0b00][0] - amp).abs() < 1e-6);
    assert!((batch.vectors[0b11][0] - amp).abs() < 1e-6);
    // |01⟩ and |10⟩ should be ≈ 0.
    assert!(batch.vectors[0b01][0].abs() < 1e-6);
    assert!(batch.vectors[0b10][0].abs() < 1e-6);
}

#[test]
fn statevector_round_trips_through_rulake_cache() {
    let raw = Arc::new({
        let b = RuquStateVectorBackend::new("ruqu-fixture");
        let _ = b.execute("bell-pair", &bell_circuit()).unwrap();
        b
    });
    let be: Arc<dyn BackendAdapter> = Arc::clone(&raw) as Arc<dyn BackendAdapter>;

    // The Hilbert dim is 4; ask for top-3 against |00⟩ as the query
    // (real-only, padded into the [re, im] layout).
    let lake = RuLake::new(20, 42).with_consistency(Consistency::Eventual { ttl_ms: 1_000 });
    lake.register_backend(be).unwrap();

    let q = vec![1.0_f32, 0.0]; // dim=2 query vector
    let hits1 = lake.search_one("ruqu-fixture", "bell-pair", &q, 3).unwrap();
    assert_eq!(hits1.len(), 3);

    // Cache hit on the second call.
    let hits2 = lake.search_one("ruqu-fixture", "bell-pair", &q, 3).unwrap();
    assert_eq!(hits1, hits2);

    let stats = lake.cache_stats();
    assert_eq!(stats.primes, 1, "second call must hit the cache");
}

#[test]
fn generation_bump_invalidates_cache() {
    let raw = Arc::new({
        let b = RuquStateVectorBackend::new("ruqu-fixture");
        let _ = b.execute("bell-pair", &bell_circuit()).unwrap();
        b
    });
    let be: Arc<dyn BackendAdapter> = Arc::clone(&raw) as Arc<dyn BackendAdapter>;

    let lake = RuLake::new(20, 42).with_consistency(Consistency::Fresh);
    lake.register_backend(be).unwrap();

    let q = vec![1.0_f32, 0.0];
    let _ = lake.search_one("ruqu-fixture", "bell-pair", &q, 3).unwrap();
    let primes_before = lake.cache_stats().primes;

    let _ = raw.bump_generation("bell-pair").unwrap();

    let _ = lake.search_one("ruqu-fixture", "bell-pair", &q, 3).unwrap();
    let primes_after = lake.cache_stats().primes;
    assert!(primes_after > primes_before, "Fresh + gen bump must invalidate");
}
