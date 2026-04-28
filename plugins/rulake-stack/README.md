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

## Default MCP wires

Three HTTP wires, all pointing at the public demo:

- `rulake` → `https://rulake-mcp.ruv.io/`
- `rvdna-mcp` → `https://rvdna-mcp.ruv.io/`
- `ruqu-mcp` → `https://ruqu-mcp.ruv.io/`

The demo wires are read-only, with no auth (`--auth none --insecure-allow-no-auth` per `docs/deploy/cloud-run.md`). For production with real data:

1. Deploy your own MCP per [`docs/deploy/cloud-run.md`](https://github.com/ruvnet/RuLake/blob/main/docs/deploy/cloud-run.md)
2. Override the URL in your local Claude Code config

## Trust posture

- Defaults to `--capabilities read` (no mutation tools surface)
- Public demo URLs are unambiguous in the Console (○ DEMO vs ● LIVE pill)
- Plugins are versioned (`2.3.0-alpha.1`); pin with `@2.3.0-alpha.1` to lock
- Releases are tagged on the repo

See [ADR-009](https://github.com/ruvnet/RuLake/blob/main/docs/adrs/sdk/ADR-009-rulake-plugin-marketplace.md) for the full positioning + trust model.
