//! Criterion bench for the load-bearing operation on `RvdnaT0Backend`:
//! `pull_vectors`. v0.0 is RAM-resident, so this measures the
//! BackendAdapter's clone-out cost (Vec<Vec<f32>> + Vec<u64>) which is
//! what the cache pays on a miss.
//!
//! Sizes: 64 / 256 / 1024 vectors at dim ∈ {128, 384}. dim=384 matches
//! the all-MiniLM-L6-v2 footprint we use elsewhere.

use std::time::Duration;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rulake::backend::BackendAdapter;
use ruvector_rulake_rvdna::{RvdnaCollection, RvdnaT0Backend};

fn fixture_collection(seed: u64, n: usize, dim: usize) -> RvdnaCollection {
    // Deterministic mulberry32-equivalent (mirrors tests/smoke.rs).
    let mut s = seed;
    let mut next = || {
        s = s.wrapping_add(0x9e3779b97f4a7c15);
        let mut x = s;
        x ^= x >> 30;
        x = x.wrapping_mul(0xbf58476d1ce4e5b9);
        x ^= x >> 27;
        x = x.wrapping_mul(0x94d049bb133111eb);
        x ^= x >> 31;
        x as f32 / u64::MAX as f32
    };
    let ids: Vec<u64> = (0..n as u64).collect();
    let vectors: Vec<Vec<f32>> = (0..n)
        .map(|_| (0..dim).map(|_| next() * 2.0 - 1.0).collect())
        .collect();
    RvdnaCollection {
        ids,
        vectors,
        dim,
        generation: 1,
    }
}

fn bench_pull_vectors(c: &mut Criterion) {
    let mut group = c.benchmark_group("rvdna_pull_vectors");
    group.measurement_time(Duration::from_secs(6));

    for &dim in &[128usize, 384] {
        for &n in &[64usize, 256, 1024] {
            let be = RvdnaT0Backend::new("rvdna-bench");
            let name = format!("col-{n}-{dim}");
            be.put_collection(&name, fixture_collection(0xc0ffee ^ (n as u64), n, dim));

            // Throughput in bytes — `n * dim * 4` for the f32 payload
            // (ids contribute another `n * 8` but the vectors dominate).
            let payload_bytes = (n * dim * std::mem::size_of::<f32>()) as u64;
            group.throughput(Throughput::Bytes(payload_bytes));

            let id = BenchmarkId::new(format!("dim{dim}"), n);
            group.bench_with_input(id, &name, |b, name| {
                b.iter(|| {
                    let batch = be.pull_vectors(name).expect("pull");
                    criterion::black_box(batch);
                });
            });
        }
    }
    group.finish();
}

criterion_group!(benches, bench_pull_vectors);
criterion_main!(benches);
