//! ADR-157 acceptance tests for [`Avx512Kernel`].
//!
//! These tests are the promotion gate from "experimental crate" to
//! "default dispatch preference" — they assert byte-equal output
//! against `CpuNaiveKernel` on the deterministic conformance fixture
//! at four seeds (1, 42, 100, 999).
//!
//! Behaviour on hosts without the required AVX-512 features:
//! `Avx512Kernel::new()` returns `None`, the test logs a skip notice
//! and exits successfully. CI on AVX-512-equipped hosts gets full
//! coverage; CI on non-AVX-512 hosts simply doesn't exercise the SIMD
//! path. (For strict CI gates, mark these `#[ignore]` and run them
//! via `cargo test -- --ignored` only on AVX-512 hardware.)

#![cfg(target_arch = "x86_64")]

use rulake::kernel::assert_kernel_conformant;
use ruvector_rulake_kernel_avx512::Avx512Kernel;

fn run_seed(seed: u64) {
    let Some(k) = Avx512Kernel::new() else {
        eprintln!(
            "skipping seed {seed}: AVX-512 (avx512f+bw+vl+vpopcntdq) not detected on this host"
        );
        return;
    };
    assert_kernel_conformant(&k, seed);
}

#[test]
fn conformance_seed_1() {
    run_seed(1);
}

#[test]
fn conformance_seed_42() {
    run_seed(42);
}

#[test]
fn conformance_seed_100() {
    run_seed(100);
}

#[test]
fn conformance_seed_999() {
    run_seed(999);
}
