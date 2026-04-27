//! ruQu v2 BackendAdapter for ruLake (ADR-008).
//!
//! v0.0 ships the StateVector backend only — small-N exact circuit
//! simulation with witness-anchored execution. Stabilizer / Clifford+T
//! / TensorNetwork / Hardware backends land in v0.1 / v0.2 per
//! ADR-008 §4 and `docs/research/ruqu/integration-with-rulake.md`.
//!
//! Trust boundary: the witness ([`witness::ruqu_bundle`]) is byte-
//! isomorphic to a native `RuLakeBundle`. Two deployments running the
//! same circuit at the same seed produce identical SHAKE-256(32)
//! witnesses and share cache — that's the cross-process replay
//! property the ADR §3 trust chain hinges on.
//!
//! v0.0 is deliberately schema-light: a small mini-IR (H/X/Y/Z/S/T/
//! Rz/CX) is enough to demo Bell pairs and prove the witness path
//! end-to-end. v0.0.2 adds an OpenQASM 3.0 frontend; v0.1 adds the
//! Stabilizer + TensorNetwork backends.

#![deny(unsafe_code)]
#![warn(missing_docs)]

use std::collections::BTreeMap;
use std::sync::RwLock;

use rulake::backend::{BackendAdapter, CollectionId, PulledBatch};
use rulake::error::Result;

pub mod circuit;
pub mod state_vector;
pub mod witness;

pub use circuit::{Circuit, Gate};
pub use state_vector::{simulate, C};
pub use witness::{inputs_for, ruqu_bundle, RuquWitnessInputs, RuquWitnessInputsOwned};

/// One executed circuit's state, keyed by a "collection name" so it
/// fits the BackendAdapter contract. The vectors handed to ruLake are
/// `[real, imag, real, imag, …]` — keeps the cache-key witness path
/// untouched (ruLake doesn't need to know they're complex amplitudes).
#[derive(Debug, Clone)]
pub struct RuquExecution {
    /// External ids — one per Hilbert-space basis state. `0..(1<<n)`.
    pub ids: Vec<u64>,
    /// Flattened amplitude pairs. `vectors[i]` has length 2:
    /// `[re, im]`. v0.0 stores one pair per basis state; v0.1 will
    /// chunk for tensor-network views.
    pub vectors: Vec<Vec<f32>>,
    /// Vector dimensionality — always 2 for `[re, im]`. The Hilbert-
    /// space dim is `vectors.len()`.
    pub dim: usize,
    /// Coherence token. Bumps when the circuit source rotates, the
    /// noise model is repinned, or the calibration snapshot changes.
    pub generation: u64,
}

impl RuquExecution {
    /// Build from a state vector + a generation. Truncates / down-
    /// casts amplitudes to `f32` so they round-trip through ruLake's
    /// vector cache; full `f64` precision is preserved in the witness
    /// chain (the seed pins the simulation to the same `f64` result).
    pub fn from_state_vector(sv: &[C], generation: u64) -> Self {
        let ids: Vec<u64> = (0..sv.len() as u64).collect();
        let vectors: Vec<Vec<f32>> = sv
            .iter()
            .map(|c| vec![c.re as f32, c.im as f32])
            .collect();
        Self { ids, vectors, dim: 2, generation }
    }
}

/// StateVector BackendAdapter for ruQu.
///
/// Holds executed circuits in a name → state-vector map. Operators
/// drive it by:
///   1. constructing a [`Circuit`];
///   2. calling [`Self::execute`] to simulate + cache;
///   3. registering this backend with `RuLake` so federated callers
///      see the result indistinguishable from a "data" lookup.
pub struct RuquStateVectorBackend {
    id: String,
    executions: RwLock<BTreeMap<String, RuquExecution>>,
}

impl RuquStateVectorBackend {
    /// Create an empty StateVector backend with a stable id.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            executions: RwLock::new(BTreeMap::new()),
        }
    }

    /// Simulate `circuit` and store the resulting state vector under
    /// `name`. Returns the bundle witness so callers can pin it (or
    /// hand it to a remote replica for byte-equality replay).
    pub fn execute(&self, name: impl Into<String>, circuit: &Circuit) -> Result<rulake::RuLakeBundle> {
        let sv = state_vector::simulate(circuit);
        let exec = RuquExecution::from_state_vector(&sv, 1);
        let inputs = witness::inputs_for(circuit, "sv", 0, exec.generation);
        let bundle = witness::ruqu_bundle(&inputs.as_borrowed());
        self.executions.write().expect("poisoned").insert(name.into(), exec);
        Ok(bundle)
    }

    /// Bump the generation of an executed circuit — call after a
    /// noise-model repinning or recalibration so the cache invalidates.
    pub fn bump_generation(&self, name: &str) -> Result<u64> {
        let mut g = self.executions.write().expect("poisoned");
        match g.get_mut(name) {
            Some(c) => {
                c.generation = c.generation.checked_add(1).unwrap_or(c.generation);
                Ok(c.generation)
            }
            None => Err(rulake::error::RuLakeError::UnknownCollection {
                backend: self.id.clone(),
                collection: name.to_string(),
            }),
        }
    }
}

impl BackendAdapter for RuquStateVectorBackend {
    fn id(&self) -> &str {
        &self.id
    }

    fn list_collections(&self) -> Result<Vec<CollectionId>> {
        Ok(self.executions.read().expect("poisoned").keys().cloned().collect())
    }

    fn pull_vectors(&self, collection: &str) -> Result<PulledBatch> {
        let g = self.executions.read().expect("poisoned");
        let c = g.get(collection).ok_or_else(|| {
            rulake::error::RuLakeError::UnknownCollection {
                backend: self.id.clone(),
                collection: collection.to_string(),
            }
        })?;
        Ok(PulledBatch {
            collection: collection.to_string(),
            ids: c.ids.clone(),
            vectors: c.vectors.clone(),
            dim: c.dim,
            generation: c.generation,
        })
    }

    fn generation(&self, collection: &str) -> Result<u64> {
        let g = self.executions.read().expect("poisoned");
        let c = g.get(collection).ok_or_else(|| {
            rulake::error::RuLakeError::UnknownCollection {
                backend: self.id.clone(),
                collection: collection.to_string(),
            }
        })?;
        Ok(c.generation)
    }
}
