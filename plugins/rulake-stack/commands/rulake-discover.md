---
description: Discover other ruLake marketplace plugins and recommend which ones to install based on what the agent is trying to do. The advisory entry-point — answers "which ruLake plugin do I need for X?".
---

# /rulake-core:rulake-discover

Reads the ruLake marketplace catalog (`.claude-plugin/marketplace.json` from `ruvnet/RuLake`), cross-references it against the locally-installed plugin set, and suggests what to install for a given goal.

## Inputs

- **goal** (optional, free-form) — what the agent is trying to do (e.g. "search genomic data", "remember user preferences across sessions", "verify a bundle's witness", "speed up large queries"). When omitted, returns the full catalog summary.

## Examples

```text
/rulake-core:rulake-discover
/rulake-core:rulake-discover goal="remember things across agent sessions"
/rulake-core:rulake-discover goal="run a quantum circuit"
/rulake-core:rulake-discover goal="benchmark the kernel on this host"
```

## What you get back (no goal — full catalog summary)

```jsonc
{
  "marketplace": "ruvnet/RuLake",
  "plugins_total": 8,
  "plugins_installed": 3,
  "catalog": [
    {
      "name": "rulake-stack",
      "installed": true,
      "purpose": "Killer-path install. Bundles core + rvdna + ruqu + witness MCP wires + the most-used commands.",
      "use_when": "you want everything working in one install"
    },
    {
      "name": "rulake-core",
      "installed": true,
      "purpose": "The 8 rulake_* MCP tools + the rulake_query decision layer.",
      "use_when": "you need raw retrieval API access"
    },
    {
      "name": "rulake-memory",
      "installed": false,
      "purpose": "Self-learning, self-optimizing agentic memory — remember/recall/forget verbs + hit-ratio tuning.",
      "use_when": "you want ruLake as the agent's working memory across sessions",
      "install": "/plugin install rulake-memory@rulake-marketplace"
    },
    {
      "name": "rulake-rvdna",
      "installed": false,
      "purpose": "Genomic substrate — DNA / variants / protein / scoring (5 tools).",
      "use_when": "the agent is reasoning about genomic data",
      "install": "/plugin install rulake-rvdna@rulake-marketplace"
    },
    {
      "name": "rulake-ruqu",
      "installed": true,
      "purpose": "Quantum-simulation substrate — circuit sim / verify / replay / optimize / qec (5 tools).",
      "use_when": "the agent is running or planning quantum circuits"
    },
    {
      "name": "rulake-kernels",
      "installed": false,
      "purpose": "Optional CPU+GPU accelerators — AVX-512 SIMD + wgpu portable GPU.",
      "use_when": "you have big workloads (D ≥ 768, n ≥ 1M) and want measurable speedup",
      "install": "/plugin install rulake-kernels@rulake-marketplace"
    },
    {
      "name": "rulake-witness",
      "installed": false,
      "purpose": "Local witness verification CLI — verify / bundle-info without booting an MCP.",
      "use_when": "you have a bundle file and need to verify it offline",
      "install": "/plugin install rulake-witness@rulake-marketplace"
    },
    {
      "name": "rulake-loop-vector",
      "installed": false,
      "purpose": "/loop-aware vector workers — incremental indexing, refresh-from-bundle, drift watchdog.",
      "use_when": "you want background jobs that keep the cache fresh on a schedule",
      "install": "/plugin install rulake-loop-vector@rulake-marketplace"
    }
  ]
}
```

## What you get back (with a goal)

```jsonc
{
  "goal": "remember things across agent sessions",
  "best_match": "rulake-memory",
  "rationale": "rulake-memory is the explicit working-memory surface — remember/recall/forget verbs, session-spanning persistence, audit-driven self-tuning. The auto-invokable `memory` skill turns ruLake into the agent's working memory transparently.",
  "install_steps": [
    "/plugin install rulake-memory@rulake-marketplace",
    "/reload-plugins",
    "/rulake-memory:memory-status"
  ],
  "alternatives": [
    {
      "name": "rulake-stack",
      "rationale": "if you also want rvdna/ruqu/witness in one install, take the killer-path bundle",
      "trade-off": "larger install surface; 18 MCP tools you may not use"
    }
  ],
  "complementary": [
    {
      "name": "rulake-loop-vector",
      "rationale": "if you want the memory to be auto-tuned on a schedule, install loop-vector and wire /memory-tune to fire weekly"
    }
  ]
}
```

## When to use

- The agent doesn't know which ruLake plugin to call for a given task
- The user asks "what can ruLake do?" — return the catalog
- The agent is about to do an expensive workaround that a not-yet-installed plugin would solve cleanly
- After a `/plugin marketplace add` to see what just became available

## How it composes

`rulake-discover` is **advisory** — it never installs anything. The recommendations are install commands the agent can hand to the user; the user runs `/plugin install` themselves. This keeps the trust posture explicit (per the trust model in the marketplace docs).
