# 04 — mcp-tool

An MCP (Model Context Protocol) server that exposes a ruLake snapshot
as agent-callable tools. Built on `@modelcontextprotocol/sdk` over
stdio, so any MCP client (Claude Code, the MCP inspector, your own
agent) can drive ruLake retrieval directly.

This is the highest-leverage Node example: it puts ruLake on the
agentic substrate that the README's "vector-native federation
intermediary" framing points at.

## Tools exposed

| Name                     | Description |
|--------------------------|-------------|
| `rulake_search`          | Brute-force exact L2 K-NN against a snapshot. Refuses to search if the bundle witness doesn't verify. |
| `rulake_verify_witness`  | Recompute SHAKE-256 over a snapshot's `table.rulake.json` and report match / mismatch. |
| `rulake_bundle_info`     | Return parsed bundle metadata (witness, data_ref, dim, generation, policy / lineage tags). |

## Honest scope

The Node side does NOT have the RaBitQ decompressor. So `rulake_search`
loads `index.rbpx` (the deterministic-rebuild payload — original ids
+ float32 vectors), and runs exact L2 over them. That's correct, but
slow at production scale. For a fast serving path host the index in a
Rust ruLake process and front it with module 02; this MCP tool is the
agent face.

The `.rbpx` reader and bounds match the Rust loader in
`vendor/ruvector/crates/ruvector-rabitq/src/persist.rs`.

## Install

```bash
cd examples/nodejs/04-mcp-tool
npm install
```

## Run as an MCP stdio server

```bash
npx tsx src/server.ts
```

The server speaks JSON-RPC on stdin/stdout. The standard way to drive
it during development is the MCP inspector:

```bash
npx @modelcontextprotocol/inspector npx tsx src/server.ts
```

To wire it into Claude Code add a stanza like this to the user MCP
config:

```json
{
  "mcpServers": {
    "rulake": {
      "command": "npx",
      "args": ["tsx", "/abs/path/to/examples/nodejs/04-mcp-tool/src/server.ts"]
    }
  }
}
```

## Producing a snapshot to point an agent at

```bash
cd /path/to/RuLake
cargo run --release --example warm_restart
# note the "Snapshot dir: /tmp/rulake-warm-demo-<pid>-<nanos>" line
```

The agent calls:

```jsonc
// rulake_verify_witness
{ "snapshot_dir": "/tmp/rulake-warm-demo-..." }

// rulake_bundle_info
{ "snapshot_dir": "/tmp/rulake-warm-demo-..." }

// rulake_search
{ "snapshot_dir": "/tmp/rulake-warm-demo-...",
  "query": [0.1, 0.2, /* ...dim floats... */],
  "k": 5 }
```

`rulake_search` refuses to search when `rulake_verify_witness` would
return `ok: false`. This is the same fail-closed stance the Rust
`read_from_dir` and module 02's HTTP server take.

## Test

```bash
npm test
```

Tests use the MCP SDK's in-memory transport pair to drive the server
end-to-end (real registration, real dispatch, real responses). When a
real Rust-produced snapshot exists at `/tmp/rulake-fixture` (created by
running any of the Rust examples and copying their output) the tests
also exercise `rulake_search` against it; otherwise those tests skip
with a clear message.
