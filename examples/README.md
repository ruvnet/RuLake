# Examples

Runnable demos that show how to use ruLake from the supported languages and
how to interoperate with it across language boundaries via the bundle protocol.

The bundle protocol — `table.rulake.json` sidecar + SHAKE-256 witness — is the
language-portable primitive. Rust ruLake produces sidecars and snapshot
directories; clients in any language read them, recompute the witness to verify
content-addressed integrity, and consume the data with full provenance. That's
the cross-language story documented in [ADR-155](../docs/adrs/ADR-155-rulake-datalake-layer.md)
and explored end-to-end in this directory.

## Layout

```
examples/
├── README.md                       # this file
├── sidecar_daemon.rs               # Rust — publisher + reader + coherence loop
├── warm_restart.rs                 # Rust — save → ship → warm-restart cycle
├── python/                         # 4 modules, no Rust bindings required
├── nodejs/                         # 4 modules, TypeScript + ESM, no Rust bindings
├── wasm/                           # 3 modules — browser, Node, WASI
└── gpu/                            # 3 modules — CUDA, wgpu portable, design note
```

Run the existing Rust examples first; their output (real `table.rulake.json`
sidecars under `/tmp/rulake-*-demo-*/`) is what the cross-language verifiers
in `python/`, `nodejs/`, and `wasm/` consume.

```bash
cargo run --release --example sidecar_daemon
cargo run --release --example warm_restart
```

## Cross-language ground truth

Every cross-language example is grounded in two facts from `src/bundle.rs`:

1. The bundle JSON shape (`format_version`, `data_ref`, `dim`, `rotation_seed`,
   `rerank_factor`, `generation`, `rvf_witness`, optional `pii_policy` /
   `lineage_id` / `memory_class`).
2. The witness algorithm — SHAKE-256(32) over a length-prefixed,
   domain-separated byte stream including a `Generation`-variant tag byte
   (the audit-driven fix from 2026-04-23 that prevents the
   `Num(7)` vs `Opaque("\x07\0…")` collision).

All three swarms verified their implementations against a real Rust-produced
sidecar and reported byte-exact agreement. The Rust-produced witness
`31c0865f078d9d646edaf0fe339d5d8c20a04bac9e95571fae91035306c2b584`
(produced by `sidecar_daemon` with `generation=Num(2)` and the publisher's
canonical fields) is reproduced identically by Python, Node, browser WASM,
Node WASM, and the WASI CLI.

## Python — `python/`

Four modules, each `pip install -e`-able, each with its own venv-friendly
pyproject.toml.

| Module | What it shows |
|--------|---------------|
| [`01-verify-witness/`](python/01-verify-witness/) | Pure-Python SHAKE-256 witness recompute + safe bundle parser with DoS caps. CLI + reusable module + pytest including the collision regression. |
| [`02-bundle-server/`](python/02-bundle-server/) | FastAPI server that watches a directory for ruLake bundles and serves them at `/bundles/{key}/table.rulake.json` with witness-as-ETag. |
| [`03-subprocess-wrapper/`](python/03-subprocess-wrapper/) | Python class wrapping `cargo run --release --bin rulake-demo`. Parses the demo's stdout into structured benchmark reports. |
| [`04-rag-grounded/`](python/04-rag-grounded/) | Real RAG pipeline: verifies witness, reads `ruvec1` data file behind the bundle, brute-force L2 search, returns hits annotated with `provenance_id = witness`. |

63 tests total (62 passing, 1 skipped behind `RULAKE_RUN_END_TO_END=1`).

## Node.js + TypeScript — `nodejs/`

Four modules, each with its own `package.json`, ESM + TypeScript strict.

| Module | What it shows |
|--------|---------------|
| [`01-verify-witness/`](nodejs/01-verify-witness/) | TS witness recompute using `@noble/hashes/sha3`. CLI + reusable module + vitest including the collision regression. |
| [`02-bundle-publisher/`](nodejs/02-bundle-publisher/) | Express server with `chokidar` filesystem watcher that publishes bundles via `GET /bundles/:key/table.rulake.json` and `/witness`. Verifies on read; refuses to serve poisoned bundles. |
| [`03-subprocess-wrapper/`](nodejs/03-subprocess-wrapper/) | TS class wrapping `rulake-demo`. Defensive stdout parser + structured `BenchmarkReport`. |
| [`04-mcp-tool/`](nodejs/04-mcp-tool/) | MCP server (`@modelcontextprotocol/sdk`) exposing `rulake_search`, `rulake_verify_witness`, `rulake_bundle_info` as agent-callable tools. The agentic example. |

43/43 tests passing across all four modules.

## Rust WASM — `wasm/`

Three packages targeting different WASM hosts. Each is a standalone Cargo
package (empty `[workspace]` section) so the wasm32 builds don't pull in the
host ruLake crate's `std::sync` / `rayon` deps.

| Module | Target | What it shows |
|--------|--------|---------------|
| [`01-witness-verifier-browser/`](wasm/01-witness-verifier-browser/) | `wasm32-unknown-unknown` (web) | Drag-drop a sidecar in the browser, get PASS/FAIL with the recomputed witness. ~127 KB compiled. |
| [`02-witness-verifier-nodejs/`](wasm/02-witness-verifier-nodejs/) | `wasm32-unknown-unknown` (nodejs) | Same Rust source, packaged for `require()`. Runnable Node tests cover the collision regression and a length-prefix regression. ~127 KB. |
| [`03-bundle-toolkit-cli/`](wasm/03-bundle-toolkit-cli/) | `wasm32-wasip1` | A `clap`-based CLI runnable in any WASI host (`wasmtime`, `wasmer`). Subcommands: `verify`, `dump`, `witness`. ~315 KB. |

Build prerequisites: `wasm-pack` (auto-installable via `cargo install wasm-pack
--locked`), `wasm32-unknown-unknown` and `wasm32-wasip1` rustup targets.
Optional: `wasm-opt` (binaryen) for ~30% size reduction. Each example's
`build.sh` `unset`s `RUSTFLAGS` first because some host setups export
`-C link-arg=-fuse-ld=mold`, which the wasm linker rejects.

## Common idioms across the swarm

- **Witness as cache key** — every example treats the SHAKE-256 hex string as
  the canonical content-address. Same witness → same compressed bytes →
  same retrieval results. This is what makes the bundle protocol
  language-agnostic.
- **DoS caps enforced everywhere** — bundle ≤ 64 KiB, fields ≤ 4 KiB, witness
  exactly 64 hex chars. Any parser that doesn't enforce these is a bug.
- **Refuse `format_version > 2`** — current format is 2 (post-2026-04-23 audit
  with the `Generation` tag byte). Older versions are not forward-compatible.
- **Tests pin the collision regression** — `Num(7)` and `Opaque("\x07\0\0\0\0\0\0\0")`
  must produce different witnesses. If your implementation gets the same hex
  for both, the tag byte is missing.

## GPU / CUDA — `gpu/`

Three packages. Honest framing: ruLake's RaBitQ compressed-scan kernel
does NOT have a GPU port today (ADR-157 is "Proposed — scaffolding only"),
so these examples take the "verify the witness, then brute-force-L2 the
floats" path, which is real and useful — just not the M2+ kernel plane.
Module 03 is the bridge: a design note for what the rabitq GPU port
would look like.

| Module | Stack | What it shows |
|--------|-------|---------------|
| [`01-cuda-brute-force/`](gpu/01-cuda-brute-force/) | CUDA via cudarc, nvcc-compiled PTX | Witness-verified brute-force L2. Measured 38× kernel-only speedup vs CPU at 100k×128 on RTX 5080. 4/4 tests pass. |
| [`02-wgpu-portable/`](gpu/02-wgpu-portable/) | wgpu / WGSL — runs on Vulkan/Metal/DX12 | Same shape, no CUDA toolkit needed. ~21× kernel-only speedup. Top-K agreement with CPU = 100% (exact L2). 1/1 test passes. |
| [`03-rabitq-gpu-design-note/`](gpu/03-rabitq-gpu-design-note/) | Markdown only | Design note for the missing rabitq GPU port: per-candidate `__popc()` kernel, AoS↔SoA layout, `VectorKernel` trait integration, recall guarantees, witness compatibility, 7-step implementation checklist. |

## What's NOT in here (and why)

- **No pyo3 / napi-rs / wasm-bindgen bindings to the full ruLake crate.**
  Bindings are weeks of work and a real engineering project — see the
  proposed-status [`docs/adrs/sdk/ADR-002`](../docs/adrs/sdk/ADR-002-python-sdk.md)
  and [`ADR-003`](../docs/adrs/sdk/ADR-003-nodejs-typescript-sdk.md) for the
  shape those would take. The bundle protocol gives you a lighter
  cross-language story that ships today.
- **No production cloud-backend examples** (Parquet, BigQuery, Iceberg, Delta).
  Those backends are M2+ roadmap per [ADR-155 §M2](../docs/adrs/ADR-155-rulake-datalake-layer.md).
  The `BackendAdapter` trait is 4 methods; building a real adapter is real
  domain work, not example-sized.
- **No GPU port of the rabitq compressed scan.** ADR-157 is scaffolding-only;
  the GPU examples in `gpu/` use brute-force L2 over uncompressed floats,
  not the rabitq 1-bit popcount path. See [`gpu/03-rabitq-gpu-design-note/`](gpu/03-rabitq-gpu-design-note/)
  for the bridge.

For deeper context on what ruLake does and doesn't do today, read
[`../docs/review/`](../docs/review/) (capability / performance / security
review) and [`../docs/research/`](../docs/research/) (vertical applications:
agentic, AI/ML, edge, exotic).
