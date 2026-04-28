# rulake-loop-vector

`/loop`-aware vector workers for ruLake. Composes `rulake-core` + `rulake-substrates` into long-running agentic patterns.

## Workers (skill-only, v0.1)

| Skill | Pattern |
|---|---|
| `/rulake-incremental-index` | Pull new bundles from a registered backend on a schedule; warm the cache; emit `WITNESS_MATCH` audit rows |
| `/rulake-refresh-from-bundle` | Schedule periodic `rulake_refresh_from_bundle_dir` calls; three-state outcome (`up_to_date` / `invalidated` / `bundle_missing`) |
| `/rulake-witness-watchdog` | Watch the audit ledger for `WITNESS_MISMATCH_REFUSED` rows; on hit, refuse the in-flight query, narrow scope, retry once with `--freshness_ms 0` |

## Why skill-only

These are composition patterns, not new tools. They orchestrate `rulake_query`, `rulake_refresh_from_bundle_dir`, and the audit ledger via the existing `rulake-core` MCP wire. No new MCP server, no new backend.

The /loop integration matches the pattern in `CLAUDE.md`'s "Advanced /loop Patterns" section — Monitor + ScheduleWakeup, with the audit ledger as the wake signal for the witness-watchdog.

## See also

- [`/CLAUDE.md` — Advanced /loop Patterns](https://github.com/ruvnet/RuLake/blob/main/CLAUDE.md) (the parent ruflo-flavored CLAUDE.md)
- [ADR-009](https://github.com/ruvnet/RuLake/blob/main/docs/adrs/sdk/ADR-009-rulake-plugin-marketplace.md) — the deterministic retrieval path that all three workers route through
