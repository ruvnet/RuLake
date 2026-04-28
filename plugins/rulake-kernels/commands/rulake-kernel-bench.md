---
description: Run the ADR-157 three-way kernel L2 bench (CpuNaive / AVX-512 / wgpu) and print the headline-grid results. Mirrors `cargo bench` against the same workload generator (PCG32 seed 0xA5A5_5A5A) so the numbers are byte-identical to the captured baseline.
---

# /rulake-kernel-bench

Runs the criterion bench against each registered kernel, sequentially (kernels share host CPU/GPU; parallel skews timings).

```text
/rulake-kernel-bench
```

Sample output:

```text
Headline grid: dim=384, n=16384, top_k=10

  cpu-naive   2.8065 ms    1.000x  (baseline)
  avx-512     2.7082 ms    0.965x  (2.87% faster)
  wgpu        7.8270 ms    2.756x  (slower — transfer-bound at this grid)

Verdict per ADR-157 acceptance gate (2x p95 lower OR 30% cost lower at recall@10):
  - avx-512: not promoted past experimental on this grid
  - wgpu:    not promoted past experimental on this grid

Both kernels stay off-by-default. Capture as: docs/research/benchmarks/kernel-l2-three-way.md
```

The captured baseline lives at [`docs/research/benchmarks/kernel-l2-three-way.md`](https://github.com/ruvnet/RuLake/blob/main/docs/research/benchmarks/kernel-l2-three-way.md) and includes provenance (CPU model, GPU adapter, kernel version, criterion config) so the numbers are reproducible.
