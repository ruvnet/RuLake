# 01 — cuda-brute-force

Witness-pinned CUDA brute-force L2 kNN over a ruLake snapshot. A standalone
Rust binary that:

1. Reads `<snapshot-dir>/table.rulake.json`, recomputes its SHAKE-256 witness
   from the upstream algorithm (`src/bundle.rs`), and refuses to proceed on
   mismatch.
2. Reads the accompanying `ruvec1` data file (the format documented in
   `src/fs_backend.rs`).
3. Generates a deterministic query vector seeded by the bundle's
   `rotation_seed`.
4. Runs a CPU brute-force L2 baseline.
5. Uploads vectors + query to the GPU, runs the L2 kernel
   (`src/kernels/l2_search.cu`), brings distances back, and finishes top-K
   on the host.
6. Reports CPU time vs GPU time (kernel-only and including transfers),
   speedup, and the top-K agreement.

## Important: this does NOT call into ruLake's compressed-scan path

ruLake's native search runs **RaBitQ 1-bit popcount over compressed codes**
on the host CPU using runtime-dispatched scalar / AVX2 / AVX-512 kernels in
`ruvector-rabitq`. There is no GPU port of that kernel today
([ADR-157](../../../docs/adrs/) is "Proposed — scaffolding-only").

What this example proves is that the **bundle protocol** is decoupled enough
that a GPU pipeline can:

- verify provenance (witness is byte-exactly the upstream algorithm),
- consume the same data file the cross-language examples consume
  (`ruvec1`, the same format `examples/python/04-rag-grounded` uses),
- and run an exact L2 comparison on the GPU,

without waiting on the M2+ kernel plane to land. The exact same flow plugs
into a GPU rabitq scan once it ships — just swap the kernel.

For the design of that GPU rabitq scan, see
[`../03-rabitq-gpu-design-note/README.md`](../03-rabitq-gpu-design-note/README.md).

## Toolchain

- Rust 1.75+ (tested with 1.95).
- CUDA Toolkit 12.x or 13.x (tested with 13.0). `nvcc` must be on `$PATH`.
- An NVIDIA GPU with compute capability ≥ 7.5 (Turing / Ampere / Ada / Hopper /
  Blackwell). Tested on **NVIDIA RTX 5080 (16 GB, sm_120, driver 580.x,
  CUDA 13.0)**.

If you have CUDA at a non-default location, set `CUDA_PATH` (or `NVCC` to
override just the compiler binary) before `cargo build`.

If you have a CUDA toolkit version other than 13.0, edit `Cargo.toml` and
change the `cuda-13000` cudarc feature to match (`cuda-12080`, `cuda-12090`,
etc.) — or use `cuda-version-from-build-system` to let cudarc autodetect by
calling `nvcc --version`.

## Install + build

```bash
# Verify toolchain.
nvidia-smi
nvcc --version           # NVIDIA Cuda compiler driver, release 12.x or 13.x

# Build.
cd examples/gpu/01-cuda-brute-force
cargo build --release
```

The first build downloads `cudarc` (and its sys-bindings for your CUDA
version) and compiles `src/kernels/l2_search.cu` to PTX via `nvcc`. The PTX
is embedded into the binary at compile time and JIT-compiled to your GPU's
SM at runtime by the CUDA driver — so the same binary runs on any
sm_75-or-later device.

If `nvcc` is unavailable at build time, the build emits a `cargo:warning`
and writes a stub PTX. The Rust binary still compiles, but it will fail at
load time with a clear "couldn't load module" message.

## Run

### Against a synthetic demo snapshot

```bash
# Materialize a 100,000 × 128-d demo snapshot at ./demo-snapshot, then
# run the comparison.
cargo run --release -- ./demo-snapshot --materialize-demo
```

### Against a real ruLake-published snapshot

```bash
# Point the binary at a directory containing table.rulake.json + a
# *.ruvec1 file (e.g. one produced by the existing
# examples/sidecar_daemon.rs or by examples/python/04-rag-grounded
# with --materialize-demo).
cargo run --release -- /path/to/snapshot --k 10
```

### Against the committed cross-language fixture

```bash
# The fixture under examples/nodejs/01-verify-witness/fixtures/
# has 8-d vectors and a known-good witness. The fixtures/ subdir of
# this example carries a copy + a matching tiny ruvec1.
cargo run --release -- ./fixtures --k 5
```

## Expected output

Measured on an RTX 5080 (16 GB), CUDA 13.0, AMD Ryzen-class host, release
build, with `--materialize-demo --demo-n 100000 --demo-dim 128 --k 10`:

```text
[demo] materialized synthetic snapshot at /tmp/rulake-cuda-demo (100000 vectors × 128 dims)
[bundle] verified witness 171e340b86195df4bcfa47e7d9519d649bd617b298173522da4b86a7bf463b65 (data_ref=file:///tmp/rulake-cuda-demo/vectors.ruvec1)
[corpus] loaded 100000 vectors × 128 dims from /tmp/rulake-cuda-demo/vectors.ruvec1
[cpu]   top-10 L2 in    5.046 ms (100000 candidates × 128 dims)
[gpu]   device: NVIDIA GeForce RTX 5080
[gpu]   top-10 L2 in    0.131 ms (kernel) +  5.573 ms (H2D+D2H) =    5.703 ms total

=== summary ===
  provenance_id (witness):  171e340b…3b65
  n × dim:                  100000 × 128
  k:                        10
  speedup (kernel-only):     38.62x  (CPU   5.046 ms vs GPU   0.131 ms)
  speedup (incl. transfer):   0.88x  (CPU   5.046 ms vs GPU   5.703 ms)
  top-10 set agreement:        10/10  (100.0%)

  top-10 (CPU baseline):
      0.  id=     61155  l2_sq=     38.759937
      1.  id=     67270  l2_sq=     38.911812
      …
```

A larger run (`--demo-n 500000 --demo-dim 256 --k 10`) on the same machine:

```text
[cpu]   top-10 L2 in   48.246 ms (500000 candidates × 256 dims)
[gpu]   top-10 L2 in    1.691 ms (kernel) + 26.574 ms (H2D+D2H) =   28.264 ms total
  speedup (kernel-only):     28.53x
  speedup (incl. transfer):   1.71x
  top-10 set agreement:        10/10  (100.0%)
```

Three things to notice:

- **Kernel-only speedup is 30-100×** depending on size. That's the
  apples-to-apples number — for a serving setup that uploads the corpus once
  and reuses it across thousands of queries, this is the relevant metric.
- **End-to-end speedup is 0.9-2×** for a single-query benchmark because the
  full H2D upload of the corpus eats most of the wall time. A real GPU
  vector store amortizes that across many queries — re-run the binary with
  the same `--snapshot-dir` and watch the kernel cost dominate.
- **Top-K agreement is 100%** for non-degenerate corpora. Brute-force L2 is
  exact in both implementations; the only way to disagree is on ties or
  near-ties (see below).

## On float order-of-operations

Both implementations compute `Σ (qᵢ - vᵢ)²` for each candidate, but in
slightly different orders:

- The CPU loop accumulates in a single tight `for` loop in source order.
- The CUDA kernel accumulates with `fmaf` (fused multiply-add) and
  `--use_fast_math`, with up to 4× loop unrolling.

For typical embedding distributions the two paths give bit-identical L2
distances. For pathological inputs (very similar vectors, high dim, near
cancellation) the last few ULPs of the distance can differ, which can swap
the order of two near-tied candidates. The "top-K agreement" metric in the
report counts set-agreement (a tie that swaps positions still counts as a
hit), so this is normally invisible. If you see ⟨k⟩/k less than ~95%,
something has gone wrong upstream.

## Caps + limits

The binary mirrors the upstream `MAX_PULLED_VECTORS = 100,000,000`,
`MAX_PULLED_DIM = 8192`, and `MAX_PULLED_BYTES = 16 GiB` from
`src/backend.rs`. A hostile or accidentally-huge `ruvec1` is rejected at
parse time, before allocation.

`--max-vectors` adds a *demo cap* on top of those: it defaults to
1,000,000 to keep VRAM in budget on a 16 GB card with dim ~128 (≈ 0.5 GB
for vectors + query + distances). Bump it for bigger benchmarks; 16 GB
fits ~ 4M × 1024-d (= 16 GB raw, before alignment) so leave headroom.

## What's worth porting next

The honest version of this example, with a real GPU rabitq scan instead of
brute-force L2, is detailed in
[`../03-rabitq-gpu-design-note/README.md`](../03-rabitq-gpu-design-note/README.md).

## File map

```
01-cuda-brute-force/
├── README.md            (this file)
├── Cargo.toml           (cudarc 0.19, sha3, hex, serde, serde_json)
├── build.rs             (compiles src/kernels/l2_search.cu via nvcc)
├── fixtures/
│   ├── table.rulake.json            (committed, witness-verified)
│   └── vectors.ruvec1               (committed, matching ruvec1 corpus)
└── src/
    ├── main.rs          (CLI: parse args, verify, CPU+GPU compare, report)
    ├── witness.rs       (local copy of compute_witness — byte-exact upstream)
    ├── ruvec1.rs        (ruvec1 parser/writer with the upstream caps)
    └── kernels/
        └── l2_search.cu (CUDA kernel — one thread per candidate)
```
