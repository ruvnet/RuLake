//! Bench `tensor_network::simulate` (MPS) on circuit shapes that
//! play to the MPS strength: low-entanglement / nearest-neighbour
//! 1-D layouts. We sweep `n_qubits ∈ {16, 32, 64}` and
//! `bond_dim_cap ∈ {2, 8, 32}`. The cross-product (9 cells) shows the
//! polynomial-in-n / cubic-in-χ scaling explicitly.
//!
//! Headline: at n=64 / χ=8 the simulator finishes in well under a
//! second, while the StateVector backend can't even allocate a state
//! at n>16 (1 MiB at f64). We also bench a small Bell-pair-style
//! GHZ-chain (`H(0); CX(i, i+1) for i in 0..n-1`) — the canonical
//! "low-entanglement" stress test — and an interleaved nearest-
//! neighbour brick-wall layer with `T` gates so the bench exercises
//! the non-Clifford path that Stabilizer can't run.
//!
//! `--quick` mode is enough to read the headline numbers; the script
//! at `cargo bench --bench tensor_network -- --quick` runs in well
//! under a minute.

use std::time::Duration;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use ruvector_rulake_ruqu::{tensor_network, Circuit, Gate};

/// GHZ-chain — `H(0); CX(0,1); CX(1,2); …; CX(n-2, n-1)`. Entanglement
/// is logarithmic in n: bond dimension 2 captures the state exactly,
/// any larger χ is wasted. Good baseline for the "MPS is cheap" claim.
fn ghz_chain(n_qubits: u8) -> Circuit {
    let mut c = Circuit::new(format!("ghz-{n_qubits}"), n_qubits);
    c.push(Gate::H { q: 0 });
    for i in 0..n_qubits.saturating_sub(1) {
        c.push(Gate::Cx {
            control: i,
            target: i + 1,
        });
    }
    c
}

/// Brick-wall layer with non-Clifford T gates. Pairs are
/// `(0,1)(2,3)…` then `(1,2)(3,4)…`. Each "layer" is one even +
/// one odd pass. T gates are sprinkled on every qubit between
/// passes so the circuit isn't simulable by Stabilizer.
fn brickwall_with_t(n_qubits: u8, layers: usize) -> Circuit {
    let mut c = Circuit::new(format!("brick-t-{n_qubits}q-{layers}L"), n_qubits);
    for q in 0..n_qubits {
        c.push(Gate::H { q });
    }
    for _ in 0..layers {
        // Even pairs.
        let mut q = 0u8;
        while q + 1 < n_qubits {
            c.push(Gate::Cx {
                control: q,
                target: q + 1,
            });
            q += 2;
        }
        for q in 0..n_qubits {
            c.push(Gate::T { q });
        }
        // Odd pairs.
        let mut q = 1u8;
        while q + 1 < n_qubits {
            c.push(Gate::Cx {
                control: q,
                target: q + 1,
            });
            q += 2;
        }
        for q in 0..n_qubits {
            c.push(Gate::T { q });
        }
    }
    c
}

fn bench_ghz_chain(c: &mut Criterion) {
    let mut group = c.benchmark_group("tensor_network_ghz");
    group.measurement_time(Duration::from_secs(5));

    for &n in &[16u8, 32, 64] {
        for &chi in &[2usize, 8, 32] {
            let circuit = ghz_chain(n);
            // Throughput in "site-tensor entries touched per second" —
            // approximated as n * χ² * gate_count.
            let cells = (n as u64) * (chi as u64) * (chi as u64) * circuit.gates.len() as u64;
            group.throughput(Throughput::Elements(cells.max(1)));

            let id = BenchmarkId::new("ghz_chain", format!("n{n}_chi{chi}"));
            group.bench_with_input(id, &(circuit, chi), |b, (circuit, chi)| {
                b.iter(|| {
                    let mps =
                        tensor_network::simulate(circuit, *chi).expect("MPS sim must succeed");
                    criterion::black_box(mps);
                });
            });
        }
    }
    group.finish();
}

fn bench_brickwall_with_t(c: &mut Criterion) {
    let mut group = c.benchmark_group("tensor_network_brickwall_t");
    group.measurement_time(Duration::from_secs(5));
    let layers = 2;

    for &n in &[16u8, 32, 64] {
        for &chi in &[2usize, 8, 32] {
            let circuit = brickwall_with_t(n, layers);
            let cells = (n as u64) * (chi as u64) * (chi as u64) * circuit.gates.len() as u64;
            group.throughput(Throughput::Elements(cells.max(1)));

            let id = BenchmarkId::new("brickwall_t", format!("n{n}_chi{chi}_L{layers}"));
            group.bench_with_input(id, &(circuit, chi), |b, (circuit, chi)| {
                b.iter(|| {
                    let mps =
                        tensor_network::simulate(circuit, *chi).expect("MPS sim must succeed");
                    criterion::black_box(mps);
                });
            });
        }
    }
    group.finish();
}

criterion_group!(benches, bench_ghz_chain, bench_brickwall_with_t);
criterion_main!(benches);
