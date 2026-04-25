# ruLake — Node.js / TypeScript examples

Four self-contained TypeScript examples that show how Node code
interoperates with ruLake. There are no native bindings; every example
goes through one of the two adoption paths the README documents:

1. The **bundle protocol** — language-portable JSON + binary files
   the Rust crate produces (`table.rulake.json`, `index.rbpx`).
2. The `rulake-demo` Rust **subprocess**, parsed from stdout.

This is the real adoption path: most teams add ruLake to a heterogeneous
stack via the bundle protocol or by shelling out, long before they
write FFI bindings.

## Prerequisites

- Node 20+ (each example pins `engines.node >=20`)
- The Rust crate built with `cargo build --release` for examples that
  need a Rust-produced bundle or the `rulake-demo` binary

```bash
cd /path/to/RuLake
cargo build --release --bin rulake-demo
```

## The four examples

| # | Path | What it shows |
|---|------|----------------|
| 1 | [`01-verify-witness/`](./01-verify-witness/) | Compute and verify a `table.rulake.json` SHAKE-256 witness, byte-for-byte compatible with `src/bundle.rs::compute_witness`. |
| 2 | [`02-bundle-publisher/`](./02-bundle-publisher/) | Express HTTP server that watches a publish directory and serves bundles with witness-keyed ETags + 304 revalidation. |
| 3 | [`03-subprocess-wrapper/`](./03-subprocess-wrapper/) | Async TS class that spawns `rulake-demo`, parses its stdout, returns structured benchmark results. |
| 4 | [`04-mcp-tool/`](./04-mcp-tool/) | Model Context Protocol server exposing ruLake snapshots as agent-callable tools (search, witness verify, bundle info). |

## Recommended reading order

1. **01-verify-witness** — pins down the on-disk bundle contract. If
   this example doesn't agree with Rust, nothing else is trustworthy.
2. **02-bundle-publisher** — operational flavour: how a Node service
   distributes bundles to readers in any language.
3. **03-subprocess-wrapper** — pragmatic: drive the Rust binary
   without bindings.
4. **04-mcp-tool** — agentic substrate: ruLake as a first-class tool
   on the MCP bus.

## Common dependencies

Every example uses standalone `npm install` — there is no shared
lockfile or workspaces config. Witness computation everywhere uses
[`@noble/hashes`](https://github.com/paulmillr/noble-hashes) (audited,
zero-dep, browser-portable SHAKE-256).

## Cross-language witness check

The most important interop guarantee is that the Node witness equals
the Rust witness for any v2 bundle. The 01-verify-witness fixture
(`fixtures/known-good-bundle.json`) was produced by the Rust
`save_cache_to_dir` API; the test suite asserts the Node-recomputed
SHAKE-256(32) digest matches the on-disk `rvf_witness` byte-for-byte.

If you change the witness algorithm in `src/bundle.rs`, regenerate the
fixture and update both sides — there's a `Num` / `Opaque` collision
regression test in 01-verify-witness that pins the 2026-04-23 audit fix.

## Testing

```bash
# Each example has its own scripts:
( cd 01-verify-witness && npm install && npm test )
( cd 02-bundle-publisher && npm install && npm test )
( cd 03-subprocess-wrapper && npm install && npm test )
( cd 04-mcp-tool && npm install && npm test )
```
