# ruLake Python SDK — A Deep Introduction

## TL;DR

The ruLake Python SDK is a PyO3 binding that exposes the Rust crate's public surface — `RuLake`, `LocalBackend`, `FsBackend`, `Bundle`, `Consistency`, the `SearchResult` value class, the `CacheStats` / `PerBackendStats` rollups, and a typed exception hierarchy — to a Python audience that lives in NumPy and never wants to see `cargo`. It is sync-first, GIL-releasing on every hot path, and zero-copies query vectors through `numpy::PyReadonlyArray*::as_slice()` so the 1.02× cache-hit tax that ADR-155 commits to survives the language boundary. The wheel is one ABI3 artefact per platform, built by maturin, distributed as `pip install rulake`; the underlying Cargo crate `rulake-py` lives at `python/` as a sibling of the parent crate per ADR-001.

## Introduction

The ruLake Rust crate ships everything the project's headline claim depends on — RaBitQ-quantised vector cache, witness-anchored bundles, federated search across heterogeneous backends, the 1.02× tax over the bare library. None of that is reachable to the audience that is most likely to *want* it. Data engineers, ML platform teams, and RAG application authors live in Python. They have their query embeddings in `np.ndarray[float32]` already, they install dependencies with `pip`, and they expect a library to honour the GIL discipline that lets a thread pool fan out 8–64 concurrent searches per process. Telling them to write Rust to evaluate ruLake is a non-starter; telling them to install a Rust toolchain is barely better.

ADR-002 (`docs/adrs/sdk/ADR-002-python-sdk.md:44`-area) is honest about the constraint. The pitch from `BENCHMARK.md` is "abstraction is free" — at D = 768 and k = 100 over RAG-typical batch sizes (32–256), a list-of-floats round-trip per query dwarfs the actual RaBitQ scan, and any binding that copies the query vector through Python lists destroys the claim before it leaves the FFI boundary. The constraint set is specific. The SDK has to (1) hide Rust so the install is a wheel and nothing else, (2) preserve the 1.02× tax across the FFI hop, (3) speak NumPy because every Python ML user already has their embeddings as `ndarray[float32]`, (4) release the GIL on hot paths so a serving process does not single-thread itself even though the underlying cache is `Send + Sync`, and (5) distribute as wheels rather than as "compile from source" because most Python installs do not have `cargo`.

Why all of this *now*, rather than as a follow-on after the Rust crate matures? Because the Rust surface is already stable enough that an ergonomic binding is not a moving target. The relevant exports from `src/lib.rs` (cited verbatim in ADR-002 §Context: `RuLake`, `LocalBackend`, `FsBackend`, `Bundle`, `Consistency`, `SearchResult`, `CacheStats`, `PerBackendStats`, `RuLakeError`) are all concrete data or structs with fixed method surfaces. The one trait — `BackendAdapter` — is `dyn Trait`, and the ADR is upfront that Python implementations of it are a v2 concern (Open question §1) because making the trait callable from Python without re-acquiring the GIL on every `pull_vectors` call requires a dedicated Python-callback thread the v1 binding does not yet build. v1 ships with `LocalBackend` and `FsBackend` as the reachable backend implementations, which the ADR judges sufficient for cache-warming use cases, and defers Python-implemented backends until a customer arrives with a real one.

A precomputed, witness-anchored representation matters here for a different reason than it does in the data-lake setting. The Python SDK's job is not to add semantic value over the Rust crate; it is to make the Rust crate's existing semantic value reachable from Python without erosion. The witness contract, the consistency knob, the federated-search shape — all of it has to round-trip through PyO3 unchanged. A binding that *added* an extra serialisation step or a per-call allocation would be visible in the benchmark gate (`python/tests/bench_tax.py`, the ADR's §Verification block: `search_one` QPS against a direct Rust binary at n = 100 k, D = 128, k = 10, ratio ≤ 1.10×). The 1.10× budget is wider than the Rust-side 1.02× because of FFI per-call overhead (~1 µs), and a regression past 1.10× is a real degradation. The whole binding exists to be invisible in that comparison.

The economic shape is the same as for any binding that wraps a fast kernel. The kernel takes a query vector and returns a small structured result; the per-call FFI cost is fixed and small relative to the per-call kernel cost. As long as the binding never copies the vector and never allocates more than the result requires, the relative tax stays bounded by the FFI-fixed cost divided by the kernel-variable cost — and that ratio improves with k, with batch size, and with the dim of the embeddings. A 1.02× tax for D = 128 becomes a smaller tax for D = 768. The SDK is honest about not making things worse rather than dishonest about making things faster.

## The decision in detail

ADR-002 makes six coupled decisions; the load-bearing four are the binding library, the buffer protocol, the GIL release, and the wheel shape. The other two (sync-first API, single-inheritance exception classes) are explicit cost-of-doing-business simplifications.

The first is to use **PyO3 + maturin** rather than `ctypes`/`cffi` against a hand-rolled `cdylib`, or a pure-Python rewrite, or an HTTP client against a separate `rvf-server` process. The ADR's §Alternatives block walks each. ctypes/cffi works but does not give automatic `#[pyclass]` → Python class generation, `Drop`-tied-to-`__del__` lifecycle, or NumPy buffer-protocol integration without writing the dance manually each call. A pure-Python rewrite of the cache + router throws away the kernel speed (NumPy hamming over uint64 × n=100k for k=10 is ~30 ms vs ~1 ms in Rust per `BENCHMARK.md`, per the ADR's §B math) and forces a second implementation of the SHAKE-256 witness recipe — one too many. An HTTP client over `rvf-server` is a real alternative for v2 but the in-process pitch (notebook, single-server RAG) cannot tolerate the 1–5 ms RVF-wire round-trip cost, so v1 ships in-process and `rulake.client.HttpRuLake` is reserved for later.

The second is the **buffer protocol**. Every API that takes a vector takes `PyReadonlyArray1<f32>` (single query) or `PyReadonlyArray2<f32>` (batch / collection put). The binding calls `.as_slice()?` (ADR-002 §2; reflected at `python/src/lib.rs:387`-area for `LocalBackend.put_collection`) which returns `&[f32]` *without copying* when the array is C-contiguous and float32. If the array is non-contiguous or the wrong dtype, the binding errors explicitly with a `PyValueError` rather than silently copying — silent copies hide the regression the binding exists to prevent. Result IDs come back as `Vec<PySearchResult>`, a small `#[pyclass]` with `backend: str`, `collection: str`, `id: int`, `score: float`. For top-k of 10–100 the wrapper-object cost is negligible; for high-k workloads (k > ~500) the ADR reserves `search_one_arrays(...)` returning `(np.ndarray[uint64], np.ndarray[float32])` of length k as a v1.5 addition (Open question §3) but holds it back until a real benchmark filing arrives.

The third is **GIL release on every hot path**. PyO3's `py.allow_threads(|| …)` releases the GIL for the duration of the closure. The binding wraps every call into the underlying `RuLake`/`VectorCache` that does work — search, prime, save, warm, publish, refresh — in `allow_threads`. The pattern is: borrow or copy out of Python land *before* `allow_threads`; release the GIL for the actual work; re-enter Python land to convert results. The closures cannot touch any `Py<…>` pointer or PyO3 object, so the binding copies strings and small parameters into `Send` locals before entering the closure. The SDK's tests at `python/tests/` exercise the threadpool pattern (`concurrent.futures.ThreadPoolExecutor` over the GIL-releasing sync API) as the working concurrency story, matching the same pattern `psycopg2`, `lxml`, and every other GIL-releasing binding ships.

The fourth is **ABI3 wheels distributed via PyPI**. `pyo3 = { version = "0.22", features = ["abi3-py39", "extension-module"] }` (`python/Cargo.toml:35`) is the load-bearing feature flag: it tells PyO3 to compile against the stable Python C API as of 3.9, so a single wheel works on CPython 3.9–3.13. Without ABI3 the matrix would be five wheels per platform per release; with it, one. The wheel build matrix in CI covers Linux x86_64 / aarch64 (manylinux_2_28), macOS arm64 / x86_64, Windows x86_64. An sdist (`maturin sdist`) covers exotic-platform users, with the `[tool.maturin].include` block at `python/pyproject.toml:77` carrying the parent `src/`, `Cargo.toml`, `Cargo.lock`, and the vendored rabitq sources so `pip install <sdist>` in an air-gapped environment Just Works.

| trade-off | what got picked | what got rejected | why |
|---|---|---|---|
| binding library | PyO3 + maturin | ctypes/cffi, pyo3-asyncio-only, hand-rolled JSON-RPC | PyO3 has the only mature buffer protocol + macro surface for `#[pyclass]`. |
| API style | sync-first, GIL-releasing | async-first via pyo3-asyncio + tokio | The Rust crate is sync; an async façade adds runtime hop with no concurrency win. |
| wheel matrix | one ABI3 wheel per platform | five wheels per platform (3.9–3.13 each) | ABI3 collapses the matrix; the binding code uses no >3.9 syntax. |
| exceptions | single-inheritance from `RuLakeError` | multi-inheritance from `LookupError`/`ValueError`/`OSError` | PyO3 0.22's `create_exception!` caches classes on the Rust side; monkey-patching from `__init__.py` does not change what the binding raises. v1.5 reopens. |
| Python-implemented backends | not exposed | `BackendAdapter` callable from Python | Would force the parallel scan threads to re-acquire the GIL on every `pull_vectors`; v2 ships a dedicated callback thread. |

## Capabilities

The binding's capability surface is what the Rust crate exposes, in Python shape. `RuLake(rerank_factor, rotation_seed)` constructs a lake; `with_consistency(Consistency.eventual(ttl_ms=5000))` swaps the staleness knob; `with_max_cache_entries(n)` caps the LRU; `register_backend(be)` adds a `LocalBackend` or `FsBackend`; `search_one(backend, collection, query, k)` runs a single-route query; `search_federated(targets, query, k)` runs a multi-route fan-out (where `targets` is a list of `(backend, collection)` pairs); `search_batch(backend, collection, queries, dim, k)` amortises FFI cost over a batch of queries; `cache_stats()` and `cache_stats_by_backend()` expose the rollups; `cache_witness_of(backend, collection)` returns the SHAKE-256(32) hex witness or `None`; `invalidate_cache(backend, collection)` drops the entry. `Bundle(data_ref, dim, rotation_seed, rerank_factor, generation)` constructs a witness-anchored sidecar and exposes `verify_witness()`, `to_json()`, `Bundle.from_json(s)`, `write_to_dir(dir)`, and `Bundle.read_from_dir(dir)`. The full surface is documented in the binding source (`python/src/lib.rs`) with PyO3 docstrings and re-exported through `python/python_src/rulake/__init__.py`.

The exception hierarchy is rooted at `rulake.RuLakeError(Exception)`. Typed children — `BackendNotFoundError`, `CollectionNotFoundError`, `DimensionMismatchError`, `InvalidParameterError`, `BackendError` — inherit from it and map one-for-one off the Rust `RuLakeError` variants (`python/src/lib.rs:62`-area; `map_err` does the dispatch). `except RuLakeError:` is the catch-all idiom; `except DimensionMismatchError:` discriminates. The ADR is honest in §6 that single inheritance is a v1 simplification — the original draft proposed multi-inheritance from stdlib bases (`LookupError`, `ValueError`, `OSError`) so idiomatic `except KeyError:` / `except ValueError:` would catch our errors without learning ruLake-specific names, but PyO3 0.22's `create_exception!` macro caches the exception class on the Rust side at module load and monkey-patching from `__init__.py` cannot change what the binding raises. Building a multi-inherited class via `PyType::new_with_qualname` from inside the Rust binding is doable but adds non-trivial complexity to every variant; v1.5 reopens if a real user files an issue.

A worked example. Suppose you have 10,000 documents whose embeddings live in a NumPy `ndarray[float32]` of shape `(10_000, 768)`, and a query embedding `q` of shape `(768,)`. The Python flow is:

```python
import numpy as np, rulake
lake = rulake.RuLake(rerank_factor=20, rotation_seed=42).with_consistency(rulake.Consistency.eventual(ttl_ms=5000))
be = rulake.LocalBackend("local")
ids = np.arange(10_000, dtype=np.uint64)
vs  = np.random.randn(10_000, 768).astype(np.float32)
be.put_collection("docs", ids, vs)
lake.register_backend(be)
hits = lake.search_one("local", "docs", q, k=10)
for h in hits:
    print(h.backend, h.collection, h.id, h.score)
```

Behind the scenes, `put_collection` zero-copies `vs` through `PyReadonlyArray2.as_slice()` (`python/src/lib.rs:387`-area), validates contiguity and shape, and hands the underlying `&[f32]` to `LocalBackend::put_collection`. `search_one` zero-copies `q` through `PyReadonlyArray1.as_slice()`, releases the GIL with `py.allow_threads`, calls `RuLake::search_one`, and re-enters Python land to convert the `Vec<SearchResult>` into a `list[SearchResult]`. A `ThreadPoolExecutor` over 16 workers can fan out 16 concurrent `search_one` calls on the same `lake` because the GIL is released around the scan and `RuLake` is `Send + Sync` underneath; the headline number from `BENCHMARK.md` (8 threads × 50 queries holds correctness and hit-rate) is the upper bound the SDK exposes through this pattern.

## Trust & correctness contract — no abstraction tax

The trust contract for ADR-002 is "no abstraction tax + zero-copy where it matters". Five mechanisms enforce it.

The first is the **buffer-protocol borrow**. Every vector parameter takes a `PyReadonlyArray1<f32>` or `PyReadonlyArray2<f32>` and calls `.as_slice()?` — see `python/src/lib.rs:387` for the canonical example in `LocalBackend.put_collection`. A non-contiguous or wrong-dtype array produces an immediate `PyValueError` rather than a silent copy. Errors are visible; silent regressions are not. The binding does not own a `Vec<f32>` for any vector that crosses the FFI boundary in the common case.

The second is the **GIL release boundary**. Every method that does work — search, prime, save, warm, publish, refresh — wraps its core call in `py.allow_threads(|| …)`. The pattern is mechanical: borrow Python state out, release, do work, re-enter to convert. `Bundle.write_to_dir` (`python/src/lib.rs:325`), `Bundle.read_from_dir` (`python/src/lib.rs:331`), and the search methods all follow it. A regression here would surface as serving-process single-threading under load — visible in the integration tests at `python/tests/` and in the ADR's bench gate.

The third is the **dimension-mismatch check** at the FFI boundary. `LocalBackend.put_collection` (`python/src/lib.rs:380`-area) walks the `vectors.shape()` against `len(ids)`, returns `PyValueError("vectors must be 2-D")` for the wrong rank, and `PyValueError(format!("len(ids)={} != vectors.shape[0]={}", ids_slice.len(), n))` for shape mismatches. The dim is taken from `shape[1]` and threaded through to the Rust call, so the kernel sees a single source of truth.

The fourth is the **bundle contract preservation**. `Bundle` (`python/src/lib.rs:259`-area) wraps the Rust `RuLakeBundle` directly. The `__new__` constructor accepts `generation` as either `int` (Num variant) or `str` (Opaque variant) and dispatches at `python/src/lib.rs:278`-area; the generation tag distinction that the witness recipe at `src/bundle.rs:362` depends on is preserved. `verify_witness()` is a passthrough (`python/src/lib.rs:312`); `to_json()` and `from_json()` are passthroughs that propagate the size caps from `src/bundle.rs:218`-area (64 KiB body, 4 KiB per field, 128-char witness) — the DoS hardening lives in the Rust crate and the binding cannot bypass it.

The fifth is the **error-mapping fidelity**. `map_err` at `python/src/lib.rs:62` dispatches each `RuLakeError` variant to the typed Python class one-for-one. A Python user catching `BackendNotFoundError` sees the same condition the Rust caller would see catching `RuLakeError::UnknownBackend`. The ADR's §6 commits to this mapping and the binding tests at `python/tests/` exercise the round-trip.

The bench gate (`python/tests/bench_tax.py`) measures single-thread `search_one` QPS against a direct Rust binary at n = 100 k, D = 128, k = 10 and asserts the ratio stays ≤ 1.10×. The 1.10× budget includes ~1 µs of FFI per-call overhead on top of the Rust 1.02×; a regression past 1.10× is a real degradation that would block a release.

## Reference implementation status

The crate `rulake-py` v2.2.0 lives at `python/`. The PyPI package is `rulake`; the native cdylib is `_rulake`; the pure-Python wrapper at `python/python_src/rulake/__init__.py` re-exports from it. As of v2.2.0 (commit `2fb1730 Implement Python (PyO3) and Node.js (napi-rs) SDKs`, called out in ADR-002 §Status), the surface that is shipping:

- `RuLake` with `with_consistency`, `with_max_cache_entries`, `register_backend`, `backend_ids`, `cache_stats`, `cache_stats_by_backend`, `cache_entry_count`, `cache_witness_of`, `invalidate_cache`, `search_one`, `search_federated`, `search_batch`, `publish_bundle`, `refresh_from_bundle_dir`, `save_cache_to_dir`, `warm_from_dir` (per `python/src/lib.rs`).
- `LocalBackend(id)` with `put_collection`, `append` (`python/src/lib.rs:361`-area).
- `FsBackend(id, root)` with `register`, `write`.
- `Bundle(data_ref, dim, rotation_seed, rerank_factor, generation)` with the full method surface (`python/src/lib.rs:259`-area).
- `Consistency.fresh()`, `Consistency.eventual(ttl_ms)`, `Consistency.frozen()` (`python/src/lib.rs:82`-area).
- `SearchResult` value class, `CacheStats` and `PerBackendStats` rollup classes with `hit_rate()` and `avg_prime_ms()` helpers.
- The exception hierarchy: `RuLakeError`, `BackendNotFoundError`, `CollectionNotFoundError`, `DimensionMismatchError`, `InvalidParameterError`, `BackendError`, all rooted at `RuLakeError` (`python/src/lib.rs:47`-area; re-exported at `python/python_src/rulake/__init__.py:59`-area).
- ABI3 wheels via `pyo3 = { version = "0.22", features = ["abi3-py39", "extension-module"] }` and `numpy = "0.22"` for the buffer protocol. `[lints.rust]` block at `python/Cargo.toml:50` quietens the `cfg(gil-refs)` macro hygiene noise from PyO3 0.22 — explicitly slated for removal when the crate moves to PyO3 0.23+.

What v2.2.0 does *not* ship, per ADR-002 §Open questions:

- Python-implemented `BackendAdapter` (Open question §1; v2 — ships a `PyBackendAdapter` Rust struct that owns a `tokio::sync::mpsc` channel to a dedicated Python-callback thread so the parallel-fan-out scan threads never touch the GIL).
- Async API via `pyo3-asyncio` (Open question §2; v2 — decided once the Rust crate gains async paths, almost certainly when M3's `BigQueryBackend` lands).
- `search_one_arrays(...)` for high-k array-returning search (Open question §3; v1.5 — punted until a user files a real benchmark).
- `__reduce__`-based pickling for `RuLake` itself (Open question §4 — probably never; `RuLake` owns Rust state behind an `Arc` that does not round-trip through bytes; `Bundle` and `SearchResult` are picklable trivially).
- Free-threaded CPython (PEP 703, 3.13t) targeting (Open question §5; reopened when no-GIL builds mature).
- HTTP client `rulake.client.HttpRuLake` (Alternatives §C; reserved for v2 if a customer asks).

## Composition with the rest of ruLake

The Python SDK is a thin shim over the public Rust surface, but the consequence of the shim being thin is that everything the Rust crate composes with is reachable from Python without per-feature plumbing.

**The cache and federation primitives are reachable.** `lake.search_federated(targets, query, k)` from Python reaches `RuLake::search_federated` at `src/lake.rs:521`, which fans out across registered backends in parallel via rayon and applies the adaptive per-shard rerank. A Python caller assembling a 10-shard federated query pays one zero-copy borrow on the query vector and one `Vec` allocation on the result — the parallel scan happens in pure Rust with the GIL released. The bench gate confirms this stays inside the 1.10× tax budget.

**The bundle and witness contract is reachable.** `Bundle(...)` from Python constructs a `RuLakeBundle` via `RuLakeBundle::new` (`src/bundle.rs:166`), which calls `compute_witness` (`src/bundle.rs:362`) to produce the SHAKE-256(32) hex witness. Two Python processes constructing the same bundle (same `data_ref`, dim, seed, rerank, generation) get byte-identical witnesses, because the witness recipe is purely a function of bytes. `Bundle.from_json(s)` and `Bundle.read_from_dir(dir)` propagate the witness-fail-closed posture from `src/bundle.rs:340`-area — a Python caller reading a tampered bundle gets a `RuLakeError`, never silently bad data.

**The MCP server, IPFS backend, and substrate scaffolds compose underneath.** ADR-004's MCP server consumes the same `RuLake` instance the Python SDK constructs. ADR-005's IPFS backend produces bundles that `Bundle.from_json` will accept; cross-deployment cache via CID works the same way regardless of which language constructed the bundle. The substrate ADRs (007 / 008 / 156) define `memory_class` on the bundle (e.g. `"genomic"` for rvDNA, `"quantum-simulation"` for ruQu); `Bundle.with_memory_class(klass)` from Python tags it. The Python SDK is not the substrate layer, but it is a working consumer of the substrate's wire format.

The shape ADR-002 commits to is the same shape ADR-003 commits to for Node and ADR-004 commits to for MCP. Common decisions — zero-copy buffers, error map, sync-first vs async-first, "no abstraction tax" budget — are made jointly across the three ADRs. The Python SDK is not idiosyncratic; it is the per-language instantiation of a shared discipline.

## Open questions

Five honest unknowns track in the ADR. Python-implemented `BackendAdapter` is the largest — a real customer with a Python-only HTTP backend would surface the design pressure for the v2 dedicated-callback-thread approach, but no such customer has filed yet. Async API timing depends on when the Rust crate gains async paths (likely when M3's `BigQueryBackend` lands); shipping `pyo3-asyncio` over a sync crate is worse latency for no concurrency win. High-k array-returning search (`search_one_arrays`) waits for a real benchmark filing past k > ~500 where per-`PySearchResult` allocation cost stops being negligible. Pickling for `RuLake` itself is almost certainly *no* (Arc'd Rust state) but `Bundle` and `SearchResult` should be trivially picklable; ADR-002 §Open question §4 documents the gap. Free-threaded CPython (PEP 703, 3.13t) is a future wheel target, not a v1 question — `py.allow_threads` becomes a no-op under no-GIL but the binding code is correct under it. Each is honest about being unresolved, and none block v2.2.0 from shipping.

## References

- ADR-002: `/home/ruvultra/projects/RuLake/docs/adrs/sdk/ADR-002-python-sdk.md`
- Crate manifest: `/home/ruvultra/projects/RuLake/python/Cargo.toml`
- Build backend config: `/home/ruvultra/projects/RuLake/python/pyproject.toml`
- Binding source: `/home/ruvultra/projects/RuLake/python/src/lib.rs`
  - Exception hierarchy + `map_err`: `python/src/lib.rs:47`, `:62`
  - `Consistency` factory: `python/src/lib.rs:82`
  - `SearchResult` / `CacheStats` / `PerBackendStats`: `python/src/lib.rs:124`, `:167`, `:225`
  - `Bundle` (witness-anchored): `python/src/lib.rs:259`
  - `LocalBackend.put_collection` (zero-copy contract): `python/src/lib.rs:380`
- Pure-Python wrapper: `/home/ruvultra/projects/RuLake/python/python_src/rulake/__init__.py`
- Tests: `/home/ruvultra/projects/RuLake/python/tests/`
- Sibling-crate discipline (no workspace, submodule-aware sdist): ADR-001 (`docs/adrs/ADR-001-standalone-repo-strategy.md`)
- Public Rust surface the binding wraps: `src/lib.rs:53`-area
- Bundle witness recipe (preserved across the FFI): `src/bundle.rs:166` (`RuLakeBundle::new`), `src/bundle.rs:362` (`compute_witness`), `src/bundle.rs:340`-area (witness-fail-closed in `read_from_dir`)
- DoS-hardening size caps the binding inherits: `src/bundle.rs:218`-area, `src/backend.rs:60`-area
- Federation primitive reachable from Python: `src/lake.rs:521` (`search_federated`)
- Companion SDK ADR for Node (joint decisions): `docs/adrs/sdk/ADR-003-nodejs-typescript-sdk.md`
- Companion ADR for the MCP server (consumes the same `RuLake`): `docs/adrs/sdk/ADR-004-rulake-mcp-server.md`
- PyO3 / maturin / NumPy crate references — the ADR cites these as opaque versioned dependencies (`pyo3 = "0.22"`, `numpy = "0.22"`, `maturin >= 1.5, < 2.0`). The respective project READMEs (PyO3 user guide, maturin docs, numpy-rs README) are the upstream documentation; ADR-002 does not pin URLs, treating them as opaque versioned identifiers.
