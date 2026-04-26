# ruLake — Python SDK

PyO3 bindings for [`ruvector-rulake`](https://github.com/ruvnet/RuLake) —
a vector cache + federation intermediary that sits in front of whatever
data lake already holds your vectors.

Implements [ADR-002](../docs/adrs/sdk/ADR-002-python-sdk.md).

## Install

```bash
pip install ruvector-rulake     # once published — wheels for cp39+
```

## Build from source

This repo uses a Git submodule for the upstream RaBitQ kernel (see
[ADR-001](../docs/adrs/ADR-001-standalone-repo-strategy.md)). Clone with
submodules, then build the wheel via maturin:

```bash
git clone --recurse-submodules https://github.com/ruvnet/RuLake
cd RuLake/python
pip install maturin
maturin develop --release
pytest tests/ -v
```

`maturin develop` builds the `_rulake` native module and installs it
into the current Python environment in editable form. For a release
wheel, use `maturin build --release` — the `.whl` lands in
`../target/wheels/`.

## Usage

```python
import numpy as np
import rulake

lake = (
    rulake.RuLake(rerank_factor=20, rotation_seed=42)
    .with_consistency(rulake.Consistency.eventual(ttl_ms=5000))
    .with_max_cache_entries(1024)
)

be = rulake.LocalBackend("local")
ids     = np.arange(10_000, dtype=np.uint64)
vectors = np.random.randn(10_000, 768).astype(np.float32)
be.put_collection("docs", ids, vectors)
lake.register_backend(be)

q = np.random.randn(768).astype(np.float32)
for hit in lake.search_one("local", "docs", q, k=10):
    print(hit.backend, hit.collection, hit.id, hit.score)

# Federate across backends:
hits = lake.search_federated([("local", "docs")], q, k=5)

# Bundles — language-portable witness-anchored sidecars:
b = rulake.Bundle("s3://bucket/path", dim=768, rotation_seed=42, rerank_factor=20, generation=1)
b.write_to_dir("/tmp/snapshot")
b2 = rulake.Bundle.read_from_dir("/tmp/snapshot")
assert b2.verify_witness()

# Cache-first KPI (ADR-155 §M1.5 — target ≥ 0.95):
print(lake.cache_stats().hit_rate())
```

## Conventions

- **Vectors are `np.ndarray[float32]`, C-contiguous.** The binding
  borrows them zero-copy. Non-contiguous or wrong-dtype arrays raise
  `ValueError` rather than silently copying — see ADR-002 §2.
- **IDs are `np.ndarray[uint64]`.** Same contiguous-or-error rule.
- **The GIL is released** for every search / prime / publish / refresh
  call (ADR-002 §3). Use `concurrent.futures.ThreadPoolExecutor` for
  parallel queries; the underlying Rust crate is `Send + Sync` and the
  RaBitQ scan runs lock-free under contention.
- **Errors map to a typed hierarchy** rooted at `rulake.RuLakeError`.
  All typed subclasses (`BackendNotFoundError`, `DimensionMismatchError`,
  …) inherit from it, so `except RuLakeError:` is the catch-all idiom
  and the typed subclass discriminates (ADR-002 §6).

## What's not (yet) here

See ADR-002 §"Open questions":

- **Async API** — v1 is sync, GIL-released. Use `ThreadPoolExecutor` for
  concurrency. Async lands when the underlying crate gains async paths
  (likely M3 with the BigQuery backend).
- **Python-implemented `BackendAdapter`** — v2. Currently only the
  Rust-shipped backends (`LocalBackend`, `FsBackend`) are reachable.
- **HTTP client variant (`rulake.client.HttpRuLake`)** — v2.

## License

MIT OR Apache-2.0, matching the parent crate.
