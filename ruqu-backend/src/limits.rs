//! Resource-exhaustion guards for the StateVector path.
//!
//! Added by the v0.0 security review (see
//! `docs/research/security/v0.0-substrates.md` finding **R-2**).
//!
//! `Circuit::dim()` saturates `n_qubits` at 31, which would have
//! `state_vector::simulate` allocate `1 << 31 = 2^31` complex
//! amplitudes (~32 GiB for `C = (f64, f64)`). The crate header
//! advertises a 16-qubit cap to keep RAM bounded; this module makes
//! that cap explicit and machine-checkable so v0.0.2 can wire it into
//! `simulate` (or any future MCP entry point) without rediscovering
//! the bound.
//!
//! v0.0 leaves `state_vector::simulate` untouched per the security-
//! review constraint of not editing existing substrate src files.
//! v0.0.2 should call [`enforce_qubit_cap`] at the entry of every
//! externally-reachable simulation path (the `mcp-ruqu` `ruqu_simulate`
//! tool, any future REST/gRPC handler, the bench scaffolding).

/// Maximum qubit count v0.0 will simulate. `1 << 16 = 65 536` complex
/// amplitudes ≈ 1 MiB at `f64` precision — comfortably under the
/// per-call cap any reasonable host enforces.
///
/// Production deployments that need 17–25 qubits should opt in
/// explicitly per ADR-008 §3 (the 25-qubit `ruqu-wasm` headroom is
/// referenced there as the production target).
pub const MAX_QUBITS_V0_0: u8 = 16;

/// Hard upper bound any future tier may set. `1 << 30` complex
/// amplitudes is ~16 GiB at `f64`; beyond this point the StateVector
/// kernel is not the right backend (use TensorNetwork or hardware).
///
/// `Circuit::dim()` already saturates `n_qubits` at 31 to keep the
/// `usize` shift safe on 32-bit hosts; this constant is a separate,
/// stricter ceiling that no v0.x release should ever exceed.
pub const MAX_QUBITS_HARD: u8 = 30;

/// Enforce the v0.0 qubit cap. Returns an error variant the caller
/// can map onto `RuLakeError::InvalidParameter` (or the future
/// `RUQU_QUBITS_EXCEEDED` MCP refusal code from ADR-008 §4).
///
/// Pure function — no I/O, no allocation, easy to call from any
/// simulation entry point without coupling to the backend struct.
pub fn enforce_qubit_cap(n_qubits: u8) -> Result<(), QubitCapError> {
    if n_qubits > MAX_QUBITS_V0_0 {
        Err(QubitCapError {
            requested: n_qubits,
            cap: MAX_QUBITS_V0_0,
        })
    } else {
        Ok(())
    }
}

/// Returned by [`enforce_qubit_cap`] when a circuit would breach the
/// v0.0 cap. Carries the requested width and the cap so error
/// messages can be specific.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QubitCapError {
    /// The `n_qubits` value the caller passed.
    pub requested: u8,
    /// The cap that was breached.
    pub cap: u8,
}

impl std::fmt::Display for QubitCapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ruqu v0.0 simulate: n_qubits={} exceeds cap={} (use TensorNetwork or hardware backend for wider circuits)",
            self.requested, self.cap
        )
    }
}

impl std::error::Error for QubitCapError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cap_accepts_at_boundary() {
        assert!(enforce_qubit_cap(0).is_ok());
        assert!(enforce_qubit_cap(MAX_QUBITS_V0_0).is_ok());
    }

    #[test]
    fn cap_rejects_above_boundary() {
        let err = enforce_qubit_cap(MAX_QUBITS_V0_0 + 1).unwrap_err();
        assert_eq!(err.requested, MAX_QUBITS_V0_0 + 1);
        assert_eq!(err.cap, MAX_QUBITS_V0_0);
    }

    #[test]
    fn hard_ceiling_is_not_self_contradictory() {
        // The advertised v0.0 cap must be at or below the hard ceiling.
        assert!(MAX_QUBITS_V0_0 <= MAX_QUBITS_HARD);
        // The hard ceiling must be strictly below the `dim()` saturation
        // point (31) so we never get within a factor of two of OOM on
        // a 32 GiB host.
        assert!(MAX_QUBITS_HARD < 31);
    }

    #[test]
    fn error_display_mentions_inputs() {
        let s = format!("{}", QubitCapError { requested: 25, cap: 16 });
        assert!(s.contains("25"));
        assert!(s.contains("16"));
    }
}
