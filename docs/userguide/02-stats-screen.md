# 02 — Stats screen

The Stats screen is the landing route. It is a 1 Hz live tail of the metrics
the cache layer publishes via `rulake://stats` plus a per-backend rollup
that mirrors `rulake://stats/by-backend`.

![Stats screen with six tile counters, throughput chart, latency histogram, refusal breakdown and the vector substrate animation](../../assets/console-hero.png)

## Header — env scope

The thin coloured strip across the top reflects whichever environment tab
you have selected in the topbar:

- `lake-prod` (green) — `us-west · primary`, full traffic
- `lake-eu` (cyan) — `eu-west · replica`, ~42% load
- `lake-edge` (amber) — `pop-* · degraded`, ~8% load with elevated refusals

The numbers in every tile and chart are scaled to the active env. The
`load` percentage on the right is the multiplier applied to the
production baseline.

Two buttons live in the pane header next to the `● LIVE · 1.0 Hz` strip:

- **Pause** — freezes the substrate animation and the tile sparklines.
- **Export JSONL** — downloads a 2-line newline-delimited JSON snapshot of
  the current envelope. Useful for pasting into an issue or a notebook.

## The six tiles

Reading them left to right:

| Tile | What it counts | Source of truth |
|---|---|---|
| **Hits** | Cache hits over the rolling window | `CacheStats::hits` |
| **Misses** | Cache misses (forced backend pulls) | `CacheStats::misses` |
| **Hit rate** | `hits / (hits + misses)` as % | `CacheStats::hit_rate()` |
| **Primes** | Cache fills (backend pulls + RaBitQ build) | `CacheStats::primes` |
| **Refused** | Honest refusals — see below | aggregated from MCP audit |
| **Witnesses verified** | Successful SHAKE-256 verifications | `read_from_dir` outcomes |

Each tile carries a sparkline of the last ~30 ticks and a delta line under
the value (e.g. `+218/min`). The hit-rate tile shows the ADR-155 acceptance
target of `≥ 95%` as text — anything under that on a steady workload is the
signal that the cache is being fought by either invalidation churn or a
freshness-budget too tight for the backend's generation cadence.

## Throughput + latency

Below the tiles are two charts:

- **Query throughput · 60s window** — three coloured lines, one per
  backend. The y-axis is `q/s`. Useful for spotting sudden traffic shifts
  between regions.
- **Latency p50 / p99** — a histogram bucketed at `0 / 5 / 10 / 25 / 50 /
  100ms`. p50 and p99 numerals are repeated below the histogram for
  at-a-glance reading. Production p50 sits around 8–12 ms; edge can spike
  to ~18 ms at p50 and 60+ at p99.

## Per-backend rollup

The lower-left table is sourced from the same data the
`rulake://stats/by-backend` resource serves. Columns:

| Column | Meaning |
|---|---|
| `Backend` | Adapter id (e.g. `lake-prod`) — green dot = verified |
| `Region` | Free-form locality string from the adapter |
| `Hits` / `Miss` | Sum across all collections in that backend |
| `HR%` | Hit rate as a one-decimal percentage |
| `Prime ms` | Mean cold-prime cost across collections that have primed |
| `Witness` | Truncated SHAKE-256 of the head collection |

## Refusal breakdown

Top-right of the lower row. Five rows, each `<CODE> <count> <bar>`. The
amber bar is normalised against the largest count in the window (so the
biggest bar is always full-width — read it as relative pressure, not
absolute).

Codes you will actually see on a healthy server:

- **`WITNESS_MISMATCH_REFUSED`** — server's witness disagrees with the
  recomputed one. Always-refuse, never-degrade.
- **`STALE_BUNDLE_GUARD`** — the cached generation is older than the
  freshness budget for the consistency mode in use. Refusal under `Fresh`,
  fallback under `Eventual`.
- **`STALE_BUNDLE_FALLBACK`** — same root cause as the guard, but the
  consistency mode allowed degraded service so the request returned with a
  warning instead of a refusal.
- **`POLICY_DENIED`** — JWT scope or `pii_policy` rejected the call.
- **`BUDGET_EXCEEDED`** — query exhausted its `budget.max_latency_ms`.

The "honest refusals" stripe under the breakdown is intentional copy: a
refusal is the planner declining to serve a query whose witness, freshness,
or capability budget would be violated. Treat the count as a system-working
indicator, not as an error.

## Vector substrate (ambient)

The animation strip at the bottom is decorative. The gear icon to the right
of the section header opens a popover with three knobs:

- `density` — point count multiplier
- `speed` — frame-rate multiplier
- `perspective` — z-axis tilt multiplier

Plus toggles for the orbit ring and the witness halo. Settings persist to
`localStorage` under `rulake-substrate`. They have no effect on any
operational behaviour.

## Where the numbers come from

In `● LIVE` mode the Console polls `rulake://stats` and renders. In demo
mode it scales a deterministic fixture (`ui/src/lib/data.js`) so the screen
still reads usefully without a server. The `lake-prod` baseline is the real
shape; `lake-eu` and `lake-edge` are derived multipliers.

If you need the same tiles in your own monitoring, the wire form is
straightforward — see [05 — Backends and collections](./05-backends-and-collections.md)
for the resource schema, and [09 — Live MCP setup](./09-live-mcp-setup.md)
for getting your own server up.
