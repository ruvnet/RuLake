//! Criterion bench for [`Avx512Kernel::l2_distance_one`], mirroring
//! the root crate's `benches/kernel_l2.rs` so the two numbers can be
//! placed side by side.
//!
//! Per ADR-157 the relevant comparison is **AVX-512 vs CpuNaive at
//! identical recall@10** — which is trivially satisfied here because
//! the conformance suite already pins the AVX-512 path to bit-equal
//! output. This bench therefore measures pure throughput.
//!
//! Falls back to no-op benches when AVX-512 isn't detected at runtime,
//! so `cargo bench` doesn't crash on non-AVX-512 hosts.
//!
//! Headline grid: dim=384, n=16384 (matches root bench), with both
//! `CpuNaiveKernel` and `Avx512Kernel` sampled in the same group so
//! the comparison is on identical inputs.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

use rulake::kernel::{CpuNaiveKernel, VectorKernel};
use ruvector_rulake_kernel_avx512::Avx512Kernel;

fn make_inputs(dim: usize, n: usize, seed: u64) -> (Vec<f32>, Vec<Vec<f32>>) {
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1);
    let mut next_f32 = || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let bits = (state >> 40) as u32;
        let x = (bits as f32) / (1u32 << 24) as f32;
        x * 2.0 - 1.0
    };
    let query: Vec<f32> = (0..dim).map(|_| next_f32()).collect();
    let candidates: Vec<Vec<f32>> = (0..n)
        .map(|_| (0..dim).map(|_| next_f32()).collect())
        .collect();
    (query, candidates)
}

fn bench_l2(c: &mut Criterion) {
    let top_k = 10;
    // Keep the headline at the same point as the root bench so the
    // numbers can be compared directly: dim=384, n=16384.
    let dims = [384usize];
    let ns = [16_384usize];

    let avx = Avx512Kernel::new();
    if avx.is_none() {
        eprintln!("note: AVX-512 not detected; only the CpuNaive baseline will be benched");
    }

    let mut group = c.benchmark_group("kernel_l2");
    for &dim in &dims {
        for &n in &ns {
            let (query, candidates) = make_inputs(dim, n, 0xA5A5_5A5A);
            group.throughput(Throughput::Elements(n as u64));

            // Baseline.
            let naive = CpuNaiveKernel;
            group.bench_with_input(
                BenchmarkId::new("cpu-naive", format!("dim={dim}/n={n}")),
                &(&query, &candidates),
                |b, (q, cs)| {
                    b.iter(|| {
                        let r = naive.l2_distance_one(black_box(q), black_box(cs), top_k);
                        black_box(r);
                    });
                },
            );

            // AVX-512, only if available.
            if let Some(k) = avx.as_ref() {
                group.bench_with_input(
                    BenchmarkId::new("avx512", format!("dim={dim}/n={n}")),
                    &(&query, &candidates),
                    |b, (q, cs)| {
                        b.iter(|| {
                            let r = k.l2_distance_one(black_box(q), black_box(cs), top_k);
                            black_box(r);
                        });
                    },
                );
            }
        }
    }
    group.finish();
}

criterion_group!(benches, bench_l2);
criterion_main!(benches);
