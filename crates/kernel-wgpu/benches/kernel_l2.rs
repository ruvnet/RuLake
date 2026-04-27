//! Criterion bench for [`WgpuKernel::l2_distance_one`].
//!
//! Mirrors the root crate's `benches/kernel_l2.rs` grid (dim=384,
//! n=16384) so the GPU number can be placed alongside the CpuNaive
//! baseline. Note: GPU kernels include host↔device buffer transfer
//! overhead, so for small batches the wgpu number is dominated by
//! submit + map latency rather than raw compute throughput.
//!
//! Falls back to a no-op group if no GPU adapter is available, so
//! `cargo bench` doesn't crash on headless CI.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

use rulake::kernel::{CpuNaiveKernel, VectorKernel};
use ruvector_rulake_kernel_wgpu::WgpuKernel;

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
    let dims = [384usize];
    let ns = [16_384usize];

    let gpu = match WgpuKernel::new_blocking() {
        Ok(k) => Some(k),
        Err(e) => {
            eprintln!("note: skipping wgpu bench arm — {e}");
            None
        }
    };

    let mut group = c.benchmark_group("kernel_l2");
    for &dim in &dims {
        for &n in &ns {
            let (query, candidates) = make_inputs(dim, n, 0xA5A5_5A5A);
            group.throughput(Throughput::Elements(n as u64));

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

            if let Some(k) = gpu.as_ref() {
                group.bench_with_input(
                    BenchmarkId::new("wgpu", format!("dim={dim}/n={n}")),
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
