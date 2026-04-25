# Python interop examples for ruLake

ruLake is a Rust crate. There are no pyo3 / maturin bindings to it
yet — and there don't have to be. Every cross-process boundary in
ruLake is mediated by the **bundle protocol** (`table.rulake.json`,
plus the binary data file it points at), which is JSON + a documented
binary format. Any language that can hash and parse JSON can
participate.

These four examples show what that looks like from Python:

| # | Module | What it shows |
|---|--------|---------------|
| 01 | [`01-verify-witness`](./01-verify-witness/) | Re-derive the SHAKE-256 bundle witness in pure Python; pinned against fixtures captured live from the Rust examples |
| 02 | [`02-bundle-server`](./02-bundle-server/) | FastAPI server that watches a publish dir for sidecars and serves them (witness as ETag) over HTTP |
| 03 | [`03-subprocess-wrapper`](./03-subprocess-wrapper/) | Python wrapper around the `rulake-demo` Rust binary — spawn, parse stdout, expose typed dataclasses |
| 04 | [`04-rag-grounded`](./04-rag-grounded/) | Witness-pinned RAG: read a ruLake snapshot, brute-force L2 search, build an LLM prompt with `provenance_id = witness` on every hit |

## What you need

- Python 3.11+
- For modules 01-04: the standard `pip install -e .[dev]` recipe
  inside each subdirectory
- For module 03 to actually run the binary: a built `rulake-demo`
  somewhere on disk (the wrapper falls back to `cargo run` if it
  can't find a pre-built one)
- For module 04's optional `--use-sentence-transformers`:
  `pip install -e .[embed]`

No third-party crypto libraries needed at runtime — `hashlib.shake_256`
from the stdlib covers the SHAKE-256(32) bundle witness.

## The keystone module

`01-verify-witness` is the one to read first. Everything else builds
on the witness algorithm: the bundle server uses the witness as an
ETag, the RAG pipeline uses it as `provenance_id`, and the subprocess
wrapper happens to be the only one that doesn't touch witnesses
directly (because it's poking at the Rust binary's perf metrics
rather than its bundle output).

The witness must byte-match what Rust produces. The fixtures in
`01-verify-witness/fixtures/` include `rust-sidecar-daemon.json` and
`rust-warm-restart.json`, captured live from the corresponding Rust
examples — if Python and Rust ever drift on the witness, the test
suite goes red immediately.

## Running everything

```bash
# 01 — verify witness (no external deps)
cd 01-verify-witness && pip install -e .[dev] && pytest tests/ -v

# 02 — bundle server (depends on 01)
cd ../02-bundle-server && pip install -e ../01-verify-witness && pip install -e .[dev]
pytest tests/ -v

# 03 — subprocess wrapper (no external deps; end-to-end test gated)
cd ../03-subprocess-wrapper && pip install -e .[dev] && pytest tests/ -v
RULAKE_RUN_END_TO_END=1 pytest tests/ -v   # actually spawns rulake-demo

# 04 — rag grounded (depends on 01)
cd ../04-rag-grounded && pip install -e ../01-verify-witness && pip install -e .[dev]
pytest tests/ -v
```

## What this is and isn't

**Is**: a demonstration that ruLake's interop story does not require
language-specific bindings — the bundle protocol IS the interop story.

**Isn't**: pyo3 / maturin bindings to the Rust crate. Building real
bindings is weeks of work (ABI versioning, wheel publishing for the
four major platforms, ongoing keep-up with crate API changes). The
bundle protocol gets you witness-pinned cross-language coherence for
the cost of a SHAKE-256 hash.
