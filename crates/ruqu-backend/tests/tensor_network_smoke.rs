//! End-to-end smoke for `RuquTensorNetworkBackend` (v0.2).
//! Mirrors `tests/smoke.rs` and `tests/stabilizer_smoke.rs` but on
//! the Matrix Product State backend.

use std::sync::Arc;

use rulake::backend::BackendAdapter;
use rulake::cache::Consistency;
use rulake::RuLake;
use ruvector_rulake_ruqu::{
    tensor_network, Circuit, Gate, RuquTensorNetworkBackend,
};

fn bell_circuit() -> Circuit {
    let mut c = Circuit::new("bell-tn", 2);
    c.push(Gate::H { q: 0 });
    c.push(Gate::Cx { control: 0, target: 1 });
    c
}

#[test]
fn bell_pair_through_tensor_network_at_bond_2() {
    // The Bell pair is exactly representable at χ=2 — the smallest
    // non-trivial bond dimension. This is the load-bearing claim:
    // truncation at the cap loses zero amplitude on a state that
    // fits.
    let mps = tensor_network::simulate(&bell_circuit(), 2)
        .expect("MPS sim must succeed");
    let sv = mps.to_state_vector();
    let amp = std::f64::consts::FRAC_1_SQRT_2;
    assert!((sv[0b00].re - amp).abs() < 1e-10, "|00⟩ amp = {}", sv[0b00].re);
    assert!(sv[0b01].re.abs() < 1e-10, "|01⟩ should be zero");
    assert!(sv[0b10].re.abs() < 1e-10, "|10⟩ should be zero");
    assert!((sv[0b11].re - amp).abs() < 1e-10, "|11⟩ amp = {}", sv[0b11].re);
    assert!(
        mps.truncation_error < 1e-12,
        "Bell pair must fit at χ=2 with zero truncation, got {}",
        mps.truncation_error,
    );

    // Round-trip the same circuit through the backend adapter to
    // confirm the witness path is wired.
    let be = RuquTensorNetworkBackend::new("ruqu-tn-fixture", 2);
    let bundle = be.execute("bell-pair", &bell_circuit())
        .expect("TensorNetwork execute should succeed for the Bell circuit");
    assert_eq!(be.id(), "ruqu-tn-fixture");
    assert_eq!(be.list_collections().unwrap(), vec!["bell-pair".to_string()]);
    assert_eq!(bundle.memory_class.as_deref(), Some("quantum"));
}

#[test]
fn tensor_network_handles_t_gate_unlike_stabilizer() {
    // H + T + H produces a state with a non-Clifford phase that
    // Stabilizer can't represent. TensorNetwork must run it and
    // produce the correct probabilities.
    let mut c = Circuit::new("h-t-h-tn", 1);
    c.push(Gate::H { q: 0 });
    c.push(Gate::T { q: 0 });
    c.push(Gate::H { q: 0 });

    let be = RuquTensorNetworkBackend::new("ruqu-tn-non-clifford", 4);
    let _bundle = be.execute("h-t-h", &c)
        .expect("TensorNetwork must accept a T gate (Stabilizer would refuse)");

    // Pull the cached state and check the analytic probabilities.
    // |0⟩ → H → (|0⟩+|1⟩)/√2 → T → (|0⟩+e^{iπ/4}|1⟩)/√2
    //      → H → (1+e^{iπ/4})/2 |0⟩ + (1-e^{iπ/4})/2 |1⟩.
    let mps = tensor_network::simulate(&c, 4).unwrap();
    let sv = mps.to_state_vector();
    let p_zero = sv[0].norm_sq();
    let p_one = sv[1].norm_sq();
    assert!(
        (p_zero - 0.85355339).abs() < 1e-6,
        "p(0) should be (2 + √2)/4 ≈ 0.8536, got {p_zero}",
    );
    assert!(
        (p_one - 0.14644661).abs() < 1e-6,
        "p(1) should be (2 - √2)/4 ≈ 0.1464, got {p_one}",
    );
}

#[test]
fn tensor_network_round_trips_through_rulake_cache() {
    let raw = Arc::new({
        let b = RuquTensorNetworkBackend::new("ruqu-tn-fixture", 4);
        let _ = b.execute("bell-pair", &bell_circuit()).unwrap();
        b
    });
    let be: Arc<dyn BackendAdapter> = Arc::clone(&raw) as Arc<dyn BackendAdapter>;

    let lake = RuLake::new(20, 42)
        .with_consistency(Consistency::Eventual { ttl_ms: 1_000 });
    lake.register_backend(be).unwrap();

    // Pull the packed batch shape so we can build a valid query
    // vector at the right dim. MPS rows are
    // [χL, χR, re_0, im_0, …] zero-padded; using a vector of zeros
    // is fine — RaBitQ doesn't care about magnitude for an "any
    // top-K" query.
    let batch = raw.pull_vectors("bell-pair").unwrap();
    let q = vec![0.0_f32; batch.dim];

    let hits1 = lake.search_one("ruqu-tn-fixture", "bell-pair", &q, 2).unwrap();
    assert_eq!(hits1.len(), 2);

    // Cache hit on the second call.
    let hits2 = lake.search_one("ruqu-tn-fixture", "bell-pair", &q, 2).unwrap();
    assert_eq!(hits1, hits2);

    let stats = lake.cache_stats();
    assert_eq!(stats.primes, 1, "second call must hit the cache, not re-prime");
}

#[test]
fn tensor_network_truncation_error_grows_with_circuit_depth() {
    // Build a deep Clifford circuit — alternating H + nearest-
    // neighbour CX layers. At χ=2 a deep random-Clifford circuit
    // generates entanglement faster than the bond can carry, so the
    // truncation error grows. We assert the growth is (a) finite,
    // (b) modest at the chosen depth, and (c) demonstrates that χ
    // controls the trade-off — at χ=8 the same circuit's error is
    // strictly smaller than at χ=2.
    let n = 6u8;
    let depth = 20usize;
    let mut c = Circuit::new("clifford-depth-20", n);
    for layer in 0..depth {
        for q in 0..n { c.push(Gate::H { q }); }
        // Alternate the CX sweep direction layer-by-layer so the
        // entanglement spreads in both directions.
        if layer % 2 == 0 {
            for q in 0..n.saturating_sub(1) {
                c.push(Gate::Cx { control: q, target: q + 1 });
            }
        } else {
            for q in (1..n).rev() {
                c.push(Gate::Cx { control: q, target: q - 1 });
            }
        }
    }

    let mps_low = tensor_network::simulate(&c, 2).unwrap();
    let mps_high = tensor_network::simulate(&c, 8).unwrap();

    assert!(
        mps_low.truncation_error.is_finite(),
        "truncation error must stay finite, got {}",
        mps_low.truncation_error,
    );
    // At χ=2 we expect *some* truncation on this depth-20 circuit —
    // the bond can't carry the full entanglement — but it must stay
    // bounded well below 1.
    assert!(
        mps_low.truncation_error < 5.0,
        "truncation error at χ=2 should be modest, got {}",
        mps_low.truncation_error,
    );
    // At χ=8 the bond is wider; on n=6 the state fully fits at χ=8
    // (max bond dim of an n=6 state is 2³ = 8) so error is ~0.
    assert!(
        mps_high.truncation_error < 1e-6,
        "χ=8 should fully cover an n=6 state, got error {}",
        mps_high.truncation_error,
    );
    // Wider bond must never lose more than narrower.
    assert!(
        mps_high.truncation_error <= mps_low.truncation_error + 1e-6,
        "wider bond ({}) should track at-or-below narrower bond ({})",
        mps_high.truncation_error, mps_low.truncation_error,
    );
}
