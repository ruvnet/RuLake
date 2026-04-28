# rulake-stack

The killer-path install for ruLake. One command, working retrieval.

## What you get

This plugin bundles three sub-plugins with sensible defaults:

- **rulake-core** — the 8 `rulake_*` MCP tools + the `/rulake-query` skill
- **rulake-substrates** — `rvdna_*` + `ruqu_*` tools (5 each), wired to the live demos
- **rulake-witness** — `/rulake-verify` and `/rulake-bundle-info` commands

Kernels (`rulake-kernels`) and the `/loop`-aware workers (`rulake-loop-vector`) are intentionally **off by default** — install those separately if you need them.

## Install

```text
/plugin marketplace add ruvnet/RuLake
/plugin install rulake-stack@rulake-marketplace
```

## First query

```text
/rulake-query "what does ADR-157 commit to?"
```

You should get back the result + a `decision_trace` block showing cache hit, substrates used, kernel chosen, witness match, cost, and latency. If that loop is under 60 seconds wall-clock, the killer path works.

## Default MCP wires (local stdio)

Three local stdio wires — Claude Code spawns the binaries on demand:

```jsonc
{
  "rulake":    { "type": "stdio", "command": "rulake-mcp",
                 "args": ["stdio", "--demo-backend", "--capabilities", "read,publish,admin"] },
  "rvdna-mcp": { "type": "stdio", "command": "rvdna-mcp",
                 "args": ["stdio", "--demo-collection", "--capabilities", "read,internal"] },
  "ruqu-mcp":  { "type": "stdio", "command": "ruqu-mcp",
                 "args": ["stdio", "--capabilities", "read,publish"] }
}
```

**Local has full caps** (`admin,publish,read` on rulake), **~1ms latency** (no network round-trip), and **doesn't depend on Cloud Run being up**. Trade-off: the binaries need to be on `PATH`.

### One-time install (binaries on PATH)

```bash
# from a clone of ruvnet/RuLake
cargo install --path crates/mcp-server     # rulake-mcp
cargo install --path crates/mcp-rvdna      # rvdna-mcp
cargo install --path crates/mcp-ruqu       # ruqu-mcp
```

These land in `~/.cargo/bin/` which is typically on `PATH`. Verify with `which rulake-mcp`.

### Using the hosted demo instead

If you don't want to build the binaries, override the wires in `~/.claude.json` to point at the hosted Cloud Run instances (`https://rulake-mcp.ruv.io/`, `https://rvdna-mcp.ruv.io/`, `https://ruqu-mcp.ruv.io/`). Hosted has higher latency (network round-trip) and ships with `--auth none --insecure-allow-no-auth`.

## Trust posture

- Defaults to `--capabilities read` (no mutation tools surface)
- Public demo URLs are unambiguous in the Console (○ DEMO vs ● LIVE pill)
- Plugins are versioned (`2.3.0-alpha.1`); pin with `@2.3.0-alpha.1` to lock
- Releases are tagged on the repo

See [ADR-009](https://github.com/ruvnet/RuLake/blob/main/docs/adrs/sdk/ADR-009-rulake-plugin-marketplace.md) for the full positioning + trust model.
