//! Bench `stabilizer::simulate` against `state_vector::simulate` on
//! the same all-Clifford "QFT-like" circuit shape (alternating
//! Hadamards + CX rings). Stabilizer scales polynomially in `n` and
//! handles the v0.1 cap (n=32) cleanly; StateVector caps at n=14 in
//! this bench (its v0.0 cap is n=16, but n=14 is the largest size
//! that finishes in a quick run).
//!
//! Headline expectation: at n=14 Stabilizer should be dramatically
//! faster than StateVector, and StateVector simply can't run at the
//! n=32 sizes Stabilizer takes in stride. v0.2 will lift the v0.1
//! cap so the n=64 / n=128 case lands in a follow-up bench.

use std::time::Duration;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use ruvector_rulake_ruqu::{stabilizer, state_vector, Circuit, Gate};

fn clifford_qft_like(n_qubits: u8, layers: usize) -> Circuit {
    let mut c = Circuit::new(format!("clifford-qftlike-{n_qubits}q-{layers}L"), n_qubits);
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

fn bench_stabilizer_simulate(c: &mut Criterion) {
    let mut group = c.benchmark_group("stabilizer_simulate");
    group.measurement_time(Duration::from_secs(6));
    let layers = 4;

    // The brief asks for {16, 32, 64}; v0.1's `MAX_QUBITS_V0_1 = 32`
    // caps Stabilizer at 32 — n=64 lands in v0.2 once the cap lifts.
    // Bench at the two sizes the cap allows so this stays useful at
    // ship time. The largest size doubles to 24 to give the comparator
    // a third intermediate point.
    for &n in &[16u8, 24, stabilizer::MAX_QUBITS_V0_1] {
        let circuit = clifford_qft_like(n, layers);
        // Throughput in "tableau cells touched per second" — each
        // gate sweeps `2n + 1` rows × O(n) bits.
        let cells = (2 * n as u64 + 1) * n as u64 * circuit.gates.len() as u64;
        group.throughput(Throughput::Elements(cells));

        let id = BenchmarkId::new("clifford_qft_like", format!("n{n}_L{layers}"));
        group.bench_with_input(id, &circuit, |b, circuit| {
            b.iter(|| {
                let s = stabilizer::simulate(circuit).expect("Clifford circuit must succeed");
                criterion::black_box(s);
            });
        });
    }
    group.finish();
}

fn bench_statevector_for_compare(c: &mut Criterion) {
    let mut group = c.benchmark_group("stabilizer_vs_state_vector");
    group.measurement_time(Duration::from_secs(6));
    let layers = 4;

    // Same circuit at n=14 — the StateVector cap for a "still
    // tractable in a quick bench" comparison. Stabilizer at n=14
    // alongside lets the eye compare apples-to-apples.
    let n = 14u8;
    let circuit = clifford_qft_like(n, layers);
    let dim = 1u64 << n;
    let elements = dim * circuit.gates.len() as u64;
    group.throughput(Throughput::Elements(elements));

    let id = BenchmarkId::new("state_vector", format!("n{n}_L{layers}"));
    group.bench_with_input(id.clone(), &circuit, |b, circuit| {
        b.iter(|| {
            let sv = state_vector::simulate(circuit);
            criterion::black_box(sv);
        });
    });

    let id_stab = BenchmarkId::new("stabilizer", format!("n{n}_L{layers}"));
    group.bench_with_input(id_stab, &circuit, |b, circuit| {
        b.iter(|| {
            let s = stabilizer::simulate(circuit).expect("Clifford circuit must succeed");
            criterion::black_box(s);
        });
    });
    group.finish();
}

criterion_group!(benches, bench_stabilizer_simulate, bench_statevector_for_compare);
criterion_main!(benches);
