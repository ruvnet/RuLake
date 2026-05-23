//! Aaronson–Gottesman stabilizer-tableau simulator.
//!
//! New in v0.1. Polynomial-time simulation of the Clifford fragment
//! (H, S, X, Y, Z, CX). The tableau has `2n + 1` rows on `n` qubits:
//! the first `n` rows are destabilizer generators, the next `n` are
//! stabilizer generators, and the last row is a scratch slot used by
//! the canonical measurement routine. Each row holds two `n`-bit
//! Pauli pieces (`x_i`, `z_i`) plus a single sign bit `r`.
//!
//! The reference is Aaronson & Gottesman, *Improved Simulation of
//! Stabilizer Circuits*, Phys. Rev. A 70, 052328 (2004). The update
//! rules below mirror Table I (gate updates) and Eq. (3) (the
//! `g(x1,z1,x2,z2)` exponent that bookkeeps the sign on row sums).
//!
//! v0.1 ships: H, S, X, Y, Z, CX gate updates; circuit driver that
//! refuses non-Clifford gates with a descriptive [`StabilizerError`];
//! tableau → `Vec<Vec<f32>>` packing for the v0.0 `PulledBatch.vectors`
//! layout described in `integration-with-rulake.md` §1.4 ("vectors[0]
//! is the tableau bit-packed into f32 words"). Witness equivalence
//! between StateVector and Stabilizer is a v0.2 deliverable (see the
//! `concordance_witness_equality` test marker in
//! `tests/stabilizer_smoke.rs`).
//!
//! No `unsafe`, no extra deps — bit-packing is hand-rolled over
//! `Vec<u64>` to keep the dependency surface flat.

use crate::circuit::{Circuit, Gate};

/// v0.1 cap. The tableau is `(2n + 1) × (2n + 1)` bits ≈ `O(n²/8)`
/// bytes; at `n = 32` that's ~528 bytes — trivial for any host. The
/// limit is here to keep packed-`f32` rows bounded in
/// [`to_pulled_batch_form`] and to give operators a single number to
/// reason about. Stabilizer can in principle handle thousands of
/// qubits efficiently; v0.2 will lift the cap once an MCP-side
/// resource budget is wired.
pub const MAX_QUBITS_V0_1: u8 = 32;

/// Errors the [`simulate`] driver can raise. Maps cleanly onto the
/// `RUQU_NON_CLIFFORD` audit code v2-spec.md §h reserves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StabilizerError {
    /// The circuit contains a gate the Stabilizer backend cannot
    /// represent. Carries the gate's canonical name so the audit
    /// trail and operator messages stay specific.
    NonClifford {
        /// Lower-snake-case gate name as it appears in the mini-IR
        /// (`"t"`, `"rz"`).
        gate: &'static str,
    },
    /// `n_qubits` exceeds the v0.1 cap. Distinct from the StateVector
    /// cap so operators can tell which backend's bound bit them.
    TooManyQubits {
        /// The `n_qubits` value the caller passed.
        requested: u8,
        /// The cap that was breached (currently [`MAX_QUBITS_V0_1`]).
        cap: u8,
    },
    /// A gate referred to a qubit index that's out of range for the
    /// circuit's `n_qubits`. Caught here (not in the simulator
    /// kernel) so the kernel stays branch-free on the fast path.
    QubitIndexOutOfRange {
        /// The offending qubit index from the gate.
        qubit: u8,
        /// `n_qubits` at the time of dispatch.
        n_qubits: u8,
    },
}

impl std::fmt::Display for StabilizerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NonClifford { gate } => write!(
                f,
                "ruqu stabilizer: refusing non-Clifford gate '{gate}' \
                 (Stabilizer backend supports H, S, X, Y, Z, CX only; \
                 use the StateVector or Clifford+T backend for circuits \
                 containing T or Rz)"
            ),
            Self::TooManyQubits { requested, cap } => write!(
                f,
                "ruqu stabilizer: n_qubits={requested} exceeds v0.1 cap={cap}"
            ),
            Self::QubitIndexOutOfRange { qubit, n_qubits } => write!(
                f,
                "ruqu stabilizer: qubit index {qubit} out of range for \
                 n_qubits={n_qubits}"
            ),
        }
    }
}

impl std::error::Error for StabilizerError {}

/// Bit-packed `(2n+1) × (2n+1)`-bit tableau. The layout mirrors the
/// canonical Aaronson–Gottesman presentation:
///
/// * Row `i ∈ 0..n` is the `i`-th destabilizer generator.
/// * Row `i ∈ n..2n` is the `(i - n)`-th stabilizer generator.
/// * Row `2n` is a scratch row used by the measurement routine.
///
/// Each row stores `n` bits of `x` (qubits 0..n), then `n` bits of
/// `z` (qubits 0..n), then 1 sign bit `r`. Rows are addressed by
/// index; bits within a row use a flat `0..(2n+1)` offset that
/// `bit_pos` maps to a `(word_index, bit_in_word)` pair.
#[derive(Debug, Clone)]
pub struct Tableau {
    n: usize,
    /// Words per row. `ceil((2n + 1) / 64)`.
    words_per_row: usize,
    /// Flat storage. `rows = 2n + 1`, total `rows * words_per_row`
    /// `u64` words. Indexing: `data[row * words_per_row + word_in_row]`.
    data: Vec<u64>,
}

impl Tableau {
    /// Allocate the canonical initial tableau for `|0…0⟩`. Per
    /// Aaronson–Gottesman: destabilizers are `X_i`, stabilizers are
    /// `Z_i`, all signs are `+1`, scratch row is zero.
    pub fn identity(n: usize) -> Self {
        let rows = 2 * n + 1;
        let bits_per_row = 2 * n + 1;
        let words_per_row = bits_per_row.div_ceil(64);
        let mut t = Self {
            n,
            words_per_row,
            data: vec![0u64; rows * words_per_row],
        };
        // Destabilizer i: X_i (so x_i = 1, z_i = 0, r = 0).
        for i in 0..n {
            t.set_bit(i, i, true); // x_{i,i}
        }
        // Stabilizer i: Z_i (so x_i = 0, z_i = 1, r = 0).
        for i in 0..n {
            t.set_bit(n + i, n + i, true); // z_{i,i}
        }
        t
    }

    /// `n_qubits` the tableau is sized for.
    pub fn n_qubits(&self) -> usize {
        self.n
    }

    /// Words per row — useful for callers serialising the raw storage.
    pub fn words_per_row(&self) -> usize {
        self.words_per_row
    }

    /// Total number of rows (`2n + 1`).
    pub fn rows(&self) -> usize {
        2 * self.n + 1
    }

    /// Borrow the raw packed storage. Layout is documented on
    /// [`Tableau`].
    pub fn raw(&self) -> &[u64] {
        &self.data
    }

    /// Read the bit at `(row, col)`. `col ∈ 0..n` reads `x_col`,
    /// `col ∈ n..2n` reads `z_{col - n}`, `col == 2n` reads the sign
    /// bit `r`.
    #[inline]
    pub fn bit(&self, row: usize, col: usize) -> bool {
        let (w, b) = self.bit_pos(row, col);
        (self.data[w] >> b) & 1 == 1
    }

    /// Write the bit at `(row, col)`. Same coordinate scheme as
    /// [`Self::bit`].
    #[inline]
    pub fn set_bit(&mut self, row: usize, col: usize, v: bool) {
        let (w, b) = self.bit_pos(row, col);
        if v {
            self.data[w] |= 1u64 << b;
        } else {
            self.data[w] &= !(1u64 << b);
        }
    }

    /// Toggle the bit at `(row, col)`. Common enough on tableau update
    /// hot paths to warrant a dedicated helper.
    #[inline]
    pub fn xor_bit(&mut self, row: usize, col: usize, v: bool) {
        if !v {
            return;
        }
        let (w, b) = self.bit_pos(row, col);
        self.data[w] ^= 1u64 << b;
    }

    /// Read the `x_{row, q}` bit. Convenience for the gate updates.
    #[inline]
    pub fn x(&self, row: usize, q: usize) -> bool {
        self.bit(row, q)
    }
    /// Read the `z_{row, q}` bit. Convenience for the gate updates.
    #[inline]
    pub fn z(&self, row: usize, q: usize) -> bool {
        self.bit(row, self.n + q)
    }
    /// Read the sign bit `r_row`. Convenience for the gate updates.
    #[inline]
    pub fn r(&self, row: usize) -> bool {
        self.bit(row, 2 * self.n)
    }

    /// Write the sign bit `r_row`.
    #[inline]
    pub fn set_r(&mut self, row: usize, v: bool) {
        self.set_bit(row, 2 * self.n, v);
    }

    #[inline]
    fn bit_pos(&self, row: usize, col: usize) -> (usize, usize) {
        let word = row * self.words_per_row + (col >> 6);
        let bit = col & 63;
        (word, bit)
    }
}

/// One simulated stabilizer state — a tableau plus the qubit count.
/// The `n_qubits: u8` mirrors [`crate::circuit::Circuit`] so the
/// adapter layer can round-trip state without a `usize`/`u8` dance.
#[derive(Debug, Clone)]
pub struct StabilizerState {
    /// Number of qubits the state is defined over.
    pub n_qubits: u8,
    /// The packed tableau.
    pub tableau: Tableau,
}

impl StabilizerState {
    /// `|0…0⟩` on `n` qubits.
    pub fn zero(n: u8) -> Self {
        Self {
            n_qubits: n,
            tableau: Tableau::identity(n as usize),
        }
    }
}

/// Apply a Hadamard on qubit `q`. Per Aaronson–Gottesman Table I:
/// for every row `i`, swap `x_{i,q}` with `z_{i,q}` and update the
/// sign by `r_i ⊕= x_{i,q} · z_{i,q}` (using the *new* values, i.e.
/// equivalently the old `x · z`).
pub fn apply_h(state: &mut StabilizerState, q: usize) {
    let t = &mut state.tableau;
    for i in 0..t.rows() {
        let xi = t.x(i, q);
        let zi = t.z(i, q);
        if xi & zi {
            t.xor_bit(i, 2 * t.n, true);
        }
        // swap x_{i,q} ↔ z_{i,q}
        t.set_bit(i, q, zi);
        t.set_bit(i, t.n + q, xi);
    }
}

/// Apply a phase gate `S` on qubit `q`. Per Aaronson–Gottesman:
/// `r_i ⊕= x_{i,q} · z_{i,q}`, then `z_{i,q} ⊕= x_{i,q}`.
pub fn apply_s(state: &mut StabilizerState, q: usize) {
    let t = &mut state.tableau;
    for i in 0..t.rows() {
        let xi = t.x(i, q);
        let zi = t.z(i, q);
        if xi & zi {
            t.xor_bit(i, 2 * t.n, true);
        }
        if xi {
            t.xor_bit(i, t.n + q, true);
        }
    }
}

/// Apply a CNOT with `control` and `target` distinct qubits. Per
/// Aaronson–Gottesman: for every row `i`,
/// `r_i ⊕= x_{i,c} · z_{i,t} · (x_{i,t} ⊕ z_{i,c} ⊕ 1)`,
/// `x_{i,t} ⊕= x_{i,c}`, `z_{i,c} ⊕= z_{i,t}`.
pub fn apply_cx(state: &mut StabilizerState, control: usize, target: usize) {
    let t = &mut state.tableau;
    for i in 0..t.rows() {
        let xc = t.x(i, control);
        let zc = t.z(i, control);
        let xt = t.x(i, target);
        let zt = t.z(i, target);
        if xc & zt & (xt ^ zc ^ true) {
            t.xor_bit(i, 2 * t.n, true);
        }
        if xc {
            t.xor_bit(i, target, true);
        }
        if zt {
            t.xor_bit(i, t.n + control, true);
        }
    }
}

/// Apply Pauli-X on `q`. `X = H · Z · H`; the tableau update
/// reduces to `r_i ⊕= z_{i,q}`.
pub fn apply_x(state: &mut StabilizerState, q: usize) {
    let t = &mut state.tableau;
    for i in 0..t.rows() {
        if t.z(i, q) {
            t.xor_bit(i, 2 * t.n, true);
        }
    }
}

/// Apply Pauli-Z on `q`. The tableau update is `r_i ⊕= x_{i,q}`.
pub fn apply_z(state: &mut StabilizerState, q: usize) {
    let t = &mut state.tableau;
    for i in 0..t.rows() {
        if t.x(i, q) {
            t.xor_bit(i, 2 * t.n, true);
        }
    }
}

/// Apply Pauli-Y on `q`. `Y = i · X · Z`; the tableau update is
/// `r_i ⊕= x_{i,q} ⊕ z_{i,q}`.
pub fn apply_y(state: &mut StabilizerState, q: usize) {
    let t = &mut state.tableau;
    for i in 0..t.rows() {
        if t.x(i, q) ^ t.z(i, q) {
            t.xor_bit(i, 2 * t.n, true);
        }
    }
}

/// Run `circuit` on a fresh `|0…0⟩` stabilizer state. Returns the
/// final [`StabilizerState`] or a [`StabilizerError`] if the circuit
/// contains a non-Clifford gate.
pub fn simulate(circuit: &Circuit) -> Result<StabilizerState, StabilizerError> {
    if circuit.n_qubits > MAX_QUBITS_V0_1 {
        return Err(StabilizerError::TooManyQubits {
            requested: circuit.n_qubits,
            cap: MAX_QUBITS_V0_1,
        });
    }
    let n = circuit.n_qubits;
    let mut state = StabilizerState::zero(n);

    let in_range = |q: u8| -> Result<usize, StabilizerError> {
        if q >= n {
            Err(StabilizerError::QubitIndexOutOfRange {
                qubit: q,
                n_qubits: n,
            })
        } else {
            Ok(q as usize)
        }
    };

    for g in &circuit.gates {
        match *g {
            Gate::H { q } => apply_h(&mut state, in_range(q)?),
            Gate::S { q } => apply_s(&mut state, in_range(q)?),
            Gate::X { q } => apply_x(&mut state, in_range(q)?),
            Gate::Y { q } => apply_y(&mut state, in_range(q)?),
            Gate::Z { q } => apply_z(&mut state, in_range(q)?),
            Gate::Cx { control, target } => {
                let c = in_range(control)?;
                let t = in_range(target)?;
                if c == t {
                    return Err(StabilizerError::QubitIndexOutOfRange {
                        qubit: control,
                        n_qubits: n,
                    });
                }
                apply_cx(&mut state, c, t);
            }
            Gate::T { .. } => return Err(StabilizerError::NonClifford { gate: "t" }),
            Gate::Rz { .. } => return Err(StabilizerError::NonClifford { gate: "rz" }),
        }
    }
    Ok(state)
}

/// Pack a stabilizer state into the `Vec<Vec<f32>>` layout used by
/// `PulledBatch.vectors`. One row per stabilizer / destabilizer per
/// the integration-with-rulake.md §1.4 (line 225-ish) convention:
/// each row is `[coeff_re, coeff_im, x_0, …, x_{n-1}, z_0, …,
/// z_{n-1}, r]` with the coefficient slots pinned to `[1.0, 0.0]`
/// (Stabilizer rows carry no continuous amplitude). The output has
/// `2n + 1` rows so the scratch row round-trips for forensic
/// inspection.
///
/// The witness chain still depends only on the witness-input fields
/// in [`crate::witness`]; this packing is for the `PulledBatch`
/// payload that ruLake compresses into RaBitQ codes.
pub fn to_pulled_batch_form(state: &StabilizerState) -> Vec<Vec<f32>> {
    let n = state.n_qubits as usize;
    let row_len = 2 + 2 * n + 1;
    let mut out = Vec::with_capacity(state.tableau.rows());
    for row in 0..state.tableau.rows() {
        let mut v = Vec::with_capacity(row_len);
        v.push(1.0_f32);
        v.push(0.0_f32);
        for q in 0..n {
            v.push(if state.tableau.x(row, q) { 1.0 } else { 0.0 });
        }
        for q in 0..n {
            v.push(if state.tableau.z(row, q) { 1.0 } else { 0.0 });
        }
        v.push(if state.tableau.r(row) { 1.0 } else { 0.0 });
        out.push(v);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::circuit::{Circuit, Gate};

    #[test]
    fn identity_tableau_has_canonical_layout() {
        let t = Tableau::identity(3);
        // Destabilizer i should have x_{i,i} = 1, all other bits 0.
        for i in 0..3 {
            assert!(t.x(i, i));
            for j in 0..3 {
                if j != i {
                    assert!(!t.x(i, j));
                }
            }
            for j in 0..3 {
                assert!(!t.z(i, j));
            }
            assert!(!t.r(i));
        }
        // Stabilizer i should have z_{i,i} = 1.
        for i in 0..3 {
            assert!(t.z(3 + i, i));
            for j in 0..3 {
                if j != i {
                    assert!(!t.z(3 + i, j));
                }
            }
            for j in 0..3 {
                assert!(!t.x(3 + i, j));
            }
            assert!(!t.r(3 + i));
        }
    }

    #[test]
    fn h_on_zero_swaps_x_and_z_for_qubit() {
        let mut s = StabilizerState::zero(1);
        apply_h(&mut s, 0);
        // After H: stabilizer becomes X (x=1, z=0); destabilizer becomes Z.
        assert!(s.tableau.x(1, 0));
        assert!(!s.tableau.z(1, 0));
        assert!(!s.tableau.x(0, 0));
        assert!(s.tableau.z(0, 0));
    }

    #[test]
    fn cx_propagates_x_and_z() {
        // After CX(0,1) on |0⟩|0⟩, the stabilizers become Z_0 Z_1, Z_1
        // (the second stabilizer is unchanged because z_t propagates
        // back into z_c only when z_t was already set).
        let mut s = StabilizerState::zero(2);
        apply_cx(&mut s, 0, 1);
        // Stabilizer 0 was Z_0 (z=10), Z propagates to z=11 via the
        // z_c ^= z_t rule when z_t = 1; here z_t was 0 so unchanged.
        // The interesting case is the destabilizer: X_0 → X_0 X_1.
        assert!(s.tableau.x(0, 0));
        assert!(s.tableau.x(0, 1)); // x_t propagated by xc=1
    }

    #[test]
    fn simulate_refuses_t() {
        let mut c = Circuit::new("has-t", 1);
        c.push(Gate::T { q: 0 });
        let err = simulate(&c).unwrap_err();
        assert!(matches!(err, StabilizerError::NonClifford { gate: "t" }));
    }

    #[test]
    fn simulate_refuses_rz() {
        let mut c = Circuit::new("has-rz", 1);
        c.push(Gate::Rz { q: 0, theta: 0.5 });
        assert!(matches!(
            simulate(&c).unwrap_err(),
            StabilizerError::NonClifford { gate: "rz" },
        ));
    }

    #[test]
    fn simulate_caps_qubits() {
        let c = Circuit::new("too-wide", MAX_QUBITS_V0_1 + 1);
        assert!(matches!(
            simulate(&c).unwrap_err(),
            StabilizerError::TooManyQubits { .. },
        ));
    }

    #[test]
    fn simulate_bell_runs_clean() {
        let mut c = Circuit::new("bell-stab", 2);
        c.push(Gate::H { q: 0 });
        c.push(Gate::Cx {
            control: 0,
            target: 1,
        });
        let s = simulate(&c).unwrap();
        // After H on q0 then CX(0,1) starting from Z_0, Z_1:
        // stabilizers should be X_0 X_1 and Z_0 Z_1.
        assert!(s.tableau.x(2, 0) && s.tableau.x(2, 1));
        assert!(s.tableau.z(3, 0) && s.tableau.z(3, 1));
    }

    #[test]
    fn pulled_batch_form_has_expected_shape() {
        let s = StabilizerState::zero(2);
        let v = to_pulled_batch_form(&s);
        assert_eq!(v.len(), 2 * 2 + 1);
        for row in &v {
            assert_eq!(row.len(), 2 + 2 * 2 + 1);
            assert_eq!(row[0], 1.0);
            assert_eq!(row[1], 0.0);
        }
    }
}
