# 09 — Live MCP setup

If you want the Console to drive your own server instead of the hosted
demo at `rulake-mcp.ruv.io`, this page is the three-minute summary. The
full recipe — every gotcha, every commit reference — lives in
[`docs/deploy/cloud-run.md`](../deploy/cloud-run.md).

## The shortest possible flow

The hosted demo runs the `mcp-server` binary in distroless on Cloud Run,
fronted by a Cloudflare CNAME for the `rulake-mcp.ruv.io` hostname. To
reproduce against your own GCP project:

```bash
# 1. Build and push the image (~3 min on E2_HIGHCPU_8)
gcloud builds submit . --project=YOUR-PROJECT --region=us-central1 \
  --config=deploy/cloudbuild-mcp.yaml

# 2. Deploy to Cloud Run with the right capability and binding flags
gcloud run deploy rulake-mcp-demo \
  --image=us-central1-docker.pkg.dev/YOUR-PROJECT/cloud-run-source-deploy/rulake-mcp-demo:latest \
  --region=us-central1 --project=YOUR-PROJECT \
  --allow-unauthenticated --port=8080 \
  --memory=512Mi --cpu=1 --min-instances=1 --max-instances=1 \
  --timeout=60s \
  --set-env-vars="RUST_LOG=info" \
  --command=/usr/local/bin/rulake-mcp \
  --args="^|^http|--bind|0.0.0.0:8080|--auth|none|--insecure-allow-no-auth|--capabilities|read,publish,admin"

# 3. Tell rmcp's Host guard about your user-facing hostname(s)
gcloud run services update rulake-mcp-demo \
  --region=us-central1 --project=YOUR-PROJECT \
  --update-env-vars="^|^RULAKE_ALLOWED_HOSTS=rulake-mcp-demo-NUMBERS.us-central1.run.app,your-host.example"

# 4. Smoke-test the wire end-to-end (TLS + CORS + MCP handshake + tools/list)
URL=https://your-host.example/ ./scripts/smoke-live.sh
```

That is the whole flow. If `smoke-live.sh` exits 0 with `tools/list
returned 8 tools`, you are ready to point the Console at it from the
[Connect screen](./04-connect.md).

## What every flag is doing

| Flag | Why it matters |
|---|---|
| `--min-instances=1 --max-instances=1` | rmcp's `LocalSessionManager` is process-local. Sessions don't survive failover; pin to one warm instance for the demo wire. |
| `--port=8080` + hardcoded `--bind 0.0.0.0:8080` | Cloud Run sets `$PORT` dynamically, but distroless has no shell to interpolate. 8080 is Cloud Run's default when `--port` isn't set, so hard-coding both sides works. |
| `--auth none --insecure-allow-no-auth` | Demo / read-only. Production replaces this with JWT or mTLS — same infra, different two flags. |
| `--capabilities read,publish,admin` | The 8-tool surface. Without this, mcp-server defaults to read-only (3 tools) and the Console will report `3 tools` instead of `8`. |
| `^|^` arg delimiter | The default delimiter for `--args` is comma, but the capability list itself contains commas. `^|^` tells gcloud to split on `\|` instead. |
| `RULAKE_ALLOWED_HOSTS=...` | rmcp's DNS-rebinding guard rejects unknown `Host` headers with 403. Put every user-facing hostname in here. Also enables a stable principal so sessions survive Cloud Run's per-request internal-IP rotation. |

## What needs a custom domain

Nothing, strictly. The Cloud Run-generated URL
(`rulake-mcp-demo-NUMBERS.us-central1.run.app/`) works fine. The custom
domain is purely cosmetic. If you skip the CNAME / domain mapping
(steps 5a–5c in the deploy doc), you only need to set the Console's
default endpoint — see step 7 in the deploy doc.

## Smoke-testing without GCP

If you just want to know whether your MCP is wire-compatible with the
Console, point the smoke script at it:

```bash
URL=https://your-mcp.example/ ./scripts/smoke-live.sh
```

The script makes 11 assertions in ~3 seconds, no Chrome required:

1. TLS handshake completes
2. CORS preflight `OPTIONS` returns 204
3. Preflight carries `access-control-allow-origin` for `https://ruvnet.github.io`
4. Preflight carries `access-control-allow-methods`
5. Preflight carries `access-control-allow-headers`
6. Preflight exposes `mcp-session-id`
7. `initialize` returns 200 with an `mcp-session-id` header
8. `notifications/initialized` returns 200 or 202
9. `tools/list` returns at least one SSE `data:` line with a JSON envelope
10. The envelope's `result.tools` length matches `EXPECTED_TOOLS` (default 8)
11. Exit 0 if every assertion passed

If any assertion fails the script prints the failed one in red and exits 1.
This is the same shape the CI workflow runs on every PR.

## Pointing the Console at your server

Two paths:

1. **Per-session** — open the Console, go to [Connect](./04-connect.md),
   paste your endpoint URL, click **Connect & initialize**. The pill in
   the topbar flips to `● LIVE · <your host>`. Save the endpoint to keep
   it across sessions.
2. **Permanent** — fork the repo, edit
   `ui/src/components/screens.jsx` `ConnectScreen` `setEndpoint` default
   value, rebuild and redeploy the Console. Both the boot probe and the
   Connect form will use the new default. The release-ui workflow handles
   the GitHub Pages publish.

## Cost ballpark

Cloud Run free tier covers 2M requests / 360k GB-s / 180k CPU-s per month.
For the demo's `min=1 max=1 512Mi 1cpu` profile:

- Idle: ~$5/mo for the always-warm instance.
- Per request: free up to the 2M ceiling.

If you can tolerate a 2-3 second cold start every time the Console loads,
drop to `min=0` and the cost drops to ~$0. The session-stickiness caveat
still applies — every cold start drops every in-flight session.

## Full recipe

For everything else — `.gcloudignore` config, BuildKit gotchas, Cloudflare
CNAME, Let's Encrypt cert provisioning, the iteration history that produced
this recipe — see [`docs/deploy/cloud-run.md`](../deploy/cloud-run.md).
