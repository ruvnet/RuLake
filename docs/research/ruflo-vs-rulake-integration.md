# ruflo + ruLake — deep integration review

Two ecosystems shipped from the same author, surfaced as Claude Code marketplaces, ~28 plugins between them. They look like overlapping but they're complementary substrates that **compose**. This document maps every overlap zone, identifies the integration sweet spots, and lists the concrete next-step PRs.

## TL;DR

| Axis | ruflo | ruLake |
|---|---|---|
| **Plugins** | 20 | 8 |
| **Surfaces** | 43 skills · 26 commands · 24 agents | 2 skills · 32 commands · 0 agents |
| **MCP wires bundled per-plugin** | **0** (relies on global `claude-flow` MCP) | **3 stdio wires** (`rulake`, `rvdna-mcp`, `ruqu-mcp`) bundled into rulake-stack |
| **What it owns** | Agent **orchestration** — swarms, /loop, autopilot, intelligence, safety, dev tooling | Witness-anchored **data/retrieval** — vector cache, federated substrates, kernel plane, cryptographic receipts |
| **Memory contract** | "store + semantic search" (HNSW, AgentDB, RVF) | "store + cryptographic receipt that two agents see byte-identical answer" (SHAKE-256 witness) |
| **Loop contract** | General-purpose autonomous task loops + cron workers | Vector-specific workers — incremental indexing, refresh-from-bundle, drift watchdog |
| **Trust posture** | Code/dep security + AI safety (prompt-injection, PII) | Data integrity (witness chain, AllowList, capability tiers) |

**The composition narrative**: ruflo agents + rulake substrate. ruflo gives the agent a **brain** (memory + planning + safety). ruLake gives the agent a **nervous system** (witness-anchored shared cognition that survives across hosts).

## Architecture difference (load-bearing)

ruflo plugins are **MCP-server-less**. None of the 20 plugins bundle a `.mcp.json`. They all consume tools from a single external `claude-flow` MCP server registered via `claude mcp add claude-flow -- npx -y @claude-flow/cli@latest`. Skills and commands are thin orchestration over that one server's hundreds of tools.

ruLake plugins bundle **real Rust MCP servers via stdio** — `rulake-mcp`, `rvdna-mcp`, `ruqu-mcp` are spawned as subprocesses by Claude Code per `.mcp.json` declarations in the plugin directory. Each is a self-contained binary with its own audit ledger, capability tiers, witness chain, and rate limits.

**Implication**: ruflo's surface scales horizontally (add a new skill, no new server); ruLake's surface scales vertically (add a new substrate, ship a new binary). They don't compete because they answer different questions:

- ruflo: "what should the agent do next, and how does it learn?"
- ruLake: "what data is the agent allowed to see, and is the answer the same one another agent on another host would get?"

## Overlap zones — five places they can collide or compose

### 1. Memory (overlap: high; conflict: zero; integration potential: highest)

| Plugin | What it stores | What it returns |
|---|---|---|
| `ruflo-rag-memory` | text + ONNX 384-dim vector embeddings | semantic search results from AgentDB |
| `ruflo-agentdb` | AgentDB primitives (HNSW, causal graphs, hierarchical recall) | structured query results |
| `ruflo-rvf` | RVF portable session snapshots | restored session state |
| `rulake-memory` | (key, content) pairs through `rulake_publish_bundle` | content + decision_trace + SHAKE-256 witness |

**Integration**: `rulake-memory` could route its underlying storage through `ruflo-agentdb` instead of the in-process `LocalBackend`. The rulake witness layer wraps the agentdb HNSW search; agents get both fast semantic recall AND the cryptographic receipt that says "two agents see the same answer."

**The concrete code change** would be a `BackendAdapter` impl in `crates/agentdb-backend/` that proxies pull_vectors to ruflo-agentdb's vector search. ~200 lines.

**The opposite direction** also makes sense: ruflo-rag-memory's `recall` skill could optionally hand the recalled bytes through `rulake_verify` to confirm they haven't drifted since the last `remember`.

### 2. Loop / background workers (overlap: medium; conflict: zero; integration: clean)

ruflo provides the **general** loop machinery:
- `ruflo-loop-workers` — schedule (CronCreate), persistent workers
- `ruflo-autopilot` — autonomous /loop with prediction + outcome logging
- `ruflo-goals` — long-horizon GOAP planning across sessions

ruLake provides **vector-specific** loop tasks:
- `rulake-loop-vector` — incremental indexing, refresh-from-bundle, witness-mismatch refuse-and-replan

**Integration**: `rulake-loop-vector` should ship as a worker dispatched by `ruflo-loop-workers` rather than a parallel /loop runtime. The rulake plugin currently ships empty (`docs/research/...marketplace.md` review noted: "promises three /loop skills but the directory contains only `.claude-plugin/` and `README.md`"). Filling it as ruflo-worker handlers closes both gaps in one PR.

### 3. Security (overlap: low; conflict: zero; composition: directional)

ruflo:
- `ruflo-security-audit` — code/dep scan, CVE check
- `ruflo-aidefence` — prompt-injection defense, PII detection in inputs

ruLake:
- `rulake-witness` — SHAKE-256 bundle verification, witness drift refusal
- `rulake-mcp` AllowList + capability tiers (auth boundary)

**Pipeline**: agent prompt → `ruflo-aidefence:safety-scan` → `rulake_query` → `rulake-witness:rulake-verify` on the response → consumer.

ruflo guards the IN; ruLake guards the OUT. Neither tries to do the other's job.

**One real bug to fix here** (from the recent 7-agent review): the public rulake-mcp.ruv.io demo runs `--auth none --insecure-allow-no-auth --capabilities read,publish,admin` and the AllowList check is missing on 5 mutation tools at `crates/mcp-server/src/server.rs:434, 471, 515, 552, 587`. ruflo-aidefence won't catch that — it's a server-side authz bug. Highest-priority fix in either ecosystem right now.

### 4. Intelligence / learning (overlap: medium; integration: bidirectional)

ruflo:
- `ruflo-intelligence` — SONA neural patterns, model routing, trajectory learning
- `ruflo-daa` — cognitive patterns, knowledge sharing
- `ruflo-autopilot` — learns which actions worked

ruLake:
- `rulake-memory:memory-tune` — recommends Consistency settings (Fresh/Eventual/Frozen) per collection from observed access patterns
- `rulake-memory:memory-replay` — re-issue past queries, surface witness drift
- `rulake-memory:memory-status` — hit-ratio dashboard

**The two learning loops are different levels**:
- ruflo learns at the **agent layer** (which model, which pattern, which tool to pick)
- ruLake learns at the **data layer** (which collection to pin warm, which consistency mode is right, which queries are drifting)

**Integration**: ruflo-intelligence's trajectory-step could log `decision_trace` blocks from rulake_query as training signal. The chosen_path / cost / cache.hit / witness.match fields in `decision_trace` (per ADR-009 §"Cost-aware retrieval") are exactly the kind of structured outcome data ruflo-intelligence's pattern store wants.

### 5. Swarm + multi-agent (overlap: zero; composition: required for both to work at scale)

ruflo:
- `ruflo-swarm` — multi-agent teams, monitor streams, worktree isolation
- `ruflo-workflows` — visual workflow templates with parallel execution

ruLake: no swarm primitives. But `rulake-memory:memory-compact` distills warm-cache + audit into a portable bundle that survives session boundaries — exactly what a ruflo swarm needs to share state across agents.

**The composition**: ruflo-swarm spawns N agents each with their own Agent-tool context. They all `/rulake-memory:memory-recall` against the same shared memory. When the swarm finishes, one of them runs `/rulake-memory:memory-compact` and pins the witness — the next swarm session starts by `/rulake-memory:memory-promote`-ing that bundle and inherits the warmth.

**Without ruLake**, the swarm has no way to share verifiable state. **Without ruflo**, ruLake has no agents to coordinate. They genuinely complete each other on this axis.

## Gaps each ecosystem fills for the other

### ruflo gives ruLake (the things ruLake can't do alone)

- Multi-agent swarm orchestration (`ruflo-swarm`)
- General-purpose /loop scheduling that survives session restart (`ruflo-loop-workers`, `ruflo-autopilot`)
- Code-level security audit (`ruflo-security-audit`)
- AI safety on inputs (`ruflo-aidefence`)
- Test generation w/ coverage analysis (`ruflo-testgen`)
- Doc generation w/ drift detection (`ruflo-docs`)
- Browser automation (`ruflo-browser`)
- WASM sandboxing for untrusted code (`ruflo-wasm`)
- Local LLM inference (`ruflo-ruvllm`)
- Long-horizon goal planning (`ruflo-goals`)
- Plugin scaffolding (`ruflo-plugin-creator`)

### ruLake gives ruflo (the things ruflo can't do alone)

- **Witness-anchored data integrity** — cryptographic receipts that prove two agents on two machines see the same answer
- **Federated retrieval over operator's existing storage** — no need to move data into AgentDB; query GCS/IPFS/Parquet/files in place
- **Quantum simulation backend** (`rulake-ruqu` — real `ruqu_core::Simulator` ~26k LOC)
- **Genomic data substrate** (`rulake-rvdna` — real codon tables, brute-force kNN)
- **Hardware-acceleration plane** (`rulake-kernels` — AVX-512 + wgpu portable GPU)
- **The "deterministic retrieval path" named flow** — a workflow template ruflo-autopilot could record once and replay
- **Cost-in-relative-units economic-routing telemetry** in every response (`decision_trace.cost`)

## Concrete integration PRs (priority order)

### PR 1: `rulake-memory` README + skill points operators at `ruflo-rag-memory` for backing storage

One paragraph in `plugins/rulake-memory/README.md` saying: "for persistent storage that survives Claude Code restarts, install `ruflo-rag-memory` and configure rulake-memory's backend to AgentDB." Cost: 15 lines. Value: turns rulake-memory from in-process working memory into a real session-spanning store.

### PR 2: Fill `rulake-loop-vector` as ruflo workers (kills "ships empty" finding from recent review)

Convert the 3 promised /loop skills (`incremental indexing`, `refresh-from-bundle`, `witness-mismatch refuse-and-replan`) into worker handlers dispatched via `ruflo-loop-workers:cron-schedule`. Same skills + machinery + observability. Cost: ~150 lines of skill markdown + a worker handler. Value: closes the empty-plugin gap AND gives ruflo a real vector-aware worker class.

### PR 3: `ruflo-aidefence:safety-scan` documented as a wrapper for `rulake_query` inputs

A skill at `ruflo-aidefence/skills/rulake-safe-query/SKILL.md` that auto-invokes when the agent is about to call `rulake_query`. Pre-scans the query payload for prompt-injection / PII before forwarding. Cost: 1 skill markdown. Value: closes the input-side trust gap.

### PR 4: `decision_trace` block surfaced as `ruflo-intelligence` trajectory signal

One adapter in `ruflo-intelligence` that consumes `decision_trace.{chosen_path, cost, cache.hit, witness.match}` from rulake_query responses and writes them as `hooks_intelligence_trajectory-step` records. Closes the "every loop iteration should record decisions" gap from `~/CLAUDE.md` §6 "Memory + intelligence wiring for /loop." Cost: ~30 lines.

### PR 5: `rulake-discover` recommends ruflo plugins too

Already exists at `plugins/rulake-core/commands/rulake-discover.md`. Currently catalogs 8 ruLake plugins. Extend to also list complementary ruflo plugins: "agentic memory needs ruflo-rag-memory for persistence; multi-agent coordination needs ruflo-swarm; long-running tasks need ruflo-autopilot." Cost: 40 lines added to the existing file. Value: makes rulake-discover the single advisory entry-point spanning both ecosystems.

### PR 6: Joint `marketplace.json` curated bundle

A new `rulake-+-ruflo-stack` plugin in the rulake marketplace that lists ruflo plugin install commands as part of its README. One install ergonomic, two ecosystems composed.

## What both ecosystems should NOT do

**Don't** merge ruflo's MCP-less skill-orchestration model with ruLake's stdio-bundled-binary model. They've diverged for different reasons:

- ruflo's skills are pure prompt orchestration over a global tool surface. Authoring is `vim file.md`, distribution is `git push`. Velocity > control.
- ruLake's bundled MCPs are real Rust binaries with audit + auth + witness. Authoring is `cargo build`, distribution requires `cargo install`. Control > velocity.

Forcing ruflo to bundle MCPs would slow it down by 10x. Forcing ruLake to share a global MCP would lose its capability tiers. **The architecture asymmetry is the right call.** The integration narrative is "compose at the agent level, not the package level."

## Closing observation

Looking at the actual session state right now: the user has both marketplaces installed, the `mcp__plugin_rulake-stack_*` MCP tools are loaded, and the ruflo skill list shows ~70 ruflo skills/commands available. **The hard part — making both work in the same Claude Code session — already works.** What's left is the ergonomic glue (PRs 1-6 above) so operators don't have to discover the composition themselves by reading two marketplaces.

The two marketplaces should ship as **siblings**, not competitors. The cleanest signal of that is making `rulake-discover` recommend ruflo plugins alongside its own catalog (PR 5) — turn discovery into a single front door.
