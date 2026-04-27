# ruvector-rulake-kernel-wgpu

Portable GPU implementation of the ADR-157 `VectorKernel` trait for
ruLake, via [wgpu](https://wgpu.rs).

This crate is **standalone** (no workspace) per ADR-001. It depends on
the root `rulake` crate by relative path (`path = ".."`).

## What it does

Provides a `WgpuKernel` that runs the two hot inner loops on whatever
compute-capable backend wgpu finds (Vulkan / Metal / DX12 / GL / WebGPU):

- **L2² distance top-K** — one workgroup over the candidate batch,
  one thread per (query, candidate) distance, results scored on host.
- **rabitq popcount Hamming top-K** — packed 1-bit codes XORed and
  popcounted on-GPU per `u32` lane (WGSL `countOneBits`).

Both shaders compute **only the per-candidate distance** on GPU; the
top-K sort runs on the host with the same byte-equal tie-break the
naive kernel uses. This is the key trick that lets the GPU path stay
deterministic on the conformance fixture (see ADR-157 §"Determinism as
a hard gate", clause 1: scan must be byte-equal; tie-broken by lower
index). The L2 path is *not* bit-equal to `CpuNaiveKernel` because the
WGSL `f32` operators are IEEE-754 but the WGPU spec doesn't pin
sub-ULP behaviour across drivers — top-K **set** matches, distance
ordering matches, but raw distances may differ in the last ULP.
Conformance tests therefore exercise the popcount path (which is
exact integer math) and a coarse L2 path (top-K set, not raw
distances).

## Construction is fail-closed

`WgpuKernel::new_async()` / `WgpuKernel::new_blocking()` return
`Result<Self>`:

- `Ok` iff wgpu can request an adapter + device.
- `Err` (`WgpuKernelError::NoAdapter` / `RequestDevice`) on headless
  CI hosts. Callers should fall back to `CpuNaiveKernel`.

This means a single binary built with this crate enabled still runs on
GPU-less hosts; it just doesn't get the speedup.

## Build

```bash
cargo build --release --manifest-path kernel-wgpu/Cargo.toml
```

## Test

```bash
cargo test --release --manifest-path kernel-wgpu/Cargo.toml
```

Conformance tests skip themselves with an `eprintln!` and exit success
when no adapter is available.

## Bench

```bash
cargo bench --manifest-path kernel-wgpu/Cargo.toml
```

GPU benches include host↔device transfer overhead. At dim=384,
n=16384 the SSE/AVX-512 host kernels often beat wgpu on the byte
level — the GPU win materializes at higher D and / or larger batch.
