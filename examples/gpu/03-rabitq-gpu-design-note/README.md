# 03 — rabitq-gpu-design-note

A technical writeup of what it would take to actually port ruLake's
RaBitQ-1bit compressed scan to the GPU, plug it into ADR-157's
`VectorKernel` trait, and ship it as a separate crate without breaking
the witness-pinned coherence story.

This module ships **no code**. It exists as the bridge between:

- "ruLake doesn't ship GPU kernels today" (the truthful framing in
  modules 01 + 02 — both bypass the rabitq scan and run brute-force
  L2 over the raw vectors), and
- "here's exactly how the M2+ kernel plane fills in" (this document —
  the missing concrete sketch that ADR-157, currently "Proposed —
  scaffolding-only", needs before someone can implement it).

If you're reading this because you intend to ship a GPU rabitq scan,
this is the starting checklist.

## 1. What the CPU rabitq scan actually does

The scan kernel lives in `ruvector-rabitq` and does roughly this for
each query:

1. Apply the Haar-uniform rotation (declared by the bundle's
   `rotation_seed`) to the query vector. The rotation is shared across
   the whole index and computed once at build time; query-time it's a
   single dense GEMV.
2. Quantize the rotated query to a 1-bit code (sign bit per
   dimension), giving a `ceil(dim / 64)` u64 vector.
3. For each candidate, popcount the XOR of query-code and
   candidate-code. That popcount is the Hamming distance, which
   estimates the L2 distance under the rotation.
4. Keep the top-`rerank_factor × k` Hamming-best candidates; rerank
   them by exact L2 against the original f32 vectors; return top-K.

Steps 3-4 are the hot loop. The dispatch table in `ruvector-rabitq`
selects between scalar / AVX2 / AVX-512 popcount paths at runtime; on
a Skylake-AVX-512 core the AVX-512 path measures ~100M codes/sec/core
(per `BENCHMARK.md`).

## 2. The minimal GPU port

A single CUDA kernel (or WGSL compute shader) per warp does steps 3-4
trivially well:

```cuda
// One thread per candidate. dim_words = ceil(dim / 64).
extern "C" __global__ void rabitq_scan(
    const uint64_t * __restrict__ query_code,    // [dim_words]
    const uint64_t * __restrict__ codes,         // [n * dim_words], row-major
    uint32_t       * __restrict__ out_hamming,   // [n]
    uint32_t n,
    uint32_t dim_words
) {
    uint32_t idx = blockIdx.x * blockDim.x + threadIdx.x;
    if (idx >= n) return;
    const uint64_t * row = codes + (size_t)idx * (size_t)dim_words;
    uint32_t acc = 0;
    #pragma unroll 4
    for (uint32_t w = 0; w < dim_words; ++w) {
        acc += __popcll(query_code[w] ^ row[w]);
    }
    out_hamming[idx] = acc;
}
```

On WGSL the equivalent uses `countOneBits(u32)` over `dim_words * 2`
u32s (WGSL doesn't expose u64 yet); the dispatch shape is the same.

### Per-candidate cost

For `dim = 1024`, `dim_words = 16`, that's:

- 16 × `__popcll` ops per candidate. On Ampere/Hopper/Blackwell this
  is one cycle each at full throughput.
- 16 × 8 = 128 bytes of XOR-able codes loaded per candidate, fully
  coalesced when adjacent threads scan adjacent rows.

That puts the kernel at **memory-bandwidth-bound** in practice. An
RTX 5080 is ~1 TB/s of L2/HBM — at 128 bytes/candidate that's
~7.8 G candidates/sec, **78× the AVX-512 single-core measurement**.
Single-machine end-to-end speedup vs a 16-core AVX-512 host is more
like 5-10× because the host can run the AVX-512 path on every core
in parallel.

### Top-K reduction

The brute-force example in module 01 punts top-K to the host (memcpy
+ host sort). For the production rabitq port that's **not** acceptable
because the rerank step needs the original f32 vectors for the top
`rerank_factor × k` candidates and you'd rather not memcpy `n`
distances back. Two options:

1. **CUB::DeviceRadixSort.SortPairs** — sort hamming distances and
   indices, take the prefix. Bounded ~2× the scan time at typical n.
2. **Bitonic top-K with warp-shuffle reduction** — keeps the working
   set in shared memory, avoids the device-sort overhead, but is
   harder to write portably (no CUB equivalent in WGSL). Best for n
   ≤ ~1M per shard.

Either is fine. Pick one and commit; don't ship an option matrix.

## 3. How it plugs into ADR-157

ADR-157 declares a `VectorKernel` trait that dispatches based on:

- **rotation** (seed → rotated query)
- **rerank-factor** (governs how many candidates flow into the rerank pass)
- **index-format** (today: rabitq-1bit; future: rabitq-Nbit, IVF, HNSW)
- **device** (today: scalar / AVX2 / AVX-512; future: cuda, rocm, metal, wasm)

The intent (per the ADR) is that each per-architecture kernel ships
as a **separate crate** so consumers opt into the GPU dependency
explicitly. For CUDA that crate is `ruvector-rabitq-cuda`:

```text
ruvector-rabitq/                 (CPU; today)
├── src/scalar.rs
├── src/avx2.rs
├── src/avx512.rs
└── src/dispatch.rs              (registers VectorKernel impls)

ruvector-rabitq-cuda/            (CUDA; new — what this doc is about)
├── src/lib.rs                   (registers CudaScanKernel)
├── src/scan.cu                  (the kernel above)
├── src/topk.cu                  (or call CUB)
└── build.rs                     (nvcc → PTX, embedded)

ruvector-rabitq-wgpu/            (cross-platform; later)
└── src/scan.wgsl
```

The dispatch lookup at search-time is unchanged: the `VectorKernel`
trait object is selected from a registry at `RuLake::new`, and the
GPU crate registers itself via a `ctor`-style hook (or an explicit
`with_kernel(CudaScanKernel::new())` opt-in builder).

**Failure mode contract**: if the GPU kernel's runtime
initialization fails (no device, OOM, driver too old), the search
path silently falls back to the CPU kernel. The witness is
unchanged, so downstream callers can't tell which kernel ran. This
is what makes the M2+ kernel plane substitution-safe.

## 4. Witness compatibility

The cleanest reason to do this work is that **the witness doesn't
change**. The compressed codes are determined by:

- the input f32 vectors,
- the bundle's `rotation_seed`,
- the rabitq quantization (1-bit threshold at zero on the rotated
  vector — no learned parameters),
- the rabitq build-time rotation (Haar-uniform from the seed).

None of those depend on the device. So:

- Build the codes on CPU. Store as `index.rbpx` next to the bundle.
- At query time, the CPU kernel and the GPU kernel produce
  **byte-identical** Hamming distances for every (query, candidate)
  pair. The top-K set is identical (modulo the same float ULP swaps
  noted in the brute-force examples).
- The bundle's `rvf_witness` is anchored on `data_ref` + `dim` +
  `rotation_seed` + `rerank_factor` + `generation`. None of those
  change. The witness matches.

The integration tests for `ruvector-rabitq-cuda` should assert this
byte-exactness against the existing scalar / AVX2 outputs on a
matrix of corpora — the same test pattern `ruvector-rabitq` already
uses to validate AVX-512 against scalar. **A new test file should
exist that cross-checks every (cpu, gpu) pair on at least 100
randomized corpora**; the day that test fails is the day the kernel
plane breaks the cache-coherence story for everyone using ruLake
across heterogeneous devices.

## 5. Memory layout: SoA vs AoS

`RabitqPlusIndex` today stores codes as AoS — row-major
`u64[n][dim_words]`. That's optimal for CPU SIMD popcount (each
register holds one row's worth of codes) but **suboptimal for GPU**.

The GPU-friendly layout is **SoA**: `u64[dim_words][n]` — column 0
holds word-0 of every candidate, column 1 holds word-1, etc. With
this layout:

- Adjacent threads in a warp read adjacent candidates, so each
  word-load fetches 32 × 8 = 256 contiguous bytes — exactly one
  warp-coalesced cache line.
- The inner loop reduces from "16 strided loads" to "16 broadcast-style
  loads, each fully coalesced".

This yields ~2-3× more sustained bandwidth on Ampere+. Two options
for shipping it:

1. **Transposed mirror at build time**: keep AoS for CPU, write a
   sibling SoA layout (`index.rbpx.soa`) at the same publishing
   point. Doubles disk usage, zero CPU runtime cost, witness still
   anchored on the source.
2. **Build-time switch**: a `--gpu-layout` flag on the publisher that
   chooses AoS or SoA. Simpler on disk, requires picking up-front
   which kernel will run.

Pick (1). It's the same idea as ruLake itself — the cache trades
disk space for substitution flexibility.

## 6. Recall guarantee

ruLake's recall envelope is set by `rerank_factor`: the rabitq scan
produces a Hamming-ranked candidate set, the top
`rerank_factor × k` of which are reranked exactly. The published
guarantee is **≥90% recall@10 single-shard** at the default
`rerank_factor = 20`.

The GPU kernel produces identical Hamming distances (see §4), so it
produces an identical candidate set, so the recall is identical.
**The GPU port doesn't relax the recall guarantee**.

This is a subtle but important property: it means a deployment can
substitute the GPU kernel under a service for performance without
re-running the rerank-factor sweep that originally validated the
recall envelope.

## 7. Concrete next steps for someone who wants to ship this

In dependency order:

1. **Create `ruvector-rabitq-cuda` crate skeleton.** Standalone
   workspace member, `cudarc` 0.19 dep with the same feature flags
   the brute-force example uses (`std`, `driver`, `nvrtc`,
   `fallback-dynamic-loading`, `cuda-XXXXX` for your toolkit). Empty
   `[workspace]` table to keep it out of the host build.
2. **Write the kernel.** One `.cu` file with the scan above. Build
   it via `nvcc → PTX` in `build.rs`, exactly like
   `examples/gpu/01-cuda-brute-force/build.rs`.
3. **Implement `CudaScanKernel: VectorKernel`.** Holds a
   `CudaContext` + `CudaModule` + the `u64` device buffer for the
   index. `scan()` is a one-shot upload of the query code, dispatch,
   readback of distances. `from_index(&RabitqPlusIndex)` does the
   AoS-to-SoA transpose + upload.
4. **Add the byte-exactness test suite.** Generate ~100 random
   corpora at varying (n, dim) shapes, run them through both the
   CPU AVX-512 kernel and `CudaScanKernel`, assert hamming
   distances match for every (query, candidate). This is the load-
   bearing test — without it, a future regression in the kernel
   silently breaks the cache-coherence story.
5. **Add a benchmark.** Mirror `ruvector-rabitq/BENCHMARK.md`'s
   shape: codes/sec at (n, dim) ∈ {(100k, 128), (1M, 256), (10M,
   1024)}. Compare against the AVX-512 single-core line and the
   16-core line.
6. **Wire the registry hook.** Optional `with_cuda_kernel(0)`
   builder method on `RuLake` that constructs a `CudaScanKernel` on
   GPU 0 and registers it. **Default behavior unchanged** — opt-in
   only, falls back to CPU on any failure.
7. **Document the failure mode.** A short addendum to
   `BENCHMARK.md` explaining when the GPU path wins
   (n ≥ ~100k, throughput-bound serving) and when it loses (small
   shards, latency-sensitive single-query, no rerank to amortize the
   download cost). Same level of honesty as the modules 01/02
   READMEs.

## 8. What this document is NOT

It is not an ADR. It is not a commitment. It is the technical sketch
the next person who picks up ADR-157 needs in order to write
`ruvector-rabitq-cuda`. ADR-157 is the governance document; this is
the engineering pre-flight.

If you write the crate, please update this doc with the actual
measured numbers and link the sibling crate from the file map.

## File map

```
03-rabitq-gpu-design-note/
└── README.md            (this file — markdown only, no code)
```

## See also

- `examples/gpu/01-cuda-brute-force/` — a working CUDA example that
  bypasses the rabitq scan and proves the bundle protocol gives a
  clean GPU integration story even before this kernel plane lands.
- `examples/gpu/02-wgpu-portable/` — same, but cross-platform via wgpu.
- `BENCHMARK.md` — current AVX-512 / AVX2 / scalar throughput numbers
  the GPU port should be benchmarked against.
- `src/bundle.rs` — the witness algorithm whose byte-exact stability
  across the GPU port is the key correctness invariant.
- `ruvector-rabitq/` — the CPU kernel the GPU port mirrors.
