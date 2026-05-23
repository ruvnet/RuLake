//! Hardware backend (v0.2 stub) for ruQu.
//!
//! Per ADR-008 §4 the Hardware backend "abstracts a real QPU connector;
//! v0.2 ships a deterministic stub backend that returns a documented
//! synthetic-noise output for testing wire shape; real connectors land
//! in their own substrate crates as they're built."
//!
//! What this module is:
//!   * A `BackendAdapter` impl that simulates a QPU run by calling the
//!     existing exact [`crate::state_vector::simulate`] kernel
//!     internally, then applying a parameterised single-qubit
//!     depolarising channel (`p` per gate, in expectation) seeded from
//!     a fixed `u64` so two replicas produce byte-identical state.
//!   * A backend tag of `"hardware-stub"` so the witness derived from
//!     a Hardware-backed circuit is *distinct* from the StateVector or
//!     Stabilizer witness over the same circuit. This is the same
//!     contract the v0.1 Stabilizer backend uses (`witness::inputs_for`).
//!
//! What this module is **not**:
//!   * A real QPU connector. No network calls, no provider SDKs, no
//!     queue / calibration / runtime_class behaviour beyond the
//!     [`HardwareConfig`] holder structs that v0.3 will wire up to
//!     IBM Q / Quantinuum / IonQ.
//!   * A noise model with any physical claim. The depolarising channel
//!     here is a *wire-shape* placeholder: its job is to make the
//!     output state numerically distinguishable from the noiseless
//!     StateVector run while remaining deterministic for a given seed.
//!
//! Witness chain:
//!   `data_ref = "ruqu://<circuit-id>@hardware-stub"`,
//!   `rotation_seed = noise_seed`. Two `RuquHardwareStubBackend`s
//!   constructed with the same `noise_p` and `seed` will produce the
//!   same witness for the same circuit, so cross-process replay holds.

use std::collections::BTreeMap;
use std::sync::RwLock;

use rulake::backend::{BackendAdapter, CollectionId, PulledBatch};
use rulake::error::Result;

use crate::circuit::Circuit;
use crate::state_vector::{self, C};
use crate::witness;
use crate::RuquExecution;

/// Backend tag flowed into [`witness::inputs_for`]. Distinct from
/// `"sv"` and `"stabilizer"` so the Hardware backend's cache key
/// never collides with the exact-simulator backends.
pub const HARDWARE_STUB_BACKEND_TAG: &str = "hardware-stub";

/// v0.3 will replace this with provider-specific config (queue URL,
/// device id, calibration snapshot id, runtime_class). v0.2 holds the
/// shape so callers building against the stub already know which
/// fields will exist when the real connector lands.
#[derive(Debug, Clone, Default)]
pub struct HardwareConfig {
    /// Free-form provider name (e.g. `"ibmq"`, `"quantinuum"`,
    /// `"ionq"`). v0.2 ignores this; v0.3 dispatches on it.
    pub provider: Option<String>,
    /// Device id within the provider's catalogue. v0.2 ignores it.
    pub device: Option<String>,
    /// Calibration snapshot tag — flows into the witness `generation`
    /// once v0.3 wires real calibration. v0.2 always uses `1`.
    pub calibration_tag: Option<String>,
    /// Runtime class — `"primitive"`, `"session"`, `"batch"`. v0.2
    /// ignores it.
    pub runtime_class: Option<String>,
}

/// Hardware (stub) BackendAdapter for ruQu. v0.2.
///
/// Holds executed circuits in a name → noisy-state-vector map. The
/// noise channel is a per-gate depolarising flip parameterised by
/// `noise_p`, applied *after* the exact StateVector run. The PRNG is a
/// tiny SplitMix64 seeded from `seed` so the noise sequence is
/// deterministic and replayable.
pub struct RuquHardwareStubBackend {
    id: String,
    noise_p: f64,
    seed: u64,
    config: HardwareConfig,
    executions: RwLock<BTreeMap<String, RuquExecution>>,
}

impl RuquHardwareStubBackend {
    /// Create a Hardware stub backend with a stable id, a per-gate
    /// depolarising-channel probability `noise_p` (clamped to
    /// `[0.0, 1.0]`), and a `seed` that pins the noise PRNG.
    ///
    /// The `noise_p == 0.0` setting yields a backend that is
    /// numerically equivalent to [`crate::RuquStateVectorBackend`]'s
    /// output — but the witness still differs because of the
    /// `"hardware-stub"` backend tag. Callers wanting witness
    /// equivalence between the two backends must wait for ADR-008 §G4
    /// `--concordance-mode` (v0.3+).
    pub fn new(id: impl Into<String>, noise_p: f64, seed: u64) -> Self {
        Self {
            id: id.into(),
            noise_p: noise_p.clamp(0.0, 1.0),
            seed,
            config: HardwareConfig::default(),
            executions: RwLock::new(BTreeMap::new()),
        }
    }

    /// Attach a [`HardwareConfig`] to the backend. v0.2 stores it but
    /// does not act on it; v0.3 will dispatch real provider calls
    /// through it.
    pub fn with_config(mut self, config: HardwareConfig) -> Self {
        self.config = config;
        self
    }

    /// Borrow the [`HardwareConfig`] currently attached.
    pub fn config(&self) -> &HardwareConfig {
        &self.config
    }

    /// The per-gate depolarising probability the stub will apply.
    pub fn noise_p(&self) -> f64 {
        self.noise_p
    }

    /// The seed pinned at construction.
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// Simulate `circuit` through the exact StateVector kernel, apply
    /// the synthetic noise channel, store the (noisy) state under
    /// `name`, and return the witness bundle.
    ///
    /// Stored vectors use the same `[re, im]` row layout as
    /// [`crate::RuquStateVectorBackend`] so callers can swap backends
    /// without touching downstream code.
    pub fn execute(
        &self,
        name: impl Into<String>,
        circuit: &Circuit,
    ) -> Result<rulake::RuLakeBundle> {
        let exact = state_vector::simulate(circuit);
        let n_gates = circuit.gates.len() as u64;
        let noisy = apply_synthetic_noise(&exact, self.noise_p, self.seed, n_gates);
        let exec = RuquExecution::from_state_vector(&noisy, 1);
        let inputs = witness::inputs_for(
            circuit,
            HARDWARE_STUB_BACKEND_TAG,
            self.seed,
            exec.generation,
        );
        let bundle = witness::ruqu_bundle(&inputs.as_borrowed());
        self.executions
            .write()
            .expect("poisoned")
            .insert(name.into(), exec);
        Ok(bundle)
    }

    /// Bump the generation of an executed circuit — same semantics as
    /// the StateVector / Stabilizer backends.
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

impl BackendAdapter for RuquHardwareStubBackend {
    fn id(&self) -> &str {
        &self.id
    }

    fn list_collections(&self) -> Result<Vec<CollectionId>> {
        Ok(self
            .executions
            .read()
            .expect("poisoned")
            .keys()
            .cloned()
            .collect())
    }

    fn pull_vectors(&self, collection: &str) -> Result<PulledBatch> {
        let g = self.executions.read().expect("poisoned");
        let c = g
            .get(collection)
            .ok_or_else(|| rulake::error::RuLakeError::UnknownCollection {
                backend: self.id.clone(),
                collection: collection.to_string(),
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
        let c = g
            .get(collection)
            .ok_or_else(|| rulake::error::RuLakeError::UnknownCollection {
                backend: self.id.clone(),
                collection: collection.to_string(),
            })?;
        Ok(c.generation)
    }
}

/// Apply a deterministic synthetic-noise channel to the exact state.
///
/// Strategy: re-normalise the state into a small per-amplitude phase
/// jitter mixed with depolarising leakage. The jitter is drawn from a
/// SplitMix64 stream seeded by `(seed, n_gates)`; the leakage scale is
/// `p_total = 1 - (1 - noise_p)^n_gates`. The output is re-normalised
/// to unit L2 norm so downstream `|amplitudes|² == 1` invariants hold
/// within `1e-3`. The function is pure given `(state, noise_p, seed,
/// n_gates)` so two replicas produce identical output.
fn apply_synthetic_noise(state: &[C], noise_p: f64, seed: u64, n_gates: u64) -> Vec<C> {
    let p_total = 1.0 - (1.0 - noise_p).powi(n_gates as i32);
    let mut prng = seed
        .wrapping_add(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(n_gates);
    let mut out: Vec<C> = state
        .iter()
        .map(|amp| {
            let r0 = next_unit_signed(&mut prng);
            let r1 = next_unit_signed(&mut prng);
            // Mix the original amplitude with a small (re, im) jitter
            // scaled by p_total. The jitter is bounded to
            // `[-p_total, +p_total]` per component.
            let jr = r0 * p_total;
            let ji = r1 * p_total;
            let blend = 1.0 - p_total;
            C::new(amp.re * blend + jr, amp.im * blend + ji)
        })
        .collect();
    // Re-normalise to unit L2 so `|amplitudes|².sum() ≈ 1.0`.
    let nrm_sq: f64 = out.iter().map(|c| c.re * c.re + c.im * c.im).sum();
    if nrm_sq > 0.0 {
        let nrm = nrm_sq.sqrt();
        for c in out.iter_mut() {
            c.re /= nrm;
            c.im /= nrm;
        }
    }
    out
}

/// SplitMix64 → uniform `(-1.0, 1.0)`. Stdlib only.
fn next_unit_signed(s: &mut u64) -> f64 {
    *s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *s;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    // Map to [0, 1) then shift to (-1, 1).
    let u = (z >> 11) as f64 / (1u64 << 53) as f64;
    2.0 * u - 1.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::circuit::{Circuit, Gate};

    #[test]
    fn noise_zero_keeps_unit_norm() {
        let mut c = Circuit::new("bell-hw", 2);
        c.push(Gate::H { q: 0 });
        c.push(Gate::Cx {
            control: 0,
            target: 1,
        });
        let exact = state_vector::simulate(&c);
        let noisy = apply_synthetic_noise(&exact, 0.0, 7, c.gates.len() as u64);
        let s: f64 = noisy.iter().map(|c| c.re * c.re + c.im * c.im).sum();
        assert!((s - 1.0).abs() < 1e-9);
    }

    #[test]
    fn noise_is_deterministic_for_same_seed() {
        let mut c = Circuit::new("bell-hw", 2);
        c.push(Gate::H { q: 0 });
        c.push(Gate::Cx {
            control: 0,
            target: 1,
        });
        let exact = state_vector::simulate(&c);
        let a = apply_synthetic_noise(&exact, 0.05, 42, c.gates.len() as u64);
        let b = apply_synthetic_noise(&exact, 0.05, 42, c.gates.len() as u64);
        for (x, y) in a.iter().zip(b.iter()) {
            assert_eq!(x.re, y.re);
            assert_eq!(x.im, y.im);
        }
    }

    #[test]
    fn config_is_stored_but_not_acted_on() {
        let cfg = HardwareConfig {
            provider: Some("ibmq".into()),
            device: Some("ibm_kyoto".into()),
            calibration_tag: Some("2026-04-01".into()),
            runtime_class: Some("session".into()),
        };
        let be = RuquHardwareStubBackend::new("hw", 0.01, 9).with_config(cfg);
        assert_eq!(be.config().provider.as_deref(), Some("ibmq"));
        assert_eq!(be.noise_p(), 0.01);
        assert_eq!(be.seed(), 9);
    }
}
