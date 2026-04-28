---
description: Report which ADR-157 kernels are registered against the running RuLake instance and their KernelCapabilities (simd_width, popcount_native, gpu, deterministic flag).
---

# /rulake-kernel-status

Walks the registered kernels (in dispatch-preference order) and reports each one's capabilities. Tells you what would dispatch under a given consistency mode.

```text
/rulake-kernel-status
```

Sample output:

```text
Registered kernels (in dispatch-preference order):
  1. wgpu       caps={ simd_width=1, popcount_native=false, gpu=true, deterministic=popcount-only }
  2. avx512     caps={ simd_width=16, popcount_native=true, gpu=false, deterministic=true }
  3. cpu-naive  caps={ simd_width=1, popcount_native=false, gpu=false, deterministic=true } (default)

Under Consistency::Fresh + batch=1: cpu-naive  (avx512 below min_batch threshold; wgpu non-deterministic on L2)
Under Consistency::Eventual + batch=64: wgpu    (deterministic-required filter off)
```
