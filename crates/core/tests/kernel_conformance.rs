//! ADR-157 acceptance tests for the `VectorKernel` scaffolding.
//!
//! Every other kernel impl (SIMD / GPU / WASM) is expected to copy
//! these tests, point them at its own `VectorKernel` implementation,
//! and pass without modification. That equivalence is the promotion
//! gate from "experimental crate" to "default dispatch preference".

use rulake::kernel::{assert_kernel_conformant, CpuNaiveKernel, KernelCapabilities, VectorKernel};

#[test]
fn cpu_naive_passes_self_conformance() {
    // Trivially true — checks that the helper compiles, runs against
    // a real `&dyn VectorKernel`, and that the fixture is stable.
    let k = CpuNaiveKernel;
    for seed in [0u64, 1, 7, 0xC0FFEE] {
        assert_kernel_conformant(&k, seed);
    }
}

#[test]
fn cpu_naive_l2_returns_sorted_top_k() {
    let k = CpuNaiveKernel;
    // Query at the origin; candidates are placed at increasing
    // distance so the expected top-K is just the first K indices.
    let query = vec![0.0f32; 4];
    let candidates: Vec<Vec<f32>> = (0..8)
        .map(|i| {
            let v = (i as f32) + 1.0;
            vec![v, 0.0, 0.0, 0.0]
        })
        .collect();

    let top = k.l2_distance_one(&query, &candidates, 5);
    assert_eq!(top.len(), 5);
    // Strictly ascending by score.
    for win in top.windows(2) {
        assert!(
            win[0].1 <= win[1].1,
            "top-K must be sorted ascending by L2: {:?}",
            top
        );
    }
    // Closest candidate (index 0, distance 1.0) wins.
    assert_eq!(top[0].0, 0);
    assert!((top[0].1 - 1.0).abs() < 1e-6);
}

#[test]
fn cpu_naive_popcount_matches_count_ones_reference() {
    let k = CpuNaiveKernel;
    let query: Vec<u64> = vec![0x0000_0000_0000_0000, 0xFFFF_FFFF_FFFF_FFFF];
    let candidates: Vec<Vec<u64>> = vec![
        vec![0x0000_0000_0000_0000, 0xFFFF_FFFF_FFFF_FFFF], // identical → 0
        vec![0xFFFF_FFFF_FFFF_FFFF, 0x0000_0000_0000_0000], // inverted → 128
        vec![0x0000_0000_0000_00FF, 0xFFFF_FFFF_FFFF_FF00], // 8 + 8 = 16
        vec![0x1234_5678_9ABC_DEF0, 0x0F1E_2D3C_4B5A_6978], // mixed
    ];

    let want: Vec<(u64, u32)> = candidates
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let h: u32 = query
                .iter()
                .zip(c.iter())
                .map(|(q, x)| (q ^ x).count_ones())
                .sum();
            (i as u64, h)
        })
        .collect();
    let mut want_sorted = want.clone();
    want_sorted.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    let want_top = want_sorted.into_iter().take(3).collect::<Vec<_>>();

    let got = k.rabitq_popcount(&query, &candidates, 3);
    assert_eq!(
        got, want_top,
        "kernel popcount must match a hand-rolled count_ones reference"
    );
}

#[test]
fn cpu_naive_capabilities_struct_matches_default() {
    let k = CpuNaiveKernel;
    let caps = k.capabilities();
    assert_eq!(caps, KernelCapabilities::default());
    assert_eq!(caps.simd_width, 1);
    assert!(!caps.popcount_native);
    assert!(!caps.gpu);
    assert_eq!(k.id(), "cpu-naive");
}
