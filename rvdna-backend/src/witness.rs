//! Witness derivation for `.rvdna` v2 files.
//!
//! The bundle pointer at byte `0x00B0` of a v2 file is byte-isomorphic
//! to ruLake's [`RuLakeBundle`]. Two deployments reading the same v2
//! file MUST produce identical SHAKE-256(32) witnesses; v0.0 derives
//! the witness from the synthetic fixture's load-bearing fields the
//! same way `rulake::RuLakeBundle::new` does. v0.0.2 will read the
//! pointer directly out of the file's bytes.
//!
//! The model checkpoint id flows into `data_ref` (e.g.
//! `rvdna://<file-id>/<model>@<rev>`) so two `.rvdna` files generated
//! against ESM-2 v0.4.1 vs v0.5.0 over identical input DNA produce
//! distinct witnesses. This is the ADR-007 §c "model-bound" gotcha
//! made explicit.

use rulake::bundle::Generation;
use rulake::RuLakeBundle;

/// The scope of the witness chain — every load-bearing input to v2
/// flows through here.
#[derive(Debug, Clone)]
pub struct RvdnaWitnessInputs<'a> {
    /// `rvdna://<file-id>/<model>@<rev>` URI naming the rvDNA file.
    pub data_ref: &'a str,
    /// Vector dimensionality (k-mer embedding dim).
    pub dim: usize,
    /// Rotation seed — pinned per encoder run.
    pub rotation_seed: u64,
    /// rerank_factor — pinned per encoder run.
    pub rerank_factor: usize,
    /// Generation token. Bumps on re-encode, variant-call refresh,
    /// epigenomic clock advance, etc.
    pub generation: u64,
}

/// Build a [`RuLakeBundle`] for an rvDNA v2 collection. Identical
/// witness derivation as `RuLakeBundle::new` so the cache entry is
/// indistinguishable from any other ruLake bundle — federation,
/// audit, and the Console's verifier path all work unchanged.
pub fn rvdna_bundle(inputs: &RvdnaWitnessInputs<'_>) -> RuLakeBundle {
    RuLakeBundle::new(
        inputs.data_ref.to_string(),
        inputs.dim,
        inputs.rotation_seed,
        inputs.rerank_factor,
        Generation::Num(inputs.generation),
    )
    .with_memory_class("genomic")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn witness_deterministic_for_same_inputs() {
        let inputs = RvdnaWitnessInputs {
            data_ref: "rvdna://fixture-1/esm2@v0.4.1",
            dim: 64,
            rotation_seed: 42,
            rerank_factor: 20,
            generation: 1,
        };
        let a = rvdna_bundle(&inputs);
        let b = rvdna_bundle(&inputs);
        assert_eq!(a.rvf_witness, b.rvf_witness);
        assert_eq!(a.memory_class.as_deref(), Some("genomic"));
    }

    #[test]
    fn witness_changes_when_model_revision_bumps() {
        let mut inputs = RvdnaWitnessInputs {
            data_ref: "rvdna://fixture-1/esm2@v0.4.1",
            dim: 64,
            rotation_seed: 42,
            rerank_factor: 20,
            generation: 1,
        };
        let a = rvdna_bundle(&inputs);
        inputs.data_ref = "rvdna://fixture-1/esm2@v0.5.0";
        let b = rvdna_bundle(&inputs);
        assert_ne!(
            a.rvf_witness, b.rvf_witness,
            "model revision must rotate the witness — ADR-007 §c"
        );
    }

    #[test]
    fn witness_changes_when_generation_bumps() {
        let mut inputs = RvdnaWitnessInputs {
            data_ref: "rvdna://fixture-1/esm2@v0.4.1",
            dim: 64,
            rotation_seed: 42,
            rerank_factor: 20,
            generation: 1,
        };
        let a = rvdna_bundle(&inputs);
        inputs.generation = 2;
        let b = rvdna_bundle(&inputs);
        assert_ne!(a.rvf_witness, b.rvf_witness);
    }
}
