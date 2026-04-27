# ADR-002: Python SDK — PyO3 binding, NumPy zero-copy, ABI3 wheels

## Status

**Accepted (2026-04-25 → v2.2.0 as of 2026-04-26)** — `python/` is
shipping. Crate name `rulake-py` (cdylib `_rulake`); Python source
under `python/python_src/rulake/` per maturin's prescribed layout.
v2.2 surface includes `RuLake`, `LocalBackend`, `FsBackend`,
`Bundle`, `Consistency`, full search variants, and the exception
hierarchy from §4. NumPy zero-copy on input vectors (PyO3 `numpy =
"0.22"`); ABI3 wheels via `pyo3 = { version = "0.22", features =
["abi3-py39"] }`. Companion to [ADR-003](./ADR-003-nodejs-typescript-sdk.md);
both SDKs landed in commit `2fb1730 Implement Python (PyO3) and
Node.js (napi-rs) SDKs`.

**Originally proposed (2026-04-25)** — drafted before the first PR
opened so the FFI / async / packaging questions wouldn't relitigate
at code review.

## Date

2026-04-25

## Authors

ruv.io · RuVector engineering. Drafted alongside the standalone-repo
strategy (ADR-001) so the Python crate that this ADR introduces inherits
the same submodule-and-no-root-workspace discipline.

## Relates To

- [ADR-001](../ADR-001-standalone-repo-strategy.md) — submodule pin,
  no root workspace, concrete dep versions. The Python crate sits as
  a sibling Cargo package and follows the same rules.
- [ADR-155](../ADR-155-rulake-datalake-layer.md) — public Rust surface
  the binding wraps. The 1.02× cache-hit tax is the budget; the SDK
  must not eat it.
- [ADR-003](./ADR-003-nodejs-typescript-sdk.md) — the parallel SDK
  decision for Node.js/TypeScript. Common shape decisions are made
  *jointly* with that ADR (zero-copy buffers, async story, error map).

---

## Context

The Rust crate `rulake` (this repo) ships the cache + router +
bundle protocol. The audience that buys "vector cache in front of my
existing lake" is overwhelmingly Python-first: data engineers, ML
platform teams, RAG application authors. Telling them to write Rust
to evaluate ruLake is a non-starter.

We need a Python SDK that:

1. **Hides Rust**, so a Python team installs `pip install rulake`,
   imports it, and runs the demo on their own Parquet/BigQuery data.
2. **Preserves the 1.02× cache-hit tax**. The whole pitch from
   `BENCHMARK.md` is "abstraction is free". A binding that copies the
   query vector or the result IDs through Python lists destroys that
   claim — at D = 768 and `k = 100` over batch sizes typical for RAG
   (32–256), a list-of-floats round-trip per query dwarfs the actual
   RaBitQ scan.
3. **Speaks NumPy**. Every Python ML user already has their query
   embeddings in `np.ndarray[float32]`. Demanding `list[float]` is
   user-hostile and slow.
4. **Releases the GIL on hot paths**. RAG servers fan out 8–64
   concurrent searches per process; if every `search()` call holds the
   GIL for the duration of the cache scan, the serving process
   single-threads itself even though `RuLake` is `Send + Sync` and the
   underlying cache uses Arc-drop-lock for 8–12× concurrent lift
   (capability #5 in the README).
5. **Distributes as wheels**, not as "compile from source". Most
   Python installs do not have a Rust toolchain; the first-time
   experience must be `pip install` and nothing else.

The Rust API surface from `src/lib.rs` that v1 must expose:

```rust
pub use backend::{BackendAdapter, BackendId, CollectionId, LocalBackend, PulledBatch};
pub use bundle::{Generation, RuLakeBundle};
pub use cache::{CacheStats, PerBackendStats, VectorCache, Consistency};
pub use error::{Result, RuLakeError};
pub use fs_backend::FsBackend;
pub use lake::{RefreshResult, RuLake, SearchResult};
```

`BackendAdapter` is a `dyn Trait` — Python implementations of it are a
v2 question (see Open questions §1). Everything else is concrete data
or a struct with a fixed method surface and is straightforward to wrap.

## Decision

We ship a Python SDK as a **second Cargo crate** in this repo (planned
path: `python/`) that uses **PyO3 + maturin** to expose the public Rust
surface, with **NumPy zero-copy** for vectors, **GIL release** on every
search/prime path, **ABI3 wheels** for forward compatibility, and a
**sync-first** API in v1 with async wrappers in v2.

The crate name on PyPI is `rulake` (matches the Rust crate
name on crates.io for discoverability). The import name is `rulake`
(short, idiomatic, matches the product name).

```python
import numpy as np
import rulake

lake = rulake.RuLake(rerank_factor=20, rotation_seed=42).with_consistency_eventual(ttl_ms=5000)
backend = rulake.LocalBackend("local")
backend.put_collection("docs", ids=np.arange(10_000, dtype=np.uint64), vectors=embeddings)  # (10_000, 768) float32
lake.register_backend(backend)

q = np.random.randn(768).astype(np.float32)
hits = lake.search_one("local", "docs", q, k=10)
for h in hits:
    print(h.backend, h.collection, h.id, h.score)
```

### 1. Bindings via PyO3 + maturin

PyO3 is the only Rust↔Python binding library that:

- supports stable-ABI3 wheels (one `.whl` per platform that works for
  CPython 3.9+ — critical to keep the wheel matrix small),
- has first-class NumPy support via the `numpy` crate (`PyArray1<f32>`
  with `.as_slice()?` zero-copy borrow),
- supports `py.allow_threads(|| …)` for GIL release, and
- ships maturin, a build backend that integrates cleanly with PEP 517
  (`pip install .` Just Works) and produces manylinux wheels via the
  `manylinux2014` / `manylinux_2_28` images.

The `python/Cargo.toml` shape:

```toml
[package]
name = "rulake-py"
version = "2.2.0"
edition = "2021"
publish = false  # built and shipped by maturin, not crates.io

[lib]
name = "rulake"
crate-type = ["cdylib"]  # Python extension module

[dependencies]
rulake = { path = ".." }     # the cache+router crate (this repo)
pyo3            = { version = "0.22", features = ["abi3-py39", "extension-module"] }
numpy           = "0.22"               # PyArray + ndarray interop
```

`abi3-py39` is the load-bearing feature flag: it tells PyO3 to compile
against the stable Python C API as of 3.9, so a single wheel works on
CPython 3.9, 3.10, 3.11, 3.12, 3.13. Without ABI3 we'd ship five wheels
per platform per release; with it we ship one.

`pyproject.toml` lives at `python/pyproject.toml`:

```toml
[build-system]
requires = ["maturin>=1.5,<2.0"]
build-backend = "maturin"

[project]
name = "rulake"
version = "2.2.0"
requires-python = ">=3.9"
license = { text = "MIT OR Apache-2.0" }
dependencies = [
    "numpy>=1.21",
]

[tool.maturin]
features = ["pyo3/extension-module"]
module-name = "rulake._rulake"     # native module, re-exported from rulake/__init__.py
python-source = "python_src"        # pure-Python wrapper: type stubs, docstrings, helpers
include = [
    # The submodule is required at sdist time so `pip install <sdist>` works
    # in air-gapped environments. Without this, sdists are useless because
    # the path = "../" dep walks into vendor/ruvector.
    { path = "../vendor/ruvector/crates/ruvector-rabitq/**/*", format = "sdist" },
    { path = "../src/**/*.rs",                                  format = "sdist" },
    { path = "../Cargo.toml",                                   format = "sdist" },
]
```

The `python_src/` layout gives us a thin pure-Python wrapper:

```text
python/
├── Cargo.toml
├── pyproject.toml
├── src/
│   └── lib.rs           # PyO3 #[pymodule] entry, ~600 LOC
├── python_src/
│   └── rulake/
│       ├── __init__.py  # re-export from _rulake, attach docstrings
│       ├── py.typed     # PEP 561 marker — type checkers honor stubs
│       └── _rulake.pyi  # type stubs (manually written, drift-checked in CI)
└── tests/
    └── test_smoke.py
```

The pure-Python layer is intentionally thin. It exists for:

- type stubs that mypy / pyright / IDEs can read,
- docstrings (PyO3 docstrings on classes work but are cumbersome for
  long-form docs; we put the user-facing prose in `__init__.py`),
- convenience helpers that don't deserve to live in Rust
  (`from_parquet(path)` style, async wrappers in v2).

### 2. Zero-copy NumPy for vectors

Every API that takes a vector takes `PyReadonlyArray1<f32>` (single
query) or `PyReadonlyArray2<f32>` (batch / collection put). The
binding calls `.as_slice()?` which returns `&[f32]` *without copying*
when the array is C-contiguous and float32 — the common case for any
embedding produced by `sentence-transformers`, `openai`, `cohere`, etc.

If the array is non-contiguous or the wrong dtype, we **error
explicitly** rather than silently copying. Silent copies hide the
performance regression that this ADR exists to prevent.

```rust
#[pymethods]
impl PyRuLake {
    fn search_one<'py>(
        &self,
        py: Python<'py>,
        backend: &str,
        collection: &str,
        query: PyReadonlyArray1<'py, f32>,
        k: usize,
    ) -> PyResult<Vec<PySearchResult>> {
        let q: &[f32] = query.as_slice()
            .map_err(|_| PyValueError::new_err(
                "query must be a contiguous float32 ndarray (got non-contiguous or wrong dtype)"
            ))?;

        // GIL released around the actual scan — see decision §3.
        let hits = py.allow_threads(|| {
            self.inner.search_one(backend, collection, q, k)
        }).map_err(map_err)?;

        Ok(hits.into_iter().map(PySearchResult::from).collect())
    }
}
```

Result IDs come back as `Vec<PySearchResult>` — a small `#[pyclass]`
with `backend: str`, `collection: str`, `id: int`, `score: float`.
For top-k of 10–100 the wrapper-object cost is negligible. For
high-k workloads (k > 1000) we add a `search_one_arrays(...)` variant
in v1.5 that returns `(np.ndarray[uint64], np.ndarray[float32])` of
length k — see Open questions §3.

### 3. GIL release on every hot path

PyO3's `py.allow_threads(|| …)` releases the GIL for the duration of
the closure. We wrap *every* call into the underlying `RuLake` /
`VectorCache` that does work — search, prime, save_cache_to_dir,
warm_from_dir, publish_bundle, refresh_from_bundle_dir — in
`allow_threads`. The closures must not touch any `Py<…>` pointer or
PyO3 object; in practice this means we copy strings and small
parameters into `Send` locals before entering the closure (already
done in the slice borrow above — `&[f32]` is `Send` because the GIL
guarantees the array isn't being mutated underneath us, *but* the
underlying `numpy` crate doesn't expose that guarantee through
`allow_threads` cleanly without a careful lifetime cast).

The pattern we adopt — drilled in CI by an `unsafe`-free clippy lint
against `block_in_place` + `acquire_gil`:

```rust
// 1. Borrow / copy out of Python land *before* allow_threads.
let q: &[f32] = query.as_slice()?;
let backend = backend.to_owned();
let collection = collection.to_owned();

// 2. Release the GIL for the actual work.
let hits = py.allow_threads(|| self.inner.search_one(&backend, &collection, q, k))?;

// 3. Re-enter Python land to convert results.
Ok(hits.into_iter().map(PySearchResult::from).collect())
```

### 4. Sync API in v1, async wrappers in v2

The Rust crate is sync. Adding async to the binding without async in
the underlying crate would require running each call on an executor
inside the binding (`pyo3-asyncio` over a tokio runtime), which:

- doubles the FFI surface (two methods per operation),
- introduces a runtime that the Rust crate doesn't have,
- doesn't actually buy concurrency the user can't already get with
  `concurrent.futures.ThreadPoolExecutor` over the GIL-releasing sync
  API.

v1 ships sync. v2 ships an `async` variant *iff* a real customer asks
for `await lake.search_one(...)` semantics; until then, the
ThreadPoolExecutor pattern works because the GIL is released:

```python
from concurrent.futures import ThreadPoolExecutor
with ThreadPoolExecutor(max_workers=16) as ex:
    futures = [ex.submit(lake.search_one, "local", "docs", q, 10) for q in queries]
    for f in futures:
        print(f.result())
```

This is the same pattern used by `psycopg2`, `lxml`, and every other
GIL-releasing binding. The performance ceiling is the underlying
crate's concurrent QPS (`rulake::BENCHMARK.md` shows
8 threads × 50 queries holds correctness and hit-rate).

### 5. ABI3 wheels distributed via PyPI

CI builds wheels with `maturin build --release --strip --features pyo3/abi3-py39`
on the following matrix, all uploaded to PyPI on tag push:

| Platform | Target triple | Manylinux / image |
|---|---|---|
| Linux x86_64 | `x86_64-unknown-linux-gnu` | `manylinux_2_28` |
| Linux aarch64 | `aarch64-unknown-linux-gnu` | `manylinux_2_28` (qemu or native arm runner) |
| macOS arm64 | `aarch64-apple-darwin` | n/a (native) |
| macOS x86_64 | `x86_64-apple-darwin` | n/a (native) |
| Windows x86_64 | `x86_64-pc-windows-msvc` | n/a (native) |

Plus an sdist (`maturin sdist`) for "I have a Rust toolchain and an
exotic platform" users. The sdist must include the vendored rabitq
sources and the parent `src/` tree, hence the `[tool.maturin].include`
section above.

We **do not** ship 32-bit Linux, musl, or PyPy in v1. They have
single-digit-percent share in the data-engineering / ML audience this
SDK targets and triple the wheel matrix. Add on demand.

### 6. Error mapping — one base exception, typed variants

The Rust `RuLakeError` enum maps to a Python exception hierarchy
rooted at `rulake.RuLakeError(Exception)`:

| Rust variant | Python class |
|---|---|
| `RuLakeError::UnknownBackend` | `rulake.BackendNotFoundError` |
| `RuLakeError::UnknownCollection` | `rulake.CollectionNotFoundError` |
| `RuLakeError::DimensionMismatch` | `rulake.DimensionMismatchError` |
| `RuLakeError::InvalidParameter` | `rulake.InvalidParameterError` |
| `RuLakeError::Backend { .. }` | `rulake.BackendError` |
| `RuLakeError::Rabitq(_)` | `rulake.RuLakeError` (base) |

All typed subclasses inherit from `rulake.RuLakeError`, so
`except RuLakeError:` is the catch-all idiom and
`except DimensionMismatchError:` discriminates.

**v1 ships single inheritance.** The original ADR draft proposed
multi-inheritance from stdlib bases (`LookupError`, `ValueError`,
`OSError`) so idiomatic `except KeyError:` / `except ValueError:` could
catch our errors without learning ruLake-specific names. PyO3 0.22's
`create_exception!` macro caches the exception class on the Rust side
at module load, so monkey-patching the Python class slot in
`__init__.py` after import does not affect what the binding raises.
Building a multi-inherited class via `PyType::new_with_qualname` from
inside the Rust binding is doable but adds non-trivial complexity to
the boilerplate of every variant. Reopened as a v1.5 question if a
real user files a "this is verbose to catch" issue.

## Alternatives considered

### A. ctypes / cffi against a `cdylib`

Reject. Both work but neither gives us:

- automatic `#[pyclass]` → Python class generation,
- `Drop` semantics tied to Python `__del__` (we'd have to write
  malloc/free wrappers and document a `.close()` discipline),
- NumPy buffer-protocol integration without writing the dance manually
  every call.

PyO3 is built specifically for this and the maturin tooling makes the
"distribute wheels" question one config file instead of a CI hand-roll.

### B. Pure-Python rewrite of the cache + router

Reject. The whole product claim is "RaBitQ scan, witness-addressed
cache, 1.02× tax". A Python rewrite of the scan loop is at minimum
~30× slower (NumPy hamming over uint64 × n=100k for k=10 is ~30 ms vs
~1 ms in Rust per `BENCHMARK.md`). A rewrite of just the router with a
Python-implemented cache is feasible but bifurcates the codebase and
the witness story — two implementations of SHAKE-256 over
`(data_ref, dim, seed, rerank, gen)` is one too many.

The only Python code in this SDK is glue, type stubs, and convenience.

### C. HTTP/SSE client against `rvf-server`

Reject *for v1*. This is a real path — `rvf-server` already speaks RVF
wire (per ADR-155 §Context) and a thin Python HTTP client over it
would be ~200 LOC and zero native compile. But:

- Customers running ruLake in-process (notebook, single-server RAG)
  don't want to stand up a server.
- The 1–5 ms RVF-wire round-trip cost (ADR-155 §Consequences "Latency
  hop") is on top of the cache-hit cost, so the "1.02× tax" claim no
  longer applies.
- A wire client is *additive* — we can ship it later under
  `rulake.client` without breaking the in-process API.

Spec'd as a v2 module, `rulake.client.HttpRuLake`, with the same
method surface.

### D. PyO3 with concrete-Python wheels (no ABI3)

Reject. Five wheels × five platforms × every release = 25 wheels per
release. ABI3 collapses that to 5 wheels per release at the cost of
not being able to use Python features added after 3.9 (3.10 pattern
matching, 3.11 exception groups, 3.12 PEP 695 generics).

We don't use any of those in the binding code (it's mostly
`#[pyclass]` impls with no fancy Python). The cost is zero; the win
is large.

### E. Async-first via pyo3-asyncio + Tokio

Reject for v1. The underlying Rust crate is sync. Wrapping sync code
in an async façade by spawning blocking tasks on a tokio runtime
inside the binding gives us *worse* latency than the GIL-released
sync call (because of the cross-runtime hop) for no concurrency win
(threads + GIL-release already gets it).

If/when the Rust crate gains async (e.g. for HTTP backends in M3),
that's the right time to revisit. v1 ships sync.

### F. `BackendAdapter` implementable from Python

Reject for v1. Python-implemented `BackendAdapter`s would need every
`pull_vectors` call to re-acquire the GIL, defeating the
allow_threads pattern, and would force the sync API to become
either `Send`-bounded or to drop `Send` on `BackendAdapter` (breaking
rayon parallel fan-out). v2 will ship a concrete
`PyBackendAdapter` that uses a `tokio::sync::mpsc` queue between a
dedicated Python-callback thread and the parallel scan, so the scan
threads never touch the GIL. Not worth shipping in v1 — `LocalBackend`
+ `FsBackend` cover the cache-warming use cases for early users.

## Consequences

### Positive

- `pip install rulake` Just Works for the default Python
  audience (3.9+ on Linux/macOS/Windows). No Rust toolchain required.
- The 1.02× tax survives the binding. Vectors are zero-copied;
  results are small structs; the GIL is released around the scan.
- One ABI3 wheel per platform × release. CI matrix stays small,
  upgrades stay painless.
- Type stubs ship in the wheel via `py.typed`, so mypy and pyright
  catch dimension/dtype errors at edit time.
- Multiple-inheritance exception classes let users handle errors
  idiomatically without learning ruLake-specific exception names.

### Negative / accepted

- **Submodule must be present at sdist build time.** The
  `[tool.maturin].include` section captures this, but a developer who
  cloned without `--recurse-submodules` and runs `maturin sdist` gets
  a broken sdist. Documented in `python/README.md` and validated in
  CI via "build sdist, install sdist into a fresh container, run
  smoke test".
- **No async in v1.** Users who want `await lake.search_one(...)` must
  use `loop.run_in_executor(...)` or wait for v2. The sync API
  releases the GIL, so this isn't a concurrency limit, only an
  ergonomic one.
- **No Python-implemented backends in v1.** The `BackendAdapter` Rust
  trait is exposed *consumed* (we take an `Arc<dyn BackendAdapter>` in
  `register_backend`) but only the ruLake-shipped backends —
  `LocalBackend`, `FsBackend` — are reachable from Python. Custom
  backends require a Rust crate-level extension; documented as a v2
  feature.
- **NumPy is a mandatory dependency.** Adds ~30 MB to a fresh install.
  Every plausible user has NumPy already; the cost is paid once per
  environment, never per query.

### Neutral

- The Python crate adds ~600 LOC of binding code, ~150 LOC of pure
  Python, and ~300 LOC of type stubs. ~1 engineer-week to ship v1
  (M2-tier work, parallel with Parquet adapter).
- CI gains a wheel-build matrix. Maturin's GitHub Action template is
  well-trodden — the cibuildwheel approach is overkill for ABI3.
- A consequence of ABI3-py39 is that `match` statements in the
  binding's pure-Python layer are fine (the *minimum* is 3.9; runtime
  is whatever the user installed). Type stubs can use 3.10+ syntax
  (`X | Y`) because `py.typed` only declares stubs, not runtime code.

### Verification (acceptance for the v1 PR that lands `python/`)

```text
$ cd python && maturin develop --release
   Compiling rulake-py …
   Built wheel for CPython 3.12 — abi3-py39 — manylinux_2_28_x86_64
$ python -c "
import numpy as np, rulake
lake = rulake.RuLake(rerank_factor=20, rotation_seed=42)
be = rulake.LocalBackend('local')
ids = np.arange(10_000, dtype=np.uint64)
vs = np.random.randn(10_000, 128).astype(np.float32)
be.put_collection('docs', ids=ids, vectors=vs)
lake.register_backend(be)
q = np.random.randn(128).astype(np.float32)
hits = lake.search_one('local', 'docs', q, k=10)
assert len(hits) == 10
print('ok', hits[0].score)
"
ok 132.4
```

Plus a benchmark gate: `python/tests/bench_tax.py` measures
single-thread `search_one` QPS against a direct Rust binary at
n = 100 k, D = 128, k = 10 and asserts the ratio stays ≤ 1.10×. The
budget is wider than the Rust-side 1.02× because of FFI per-call
overhead (~1 µs); a regression past 1.10× is a real degradation.

## Open questions

1. **Python-implemented `BackendAdapter`.** v2. Likely shape: a
   `PyBackendAdapter` Rust struct that owns a `tokio::sync::mpsc`
   channel and dispatches `pull_vectors` calls to a single dedicated
   Python thread (so the parallel-fan-out scan threads never touch
   the GIL). Decide once a customer has a real backend they want to
   write in Python (most likely a custom HTTP API).

2. **Async API.** v2. Decide once the Rust crate has async paths
   (almost certainly when M3's `BigQueryBackend` lands — HTTP I/O
   wants `tokio`). At that point, expose async Python via
   `pyo3-asyncio` over the existing tokio runtime, rather than a
   second runtime.

3. **High-k array-returning search.** v1.5. `search_one_arrays(..., k)
   -> (np.ndarray[uint64], np.ndarray[float32])` for `k > ~500`,
   where the per-`PySearchResult` allocation cost stops being
   negligible. Punt until a user files a real issue with a benchmark.

4. **Pickling / serialization.** Should `RuLake` be picklable? Almost
   certainly *no* — it owns Rust state behind an `Arc` that doesn't
   round-trip through bytes. `RuLakeBundle` should be picklable
   (it already serializes to JSON via `to_json` / `from_json`); we
   delegate `__reduce__` to that. `SearchResult` should be picklable
   trivially (it's plain data).

5. **Free-threaded CPython (PEP 703, 3.13t).** ABI3 wheels do *not*
   target 3.13t. When the no-GIL build matures (likely 3.14+), revisit:
   `py.allow_threads` becomes a no-op, but the binding code is
   correct under it. New wheel target; not breaking.
