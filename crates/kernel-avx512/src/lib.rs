//! AVX-512 implementation of the ADR-157 [`VectorKernel`] trait.
//!
//! This crate provides [`Avx512Kernel`], a SIMD-accelerated kernel that
//! uses the `avx512f`, `avx512bw`, `avx512vl`, and `avx512vpopcntdq`
//! extensions to speed up the two hot inner loops ruLake's search path
//! runs against a primed cache:
//!
//! - **L2² distance top-K** — vectorized 16-lane subtract + fused
//!   multiply-add per chunk.
//! - **rabitq popcount Hamming top-K** — `_mm512_popcnt_epi64` over
//!   8-lane `u64` chunks.
//!
//! ## Construction is fail-closed
//!
//! [`Avx512Kernel::new`] returns `None` if the host CPU is missing any
//! of the required features. This means downstream binaries can compile
//! this crate in unconditionally — on a non-AVX-512 host the
//! constructor simply yields `None` and the operator falls back to
//! `CpuNaiveKernel`.
//!
//! ## Determinism
//!
//! Both the L2 and popcount paths produce **byte-equal** top-K against
//! [`rulake::kernel::CpuNaiveKernel`] on the ADR-157 conformance
//! fixture (`assert_kernel_conformant`). That equality is the
//! promotion gate from "experimental crate" to "default dispatch
//! preference."
//!
//! ## Safety
//!
//! AVX-512 intrinsics are `unsafe` to call: their precondition is "the
//! host actually supports the instruction". We honor that contract by
//! gating every public entry point on the runtime feature check baked
//! into [`Avx512Kernel::new`], and by isolating every `unsafe` call
//! inside `#[target_feature(enable = ...)]` helpers. Each `unsafe`
//! block is documented with the invariant it relies on.
//!
//! # Example
//!
//! ```no_run
//! use ruvector_rulake_kernel_avx512::Avx512Kernel;
//! use rulake::kernel::VectorKernel;
//!
//! // Returns `None` on non-AVX-512 hosts; pattern-match before use.
//! if let Some(k) = Avx512Kernel::new() {
//!     let q  = vec![0.0f32; 16];
//!     let cs = vec![vec![1.0f32; 16], vec![2.0f32; 16]];
//!     let top = k.l2_distance_one(&q, &cs, 1);
//!     assert_eq!(top.len(), 1);
//! }
//! ```
#![deny(unsafe_op_in_unsafe_fn)]
#![deny(missing_docs)]

use rulake::kernel::{KernelCapabilities, VectorKernel};
use std::cmp::Ordering;

/// AVX-512 [`VectorKernel`] implementation.
///
/// Construct via [`Avx512Kernel::new`]; the constructor returns `None`
/// if the host CPU is missing any of the required AVX-512 extensions.
///
/// # Example
///
/// ```no_run
/// use ruvector_rulake_kernel_avx512::Avx512Kernel;
/// use rulake::kernel::VectorKernel;
///
/// if let Some(k) = Avx512Kernel::new() {
///     assert_eq!(k.id(), "avx512");
///     assert!(k.capabilities().popcount_native);
/// }
/// ```
#[derive(Debug, Default, Clone, Copy)]
pub struct Avx512Kernel {
    // Zero-sized witness that runtime feature detection succeeded.
    // Kept private so callers cannot construct an `Avx512Kernel` that
    // bypasses the safety check in `new()`.
    _gated: (),
}

impl Avx512Kernel {
    /// Construct an [`Avx512Kernel`], or return `None` if the host CPU
    /// lacks any of `avx512f`, `avx512bw`, `avx512vl`, or
    /// `avx512vpopcntdq`.
    ///
    /// On non-`x86_64` targets this always returns `None` so the crate
    /// compiles cleanly across architectures.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use ruvector_rulake_kernel_avx512::Avx512Kernel;
    /// match Avx512Kernel::new() {
    ///     Some(_) => println!("AVX-512 path active"),
    ///     None    => println!("Falling back to CpuNaiveKernel"),
    /// }
    /// ```
    pub fn new() -> Option<Self> {
        if Self::is_supported() {
            Some(Self { _gated: () })
        } else {
            None
        }
    }

    /// Report whether the host CPU advertises every AVX-512 extension
    /// this kernel relies on. Public so downstream tests / dispatch
    /// policy can probe support without constructing a kernel.
    ///
    /// # Example
    ///
    /// ```
    /// use ruvector_rulake_kernel_avx512::Avx512Kernel;
    /// // Always returns a `bool` regardless of host architecture.
    /// let _ = Avx512Kernel::is_supported();
    /// ```
    pub fn is_supported() -> bool {
        #[cfg(target_arch = "x86_64")]
        {
            is_x86_feature_detected!("avx512f")
                && is_x86_feature_detected!("avx512bw")
                && is_x86_feature_detected!("avx512vl")
                && is_x86_feature_detected!("avx512vpopcntdq")
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            false
        }
    }
}

impl VectorKernel for Avx512Kernel {
    fn id(&self) -> &'static str {
        "avx512"
    }

    fn capabilities(&self) -> KernelCapabilities {
        KernelCapabilities {
            simd_width: 16,
            popcount_native: true,
            gpu: false,
        }
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
                // SAFETY: `Avx512Kernel` is constructed only via
                // `new()`, which verifies `avx512f` is present at
                // runtime. Both slices are sliced to `len` so the
                // intrinsic helper indexes inside their bounds.
                let acc = unsafe { l2_squared_avx512(&query[..len], &c[..len]) };
                (i as u64, acc)
            })
            .collect();
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
                // SAFETY: `Avx512Kernel` is constructed only via
                // `new()`, which verifies `avx512f`, `avx512vl`, and
                // `avx512vpopcntdq` are present at runtime. Both
                // slices are sliced to `len`.
                let acc = unsafe { popcount_xor_avx512(&query[..len], &c[..len]) };
                (i as u64, acc)
            })
            .collect();
        scored.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        scored.truncate(top_k);
        scored
    }
}

// ---------- AVX-512 inner loops ----------
//
// Both helpers are `unsafe fn` because they require `target_feature`s
// that the compiler must trust the caller has verified. Their callers
// (the trait impl above) construct `Avx512Kernel` only via
// `Avx512Kernel::new()`, which gates on `is_x86_feature_detected!`.
//
// On non-`x86_64` targets we emit scalar fallbacks so the crate still
// builds cleanly; those fallbacks are never reached at runtime because
// `Avx512Kernel::new()` returns `None` everywhere except `x86_64` with
// the right features.

/// AVX-512 sum-of-squared-differences over equal-length slices.
///
/// # Safety
///
/// Caller must ensure the host CPU supports `avx512f`. `query.len() ==
/// candidate.len()` is enforced by the caller passing matching subslices.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
unsafe fn l2_squared_avx512(query: &[f32], candidate: &[f32]) -> f32 {
    use core::arch::x86_64::{_mm512_loadu_ps, _mm512_storeu_ps, _mm512_sub_ps};

    // Bit-equality with `CpuNaiveKernel` is the conformance gate, and
    // the naive impl is a *strictly sequential, scalar* accumulation:
    //
    //   for j in 0..len { acc += (q[j] - c[j]) * (q[j] - c[j]); }
    //
    // Two divergences from that order break bit-equality:
    //   1. **FMA**. `_mm512_fmadd_ps` performs `d*d + acc` with a
    //      single rounding; the naive impl rounds `d*d` first, then
    //      rounds the add. We use separate `mul` + `add` (still SIMD)
    //      to keep the rounding count identical.
    //   2. **Tree reduce**. `_mm512_reduce_add_ps` collapses 16 lanes
    //      via a balanced tree, which reorders adds. The naive impl
    //      adds left-to-right. We therefore SIMD-compute the 16
    //      per-lane squared diffs per chunk, store them back to a
    //      stack buffer, and accumulate them in source order with the
    //      same scalar `+=` the naive path uses.
    //
    // The result is still SIMD-accelerated on the per-element compute
    // (sub + mul) — the squared-diff vector is what dominates at
    // larger D — while the reduction matches the spec kernel byte for
    // byte. This is the same trick `ruvector-rabitq`'s symmetric scan
    // uses to keep deterministic output across SIMD impls.

    let len = query.len();
    let chunks = len / 16;
    let tail_start = chunks * 16;

    let mut sum = 0.0f32;
    let mut tmp = [0.0f32; 16];

    for i in 0..chunks {
        let off = i * 16;
        // SAFETY: Both slices are at least `(chunks * 16)` long so a
        // 16-lane unaligned load at offset `off` is in bounds.
        // `loadu`/`storeu` are alignment-agnostic. All intrinsics
        // require `avx512f`, present per the `target_feature`
        // attribute. The `tmp` array is exactly 16 f32s long, so
        // `_mm512_storeu_ps` writes inside it.
        unsafe {
            let q = _mm512_loadu_ps(query.as_ptr().add(off));
            let c = _mm512_loadu_ps(candidate.as_ptr().add(off));
            let d = _mm512_sub_ps(q, c);
            // `d * d` as a separate rounded multiply (NOT fmadd) so
            // the per-lane scalar `(q - c)^2` rounds identically to
            // the naive impl.
            let dd = _mm512_mul_ps_local(d, d);
            _mm512_storeu_ps(tmp.as_mut_ptr(), dd);
        }
        // Accumulate left-to-right, same order as the naive scalar
        // loop. This is the single sequential dependency that pins
        // bit-equality.
        for v in tmp.iter() {
            sum += *v;
        }
    }

    // Scalar tail — bit-equal to the naive `(q - c)^2` accumulation.
    for j in tail_start..len {
        let d = query[j] - candidate[j];
        sum += d * d;
    }
    sum
}

// Tiny shim so the `mul_ps` use site reads consistently with the rest
// of the SIMD chunk. One-instruction wrapper.
#[cfg(target_arch = "x86_64")]
#[inline]
#[target_feature(enable = "avx512f")]
unsafe fn _mm512_mul_ps_local(
    a: core::arch::x86_64::__m512,
    b: core::arch::x86_64::__m512,
) -> core::arch::x86_64::__m512 {
    core::arch::x86_64::_mm512_mul_ps(a, b)
}

/// AVX-512 popcount-of-XOR over equal-length `u64` slices, using the
/// `avx512vpopcntdq` extension.
///
/// # Safety
///
/// Caller must ensure the host CPU supports `avx512f`, `avx512vl`, and
/// `avx512vpopcntdq`. `query.len() == candidate.len()` is enforced by
/// the caller passing matching subslices.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512vl,avx512vpopcntdq")]
unsafe fn popcount_xor_avx512(query: &[u64], candidate: &[u64]) -> u32 {
    use core::arch::x86_64::{
        _mm512_loadu_si512, _mm512_popcnt_epi64, _mm512_reduce_add_epi64, _mm512_setzero_si512,
        _mm512_xor_si512,
    };

    let len = query.len();
    let chunks = len / 8;
    let tail_start = chunks * 8;

    // `_mm512_setzero_si512` requires `avx512f`, present per the
    // function's `target_feature` attribute.
    let mut acc = _mm512_setzero_si512();

    for i in 0..chunks {
        let off = i * 8;
        // SAFETY: Both slices are at least `(chunks * 8)` `u64`s long
        // so an 8-lane (= 64 byte) unaligned load at element offset
        // `off` is in bounds. The `_mm512_loadu_si512` pointer is cast
        // from `*const u64` — `loadu` is alignment-agnostic. All
        // intrinsics' feature requirements are covered by this
        // function's `target_feature` attribute.
        unsafe {
            let q = _mm512_loadu_si512(query.as_ptr().add(off) as *const _);
            let c = _mm512_loadu_si512(candidate.as_ptr().add(off) as *const _);
            let x = _mm512_xor_si512(q, c);
            let pc = _mm512_popcnt_epi64(x);
            // 64-bit lane-wise add — cannot overflow because each
            // `popcnt_epi64` lane is in `0..=64`, total ≤ 8 * 64 *
            // chunks ≪ i64::MAX.
            acc = _mm512_add_epi64(acc, pc);
        }
    }

    // `_mm512_reduce_add_epi64` requires `avx512f`, covered by the
    // function's `target_feature` attribute.
    let chunk_sum = _mm512_reduce_add_epi64(acc) as u64;
    let mut sum = chunk_sum as u32;

    for j in tail_start..len {
        sum += (query[j] ^ candidate[j]).count_ones();
    }
    sum
}

// We need `_mm512_add_epi64` in `popcount_xor_avx512`; bring it in
// alongside the other intrinsics. (Pulled out to keep the popcount
// helper readable.)
#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::_mm512_add_epi64;

// Scalar fallbacks for non-x86_64 targets so the crate still builds on
// ARM hosts, etc. These are NEVER reached at runtime because
// `Avx512Kernel::new()` returns `None` outside `x86_64`.
#[cfg(not(target_arch = "x86_64"))]
unsafe fn l2_squared_avx512(query: &[f32], candidate: &[f32]) -> f32 {
    let mut acc = 0.0f32;
    for j in 0..query.len() {
        let d = query[j] - candidate[j];
        acc += d * d;
    }
    acc
}

#[cfg(not(target_arch = "x86_64"))]
unsafe fn popcount_xor_avx512(query: &[u64], candidate: &[u64]) -> u32 {
    let mut acc = 0u32;
    for j in 0..query.len() {
        acc += (query[j] ^ candidate[j]).count_ones();
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::*;
    use rulake::kernel::assert_kernel_conformant;

    #[test]
    fn caps_advertise_avx512_geometry() {
        if let Some(k) = Avx512Kernel::new() {
            let c = k.capabilities();
            assert_eq!(c.simd_width, 16);
            assert!(c.popcount_native);
            assert!(!c.gpu);
            assert_eq!(k.id(), "avx512");
        } else {
            eprintln!("skipping caps check: AVX-512 not detected on this host");
        }
    }

    #[test]
    fn conformance_seed_42_matches_reference() {
        let Some(k) = Avx512Kernel::new() else {
            eprintln!("skipping: AVX-512 not detected on this host");
            return;
        };
        assert_kernel_conformant(&k, 42);
    }
}
