//! Tiny exact StateVector simulator. v0.0 keeps the kernel under 200
//! lines so the witness path is easy to audit; v0.1 swaps in a
//! `simba`-vectorised inner loop and rayon-parallel chunking.
//!
//! The simulator is deterministic given a seed and circuit — that's
//! the load-bearing property the witness chain relies on.

use crate::circuit::{Circuit, Gate};

/// Complex number — two `f64`s, matches the layout used downstream
/// when we serialise into `PulledBatch.vectors` (real, imag, real, …).
#[derive(Debug, Clone, Copy, Default)]
pub struct C {
    /// Real component.
    pub re: f64,
    /// Imaginary component.
    pub im: f64,
}

impl C {
    /// Build from a `(real, imag)` pair.
    pub const fn new(re: f64, im: f64) -> Self {
        Self { re, im }
    }
    /// `0 + 0i`.
    pub const fn zero() -> Self {
        Self { re: 0.0, im: 0.0 }
    }
    /// `1 + 0i`.
    pub const fn one() -> Self {
        Self { re: 1.0, im: 0.0 }
    }
    /// `|z|² = re² + im²`.
    pub fn norm_sq(self) -> f64 {
        self.re * self.re + self.im * self.im
    }
}

impl std::ops::Add for C {
    type Output = Self;
    fn add(self, o: Self) -> Self {
        Self {
            re: self.re + o.re,
            im: self.im + o.im,
        }
    }
}

impl std::ops::Sub for C {
    type Output = Self;
    fn sub(self, o: Self) -> Self {
        Self {
            re: self.re - o.re,
            im: self.im - o.im,
        }
    }
}

impl std::ops::Mul for C {
    type Output = Self;
    /// Complex multiply: `(a+bi)(c+di) = (ac-bd) + (ad+bc)i`.
    fn mul(self, o: Self) -> Self {
        Self {
            re: self.re * o.re - self.im * o.im,
            im: self.re * o.im + self.im * o.re,
        }
    }
}

use std::f64::consts::FRAC_1_SQRT_2;

/// Run a circuit from |0…0⟩. Returns the full state vector of length
/// `1 << n_qubits`.
pub fn simulate(c: &Circuit) -> Vec<C> {
    let dim = c.dim();
    let mut sv = vec![C::zero(); dim];
    sv[0] = C::one();

    for g in &c.gates {
        match *g {
            Gate::H { q } => apply_1q(
                &mut sv,
                q,
                [
                    [C::new(FRAC_1_SQRT_2, 0.0), C::new(FRAC_1_SQRT_2, 0.0)],
                    [C::new(FRAC_1_SQRT_2, 0.0), C::new(-FRAC_1_SQRT_2, 0.0)],
                ],
            ),
            Gate::X { q } => apply_1q(&mut sv, q, [[C::zero(), C::one()], [C::one(), C::zero()]]),
            Gate::Y { q } => apply_1q(
                &mut sv,
                q,
                [
                    [C::zero(), C::new(0.0, -1.0)],
                    [C::new(0.0, 1.0), C::zero()],
                ],
            ),
            Gate::Z { q } => apply_1q(
                &mut sv,
                q,
                [[C::one(), C::zero()], [C::zero(), C::new(-1.0, 0.0)]],
            ),
            Gate::S { q } => apply_1q(
                &mut sv,
                q,
                [[C::one(), C::zero()], [C::zero(), C::new(0.0, 1.0)]],
            ),
            Gate::T { q } => {
                let phase = C::new(FRAC_1_SQRT_2, FRAC_1_SQRT_2); // e^{iπ/4}
                apply_1q(&mut sv, q, [[C::one(), C::zero()], [C::zero(), phase]]);
            }
            Gate::Rz { q, theta } => {
                let half = theta / 2.0;
                let neg = C::new(half.cos(), -half.sin());
                let pos = C::new(half.cos(), half.sin());
                apply_1q(&mut sv, q, [[neg, C::zero()], [C::zero(), pos]]);
            }
            Gate::Cx { control, target } => apply_cx(&mut sv, control, target),
        }
    }
    sv
}

fn apply_1q(sv: &mut [C], q: u8, m: [[C; 2]; 2]) {
    let bit = 1usize << (q as usize);
    let mut i = 0;
    while i < sv.len() {
        if i & bit == 0 {
            let j = i | bit;
            let a = sv[i];
            let b = sv[j];
            sv[i] = m[0][0] * a + m[0][1] * b;
            sv[j] = m[1][0] * a + m[1][1] * b;
        }
        i += 1;
    }
}

fn apply_cx(sv: &mut [C], control: u8, target: u8) {
    let cb = 1usize << (control as usize);
    let tb = 1usize << (target as usize);
    let mut i = 0;
    while i < sv.len() {
        if (i & cb) != 0 && (i & tb) == 0 {
            let j = i | tb;
            sv.swap(i, j);
        }
        i += 1;
    }
}

/// Total |ψ|² — should stay at 1.0 within rounding for unitary
/// circuits. Used by tests as a sanity probe; the witness check is
/// the real verifier.
pub fn norm_sq_total(sv: &[C]) -> f64 {
    sv.iter().map(|c| c.norm_sq()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::circuit::{Circuit, Gate};

    #[test]
    fn single_h_creates_uniform_superposition() {
        let mut c = Circuit::new("h-1q", 1);
        c.push(Gate::H { q: 0 });
        let sv = simulate(&c);
        let amp = FRAC_1_SQRT_2;
        assert!((sv[0].re - amp).abs() < 1e-12);
        assert!((sv[1].re - amp).abs() < 1e-12);
        assert!((norm_sq_total(&sv) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn bell_pair_h_then_cx() {
        let mut c = Circuit::new("bell", 2);
        c.push(Gate::H { q: 0 });
        c.push(Gate::Cx {
            control: 0,
            target: 1,
        });
        let sv = simulate(&c);
        let amp = FRAC_1_SQRT_2;
        assert!((sv[0b00].re - amp).abs() < 1e-12, "|00⟩ amplitude");
        assert!(sv[0b01].re.abs() < 1e-12, "|01⟩ should be zero");
        assert!(sv[0b10].re.abs() < 1e-12, "|10⟩ should be zero");
        assert!((sv[0b11].re - amp).abs() < 1e-12, "|11⟩ amplitude");
        assert!((norm_sq_total(&sv) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn x_flips_zero_to_one() {
        let mut c = Circuit::new("x-1q", 1);
        c.push(Gate::X { q: 0 });
        let sv = simulate(&c);
        assert!((sv[0].re).abs() < 1e-12);
        assert!((sv[1].re - 1.0).abs() < 1e-12);
    }
}
