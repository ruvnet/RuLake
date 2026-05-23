//! Tiny mini-IR for v0.0 — enough to demonstrate the trust chain
//! end-to-end without pulling in a full OpenQASM parser. v0.0.2 will
//! add an OpenQASM 3.0 frontend per ADR-008 §6.

use serde::{Deserialize, Serialize};

/// One quantum gate operation. v0.0 covers Clifford + T + RZ; that's
/// universal for unitary computation when combined.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Gate {
    /// Hadamard on `q`.
    H {
        /// Qubit index (0-based).
        q: u8,
    },
    /// Pauli-X on `q`.
    X {
        /// Qubit index (0-based).
        q: u8,
    },
    /// Pauli-Y on `q`.
    Y {
        /// Qubit index (0-based).
        q: u8,
    },
    /// Pauli-Z on `q`.
    Z {
        /// Qubit index (0-based).
        q: u8,
    },
    /// S = diag(1, i) on `q`.
    S {
        /// Qubit index (0-based).
        q: u8,
    },
    /// T = diag(1, e^{iπ/4}) on `q`.
    T {
        /// Qubit index (0-based).
        q: u8,
    },
    /// Z-axis rotation by `theta` radians on `q`.
    ///
    /// **v0.0 caveat (security finding R-3):** `theta` is NOT bound
    /// into the witness chain in v0.0 — only `Circuit.id` flows into
    /// `data_ref`. Two circuits with the same id but different `theta`
    /// values therefore collide on the same witness. v0.0.2 will hash
    /// gate parameters into a content-derived `data_ref` to close
    /// this. For now: callers MUST set `Circuit.id` from a hash of
    /// the program source if they care about replay determinism.
    Rz {
        /// Qubit index (0-based).
        q: u8,
        /// Rotation angle in radians.
        theta: f64,
    },
    /// CNOT with `control` and `target` distinct qubits.
    Cx {
        /// Control qubit index (0-based).
        control: u8,
        /// Target qubit index (0-based).
        target: u8,
    },
}

/// A compiled circuit. v0.0 stores gates verbatim — no rewrites,
/// no scheduling. v0.1 will add a planner per ADR-008 §3.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Circuit {
    /// Number of qubits. State vector dim is `1 << n_qubits`.
    pub n_qubits: u8,
    /// Operations applied left-to-right.
    pub gates: Vec<Gate>,
    /// Stable identifier flowing into the witness `data_ref`.
    /// Operators set this from a content hash of the source program.
    pub id: String,
}

impl Circuit {
    /// Empty circuit on `n` qubits.
    pub fn new(id: impl Into<String>, n_qubits: u8) -> Self {
        Self {
            id: id.into(),
            n_qubits,
            gates: Vec::new(),
        }
    }

    /// Push a gate. Returns `&mut self` for ergonomic chaining.
    pub fn push(&mut self, g: Gate) -> &mut Self {
        self.gates.push(g);
        self
    }

    /// Hilbert-space dimension. `1 << n_qubits`. Saturates at 31 to
    /// keep the math `usize`-safe on 32-bit hosts (v0.0 caps at 16).
    pub fn dim(&self) -> usize {
        let n = self.n_qubits.min(31);
        1usize << n
    }
}
