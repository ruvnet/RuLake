# 01 — verify-witness

A pure-Python re-implementation of the ruLake bundle witness so any
Python service can verify that a `table.rulake.json` it was handed
came from the byte-stream it claims to describe.

The witness is a domain-separated SHAKE-256(32) over the bundle's
`(data_ref, dim, rotation_seed, rerank_factor, generation)` tuple. Two
bundles with the same witness are interchangeable for any query
(see `src/bundle.rs` in the parent crate). If you trust the witness,
you can cache against it across language boundaries.

The Python implementation here MUST byte-exactly match the Rust
output. The test suite pins three fixtures:

- `fixtures/known-good-bundle.json` — hand-rolled with deterministic
  inputs.
- `fixtures/rust-sidecar-daemon.json` — produced live by
  `cargo run --release --example sidecar_daemon`.
- `fixtures/rust-warm-restart.json` — produced live by
  `cargo run --release --example warm_restart`.

## Install

```bash
cd examples/python/01-verify-witness
python3 -m venv .venv
. .venv/bin/activate
pip install -e .[dev]
```

No third-party crypto needed at runtime — `hashlib.shake_256` from the
stdlib covers SHAKE-256(32). `pytest` and `mypy` are pulled in only as
dev extras.

## Run the CLI

```bash
# Verify the hand-rolled fixture:
python verify_witness.py fixtures/known-good-bundle.json

# Verify a real sidecar produced by the Rust example:
cd /path/to/RuLake
cargo run --release --example sidecar_daemon &
# (the example creates /tmp/rulake-sidecar-demo-<pid>/ then cleans up;
#  copy table.rulake.json out before it exits)

python /path/to/examples/python/01-verify-witness/verify_witness.py \
    /tmp/rulake-sidecar-demo-<pid>/
```

Exit codes:

- `0` — parsed cleanly and the witness matched.
- `1` — witness mismatch (tamper, drifted writer, schema bug).
- `2` — could not be opened or parsed (missing, oversize, bad JSON,
  unknown `format_version`).

## Run the tests

```bash
pytest tests/ -v
```

The tests cover:

- the three pinned fixtures recompute byte-for-byte;
- the `Num(7)` vs `Opaque("\x07\0\0\0\0\0\0\0")` collision regression
  (the audit fix from 2026-04-23);
- length-prefixing of `data_ref`/`generation` (so `"a|b"` cannot collide
  with `"ab|"`);
- DoS guards reject oversize JSON, oversize string fields, future
  `format_version`, JSON booleans masquerading as `Generation::Num`;
- a tampered `dim` field on disk is caught by `verify_witness()`.

## Use as a library

```python
from rulake_witness import compute_witness, read_bundle, BundleError

# Recompute from raw fields.
w = compute_witness(
    data_ref="gs://bucket/x.parquet",
    dim=128, rotation_seed=42, rerank_factor=20,
    generation=7,                # int -> Generation::Num
    # generation="01JCX7...",    # str -> Generation::Opaque
)
assert len(w) == 64

# Read a bundle from disk; verify it.
bundle = read_bundle("/path/to/table.rulake.json")
if not bundle.verify_witness():
    raise RuntimeError("bundle witness did not match recompute")
```

## Witness algorithm (verbatim from `src/bundle.rs`)

```
SHAKE-256(32) of:
    "rulake-bundle-witness-v1|"
    || u64_le(len(data_ref)) || data_ref
    || "|"
    || u64_le(dim) || u64_le(rotation_seed) || u64_le(rerank_factor)
    || "|"
    || u64_le(len(generation_bytes)) || generation_bytes
```

`generation_bytes` is tagged: `0x00 || u64_le(n)` for numeric, or
`0x01 || utf8(s)` for opaque. The tag byte is the audit-driven fix
preventing `Num(7)` colliding with `Opaque("\x07\0\0\0\0\0\0\0")`.

## DoS caps (mirror of the Rust parser)

- whole bundle ≤ 64 KiB (`MAX_JSON_BYTES`)
- each string field ≤ 4 KiB (`MAX_FIELD_BYTES`)
- `rvf_witness` length ≤ 128 (must be exactly 64 hex chars in practice)
- `format_version > 2` rejected
- JSON `true` / `false` for `generation` rejected (would silently
  digest as `Num(0)` / `Num(1)` otherwise)
