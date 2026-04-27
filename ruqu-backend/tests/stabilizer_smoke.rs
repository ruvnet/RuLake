//! End-to-end smoke for `RuquStabilizerBackend` (v0.1).
//! Mirrors `tests/smoke.rs` but on the tableau backend.

use std::sync::Arc;

use rulake::backend::BackendAdapter;
use rulake::cache::Consistency;
use rulake::RuLake;
use ruvector_rulake_ruqu::{Circuit, Gate, RuquStabilizerBackend, RuquStateVectorBackend};

fn bell_circuit() -> Circuit {
    let mut c = Circuit::new("bell-stab", 2);
    c.push(Gate::H { q: 0 });
    c.push(Gate::Cx { control: 0, target: 1 });
    c
}

#[test]
fn bell_pair_through_stabilizer() {
    let be = RuquStabilizerBackend::new("ruqu-stab-fixture");
    let bundle = be.execute("bell-pair", &bell_circuit())
        .expect("Stabilizer execute should succeed for an all-Clifford circuit");

    assert_eq!(be.id(), "ruqu-stab-fixture");
    assert_eq!(be.list_collections().unwrap(), vec!["bell-pair".to_string()]);
    assert_eq!(bundle.memory_class.as_deref(), Some("quantum"));

    let batch = be.pull_vectors("bell-pair").unwrap();
    assert_eq!(batch.collection, "bell-pair");
    // (2 * n + 1) tableau rows for n=2 → 5 rows.
    assert_eq!(batch.vectors.len(), 5);
    // Row layout: [coeff_re, coeff_im, x_0, x_1, z_0, z_1, r] → 7 slots.
    assert_eq!(batch.dim, 7);
    for v in &batch.vectors {
        assert_eq!(v.len(), 7);
        assert_eq!(v[0], 1.0);
        assert_eq!(v[1], 0.0);
    }
    assert_eq!(batch.generation, 1);
}

#[test]
fn stabilizer_refuses_t_gate() {
    let be = RuquStabilizerBackend::new("ruqu-stab-fixture");
    let mut c = Circuit::new("has-t", 1);
    c.push(Gate::T { q: 0 });
    let err = be.execute("nope", &c).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.to_lowercase().contains("non-clifford") || msg.contains("'t'") || msg.contains(" t "),
        "expected the error to identify the non-Clifford gate; got {msg:?}",
    );
    // Nothing should have been stored.
    assert!(be.list_collections().unwrap().is_empty());
}

#[test]
fn stabilizer_round_trips_through_rulake_cache() {
    let raw = Arc::new({
        let b = RuquStabilizerBackend::new("ruqu-stab-fixture");
        let _ = b.execute("bell-pair", &bell_circuit()).unwrap();
        b
    });
    let be: Arc<dyn BackendAdapter> = Arc::clone(&raw) as Arc<dyn BackendAdapter>;

    let lake = RuLake::new(20, 42).with_consistency(Consistency::Eventual { ttl_ms: 1_000 });
    lake.register_backend(be).unwrap();

    // Query in the packed-row dim (7 for n=2): [coeff_re=1, coeff_im=0, X bits, Z bits, r].
    let q = vec![1.0_f32, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    let hits1 = lake.search_one("ruqu-stab-fixture", "bell-pair", &q, 3).unwrap();
    assert_eq!(hits1.len(), 3);

    // Cache hit on the second call.
    let hits2 = lake.search_one("ruqu-stab-fixture", "bell-pair", &q, 3).unwrap();
    assert_eq!(hits1, hits2);

    let stats = lake.cache_stats();
    assert_eq!(stats.primes, 1, "second call must hit the cache, not re-prime");
}

/// Concordance test (ADR-008 §G4): the StateVector and Stabilizer
/// backends should produce *equal* witnesses for the same all-Clifford
/// circuit when run under a `--concordance-mode` flag that strips the
/// backend tag from `data_ref`. v0.1 deliberately keeps the backend
/// tag in the witness (so the cache discriminates the two backends),
/// so this test would fail today; it documents the v0.2 contract.
///
/// Implementation deferred to v0.2 along with the strip flag.
#[test]
#[ignore = "v0.2: needs --concordance-mode to strip the backend tag from data_ref"]
fn concordance_witness_equality() {
    let circuit = bell_circuit();
    let sv = RuquStateVectorBackend::new("sv");
    let st = RuquStabilizerBackend::new("st");
    let bundle_sv = sv.execute("bell", &circuit).unwrap();
    let bundle_st = st.execute("bell", &circuit).unwrap();
    // v0.2 must make this assertion hold under concordance mode.
    assert_eq!(bundle_sv.rvf_witness, bundle_st.rvf_witness);
}
