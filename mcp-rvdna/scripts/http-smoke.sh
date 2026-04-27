#!/usr/bin/env bash
#
# mcp-rvdna v0.0.1 — end-to-end HTTP smoke.
#
# Builds + launches rvdna-mcp on a free port, walks the MCP handshake
# (initialize → notifications/initialized → tools/list), asserts the
# response carries the five expected tool names, and asserts the
# trust-anchor tool (rvdna_lineage) is present when --capabilities
# internal is granted. Tears the server down on exit.
#
# Companion to ui/scripts/smoke.sh — that one validates the Console
# in WASM-local + live-mcp-server modes; this one validates that the
# rvdna-mcp binary actually runs and serves over HTTP.
#
# Usage:
#   ./mcp-rvdna/scripts/http-smoke.sh
#
# Exit:
#   0 on green; 1 on any assertion failure or curl error.

set -euo pipefail

CRATE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIND_HOST=127.0.0.1
BIND_PORT=17441   # high port to avoid clashing with the default 7441
BIND="${BIND_HOST}:${BIND_PORT}"
URL="http://${BIND}/mcp"
LOG=/tmp/rvdna-mcp-smoke.log

ok()   { printf '\033[32m  ✓\033[0m %s\n' "$*"; }
err()  { printf '\033[31m  ✗\033[0m %s\n' "$*" >&2; FAILED=1; }
hdr()  { printf '\n\033[1m▸ %s\033[0m\n' "$*"; }
info() { printf '    %s\n' "$*"; }

FAILED=0

cleanup() {
  if [[ -n "${MCP_PID:-}" ]] && kill -0 "$MCP_PID" 2>/dev/null; then
    kill -TERM "$MCP_PID" 2>/dev/null || true
    sleep 0.3
    kill -9 "$MCP_PID" 2>/dev/null || true
  fi
  pkill -9 -f rvdna-mcp 2>/dev/null || true
}
trap cleanup EXIT

hdr "build"
cargo build --release --manifest-path "${CRATE_DIR}/Cargo.toml" --bin rvdna-mcp 2>&1 | tail -2
ok "rvdna-mcp built"

hdr "launch"
"${CRATE_DIR}/target/release/rvdna-mcp" http \
  --bind "${BIND}" \
  --capabilities read,internal \
  > "${LOG}" 2>&1 &
MCP_PID=$!
sleep 1.2
if ! kill -0 "$MCP_PID" 2>/dev/null; then
  err "rvdna-mcp died at launch — see ${LOG}"
  cat "${LOG}" >&2
  exit 1
fi
ok "rvdna-mcp listening on ${BIND} (pid ${MCP_PID})"

hdr "CORS preflight (iter 32 regression — Console at :4173 must be allowed)"
# OPTIONS request mimicking a browser preflight from the Console.
# Iter 32 fixed mcp-rvdna's missing CORS layer; this guards against
# a regression that would otherwise only surface in the cross-mcp
# browser smoke.
PREFLIGHT=$(curl -isS -X OPTIONS "${URL}" \
  -H 'Origin: http://localhost:4173' \
  -H 'Access-Control-Request-Method: POST' \
  -H 'Access-Control-Request-Headers: content-type, mcp-session-id')
PRE_STATUS=$(echo "${PREFLIGHT}" | head -1 | awk '{print $2}')
if [[ "${PRE_STATUS}" == "204" ]]; then
  ok "preflight returned 204 No Content"
else
  err "preflight returned ${PRE_STATUS}, expected 204"
fi
for header in 'access-control-allow-origin: http://localhost:4173' \
              'access-control-allow-methods:' \
              'access-control-allow-headers:' \
              'access-control-expose-headers: mcp-session-id'; do
  if echo "${PREFLIGHT}" | grep -qi "^${header}"; then
    ok "preflight carries: ${header%%:*}"
  else
    err "preflight missing header: ${header%%:*}"
  fi
done

hdr "MCP handshake"
INIT_RAW=$(curl -isS -X POST "${URL}" \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"smoke","version":"0"}}}')
SESSION=$(echo "${INIT_RAW}" | grep -i '^mcp-session-id:' | head -1 | tr -d '\r' | awk '{print $2}')
if [[ -z "${SESSION}" ]]; then
  err "initialize did not return mcp-session-id"
  echo "${INIT_RAW}" | head -10 >&2
  exit 1
fi
ok "session id received: ${SESSION:0:8}..."

curl -fsS -X POST "${URL}" \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -H "mcp-session-id: ${SESSION}" \
  -d '{"jsonrpc":"2.0","method":"notifications/initialized"}' > /dev/null
ok "notifications/initialized accepted"

hdr "tools/list"
TOOLS_RAW=$(curl -fsS -X POST "${URL}" \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -H "mcp-session-id: ${SESSION}" \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}')
# Response is SSE-framed; extract the JSON line that starts with `data:` then jq it.
TOOLS_JSON=$(echo "${TOOLS_RAW}" | grep -E '^data: ' | grep -v '^data: $' | head -1 | sed 's/^data: //')
if [[ -z "${TOOLS_JSON}" ]]; then
  err "tools/list returned no SSE data line"
  echo "${TOOLS_RAW}" | head -10 >&2
  exit 1
fi

EXPECTED=(rvdna_find rvdna_call_variants rvdna_translate rvdna_score rvdna_lineage)
for tool in "${EXPECTED[@]}"; do
  if echo "${TOOLS_JSON}" | grep -q "\"name\":\"${tool}\""; then
    ok "tool present: ${tool}"
  else
    err "tool missing: ${tool}"
  fi
done

hdr "tools/call rvdna_lineage (no backend registered → expect RVDNA_UNKNOWN_COLLECTION)"
# The binary starts with an empty registry — no backends pinned at
# launch in v0.0.1. Calling rvdna_lineage against an unknown
# (backend, collection) MUST refuse cleanly with the documented code,
# not panic or return a malformed envelope. This proves the error
# path of the trust-anchor tool over HTTP, complementing the in-process
# integration test in tests/http_e2e.rs.
LINEAGE_RAW=$(curl -fsS -X POST "${URL}" \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -H "mcp-session-id: ${SESSION}" \
  -d '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"rvdna_lineage","arguments":{"backend":"nope","collection":"nope"}}}')
LINEAGE_JSON=$(echo "${LINEAGE_RAW}" | grep -E '^data: ' | grep -v '^data: $' | head -1 | sed 's/^data: //')
if [[ -z "${LINEAGE_JSON}" ]]; then
  err "rvdna_lineage returned no SSE data line"
  echo "${LINEAGE_RAW}" | head -10 >&2
elif echo "${LINEAGE_JSON}" | grep -q 'RVDNA_UNKNOWN_COLLECTION'; then
  ok "refusal carries RVDNA_UNKNOWN_COLLECTION"
else
  err "expected RVDNA_UNKNOWN_COLLECTION refusal, got: ${LINEAGE_JSON:0:200}..."
fi

if [[ "${FAILED}" -eq 0 ]]; then
  echo
  printf '\033[32m\033[1m✓ mcp-rvdna http smoke green\033[0m — all 5 tools served, trust-anchor refusal path verified\n'
  exit 0
else
  echo
  printf '\033[31m\033[1m✗ mcp-rvdna http smoke RED\033[0m — see failures above\n'
  exit 1
fi
