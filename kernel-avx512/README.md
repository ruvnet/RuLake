# ruvector-rulake-kernel-avx512

AVX-512 implementation of the ADR-157 `VectorKernel` trait for ruLake.

This crate is **standalone** (no workspace) per ADR-001. It depends on
the root `rulake` crate by relative path (`path = ".."`).

## What it does

Provides an `Avx512Kernel` that accelerates:

- **L2² distance top-K** — `_mm512_sub_ps` + `_mm512_fmadd_ps` per
  16-lane chunk, horizontal reduce on the tail.
- **rabitq popcount Hamming top-K** — `_mm512_popcnt_epi64` (the
  `avx512vpopcntdq` extension) over 8-lane `u64` chunks, scalar tail.

Both paths are bit-equal to `rulake::kernel::CpuNaiveKernel` on the
ADR-157 conformance fixture (`assert_kernel_conformant`); promotion past
experimental is gated on that.

## Runtime feature detection

`Avx512Kernel::new()` returns `Option<Self>`:

- `Some` iff the host CPU advertises **all** of `avx512f`, `avx512bw`,
  `avx512vl`, and `avx512vpopcntdq`.
- `None` otherwise — callers should fall back to `CpuNaiveKernel` (or
  whichever non-AVX-512 kernel they have registered).

This means a single binary built with this crate enabled still runs on
non-AVX-512 hosts; it just doesn't get the speedup.

## Safety

The kernel uses `core::arch::x86_64` intrinsics, which are `unsafe` to
call. We localize the `unsafe` blocks to the inner SIMD loops, hide them
behind `#[target_feature(enable = ...)]` functions, and gate every entry
into those functions on the runtime feature check above. Per-function
documentation explains the invariant for each `unsafe` block.

## Build

```bash
cargo build --release --manifest-path kernel-avx512/Cargo.toml
```

## Test

```bash
cargo test --release --manifest-path kernel-avx512/Cargo.toml
```

Conformance tests skip themselves (`Skipping: AVX-512 not detected`) on
hosts without the required features.

## Bench

```bash
cargo bench --manifest-path kernel-avx512/Cargo.toml
```

Mirrors the root `benches/kernel_l2.rs` for direct comparison against
the `CpuNaiveKernel` baseline.
