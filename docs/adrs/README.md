# Architecture Decision Records — ruLake

The decisions, ratified, that shape this project. Each ADR records a single
choice, the alternatives considered, and the load-bearing trade-offs. ADRs
that ship implementations carry an updated `Status:` line that points at
the merged code; design-only ADRs stay `Proposed` because the contract IS
the deliverable.

Every shipped ADR has a 2,500–3,700-word narrative companion in
[`../gists/`](../gists/) — the ADR is the contract, the gist is the
prose walkthrough.

## Status snapshot (2026-04-27)

| ADR | Title | Status | Code | Gist |
|---|---|---|---|---|
| [001](ADR-001-standalone-repo-strategy.md) | Standalone repo strategy | **Accepted** | repo root | [standalone-repo-deep.md](../gists/standalone-repo-deep.md) |
| [002](sdk/ADR-002-python-sdk.md) | Python SDK — PyO3 + ABI3 wheels | **Accepted v2.2** | [`python/`](../../python/) | [python-sdk-deep.md](../gists/python-sdk-deep.md) |
| [003](sdk/ADR-003-nodejs-typescript-sdk.md) | Node SDK — napi-rs + ESM-first | **Accepted v2.2** | [`node/`](../../node/) | [node-sdk-deep.md](../gists/node-sdk-deep.md) |
| [004](sdk/ADR-004-rulake-mcp-server.md) | MCP server — Streamable HTTP, JWT scopes | **Accepted v0.10** | [`crates/mcp-server/`](../../mcp-server/) | [mcp-server-deep.md](../gists/mcp-server-deep.md) |
| [005](sdk/ADR-005-ipfs-backend-and-deploy.md) | IPFS backend — CIDv1 + kubo + gateway-fallback | **Accepted v0.1** | [`crates/ipfs-backend/`](../../ipfs-backend/) | [ipfs-backend-deep.md](../gists/ipfs-backend-deep.md) |
| [006](ADR-006-rulake-console-vite-github-pages.md) | Console — Vite + React + GitHub Pages | **Accepted** | [`ui/`](../../ui/) | [console-deep.md](../gists/console-deep.md) |
| [007](ADR-007-rvdna-as-rulake-substrate.md) | rvDNA v2 — genomic substrate | **Accepted — Scaffolded v0.0.1** | [`crates/rvdna-backend/`](../../rvdna-backend/) + [`crates/mcp-rvdna/`](../../mcp-rvdna/) | [rvdna-v2-deep.md](../gists/rvdna-v2-deep.md) |
| [008](ADR-008-ruqu-as-rulake-substrate.md) | ruQu v2 — quantum execution substrate | **Accepted — Scaffolded v0.0.1** | [`crates/ruqu-backend/`](../../ruqu-backend/) + [`crates/mcp-ruqu/`](../../mcp-ruqu/) | [ruqu-v2-deep.md](../gists/ruqu-v2-deep.md) |
| [155](ADR-155-rulake-datalake-layer.md) | Datalake layer — vector-native federation intermediary | **Accepted M3** | core + 4 backends + 3 MCP servers | [datalake-layer-deep.md](../gists/datalake-layer-deep.md) |
| [156](ADR-156-rulake-as-memory-substrate.md) | ruLake as memory substrate for agent brains | **Accepted (positioning ratified)** | README + Console + gists | [memory-substrate-deep.md](../gists/memory-substrate-deep.md) |
| [157](ADR-157-optional-accelerator-plane.md) | Optional accelerator plane — `VectorKernel` trait | **Proposed** *(scaffolding-only)* | — | — |
| [158](ADR-158-optional-rotation-and-qvcache-positioning.md) | Optional rotation kind + QVCache positioning | **Proposed** *(knob-locking)* | — | — |

ADRs 157 and 158 explicitly note "no code changes accompany this ADR" as
part of their decision — the contract IS the deliverable. They'll get
`Accepted` + a gist when implementations ship.

## Layout

```
docs/adrs/
├── README.md                                          # this file
├── ADR-001-standalone-repo-strategy.md                # canonical (no subdir)
├── ADR-006-rulake-console-vite-github-pages.md        # canonical
├── ADR-007-rvdna-as-rulake-substrate.md               # canonical
├── ADR-008-ruqu-as-rulake-substrate.md                # canonical
├── ADR-155-rulake-datalake-layer.md                   # canonical
├── ADR-156-rulake-as-memory-substrate.md              # canonical
├── ADR-157-optional-accelerator-plane.md              # canonical
├── ADR-158-optional-rotation-and-qvcache-positioning.md  # canonical
└── sdk/
    ├── ADR-002-python-sdk.md                          # SDK family
    ├── ADR-003-nodejs-typescript-sdk.md
    ├── ADR-004-rulake-mcp-server.md
    └── ADR-005-ipfs-backend-and-deploy.md
```

The `sdk/` subdirectory is historical (ADR-002–005 were originally drafted
as one batch addressing the public-facing SDK + server surface). New ADRs
land at the top level.

## Numbering convention

- **001–099** — core project shape (repo strategy, SDKs, server, backends).
  Single-digit ADRs in the 1xx range pre-date the substrate-tier framing.
- **155–158** — the datalake / memory / accelerator / rotation strategic
  family (drafted together in April 2026 as a coherent positioning batch).
- **Substrate ADRs** (007, 008) carry the same number as their canonical
  position in the project; the v2 file-format-style substrates picked their
  numbers to avoid collision with the 1xx series.

## Reading order if you're new

1. **[ADR-001](ADR-001-standalone-repo-strategy.md)** — what kind of project this is (standalone, no workspace, submodule pin).
2. **[ADR-155](ADR-155-rulake-datalake-layer.md)** — what the project does (vector-native federation intermediary).
3. **[ADR-156](ADR-156-rulake-as-memory-substrate.md)** — who it's for (agent brains).
4. **[ADR-004](sdk/ADR-004-rulake-mcp-server.md)** — how agents reach it (MCP over HTTP).
5. **[ADR-006](ADR-006-rulake-console-vite-github-pages.md)** — the operator's view (Console).
6. Pick a substrate: **[ADR-007](ADR-007-rvdna-as-rulake-substrate.md)** (genomic) or **[ADR-008](ADR-008-ruqu-as-rulake-substrate.md)** (quantum) for the trust-chain story end-to-end.
