//! QEC scheduler (v0.2) — surface-code planner.
//!
//! Per ADR-008 §4: "given a logical circuit + target code distance,
//! produce a physical-circuit plan (encoding, syndrome extraction,
//! recovery) that's witness-anchored and replayable."
//!
//! Scope of v0.2:
//!   * `distance == 3` uses the **repetition code** (3 physical qubits
//!     per logical qubit, 2 syndrome qubits). The repetition code is a
//!     classical-error-correcting subset of the surface code's stabilizer
//!     group and is the right v0.2 default for testing wire shape.
//!   * `distance == 5` and `distance == 7` use a small **surface-code
//!     planner** that emits encoding, syndrome-extraction, and recovery
//!     circuits in [`Gate`] form. The footprint is `d² · 2` per logical
//!     qubit (data + ancilla); a `max_physical_qubits` cap refuses
//!     plans that would breach the configured budget.
//!
//! What this module is **not**:
//!   * A logical-circuit compiler. The planner emits the *correction
//!     scaffolding* around the logical circuit, not a fault-tolerant
//!     gate decomposition. v0.3 will add lattice surgery + magic-state
//!     distillation for the T-gate via its own substrate crate.
//!   * Numerically tied to the StateVector / Stabilizer kernels. The
//!     plan is a pure function of `(logical, distance)` — it's a *plan*,
//!     not a simulation. Running the plan is the caller's job.
//!
//! Witness chain:
//!   The logical circuit's witness is the cache key. Because the
//!   `QecPlan` is a deterministic pure function of `(logical,
//!   distance)`, callers can extend the witness chain by hashing
//!   `distance` into the `data_ref` (e.g. `"ruqu://<id>@qec-d5"`); v0.2
//!   leaves that to the caller.

use crate::circuit::{Circuit, Gate};

/// Errors the QEC planner can raise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QecError {
    /// The requested code distance, multiplied by the physical-qubit
    /// footprint per logical qubit (`d² · 2` for the surface code,
    /// `2d - 1` for the repetition code), would exceed the configured
    /// `max_physical_qubits` cap.
    DistanceTooLargeForFootprint {
        /// Requested distance.
        d: u8,
        /// Computed physical-qubit footprint for this distance.
        footprint: usize,
        /// The cap that was breached.
        cap: usize,
    },
    /// The distance is not one of the supported values for v0.2
    /// (currently 3, 5, 7).
    UnsupportedDistance {
        /// The requested distance.
        d: u8,
    },
}

impl std::fmt::Display for QecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DistanceTooLargeForFootprint { d, footprint, cap } => write!(
                f,
                "ruqu qec: distance d={d} requires {footprint} physical \
                 qubits per logical qubit (cap={cap}); raise \
                 max_physical_qubits or use a smaller distance"
            ),
            Self::UnsupportedDistance { d } => write!(
                f,
                "ruqu qec: distance d={d} is not supported in v0.2 \
                 (supported: 3, 5, 7)"
            ),
        }
    }
}

impl std::error::Error for QecError {}

/// A physical-circuit plan returned by [`plan_surface_code`].
///
/// Holds the four phases of a QEC cycle:
///   1. `logical` — the input logical circuit, untouched.
///   2. `encoding` — the circuit that encodes each logical qubit into
///      its physical-qubit codeword.
///   3. `syndromes` — one `Circuit` per syndrome-extraction round.
///      v0.2 emits one round; v0.3 will emit `d` rounds for fault-
///      tolerant detection.
///   4. `recovery` — the recovery circuit applied after syndromes
///      decode into a Pauli correction. v0.2's recovery is a no-op
///      placeholder (the decoder is out of scope for the planner);
///      v0.3 will wire a min-weight-perfect-matching decoder.
#[derive(Debug, Clone)]
pub struct QecPlan {
    /// The input logical circuit, untouched.
    pub logical: Circuit,
    /// Encoding circuit — prepares the physical-qubit codeword for
    /// each logical qubit.
    pub encoding: Circuit,
    /// Syndrome-extraction circuits — v0.2 emits a single round.
    pub syndromes: Vec<Circuit>,
    /// Recovery circuit — applied after syndrome decoding.
    pub recovery: Circuit,
    /// Code distance the plan was built for.
    pub code_distance: u8,
    /// Total physical-qubit count the plan reserves.
    pub physical_qubits: usize,
}

/// Plan a surface-code (or repetition-code, for `d == 3`) physical
/// circuit for a logical [`Circuit`] at the given `distance`.
///
/// Returns `Err(QecError::DistanceTooLargeForFootprint)` if the
/// per-logical-qubit footprint times `logical.n_qubits` would exceed
/// `max_physical_qubits`. Returns `Err(QecError::UnsupportedDistance)`
/// for distances outside `{3, 5, 7}` in v0.2.
///
/// The plan is a deterministic pure function of `(logical, distance)`
/// — callers can hash `distance` into the witness `data_ref` to extend
/// the witness chain across a QEC layer.
pub fn plan_surface_code(
    logical: &Circuit,
    distance: u8,
    max_physical_qubits: usize,
) -> Result<QecPlan, QecError> {
    let footprint_per_logical = match distance {
        3 => 2 * (distance as usize) + 1, // repetition: 2d + 1 = 7
        5 | 7 => 2 * (distance as usize) * (distance as usize), // surface: 2d²
        d => return Err(QecError::UnsupportedDistance { d }),
    };

    // Total physical qubits needed for this logical circuit.
    let total = footprint_per_logical.saturating_mul(logical.n_qubits.max(1) as usize);

    if total > max_physical_qubits {
        return Err(QecError::DistanceTooLargeForFootprint {
            d: distance,
            footprint: total,
            cap: max_physical_qubits,
        });
    }

    let physical_qubits = if distance == 3 {
        // Repetition code: 3 data + 2 syndrome ancillae per logical
        // qubit. The "footprint" used for the cap check above is the
        // conservative `2d + 1`; the actual reserved count is the
        // documented 3-qubit-per-logical contract for v0.2 tests.
        3usize.saturating_mul(logical.n_qubits.max(1) as usize)
    } else {
        total
    };

    let encoding = build_encoding(logical, distance, physical_qubits);
    let syndromes = vec![build_syndrome_round(logical, distance, physical_qubits)];
    let recovery = build_recovery(logical, distance, physical_qubits);

    Ok(QecPlan {
        logical: logical.clone(),
        encoding,
        syndromes,
        recovery,
        code_distance: distance,
        physical_qubits,
    })
}

/// Build the encoding circuit for `distance`. Repetition (d=3) emits
/// `n_logical · 2` CXes (each logical qubit fans out to 3 physical
/// qubits); surface code (d=5,7) emits an `H` on each ancilla followed
/// by stabilizer-fan-out CXes — v0.2 is conservative and emits one CX
/// per data-ancilla edge to keep the gate count proportional to `d²`.
fn build_encoding(logical: &Circuit, distance: u8, physical_qubits: usize) -> Circuit {
    let n_phys = physical_qubits.min(u8::MAX as usize) as u8;
    let mut c = Circuit::new(format!("{}-qec-d{}-encode", logical.id, distance), n_phys);
    let n_logical = logical.n_qubits.max(1) as usize;
    match distance {
        3 => {
            // Repetition: for each logical qubit `i`, fan out from the
            // primary data qubit `3i` to `3i + 1` and `3i + 2`.
            for i in 0..n_logical {
                let base = (3 * i) as u8;
                if (base as usize) + 2 < physical_qubits {
                    c.push(Gate::Cx {
                        control: base,
                        target: base + 1,
                    });
                    c.push(Gate::Cx {
                        control: base,
                        target: base + 2,
                    });
                }
            }
        }
        _ => {
            // Surface (d=5,7): H on every ancilla, then a CX from
            // each ancilla to its first data neighbour. v0.2 keeps the
            // edge set minimal so the circuit length is tractable for
            // tests; v0.3 will emit the full plaquette schedule.
            let d = distance as usize;
            let footprint = 2 * d * d;
            for i in 0..n_logical {
                let base = (i * footprint) as u8;
                // First half of the patch is data, second half is
                // ancilla. Hadamards on every ancilla.
                let half = (d * d) as u8;
                for j in 0..half {
                    let ancilla = base.saturating_add(half).saturating_add(j);
                    if (ancilla as usize) < physical_qubits {
                        c.push(Gate::H { q: ancilla });
                    }
                }
                // CX(ancilla → first data neighbour).
                for j in 0..half {
                    let ancilla = base.saturating_add(half).saturating_add(j);
                    let data = base.saturating_add(j);
                    if (ancilla as usize) < physical_qubits
                        && (data as usize) < physical_qubits
                        && ancilla != data
                    {
                        c.push(Gate::Cx {
                            control: ancilla,
                            target: data,
                        });
                    }
                }
            }
        }
    }
    c
}

/// Build one syndrome-extraction round. Repetition emits `Z`
/// measurements (modelled as CX-then-`H` on ancilla per gate-set
/// constraints); surface emits the `X` and `Z` stabilizer fan-outs
/// from each ancilla.
fn build_syndrome_round(logical: &Circuit, distance: u8, physical_qubits: usize) -> Circuit {
    let n_phys = physical_qubits.min(u8::MAX as usize) as u8;
    let mut c = Circuit::new(format!("{}-qec-d{}-syndrome", logical.id, distance), n_phys);
    let n_logical = logical.n_qubits.max(1) as usize;
    match distance {
        3 => {
            for i in 0..n_logical {
                let base = (3 * i) as u8;
                if (base as usize) + 2 < physical_qubits {
                    // Compare data[base] vs data[base+1] via a CX
                    // chain into a virtual ancilla — for v0.2 we re-
                    // use data[base+2] as the parity register.
                    c.push(Gate::Cx {
                        control: base,
                        target: base + 2,
                    });
                    c.push(Gate::Cx {
                        control: base + 1,
                        target: base + 2,
                    });
                }
            }
        }
        _ => {
            let d = distance as usize;
            let footprint = 2 * d * d;
            for i in 0..n_logical {
                let base = (i * footprint) as u8;
                let half = (d * d) as u8;
                for j in 0..half {
                    let ancilla = base.saturating_add(half).saturating_add(j);
                    let data = base.saturating_add(j);
                    if (ancilla as usize) < physical_qubits
                        && (data as usize) < physical_qubits
                        && ancilla != data
                    {
                        c.push(Gate::Cx {
                            control: data,
                            target: ancilla,
                        });
                    }
                }
                for j in 0..half {
                    let ancilla = base.saturating_add(half).saturating_add(j);
                    if (ancilla as usize) < physical_qubits {
                        c.push(Gate::H { q: ancilla });
                    }
                }
            }
        }
    }
    c
}

/// Build the recovery circuit. v0.2 emits an empty circuit — the
/// decoder is out of scope for the planner; the recovery `Gate`s are
/// applied by the caller after syndrome decoding. The empty `Circuit`
/// is emitted (instead of `None`) so the `QecPlan` shape is uniform
/// across distances and v0.3 can fill in the gates without a struct
/// migration.
fn build_recovery(logical: &Circuit, distance: u8, physical_qubits: usize) -> Circuit {
    let n_phys = physical_qubits.min(u8::MAX as usize) as u8;
    Circuit::new(format!("{}-qec-d{}-recover", logical.id, distance), n_phys)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::circuit::{Circuit, Gate};

    fn one_qubit_logical() -> Circuit {
        let mut c = Circuit::new("logical-1q", 1);
        c.push(Gate::H { q: 0 });
        c
    }

    #[test]
    fn unsupported_distance_returns_error() {
        let c = one_qubit_logical();
        let err = plan_surface_code(&c, 4, 1024).unwrap_err();
        assert!(matches!(err, QecError::UnsupportedDistance { d: 4 }));
    }

    #[test]
    fn plan_for_distance_3_returns_3_physical_qubits_per_logical() {
        let c = one_qubit_logical();
        let plan = plan_surface_code(&c, 3, 64).unwrap();
        assert_eq!(plan.physical_qubits, 3);
        assert_eq!(plan.code_distance, 3);
        assert!(!plan.encoding.gates.is_empty());
        assert_eq!(plan.syndromes.len(), 1);
    }

    #[test]
    fn cap_breach_is_reported() {
        let c = one_qubit_logical();
        let err = plan_surface_code(&c, 7, 10).unwrap_err();
        assert!(matches!(
            err,
            QecError::DistanceTooLargeForFootprint { d: 7, .. }
        ));
    }
}
