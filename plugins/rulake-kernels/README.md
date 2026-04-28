# rulake-kernels

The ADR-157 accelerator plane. **Off by default** — operators opt in explicitly because the kernels need the host requirements (AVX-512 features for the SIMD path, a wgpu-discoverable adapter for the GPU path).

## What ships

| Kernel | Crate | Determinism | Acceptance gate (this grid) |
|---|---|---|---|
| `Avx512Kernel` | `crates/kernel-avx512/` | bit-equal to CpuNaive | not yet — needs higher D / n |
| `WgpuKernel` | `crates/kernel-wgpu/` | popcount bit-equal; L2 coarse | not yet — host↔device transfer dominates |

Latest 3-way bench: see [`docs/research/benchmarks/kernel-l2-three-way.md`](https://github.com/ruvnet/RuLake/blob/main/docs/research/benchmarks/kernel-l2-three-way.md).

## Security

R-WGPU-1 cap (committed `21a5610`): `WgpuKernel::l2_distance_one` and `rabitq_popcount` refuse any request whose host-side flat buffer would exceed `MAX_DISPATCH_BYTES = 1 GiB`. Returns empty top-K + `eprintln!` warning instead of OOM-ing the worker thread.

Full security review: [`docs/research/security/kernels-v2.md`](https://github.com/ruvnet/RuLake/blob/main/docs/research/security/kernels-v2.md).

## Commands

- `/rulake-kernel-status` — show which kernels are registered + their `KernelCapabilities`
- `/rulake-kernel-bench` — run the 3-way bench (CpuNaive / AVX-512 / wgpu) on the headline grid

## Construction is fail-closed

Both kernels return `Option<Self>` / `Result<Self>` from their constructors. A binary linked against this plugin still runs on hosts that lack the accelerator — it just doesn't get the speedup.

## See also

- [ADR-157](https://github.com/ruvnet/RuLake/blob/main/docs/adrs/ADR-157-optional-accelerator-plane.md) — the trait + dispatch contract
- [`accelerator-plane-deep.md`](https://github.com/ruvnet/RuLake/blob/main/docs/gists/accelerator-plane-deep.md) — the deep gist
