//! Criterion bench for `RvdnaT2Backend::pull_vectors` — the cold-lazy
//! tier (`seek+read` plus per-collection LRU window). Compares three
//! configurations head-to-head:
//!
//!   - **T2 cold**: drop and re-register the backend before every
//!     iteration so the LRU is empty; every pull pays the full
//!     seek+read+decode cost. Models the very first query against a
//!     freshly opened `.rvdna` file.
//!   - **T2 warm**: register once, prime once, then loop. Every pull
//!     after the prime is LRU-resident — no syscall, no decode.
//!   - **T1 mmap**: the v0.1 warm-tier baseline. Same byte-equal
//!     fixture, served via the mmap'd `bytemuck` cast path.
//!
//! Sizes: 256 / 1024 vectors at dim=384 (matches `t1_pull_vectors.rs`
//! so the two benches plot on the same axes).
//!
//! Expected ranges per the v0.2 ship table in
//! `docs/research/rvdna/integration-with-rulake.md`:
//!   - T2 cold ≈ 5–10× slower than T1 mmap (one big `pread` plus a
//!     full per-row decode)
//!   - T2 warm ≈ comparable to T1 mmap (both serve from already-resident
//!     memory; T2 walks an `LruCache`, T1 walks the page cache)
//!
//! Run with:  cargo bench --bench t2_pull_vectors -- --quick

use std::io::Write;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rulake::backend::BackendAdapter;
use ruvector_rulake_rvdna::{RvdnaT1Backend, RvdnaT2Backend};

fn deterministic_vectors(seed: u64, n: usize, dim: usize) -> Vec<f32> {
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
    (0..n * dim).map(|_| next() * 2.0 - 1.0).collect()
}

fn write_fixture(seed: u64, n: usize, dim: usize) -> tempfile::NamedTempFile {
    let flat = deterministic_vectors(seed, n, dim);
    let mut f = tempfile::NamedTempFile::new().expect("tempfile");
    let bytes: &[u8] = bytemuck::cast_slice(&flat);
    f.write_all(bytes).expect("write");
    f.flush().expect("flush");
    f
}

fn bench_t2(c: &mut Criterion) {
    let mut group = c.benchmark_group("rvdna_pull_vectors_t2_vs_t1");
    group.measurement_time(Duration::from_secs(5));

    let dim = 384usize;
    for &n in &[256usize, 1024] {
        let payload_bytes = (n * dim * std::mem::size_of::<f32>()) as u64;
        group.throughput(Throughput::Bytes(payload_bytes));
        let name = format!("col-{n}-{dim}");
        let file = write_fixture(0xc0ffee ^ (n as u64), n, dim);

        // ── T2 cold: re-register before every iter so the LRU is
        // empty. We use `iter_batched` so the rebuild cost stays out
        // of the measured region.
        let cold_id = BenchmarkId::new(format!("t2-cold-dim{dim}"), n);
        group.bench_with_input(cold_id, &name, |b, name| {
            b.iter_batched(
                || {
                    let t2 = RvdnaT2Backend::new("t2-bench-cold");
                    t2.register_file(name, file.path(), dim, 1, Vec::new(), 0, n)
                        .expect("t2 register");
                    t2
                },
                |t2| {
                    let batch = t2.pull_vectors(name).expect("t2 cold pull");
                    criterion::black_box(batch);
                },
                criterion::BatchSize::SmallInput,
            );
        });

        // ── T2 warm: prime once, then loop. Every pull after the
        // prime is LRU-resident.
        let t2_warm = RvdnaT2Backend::new("t2-bench-warm");
        t2_warm
            .register_file(&name, file.path(), dim, 1, Vec::new(), 0, n)
            .expect("t2 register");
        let _ = t2_warm.pull_vectors(&name).expect("t2 warmup");
        let warm_id = BenchmarkId::new(format!("t2-warm-dim{dim}"), n);
        group.bench_with_input(warm_id, &name, |b, name| {
            b.iter(|| {
                let batch = t2_warm.pull_vectors(name).expect("t2 warm pull");
                criterion::black_box(batch);
            });
        });

        // ── T1 mmap baseline: same fixture, mmap'd warm tier.
        let t1 = RvdnaT1Backend::new("t1-bench");
        t1.register_file(&name, file.path(), dim, 1, 0, 0, n, false)
            .expect("t1 register");
        let _ = t1.pull_vectors(&name).expect("t1 warmup");
        let t1_id = BenchmarkId::new(format!("t1-mmap-dim{dim}"), n);
        group.bench_with_input(t1_id, &name, |b, name| {
            b.iter(|| {
                let batch = t1.pull_vectors(name).expect("t1 pull");
                criterion::black_box(batch);
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_t2);
criterion_main!(benches);
