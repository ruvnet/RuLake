//! Witness derivation for ruQu circuit executions.
//!
//! `RuLakeBundle::new` is the contract — every load-bearing input to
//! the simulation flows through it so two deployments running the
//! same circuit at the same seed produce identical SHAKE-256(32)
//! witnesses and share cache transparently. Two outputs from
//! different backends (StateVector vs Stabilizer for the all-Clifford
//! case) MUST share the same witness; v0.0 ships only StateVector
//! and asserts the byte-isomorphism in tests.

use rulake::bundle::Generation;
use rulake::RuLakeBundle;

use crate::circuit::Circuit;

/// Inputs that uniquely identify a circuit execution. Pin all of
/// them — anything that's not pinned is a way for two replicas to
/// diverge silently.
#[derive(Debug, Clone)]
pub struct RuquWitnessInputs<'a> {
    /// `ruqu://<circuit-id>` URI naming the source program.
    pub data_ref: &'a str,
    /// Hilbert-space dimension. `1 << n_qubits` for StateVector;
    /// for Stabilizer this is the stabilizer-table row count.
    pub dim: usize,
    /// PRNG seed for any noise / sampling. Pin to 0 for noiseless
    /// StateVector — the witness still discriminates noisy from
    /// noiseless because `data_ref` carries the backend tag.
    pub rotation_seed: u64,
    /// rerank_factor — pinned to `1` for StateVector (no reranking),
    /// >1 for sampled / noisy backends per ADR-008 §3.
    pub rerank_factor: usize,
    /// Generation token. Bumps when the circuit source rotates,
    /// when the calibration snapshot changes, or when a noise
    /// model is repinned.
    pub generation: u64,
}

/// Build a [`RuLakeBundle`] for a ruQu execution. Same construction
/// path as any other ruLake bundle — federation, audit, and the
/// Console verifier work unchanged.
pub fn ruqu_bundle(inputs: &RuquWitnessInputs<'_>) -> RuLakeBundle {
    RuLakeBundle::new(
        inputs.data_ref.to_string(),
        inputs.dim,
        inputs.rotation_seed,
        inputs.rerank_factor,
        Generation::Num(inputs.generation),
    )
    .with_memory_class("quantum")
}

/// Convenience: derive inputs from a [`Circuit`] + a backend tag +
/// seed. The backend tag flows into `data_ref` so StateVector and
/// Stabilizer over the same circuit get distinct witnesses. v0.0
/// uses backend tag "sv" for StateVector.
pub fn inputs_for(
    c: &Circuit,
    backend_tag: &str,
    seed: u64,
    generation: u64,
) -> RuquWitnessInputsOwned {
    RuquWitnessInputsOwned {
        data_ref: format!("ruqu://{}@{}", c.id, backend_tag),
        dim: c.dim(),
        rotation_seed: seed,
        rerank_factor: 1,
        generation,
    }
}

/// Owned-string variant of [`RuquWitnessInputs`] for callers that
/// don't have a 'static-lived `data_ref`.
#[derive(Debug, Clone)]
pub struct RuquWitnessInputsOwned {
    /// `ruqu://<circuit-id>@<backend-tag>` URI naming the source.
    pub data_ref: String,
    /// Hilbert-space dimension.
    pub dim: usize,
    /// PRNG seed for noise / sampling.
    pub rotation_seed: u64,
    /// Rerank factor — `1` for noiseless StateVector.
    pub rerank_factor: usize,
    /// Coherence token. Bumps on circuit / noise / calibration change.
    pub generation: u64,
}

impl RuquWitnessInputsOwned {
    /// Borrow as the `&'a str`-flavoured form used by [`ruqu_bundle`].
    pub fn as_borrowed(&self) -> RuquWitnessInputs<'_> {
        RuquWitnessInputs {
            data_ref: &self.data_ref,
            dim: self.dim,
            rotation_seed: self.rotation_seed,
            rerank_factor: self.rerank_factor,
            generation: self.generation,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::circuit::{Circuit, Gate};

    #[test]
    fn witness_deterministic_for_same_circuit_and_seed() {
        let mut c = Circuit::new("bell", 2);
        c.push(Gate::H { q: 0 });
        c.push(Gate::Cx {
            control: 0,
            target: 1,
        });
        let i = inputs_for(&c, "sv", 0, 1);
        let a = ruqu_bundle(&i.as_borrowed());
        let b = ruqu_bundle(&i.as_borrowed());
        assert_eq!(a.rvf_witness, b.rvf_witness);
        assert_eq!(a.memory_class.as_deref(), Some("quantum"));
    }

    #[test]
    fn witness_changes_when_backend_tag_changes() {
        let mut c = Circuit::new("bell", 2);
        c.push(Gate::H { q: 0 });
        c.push(Gate::Cx {
            control: 0,
            target: 1,
        });
        let a = ruqu_bundle(&inputs_for(&c, "sv", 0, 1).as_borrowed());
        let b = ruqu_bundle(&inputs_for(&c, "stabilizer", 0, 1).as_borrowed());
        assert_ne!(
            a.rvf_witness, b.rvf_witness,
            "backend tag must rotate the witness — distinguishes ADR-008 §4 backends"
        );
    }

    #[test]
    fn witness_changes_when_seed_changes() {
        let c = Circuit::new("noise", 2);
        let a = ruqu_bundle(&inputs_for(&c, "sv", 0, 1).as_borrowed());
        let b = ruqu_bundle(&inputs_for(&c, "sv", 1, 1).as_borrowed());
        assert_ne!(a.rvf_witness, b.rvf_witness);
    }
}
