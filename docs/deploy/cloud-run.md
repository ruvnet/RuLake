# Deploying `mcp-server` to Cloud Run

The reference deploy that powers [`https://rulake-mcp.ruv.io/`](https://rulake-mcp.ruv.io/)
(the live MCP the Console at [ruvnet.github.io/RuLake/](https://ruvnet.github.io/RuLake/)
auto-probes). Iter 51–54 of the working session hit and resolved every
gotcha below; this doc codifies them so the next deploy is a 5-minute
recipe instead of a 2-hour rediscovery session.

> **Heads up.** This recipe is for **demo / read-only** deployments —
> we explicitly run `--auth none --insecure-allow-no-auth` so the
> Console can hit it without bearer/JWT plumbing. For production with
> real data, follow the same recipe but configure JWT or mTLS per
> [ADR-004](../adrs/sdk/ADR-004-rulake-mcp-server.md) §5; the
> infrastructure pieces (image build, env vars, domain mapping) are
> identical.

## Prereqs

- A GCP project with `run.googleapis.com`, `artifactregistry.googleapis.com`,
  and `cloudbuild.googleapis.com` enabled. (`gcloud services enable run.googleapis.com artifactregistry.googleapis.com cloudbuild.googleapis.com`)
- Workflow permissions on the GCP project's Actions:
  `gcloud api repos/.../actions/permissions/workflow → default_workflow_permissions=write`
  (only matters if you want CI to deploy too).
- The ruLake repo checked out at HEAD.

## Step 1 — `.gcloudignore`

Without one, `gcloud run deploy --source .` uploads the entire working
tree (~3 GB once `target/`, `examples/`, `vendor/ruvector/{npm,docs,...}`
are counted). The repo ships with a `.gcloudignore` that gets the
upload down to ~28 MB. Verify it exists:

```bash
test -f .gcloudignore && echo OK || echo "missing — see commit 73e2ef8"
```

## Step 2 — Cloud Build

`gcloud run deploy --source . --dockerfile=deploy/Dockerfile.mcp` doesn't
exist as a single step in current gcloud — the `--source` flag picks
`./Dockerfile` if present (which we moved into `deploy/Dockerfile.demo`
so `--source` no longer auto-picks it; explicit `-f` is required either
way). Use a two-step Cloud Build instead:

```yaml
# /tmp/cloudbuild-mcp.yaml
steps:
  - name: gcr.io/cloud-builders/docker
    env:
      - 'DOCKER_BUILDKIT=1'           # deploy/Dockerfile.mcp uses --mount=type=cache
    args:
      - 'build'
      - '-f'
      - 'deploy/Dockerfile.mcp'
      - '-t'
      - 'us-central1-docker.pkg.dev/YOUR-PROJECT/cloud-run-source-deploy/rulake-mcp-demo:latest'
      - '.'
images:
  - 'us-central1-docker.pkg.dev/YOUR-PROJECT/cloud-run-source-deploy/rulake-mcp-demo:latest'
options:
  machineType: E2_HIGHCPU_8
timeout: 1200s
```

```bash
gcloud builds submit . --project=YOUR-PROJECT --region=us-central1 \
  --config=/tmp/cloudbuild-mcp.yaml
```

Build takes ~3 min on `E2_HIGHCPU_8`.

> **Gotcha — BuildKit.** Cloud Build's `gcr.io/cloud-builders/docker`
> doesn't enable BuildKit by default. The `deploy/Dockerfile.mcp` uses
> `RUN --mount=type=cache,...` for cargo deps. Without
> `DOCKER_BUILDKIT=1` in the env, the build fails with "the --mount
> option requires BuildKit".

## Step 3 — Deploy

```bash
gcloud run deploy rulake-mcp-demo \
  --image=us-central1-docker.pkg.dev/YOUR-PROJECT/cloud-run-source-deploy/rulake-mcp-demo:latest \
  --region=us-central1 \
  --project=YOUR-PROJECT \
  --allow-unauthenticated \
  --port=8080 \
  --memory=512Mi --cpu=1 \
  --min-instances=1 --max-instances=1 \
  --timeout=60s \
  --set-env-vars="RUST_LOG=info" \
  --command=/usr/local/bin/rulake-mcp \
  --args="^|^http|--bind|0.0.0.0:8080|--auth|none|--insecure-allow-no-auth|--capabilities|read,publish,admin|--demo-backend"
```

Four things in there are non-obvious:

> **Gotcha — `--demo-backend`.** Without this flag the demo deploy ships
> with **zero registered backends**. Every `rulake_query` then refuses
> with `POLICY_REFUSED_ALLOWLIST` (the witness chain refuses to serve
> from an empty backend set), and `rulake_list_collections` returns
> empty. With `--demo-backend`, the server registers a deterministic
> `LocalBackend("demo")` with one seeded collection (`memory`, 100
> vectors at D=8, PCG32 seed `0xDEADBEEF`) and adds a matching
> `[[allow]]` block granting `read,publish` on `(demo, .*)`. Result:
> a fresh `rulake_query intent=search target.routes=[["demo","memory"]]`
> returns real witness-anchored hits out of the box, with no further
> operator action. Off in production deploys (the witness over a 100-row
> toy collection isn't useful there).

> **Gotcha — `--port=8080` + hardcoded bind.** Cloud Run sets `PORT`
> dynamically; the container is supposed to listen on `$PORT`. Our
> binary takes the bind address on the CLI, and distroless has no
> shell to do `--bind 0.0.0.0:$PORT` interpolation. Hardcoding `8080`
> on both sides works because Cloud Run defaults `--port` to 8080
> when not explicitly set.

> **Gotcha — `^|^` arg delimiter.** The default delimiter for `--args`
> is comma, but the capability list itself contains commas
> (`read,publish,admin`). The `^|^` syntax tells gcloud to split on
> `|` instead so the comma-list survives intact.

> **Gotcha — `--min-instances=1 --max-instances=1`.** rmcp's
> `LocalSessionManager` is process-local; sessions don't survive
> failover. With `min=0`, every cold start drops every session, and
> with `max>1`, follow-up MCP calls can land on a different instance
> and 401. For the demo wire we pin to one warm instance.

## Step 4 — `RULAKE_ALLOWED_HOSTS` (the proxy fix)

Cloud Run's frontend forwards requests with `Host` set to the
caller's user-facing hostname (e.g. `rulake-mcp.ruv.io`), not the
bind address. rmcp's DNS-rebinding guard rejects unknown Host values
with 403. Two fixes are bundled in `crates/mcp-server/src/http.rs`:

1. **Host allowlist** — set `RULAKE_ALLOWED_HOSTS` to a comma-
   separated list of every hostname operators will use. The repo
   includes the Cloud Run-generated host *and* the custom domain.
2. **Stable principal under proxy** — when `RULAKE_ALLOWED_HOSTS` is
   set, mcp-server uses `anon:proxied` instead of `anon:{peer}`, so
   sessions survive Cloud Run's per-request internal-IP rotation.

```bash
gcloud run services update rulake-mcp-demo \
  --region=us-central1 --project=YOUR-PROJECT \
  --update-env-vars="^|^RULAKE_ALLOWED_HOSTS=rulake-mcp-demo-NUMBERS.us-central1.run.app,rulake-mcp.YOURDOMAIN.io"
```

The `^|^` delimiter again — the env value contains a comma.

## Step 5 — Custom domain (optional)

The Cloud Run-generated URL works fine; the custom domain is purely
cosmetic + memorable. If you skip this, the Console's default endpoint
in `ui/src/components/screens.jsx` needs to be set to the
`*.run.app` URL instead.

### 5a — Cloud Run domain mapping

```bash
gcloud beta run domain-mappings create \
  --service=rulake-mcp-demo \
  --domain=rulake-mcp.YOURDOMAIN.io \
  --region=us-central1 --project=YOUR-PROJECT
```

This prints the CNAME target you need (`ghs.googlehosted.com.`).

### 5b — Cloudflare CNAME

```bash
CF_TOKEN=$(gcloud secrets versions access latest --secret=cloudflare-api-token)
ZONE_ID=$(curl -s "https://api.cloudflare.com/client/v4/zones?name=YOURDOMAIN.io" \
  -H "Authorization: Bearer $CF_TOKEN" | jq -r '.result[0].id')

curl -sX POST "https://api.cloudflare.com/client/v4/zones/${ZONE_ID}/dns_records" \
  -H "Authorization: Bearer $CF_TOKEN" -H "Content-Type: application/json" \
  -d '{
    "type":"CNAME",
    "name":"rulake-mcp",
    "content":"ghs.googlehosted.com",
    "ttl":1,
    "proxied":false
  }'
```

> **Gotcha — `proxied: false`.** Cloud Run's domain mapping verifies
> via `01-challenge` over HTTP. If Cloudflare proxying is on (orange
> cloud), the verifier hits Cloudflare's edge instead of `ghs.*` and
> the cert never provisions. Keep proxying OFF until cert is live;
> you can flip it on later if you want Cloudflare's WAF in front, but
> Cloud Run's already terminating TLS so there's not much win.

### 5c — Wait for cert

Let's Encrypt cert provisioning takes 5–15 min once the CNAME
resolves. Track with:

```bash
gcloud beta run domain-mappings describe \
  --domain=rulake-mcp.YOURDOMAIN.io \
  --region=us-central1 --project=YOUR-PROJECT \
  --format="value(status.conditions[].type:label=cond,status.conditions[].status:label=stat)"
```

Wait until `CertificateProvisioned=True`.

## Step 6 — Smoke-test

The repo ships [`scripts/smoke-live.sh`](../../scripts/smoke-live.sh)
that exercises the production wire end-to-end:

```bash
URL=https://rulake-mcp.YOURDOMAIN.io/ ./scripts/smoke-live.sh
```

Eleven assertions: TLS handshake → 5 CORS preflight headers → MCP
handshake (initialize + notifications/initialized) → tools/list returns
exactly the expected count. ~3 s wall, no Chrome required. If this
goes red, the deploy regressed somewhere.

## Step 7 — Wire the Console (optional)

If you set up a custom domain, update the Console's default endpoint:

```js
// ui/src/components/screens.jsx ConnectScreen
const [endpoint, setEndpoint] = useState('https://rulake-mcp.YOURDOMAIN.io/');
```

The Topbar's auto-probe (in `Topbar`) already hits this URL on boot;
visitors land in DEMO and the pill flips to `● LIVE` once the probe
completes.

## Cost

Cloud Run free tier: 2M requests/month, 360k GB-s, 180k CPU-s. For a
demo wire with `min=1 max=1 512Mi 1cpu`:

- Idle: ~$5/mo for the always-warm instance (the `min=1` part)
- Per request: free up to the 2M ceiling

If you don't mind a 2-3s cold start every time the Console loads, drop
to `min=0` and the cost is essentially zero. The session-stickiness
caveat above still applies (every cold start drops all in-flight
sessions).

## Iteration history

These commits were the load-bearing fixes (all on `main`):

| Commit | What it fixed |
|---|---|
| `73e2ef8` | `.gcloudignore` (3 GB → 28 MB upload) |
| `99a9658` | mcp-server pinned `rulake = "2.3.0-alpha.1"` (post-ADR-157 root bump) |
| `4248b75` | `RULAKE_ALLOWED_HOSTS` env var for reverse-proxy deployments |
| `2427543` | Stable principal under `RULAKE_ALLOWED_HOSTS` (Cloud Run sessions) |
| `c9bde11` + `84d4bf5` | UI fix unrelated, but exposed in same iter window — `var(--ink-1)` typos |
| `a493f77` | Console default endpoint → `https://rulake-mcp.ruv.io/` |
| `84d4bf5` | Console auto-probe live MCP at boot |

For the original session log: see `CHANGELOG.md` under "Added — live demo (iter 51-54)".
