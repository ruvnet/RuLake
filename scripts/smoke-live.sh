#!/usr/bin/env bash
#
# ruLake — production wire smoke against the live Cloud Run deploy.
#
# Hits https://rulake-mcp.ruv.io/ (Cloudflare CNAME → Cloud Run domain
# mapping → mcp-server) and asserts the full MCP handshake plus an
# 8-tool tools/list response. ~3 s wall, no Chrome required.
#
# This is the production sibling to:
#   - ui/scripts/smoke.sh            (Console WASM-local mode)
#   - crates/mcp-rvdna/scripts/http-smoke.sh (rvdna-mcp HTTP, in-isolation)
#   - ui/scripts/smoke-cross-mcp.sh  (Console + local rvdna-mcp wire)
#
# What it catches that the local smokes don't:
#   - Cert expiry on rulake-mcp.ruv.io
#   - Cloudflare CNAME drift / accidental proxy-on (which breaks
#     Cloud Run's domain mapping)
#   - Cloud Run service downtime / scale-to-zero failures
#   - mcp-server's RULAKE_ALLOWED_HOSTS / proxy-mode behavior under
#     real Google frontend conditions (per-request peer IP rotation,
#     SSE forwarding, etc.)
#
# Usage:
#   ./scripts/smoke-live.sh
#   URL=https://your-other-mcp.example/ ./scripts/smoke-live.sh
#
# Exit:
#   0 — green
#   1 — any assertion failed

set -uo pipefail

URL="${URL:-https://rulake-mcp.ruv.io/}"
EXPECTED_TOOLS="${EXPECTED_TOOLS:-8}"

ok()   { printf '\033[32m  ✓\033[0m %s\n' "$*"; }
err()  { printf '\033[31m  ✗\033[0m %s\n' "$*" >&2; FAILED=1; }
hdr()  { printf '\n\033[1m▸ %s\033[0m\n' "$*"; }
info() { printf '    %s\n' "$*"; }
FAILED=0

hdr "TLS reachable"
TLS_HEAD=$(curl -sI -m 10 "${URL}" 2>&1 | head -1 | awk '{print $2}')
if [[ -n "${TLS_HEAD}" ]]; then
  ok "TLS handshake completed (HTTP ${TLS_HEAD})"
else
  err "TLS handshake failed against ${URL}"
  exit 1
fi

hdr "CORS preflight (browser callers)"
PRE=$(curl -isS -m 10 -X OPTIONS "${URL}" \
  -H 'Origin: https://ruvnet.github.io' \
  -H 'Access-Control-Request-Method: POST' \
  -H 'Access-Control-Request-Headers: content-type, mcp-session-id')
PRE_STATUS=$(echo "${PRE}" | head -1 | awk '{print $2}')
if [[ "${PRE_STATUS}" == "204" ]]; then
  ok "preflight returned 204 No Content"
else
  err "preflight returned ${PRE_STATUS:-<empty>}, expected 204"
fi
for header in 'access-control-allow-origin: https://ruvnet.github.io' \
              'access-control-allow-methods:' \
              'access-control-allow-headers:' \
              'access-control-expose-headers: mcp-session-id'; do
  if echo "${PRE}" | grep -qi "^${header}"; then
    ok "preflight carries: ${header%%:*}"
  else
    err "preflight missing: ${header%%:*}"
  fi
done

hdr "MCP handshake"
INIT=$(curl -isS -m 15 -X POST "${URL}" \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"smoke-live","version":"0"}}}')
SESS=$(echo "${INIT}" | grep -i '^mcp-session-id:' | head -1 | tr -d '\r' | awk '{print $2}')
if [[ -n "${SESS}" ]]; then
  ok "initialize → 200, session ${SESS:0:8}..."
else
  err "initialize did not return mcp-session-id"
  echo "${INIT}" | head -10 >&2
  exit 1
fi

NOTIFY_STATUS=$(curl -sS -m 10 -o /dev/null -w '%{http_code}' -X POST "${URL}" \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -H "mcp-session-id: ${SESS}" \
  -d '{"jsonrpc":"2.0","method":"notifications/initialized"}')
if [[ "${NOTIFY_STATUS}" == "202" || "${NOTIFY_STATUS}" == "200" ]]; then
  ok "notifications/initialized → ${NOTIFY_STATUS}"
else
  err "notifications/initialized returned ${NOTIFY_STATUS}, expected 202"
fi

hdr "tools/list"
TOOLS=$(curl -fsS -m 15 -X POST "${URL}" \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -H "mcp-session-id: ${SESS}" \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
  | grep -E '^data: \{' | head -1 | sed 's/^data: //')
if [[ -z "${TOOLS}" ]]; then
  err "tools/list returned no SSE data line"
  exit 1
fi
COUNT=$(echo "${TOOLS}" | python3 -c "import sys,json; d=json.load(sys.stdin); print(len(d['result']['tools']))" 2>/dev/null)
if [[ "${COUNT}" == "${EXPECTED_TOOLS}" ]]; then
  ok "tools/list returned ${COUNT} tools (expected ${EXPECTED_TOOLS})"
  echo "${TOOLS}" | python3 -c "import sys,json; print('     ' + ', '.join(t['name'] for t in json.load(sys.stdin)['result']['tools']))" 2>/dev/null
else
  err "expected ${EXPECTED_TOOLS} tools, got ${COUNT:-<unparseable>}"
fi

echo
if [[ "${FAILED}" -eq 0 ]]; then
  printf '\033[32m\033[1m✓ smoke-live green\033[0m — production MCP at %s healthy\n' "${URL}"
  exit 0
else
  printf '\033[31m\033[1m✗ smoke-live RED\033[0m — see failures above\n'
  exit 1
fi
