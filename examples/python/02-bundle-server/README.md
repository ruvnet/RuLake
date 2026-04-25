# 02 — bundle-server

A FastAPI server that watches a publish directory for ruLake
`table.rulake.json` sidecars and serves them over HTTP. This is the
publication side of the bundle protocol — Rust ruLake instances write
sidecars atomically, this server picks them up, and any consumer (any
language) can fetch the JSON or just the witness over HTTP.

Why HTTP rather than shared filesystem? Most production deployments
have services in different containers / pods / clouds. HTTP with an
ETag set to the witness gets you 304-able cache-rotation detection
without the consumer needing read access to the publisher's disk.

## Endpoints

| Method | Path | Returns |
|--------|------|---------|
| `GET` | `/health` | `{ status, root, bundles, uptime_unix }` |
| `GET` | `/bundles` | `{ keys: [...] }` |
| `GET` | `/bundles/{key}/table.rulake.json` | the raw sidecar JSON |
| `GET` | `/bundles/{key}/witness` | the bare 64-hex `rvf_witness` |

`{key}` may include a single `/` to model the typical
`backend-id/collection` pair (e.g. `prod-warehouse/memories`). The
server URL-decodes it as a path.

Every served bundle:

- has had its witness re-verified at load time (a sidecar that fails
  its own witness is silently dropped — never served);
- carries `ETag: "<witness>"` so clients can short-circuit on
  unchanged bundles;
- carries `X-RuLake-Witness: <witness>` for cheap header-only probes;
- carries `Cache-Control: no-cache` because the next rotation may
  arrive at any moment.

## Install

```bash
cd examples/python/02-bundle-server

# 01-verify-witness is a path dep, install it first.
pip install -e ../01-verify-witness

python3 -m venv .venv
. .venv/bin/activate
pip install -e .[dev]
```

## Run the server

```bash
mkdir -p ./publish-root
python server.py ./publish-root --host 127.0.0.1 --port 8088
```

The server scans `./publish-root` once on startup, then watches it for
`table.rulake.json` files at any depth. The directory layout is::

    publish-root/
      <key-component>/<...>/table.rulake.json

so e.g. dropping a file at
`publish-root/prod-warehouse/memories/table.rulake.json` makes it
visible at `GET /bundles/prod-warehouse/memories/table.rulake.json`.

## Publish a bundle

```bash
# atomic publish — copies + renames into place so the server only
# ever observes a fully-written file.
python publish.py ./publish-root prod-warehouse/memories \
    /path/to/source-bundle.json
```

`publish.py` verifies the source bundle's witness *before* publishing —
a broken bundle never reaches the publish dir. The destination is
`<root>/<key>/table.rulake.json`, written via tempfile + `os.replace`,
matching the Rust `RuLakeBundle::write_to_dir` semantics.

## End-to-end with the Rust side

```bash
# Terminal 1 — start the server.
cd examples/python/02-bundle-server
python server.py ./publish-root

# Terminal 2 — produce a real ruLake bundle and publish it.
cd RuLake
cargo run --release --example sidecar_daemon &
SIDECAR=$(ls /tmp/rulake-sidecar-demo-*/table.rulake.json | head -1)
python examples/python/02-bundle-server/publish.py \
    examples/python/02-bundle-server/publish-root \
    publisher/memories \
    "$SIDECAR"

# Terminal 3 — fetch.
curl -s http://127.0.0.1:8088/bundles/publisher/memories/witness
curl -s http://127.0.0.1:8088/bundles/publisher/memories/table.rulake.json
```

## Tests

```bash
pytest tests/ -v
```

Tests use FastAPI's `TestClient` and disable the watchdog observer
(`watch=False`), driving `BundleIndex.load_one` / `drop` directly.
That avoids cross-platform timing slop and keeps the suite at
sub-second wall time.

## Design notes

- The watcher relies on `on_moved` to catch the atomic publish path
  (`tempfile -> rename -> table.rulake.json`); `on_modified` is a
  fallback for editors / publishers that write in-place.
- Sidecars whose witness fails are *not* cached. Logging stays loud so
  a misconfigured publisher gets caught.
- The `key` path component must not contain `..`, `\`, or `:` and
  cannot be empty. Anything else flowing into `publish.py` is rejected
  with a non-zero exit.
- ETag is the witness, so consumers doing `If-None-Match` get free
  bandwidth back when the cache hasn't rotated.
