//! Criterion bench for `RvdnaT1Backend::pull_vectors` — the warm-tier
//! mmap'd path. Measures the cost of decoding a section of the mmap'd
//! `.rvdna` v2 file as `&[f32]` (via `bytemuck`) and copying it into a
//! `PulledBatch`.
//!
//! Sizes: 256 / 1024 vectors at dim=384 (the all-MiniLM-L6-v2 footprint
//! we use elsewhere in the workspace). Expected: T1 should be in the
//! same order of magnitude as T0 — mmap'd reads hit the page cache
//! after first access, and the witness-anchored ruLake cache means
//! most production calls are cache hits anyway. The bench-vs-T0 comp
//! is informational, not a regression gate.
//!
//! Run with:  cargo bench --bench t1_pull_vectors -- --quick

use std::io::Write;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rulake::backend::BackendAdapter;
use ruvector_rulake_rvdna::{RvdnaCollection, RvdnaT0Backend, RvdnaT1Backend};

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

fn bench_t1_vs_t0_pull_vectors(c: &mut Criterion) {
    let mut group = c.benchmark_group("rvdna_pull_vectors_t1_vs_t0");
    group.measurement_time(Duration::from_secs(5));

    let dim = 384usize;
    for &n in &[256usize, 1024] {
        let payload_bytes = (n * dim * std::mem::size_of::<f32>()) as u64;
        group.throughput(Throughput::Bytes(payload_bytes));

        // ── T0: RAM-resident ──
        let t0 = RvdnaT0Backend::new("t0-bench");
        let flat = deterministic_vectors(0xc0ffee ^ (n as u64), n, dim);
        let ids: Vec<u64> = (0..n as u64).collect();
        let vectors: Vec<Vec<f32>> = (0..n)
            .map(|i| flat[i * dim..(i + 1) * dim].to_vec())
            .collect();
        let name = format!("col-{n}-{dim}");
        t0.put_collection(
            &name,
            RvdnaCollection { ids, vectors, dim, generation: 1 },
        );

        let t0_id = BenchmarkId::new(format!("t0-dim{dim}"), n);
        group.bench_with_input(t0_id, &name, |b, name| {
            b.iter(|| {
                let batch = t0.pull_vectors(name).expect("t0 pull");
                criterion::black_box(batch);
            });
        });

        // ── T1: mmap'd file ──
        let file = write_fixture(0xc0ffee ^ (n as u64), n, dim);
        let t1 = RvdnaT1Backend::new("t1-bench");
        t1.register_file(&name, file.path(), dim, 1, 0, 0, n, false)
            .expect("t1 register");
        // Warm the page cache so we measure steady-state, not first-
        // touch fault cost.
        let _ = t1.pull_vectors(&name).expect("t1 warmup");

        let t1_id = BenchmarkId::new(format!("t1-dim{dim}"), n);
        group.bench_with_input(t1_id, &name, |b, name| {
            b.iter(|| {
                let batch = t1.pull_vectors(name).expect("t1 pull");
                criterion::black_box(batch);
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_t1_vs_t0_pull_vectors);
criterion_main!(benches);
