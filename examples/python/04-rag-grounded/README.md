# 04 — rag-grounded

A witness-pinned RAG pipeline that consumes a ruLake snapshot
directory from pure Python. Demonstrates the cross-language
provenance story: a Rust ruLake instance publishes a snapshot, a
Python service grounds an LLM prompt against it, and every retrieved
hit carries the bundle's witness as `provenance_id` so a downstream
auditor can verify which exact snapshot the model was grounded on.

## What this example does

1. Reads `dir/table.rulake.json` and **verifies the witness**
   (re-uses the verifier from `01-verify-witness`).
2. Reads the binary vector data file (the documented `ruvec1` format
   that `FsBackend` writes — see `src/fs_backend.rs`).
3. Cross-checks the bundle's `dim` against the file header.
4. Embeds the query (deterministic SHAKE-256 projection by default;
   try `--use-sentence-transformers` to swap in a real model).
5. Brute-force L2 search for the top-K nearest vectors.
6. Returns hits annotated with the witness as `provenance_id`.
7. Builds a witness-pinned LLM prompt and prints it.

## A note on `index.rbpx` vs `ruvec1`

The `warm_restart.rs` Rust example writes a rabitq-compressed
`index.rbpx` next to the bundle. That file is opaque without the
`ruvector-rabitq` library, which is not available from Python. We
instead consume the **`ruvec1`** format that `FsBackend` writes —
which is plainly parseable from Python — and play the same role:
witnessed bundle + portable data file. The directional value
(witness-pinned provenance, deterministic snapshots, language-portable
bundle) survives the swap. When pyo3 bindings to `ruvector-rabitq`
land, this pipeline can swap the data-file reader and keep everything
above it intact.

## Install

```bash
cd examples/python/04-rag-grounded

# 01-verify-witness is a path dep, install it first.
pip install -e ../01-verify-witness

python3 -m venv .venv
. .venv/bin/activate
pip install -e .[dev]

# Optional: real sentence embeddings.
pip install -e .[embed]
```

## Run the demo

```bash
# Materialize a tiny demo snapshot and run the pipeline against it.
python pipeline.py --materialize-demo --snapshot ./demo-snapshot
```

Expected output (witness will differ; everything else stable)::

    materialized demo snapshot at demo-snapshot
    snapshot: demo-snapshot
    corpus size: 50
    witness: 7c1b...                                                ...d9f4
    data_ref: file:///abs/path/to/demo-snapshot/vectors.ruvec1

    top-5 hits for: 'how do warm caches survive a restart?'
      id= 1003  l2_sq=    0.0421  provenance=7c1b...
      id= 1008  l2_sq=    0.0498  provenance=7c1b...
      ...

    [SYSTEM]
    Source: file:///abs/path/to/demo-snapshot/vectors.ruvec1
    Provenance witness: 7c1b...
    ...
    [QUERY]
    how do warm caches survive a restart?

    [RESPONSE]

## Use against a Rust-published snapshot

```bash
# 1. Have a Rust producer write a ruvec1 file + bundle into <dir>/.
#    This needs a ruLake program that uses FsBackend (the warm_restart
#    example writes a different, rabitq-compressed format that this
#    example doesn't parse — see "A note on index.rbpx" above).
# 2. Run the pipeline with --snapshot pointing at that dir.

python pipeline.py \
    --snapshot /path/to/rust-published-snapshot \
    --query "an actual question" \
    --k 10
```

## Use as a library

```python
from pathlib import Path
from pipeline import load_corpus, brute_force_topk, embed_query, build_prompt

corpus = load_corpus(Path("/path/to/snapshot"))
qvec = embed_query("query text", corpus.dim)
hits = brute_force_topk(corpus, qvec, k=5)

# The witness travels with every hit.
for h in hits:
    assert h.provenance_id == corpus.witness()

prompt = build_prompt("query text", hits, corpus_label=corpus.bundle.data_ref)
# ... hand `prompt` to the LLM SDK of your choice ...
```

## Tests

```bash
pytest tests/ -v
```

Tests cover:

- ruvec1 round-trip (bytes byte-equal to what `FsBackend::write`
  produces);
- ruvec1 parser DoS guards (bad magic, truncated, oversize dim,
  payload-size mismatch);
- witness verification on `load_corpus`;
- bundle-vs-corpus dim mismatch is rejected;
- deterministic embedder + brute-force ranking are stable;
- end-to-end pipeline propagates the witness as `provenance_id` on
  every hit and into the prompt's system block.

## ruvec1 format reference

From `src/fs_backend.rs` (verbatim)::

    bytes  field
    0..8   magic         = b"ruvec1\0\0"
    8..16  count : u64   little-endian
   16..20  dim   : u32   little-endian
   20..24  _reserved     (must be zero)
   24..    records × count, each:
             id : u64  little-endian
             v  : f32 × dim  little-endian

`write_ruvec1` and `read_ruvec1` in `pipeline.py` are byte-equal to
the Rust `FsBackend::write` / `FsBackend::pull_vectors` paths. A Rust
ruLake reader can consume a snapshot this Python module wrote and
vice-versa.
