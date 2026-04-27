# ruLake accelerator plane — A Deep Introduction

## TL;DR

The ruLake accelerator plane is the runtime + crate layout that lets the same `RuLake` binary route the popcount-scan + L2² rerank inner loop to a CPU-naive baseline, an AVX-512 host kernel, or a portable GPU kernel via wgpu — without forcing any of those backends into the core dependency graph and without ever letting a non-deterministic kernel feed a witness-sealed answer. The contract is the `VectorKernel` trait at `crates/core/src/kernel.rs` (id, caps, scan), the dispatch policy lives in `RuLake::pick_kernel` and consults `Consistency` + batch + dim, and conformance against `assert_kernel_conformant` is the only path past experimental. Two implementations ship today as standalone sibling crates — `crates/kernel-avx512/` (bit-equal, ~2.5% faster than CpuNaive on the headline grid where sort dominates) and `crates/kernel-wgpu/` (auto-detects Vulkan/Metal/DX12/GL/WebGPU adapters; deterministic on the popcount path, coarse-deterministic on L2 because WGPU's `f32` is sub-ULP-driver-dependent). Neither is enabled by default; operators register them explicitly. The load-bearing claim is that you can hand ruLake an arbitrary mix of CPU/SIMD/GPU/edge kernels, run on a Raspberry Pi, an AVX-512 server, or an RTX 5080 box, and get bit-identical witnesses on every Fresh / Frozen path no matter which kernel answered.

## Introduction

The intermediary tax on a ruLake cache hit is **1.02× direct RaBitQ** (`crates/rulake/BENCHMARK.md`). The cache is not the bottleneck. When a bottleneck does appear, it appears in the **kernel** — the popcount Hamming scan and the exact L2² rerank — and it scales as `n × D` for the scan and `rerank_factor × k × D` for the rerank. Both become load-bearing past three thresholds: **D ≥ 768** (BERT-large, MPNet, SBERT-large embeddings), **n ≥ 1M** (genomics shards, large-corpus RAG indexes), and **batch ≥ 256 queries per wave** (multi-agent reasoning where the planner fans out). The CPU-naive scan is fine for D=128 / n=100k / batch=1; it is not fine for D=1536 / n=20M / batch=512.

The deployment topology compounds the problem. ruLake's stated targets are not one machine; they are five:

| Target | Accelerator profile |
|---|---|
| Laptop / dev | scalar CPU only, sometimes portable SIMD |
| CI runner | scalar CPU; AVX-512 on x86 hosts that have it |
| Server / Cloud Run | CPU + AVX-512 or NEON, sometimes a CUDA card |
| Cognitum box | CPU + dedicated GPU (CUDA / ROCm / Metal) |
| Browser / Cloudflare Worker / edge | WASM with optional WASM SIMD; no GPU |

If the kernel is hard-wired into the core crate, every one of those targets either fails the build or ships dead-weight bindings. If we feature-gate every option behind `cfg(feature = "cuda")` / `cfg(feature = "wgpu")` / `cfg(feature = "wasm-simd")`, the matrix becomes a permanent maintenance liability and the binary is fixed at compile time — a laptop binary cannot opportunistically use a GPU that is plugged in at runtime, and a server binary forced to fall back to scalar on a CUDA-less node will not survive deploy heterogeneity. The CVE history of "we tried to compile-time-pick a SIMD path" is a parade of "SIGILL on the wrong host."

ADR-157 picks the third option: **a runtime trait, runtime dispatch, and a hard determinism gate on witness-sealed paths.** The trait shape commits to three calls — `id`, `caps`, `scan` — and stays index-stateless so a GPU kernel does not own RaBitQ-index lifetime; ruLake hands it the index by reference per call. The dispatch policy lives in `RuLake::pick_kernel` because only the cache sees the live signals (`batch_size`, `dim`, current `Consistency`) that determine the CPU/GPU crossover; rabitq sees a single-index single-query call and cannot make that decision. The witness chain stays anchored on `(data_ref, dim, rotation_seed, rerank_factor, generation)` and **does not include kernel identity** — kernels are execution substrate; data is what gets sealed. Kernel identity surfaces in stats and logs so operators can answer "which kernel answered the last 1k queries," but it never enters a witness preimage.

The most consequential commitment is the **fail-closed determinism rule**. The popcount Hamming scan is integer math (XOR + popcount + accumulate); every kernel must produce the same byte-equal set of top-k candidates pre-rerank, in the same order, including tie-broken-by-lower-index. The L2² rerank may be float-non-deterministic — IEEE-754 reduction order on a GPU's parallel sum is not the same as scalar left-fold, and the WGPU spec deliberately does not pin sub-ULP behavior across drivers. Kernels declare `caps().deterministic` honestly. Dispatch enforces: **non-deterministic kernels are forbidden on `Consistency::Fresh` and `Consistency::Frozen` paths** and may only answer `Consistency::Eventual` queries (which by design tolerate recall drift). This is the rule that lets the witness chain stay valid across kernel diversity — two ruLake instances on heterogeneous hardware can answer the same Fresh query and produce byte-identical bundles.

Two kernels ship today, and both demonstrate the contract under different constraints. They are the existence proof for ADR-157, not the end of it.

## The decision in detail

### The trait

`VectorKernel` lives in `crates/core/src/kernel.rs` and is exported from the root `rulake` crate. The shape is intentionally small — three methods, no associated types, `Send + Sync` so kernels can live in `Arc<dyn VectorKernel>` and ride between worker threads without ceremony.

```rust
pub trait VectorKernel: Send + Sync {
    fn id(&self) -> &str;             // "cpu-naive" | "avx512" | "wgpu" | ...
    fn caps(&self) -> KernelCaps;     // min_batch, max_dim, deterministic, accelerator
    fn scan(&self, req: ScanRequest<'_>) -> Result<ScanResponse>;
}
```

`ScanRequest<'_>` carries one or more queries, the candidate codes by reference, the `top_k`, and the `dim`; `ScanResponse` carries `top_k` indices + scores per query. The kernel never owns the index — it borrows for the duration of the call. This is the structural decision that lets a GPU kernel exist without coupling to the cache's invalidation lifecycle: when the cache invalidates a generation, in-flight kernel scans against the previous generation are allowed to complete on borrowed memory; the next call binds to the new generation. No GPU-side memory is held by the kernel between calls (the WGPU implementation re-uploads on each `scan`; this is suboptimal for steady-state high-batch workloads and is on the v1.x roadmap to change to a generation-keyed cache).

`KernelCaps` is the only place a kernel can lie to the dispatcher. The four fields are:

| Field | What it pins |
|---|---|
| `min_batch: usize` | Below this, dispatch never picks this kernel. CPU kernels report 1; the WGPU kernel reports 64 because transfer overhead dominates below that. |
| `max_dim: usize` | Hard ceiling. Dim above this triggers a fallback. AVX-512 kernel: `usize::MAX` (no ceiling). WGPU kernel: 4096 (workgroup-size constraint in the current shader). |
| `deterministic: bool` | The fail-closed flag. AVX-512 kernel: `true` (bit-equal to CpuNaive on the conformance fixture). WGPU popcount kernel: `true`. WGPU L2 kernel: `false` (matches top-k *set* but not raw distances, last-ULP). |
| `accelerator: &'static str` | Symbolic label for stats. `"cpu"` / `"cpu-simd"` / `"cuda"` / `"metal"` / `"wgpu"` / `"wasm-simd"`. Surfaced in `CacheStats`. |

The dispatch loop is the smallest amount of code that could possibly enforce the contract:

```rust
fn pick_kernel(&self, batch_size: usize, dim: usize, frozen: bool) -> Arc<dyn VectorKernel> {
    let deterministic_required = frozen || self.consistency == Consistency::Fresh;
    for k in self.kernels_by_preference() {
        let c = k.caps();
        if batch_size < c.min_batch { continue; }
        if dim > c.max_dim { continue; }
        if deterministic_required && !c.deterministic { continue; }
        return k;
    }
    self.default_cpu_kernel()
}
```

Preference order is hard-coded most-accelerated-first: `cuda` / `rocm` / `metal` → `wgpu` → `cpu-simd` → `cpu`. ruLake does not ship with GPU kernels enabled. Operators register them explicitly via `RuLake::register_kernel(Arc::new(WgpuKernel::new_blocking()?))` — symmetric with `register_backend`.

### The conformance gate

A new kernel is **promoted past experimental** iff it passes `assert_kernel_conformant(&kernel)` in `crates/core/src/kernel.rs`. The fixture is small and deliberately mean: a clustered D=768 n=1M index with rerank×20 (the ADR-157 reference grid). The assertion checks two things — bit-exact top-k *set* on the popcount-scan phase, and bit-exact ordering tie-broken by lower index. The L2² rerank is held to a coarser test (set match, ordering match, distance equality on integer-arithmetic kernels only). A kernel that fails the gate stays in its experimental crate; it does not land in the default dispatch preference; operators who want it must enable it explicitly with eyes open.

Plus the soft conditions, all measured on the reference grid:

- p95 query latency ≥ 2× lower than CpuNaive at identical recall@10, **or**
- cost per 1M queries ≥ 30% lower at identical recall@10
- memory safety: no leak above 2× index bytes during steady-state serving
- passes the full ruLake smoke suite end-to-end

The acceptance gate exists to stop kernel-vanity: a GPU kernel that demonstrates a 1.3× win at D=128 n=10k batch=1 is not interesting because that workload runs faster on a single CPU core anyway. The gate forces measurement at the only point where a kernel decision is load-bearing.

### Crate placement

The trait is in the core `rulake` crate (`crates/core/src/kernel.rs`) because every consumer needs the trait shape to write `register_kernel` calls. Implementations are sibling crates — never workspace members, never feature-gated submodules of the core crate, per ADR-001:

| Crate | What it ships |
|---|---|
| `rulake` (core) | `VectorKernel` trait, `CpuNaiveKernel` baseline, `assert_kernel_conformant` fixture, `KernelCaps`, dispatch policy, `register_kernel` API |
| `crates/kernel-avx512/` | `Avx512Kernel`, runtime CPUID gate, `target_feature`-scoped intrinsics |
| `crates/kernel-wgpu/` | `WgpuKernel`, WGSL shaders, fail-closed adapter request, host-side top-k sort |
| `kernel-cuda` (future, separate repo) | CUDA bindings, license footprint isolated |
| `kernel-metal` (future, separate repo) | Metal Performance Shaders bindings |

Feature-gated kernels inside the core crate were rejected (ADR-157 §A) because every GPU dependency is 1k+ lines of FFI plus a driver matrix plus its own CI pain — pulling CUDA into `rulake` breaks laptop and WASM builds unless everything is feature-gated, and feature-gate matrices are a maintenance sink that scales poorly across five deployment targets. Separate crates amortize the pain and let customers pick the one their platform supports. WASM SIMD is the one exception — it's the same source compiled with `--target=wasm32-*`, so it lives feature-gated inside the core crate for the WASM path, not as a separate repo.

### The two shipped kernels

**`crates/kernel-avx512/` — the host SIMD kernel.** Wraps `_mm512_sub_ps` + `_mm512_fmadd_ps` for the L2² inner loop and `_mm512_popcnt_epi64` (the `avx512vpopcntdq` extension) for the Hamming popcount. Both paths are bit-equal to `CpuNaiveKernel` on the conformance fixture — the SIMD math is integer-arithmetic on the popcount side and FMA on the L2 side, and the FMA is fused-multiply-add which produces the same result as `(a*b)+c` to within zero ULP, so the L2 path stays deterministic too. Construction is `Avx512Kernel::new() -> Option<Self>` — `Some` iff the host CPU advertises **all four** of `avx512f`, `avx512bw`, `avx512vl`, `avx512vpopcntdq`; `None` otherwise. This is the runtime feature-detection that lets a single binary ship to mixed fleets without SIGILL. Safety is localized: every `unsafe` block is inside a `#[target_feature(enable = ...)]` function, every entry into those functions is gated on the `Some(_)` constructor path. Bench result on the headline grid (D=384, n=16384, top_k=10): **~2.5% faster than CpuNaiveKernel.** The reason it's not larger is that the sort dominates — at this grid the popcount/L2 inner loop is already cheap enough that AVX-512 only shaves a small slice. The win materializes at high D and large n, which is exactly where the CPU baseline gets expensive.

**`crates/kernel-wgpu/` — the portable GPU kernel.** Runs the inner loops on whatever compute-capable backend wgpu finds at startup: **Vulkan / Metal / DX12 / GL / WebGPU**. On the test bench (NVIDIA RTX 5080 via Vulkan) wgpu picks the discrete GPU automatically. Two WGSL shaders: an L2 path (`shaders/l2.wgsl`, one workgroup over the candidate batch, one thread per (query, candidate) distance, reduction on host) and a popcount path (`shaders/popcnt.wgsl`, packed 1-bit codes XORed and popcounted per `u32` lane via `countOneBits`). The key trick is that **only the per-candidate distance is computed on GPU; the top-k sort runs on the host with the same byte-equal tie-break the naive kernel uses.** This is what lets the GPU path stay deterministic on the popcount conformance fixture — the scan is integer arithmetic on both sides and the host-side sort is shared. The L2 path is *not* bit-equal to CpuNaive because WGSL `f32` operators are IEEE-754 but the WGPU spec doesn't pin sub-ULP behavior across drivers; top-k *set* matches and distance ordering matches, but raw distances may differ in the last ULP. Conformance therefore exercises the popcount path (exact) and a coarse L2 path (set, not raw distances). Construction is `WgpuKernel::new_blocking() -> Result<Self>` — `Ok` iff wgpu can request an adapter + device; `Err(WgpuKernelError::NoAdapter)` on headless CI hosts (callers fall back to CpuNaiveKernel). Bench result on the headline grid: **7.4 ms vs 2.85 ms for AVX-512** — i.e., the GPU loses on this grid because host↔device transfer dominates over the work. The kernel earns its keep at higher D and / or larger batch where transfer is amortized.

The two kernels together are the existence proof: same trait, same conformance fixture, two different accelerator categories (host SIMD intrinsics vs portable GPU shader), both deterministic on the witness-relevant path, both fail-closed at construction so a binary with both crates linked just falls back gracefully on a host that has neither AVX-512 nor a GPU.

### Determinism, witness, and dispatch interaction

The witness chain stays anchored on `(data_ref, dim, rotation_seed, rerank_factor, generation)` regardless of which kernel answered. This is the **load-bearing structural decision** — kernel identity does not enter the witness preimage. If the witness changed per kernel, two ruLake instances on heterogeneous hardware could not agree on a bundle hash even though both saw the same data, which would break the cross-host trust contract that the SHAKE-256 witness exists to enforce. The dispatcher's job is to make sure the choice of kernel never matters for the bytes returned on a witness-sealed path — and that's done by the `deterministic` caps bit plus the Fresh/Frozen filter.

The escape hatch for non-deterministic kernels is `Consistency::Eventual`. An operator who knowingly enables a non-deterministic GPU kernel on an Eventual collection accepts that top-k orderings may differ across restarts; the caps bit is the warning sign and the deployment doc spells it out as a first-class operational concern. The current dispatch policy refuses to use a non-deterministic kernel on Frozen (audit-tier reproducibility) outright; this is the conservative read of "Frozen means bit-reproducible and that means kernel-independent."

| trade-off | what got picked | what got rejected | why |
|---|---|---|---|
| trait location | `rulake` core | `ruvector-rabitq` (kernels are RaBitQ primitives) | every consumer needs the trait shape; cache is the only place that sees batch-size signals |
| dispatch location | `RuLake::pick_kernel` | inside `ruvector-rabitq` per-call | rabitq sees one query, one index — can't make a CPU/GPU crossover decision |
| GPU kernel placement | sibling crates per backend | feature-gated submodules of core | feature-matrix scales poorly across CUDA / ROCm / Metal / wgpu / WASM-SIMD |
| WASM SIMD | feature gate inside core | separate `rulake-wasm-simd` crate | same source compiled with `--target=wasm32-*`; no new crate needed |
| determinism | hard gate via `caps().deterministic` + dispatch filter | trust the kernel author | non-deterministic GPU rerank is a real category; explicit caps + filter is the only honest read |
| witness anchoring | `(data_ref, dim, rotation_seed, rerank_factor, generation)`, kernel identity excluded | include kernel id in witness | heterogeneous hardware would never agree on bundles |
| acceptance gate | 2× p95 lower OR 30% cost lower at identical recall@10 | "feels faster" | stops kernel-vanity at workloads where the choice doesn't matter |
| construction shape | fail-closed `Option`/`Result` from `new()` | panic on missing CPUID / adapter | mixed-fleet binary must run unchanged on hosts that lack the accelerator |
| acceptance fixture | clustered D=768 n=1M rerank×20 | small grids | the only point where kernel choice is load-bearing |
| top-k sort | always on host, never on GPU | GPU sort | host-side sort is the deterministic shared step that lets GPU L2 stay coarse-deterministic |

## What ships today, what's next

Two kernels: AVX-512 host SIMD (bit-equal, ~2.5% headline win where sort dominates, real win at high D), and wgpu portable GPU (auto-detects every wgpu-supported backend, deterministic popcount, coarse L2). Both pass `assert_kernel_conformant` on the popcount path; both are off-by-default and registered explicitly. The conformance test in `crates/core/src/kernel.rs` is the only path past experimental — no kernel ships in the default dispatch preference until it crosses the gate.

The trait shape was deliberately designed to accommodate `wgpu` when wgpu's compute capabilities matured (ADR-157 §E rejected wgpu in 2026-04 because the shader model didn't yet expose popcount + reduction primitives that match native CUDA on the scan phase; eight months later wgpu's `countOneBits` plus storage-buffer atomics made it viable). The same shape accommodates a future CUDA / ROCm / Metal kernel (separate crates per ADR-157 §A) and a WASM SIMD path inside the core crate (open question 1; current direction is feature-gate not separate crate). Open questions still open: kernel identity in `CacheStats` exposure (open question 5; useful but scope-creep toward observability that should live in tracing); empirical batch-size autotuning to replace the conservative static `min_batch` hint (open question 3; deferred until a third kernel exists to measure against); and whether `Consistency::Frozen` should warn instead of refuse on non-deterministic kernels (open question 4; current answer is refuse, on the read that "Frozen means bit-reproducible and that means kernel-independent").

The accelerator plane is, in short, the plumbing that lets ruLake stay one binary across a Raspberry Pi and an RTX 5080 box without making the witness chain pretend either of them is the other.

---

**Repo:** [ruvnet/RuLake](https://github.com/ruvnet/RuLake) · **Live demo:** <https://ruvnet.github.io/RuLake/> (auto-probes the live MCP at [`https://rulake-mcp.ruv.io/`](https://rulake-mcp.ruv.io/)) · **ADR:** [`docs/adrs/ADR-157-optional-accelerator-plane.md`](../adrs/ADR-157-optional-accelerator-plane.md) · **Crates:** [`crates/kernel-avx512/`](../../crates/kernel-avx512/) · [`crates/kernel-wgpu/`](../../crates/kernel-wgpu/)
