# 02 — wgpu-portable

Witness-pinned cross-platform GPU L2 kNN over a ruLake snapshot.
Identical structure to module 01, but the GPU side is implemented with
[`wgpu`](https://wgpu.rs) so it runs unmodified on:

- **Vulkan** (Linux + Windows, NVIDIA / AMD / Intel)
- **Metal** (macOS + iOS, Apple Silicon)
- **DX12** (Windows)
- **WebGPU** (browser)

No NVIDIA-specific tooling required — no CUDA toolkit, no `nvcc`.

## Important: this does NOT call into ruLake's compressed-scan path

Same caveat as module 01. ruLake's native search runs **RaBitQ 1-bit
popcount over compressed codes** on the host CPU using
runtime-dispatched scalar / AVX2 / AVX-512 kernels in
`ruvector-rabitq`. There is no GPU port of that kernel today
([ADR-157](../../../docs/adrs/) is "Proposed — scaffolding-only").

What this example proves is that the **bundle protocol** is decoupled
enough that a GPU pipeline can verify provenance, consume the same
data file the cross-language examples consume, and run an exact L2
comparison on the GPU — no waiting on the M2+ kernel plane.

For the design of an actual GPU port of the rabitq scan, see
[`../03-rabitq-gpu-design-note/README.md`](../03-rabitq-gpu-design-note/README.md).

## Toolchain

- Rust 1.87+ (tested with 1.95).
- A GPU + driver supported by one of wgpu's backends:
  - Linux + NVIDIA: Vulkan driver (libvulkan1, mesa-vulkan-drivers, or
    NVIDIA proprietary). Tested on **RTX 5080 + driver 580.x + Vulkan**.
  - macOS: Metal (always available on supported macOS versions).
  - Windows: DX12 or Vulkan.

No CUDA toolkit needed. No `nvcc` invocation in `build.rs` (this crate
doesn't have one — the WGSL shader is included as text and compiled by
the wgpu runtime against whichever backend is active).

## Install + build

```bash
cd examples/gpu/02-wgpu-portable
cargo build --release
```

First build pulls `wgpu` and its backend deps; on Linux this includes
`ash` (Vulkan loader bindings). The build does not require any GPU on
the build machine — only running the binary does.

## Run

```bash
# Materialize a 100,000 × 128-d demo snapshot at ./demo-snapshot, then
# run the comparison.
cargo run --release -- ./demo-snapshot --materialize-demo

# Against a real ruLake-published snapshot:
cargo run --release -- /path/to/snapshot --k 10

# Against the committed cross-language fixture:
cargo run --release -- ./fixtures --k 5
```

## Expected output

Measured on an RTX 5080 (16 GB) via the **Vulkan** backend, CUDA-class
host, release build, with `--materialize-demo --demo-n 100000
--demo-dim 128 --k 10`:

```text
[demo] materialized synthetic snapshot at /tmp/rulake-wgpu-demo (100000 vectors × 128 dims)
[bundle] verified witness d89ec243…a119 (data_ref=file:///tmp/rulake-wgpu-demo/vectors.ruvec1)
[corpus] loaded 100000 vectors × 128 dims from /tmp/rulake-wgpu-demo/vectors.ruvec1
[cpu]   top-10 L2 in    5.035 ms (100000 candidates × 128 dims)
[gpu]   adapter: Vulkan on NVIDIA GeForce RTX 5080
[gpu]   top-10 L2 in    0.240 ms (dispatch) +  9.287 ms (upload+download) =    9.527 ms total

=== summary ===
  provenance_id (witness):  d89ec243…a119
  n × dim:                  100000 × 128
  k:                        10
  backend:                  Vulkan on NVIDIA GeForce RTX 5080
  speedup (dispatch-only):   20.96x  (CPU   5.035 ms vs GPU   0.240 ms)
  speedup (incl. transfer):   0.53x  (CPU   5.035 ms vs GPU   9.527 ms)
  top-10 set agreement:        10/10  (100.0%)
```

A larger run (`--demo-n 500000 --demo-dim 256`) on the same machine:

```text
[cpu]   top-10 L2 in   47.562 ms
[gpu]   top-10 L2 in    2.473 ms (dispatch) + 57.465 ms (upload+download)
  speedup (dispatch-only):   19.23x
  speedup (incl. transfer):   0.79x
  top-10 set agreement:        10/10  (100.0%)
```

## How this compares to module 01 (CUDA)

Both modules implement the same kernel against the same data file.
At 100k × 128, on the same RTX 5080:

| Metric                        | CUDA (module 01) | wgpu (module 02) |
|-------------------------------|------------------|------------------|
| Dispatch-only time            | 0.13 ms          | 0.24 ms          |
| Upload + download time        | 5.6 ms           | 9.3 ms           |
| Speedup (dispatch-only)       | ~38×             | ~21×             |
| Top-K agreement vs CPU        | 100%             | 100%             |
| Toolchain dependency          | CUDA toolkit     | none             |
| Runs on AMD / Intel / Apple   | no               | yes              |

CUDA wins on raw throughput because:

- The CUDA driver compiles the PTX with `--use_fast_math` and explicit
  FMA at build time; wgpu's WGSL → SPIR-V → driver path is more
  conservative.
- cudarc's H2D/D2H lands directly on `cudaMemcpyAsync`; wgpu's
  `Queue::write_buffer` goes through a managed staging belt that
  serializes against the next submission.

Module 02 wins on portability — same source tree works on a Mac with
no NVIDIA hardware, and (with a thin web wrapper) in a browser via
WebGPU. Pick the one that matches your deployment.

## On float order-of-operations

Both implementations compute `Σ (qᵢ - vᵢ)²` for each candidate but in
slightly different orders (the WGSL spec doesn't guarantee FMA, so the
SPIR-V backend may emit explicit `mul` + `add`). For typical embedding
distributions this gives bit-identical L2 distances to the CPU
baseline. For pathological inputs the last few ULPs can differ, which
can swap the order of two near-tied candidates. The "top-K agreement"
metric counts set-agreement (a tie that swaps positions still counts
as a hit), so this is normally invisible. If you see ⟨k⟩/k less than
~95%, something has gone wrong upstream.

## Caps + limits

The binary mirrors the upstream `MAX_PULLED_VECTORS = 100,000,000`,
`MAX_PULLED_DIM = 8192`, and `MAX_PULLED_BYTES = 16 GiB` from
`src/backend.rs`. A hostile or accidentally-huge `ruvec1` is rejected at
parse time, before allocation.

`--max-vectors` adds a *demo cap* on top of those, defaulting to
1,000,000. **wgpu adds an extra ceiling**: each storage buffer must
fit `max_storage_buffer_binding_size`, which on most desktop drivers
is 2 GiB but on some integrated GPUs is 128 MiB. The binary queries the
adapter's reported limit and refuses requests it can't satisfy with a
clear error pointing at `--max-vectors` / `--demo-dim`.

## File map

```
02-wgpu-portable/
├── README.md            (this file)
├── Cargo.toml           (wgpu 29, pollster, bytemuck, sha3, hex, serde)
├── fixtures/
│   ├── table.rulake.json            (committed, witness-verified)
│   └── vectors.ruvec1               (committed, matching ruvec1 corpus)
└── src/
    ├── main.rs          (CLI: parse args, verify, CPU+GPU compare, report)
    ├── witness.rs       (local copy of compute_witness — byte-exact upstream)
    ├── ruvec1.rs        (ruvec1 parser/writer with the upstream caps)
    └── shaders/
        └── l2_search.wgsl  (WGSL compute shader — one thread per candidate)
```
