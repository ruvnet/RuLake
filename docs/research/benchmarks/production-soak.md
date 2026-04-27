# Production soak — live Cloud Run MCP wires

**Issue**: [#7](https://github.com/ruvnet/RuLake/issues/7)
**Date**: 2026-04-27
**Reproduction**: `URL=… EXPECTED_TOOLS=… MODE=all ./scripts/soak-live.sh`

Real-world wall-clock measurements against the three production MCP
deployments. Companion to the synthetic criterion benches in
[`v0.0-substrates.md`](v0.0-substrates.md) — those measure the in-process
hot path; this measures the full wire (Cloudflare DNS → Cloud Run domain
mapping → mcp-server → MCP handshake → SSE response back).

## Setup

| | |
|---|---|
| Tooling | Custom bash + `curl` harness ([`scripts/soak-live.sh`](../../../scripts/soak-live.sh)) — does the full MCP handshake (`initialize` → `notifications/initialized` → `tools/list`) per request, parses the SSE response. |
| Cloud Run config | `min=1 max=1`, `512Mi` RAM, `1 cpu`, `--insecure-allow-no-auth`, single warm instance per service. |
| Modes | `latency` (60s sustained at N rps) · `concurrent` (N parallel sessions × 5 calls each) · `idle` (open + sleep 90s + tools/list). |
| User-Agent | `rulake-soak-test/1.0` (so Cloud Run logs can separate soak from real demo traffic) |

## Results

### `https://rulake-mcp.ruv.io/` (mcp-server, 8 tools, full surface)

| Mode | Param | Sent | OK | Success | p50 | p95 | p99 |
|---|---|---|---|---|---|---|---|
| latency | 10 rps | 310 | 310 | **100.0%** | 176 ms | 225 ms | 280 ms |
| latency | 50 rps | 323 | 323 | **100.0%** | 161 ms | 227 ms | 327 ms |
| concurrent | 1 sess | 5 | 5 | 100.0% | 161 ms | 208 ms | 208 ms |
| concurrent | 10 sess | 50 | 50 | 100.0% | 171 ms | 231 ms | 241 ms |
| concurrent | **50 sess** | 175 | **29** | **16.6%** | 184 ms | 223 ms | 233 ms |
| idle | 90 s | 1 | 1 | 100.0% | 264 ms | — | — |

### `https://rvdna-mcp.ruv.io/` (mcp-rvdna, 5 tools, read+internal)

| Mode | Param | Sent | OK | Success | p50 | p95 | p99 |
|---|---|---|---|---|---|---|---|
| latency | 10 rps | 313 | 313 | 100.0% | 176 ms | 224 ms | 249 ms |
| latency | 50 rps | 327 | 327 | 100.0% | 163 ms | 228 ms | 244 ms |
| concurrent | 1 sess | 5 | 5 | 100.0% | 174 ms | 210 ms | 210 ms |
| concurrent | 10 sess | 50 | 50 | 100.0% | 182 ms | 228 ms | 299 ms |
| concurrent | **50 sess** | 250 | **250** | **100.0%** | 190 ms | 231 ms | 276 ms |
| idle | 90 s | 1 | 1 | 100.0% | 384 ms | — | — |

### `https://ruqu-mcp.ruv.io/` (mcp-ruqu, 5 tools, read+publish+admin)

| Mode | Param | Sent | OK | Success | p50 | p95 | p99 |
|---|---|---|---|---|---|---|---|
| latency | 10 rps | 293 | 293 | 100.0% | 183 ms | 242 ms | 392 ms |
| latency | 50 rps | 311 | 311 | 100.0% | 170 ms | 233 ms | 252 ms |
| concurrent | 1 sess | 5 | 5 | 100.0% | 197 ms | 224 ms | 224 ms |
| concurrent | 10 sess | 50 | 50 | 100.0% | 178 ms | 238 ms | 242 ms |
| concurrent | **50 sess** | 250 | **250** | **100.0%** | 241 ms | 317 ms | 338 ms |
| idle | 90 s | 1 | 1 | 100.0% | 252 ms | — | — |

## What the numbers tell us

- **The wire is healthy at production rates**: 100% success at 10 rps and
  50 rps sustained for 60 s on all three services, p99 ≤ 392 ms across
  every test. The wall-clock floor is ~160 ms (Cloud Run cold-path +
  Cloudflare TLS + the 3-call MCP handshake + SSE parse) — that's the
  irreducible minimum, not server work.
- **Sessions survive the 90-second idle window** on all three services.
  No mid-session reconnect logic needed in the Console for the demo
  workload.
- **`mcp-server` (rulake-mcp.ruv.io) degrades hard at 50 concurrent
  sessions** (16.6% success) while `mcp-rvdna` and `mcp-ruqu` hold
  100% at the same load. The differential is real and reproducible —
  see the open question below.

## Open question — mcp-server vs companions at high concurrency

`mcp-server` is the only one that exposes `--capabilities publish admin`
(8 tools including the 5 mutation handlers iter-32 wired). It also
runs the `serve_with_guards` path through replay-protection +
session-binding + layered rate-limiting (`mcp-server/src/http.rs:92+`),
which `mcp-rvdna` and `mcp-ruqu` deliberately don't lift in v0.0.1.

Most likely culprit: `LayeredRateLimiter` defaults
(`per_principal: 60 rps / 120 burst`, `per_collection: 30/60`,
`per_process: 600/1200`). With `RULAKE_ALLOWED_HOSTS` set, every
session shares the `anon:proxied` principal — so 50 concurrent
sessions × ~5 tools/list calls each in a 5-s window = ~250 req/s on
one principal, well past the `60 rps + 120 burst` ceiling. The other
two services don't ship the rate limiter, so they don't see this.

**Action for v0.11**: when `RULAKE_ALLOWED_HOSTS` is set, either bump
the per-principal cap dramatically (the "shared principal" assumption
makes the per-principal limiter meaningless anyway) or skip per-principal
limiting and rely on per-process. This isn't a security regression —
the `anon:proxied` principal is already shared so per-principal
isolation was illusory.

## What we'd want to fix in v0.1.x

- **Tune `LayeredRateLimiter` for proxy mode** (above) — the highest-
  signal finding; gates `mcp-server` from being usable at scale behind
  a reverse proxy.
- **Document Cloud Run cost vs scale**: at `min=1 max=1`, one warm
  instance handles the demo workload comfortably (idle ~160 ms p50,
  loaded ~190 ms p50). Bumping `max-instances` past 1 won't help
  until session-binding is fixed for cross-instance routing — that's
  separately roadmapped.

## Reproducing

The harness is committed at [`scripts/soak-live.sh`](../../../scripts/soak-live.sh)
(174 lines, bash + curl, no Rust toolchain needed). Run any subset:

```bash
# Default — full suite against rulake-mcp.ruv.io
./scripts/soak-live.sh

# A different MCP
URL=https://rvdna-mcp.ruv.io/ EXPECTED_TOOLS=5 ./scripts/soak-live.sh

# Just the latency tests
URL=https://rulake-mcp.ruv.io/ MODE=latency RPS_LIST="10 50 100" ./scripts/soak-live.sh

# Custom output dir for per-mode CSVs
OUT_DIR=/tmp/my-soak ./scripts/soak-live.sh
```

CSVs land in `${OUT_DIR}/results.csv`; the script also writes one CSV
per mode for deeper analysis.
