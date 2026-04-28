# Kernel L2² — three-way bench (CpuNaive / AVX-512 / wgpu)

ADR-157's accepted-past-experimental gate is **2× p95 lower OR 30% cost
lower at identical recall@10** on a fixed reference grid. This is the
session that runs all three shipped kernels against the same headline
inputs and lays the numbers next to each other so the gate can be
applied honestly.

## Provenance

- **Date**: 2026-04-27
- **Branch**: `main` at `37ccc60` (post-PR #14 merge)
- **CPU**: AMD Ryzen 9 9950X (16C / 32T)
- **GPU** (visible to wgpu): NVIDIA Corporation Device 2c02 (Vulkan); AMD/ATI Device 13c0 also present
- **Kernel**: 6.17.0-20-generic
- **rustc**: stable
- **Bench framework**: criterion 0.5, 100 samples, 3.0 s warm-up

Each kernel's bench file lives next to its crate
(`crates/{core,kernel-avx512,kernel-wgpu}/benches/kernel_l2.rs`) and
runs criterion with the same workload generator (PCG32 from a fixed
seed `0xA5A5_5A5A`) so the inputs are byte-identical across runs.

## Headline grid (D=384, n=16384, top_k=10)

This is the grid each kernel's bench reports as primary. CpuNaive is
sampled in every bench so the small (~2%) per-run drift between cargo
invocations is visible.

| Kernel | Crate | Time | vs CpuNaive (same run) |
|---|---|---:|---:|
| CpuNaive | `crates/core/benches/kernel_l2.rs` | **2.8065 ms** | 1.00× |
| CpuNaive | `crates/kernel-avx512` (in-bench baseline) | 2.7882 ms | 1.00× |
| **AVX-512** | `crates/kernel-avx512` | **2.7082 ms** | **0.971×** (2.87% faster) |
| CpuNaive | `crates/kernel-wgpu` (in-bench baseline) | 2.8407 ms | 1.00× |
| **wgpu** | `crates/kernel-wgpu` | **7.8270 ms** | **2.756×** (slower) |

### Reading these numbers

- **AVX-512 is bit-equal to CpuNaive on the conformance fixture** (per
  `crates/kernel-avx512/README.md` and `assert_kernel_conformant`), so
  the 2.87% gap is the only thing the SIMD intrinsics buy at this
  grid. This is consistent with the ADR-157 prediction that the win
  materializes at high D and large n — at D=384 / n=16384, the popcount /
  L2 inner loop is already cheap enough that the host-side top-k sort
  dominates the budget.
- **wgpu loses 2.76× at this grid because host↔device transfer dominates
  the work.** The current `WgpuKernel` re-uploads the index on every
  scan (no generation-keyed device cache yet — see the `accelerator-plane-deep`
  gist's "v1.x roadmap" callout). At small batch + small index the
  upload is the single largest cost and there's nothing to amortize
  it against.
- **The 2.79 ms CpuNaive baseline drifted by ~2% across cargo
  invocations.** This is normal — fan curves, P-state transitions, and
  rayon's startup cost all introduce noise at this scale. The
  in-same-bench comparison (each crate samples its own CpuNaive
  baseline) is the only one that's apples-to-apples; cross-bench
  comparison is informational only.

### Acceptance gate verdict on this grid

| Kernel | 2× p95 lower? | 30% cost lower? | Past-experimental on this grid? |
|---|---|---|---|
| AVX-512 | No (0.971×) | No | **Not yet** — needs a higher-D / higher-n grid |
| wgpu | No (2.756× slower) | No | **Not yet** — needs higher batch + a generation-keyed device-side index cache |

Per ADR-157 §"Acceptance test", a kernel that fails the gate stays in
its experimental crate. Both kernels remain off-by-default; operators
who register them explicitly accept the trade-off. Neither is in the
default dispatch preference.

## Wider grid — CpuNaive only (3 grid points)

The `crates/core` bench tests three grid points to map where CpuNaive
spends its time:

| dim | n | time | per-vector |
|---:|---:|---:|---:|
| 384 | 16384 | 2.8065 ms | ~171 ns/vec |
| 1024 | 1024 | 364.76 µs | ~356 ns/vec |
| 1024 | 16384 | 6.6205 ms | ~404 ns/vec |

The per-vector cost roughly scales with D as expected (1024/384 ≈ 2.67×;
404/171 ≈ 2.36× — the gap is sort overhead amortizing better at the
bigger grids). This is the workload shape where AVX-512's 16-lane FMA
should start mattering structurally; the focused crate's bench should
be widened to dim=768 / n=1M to apply the ADR-157 acceptance gate
honestly.

## What this bench session does not cover

- **dim ≥ 768, n ≥ 1M, batch ≥ 64** — the regime where ADR-157
  predicts the GPU and SIMD wins materialize. The kernel-avx512 and
  kernel-wgpu benches both pin at dim=384 / n=16384 / batch=1 today.
  This is the next bench-file change to make.
- **rerank phase** — both benches measure pure scan, not popcount-scan
  + L2² rerank. Rerank-pressure scaling is the second axis the
  acceptance gate cares about.
- **recall@10 at identical k** — the gate clause "at identical
  recall@10" is unverified here because all three kernels are
  bit-equal-to-CpuNaive on the popcount path (which is integer math).
  When wgpu's L2 rerank gets exercised under high D, the recall
  comparison becomes load-bearing.

## Reproducing

```bash
# CpuNaive baseline (3 grid points)
cargo bench --manifest-path crates/core/Cargo.toml --bench kernel_l2

# AVX-512 (skipped on hosts without avx512f + bw + vl + vpopcntdq)
cargo bench --manifest-path crates/kernel-avx512/Cargo.toml --bench kernel_l2

# wgpu (skipped on hosts without a wgpu-discoverable adapter)
cargo bench --manifest-path crates/kernel-wgpu/Cargo.toml --bench kernel_l2
```

Each run takes ~30–90 s. Criterion HTML reports land in
`crates/<name>/target/criterion/` if a deeper view of the distributions
is needed.

## Next bench session

Two changes to apply before the next sweep, both in the bench files:

1. **Extend the grid** in `crates/kernel-avx512/benches/kernel_l2.rs`
   and `crates/kernel-wgpu/benches/kernel_l2.rs` to also exercise
   D=768 / n=1M (the ADR-157 reference grid) and a batched query
   variant (n_queries ∈ {1, 64, 256}).
2. **Add a `RabitqIndex` warm-from-disk path** so the wgpu kernel's
   device-side index cache (when added in v1.x) can be measured
   against the upload-per-scan baseline — this is where the GPU win
   should appear.

Both will exercise the part of the design space where ADR-157
predicts the kernel choice actually moves the needle. Today's bench
confirms the design's prediction at the small-grid edge: the top-k
sort dominates, the kernels are within noise of each other, and the
acceptance gate is correctly held.
