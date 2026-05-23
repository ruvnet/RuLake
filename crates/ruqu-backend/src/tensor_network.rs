//! Matrix Product State (MPS) tensor-network simulator.
//!
//! New in **v0.2**. The state on `n` qubits is a chain of `n` rank-3
//! tensors `T_q ∈ ℂ^{χ_L × 2 × χ_R}` where `χ` is the *bond
//! dimension*. The full amplitude
//!
//! ```text
//! ⟨s_0 … s_{n-1}|ψ⟩ = ∑_{α_0…α_n} T_0[α_0, s_0, α_1] · T_1[α_1, s_1, α_2] · … · T_{n-1}[α_{n-1}, s_{n-1}, α_n]
//! ```
//!
//! is the contraction of the chain over the bond indices. χ=1 is a
//! product state; the Bell pair needs χ=2; generic n-qubit states
//! need χ up to `2^{n/2}` (then the MPS is no cheaper than a state
//! vector). The MPS shines on **low-entanglement** circuits — which
//! is most of what near-term quantum hardware actually runs — where a
//! small `bond_dim_cap` retains essentially all the amplitude weight.
//!
//! ### Gate application
//!
//! * **Single-qubit gate** on qubit `q`: contract the 2×2 matrix into
//!   the physical leg of `T_q`. Cost `O(χ²)`. No truncation needed —
//!   single-qubit gates can't grow entanglement.
//! * **Two-qubit gate** on adjacent qubits `(q, q+1)`:
//!     1. Merge `T_q [χL, 2, χM]` and `T_{q+1} [χM, 2, χR]` into a
//!        single rank-4 tensor `M [χL, 2, 2, χR]` by contracting on
//!        the shared bond `χM`.
//!     2. Apply the 4×4 gate on the two physical legs.
//!     3. Reshape to a `(χL · 2) × (2 · χR)` matrix `A`.
//!     4. SVD: `A = U · Σ · Vᴴ`. Singular values capture the
//!        entanglement across the bond.
//!     5. Truncate to the top `χ ≤ bond_dim_cap` singular values and
//!        absorb `√Σ` symmetrically into `U` and `Vᴴ`.
//!     6. Reshape back to `T_q' [χL, 2, χ_new]` and `T_{q+1}'
//!        [χ_new, 2, χR]`. The discarded singular values are the
//!        **truncation error** for this gate.
//! * **Two-qubit gate on non-adjacent qubits**: SWAP-decompose. We
//!   shuttle `q'` to be adjacent to `q` via a chain of nearest-
//!   neighbour SWAPs, apply the gate, then SWAP back. Each SWAP is a
//!   two-qubit gate of its own, so the bond dimension can grow along
//!   the path; this is the price of long-range entanglement on a 1D
//!   chain.
//!
//! ### SVD
//!
//! We need a complex SVD on a small dense matrix (typically
//! `≤ 2χ × 2χ`, so ≤ 64×64 at `χ=32`). To keep the dependency surface
//! flat we hand-roll a Hermitian-eigendecomposition path (Jacobi
//! rotations on `Aᴴ A`). Adequate for the matrix sizes that arise on
//! the MPS hot path; an `ndarray-linalg` swap-in is a v0.3 perf knob.
//!
//! ### Witness convention
//!
//! [`crate::witness::inputs_for`] takes a backend tag `"tensor_network"`
//! that flows into the bundle's `data_ref`. v0.2 keeps the per-
//! backend tag in place, so the same circuit on StateVector and
//! TensorNetwork produces *distinct* witnesses (cache discriminates
//! the backends). The concordance-mode strip per ADR-008 §G4 is still
//! a v0.3 deliverable.

use ndarray::{s, Array1, Array2, Array3, Array4};

use crate::circuit::{Circuit, Gate};
use crate::state_vector::C;

/// v0.2 cap on `n_qubits` for the TensorNetwork backend. The MPS data
/// structure itself scales linearly in `n` at fixed bond dim, but the
/// adapter packs every site tensor into a `Vec<f32>` row for ruLake's
/// cache; at `n = 256` and `bond_dim_cap = 64` the per-state payload
/// already crosses 1 MiB. We pin a conservative ceiling for v0.2 and
/// will lift it in v0.3 alongside an MCP-side resource budget.
pub const MAX_QUBITS_TN: u8 = 128;

/// Default cap when the caller doesn't pin one. χ=16 captures most
/// "low-entanglement" circuits the operator literature talks about
/// (1D Ising trotterised dynamics, brick-wall random circuits at
/// shallow depth, etc.) without breaking the bank on 64-qubit states.
pub const DEFAULT_BOND_DIM: usize = 16;

use std::f64::consts::FRAC_1_SQRT_2;

/// One MPS site tensor — rank 3 with axes
/// `[bond_left, physical (=2), bond_right]`.
///
/// Stored as an [`Array3<C>`]; the `C` complex type comes from
/// [`crate::state_vector`] so the witness chain doesn't fork into a
/// new "complex"-flavoured type. v0.2 uses `f64` precision throughout
/// and only down-casts to `f32` at the [`to_pulled_batch_form`] edge
/// (matching the StateVector adapter convention).
pub type MpsTensor = Array3<C>;

/// Errors the MPS [`simulate`] driver can raise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TensorNetworkError {
    /// `n_qubits` exceeds the v0.2 cap.
    TooManyQubits {
        /// The `n_qubits` value the caller passed.
        requested: u8,
        /// The cap that was breached (currently [`MAX_QUBITS_TN`]).
        cap: u8,
    },
    /// `bond_dim_cap` was zero — bond dimension must be at least 1
    /// (a product state). We refuse zero so the contraction loop has
    /// no empty-axis edge cases.
    ZeroBondDim,
    /// A gate referred to a qubit index out of range.
    QubitIndexOutOfRange {
        /// The offending qubit index.
        qubit: u8,
        /// `n_qubits` at the time of dispatch.
        n_qubits: u8,
    },
    /// A two-qubit gate's control and target are the same qubit.
    SameQubit {
        /// The duplicated qubit index.
        qubit: u8,
    },
}

impl std::fmt::Display for TensorNetworkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooManyQubits { requested, cap } => write!(
                f,
                "ruqu tensor_network: n_qubits={requested} exceeds v0.2 cap={cap}"
            ),
            Self::ZeroBondDim => write!(
                f,
                "ruqu tensor_network: bond_dim_cap must be ≥ 1 (a product state has χ=1)"
            ),
            Self::QubitIndexOutOfRange { qubit, n_qubits } => write!(
                f,
                "ruqu tensor_network: qubit index {qubit} out of range for n_qubits={n_qubits}"
            ),
            Self::SameQubit { qubit } => write!(
                f,
                "ruqu tensor_network: control and target are the same qubit ({qubit})"
            ),
        }
    }
}

impl std::error::Error for TensorNetworkError {}

/// A full MPS state. `tensors[q]` has shape `[χ_L, 2, χ_R]`; the
/// `χ_L` of `tensors[0]` and the `χ_R` of `tensors[n-1]` are both 1
/// (open boundary conditions).
#[derive(Debug, Clone)]
pub struct Mps {
    /// One rank-3 tensor per qubit. `tensors.len() == n_qubits`.
    pub tensors: Vec<MpsTensor>,
    /// Number of qubits the state is defined over.
    pub n_qubits: u8,
    /// Truncation cap: SVDs after each two-qubit gate keep at most
    /// this many singular values per bond.
    pub bond_dim_cap: usize,
    /// Sum of (squared) discarded singular weight across every
    /// truncation since construction. Tracks the "fidelity loss" the
    /// simulator absorbed; tests use this to assert error stays
    /// bounded for low-entanglement circuits.
    pub truncation_error: f64,
}

impl Mps {
    /// Build the product state `|0…0⟩` on `n` qubits. Every tensor
    /// has shape `[1, 2, 1]` with amplitude 1 on the `|0⟩` slot.
    pub fn zero_state(n: u8, bond_dim_cap: usize) -> Self {
        let mut tensors = Vec::with_capacity(n as usize);
        for _ in 0..n {
            let mut t = Array3::<C>::default((1, 2, 1));
            t[[0, 0, 0]] = C::one();
            tensors.push(t);
        }
        Self {
            tensors,
            n_qubits: n,
            bond_dim_cap,
            truncation_error: 0.0,
        }
    }

    /// Apply a 2×2 gate on qubit `q`. Contracts the matrix into the
    /// physical axis of `tensors[q]`. `O(χ²)` work; no truncation.
    pub fn apply_1q(&mut self, q: u8, gate: [[C; 2]; 2]) {
        let t = &mut self.tensors[q as usize];
        let (cl, _ph, cr) = (t.dim().0, t.dim().1, t.dim().2);
        // out[l, s, r] = ∑_{s'} gate[s, s'] · t[l, s', r].
        let mut out = Array3::<C>::default((cl, 2, cr));
        for l in 0..cl {
            for r in 0..cr {
                let v0 = t[[l, 0, r]];
                let v1 = t[[l, 1, r]];
                out[[l, 0, r]] = gate[0][0] * v0 + gate[0][1] * v1;
                out[[l, 1, r]] = gate[1][0] * v0 + gate[1][1] * v1;
            }
        }
        *t = out;
    }

    /// Apply a 4×4 gate on adjacent qubits `(q, q+1)`. Truncates the
    /// resulting bond back to `bond_dim_cap`, accumulating the
    /// discarded singular weight into [`Self::truncation_error`].
    ///
    /// Returns the discarded singular weight for this call so callers
    /// can build per-step error budgets if they want.
    pub fn apply_2q_adjacent(&mut self, q: u8, gate: [[C; 4]; 4]) -> f64 {
        let q = q as usize;
        let (lt, rt) = self.tensors.split_at_mut(q + 1);
        let left = &mut lt[q];
        let right = &mut rt[0];
        let cl = left.dim().0;
        let cm = left.dim().2;
        debug_assert_eq!(cm, right.dim().0);
        let cr = right.dim().2;

        // Merge into a single rank-4 tensor M[l, s1, s2, r] by
        // contracting on the shared bond cm.
        let mut merged = Array4::<C>::default((cl, 2, 2, cr));
        for l in 0..cl {
            for r in 0..cr {
                for s1 in 0..2 {
                    for s2 in 0..2 {
                        let mut acc = C::zero();
                        for m in 0..cm {
                            acc = acc + left[[l, s1, m]] * right[[m, s2, r]];
                        }
                        merged[[l, s1, s2, r]] = acc;
                    }
                }
            }
        }

        // Apply the 4×4 gate on (s1, s2).
        // Index convention: idx = s1 * 2 + s2 ∈ {0..4}.
        let mut after = Array4::<C>::default((cl, 2, 2, cr));
        for l in 0..cl {
            for r in 0..cr {
                let v: [C; 4] = [
                    merged[[l, 0, 0, r]],
                    merged[[l, 0, 1, r]],
                    merged[[l, 1, 0, r]],
                    merged[[l, 1, 1, r]],
                ];
                for (i, gate_row) in gate.iter().enumerate() {
                    let mut acc = C::zero();
                    for j in 0..4 {
                        acc = acc + gate_row[j] * v[j];
                    }
                    let s1 = i >> 1;
                    let s2 = i & 1;
                    after[[l, s1, s2, r]] = acc;
                }
            }
        }

        // Reshape to a (cl*2) × (2*cr) matrix A and SVD-truncate.
        let rows = cl * 2;
        let cols = 2 * cr;
        let mut a = Array2::<C>::default((rows, cols));
        for l in 0..cl {
            for s1 in 0..2 {
                for s2 in 0..2 {
                    for r in 0..cr {
                        a[[l * 2 + s1, s2 * cr + r]] = after[[l, s1, s2, r]];
                    }
                }
            }
        }

        let (u, sigma, vt) = svd_complex(&a);
        let cap = self.bond_dim_cap.max(1);
        let keep = sigma.len().min(cap);
        // Discarded squared singular weight (= fidelity loss for an
        // initially normalised state, modulo per-bond renormalisation).
        let discarded: f64 = sigma.iter().skip(keep).map(|s| s * s).sum();
        self.truncation_error += discarded;

        // Absorb √Σ symmetrically. This keeps both new tensors at a
        // similar scale; canonical-form aficionados will rebalance,
        // but for v0.2 the symmetric split is fine — the witness
        // chain only sees the contracted amplitudes.
        let mut new_left = Array3::<C>::default((cl, 2, keep));
        let mut new_right = Array3::<C>::default((keep, 2, cr));
        for k in 0..keep {
            let s_sqrt = sigma[k].sqrt();
            for l in 0..cl {
                for s1 in 0..2 {
                    let u_val = u[[l * 2 + s1, k]];
                    new_left[[l, s1, k]] = scale_real(u_val, s_sqrt);
                }
            }
            for s2 in 0..2 {
                for r in 0..cr {
                    let vt_val = vt[[k, s2 * cr + r]];
                    new_right[[k, s2, r]] = scale_real(vt_val, s_sqrt);
                }
            }
        }

        *left = new_left;
        *right = new_right;
        discarded
    }

    /// Apply a 4×4 two-qubit gate to a possibly non-adjacent qubit
    /// pair `(q1, q2)` by SWAPping `q2` next to `q1`, applying the
    /// gate, then SWAPping back. Each intermediate SWAP is itself a
    /// two-qubit gate that can grow the bond dimension; we rely on
    /// the bond cap to keep growth bounded.
    pub fn apply_2q(&mut self, q1: u8, q2: u8, gate: [[C; 4]; 4]) -> f64 {
        if q1 == q2 {
            return 0.0;
        }
        let (lo, hi, swap_orientation) = if q1 < q2 {
            (q1, q2, false)
        } else {
            (q2, q1, true)
        };
        // Bring qubit `hi` down to position `lo + 1` via SWAPs.
        let mut err_acc = 0.0;
        for k in (lo + 1..hi).rev() {
            err_acc += self.apply_2q_adjacent(k, swap_gate());
        }
        // Now the gate's two operands sit on (lo, lo+1). If the
        // caller's q1 > q2 we have to swap operands of the gate so
        // (s1, s2) means (q1, q2) the way the caller expects.
        let final_gate = if swap_orientation {
            swap_gate_axes(&gate)
        } else {
            gate
        };
        err_acc += self.apply_2q_adjacent(lo, final_gate);
        // SWAP everything back.
        for k in lo + 1..hi {
            err_acc += self.apply_2q_adjacent(k, swap_gate());
        }
        err_acc
    }

    /// Contract the entire MPS into a dense state vector. Used for
    /// tests and small-N forensics; cost is `O(2^n · χ²)`. Don't call
    /// at large `n`.
    pub fn to_state_vector(&self) -> Vec<C> {
        let n = self.n_qubits as usize;
        let dim = 1usize << n;
        let mut out = vec![C::zero(); dim];
        for (basis, slot) in out.iter_mut().enumerate() {
            // Contract along the chain for this basis state.
            let mut row = Array2::<C>::default((1, 1));
            row[[0, 0]] = C::one();
            for q in 0..n {
                let s = (basis >> q) & 1;
                let t = &self.tensors[q];
                let cl = t.dim().0;
                let cr = t.dim().2;
                // Slice physical leg = s.
                let slice = t.slice(s![.., s, ..]);
                debug_assert_eq!(row.dim().1, cl);
                let mut next = Array2::<C>::default((row.dim().0, cr));
                for i in 0..row.dim().0 {
                    for j in 0..cr {
                        let mut acc = C::zero();
                        for k in 0..cl {
                            acc = acc + row[[i, k]] * slice[[k, j]];
                        }
                        next[[i, j]] = acc;
                    }
                }
                row = next;
            }
            *slot = row[[0, 0]];
        }
        out
    }
}

/// Run `circuit` from `|0…0⟩` with the given bond cap. Returns the
/// final [`Mps`] for caller inspection (amplitudes, bond dims,
/// accumulated truncation error).
pub fn simulate(circuit: &Circuit, bond_dim_cap: usize) -> Result<Mps, TensorNetworkError> {
    if circuit.n_qubits > MAX_QUBITS_TN {
        return Err(TensorNetworkError::TooManyQubits {
            requested: circuit.n_qubits,
            cap: MAX_QUBITS_TN,
        });
    }
    if bond_dim_cap == 0 {
        return Err(TensorNetworkError::ZeroBondDim);
    }
    let n = circuit.n_qubits;
    let mut mps = Mps::zero_state(n, bond_dim_cap);

    let in_range = |q: u8| -> Result<u8, TensorNetworkError> {
        if q >= n {
            Err(TensorNetworkError::QubitIndexOutOfRange {
                qubit: q,
                n_qubits: n,
            })
        } else {
            Ok(q)
        }
    };

    for g in &circuit.gates {
        match *g {
            Gate::H { q } => {
                let _ = in_range(q)?;
                mps.apply_1q(
                    q,
                    [
                        [C::new(FRAC_1_SQRT_2, 0.0), C::new(FRAC_1_SQRT_2, 0.0)],
                        [C::new(FRAC_1_SQRT_2, 0.0), C::new(-FRAC_1_SQRT_2, 0.0)],
                    ],
                );
            }
            Gate::X { q } => {
                let _ = in_range(q)?;
                mps.apply_1q(q, [[C::zero(), C::one()], [C::one(), C::zero()]]);
            }
            Gate::Y { q } => {
                let _ = in_range(q)?;
                mps.apply_1q(
                    q,
                    [
                        [C::zero(), C::new(0.0, -1.0)],
                        [C::new(0.0, 1.0), C::zero()],
                    ],
                );
            }
            Gate::Z { q } => {
                let _ = in_range(q)?;
                mps.apply_1q(q, [[C::one(), C::zero()], [C::zero(), C::new(-1.0, 0.0)]]);
            }
            Gate::S { q } => {
                let _ = in_range(q)?;
                mps.apply_1q(q, [[C::one(), C::zero()], [C::zero(), C::new(0.0, 1.0)]]);
            }
            Gate::T { q } => {
                let _ = in_range(q)?;
                let phase = C::new(FRAC_1_SQRT_2, FRAC_1_SQRT_2); // e^{iπ/4}
                mps.apply_1q(q, [[C::one(), C::zero()], [C::zero(), phase]]);
            }
            Gate::Rz { q, theta } => {
                let _ = in_range(q)?;
                let half = theta / 2.0;
                let neg = C::new(half.cos(), -half.sin());
                let pos = C::new(half.cos(), half.sin());
                mps.apply_1q(q, [[neg, C::zero()], [C::zero(), pos]]);
            }
            Gate::Cx { control, target } => {
                let _ = in_range(control)?;
                let _ = in_range(target)?;
                if control == target {
                    return Err(TensorNetworkError::SameQubit { qubit: control });
                }
                mps.apply_2q(control, target, cnot_gate(control < target));
            }
        }
    }
    Ok(mps)
}

/// Pack an MPS into the `Vec<Vec<f32>>` layout used by
/// `PulledBatch.vectors`. One row per site tensor; each row is
/// `[χL, χR, re_0, im_0, re_1, im_1, …]` where the leading two
/// entries are the bond dimensions of that site (rounded down to
/// `f32`; useful for downstream introspection without needing to
/// reparse). All rows are zero-padded to the maximum site size so
/// ruLake's RaBitQ layer sees a uniform `dim`.
pub fn to_pulled_batch_form(mps: &Mps) -> Vec<Vec<f32>> {
    // Determine max payload size = 2 + 2 * max(χL · 2 · χR).
    let max_elems = mps
        .tensors
        .iter()
        .map(|t| t.dim().0 * t.dim().1 * t.dim().2)
        .max()
        .unwrap_or(0);
    let row_len = 2 + 2 * max_elems;
    let mut out = Vec::with_capacity(mps.tensors.len());
    for t in &mps.tensors {
        let mut row = Vec::with_capacity(row_len);
        row.push(t.dim().0 as f32);
        row.push(t.dim().2 as f32);
        for &c in t.iter() {
            row.push(c.re as f32);
            row.push(c.im as f32);
        }
        // Zero-pad to row_len.
        while row.len() < row_len {
            row.push(0.0);
        }
        out.push(row);
    }
    out
}

// ---- private helpers ----

fn scale_real(c: C, s: f64) -> C {
    C::new(c.re * s, c.im * s)
}

/// `Cᴴ` — complex-conjugate, no transpose.
fn cconj(c: C) -> C {
    C::new(c.re, -c.im)
}

/// 4×4 SWAP gate. Standard basis order |s1 s2⟩ ∈ {|00⟩, |01⟩, |10⟩, |11⟩}.
fn swap_gate() -> [[C; 4]; 4] {
    let z = C::zero();
    let o = C::one();
    [[o, z, z, z], [z, z, o, z], [z, o, z, z], [z, z, z, o]]
}

/// CNOT in the basis order |s1 s2⟩ where the first index is the
/// "left" qubit on the chain. If `control_is_left` is true the
/// control sits on s1; otherwise we flip the gate so the control
/// stays semantically on the caller's `control` qubit.
fn cnot_gate(control_is_left: bool) -> [[C; 4]; 4] {
    let z = C::zero();
    let o = C::one();
    if control_is_left {
        // CNOT with control on s1, target on s2:
        // |00⟩→|00⟩, |01⟩→|01⟩, |10⟩→|11⟩, |11⟩→|10⟩.
        [[o, z, z, z], [z, o, z, z], [z, z, z, o], [z, z, o, z]]
    } else {
        // CNOT with control on s2, target on s1:
        // |00⟩→|00⟩, |01⟩→|11⟩, |10⟩→|10⟩, |11⟩→|01⟩.
        [[o, z, z, z], [z, z, z, o], [z, z, o, z], [z, o, z, z]]
    }
}

/// Swap the (s1, s2) physical leg roles of a 4×4 gate. Equivalent to
/// applying SWAP on either side: `G' = SWAP · G · SWAP`. Used by
/// `apply_2q` so the caller's (q1, q2) ordering is preserved when
/// `q1 > q2`.
fn swap_gate_axes(g: &[[C; 4]; 4]) -> [[C; 4]; 4] {
    // Permutation on basis indices: 0↔0, 1↔2, 3↔3 (i.e. swap bits).
    let perm = [0usize, 2, 1, 3];
    let mut out = [[C::zero(); 4]; 4];
    for i in 0..4 {
        for j in 0..4 {
            out[i][j] = g[perm[i]][perm[j]];
        }
    }
    out
}

/// Hand-rolled complex SVD: returns `(U, σ, Vᴴ)` for the input
/// `A : ℂ^{m×n}` with `σ` the singular values in non-increasing order
/// and only the leading `min(m, n)` columns / rows of U / Vᴴ retained
/// ("thin SVD"). Built via Hermitian eigendecomposition on the
/// smaller of `Aᴴ A` (n×n) or `A Aᴴ` (m×m), then back-solving for the
/// other factor. Adequate for the matrix sizes that arise on the MPS
/// hot path (≤ 64×64 at χ=32).
fn svd_complex(a: &Array2<C>) -> (Array2<C>, Vec<f64>, Array2<C>) {
    let (m, n) = a.dim();
    let k = m.min(n);
    if m == 0 || n == 0 {
        return (
            Array2::<C>::default((m, k)),
            Vec::new(),
            Array2::<C>::default((k, n)),
        );
    }

    if n <= m {
        // Diagonalise B = Aᴴ A (n × n, Hermitian PSD). Eigenvectors
        // give V; σ_i = √λ_i; U_i = A v_i / σ_i.
        let mut b = Array2::<C>::default((n, n));
        for i in 0..n {
            for j in 0..n {
                let mut acc = C::zero();
                for r in 0..m {
                    acc = acc + cconj(a[[r, i]]) * a[[r, j]];
                }
                b[[i, j]] = acc;
            }
        }
        let (eigvals, eigvecs) = jacobi_eigen_hermitian(&b);
        // Sort eigenpairs by descending eigenvalue.
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by(|&a, &b| {
            eigvals[b]
                .partial_cmp(&eigvals[a])
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut sigma = Vec::with_capacity(k);
        let mut v = Array2::<C>::default((n, k));
        let mut u = Array2::<C>::default((m, k));
        for (slot, &idx) in order.iter().take(k).enumerate() {
            let lam = eigvals[idx].max(0.0);
            let s = lam.sqrt();
            sigma.push(s);
            for i in 0..n {
                v[[i, slot]] = eigvecs[[i, idx]];
            }
            // u_slot = A v_slot / σ_slot (if σ ≈ 0, leave column as
            // zero; the corresponding singular direction has no
            // amplitude, so MPS contraction won't read it).
            if s > 1e-14 {
                for r in 0..m {
                    let mut acc = C::zero();
                    for i in 0..n {
                        acc = acc + a[[r, i]] * v[[i, slot]];
                    }
                    u[[r, slot]] = scale_real(acc, 1.0 / s);
                }
            }
        }
        // Vᴴ has rows = conjugate-transpose of v's columns.
        let mut vt = Array2::<C>::default((k, n));
        for i in 0..k {
            for j in 0..n {
                vt[[i, j]] = cconj(v[[j, i]]);
            }
        }
        (u, sigma, vt)
    } else {
        // m < n: diagonalise C = A Aᴴ (m × m). Eigenvectors give U;
        // σ_i = √λ_i; row i of Vᴴ = (1/σ_i) · uᴴ_i · A.
        let mut c_mat = Array2::<C>::default((m, m));
        for i in 0..m {
            for j in 0..m {
                let mut acc = C::zero();
                for r in 0..n {
                    acc = acc + a[[i, r]] * cconj(a[[j, r]]);
                }
                c_mat[[i, j]] = acc;
            }
        }
        let (eigvals, eigvecs) = jacobi_eigen_hermitian(&c_mat);
        let mut order: Vec<usize> = (0..m).collect();
        order.sort_by(|&a, &b| {
            eigvals[b]
                .partial_cmp(&eigvals[a])
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut sigma = Vec::with_capacity(k);
        let mut u = Array2::<C>::default((m, k));
        let mut vt = Array2::<C>::default((k, n));
        for (slot, &idx) in order.iter().take(k).enumerate() {
            let lam = eigvals[idx].max(0.0);
            let s = lam.sqrt();
            sigma.push(s);
            for i in 0..m {
                u[[i, slot]] = eigvecs[[i, idx]];
            }
            if s > 1e-14 {
                // Row of Vᴴ at slot = (1/s) · uᴴ_slot · A.
                for j in 0..n {
                    let mut acc = C::zero();
                    for i in 0..m {
                        acc = acc + cconj(u[[i, slot]]) * a[[i, j]];
                    }
                    vt[[slot, j]] = scale_real(acc, 1.0 / s);
                }
            }
        }
        (u, sigma, vt)
    }
}

/// Jacobi eigendecomposition for a complex Hermitian matrix `H`.
/// Returns `(eigenvalues, eigenvectors_columns)` where the i-th
/// eigenvector is `eigvecs.column(i)`. Uses cyclic Jacobi rotations
/// on real / complex off-diagonal pairs; `O(n³)` per sweep, converges
/// in `O(log n)` sweeps for the well-conditioned matrices we feed it.
fn jacobi_eigen_hermitian(h: &Array2<C>) -> (Array1<f64>, Array2<C>) {
    let n = h.dim().0;
    debug_assert_eq!(h.dim().1, n);
    let mut a = h.clone();
    // Initialise V = I.
    let mut v = Array2::<C>::default((n, n));
    for i in 0..n {
        v[[i, i]] = C::one();
    }

    let max_sweeps = 80;
    let tol = 1e-14;

    for _sweep in 0..max_sweeps {
        // Compute off-diagonal Frobenius norm to test convergence.
        let mut off = 0.0_f64;
        for i in 0..n {
            for j in (i + 1)..n {
                off += a[[i, j]].norm_sq();
            }
        }
        if off < tol {
            break;
        }

        for p in 0..n {
            for q in (p + 1)..n {
                let apq = a[[p, q]];
                if apq.norm_sq() < 1e-30 {
                    continue;
                }
                let app = a[[p, p]].re; // diagonal of Hermitian is real.
                let aqq = a[[q, q]].re;
                let modulus = apq.norm_sq().sqrt();
                // Phase factor so the rotated off-diagonal becomes
                // real: φ = apq / |apq|. Then rotate in the real
                // (p, q) plane by θ where tan(2θ) = 2|apq| / (app - aqq).
                let phase = C::new(apq.re / modulus, apq.im / modulus);
                let phase_conj = cconj(phase);
                let denom = app - aqq;
                let theta = if denom.abs() < 1e-30 {
                    std::f64::consts::FRAC_PI_4
                } else {
                    0.5 * (2.0 * modulus / denom).atan()
                };
                let c_t = theta.cos();
                let s_t = theta.sin();

                // Update diagonals.
                let new_app = c_t * c_t * app + s_t * s_t * aqq + 2.0 * s_t * c_t * modulus;
                let new_aqq = s_t * s_t * app + c_t * c_t * aqq - 2.0 * s_t * c_t * modulus;
                a[[p, p]] = C::new(new_app, 0.0);
                a[[q, q]] = C::new(new_aqq, 0.0);
                a[[p, q]] = C::zero();
                a[[q, p]] = C::zero();

                // Update off-diagonal entries in rows/cols p and q.
                for i in 0..n {
                    if i == p || i == q {
                        continue;
                    }
                    let aip = a[[i, p]];
                    let aiq = a[[i, q]];
                    // Rotation: aip' = c·aip + s·phase·aiq.
                    //           aiq' = -s·phase_conj·aip + c·aiq.
                    let new_aip = scale_real(aip, c_t) + scale_real(phase * aiq, s_t);
                    let new_aiq = scale_real(phase_conj * aip, -s_t) + scale_real(aiq, c_t);
                    a[[i, p]] = new_aip;
                    a[[p, i]] = cconj(new_aip);
                    a[[i, q]] = new_aiq;
                    a[[q, i]] = cconj(new_aiq);
                }

                // Update eigenvector matrix V.
                for i in 0..n {
                    let vip = v[[i, p]];
                    let viq = v[[i, q]];
                    let new_vip = scale_real(vip, c_t) + scale_real(phase * viq, s_t);
                    let new_viq = scale_real(phase_conj * vip, -s_t) + scale_real(viq, c_t);
                    v[[i, p]] = new_vip;
                    v[[i, q]] = new_viq;
                }
            }
        }
    }

    let mut eigvals = Array1::<f64>::zeros(n);
    for i in 0..n {
        eigvals[i] = a[[i, i]].re;
    }
    (eigvals, v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::circuit::{Circuit, Gate};

    fn approx(a: f64, b: f64, eps: f64) -> bool {
        (a - b).abs() < eps
    }

    #[test]
    fn zero_state_has_unit_amplitude_on_basis_zero() {
        let mps = Mps::zero_state(3, 4);
        let sv = mps.to_state_vector();
        assert_eq!(sv.len(), 8);
        assert!(approx(sv[0].re, 1.0, 1e-12));
        assert!(approx(sv[0].im, 0.0, 1e-12));
        for elem in sv.iter().take(8).skip(1) {
            assert!(approx(elem.re, 0.0, 1e-12));
            assert!(approx(elem.im, 0.0, 1e-12));
        }
    }

    #[test]
    fn single_h_creates_uniform_superposition_via_mps() {
        let mut c = Circuit::new("h-1q-mps", 1);
        c.push(Gate::H { q: 0 });
        let mps = simulate(&c, 4).unwrap();
        let sv = mps.to_state_vector();
        let amp = FRAC_1_SQRT_2;
        assert!(approx(sv[0].re, amp, 1e-10));
        assert!(approx(sv[1].re, amp, 1e-10));
    }

    #[test]
    fn bell_pair_at_bond_dim_two() {
        let mut c = Circuit::new("bell-mps", 2);
        c.push(Gate::H { q: 0 });
        c.push(Gate::Cx {
            control: 0,
            target: 1,
        });
        let mps = simulate(&c, 2).unwrap();
        let sv = mps.to_state_vector();
        let amp = FRAC_1_SQRT_2;
        assert!(
            approx(sv[0b00].re, amp, 1e-10),
            "|00⟩ amp = {}",
            sv[0b00].re
        );
        assert!(approx(sv[0b01].re, 0.0, 1e-10), "|01⟩ should be zero");
        assert!(approx(sv[0b10].re, 0.0, 1e-10), "|10⟩ should be zero");
        assert!(
            approx(sv[0b11].re, amp, 1e-10),
            "|11⟩ amp = {}",
            sv[0b11].re
        );
        assert!(
            mps.truncation_error < 1e-12,
            "Bell pair fits exactly at χ=2"
        );
    }

    #[test]
    fn t_gate_is_supported() {
        // Verify the TensorNetwork backend executes a T gate cleanly,
        // unlike the Stabilizer backend which refuses non-Clifford.
        let mut c = Circuit::new("h-t-h", 1);
        c.push(Gate::H { q: 0 });
        c.push(Gate::T { q: 0 });
        c.push(Gate::H { q: 0 });
        let mps = simulate(&c, 4).unwrap();
        let sv = mps.to_state_vector();
        // |0⟩ → H → (|0⟩+|1⟩)/√2 → T → (|0⟩+e^{iπ/4}|1⟩)/√2
        //      → H → (1+e^{iπ/4})/2 |0⟩ + (1-e^{iπ/4})/2 |1⟩.
        let p_zero = sv[0].norm_sq();
        let p_one = sv[1].norm_sq();
        // (|1+e^{iπ/4}|/2)² = (2 + 2cos(π/4))/4 ≈ 0.85355.
        assert!(approx(p_zero, 0.85355339, 1e-6), "p(0)={}", p_zero);
        assert!(approx(p_one, 0.14644661, 1e-6), "p(1)={}", p_one);
    }

    #[test]
    fn nonadjacent_cnot_via_swap_decomp() {
        // CNOT(0, 2) on |+00⟩ should give (|000⟩ + |101⟩)/√2.
        let mut c = Circuit::new("non-adj-cnot", 3);
        c.push(Gate::H { q: 0 });
        c.push(Gate::Cx {
            control: 0,
            target: 2,
        });
        let mps = simulate(&c, 4).unwrap();
        let sv = mps.to_state_vector();
        let amp = FRAC_1_SQRT_2;
        assert!(approx(sv[0b000].re, amp, 1e-10), "|000⟩ amp");
        assert!(approx(sv[0b101].re, amp, 1e-10), "|101⟩ amp");
        for (i, elem) in sv.iter().enumerate().take(8) {
            if i != 0b000 && i != 0b101 {
                assert!(elem.norm_sq() < 1e-18, "basis {i:03b} should be zero");
            }
        }
    }

    #[test]
    fn rejects_zero_bond_dim() {
        let c = Circuit::new("zero-bond", 1);
        assert!(matches!(
            simulate(&c, 0),
            Err(TensorNetworkError::ZeroBondDim)
        ));
    }

    #[test]
    fn rejects_too_many_qubits() {
        let c = Circuit::new("too-wide", MAX_QUBITS_TN + 1);
        assert!(matches!(
            simulate(&c, 4),
            Err(TensorNetworkError::TooManyQubits { .. })
        ));
    }

    #[test]
    fn pulled_batch_has_n_rows() {
        let mps = Mps::zero_state(4, 8);
        let v = to_pulled_batch_form(&mps);
        assert_eq!(v.len(), 4);
        // All rows same width (zero-padded).
        let w = v[0].len();
        for row in &v {
            assert_eq!(row.len(), w);
        }
        // First two entries are χL, χR; for |0…0⟩ both are 1.
        for row in &v {
            assert_eq!(row[0], 1.0);
            assert_eq!(row[1], 1.0);
        }
    }

    #[test]
    fn svd_round_trips_small_complex_matrix() {
        // 3×2 matrix; check A ≈ U Σ Vᴴ.
        let mut a = Array2::<C>::default((3, 2));
        a[[0, 0]] = C::new(1.0, 0.5);
        a[[0, 1]] = C::new(0.0, -1.0);
        a[[1, 0]] = C::new(2.0, 0.0);
        a[[1, 1]] = C::new(1.0, 1.0);
        a[[2, 0]] = C::new(0.5, -0.5);
        a[[2, 1]] = C::new(1.5, 0.0);
        let (u, sigma, vt) = svd_complex(&a);
        // Reconstruct.
        let mut recon = Array2::<C>::default((3, 2));
        for i in 0..3 {
            for j in 0..2 {
                let mut acc = C::zero();
                for k in 0..sigma.len() {
                    let s = C::new(sigma[k], 0.0);
                    acc = acc + u[[i, k]] * s * vt[[k, j]];
                }
                recon[[i, j]] = acc;
            }
        }
        for i in 0..3 {
            for j in 0..2 {
                assert!(
                    (a[[i, j]].re - recon[[i, j]].re).abs() < 1e-8
                        && (a[[i, j]].im - recon[[i, j]].im).abs() < 1e-8,
                    "SVD mismatch at ({i}, {j}): {:?} vs {:?}",
                    a[[i, j]],
                    recon[[i, j]],
                );
            }
        }
    }
}
