//! Baseline criterion bench for `CpuNaiveKernel::l2_distance_one`
//! across a representative dim × n grid.
//!
//! Per ADR-157, this number is the **specification kernel** baseline
//! that future SIMD / GPU kernel crates compare themselves against on
//! identical inputs. Promotion past experimental requires either
//! ≥ 2× lower p95 or ≥ 30% lower cost at identical recall@10.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

use rulake::kernel::{CpuNaiveKernel, VectorKernel};

fn make_inputs(dim: usize, n: usize, seed: u64) -> (Vec<f32>, Vec<Vec<f32>>) {
    // Tiny LCG so the bench fixture is reproducible without rand.
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
    let kernel = CpuNaiveKernel;
    let top_k = 10;
    let dims = [64usize, 384, 1024];
    let ns = [1024usize, 16_384];

    let mut group = c.benchmark_group("kernel_l2");
    for &dim in &dims {
        for &n in &ns {
            let (query, candidates) = make_inputs(dim, n, 0xA5A5_5A5A);
            // Throughput in candidate-vectors-per-second — easier to
            // compare across dim than wall time alone.
            group.throughput(Throughput::Elements(n as u64));
            group.bench_with_input(
                BenchmarkId::from_parameter(format!("dim={dim}/n={n}")),
                &(query, candidates),
                |b, (q, cs)| {
                    b.iter(|| {
                        let r = kernel.l2_distance_one(black_box(q), black_box(cs), top_k);
                        black_box(r);
                    });
                },
            );
        }
    }
    group.finish();
}

criterion_group!(benches, bench_l2);
criterion_main!(benches);
