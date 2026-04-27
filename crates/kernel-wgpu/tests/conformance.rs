//! ADR-157 acceptance tests for [`WgpuKernel`].
//!
//! Per the lib-level docs and ADR-157 §"Determinism as a hard gate":
//!
//! - The **popcount path** is exact integer math; we assert byte-equal
//!   output against `CpuNaiveKernel` on the deterministic conformance
//!   fixture across multiple seeds (matches clause 1: "scan must be
//!   byte-equal").
//! - The **L2 path** is recall-equivalent across drivers but the WGSL
//!   spec doesn't pin sub-ULP behaviour, so we assert top-K *set*
//!   equality (matches clause 2: "rerank may be float-nondeterministic
//!   on GPU; the cap bit announces it").
//!
//! Tests gracefully skip with an `eprintln!` and exit success when no
//! GPU adapter is available (typical for headless CI). For strict CI
//! gates on GPU-equipped runners, mark them `#[ignore]` and run them
//! via `cargo test -- --ignored`.

use rulake::kernel::{CpuNaiveKernel, VectorKernel};
use ruvector_rulake_kernel_wgpu::WgpuKernel;

fn try_kernel() -> Option<WgpuKernel> {
    match WgpuKernel::new_blocking() {
        Ok(k) => Some(k),
        Err(e) => {
            eprintln!("skipping wgpu conformance: {e}");
            None
        }
    }
}

/// Same LCG fixture `assert_kernel_conformant` uses, exposed here so we
/// can drive the popcount path byte-for-byte and the L2 path
/// set-equivalent under one helper.
fn fixture(seed: u64) -> (Vec<f32>, Vec<Vec<f32>>, Vec<u64>, Vec<Vec<u64>>) {
    let dim = 16usize;
    let n = 10usize;
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1);
    let mut next_u64 = || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        state
    };
    let mut next_f32 = || {
        let bits = (next_u64() >> 40) as u32;
        let x = (bits as f32) / (1u32 << 24) as f32;
        x * 2.0 - 1.0
    };
    let qf: Vec<f32> = (0..dim).map(|_| next_f32()).collect();
    let cf: Vec<Vec<f32>> = (0..n).map(|_| (0..dim).map(|_| next_f32()).collect()).collect();
    let qu: Vec<u64> = (0..dim).map(|_| next_u64()).collect();
    let cu: Vec<Vec<u64>> = (0..n).map(|_| (0..dim).map(|_| next_u64()).collect()).collect();
    (qf, cf, qu, cu)
}

fn assert_popcount_byte_equal(k: &WgpuKernel, seed: u64) {
    let (_qf, _cf, qu, cu) = fixture(seed);
    let top_k = 5;
    let got = k.rabitq_popcount(&qu, &cu, top_k);
    let want = CpuNaiveKernel.rabitq_popcount(&qu, &cu, top_k);
    assert_eq!(
        got, want,
        "wgpu rabitq_popcount must be byte-equal to CpuNaive (seed {seed})"
    );
}

fn assert_l2_top_k_set_equal(k: &WgpuKernel, seed: u64) {
    let (qf, cf, _qu, _cu) = fixture(seed);
    let top_k = 5;
    let got = k.l2_distance_one(&qf, &cf, top_k);
    let want = CpuNaiveKernel.l2_distance_one(&qf, &cf, top_k);
    let got_set: std::collections::BTreeSet<u64> = got.iter().map(|(i, _)| *i).collect();
    let want_set: std::collections::BTreeSet<u64> = want.iter().map(|(i, _)| *i).collect();
    assert_eq!(
        got_set, want_set,
        "wgpu l2_distance_one top-K set must match CpuNaive (seed {seed})"
    );
    // Distances must agree to within a small relative tolerance —
    // wgpu drivers may reorder floating ops, so we use 1e-4 (about
    // log2(10000) ≈ 13 bits of precision) which still catches a
    // genuinely-broken shader without flagging legitimate per-driver
    // float reordering.
    for ((gi, gv), (wi, wv)) in got.iter().zip(want.iter()) {
        assert_eq!(gi, wi, "top-K candidate order diverges (seed {seed})");
        let denom = wv.abs().max(1e-12);
        let rel = (gv - wv).abs() / denom;
        assert!(
            rel < 1e-4,
            "wgpu L2 distance off by > 1e-4 relative ({gv} vs {wv}, seed {seed})"
        );
    }
}

#[test]
fn conformance_popcount_byte_equal_two_seeds() {
    let Some(k) = try_kernel() else { return };
    assert_popcount_byte_equal(&k, 42);
    assert_popcount_byte_equal(&k, 1337);
}

#[test]
fn conformance_l2_set_equal_two_seeds() {
    let Some(k) = try_kernel() else { return };
    assert_l2_top_k_set_equal(&k, 42);
    assert_l2_top_k_set_equal(&k, 1337);
}
