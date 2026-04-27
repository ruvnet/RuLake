//! Cold-vs-hot cache bench. Registers an `RvdnaT0Backend` with a
//! `RuLake`, primes it once (cold = miss), then re-runs `search_one`
//! repeatedly (hot = cache hit). The ratio shows the cache earning
//! its keep — 50–500x is the expected band per ADR-007 §6 v0.0 notes.
//!
//! The cold sample is deliberately measured separately (one-shot,
//! since priming only fires once per backend+collection generation).

use std::sync::Arc;
use std::time::{Duration, Instant};

use criterion::{criterion_group, criterion_main, Criterion};
use rulake::backend::BackendAdapter;
use rulake::cache::Consistency;
use rulake::RuLake;
use ruvector_rulake_rvdna::{RvdnaCollection, RvdnaT0Backend};

fn fixture_collection(seed: u64, n: usize, dim: usize) -> RvdnaCollection {
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
    RvdnaCollection { ids, vectors, dim, generation: 1 }
}

fn build_lake_with_collection(n: usize, dim: usize) -> RuLake {
    let be: Arc<dyn BackendAdapter> = Arc::new({
        let b = RvdnaT0Backend::new("rvdna-bench");
        b.put_collection("col", fixture_collection(0xc0ffee, n, dim));
        b
    });
    let lake = RuLake::new(20, 42)
        .with_consistency(Consistency::Eventual { ttl_ms: 60_000 });
    lake.register_backend(be).unwrap();
    lake
}

fn bench_cold_vs_hot(c: &mut Criterion) {
    // Pick the realistic "embedding-class" middle: 256 vectors x dim 384.
    let n = 256usize;
    let dim = 384usize;
    let q = vec![0.0_f32; dim];

    // ---- cold path: one-shot, measured by hand. Each Criterion
    // iteration must rebuild a fresh lake so the cache is empty; the
    // setup cost is excluded via iter_with_setup.
    let mut group_cold = c.benchmark_group("rvdna_cache_cold");
    group_cold.sample_size(20); // build-and-prime is heavy; keep it short
    group_cold.measurement_time(Duration::from_secs(8));
    group_cold.bench_function("search_one_first_call", |b| {
        b.iter_with_setup(
            || build_lake_with_collection(n, dim),
            |lake| {
                let hits = lake
                    .search_one("rvdna-bench", "col", &q, 5)
                    .expect("search");
                criterion::black_box(hits);
            },
        );
    });
    group_cold.finish();

    // ---- hot path: shared lake, cache primed once.
    let lake = build_lake_with_collection(n, dim);
    let _warm = lake.search_one("rvdna-bench", "col", &q, 5).unwrap();

    let mut group_hot = c.benchmark_group("rvdna_cache_hot");
    group_hot.measurement_time(Duration::from_secs(6));
    group_hot.bench_function("search_one_repeat", |b| {
        b.iter(|| {
            let hits = lake
                .search_one("rvdna-bench", "col", &q, 5)
                .expect("search");
            criterion::black_box(hits);
        });
    });
    group_hot.finish();

    // ---- cold/hot ratio: print a coarse manual number to stdout so
    // it lands in the criterion log even though it isn't a Bencher loop.
    // We measure 5 cold calls (each with a fresh lake) and 10_000 hot
    // calls; the per-call ratio is what we'd quote in the report.
    let cold_iters = 5;
    let hot_iters = 10_000;

    let mut cold_total = Duration::ZERO;
    for _ in 0..cold_iters {
        let lake = build_lake_with_collection(n, dim);
        let t0 = Instant::now();
        let _ = lake.search_one("rvdna-bench", "col", &q, 5).unwrap();
        cold_total += t0.elapsed();
    }
    let cold_avg = cold_total / cold_iters as u32;

    let lake = build_lake_with_collection(n, dim);
    let _ = lake.search_one("rvdna-bench", "col", &q, 5).unwrap(); // prime
    let t0 = Instant::now();
    for _ in 0..hot_iters {
        let _ = lake.search_one("rvdna-bench", "col", &q, 5).unwrap();
    }
    let hot_total = t0.elapsed();
    let hot_avg = hot_total / hot_iters as u32;

    let ratio = cold_avg.as_secs_f64() / hot_avg.as_secs_f64();
    println!(
        "rvdna_cache_ratio  cold={:?}  hot={:?}  cold/hot={:.1}x",
        cold_avg, hot_avg, ratio
    );
}

criterion_group!(benches, bench_cold_vs_hot);
criterion_main!(benches);
