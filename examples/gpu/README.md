# GPU examples

Witness-pinned GPU vector search against ruLake snapshots, in three
modules. None of these go through ruLake's rabitq compressed-scan
path (there's no GPU port of that kernel today — see module 03 for
why and how to fix it). What they do prove is that the bundle
protocol gives a clean GPU integration story even before the M2+
kernel plane lands.

## Layout

```
examples/gpu/
├── README.md                       # this file
├── 01-cuda-brute-force/            # native CUDA via cudarc + nvcc
├── 02-wgpu-portable/               # cross-platform via wgpu
└── 03-rabitq-gpu-design-note/      # markdown-only: how to ship a real
                                    # GPU port of the rabitq scan
```

## Why GPU examples can't (yet) call into ruLake's compressed scan

ruLake's native search runs **RaBitQ 1-bit popcount over compressed
codes** on the host CPU using runtime-dispatched scalar / AVX2 /
AVX-512 kernels in `ruvector-rabitq`. There is **no GPU port of that
kernel today** —
[ADR-157](../../docs/adrs/) ("Optional Accelerator Plane —
`VectorKernel` Trait + Dispatch") is "Proposed — scaffolding-only" on
both ruLake and `ruvector-rabitq`, and the per-architecture kernels
(cuda, rocm, metal, wasm) are intended to ship as separate crates
(`ruvector-rabitq-cuda`, etc.) that don't exist yet.

So the modules below take a different path: they verify a ruLake
bundle on host (witness-pinned), read the data file behind it (the
same `ruvec1` format the existing Python `04-rag-grounded` consumes —
see [`examples/python/04-rag-grounded/pipeline.py`](../python/04-rag-grounded/pipeline.py)
for the parser), and run brute-force L2 nearest-neighbor on the GPU.
This is the "bypass-the-rabitq-codes" path. It's a real, useful,
honest GPU example: provenance is preserved end-to-end, the data file
is the same one the rest of the cross-language examples consume, and
the GPU kernel is exact (no recall loss). It just doesn't share work
with ruLake's compressed-scan kernel — yet.

[Module 03](./03-rabitq-gpu-design-note/README.md) is the technical
sketch of what closing that gap would look like.

## Module overview

### 01 — `01-cuda-brute-force/` (native CUDA)

A standalone Rust binary that loads a `table.rulake.json` + the
matching `ruvec1` data file, recomputes the SHAKE-256 witness with
the upstream algorithm, runs a CPU brute-force L2 baseline, then runs
the same L2 on the GPU via a `nvcc`-compiled CUDA kernel loaded at
runtime through `cudarc`. Reports CPU-vs-GPU timing and top-K
agreement with the CPU.

**Toolchain**: CUDA Toolkit 12.x or 13.x, NVIDIA GPU with compute
capability ≥ 7.5. Tested on RTX 5080 (sm_120, CUDA 13.0).

### 02 — `02-wgpu-portable/` (cross-platform via wgpu)

Same example, expressed against `wgpu` so it runs on Vulkan / Metal /
DX12 / WebGPU. Lower peak throughput than module 01 on NVIDIA HW
(~20× kernel speedup vs ~38×) but no NVIDIA-specific tooling needed
and it works on AMD, Intel, and Apple Silicon as-is.

**Toolchain**: any GPU + driver supported by one of wgpu's backends.
No CUDA toolkit, no `nvcc`. Tested on RTX 5080 + Vulkan.

### 03 — `03-rabitq-gpu-design-note/` (markdown only)

The "what would have to change to plumb the rabitq compressed scan
through the GPU" writeup that ADR-157 owes. Covers: the per-candidate
GPU kernel, the on-device top-K choice, the AoS↔SoA layout question,
how the new crate plugs into the `VectorKernel` trait dispatch, the
witness-compatibility argument, the recall-guarantee argument, and a
concrete implementation checklist for someone who wants to ship
`ruvector-rabitq-cuda`.

## What's the same across modules 01 and 02

Both modules:

- Use the **byte-exact** witness algorithm from `src/bundle.rs`
  (re-implemented in `src/witness.rs` of each module so the example
  doesn't need to depend on the upstream crate; tested against the
  committed cross-language fixture witness
  `dea58c64adb1eb4109438f0353a2b1749d4dc29ed7266e9236720ab6cf07d7e4`).
- Parse `ruvec1` files with the upstream caps from `src/backend.rs`
  (`MAX_PULLED_VECTORS = 100M`, `MAX_PULLED_DIM = 8192`,
  `MAX_PULLED_BYTES = 16 GiB`).
- Generate a deterministic query vector from a SHAKE-256 stream
  seeded by the bundle's `rotation_seed` so reruns are reproducible.
- Print the witness as the run's `provenance_id` so an operator can
  correlate which exact snapshot the GPU just searched.
- Produce 100% top-K set-agreement with the CPU baseline on
  non-degenerate inputs (brute-force L2 is exact; ULP differences
  only matter on near-ties, see each module's "On float
  order-of-operations" section).

## Running them

Each module is a standalone Cargo crate (empty `[workspace]` table)
so neither one drags into the host ruLake build. Standard pattern:

```bash
cd examples/gpu/01-cuda-brute-force         # or 02-wgpu-portable
cargo build --release
cargo run --release -- ./demo-snapshot --materialize-demo
```

See each module's README for the full toolchain + example output.

## Related

- [ADR-155](../../docs/adrs/) — the bundle protocol that makes any of
  this possible.
- [ADR-157](../../docs/adrs/) — the proposed accelerator plane that
  module 03 sketches the implementation of.
- [`examples/python/04-rag-grounded/`](../python/04-rag-grounded/) —
  the same `ruvec1`-consumption pattern in Python; the GPU modules
  are essentially "what if step 4 (brute-force L2) ran on the GPU?"
- [`BENCHMARK.md`](../../BENCHMARK.md) — the CPU rabitq scan numbers
  the future `ruvector-rabitq-cuda` benchmark should compare against.
