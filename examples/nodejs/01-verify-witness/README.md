# 01 — verify-witness

Verify a ruLake `table.rulake.json` bundle from Node / TypeScript without
any native bindings. Recomputes the SHAKE-256(32) `rvf_witness`
byte-for-byte against the Rust crate's `compute_witness` and reports
match / mismatch.

This is the foundation for every cross-language ruLake interop story:
once you can verify a bundle in your runtime, you can trust any cache
entry it anchors.

## What it implements

- `src/witness.ts` — `computeWitness()` / `verifyBundle()`, matching
  `src/bundle.rs::compute_witness` (witness format v2, 2026-04-23 audit
  fix included).
- `src/bundle.ts` — strict, DoS-capped parser for the JSON bundle.
- `src/cli.ts` — `tsx src/cli.ts <dir>` verifier for use in CI.
- `tests/witness.test.ts` — unit tests + the `Num`/`Opaque` collision
  regression + a real Rust-produced fixture.

## Install

```bash
cd examples/nodejs/01-verify-witness
npm install
```

## Run the tests

```bash
npm test
```

The fixture round-trip test loads `fixtures/known-good-bundle.json`
(produced by the Rust `save_cache_to_dir` API) and asserts the Node
witness matches.

## Verify a real Rust-produced sidecar

From the repo root:

```bash
# Produce a bundle (any of the existing Rust examples will do).
cargo run --release --example sidecar_daemon
# ...note the "Publish directory: /tmp/rulake-sidecar-demo-<pid>" line.

# Or simpler: regenerate the bundled fixture path used by the tests.
# The 01-verify-witness CLI takes any dir containing a table.rulake.json:
cd examples/nodejs/01-verify-witness
npx tsx src/cli.ts /tmp/rulake-sidecar-demo-<pid>
```

Expected output:

```
bundle: /tmp/rulake-sidecar-demo-<pid>/table.rulake.json
format_version: 2
data_ref:       local://publisher/memories
dim:            8
rotation_seed:  42
rerank_factor:  20
generation:     Num(1)
witness:        dea58c64adb1eb4109438f0353a2b1749d4dc29ed7266e9236720ab6cf07d7e4

MATCH — recomputed SHAKE-256(32) agrees with on-disk rvf_witness.
```

Exit codes: `0` MATCH, `1` MISMATCH, `2` missing or malformed bundle.

## Implementation notes

- u64 fields are handled as `bigint` end-to-end so values above
  `Number.MAX_SAFE_INTEGER` round-trip without precision loss. The
  little-endian encoder lives in `u64LE()` and is the most fragile
  part — every bug in the witness mismatch path eventually traces
  back to it.
- The variant tag byte (`0x00` for `Num`, `0x01` for `Opaque`) is the
  audit-driven 2026-04-23 fix. Without it, `Num(7)` collides with
  `Opaque("\x07\0\0\0\0\0\0\0")`. The included regression test pins
  this contract.
- DoS caps: 64 KiB whole bundle, 4 KiB per string field, 64-char
  witness. These match the Rust `MAX_JSON_BYTES` / `MAX_FIELD_BYTES`.
