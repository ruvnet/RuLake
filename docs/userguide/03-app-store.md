# 03 — App store

The App store route is a static catalog of substrates that compose with
ruLake. It is not a marketplace — there is nothing to buy and nothing to
download from this screen. It exists to answer one question: *what other
crates plug into the same witness-anchored cache, and where is each one in
its lifecycle?*

![App Store screen listing four substrates with status tags and install commands](../../assets/console-appstore.png)

## How to read an entry

Each card has the same six regions:

```
┌─────────────────────────────────────────────────────────────────┐
│ <name>  · <one-line tag>                              [STATUS]  │
│ <pitch — 2-3 sentences on what this adds>                       │
│                                                                 │
│ ADDS                              INSTALL                       │
│ · <tool name 1>                   $ cargo add <crate>           │
│ · <tool name 2>                   $ cargo run -p <mcp crate>    │
│ · <tool name 3>                   ● live · <hosted url>         │
│ · ...                             $ npm install <pkg>           │
│                                                                 │
│ tier: <runtime tier descriptor>          ADR ↗   gist ↗   research/ ↗
└─────────────────────────────────────────────────────────────────┘
```

- **Name + tag** — short product name and a pitch-line.
- **Status pill** (top right) — see "Status tags" below.
- **Pitch** — what the substrate adds to the agentic memory model.
- **ADDS** — the new MCP tools / capabilities the substrate brings
  alongside the core eight `rulake-mcp` tools.
- **INSTALL** — a stack of one-liners. Rust crate first, then the MCP
  companion server, then the live URL if hosted, then npm and any browser
  surface.
- **Footer links** — ADR (the design decision), gist (a 2,500–3,700 word
  deep companion), and `research/` (benchmarks, security reviews).

## Status tags

The tag colour and text correspond to the substrate's lifecycle stage. The
mapping is deliberate — no marketing inflation.

| Tag | What it actually means | When to use it |
|---|---|---|
| `SHIPPING` (green) | Source code lives in `crates/<name>/`, tests are green on CI, an MCP companion or Rust API is usable today. | Production-eligible. Read the ADR for the supported tier matrix. |
| `SCAFFOLDED` (amber) | Crate exists, design is ratified in an ADR, but only a v0.0.1 surface is implemented. Behaviour is real; coverage is partial. | Try it. Expect rough edges and `Proposed` items in the ADR roadmap. |
| `PROPOSED` (warm) | ADR exists; no source. | Read the ADR if curious. Don't depend on it. |
| `ROADMAP` (cool) | Future intent only. | Information-only. |

The Console currently ships four cards, all `SHIPPING`:

- **rvDNA v2** — genomic intelligence substrate
  ([ADR-007](https://github.com/ruvnet/RuLake/blob/main/docs/adrs/ADR-007-rvdna-as-rulake-substrate.md)).
  Five MCP tools via `mcp-rvdna`; live demo at
  [`rvdna-mcp.ruv.io`](https://rvdna-mcp.ruv.io/).
- **ruQu v2** — quantum execution intelligence substrate
  ([ADR-008](https://github.com/ruvnet/RuLake/blob/main/docs/adrs/ADR-008-ruqu-as-rulake-substrate.md)).
  Five MCP tools via `mcp-ruqu`; live demo at
  [`ruqu-mcp.ruv.io`](https://ruqu-mcp.ruv.io/).
- **gcs-backend** — Parquet on Google Cloud Storage
  ([ADR-155](https://github.com/ruvnet/RuLake/blob/main/docs/adrs/ADR-155-rulake-datalake-layer.md)).
  Storage adapter; cache coherence rides per-object generation tokens.
- **ipfs-backend** — witness-anchored bundle distribution
  ([ADR-005](https://github.com/ruvnet/RuLake/blob/main/docs/adrs/ADR-005-ipfs-backend-and-deploy.md)).
  Three modes: kubo, gateway-only, kubo + gateway-fallback.

## How a substrate plugs in

Every substrate in this catalog implements `BackendAdapter` from
`crates/core/src/backend.rs`. That trait is four methods:

```rust
pub trait BackendAdapter: Send + Sync {
    fn id(&self) -> &str;
    fn list_collections(&self) -> Result<Vec<CollectionId>>;
    fn pull_vectors(&self, collection: &str) -> Result<PulledBatch>;
    fn generation(&self, collection: &str) -> Result<u64>;
    // Optional — return the bundle the cache should anchor to.
    fn current_bundle(&self, collection: &str) -> Result<Option<RuLakeBundle>> { Ok(None) }
}
```

That is the entire integration surface. Implement it once per substrate and
ruLake's federation, witness, audit, and consistency modes work without any
substrate-specific special-casing in the core.

## Live demo links

For substrates with a hosted MCP, the install card includes a `● live · <url>`
line. Click it to open the substrate's own MCP endpoint in a new tab — you
will get a 405 (it only accepts POST), which confirms the server is up.
Use the [04 — Connect](./04-connect.md) screen to point the Console at any
of these instead of the default `rulake-mcp.ruv.io`.

## Where to look in the source

| File | What it holds |
|---|---|
| `ui/src/components/screens.jsx` `AppStoreScreen` | The card data + render |
| `crates/<name>/` | Each substrate's Rust crate |
| `crates/mcp-<name>/` | The matching MCP companion server |
| `docs/adrs/ADR-NNN-*.md` | Design decision per substrate |
| `docs/gists/<name>-deep.md` | Long-form prose companion (2.5k+ words) |
| `docs/research/<name>/` | Benchmarks, security reviews |
