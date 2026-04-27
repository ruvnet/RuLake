# Deep gists — narrative companions to the ADRs

Each shipped ADR has a 2,500–3,700-word narrative companion in this
directory. The ADR itself is the contract — the gist is the prose
walkthrough you'd hand a new collaborator who needs to understand
*why* a decision was made, not just what the decision is.

All gists follow the same nine-section template (TL;DR · Introduction
· Decision in detail · Capabilities · Trust contract · Reference
implementation status · Composition with the rest of ruLake · Open
questions · References) and use absolute `file:line` citations so a
reader can land cold and follow along in the source.

## By ADR family

### Foundational (the project shape)

| ADR | Gist | Words | What it covers |
|---|---|---|---|
| [ADR-001](../adrs/ADR-001-standalone-repo-strategy.md) | [standalone-repo-deep.md](standalone-repo-deep.md) | 2,822 | Why ruLake stands as its own repo (vs. living inside `RuVector/crates/`); submodule pin, no root workspace, concrete dep versions. |
| [ADR-002](../adrs/sdk/ADR-002-python-sdk.md) | [python-sdk-deep.md](python-sdk-deep.md) | 3,374 | PyO3 bindings, NumPy zero-copy, ABI3 wheels; the `rulake-py` cdylib at `python/`. |
| [ADR-003](../adrs/sdk/ADR-003-nodejs-typescript-sdk.md) | [node-sdk-deep.md](node-sdk-deep.md) | 3,595 | napi-rs binding, Float32Array zero-copy, ESM-first with .mjs/.cjs dual export; the `rulake-node` cdylib at `node/`. |

### Server-tier (how callers reach the lake)

| ADR | Gist | Words | What it covers |
|---|---|---|---|
| [ADR-004](../adrs/sdk/ADR-004-rulake-mcp-server.md) | [mcp-server-deep.md](mcp-server-deep.md) | 3,707 | Streamable HTTP MCP, JWT scopes → CapabilitySet, JSONL audit + 256-entry tail buffer, the eight tool surface. |
| [ADR-006](../adrs/ADR-006-rulake-console-vite-github-pages.md) | [console-deep.md](console-deep.md) | 3,186 | The Vite + React Console at `ui/`, tri-mode (Demo / WASM-local / Live MCP), 7 routes including App Store. |

### Storage-tier (where the data lives)

| ADR | Gist | Words | What it covers |
|---|---|---|---|
| [ADR-005](../adrs/sdk/ADR-005-ipfs-backend-and-deploy.md) | [ipfs-backend-deep.md](ipfs-backend-deep.md) | 2,732 | Witness-anchored bundle distribution by CIDv1 over kubo + gateway-fallback; the R-IPFS-1 hard-refuse on data_ref ↔ CID mismatch. |
| [ADR-155](../adrs/ADR-155-rulake-datalake-layer.md) | [datalake-layer-deep.md](datalake-layer-deep.md) | 2,774 | The vector-native federation-intermediary positioning; M1 → M2 → M3 ship table; the BackendAdapter trait as the plug point. |

### Substrate-tier (specialized data shapes)

| ADR | Gist | Words | What it covers |
|---|---|---|---|
| [ADR-007](../adrs/ADR-007-rvdna-as-rulake-substrate.md) | [rvdna-v2-deep.md](rvdna-v2-deep.md) | 2,804 | The `.rvdna` v2 file format (8 sections, bundle pointer at byte 0x00B0); raw DNA + embeddings + variants + protein + epigenomic series; T0/T1/T2 tier roadmap. |
| [ADR-008](../adrs/ADR-008-ruqu-as-rulake-substrate.md) | [ruqu-v2-deep.md](ruqu-v2-deep.md) | 3,054 | Five quantum simulation backends (StateVector / Stabilizer / Clifford+T / TensorNetwork / Hardware); witness-anchored execution; cross-process replay. |

### Positioning

| ADR | Gist | Words | What it covers |
|---|---|---|---|
| [ADR-156](../adrs/ADR-156-rulake-as-memory-substrate.md) | [memory-substrate-deep.md](memory-substrate-deep.md) | 3,174 | What makes ruLake a good memory layer for agentic systems; the witness-anchored cache as the trust anchor for cross-agent state. |

## ADRs without gists

[ADR-157](../adrs/ADR-157-optional-accelerator-plane.md) (Optional
Accelerator Plane / VectorKernel) and
[ADR-158](../adrs/ADR-158-optional-rotation-and-qvcache-positioning.md)
(Rotation Kind / QVCache positioning) are pure design contracts that
explicitly note "no code changes accompany this ADR" as part of their
decision. The ADR itself is the deliverable; a gist would just
restate it. They'll get gists when they ship implementations.

## Total

**~31,000 words** across 10 gists, distilled from the ADRs + research
notes + source code. Reading order if you're new to the project: start
with [`standalone-repo-deep.md`](standalone-repo-deep.md) (what kind
of project this is), then [`datalake-layer-deep.md`](datalake-layer-deep.md)
(what it does), then [`mcp-server-deep.md`](mcp-server-deep.md) (how
agents call into it), then pick a substrate
([`rvdna-v2-deep.md`](rvdna-v2-deep.md) or
[`ruqu-v2-deep.md`](ruqu-v2-deep.md)) for a deep dive on the
trust-chain story.
