# ADR-009: ruLake as a Claude Code Plugin Marketplace + GitHub Pages Storefront

## Status

**Proposed (revised 2026-04-27)** — discovery + distribution + positioning
surface for ruLake. The first revision was packaging; this revision adds
the killer-path plugin (`rulake-stack`), the cost-aware response shape on
`rulake_query`, the named default flow ("deterministic retrieval path"),
and the answer to "invisible infrastructure or developer-facing product?"
(both, at different layers). No new *core* code; what's new are install
ergonomics, response-field surface, and a documented happy path.

## One-line positioning

> **ruLake turns retrieval into a first-class, installable primitive for agents.**

Agents (Claude Code, Cursor, Cline, Continue, agentic-flow, OpenAI Apps,
Replit Agent, Flow Nexus) get one MCP tool — `rulake_query` — that takes
intent + freshness budget + policy and returns a witness-anchored result
plus the cost / latency / trust trace it took to produce it. Plugins are
how operators install that primitive into their Claude Code session in
under 60 seconds.

## Date

2026-04-27

## Authors

ruv.io · session-pattern transferred from the ruvnet/ruflo marketplace
proposal (the same Claude-Code-native plugin shape, retargeted to
ruLake's witness-anchored vector-federation capabilities).

## Relates To

- ADR-001 — standalone repo strategy (each plugin is a sibling crate
  or static asset; no workspace coupling)
- ADR-004 — `rulake-mcp` server (the load-bearing plugin: `rulake-core`)
- ADR-005 — ipfs-backend + deploy (one of the substrate plugins)
- ADR-006 — Console + GitHub Pages (the existing Pages deploy already
  proves the static-storefront pattern)
- ADR-007 — rvDNA substrate (substrate plugin)
- ADR-008 — ruQu substrate (substrate plugin)
- ADR-157 — accelerator plane / VectorKernel (kernel plugins)

---

## Context

Claude Code now supports plugin marketplaces via `/plugin marketplace add owner/repo`, which reads a `.claude-plugin/marketplace.json` catalog and installs `.claude-plugin/plugin.json`-shaped sub-packages (`https://code.claude.com/docs/en/plugin-marketplaces`). This is the same shape Anthropic's own `claude-plugins-official` directory uses, which means it is already familiar to operators discovering Claude Code agentic tooling. ruvnet's separate `ruflo` proposal lays out one plausible packaging — `ruflo-core`, `ruflo-swarm`, `ruflo-loop-workers`, etc. — for the general agentic-orchestration story.

ruLake's surface is different from ruflo's. ruLake is **vector-data plumbing**: a witness-anchored cache (`rulake_query` and the eight tools at ADR-004 §4b), four backend adapters (`gcs`, `ipfs`, `rvdna`, `ruqu`), two accelerator kernels (`avx-512`, `wgpu` per ADR-157), and a Console (ADR-006) that exercises the wire. The natural plugin axes are therefore the substrates and accelerators an operator wants to enable, not the agent-orchestration patterns. A "ruLake substrates" plugin is a coherent install unit: it brings in the adapter, the audit-code catalog, the smoke that proves the wire, and the docs that show how to wire it into a `RuLake::register_backend` call.

Two distinct distribution surfaces serve two distinct populations:

1. **Plugin marketplace** (`/plugin marketplace add ruvnet/RuLake`) — for **agent operators** who run Claude Code, Cursor, Cline, Continue, agentic-flow, etc., and want to install ruLake as a working MCP-callable retrieval layer they can hand a query intent and a freshness budget. They install a plugin, get a `rulake_query` tool, and call it from their agent.
2. **GitHub Pages storefront** (today: `https://ruvnet.github.io/RuLake/`, the live Console) — for **discovery + documentation**: copy install commands, browse capabilities, see the live demo, click through to the source. Already exists for the Console; the marketplace proposal extends it with a per-plugin landing tile.

The repository today is shaped for the second (the Console at `ui/`, the deep gists at `docs/gists/`, the ADRs, the deploy doc). It is **not yet shaped for the first** — there is no `.claude-plugin/marketplace.json`, no per-plugin `plugin.json`, and no `.mcp.json` that wires `rulake-mcp` into Claude Code's plugin-load path. The wire-up is mechanical; what this ADR commits to is the **packaging shape** (which plugins, what they each install, how they layer) and the **trust posture** (signing, pinning, least-privilege defaults), not the contents of `marketplace.json` line-by-line.

The repo is at the right inflection. v2.3-alpha shipped on 2026-04-27 (PR #14 merged): Cloud Run deploy live at `https://rulake-mcp.ruv.io/`, `rulake-wasm@2.3.0-alpha.1` published to npm, all 11 deep gists complete, Docker E2E green for all three MCP servers, the kernel security review captured and the R-WGPU-1 cap acted on. The shipping artifacts are stable enough that wrapping them in plugin packages does not chase a moving target. The marketplace is the right next layer; this ADR locks the shape.

## Decision

**Ship ruLake as a Claude Code plugin marketplace at `ruvnet/RuLake` with a five-plugin catalog and a GitHub Pages storefront extension.** The marketplace catalog and per-plugin manifests live alongside the existing crates / sdk / docs tree; no fork, no separate repo, no workspace.

### Repo layout (additive)

```text
RuLake/
├── .claude-plugin/
│   └── marketplace.json              # NEW — plugin catalog
├── plugins/                          # NEW — per-plugin packaging
│   ├── rulake-core/
│   │   ├── .claude-plugin/plugin.json
│   │   ├── .mcp.json                 # wires rulake-mcp into Claude Code
│   │   ├── commands/                 # /rulake-* commands
│   │   ├── skills/                   # /rulake-query skill, etc.
│   │   ├── agents/                   # ruLake-shaped sub-agents
│   │   └── README.md
│   ├── rulake-substrates/
│   │   ├── .claude-plugin/plugin.json
│   │   ├── .mcp.json                 # rvdna-mcp + ruqu-mcp + adapters
│   │   ├── commands/
│   │   └── README.md
│   ├── rulake-kernels/
│   │   ├── .claude-plugin/plugin.json
│   │   ├── commands/                 # /rulake-bench, /rulake-kernel-status
│   │   └── README.md
│   ├── rulake-witness/
│   │   ├── .claude-plugin/plugin.json
│   │   ├── commands/                 # /rulake-verify, /rulake-bundle-info
│   │   └── README.md
│   └── rulake-loop-vector/
│       ├── .claude-plugin/plugin.json
│       ├── skills/                   # /loop-aware vector ops
│       ├── workers/                  # incremental indexing, refresh-from-bundle
│       └── README.md
├── crates/                           # unchanged (the actual binaries the plugins reference)
├── sdk/                              # unchanged
├── ui/                               # the existing Console — gets a /marketplace route
└── docs/                             # unchanged
```

The plugins **do not duplicate code**. Each plugin's `.mcp.json` references the binaries that already live in `crates/*/target/release/` (or, for production deploys, the public Cloud Run URL `https://rulake-mcp.ruv.io/`). A plugin install is a declaration that "this Claude Code session knows about these tools"; it is not a code transfer.

### The six plugins (one killer-path + five composable)

| Plugin | Role | Wraps | Default MCP wire |
|---|---|---|---|
| **`rulake-stack`** | **Killer path — the 90% case.** One install gets retrieval working end-to-end. Bundles `rulake-core` + `rulake-substrates` + `rulake-witness` with sensible defaults; `rulake-kernels` and `rulake-loop-vector` stay opt-in. | composition of the four below | inherits `rulake-core`'s default |
| `rulake-core` | The load-bearing plugin. Installs the eight `rulake_*` tools (ADR-004 §4b) plus the `/rulake-query` decision-layer skill. | `crates/{mcp-server,core}/` | `https://rulake-mcp.ruv.io/` (public demo) or local stdio |
| `rulake-substrates` | The four backend adapters and their two companion MCP servers. Installs `rvdna_*` + `ruqu_*` tools (5 each, ADR-007/008). | `crates/{rvdna,ruqu,gcs,ipfs}-backend/` + `crates/{mcp-rvdna,mcp-ruqu}/` | `https://{rvdna,ruqu}-mcp.ruv.io/` |
| `rulake-kernels` | The ADR-157 accelerator plane. Off-by-default, operator opt-in. Installs `/rulake-kernel-bench` + `/rulake-kernel-status` commands surfacing the `KernelCapabilities` and the R-WGPU-1 security cap. | `crates/kernel-{avx512,wgpu}/` | none — kernels run in-process |
| `rulake-witness` | The trust-anchored bundle ergonomics. Installs `/rulake-verify <bundle>`, `/rulake-bundle-info`, and the witness viewer's CLI counterpart. | `crates/core/src/{bundle,witness}.rs` | none — verification is local |
| `rulake-loop-vector` | `/loop`-aware vector workers: incremental indexing from a backend, refresh-from-bundle on schedule, witness-mismatch refuse-and-replan. The agentic-flow integration. | `scripts/smoke-*.sh` patterns + ADR-005 workers | uses `rulake-core` + `rulake-substrates` |

The killer-path install is intentional: a new operator who wants to feel
the value of ruLake should not be making a five-plugin shopping decision.
`/plugin install rulake-stack` gets them a retrieval primitive in one
command. The composable plugins exist for operators who already know
they want a specific subset (kernel-only on a CI runner; substrates-only
on a worker that doesn't itself answer queries; loop-vector-only when
folding into an agentic-flow workflow).

### The "deterministic retrieval path" — the named default flow

Every install of `rulake-stack` ships with one opinionated default flow,
named so it can be referenced + reasoned about:

```
intent  →  cache check  →  substrate fanout  →  mincut prune  →  witness verify  →  return
```

| Step | What happens | Where it lives |
|---|---|---|
| **intent** | Parse `rulake_query`'s intent (search / verify / explain / refresh), risk, freshness budget, target collection set. | `crates/mcp-server/src/server.rs` `rulake_query` handler |
| **cache check** | Hit the `RuLake` in-process cache. ~1 ms cache-hit at n=100k, D=128 (1.02× direct RaBitQ). If hit + within freshness budget → skip to witness verify. | `crates/core/src/cache.rs` |
| **substrate fanout** | If miss, fan out to the registered `BackendAdapter`s in parallel (federation graph). Each adapter returns its own witnessed bundle pointer. | `crates/core/src/backend.rs` + the four substrate crates |
| **mincut prune** | Apply the contrastive-retrieval primitive: rank not by "find similar" but by "find boundary-defining" (the mincut-pruned candidate set that maximally distinguishes the query from the corpus). This is the unique surface ruLake exposes that no other retrieval layer ships cleanly today. | future — `crates/core/src/select.rs` (v2.4 roadmap) |
| **witness verify** | Recompute SHAKE-256(32) over the returned bundle; compare to the substrate-supplied witness. On mismatch, **refuse with `WITNESS_MISMATCH_REFUSED`** — never serve stale data with a high score. | `crates/core/src/witness.rs` |
| **return** | Result + `decision_trace` (see "Cost-aware retrieval" below) + audit row emitted to JSONL sink. | `crates/mcp-server/src/server.rs` + `audit.rs` |

The whole path is the default contract of `rulake_query`. An operator who
wants a different ordering (e.g. "skip mincut," "skip cache, always pull
fresh") sets the corresponding policy flag in the call. The point of
naming the path is that it becomes a first-class object — the docs, the
audit trace, and the metrics all reference "deterministic retrieval path"
as the unit of measurement.

### Cost-aware retrieval — the response shape

Every `rulake_query` response carries a `decision_trace` block alongside
the data. This is the structural answer to "should `rulake_query` expose
policy inputs?" — it does, and it also exposes the policy *outputs* it
chose:

```jsonc
{
  "result": [ /* the actual ranked retrieval */ ],
  "decision_trace": {
    "chosen_path": "deterministic-retrieval-path",
    "intent": "search",
    "freshness": { "budget_ms": 5000, "actual_ms": 1031 },
    "cache": { "hit": true, "hit_ratio_session": 0.87 },
    "substrates_used": ["gcs-backend", "rvdna-backend"],
    "kernel": { "id": "cpu-naive", "deterministic": true },
    "witness": { "expected": "...32-byte hex...", "computed": "...32-byte hex...", "match": true },
    "cost": {
      "compute_kernel": 0.0,
      "backend_fetch": 0.0,
      "cache_hit_discount": -1.0,
      "currency": "relative-units",
      "comment": "Free + open source — costs are relative-units used by the dispatch policy, not USD"
    },
    "latency": {
      "total_ms": 1.02,
      "cache_ms": 0.4,
      "fanout_ms": 0.0,
      "witness_ms": 0.6
    },
    "refusals": []
  }
}
```

The `cost` block is **economic routing telemetry**, not billing. Even
though ruLake is free + MIT/Apache, the dispatch policy uses relative
cost units to decide which substrate / kernel to pick — and surfacing
those numbers to the agent caller is what lets a calling agent make
informed `rulake_query` calls (e.g. "if cost > X, narrow the query and
retry"). This is the missing layer that turns retrieval from an opaque
black box into a primitive an agent can negotiate with.

### Marketplace catalog (`.claude-plugin/marketplace.json`)

```json
{
  "name": "rulake-marketplace",
  "owner": { "name": "ruvnet" },
  "description": "Witness-anchored vector federation for agentic AI — installable as Claude Code plugins.",
  "default_install": "rulake-stack",
  "plugins": [
    {
      "name": "rulake-stack",
      "source": "./plugins/rulake-stack",
      "description": "The 90% install. Bundles rulake-core + rulake-substrates + rulake-witness with the deterministic retrieval path enabled and kernels off by default.",
      "default": true
    },
    {
      "name": "rulake-core",
      "source": "./plugins/rulake-core",
      "description": "The eight rulake_* MCP tools, the rulake_query decision layer, and the witness-anchored cache."
    },
    {
      "name": "rulake-substrates",
      "source": "./plugins/rulake-substrates",
      "description": "Four backend adapters (rvdna, ruqu, gcs, ipfs) + two companion MCP servers (rvdna-mcp, ruqu-mcp)."
    },
    {
      "name": "rulake-kernels",
      "source": "./plugins/rulake-kernels",
      "description": "ADR-157 accelerator plane: AVX-512 host SIMD + wgpu portable GPU. Off-by-default; operator opt-in."
    },
    {
      "name": "rulake-witness",
      "source": "./plugins/rulake-witness",
      "description": "SHAKE-256(32) bundle verification + witness viewer CLI."
    },
    {
      "name": "rulake-loop-vector",
      "source": "./plugins/rulake-loop-vector",
      "description": "/loop-aware vector workers — incremental indexing, refresh-from-bundle, witness-mismatch refuse-and-replan."
    }
  ]
}
```

The "first-query-in-60-seconds" success metric:

```text
1. /plugin marketplace add ruvnet/RuLake          ~5 s
2. /plugin install rulake-stack                   ~10 s
3. /rulake-query "what does ADR-157 commit to?"   ~1 s
```

The result returns with the data + the `decision_trace` (cache hit?
which substrate? what witness?). If that loop is under 60 s wall-clock
on a fresh Claude Code install, the killer path has succeeded.

### Per-plugin manifest example (`plugins/rulake-core/.claude-plugin/plugin.json`)

```json
{
  "name": "rulake-core",
  "version": "2.3.0-alpha.1",
  "description": "Witness-anchored vector federation cache + the eight rulake_* MCP tools.",
  "homepage": "https://ruvnet.github.io/RuLake/",
  "repository": "https://github.com/ruvnet/RuLake",
  "license": "MIT OR Apache-2.0"
}
```

### MCP wire (`plugins/rulake-core/.mcp.json`)

Two profiles — public demo (no install required) vs local stdio:

```json
{
  "mcpServers": {
    "rulake": {
      "type": "http",
      "url": "https://rulake-mcp.ruv.io/",
      "comment": "Public demo. No backends wired; read-only. For real data, switch to the stdio profile."
    },
    "rulake-stdio": {
      "type": "stdio",
      "command": "rulake-mcp",
      "args": ["stdio", "--capabilities", "read,publish,admin"],
      "comment": "Requires the `rulake-mcp` binary on PATH (cargo install --path crates/mcp-server)."
    }
  }
}
```

### GitHub Pages storefront (`ui/` extension)

The existing Console at `https://ruvnet.github.io/RuLake/` gains a `/marketplace` route:

| Route | Content |
|---|---|
| `/` | The existing 7-route Console (Stats, App Store, Connect, Browse, Bundle, Playground, Audit) |
| `/marketplace` | Per-plugin landing tiles, install-command copy widgets, security/trust model, link to GitHub source |
| `/marketplace/<plugin>` | Plugin detail page — capabilities, install command, MCP wire, screenshot, "try it in the live demo" link |

Install commands shown on the storefront (literal):

```text
/plugin marketplace add ruvnet/RuLake
/plugin install rulake-core@rulake-marketplace
/plugin install rulake-substrates@rulake-marketplace
/plugin install rulake-kernels@rulake-marketplace
/plugin install rulake-witness@rulake-marketplace
/plugin install rulake-loop-vector@rulake-marketplace
```

The storefront is **discovery + copy/paste**, not the installer. The actual install is Claude Code's `/plugin install` command reading the marketplace.json on GitHub directly.

### Trust + security posture

Plugin contents are not auto-verified by Claude Code (per `anthropics/claude-plugins-official`'s own warning). ruLake's plugins must:

1. **Pin versions in `marketplace.json`** — the catalog references plugin sources by directory path, but each plugin's manifest carries an explicit `version` field that Claude Code surfaces at install time. Operators can install `@version` to lock.
2. **Default to the most-restricted capabilities** — `rulake-core`'s `.mcp.json` ships with `--capabilities read` (not `read,publish,admin`). Operators wanting mutation must override explicitly.
3. **Document the public-demo caveat** — the `https://rulake-mcp.ruv.io/` profile is `--auth none --insecure-allow-no-auth` per the deploy recipe; plugin docs must say "for production with real data, switch to the stdio profile or a JWT-protected HTTP profile."
4. **Tag releases** — use `git tag plugin-v<plugin>-<version>` so operators can pin against tagged releases.
5. **Run `gh plugin sign`** when Anthropic ships plugin signing (currently not available; this is a forward commitment).
6. **Surface the audit-row contract** — every plugin that wires an MCP server makes the operator aware that mutations emit JSONL audit rows (ADR-004 §M4) and where they land.

### CI for marketplace.json

A new `.github/workflows/validate-marketplace.yml` validates on every push:

- `marketplace.json` parses
- Every `source:` path exists in the repo
- Every plugin has a `.claude-plugin/plugin.json` with required fields (`name`, `version`, `description`)
- Every plugin's `version` matches the parent crate's `Cargo.toml` version (consistency with the underlying binary)
- The literal install commands shown on the storefront resolve via `gh api repos/ruvnet/RuLake/contents/plugins/<name>` (i.e. the storefront doesn't drift from the catalog)

## Alternatives considered

### A. Single mega-plugin (`rulake-everything`)

Rejected. Operators wanting just `rulake_query` would be forced to install every backend adapter, every kernel, the witness CLI, and the loop workers. The capability-gated CLI surface (ADR-004 §4b) already proves operators want fine-grained opt-in; the plugin shape should mirror that.

### B. One plugin per crate (10+ plugins)

Rejected. The five-plugin shape buckets capabilities the way operators install them — "I want the substrates" or "I want the accelerators" — not by source-tree boundary. A 10-plugin marketplace fragments the install story without giving operators a useful new axis.

### C. Separate repo (`ruvnet/rulake-marketplace`)

Rejected. The marketplace catalog references binaries that live in `ruvnet/RuLake`; a separate repo would either (a) duplicate the source, (b) submodule it, or (c) reference release artifacts only. All three violate ADR-001's "one source, one truth, no workspace" principle. The marketplace is metadata + packaging; it lives next to the source.

### D. Static marketplace JSON without per-plugin folders

A minimal `marketplace.json` with no `plugins/*/` directory — the catalog references binaries directly. Rejected because Claude Code's plugin-load path expects each plugin to be a directory with its own `.claude-plugin/plugin.json`, `commands/`, `agents/`, `skills/`, etc. Skipping the per-plugin folders breaks the install contract.

### E. GitHub Pages as the installer (not just the storefront)

Rejected. Claude Code's installer is `/plugin install` — running over the GitHub raw content of `marketplace.json`. A Pages-based installer would either (a) shell out to `gh` from the static page (impossible in a browser), (b) ship a backend service (pulls a deploy + a runtime), or (c) reinvent the install protocol. Pages stays discovery-only; install is Claude-Code-native.

## Consequences

### Positive

- **One install command per user role.** A retrieval-only agent operator runs `/plugin install rulake-core` and is done. A multi-substrate genomics+quantum operator runs three commands. The store fronting matches how people actually want to think about the install.
- **The live demo + the marketplace are the same artifact.** The Cloud Run deploy at `https://rulake-mcp.ruv.io/` *is* the default `rulake-core` MCP wire — operators who install the plugin and don't switch to stdio are immediately running against a real, monitored, smoke-tested production wire.
- **Plugin = packaging, not code.** Plugins reference binaries that already exist; updates to the underlying crates flow into the plugin without a packaging-side change. Less to keep in sync.
- **Discovery surface compounds.** Anthropic's plugin directory at `claude-plugins-official` is the next-level surface; ruLake's marketplace becomes submission-ready once the five plugins are packaged.

### Negative

- **Two install paths to support.** The npm `rulake-wasm@2.3.0-alpha.1` install (for browser/edge consumers) and the Claude Code `/plugin install rulake-core` path (for agent operators) are both real and both need docs. Mitigation: docs/userguide already differentiates "open the live console" vs "deploy your own MCP"; add a third axis "install the Claude Code plugin."
- **Trust model is operator-side.** Claude Code does not auto-verify plugin content. ruLake's posture (pin versions, default to least-privilege capabilities, document the demo caveat) is documentation, not enforcement. An operator who installs and runs `--capabilities admin` against the demo URL is on their own. Mitigation: the storefront's per-plugin landing page leads with the trust model.
- **CI scope grows.** A new `validate-marketplace.yml` workflow adds 1 CI job. Acceptable given the marketplace's central role in the discovery surface.
- **Versioning coupling.** Each plugin's `version` field must track its parent crate's version. The new CI job catches drift, but operators bumping a crate version must remember to bump the plugin manifest. Recommend a `scripts/bump-plugin-versions.sh` helper.

### Neutral

- The five-plugin shape is a starting point. If operator usage proves a different axis matters (e.g. "I want only the IPFS adapter, not GCS"), the catalog can split `rulake-substrates` without breaking the installed-base contract — a new plugin replaces a subset of the old plugin's tools, and the deprecation cycle is managed via marketplace-version pins.
- The storefront extension to `/marketplace` reuses the Console's existing route shape (the seven `*Screen.jsx` components in `ui/src/components/screens.jsx`); it's a sixth screen, not a separate app. No new build pipeline.

## Failure modes (and how the design mitigates each)

| Failure mode | Symptom an operator hits | Mitigation in this ADR |
|---|---|---|
| **Plugin fatigue** | Operator faces five install commands and walks away. | `rulake-stack` is the marketplace's `"default": true`. The storefront leads with one install command; the four composable plugins are below the fold. |
| **Trust gap** | Operator installs an unsigned plugin and runs `--capabilities admin` against the demo URL. | Plugins default to `--capabilities read`. Storefront's per-plugin landing page leads with the trust model. Releases are tagged. Signing is forward-committed to Anthropic's roadmap. |
| **Demo misuse** | Operator installs `rulake-core`, points production data at `https://rulake-mcp.ruv.io/`. | The default `.mcp.json` ships the public-demo URL with a `comment` field warning "for real data, switch to the stdio profile." The Console's topbar shows ● LIVE vs ○ DEMO so the wire is unambiguous. The deploy doc walks the production-mode setup. |
| **Latency variance across substrates** | A query that fans out to GCS + IPFS sees 800 ms tail latency from the slowest substrate. | The `decision_trace.latency` block exposes per-substrate timing. The deterministic-retrieval-path's substrate-fanout step is parameterized by a per-substrate timeout; tail-substrate timeout returns a partial result with a `refusals: ["substrate-X-timeout"]` row in the trace, not a 500. |

These four failure modes are the ones that look bad in a demo before
they look bad in production; the mitigations are baked into the install
defaults, the response shape, and the docs — not deferred.

## Design questions resolved

The first revision left two big questions open. This revision answers
both.

**Q: Invisible infrastructure or developer-facing product?**

**Both, at different layers.** ruLake is *invisible at the agent layer*
— the agent calls `rulake_query`, gets a result + a `decision_trace`,
and never sees the witness machinery, the substrate fanout, or the
kernel dispatch. The agent author writes one line of MCP-tool code, and
the rest of the stack is below the API.

ruLake is *developer-facing at the operator layer* — the Console (with
its 7 routes), the CLI (`/rulake-verify`, `/rulake-bundle-info`,
`/rulake-kernel-status`), the per-plugin install commands, the audit
ledger, the deploy docs. Operators see, configure, and debug the
substrate.

The two layers don't compete; they're stacked. An agent that uses
`rulake_query` doesn't need to know there's a Console. An operator
running the Console doesn't need to write agent code. The plugin
shape (one MCP-tool plugin + the storefront) is the bridge between
them.

**Q: Should `rulake_query` expose policy inputs (cost, trust, latency),
or stay simple?**

**Expose them.** The `decision_trace` block (above) is the structural
answer. Hiding the cost / trust / latency intelligence inside the
server makes it impossible for an agent to negotiate — it has to
either accept whatever ruLake decides, or reach around the abstraction
to the underlying substrates (which destroys the value of the
intermediary). Exposing the trace lets an agent author make
risk-vs-cost trade-offs in their prompt logic ("if cost.backend_fetch
> X, narrow query and retry"). Keep `rulake_query`'s **inputs** simple
(intent / risk / freshness / target) but its **outputs** rich
(`decision_trace` is part of the contract, not a debug field).

The opposite design (hidden policy) was the failure mode of every
"smart router" that came before — opaque dispatch is what made
LangChain and the early MCP middlewares un-debuggable. ruLake explicitly
rejects that path.

## Open questions (down from 5 to 4)

1. **Should `rulake-kernels` be installable on hosts without the host requirements?** The AVX-512 kernel needs `is_x86_feature_detected!("avx512vpopcntdq")`; the wgpu kernel needs an adapter. Both already fail-closed at construction. But should the *plugin install* refuse on those hosts, or install with a runtime-warning banner? Current direction: install always, document the capability.
2. **Does `rulake-loop-vector` need its own MCP server, or is it a skill-only plugin?** It composes `rulake-core` + `rulake-substrates`; if all its capabilities are reachable through those plugins' tools, a skill-only shape is cleaner. Tentative: skill-only for v0.1, revisit if a /loop-specific tool surface emerges.
3. **Submission to `anthropics/claude-plugins-official`?** The ADR commits to "submission-ready" but not "submitted." Submission is downstream and depends on Anthropic's review cadence. Mitigation: ship the marketplace at `ruvnet/RuLake` first; submit once the six plugins have been smoke-installed by external operators.
4. **The mincut-prune step** in the deterministic retrieval path is **v2.4 roadmap** — `crates/core/src/select.rs` doesn't exist yet. The flow is otherwise complete; this is the one step the contractual diagram references that ships later. Until then, the path is `intent → cache check → substrate fanout → witness verify → return` (no pruning), and `decision_trace.chosen_path` reports `"deterministic-retrieval-path-v0.1"` so the upgrade is visible.

(Plugin signing was open question 4 in the previous revision; it's now
folded into the trust-posture commitment — no separate question.)
