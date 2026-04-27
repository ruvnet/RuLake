# 04 — Connect

The Connect screen is where you tell the Console which `rulake-mcp` server
to drive. It is also where you save additional endpoints and pick an auth
mode. All credentials are kept in your browser — IndexedDB for the saved
list, JS memory only for the live token.

![Connect screen — endpoint URL, auth mode segmented control, JWT textarea, capability matrix](../../assets/console-connect.png)

## The default endpoint

Out of the box the endpoint field is pre-filled with:

```
https://rulake-mcp.ruv.io/
```

That is the project's reference Cloud Run deploy. It runs `rulake-mcp` with
`--auth none --insecure-allow-no-auth --capabilities read,publish,admin`,
so the Console can `tools/list` and `tools/call` without any token. See
[09 — Live MCP setup](./09-live-mcp-setup.md) for the deploy story.

If you change the endpoint, the change does not auto-save. Click
**Save endpoint** to persist it. Click **Connect & initialize** (or
**Test only** — they call the same handler) to actually drive the wire.

## What "● LIVE" means in the topbar

The pill in the top-right of every screen shows one of:

- **`○ DEMO · no live MCP`** — `window.RULakeActiveClient` is unset. Every
  screen renders fixture data.
- **`● LIVE · <host>`** — `window.RULakeActiveClient` is populated with a
  real MCP session id. Browse, Bundle, and Playground will dispatch real
  `tools/call` requests against that endpoint.

The pill flips on two events:

1. **Boot probe** — the Console tries `https://rulake-mcp.ruv.io/`
   automatically on first paint. Silent success → green pill. Silent
   failure → stay at DEMO.
2. **Connect → Test/Connect button** — same handshake, but with whatever
   endpoint and token you have entered. Green pill on success, with the
   host you pointed at.

The handshake itself is the standard MCP Streamable HTTP dance:

```bash
# 1. initialize — server sets mcp-session-id response header
curl -isS -X POST https://rulake-mcp.ruv.io/ \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize",
       "params":{"protocolVersion":"2024-11-05",
                 "capabilities":{},
                 "clientInfo":{"name":"manual","version":"0"}}}'

# 2. notifications/initialized — same session id from step 1
curl -sS -X POST https://rulake-mcp.ruv.io/ \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -H 'mcp-session-id: <session-from-step-1>' \
  -d '{"jsonrpc":"2.0","method":"notifications/initialized"}'

# 3. tools/list — same session
curl -fsS -X POST https://rulake-mcp.ruv.io/ \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -H 'mcp-session-id: <session>' \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/list"}'
```

The Console's status line under the action buttons reports the round-trip:
`← initialize OK · 187ms · 8 tools · session 7f3c2a91…`. The "8 tools" count
is the contract for the read+publish+admin capability set.

## Auth modes

The segmented control offers four. Pick the one your server is configured
for; the default and the hosted demo are both `none`.

| Mode | What the Console sends | Use case |
|---|---|---|
| **No auth** | No `Authorization` header | Loopback, demo, dev |
| **Bearer** | `Authorization: Bearer <token>` | Static API key in front of mcp-server |
| **JWT** | `Authorization: Bearer <jwt>` | OAuth / PKCE; scope claims drive per-tool caps |
| **mTLS** | (cannot be done from the browser — see below) | Production with client-cert pinning |

### JWT specifics

The JWT field expects a serialized token. Once connected, the server
applies per-call caps from the scope claim. The expected shape is:

```
aud:    https://rulake.ruv.net
scope:  mcp:rulake:read mcp:rulake:publish
```

Per-claim cap enforcement landed in mcp-server v0.8 (see commit `67fc821`
on `main`). If the JWT's scope is missing a capability, the matching tool
will return `POLICY_DENIED` with the specific claim name.

### Why mTLS is greyed out from the browser

The TLS handshake selects the client certificate before any JavaScript
runs. Browsers expose no API to control that selection per fetch. The
Console renders a warning strip:

> mTLS needs a non-browser client. The TLS handshake picks the client cert
> before any JavaScript runs. Use `rulake/http` from Node.js or
> `rulake-cli` instead.

For an mTLS-fronted server, put a Bearer/JWT proxy in front for browser
callers, or skip the Console and use the SDKs.

## Saving and switching endpoints

The **Save endpoint** button writes a row into IndexedDB:
`{label, endpoint, mode, token}`. The token is stored only as the first 8
characters with an ellipsis (e.g. `eyJhbGci…`) — never the whole secret.

**Saved (N)** opens a list of every saved row. Click one to populate the
fields; click Connect to switch the active client. The audit ledger gains
an `INIT_OK` row each time, with the new endpoint as the target.

## The capability matrix

Below the form is a three-row table:

| Capability | Tools | Resources | Status |
|---|---|---|---|
| `read` | `rulake_query` · `list_backends` | `rulake://stats · …/by-backend · …/bundle/*` | GRANTED on the demo |
| `publish` | `rulake_publish_bundle` · `refresh` · `invalidate` | — | GRANTED on the demo |
| `admin` | `save_cache` · `warm_from_dir` · `audit_tail` | — | NOT REQUESTED on the demo |

The actual GRANTED / NOT REQUESTED state comes from the JWT scope claim on
your token (or from the `--capabilities` flag the operator passed to
`rulake-mcp` when there is no token). On the hosted demo all three show
green because the server runs with `--capabilities read,publish,admin`.

## Storage & runtime settings card

Below the connect card sits the **Storage settings** card. Two unrelated
toggles live there:

- **Embedding provider** — pick which API key to use when the Playground
  calls `text-embedding-3-small`. Keys are kept in JS memory only.
- **Workers** — when on, vector search is offloaded to a Web Worker.
  Default is off.

These settings persist to IndexedDB (under the `kv` store) so they survive
a reload.
