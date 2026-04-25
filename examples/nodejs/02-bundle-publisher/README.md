# 02 — bundle-publisher

HTTP server that watches a ruLake publish directory and serves the
`table.rulake.json` sidecars over the network with witness-based
ETags. Pair it with a Rust ruLake worker that calls
`publish_bundle(key, dir)` into the same directory and you have a
language-portable cache-coherence fan-out: any reader (Rust ruLake,
the Node verifier from module 01, an MCP agent from module 04, etc.)
can poll for a witness change and decide whether to refresh.

## Endpoints

| Method | Path | Purpose |
|--------|------|---------|
| GET    | `/health` | `{ ok, watched, bundles }` |
| GET    | `/bundles` | Index of all served bundles |
| GET    | `/bundles/:backend/:collection/table.rulake.json` | The bundle JSON. ETag = `"<witness>"`, `Cache-Control: no-cache, must-revalidate`. |
| GET    | `/bundles/:backend/:collection/witness` | Cheap witness probe `{ witness, generation, format_version, verified }`. |

## Install

```bash
cd examples/nodejs/02-bundle-publisher
npm install
```

## Run

```bash
# Point it at any directory the Rust crate's publish_bundle() writes into.
tsx src/server.ts /path/to/publish-dir 8787
```

Then drive it from another terminal:

```bash
curl -s http://localhost:8787/health | jq
curl -s http://localhost:8787/bundles | jq
curl -sD - http://localhost:8787/bundles/publisher/memories/table.rulake.json
# Re-request with If-None-Match to see a 304:
curl -sD - -H 'If-None-Match: "<witness-from-previous-response>"' \
   http://localhost:8787/bundles/publisher/memories/table.rulake.json
```

## End-to-end with the Rust crate

```bash
# Terminal 1 — produce bundles.
cd /path/to/RuLake
cargo run --release --example sidecar_daemon &
# (note the "Publish directory: /tmp/rulake-sidecar-demo-<pid>" line)

# Terminal 2 — serve them.
cd examples/nodejs/02-bundle-publisher
tsx src/server.ts /tmp/rulake-sidecar-demo-<pid> 8787
```

Any change the Rust publisher makes to the directory tree shows up in
the next HTTP request: the witness changes, the ETag changes, clients
revalidate.

## Test

```bash
npm test
```

The test suite uses real on-disk bundles (created with the same
witness algorithm the Rust crate uses), drives the express app with
supertest, and includes a poison-guard test that verifies the server
refuses to serve a sidecar whose witness doesn't validate.

## Design notes

- The witness IS the strong ETag. There is no payload-content hash;
  the publisher already paid for SHAKE-256.
- Every `add` / `change` event re-parses + re-verifies. A failed
  verification drops the entry rather than serving stale or poisoned
  bytes. This is the same "fail closed" stance the Rust
  `read_from_dir` takes.
- The watcher expects the Rust `publish_bundle` layout
  (`<backend>/<collection>/table.rulake.json`). Files outside that
  shape are ignored, not errored on.
- `awaitWriteFinish` is set to 75 ms — long enough to see the atomic
  rename happen, short enough not to add user-visible latency.
