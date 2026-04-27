# 10 — Troubleshooting

The Console is mostly self-explanatory, but four classes of failure are
common enough to deserve a checklist. Each entry below names the
symptom you will see, the root cause, and the smallest fix that closes it.

For any failure: open the [Audit ledger](./08-audit-ledger.md) first. The
specific code (`CONNECT_FAILED`, `LIST_COLLECTIONS_FAILED`,
`WITNESS_MISMATCH_REFUSED`) and the message field tell you which of the
sections below applies.

## TLS handshake failed

**Symptom.** `smoke-live.sh` exits red on the very first assertion:
`TLS handshake failed against <URL>`. Or a Connect attempt produces a
`CONNECT_FAILED` audit row whose message contains `Failed to fetch` or
`net::ERR_CERT_*`.

**Cause.** Most often the Cloudflare CNAME for the custom domain has
`proxied: true` (orange cloud), which prevents Cloud Run's domain mapping
from completing the `01-challenge` and provisioning the Let's Encrypt cert.
Less often: the cert is still provisioning (5–15 min after CNAME
resolves), or the cert has expired.

**Fix.**

1. Confirm the CNAME is `proxied: false`:
   ```bash
   curl -sI https://your-host.example/ | head -1
   # Expect HTTP/2 200 or 405; if you see Cloudflare's challenge page or
   # a 525, proxying is on.
   ```
2. If you just provisioned, wait. Track the cert state:
   ```bash
   gcloud beta run domain-mappings describe \
     --domain=your-host.example \
     --region=us-central1 --project=YOUR-PROJECT \
     --format='value(status.conditions[].type:label=cond,status.conditions[].status:label=stat)'
   # Wait until CertificateProvisioned=True
   ```
3. If proxying is on by policy, you can keep it on but only after the
   cert is live. Cloud Run is already terminating TLS, so there is little
   to gain from also letting Cloudflare terminate.

## Topbar pill stays at `○ DEMO`

**Symptom.** You opened the Console, the boot probe should have flipped
to `● LIVE`, but the pill is still grey. Or you clicked Connect and got
a green status line — but the pill never updated.

**Cause.** Three common ones:

1. The boot probe ran, but `https://rulake-mcp.ruv.io/` is unreachable
   from your network (corporate VPN, geo-block, gateway down).
2. The probe succeeded but a downstream JS error swallowed the
   `rulake:live-connected` event before the topbar listener fired.
3. The endpoint you typed in the Connect screen does not include the
   trailing slash. `mcp-server` mounts on `/`; some reverse proxies do
   strict path matching and 404 the no-slash form.

**Fix.**

1. Sanity-check the wire from your machine:
   ```bash
   curl -sI https://rulake-mcp.ruv.io/
   # Expect HTTP/2 405 (it accepts only POST). Any other status is a
   # network problem, not an MCP problem.
   ```
2. Open devtools. The Connect handler logs every step; a `RuLakeHttp`
   exception will be visible there even if the toast slid past you.
3. Always include the trailing slash in the endpoint. The default
   `https://rulake-mcp.ruv.io/` is correct; `https://rulake-mcp.ruv.io`
   may not be.

## CORS preflight rejected

**Symptom.** The Connect attempt produces `CONNECT_FAILED` with a message
containing `Failed to fetch`. The browser console shows a CORS error
explaining that the preflight `OPTIONS /` did not return the expected
headers.

**Cause.** The MCP server is missing CORS support, or its CORS layer is
allow-listed to a different Origin than `https://ruvnet.github.io` (or
whichever origin you are loading the Console from).

**Fix.**

1. Confirm preflight directly:
   ```bash
   curl -isS -X OPTIONS https://your-mcp.example/ \
     -H 'Origin: https://ruvnet.github.io' \
     -H 'Access-Control-Request-Method: POST' \
     -H 'Access-Control-Request-Headers: content-type, mcp-session-id' \
     | head -10
   # Expect HTTP/2 204 with access-control-allow-* headers.
   ```
2. The reference implementation is `crates/mcp-server/src/http.rs:287+`.
   If you wrote your own server, port that pattern: echo the requesting
   `Origin`, expose `mcp-session-id` and `mcp-protocol-version`, and
   short-circuit `OPTIONS` with 204.
3. If the server is fronted by a reverse proxy, make sure the proxy
   does not strip `Access-Control-*` headers on its way back.

This is the same bug class that hit `mcp-rvdna` v0.0.1 and was fixed in
iter 32 — the symptom in the wild was `Failed to fetch` on `OPTIONS /mcp`.

## Session 401 / sessions drop mid-conversation

**Symptom.** `initialize` succeeds (you get a session id back), but the
next call — `tools/list`, `tools/call`, anything — returns 401 with a
message about an unknown session. Or sessions work for a few minutes then
start failing.

**Cause.** rmcp's `LocalSessionManager` keeps sessions in process-local
memory. Two failure modes:

1. **`max-instances > 1`** — the second call lands on a different Cloud
   Run instance which has no record of the session.
2. **Per-request peer-IP rotation** — Cloud Run's frontend rotates the
   internal source IP between requests. Without
   `RULAKE_ALLOWED_HOSTS` set, mcp-server keys principals on
   `anon:{peer}`, which changes between requests, which invalidates the
   session.

**Fix.**

1. Pin the service to one instance:
   ```bash
   gcloud run services update rulake-mcp-demo \
     --region=us-central1 --project=YOUR-PROJECT \
     --min-instances=1 --max-instances=1
   ```
2. Set the allowed-hosts env var (this also enables the
   `anon:proxied` stable principal under reverse proxies, see commit
   `2427543`):
   ```bash
   gcloud run services update rulake-mcp-demo \
     --region=us-central1 --project=YOUR-PROJECT \
     --update-env-vars='^|^RULAKE_ALLOWED_HOSTS=your-host.example,rulake-mcp-demo-NUMBERS.us-central1.run.app'
   ```

For production with a session manager that survives failover, the right
move is to swap `LocalSessionManager` for a Redis-backed one. That is on
the roadmap, not shipped today.

## `RULAKE_ALLOWED_HOSTS` gotcha

**Symptom.** Every request, including `initialize`, returns 403. The
server logs include a tracing entry rejecting the `Host` header with
"DNS rebinding guard".

**Cause.** rmcp's Streamable HTTP transport rejects unknown `Host` values
with 403 by default. Cloud Run's frontend forwards the user-facing
hostname (e.g. `rulake-mcp.ruv.io`), not the internal `0.0.0.0:8080` the
container thinks it is bound to. Without an allow-list, every request
from the public internet 403s.

**Fix.** Set `RULAKE_ALLOWED_HOSTS` to a comma-separated list of every
hostname operators will use. This must include both the Cloud Run-generated
host and any custom domain CNAME'd in front of it:

```bash
gcloud run services update rulake-mcp-demo \
  --region=us-central1 --project=YOUR-PROJECT \
  --update-env-vars='^|^RULAKE_ALLOWED_HOSTS=rulake-mcp.ruv.io,rulake-mcp-demo-iru57wnnaq-uc.a.run.app'
```

The `^|^` delimiter is required because the env value contains a comma —
this is gcloud's escape syntax for delimiter-bearing values.

The reverse-proxy fix (commit `4248b75`) and the stable-principal fix
(`2427543`) both ride on the same env var. Setting it correctly fixes
both classes of failure simultaneously.

## When all else fails

- **Run the smoke script.** `URL=https://your-mcp.example/ scripts/smoke-live.sh`
  walks the entire wire and tells you exactly which assertion failed.
- **Check the audit tail.** Every Console action that touches the wire
  emits a row with the failing code and message in plain text.
- **Read the deploy doc.** `docs/deploy/cloud-run.md` documents every
  gotcha encountered while bringing up the live demo, with commit
  references.
- **Check `CHANGELOG.md`.** The "Iteration history" tables there call
  out commits that fixed each class of regression. Searching the
  changelog for your symptom is often faster than re-deriving the fix.
