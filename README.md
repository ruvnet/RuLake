# ruLake — A Memory Lake for Agentic AI

<a href="https://ruvnet.github.io/RuLake/"><img src="assets/console-hero.png" alt="ruLake Console — live demo at ruvnet.github.io/RuLake/" width="100%" /></a>

> **[Try the live Console →](https://ruvnet.github.io/RuLake/)** — boots in DEMO, auto-probes the hosted MCP at [`rulake-mcp.ruv.io`](https://rulake-mcp.ruv.io/), flips to `● LIVE` when the wire's up. Eight tools served, zero install.

[![Crates.io](https://img.shields.io/crates/v/rulake.svg)](https://crates.io/crates/rulake)
[![Rust 1.89+](https://img.shields.io/badge/rust-1.89%2B-orange.svg)](https://www.rust-lang.org)
[![RuVector](https://img.shields.io/badge/part_of-ruvector-purple.svg)](https://github.com/ruvnet/ruvector)
[![ruv.io](https://img.shields.io/badge/ruv.io-website-purple.svg)](https://ruv.io)
[![MIT / Apache-2.0](https://img.shields.io/badge/License-MIT%2FApache--2.0-blue.svg)](#license)

[![rulake-mcp](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/ruvnet/RuLake/main/badges/rulake-mcp.json)](https://rulake-mcp.ruv.io/)
[![rvdna-mcp](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/ruvnet/RuLake/main/badges/rvdna-mcp.json)](https://rvdna-mcp.ruv.io/)
[![ruqu-mcp](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/ruvnet/RuLake/main/badges/ruqu-mcp.json)](https://ruqu-mcp.ruv.io/)

### Give your AI agents fast, trustworthy memory — without standing up a vector database.

ruLake is the layer between your **agents** and the **data they remember**. Plug in the storage you already have (S3, BigQuery, Snowflake, Parquet, files), expose it through one MCP tool, and every agent on every host gets the same low-latency, content-addressed view of memory. About **1 millisecond per query** at 100k vectors (1.02× raw RaBitQ — the abstraction is free), **32× smaller** than f32 vectors, and every result carries a **SHAKE-256 witness** so two agents on two machines querying the same data get the byte-identical answer or an honest refusal.

> Created by [rUv](https://ruv.io). Part of the [RuVector](https://github.com/ruvnet/ruvector) ecosystem alongside [`ruvector-rabitq`](https://github.com/ruvnet/ruvector/tree/main/crates/ruvector-rabitq) (1-bit compression kernel) and RVF (durable segment format). Powered by [Cognitum](https://cognitum.one).

## Quick start

```bash
# Five install paths. Pick the one that fits where your agent runs.
cargo add rulake                      # Rust
pip   install rulake                  # Python
npm   install rulake                  # Node.js / TypeScript (native binary)
npm   install rulake-wasm             # Browsers, Cloudflare Workers, Deno, Bun
```

```text
# Claude Code, Cursor, Cline — install the marketplace (ADR-009)
/plugin marketplace add ruvnet/RuLake
/plugin install rulake-stack@rulake-marketplace
/reload-plugins                                  # required — Claude Code's install message asks for this

# Slash commands resolve via the <plugin>:<command> namespace.
# Type /rul to autocomplete.
/rulake-core:rulake-query "what does ADR-157 commit to?"
/rulake-witness:rulake-verify path/to/table.rulake.json
/rulake-witness:rulake-bundle-info path/to/table.rulake.json
```

`rulake-stack` bundles three live MCP wires (`rulake-mcp.ruv.io`, `rvdna-mcp.ruv.io`, `ruqu-mcp.ruv.io`) and the slash commands above. The query against the live demo MCP returns the data plus a `decision_trace` block (cost in relative-units, witness match, substrates used, latency).

## Where to next

- **[USERGUIDE.md](USERGUIDE.md)** — full README content moved here: architecture, performance, capabilities (35 features), backends, kernels, security model, examples, status, comparisons.
- **[`docs/userguide/`](docs/userguide/)** — screen-by-screen Console walkthrough with all 7 screenshots. 11 markdown files covering Stats / App Store / Connect / Backends / Bundle / Playground / Audit + Live MCP setup + Troubleshooting.
- **[`docs/gists/`](docs/gists/)** — 11 deep gists, ~34k words. One per shipped ADR.
- **[`docs/deploy/cloud-run.md`](docs/deploy/cloud-run.md)** — production deploy recipe (the one that powers `https://rulake-mcp.ruv.io/`).
- **[ADR-009](docs/adrs/sdk/ADR-009-rulake-plugin-marketplace.md)** — the Claude Code plugin marketplace shape + trust posture.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option. Open source. ❤️ Free forever.
