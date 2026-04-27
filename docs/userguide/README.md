# ruLake Console — User Guide

ruLake is a cache-coherent vector federation intermediary built on RVF (Reified
Vector Format). It sits between your existing vector stores (GCS Parquet, IPFS,
local files, custom adapters) and AI agents speaking MCP, returning
trust-anchored bundles whose contents are pinned by a SHAKE-256(32) witness
hash. The Console is a static React SPA that exercises the wire end-to-end
against a hosted MCP at [`https://rulake-mcp.ruv.io/`](https://rulake-mcp.ruv.io/),
auto-flipping from `○ DEMO` to `● LIVE` once the probe lands. This guide walks
through every screen, the tools the Console drives, and the operational knobs
that matter when you point it at your own server.

## Pages

| # | Page | What it covers |
|---|---|---|
| 01 | [Getting started](./01-getting-started.md) | Open the live console, what auto-probes on boot, keyboard shortcuts |
| 02 | [Stats screen](./02-stats-screen.md) | Hits / misses / hit-rate / primes / refusals / verified counts |
| 03 | [App store](./03-app-store.md) | Substrate catalog, `SHIPPING` vs `SCAFFOLDED` tags, install commands |
| 04 | [Connect](./04-connect.md) | Endpoint, auth modes, what the topbar `LIVE` pill means |
| 05 | [Backends and collections](./05-backends-and-collections.md) | Browse backends, `BackendAdapter` trait, federation graph |
| 06 | [Bundle / witness viewer](./06-bundle-witness-viewer.md) | Fields of a `table.rulake.json` sidecar, browser-side recompute |
| 07 | [Playground](./07-playground.md) | Issuing `rulake_query`, k / risk / target, decision trace |
| 08 | [Audit ledger](./08-audit-ledger.md) | Event types, outcome filters, where rows come from |
| 09 | [Live MCP setup](./09-live-mcp-setup.md) | Three-minute summary; defers to the Cloud Run deploy doc |
| 10 | [Troubleshooting](./10-troubleshooting.md) | TLS, DEMO-stuck, CORS, 401, `RULAKE_ALLOWED_HOSTS` |

## Quick links

- Hosted Console: <https://ruvnet.github.io/RuLake/>
- Hosted MCP: <https://rulake-mcp.ruv.io/>
- Repository: <https://github.com/ruvnet/RuLake>
- Cloud Run deploy recipe: [`docs/deploy/cloud-run.md`](../deploy/cloud-run.md)
- Live-wire smoke test: [`scripts/smoke-live.sh`](../../scripts/smoke-live.sh)

## Conventions used in this guide

- Shell snippets assume bash and a recent `curl`. They run unchanged against
  the public MCP at `https://rulake-mcp.ruv.io/`.
- Cargo / npm install lines reference real published crates and packages
  (`rulake` on crates.io, `rulake-wasm` on npm).
- "the Console" refers to the SPA at `https://ruvnet.github.io/RuLake/`.
  "the MCP" refers to whichever `rulake-mcp` server it is currently pointed at.
