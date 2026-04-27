//! End-to-end bench through the BackendAdapter:
//!   * `execute`   — simulate + store + derive witness bundle.
//!   * `pull_vectors` — clone-out cost the cache pays on a miss.
//!
//! Sizes mirror benches/simulate.rs (n_qubits ∈ {6, 10, 14}) so the
//! delta from raw `simulate` to `execute` shows the BackendAdapter
//! overhead (down-cast to f32 [re,im] pairs + RwLock insert + witness
//! derivation).

use std::time::Duration;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rulake::backend::BackendAdapter;
use ruvector_rulake_ruqu::{Circuit, Gate, RuquStateVectorBackend};

fn qft_like_circuit(n_qubits: u8, layers: usize) -> Circuit {
    let mut c = Circuit::new(format!("qftlike-{n_qubits}q-{layers}L"), n_qubits);
    for _ in 0..layers {
        for q in 0..n_qubits {
            c.push(Gate::H { q });
        }
        for q in 0..n_qubits {
            let target = (q + 1) % n_qubits;
            c.push(Gate::Cx { control: q, target });
        }
    }
    c
}

fn bench_execute(c: &mut Criterion) {
    let mut group = c.benchmark_group("ruqu_execute");
    group.measurement_time(Duration::from_secs(8));
    let layers = 4;

    for &n in &[6u8, 10, 14] {
        let circuit = qft_like_circuit(n, layers);
        let dim = 1u64 << n;
        // Bytes serialised into PulledBatch.vectors: dim pairs of f32.
        let payload_bytes = dim * 2 * std::mem::size_of::<f32>() as u64;
        group.throughput(Throughput::Bytes(payload_bytes));

        let id = BenchmarkId::new("execute", format!("n{n}_L{layers}"));
        group.bench_with_input(id, &circuit, |b, circuit| {
            // Fresh backend per iteration so we measure the full
            // simulate→insert→witness path (BTreeMap insert cost is
            // O(log k) on the existing key set; keep k=1 here).
            b.iter_with_setup(
                || RuquStateVectorBackend::new("ruqu-bench"),
                |be| {
                    let bundle = be.execute("run", circuit).expect("execute");
                    criterion::black_box(bundle);
                },
            );
        });
    }
    group.finish();
}

fn bench_pull_after_exec(c: &mut Criterion) {
    let mut group = c.benchmark_group("ruqu_pull_vectors");
    group.measurement_time(Duration::from_secs(6));
    let layers = 4;

    for &n in &[6u8, 10, 14] {
        let be = RuquStateVectorBackend::new("ruqu-bench");
        let circuit = qft_like_circuit(n, layers);
        be.execute("run", &circuit).expect("execute");

        let dim = 1u64 << n;
        let payload_bytes = dim * 2 * std::mem::size_of::<f32>() as u64;
        group.throughput(Throughput::Bytes(payload_bytes));

        let id = BenchmarkId::new("pull", format!("n{n}"));
        group.bench_with_input(id, &be, |b, be| {
            b.iter(|| {
                let batch = be.pull_vectors("run").expect("pull");
                criterion::black_box(batch);
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_execute, bench_pull_after_exec);
criterion_main!(benches);
