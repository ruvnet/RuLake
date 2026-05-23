//! **Experimental** — this module is stabilizing for v3.0; the API may change before that.
//!
//! Optional accelerator plane scaffolding per **ADR-157**.
//!
//! This module defines the `VectorKernel` trait that future SIMD, GPU,
//! and WASM kernels will implement. Today only the [`CpuNaiveKernel`]
//! reference impl ships, and **no current ruLake code path dispatches
//! through this trait** — it exists purely as the v3.0 contract preview
//! so that downstream kernel crates (`ruvector-rabitq-cuda`, etc.) can
//! start building against a frozen surface.
//!
//! Every kernel must produce **byte-equal top-K** against
//! [`CpuNaiveKernel`] on the conformance fixture (see
//! [`assert_kernel_conformant`]). That equality is ADR-157's promotion
//! gate: a non-naive kernel stays in its experimental crate until it
//! passes.
//!
//! ## Status
//!
//! Trait + types are `#[doc(hidden)]` on crates.io to keep the v2.x
//! public surface unchanged while the design ages. The names are stable
//! enough for kernel authors to consume by full path
//! (`rulake::kernel::VectorKernel`); they are not yet stable enough to
//! commit to a SemVer-protected re-export.

use std::cmp::Ordering;

/// Static description of what a [`VectorKernel`] implementation can do.
///
/// Used by the (future) ruLake dispatch policy to decide whether a
/// given query should hit this kernel or fall back to the CPU naive
/// reference. The struct is intentionally minimal in v2.3-alpha — more
/// fields will accrete as real SIMD / GPU kernels appear and reveal
/// what dispatch actually needs to know.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KernelCapabilities {
    /// Width of the SIMD lane the kernel uses, in 32-bit floats.
    /// `1` means scalar (no SIMD). `8` means AVX2 / NEON-style 256-bit.
    /// `16` means AVX-512.
    pub simd_width: u8,
    /// Whether the kernel uses a hardware popcount instruction
    /// (`POPCNT` on x86, `CNT` on ARMv8) for the rabitq Hamming path.
    pub popcount_native: bool,
    /// Whether the kernel runs on a GPU. CPU kernels — scalar or SIMD
    /// — set this to `false`.
    pub gpu: bool,
}

impl Default for KernelCapabilities {
    /// Defaults match the [`CpuNaiveKernel`] specification kernel:
    /// scalar lanes, software popcount, no GPU.
    fn default() -> Self {
        Self {
            simd_width: 1,
            popcount_native: false,
            gpu: false,
        }
    }
}

/// A vector kernel executes the two hot inner loops that ruLake's
/// search path runs against a primed cache: exact L2² top-K and
/// 1-bit rabitq Hamming top-K.
///
/// Kernels are stateless w.r.t. the index — the index lives in the
/// cache and is passed in by reference on every call. This keeps GPU
/// kernels from needing to own index lifetimes.
///
/// **Conformance**: every implementation MUST produce byte-equal
/// output against [`CpuNaiveKernel`] on the deterministic fixture
/// driven by [`assert_kernel_conformant`]. That equality is ADR-157's
/// promotion gate.
#[doc(hidden)]
pub trait VectorKernel: Send + Sync {
    /// Stable identifier surfaced in stats and logs. Examples:
    /// `"cpu-naive"`, `"avx512"`, `"neon"`, `"cuda"`, `"metal"`,
    /// `"wasm-simd"`.
    fn id(&self) -> &'static str;

    /// Advertise what this kernel can do. See [`KernelCapabilities`].
    fn capabilities(&self) -> KernelCapabilities;

    /// Top-K nearest by exact L2² distance. Returns
    /// `(candidate_index, distance_squared)` pairs sorted by
    /// ascending distance, length `min(top_k, candidates.len())`.
    ///
    /// `query.len()` must equal every `candidates[i].len()`. Behaviour
    /// on mismatch is implementation-defined (the naive impl
    /// truncates to the shorter slice).
    fn l2_distance_one(
        &self,
        query: &[f32],
        candidates: &[Vec<f32>],
        top_k: usize,
    ) -> Vec<(u64, f32)>;

    /// Top-K nearest by Hamming distance over packed 1-bit codes.
    /// Returns `(candidate_index, hamming_distance)` pairs sorted by
    /// ascending distance, length `min(top_k, candidates.len())`.
    ///
    /// `query.len()` must equal every `candidates[i].len()`; both are
    /// arrays of `u64` words holding packed 1-bit rabitq codes.
    fn rabitq_popcount(
        &self,
        query: &[u64],
        candidates: &[Vec<u64>],
        top_k: usize,
    ) -> Vec<(u64, u32)>;
}

/// CPU reference implementation of [`VectorKernel`] — the **specification
/// kernel** under ADR-157.
///
/// Every other kernel impl (SIMD, GPU, WASM) must produce byte-equal
/// top-K against this struct's output to be considered conformant. The
/// L2 path is straightforward sum-of-squared-diffs; the popcount path
/// is `u64::count_ones` over each chunk. Both are deliberately the
/// most boring implementations possible so that ambiguity can never
/// be the source of a conformance failure.
///
/// Tie-breaking: when two candidates have identical distance, the one
/// with the **lower index** wins. This is the contract every other
/// kernel must honor.
#[doc(hidden)]
#[derive(Debug, Default, Clone, Copy)]
pub struct CpuNaiveKernel;

impl VectorKernel for CpuNaiveKernel {
    fn id(&self) -> &'static str {
        "cpu-naive"
    }

    fn capabilities(&self) -> KernelCapabilities {
        KernelCapabilities::default()
    }

    fn l2_distance_one(
        &self,
        query: &[f32],
        candidates: &[Vec<f32>],
        top_k: usize,
    ) -> Vec<(u64, f32)> {
        let mut scored: Vec<(u64, f32)> = candidates
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let len = query.len().min(c.len());
                let mut acc = 0.0f32;
                for j in 0..len {
                    let d = query[j] - c[j];
                    acc += d * d;
                }
                (i as u64, acc)
            })
            .collect();
        // (distance asc, index asc) — ties broken by lower index.
        scored.sort_by(|a, b| match a.1.partial_cmp(&b.1) {
            Some(Ordering::Equal) | None => a.0.cmp(&b.0),
            Some(o) => o,
        });
        scored.truncate(top_k);
        scored
    }

    fn rabitq_popcount(
        &self,
        query: &[u64],
        candidates: &[Vec<u64>],
        top_k: usize,
    ) -> Vec<(u64, u32)> {
        let mut scored: Vec<(u64, u32)> = candidates
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let len = query.len().min(c.len());
                let mut acc: u32 = 0;
                for j in 0..len {
                    acc += (query[j] ^ c[j]).count_ones();
                }
                (i as u64, acc)
            })
            .collect();
        scored.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        scored.truncate(top_k);
        scored
    }
}

/// ADR-157 promotion gate: assert that `k` produces byte-equal output
/// against [`CpuNaiveKernel`] on a deterministic fixture.
///
/// The fixture is 10 candidates of dimension 16, generated from a
/// linear-congruential PRNG seeded by `fixture_seed`. Both the L2 and
/// popcount paths are exercised; both must match the reference exactly
/// (including index ordering on ties).
///
/// Panics with a descriptive message on any mismatch. Intended to be
/// called from the kernel author's own test suite.
#[doc(hidden)]
pub fn assert_kernel_conformant(k: &dyn VectorKernel, fixture_seed: u64) {
    let dim = 16usize;
    let n = 10usize;
    let top_k = 5usize;

    // Tiny LCG so the fixture is reproducible without pulling rand.
    let mut state = fixture_seed
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(1);
    let mut next_u64 = || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        state
    };
    let mut next_f32 = || {
        // Map upper 24 bits to [-1, 1).
        let bits = (next_u64() >> 40) as u32; // 24 bits
        let x = (bits as f32) / (1u32 << 24) as f32; // [0, 1)
        x * 2.0 - 1.0
    };

    let query_f: Vec<f32> = (0..dim).map(|_| next_f32()).collect();
    let candidates_f: Vec<Vec<f32>> = (0..n)
        .map(|_| (0..dim).map(|_| next_f32()).collect())
        .collect();

    let query_u: Vec<u64> = (0..dim).map(|_| next_u64()).collect();
    let candidates_u: Vec<Vec<u64>> = (0..n)
        .map(|_| (0..dim).map(|_| next_u64()).collect())
        .collect();

    let reference = CpuNaiveKernel;

    let got_l2 = k.l2_distance_one(&query_f, &candidates_f, top_k);
    let want_l2 = reference.l2_distance_one(&query_f, &candidates_f, top_k);
    assert_eq!(
        got_l2,
        want_l2,
        "kernel {:?} l2_distance_one diverges from CpuNaiveKernel reference",
        k.id()
    );

    let got_pc = k.rabitq_popcount(&query_u, &candidates_u, top_k);
    let want_pc = reference.rabitq_popcount(&query_u, &candidates_u, top_k);
    assert_eq!(
        got_pc,
        want_pc,
        "kernel {:?} rabitq_popcount diverges from CpuNaiveKernel reference",
        k.id()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn naive_id_and_caps_are_stable() {
        let k = CpuNaiveKernel;
        assert_eq!(k.id(), "cpu-naive");
        assert_eq!(k.capabilities(), KernelCapabilities::default());
    }

    #[test]
    fn naive_self_conformance_across_seeds() {
        let k = CpuNaiveKernel;
        for seed in [1u64, 42, 1337, 0xDEAD_BEEF] {
            assert_kernel_conformant(&k, seed);
        }
    }
}
