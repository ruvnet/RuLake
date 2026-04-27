#!/usr/bin/env bash
#
# mcp-ruqu v0.1 — end-to-end HTTP smoke.
#
# Builds + launches ruqu-mcp on a free port, walks the MCP handshake
# (initialize → notifications/initialized → tools/list), asserts the
# response carries the five expected ruqu_* tool names, and asserts
# that the `ruqu_optimize` stub returns the documented
# `RUQU_OPTIMIZE_STUB` audit code over the wire. Tears the server down
# on exit.
#
# Companion to mcp-rvdna/scripts/http-smoke.sh — same iter 32 + 41
# CORS contract (5 preflight headers), same SSE-framed tools/list
# parsing.
#
# Usage:
#   ./mcp-ruqu/scripts/http-smoke.sh
#
# Exit:
#   0 on green; 1 on any assertion failure or curl error.

set -euo pipefail

CRATE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIND_HOST=127.0.0.1
BIND_PORT=17442   # high port to avoid clashing with the default 7442
BIND="${BIND_HOST}:${BIND_PORT}"
URL="http://${BIND}/mcp"
LOG=/tmp/ruqu-mcp-smoke.log

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
  pkill -9 -f ruqu-mcp 2>/dev/null || true
}
trap cleanup EXIT

hdr "build"
cargo build --release --manifest-path "${CRATE_DIR}/Cargo.toml" --bin ruqu-mcp 2>&1 | tail -2
ok "ruqu-mcp built"

hdr "launch"
"${CRATE_DIR}/target/release/ruqu-mcp" http \
  --bind "${BIND}" \
  > "${LOG}" 2>&1 &
MCP_PID=$!
sleep 1.2
if ! kill -0 "$MCP_PID" 2>/dev/null; then
  err "ruqu-mcp died at launch — see ${LOG}"
  cat "${LOG}" >&2
  exit 1
fi
ok "ruqu-mcp listening on ${BIND} (pid ${MCP_PID})"

hdr "CORS preflight (iter 41 contract — Console at :4173 must be allowed)"
# OPTIONS request mimicking a browser preflight from the Console.
# v0.1 lifts mcp-rvdna's iter 32 CORS layer wholesale; this guards
# against a regression that would otherwise only surface in the
# cross-mcp browser smoke.
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
              'access-control-expose-headers: mcp-session-id' \
              'access-control-max-age:'; do
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

hdr "tools/list (5 ruqu_* tools expected)"
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

EXPECTED=(ruqu_simulate ruqu_verify ruqu_replay ruqu_optimize ruqu_qec_schedule)
for tool in "${EXPECTED[@]}"; do
  if echo "${TOOLS_JSON}" | grep -q "\"name\":\"${tool}\""; then
    ok "tool present: ${tool}"
  else
    err "tool missing: ${tool}"
  fi
done

hdr "tools/call ruqu_optimize (stub → expect RUQU_OPTIMIZE_STUB in note)"
# `ruqu_optimize` is a v0.0.1 stub (planner ships in v0.2). The
# response envelope MUST carry `RUQU_OPTIMIZE_STUB` in the `note`
# field — same string the audit row carries — so the wire and audit
# stories agree. This proves the live tool path over HTTP without
# requiring a backend / lake to be wired in.
OPT_RAW=$(curl -fsS -X POST "${URL}" \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -H "mcp-session-id: ${SESSION}" \
  -d '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"ruqu_optimize","arguments":{"circuit":{"n_qubits":2,"gates":[{"kind":"h","q":0}],"id":"smoke-optimize"}}}}')
OPT_JSON=$(echo "${OPT_RAW}" | grep -E '^data: ' | grep -v '^data: $' | head -1 | sed 's/^data: //')
if [[ -z "${OPT_JSON}" ]]; then
  err "ruqu_optimize returned no SSE data line"
  echo "${OPT_RAW}" | head -10 >&2
elif echo "${OPT_JSON}" | grep -q 'RUQU_OPTIMIZE_STUB'; then
  ok "stub envelope carries RUQU_OPTIMIZE_STUB"
else
  err "expected RUQU_OPTIMIZE_STUB in response, got: ${OPT_JSON:0:200}..."
fi

if [[ "${FAILED}" -eq 0 ]]; then
  echo
  printf '\033[32m\033[1m✓ mcp-ruqu http smoke green\033[0m — all 5 tools served, RUQU_OPTIMIZE_STUB path verified\n'
  exit 0
else
  echo
  printf '\033[31m\033[1m✗ mcp-ruqu http smoke RED\033[0m — see failures above\n'
  exit 1
fi
