//! Bench the load-bearing kernel: `state_vector::simulate`. Uses a
//! "QFT-like" shape — alternating layers of Hadamards on every qubit
//! and CNOT rings — for sizes n_qubits ∈ {6, 10, 14}. State vector
//! length is `1 << n`, so 14 → 16,384 amplitudes; close to where the
//! v0.0 RAM-only design starts to feel its bound.

use std::time::Duration;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use ruvector_rulake_ruqu::{state_vector, Circuit, Gate};

/// Build an alternating-H + CX-ring circuit with `layers` repetitions.
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

fn bench_simulate(c: &mut Criterion) {
    let mut group = c.benchmark_group("ruqu_simulate");
    group.measurement_time(Duration::from_secs(8));
    // 4 layers keeps the gate count visible across all sizes.
    let layers = 4;

    for &n in &[6u8, 10, 14] {
        let circuit = qft_like_circuit(n, layers);
        // Throughput in "amplitudes touched per second" — approximated
        // as state-dim * gate-count (each gate sweeps the whole sv).
        let dim = 1u64 << n;
        let elements = dim * circuit.gates.len() as u64;
        group.throughput(Throughput::Elements(elements));

        let id = BenchmarkId::new("qft_like", format!("n{n}_L{layers}"));
        group.bench_with_input(id, &circuit, |b, circuit| {
            b.iter(|| {
                let sv = state_vector::simulate(circuit);
                criterion::black_box(sv);
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_simulate);
criterion_main!(benches);
