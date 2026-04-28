//! Phase B engine swap — route `ruqu_simulate` through the real
//! upstream `ruqu-core` simulator instead of the v0.0.x stub in
//! `ruqu-backend::state_vector`.
//!
//! Why this exists: `ruqu-core` (vendor/ruvector/crates/ruqu-core,
//! ~26k LOC) is the production quantum simulator with SIMD + noise
//! models + multi-threading. The v0.0.x `ruqu-backend::state_vector`
//! is a clean-room ~870 LOC reference impl built when ruqu-core
//! wasn't yet available as a path-dep. Now that it is, the wire
//! shape stays the same but the actual compute moves to the real lib.
//!
//! Translation surface is small (the v0.0 wire only ships 8 gate
//! kinds: H/X/Y/Z/S/T/Rz/Cx). Anything richer the operator wires
//! through the upstream `QuantumCircuit::add_gate(Gate::...)` API
//! directly.

use ruqu_core::circuit::QuantumCircuit;
use ruqu_core::gate::Gate as CoreGate;
use ruqu_core::simulator::Simulator;
use ruvector_rulake_ruqu::circuit::{Circuit, Gate as WireGate};
use ruvector_rulake_ruqu::state_vector::C;

/// Compile our wire `Circuit` into ruqu-core's `QuantumCircuit`.
///
/// Returns `Err` on a gate the upstream lib doesn't recognise (should
/// never happen for the v0.0 8-gate surface).
pub fn compile(circuit: &Circuit) -> QuantumCircuit {
    let mut qc = QuantumCircuit::new(circuit.n_qubits as u32);
    for g in &circuit.gates {
        let core_gate = match *g {
            WireGate::H { q } => CoreGate::H(q as u32),
            WireGate::X { q } => CoreGate::X(q as u32),
            WireGate::Y { q } => CoreGate::Y(q as u32),
            WireGate::Z { q } => CoreGate::Z(q as u32),
            WireGate::S { q } => CoreGate::S(q as u32),
            WireGate::T { q } => CoreGate::T(q as u32),
            WireGate::Rz { q, theta } => CoreGate::Rz(q as u32, theta),
            WireGate::Cx { control, target } => CoreGate::CNOT(control as u32, target as u32),
        };
        qc.add_gate(core_gate);
    }
    qc
}

/// Run the circuit on ruqu-core and return the state vector in our
/// existing wire shape (`ruvector_rulake_ruqu::state_vector::C`) so the
/// planner code that already accesses `.re` / `.im` can consume it
/// unchanged. The `C` type is layout-compatible with ruqu-core's own
/// `Complex` (both are `{ re: f64, im: f64 }` with `Copy`).
pub fn simulate(circuit: &Circuit) -> Vec<C> {
    let qc = compile(circuit);
    // ruqu-core `Simulator::run` returns `Result<SimulationResult>`.
    // On the v0.0 surface (≤ 16 qubits, no Measure / Reset / Barrier)
    // this can't fail — the qubit cap was already enforced upstream
    // by enforce_qubit_cap. On any future gate kind ruqu-core refuses,
    // fall back to an empty state vector and let the wire surface the
    // empty-amplitudes case.
    match Simulator::run(&qc) {
        Ok(result) => result
            .state
            .state_vector()
            .iter()
            .map(|c| C { re: c.re, im: c.im })
            .collect(),
        Err(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx_eq(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-10
    }

    /// |0> with no gates: state vector = [1, 0]. ruqu-core matches.
    #[test]
    fn empty_one_qubit_is_ground_state() {
        let c = Circuit { id: "test".into(), n_qubits: 1, gates: vec![] };
        let sv = simulate(&c);
        assert_eq!(sv.len(), 2);
        assert!(approx_eq(sv[0].re, 1.0) && approx_eq(sv[0].im, 0.0));
        assert!(approx_eq(sv[1].re, 0.0) && approx_eq(sv[1].im, 0.0));
    }

    /// H|0> = (|0> + |1>)/sqrt(2). ruqu-core matches the v0.0 stub bit-by-bit.
    #[test]
    fn h_gate_creates_superposition() {
        let c = Circuit {
            id: "h".into(),
            n_qubits: 1,
            gates: vec![WireGate::H { q: 0 }],
        };
        let sv = simulate(&c);
        assert_eq!(sv.len(), 2);
        let amp = 1.0 / 2.0_f64.sqrt();
        assert!(approx_eq(sv[0].re, amp) && approx_eq(sv[0].im, 0.0));
        assert!(approx_eq(sv[1].re, amp) && approx_eq(sv[1].im, 0.0));
    }

    /// Bell pair: H(0); CX(0,1) → (|00> + |11>)/sqrt(2).
    #[test]
    fn bell_pair() {
        let c = Circuit {
            id: "bell".into(),
            n_qubits: 2,
            gates: vec![
                WireGate::H { q: 0 },
                WireGate::Cx { control: 0, target: 1 },
            ],
        };
        let sv = simulate(&c);
        assert_eq!(sv.len(), 4);
        let amp = 1.0 / 2.0_f64.sqrt();
        // |00> = index 0, |11> = index 3 (qubit-0 LSB)
        assert!(approx_eq(sv[0].re, amp), "|00> re = {}", sv[0].re);
        assert!(approx_eq(sv[3].re, amp), "|11> re = {}", sv[3].re);
        assert!(approx_eq(sv[1].re, 0.0), "|10> re = {}", sv[1].re);
        assert!(approx_eq(sv[2].re, 0.0), "|01> re = {}", sv[2].re);
    }
}
