# rulake-core

The load-bearing plugin: the witness-anchored cache and the eight `rulake_*` MCP tools.

## Tools added

| Tool | Capability tier | Purpose |
|---|---|---|
| `rulake_query` | read | The decision-layer tool — search / verify / explain / refresh |
| `rulake_list_backends` | read | Enumerate registered backend ids |
| `rulake_list_collections` | read | Per-backend collection list |
| `rulake_publish_bundle` | publish | Atomic write of `table.rulake.json` |
| `rulake_refresh_from_bundle_dir` | publish | Three-state refresh |
| `rulake_save_cache_to_dir` | admin | Snapshot to disk |
| `rulake_warm_from_dir` | admin | Restore from disk |
| `rulake_invalidate_cache` | admin | Drop pointer (substrate forget) |

## How to wire the MCP

`rulake-core` ships **commands only** (no bundled `.mcp.json`). Pick one of:

**Option 1 — install `rulake-stack`** (recommended, includes the wire):

```text
/plugin install rulake-stack@rulake-marketplace
```

**Option 2 — add the wire to your global Claude Code config**:

```json
// ~/.claude.json
{
  "mcpServers": {
    "rulake": { "type": "http", "url": "https://rulake-mcp.ruv.io/" }
  }
}
```

**Option 3 — production stdio**:

```json
{
  "rulake": {
    "type": "stdio",
    "command": "rulake-mcp",
    "args": ["stdio", "--capabilities", "read,publish,admin"]
  }
}
```

The reason `rulake-core` doesn't bundle the wire: when both `rulake-core` and `rulake-stack` are installed, Claude Code dedupes the duplicate `rulake` MCP server name and emits a warning. Cleaner to source the wire from one place.

## Skills

- `/rulake-query` — the recommended entry point; wraps `rulake_query` with intent + freshness budget defaults

## See also

- [`rulake-stack`](../rulake-stack/) — bundles this plugin with substrates + witness
- [ADR-004](https://github.com/ruvnet/RuLake/blob/main/docs/adrs/sdk/ADR-004-rulake-mcp-server.md) — the MCP server design
- [`docs/deploy/cloud-run.md`](https://github.com/ruvnet/RuLake/blob/main/docs/deploy/cloud-run.md) — production deploy recipe
