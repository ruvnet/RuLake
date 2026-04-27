//! ruQu v2 BackendAdapter for ruLake (ADR-008).
//!
//! v0.0 shipped the StateVector backend; **v0.1 adds the Stabilizer
//! backend** ([`RuquStabilizerBackend`] / [`stabilizer::simulate`])
//! built on the Aaronson–Gottesman tableau formalism. Clifford-only
//! circuits run in polynomial time and at much wider `n` than the
//! exponential StateVector path — see the bench at
//! `benches/stabilizer.rs` for the headline numbers. The TensorNetwork
//! / Clifford+T / Hardware backends land in v0.2 per ADR-008 §4 and
//! `docs/research/ruqu/integration-with-rulake.md`.
//!
//! Trust boundary: the witness ([`witness::ruqu_bundle`]) is byte-
//! isomorphic to a native `RuLakeBundle`. Two deployments running the
//! same circuit at the same seed produce identical SHAKE-256(32)
//! witnesses and share cache — that's the cross-process replay
//! property the ADR §3 trust chain hinges on. v0.1 keeps the per-
//! backend tag in `data_ref` (`"sv"` vs `"stabilizer"`), so the same
//! all-Clifford circuit on the two backends produces *distinct*
//! witnesses. v0.2 introduces a `--concordance-mode` strip per
//! ADR-008 §G4 so the two backends can attest the same result.
//!
//! v0.1 is still schema-light: the mini-IR (H/X/Y/Z/S/T/Rz/CX) is
//! enough to demo Bell pairs end-to-end on both backends. v0.0.2
//! adds an OpenQASM 3.0 frontend.

#![deny(unsafe_code)]
#![warn(missing_docs)]

use std::collections::BTreeMap;
use std::sync::RwLock;

use rulake::backend::{BackendAdapter, CollectionId, PulledBatch};
use rulake::error::Result;

pub mod circuit;
pub mod hardware;
pub mod limits;
pub mod qec;
pub mod stabilizer;
pub mod state_vector;
pub mod witness;

pub use circuit::{Circuit, Gate};
pub use hardware::{HardwareConfig, RuquHardwareStubBackend, HARDWARE_STUB_BACKEND_TAG};
pub use qec::{plan_surface_code, QecError, QecPlan};
pub use stabilizer::{
    simulate as simulate_stabilizer, StabilizerError, StabilizerState, Tableau,
};
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

/// Stabilizer BackendAdapter for ruQu (v0.1).
///
/// Mirrors [`RuquStateVectorBackend`]'s shape: operators construct it,
/// call [`Self::execute`] to simulate a Clifford circuit, then
/// register it with `RuLake`. The backing simulator is the Aaronson–
/// Gottesman tableau in [`stabilizer`]; non-Clifford gates (`T`,
/// `Rz`) are rejected with a [`StabilizerError`] mapped onto
/// [`rulake::error::RuLakeError::Backend`] for the adapter surface.
///
/// The witness flowing into the [`RuLakeBundle`] uses the
/// `"stabilizer"` backend tag, so the same Clifford circuit on the
/// StateVector backend produces a *distinct* witness in v0.1. The
/// concordance-mode strip per ADR-008 §G4 lands in v0.2.
pub struct RuquStabilizerBackend {
    id: String,
    executions: RwLock<BTreeMap<String, RuquExecution>>,
}

impl RuquStabilizerBackend {
    /// Create an empty Stabilizer backend with a stable id.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            executions: RwLock::new(BTreeMap::new()),
        }
    }

    /// Simulate `circuit` on the tableau, store the packed state under
    /// `name`, and return the witness bundle. Returns a
    /// [`rulake::error::RuLakeError::Backend`] when the circuit
    /// contains a non-Clifford gate or breaches the v0.1 qubit cap.
    pub fn execute(
        &self,
        name: impl Into<String>,
        circuit: &Circuit,
    ) -> Result<rulake::RuLakeBundle> {
        let state = stabilizer::simulate(circuit).map_err(|e| {
            rulake::error::RuLakeError::Backend {
                backend: self.id.clone(),
                detail: e.to_string(),
            }
        })?;
        let vectors = stabilizer::to_pulled_batch_form(&state);
        let dim = vectors.first().map(|v| v.len()).unwrap_or(0);
        let ids: Vec<u64> = (0..vectors.len() as u64).collect();
        let exec = RuquExecution { ids, vectors, dim, generation: 1 };
        let inputs = witness::inputs_for(circuit, "stabilizer", 0, exec.generation);
        let bundle = witness::ruqu_bundle(&inputs.as_borrowed());
        self.executions.write().expect("poisoned").insert(name.into(), exec);
        Ok(bundle)
    }

    /// Bump the generation of an executed circuit — same semantics as
    /// [`RuquStateVectorBackend::bump_generation`].
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

impl BackendAdapter for RuquStabilizerBackend {
    fn id(&self) -> &str { &self.id }

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
