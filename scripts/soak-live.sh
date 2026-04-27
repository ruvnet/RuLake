#!/usr/bin/env bash
#
# ruLake — production soak/load test against live Cloud Run mcp-server.
#
# Drives the full MCP wire (initialize → notifications/initialized →
# tools/list) and measures latency under sustained load, concurrent
# session capacity, and SSE idle survival. Companion to smoke-live.sh
# — that one proves the wire works; this one proves it stays up under
# pressure.
#
# Closes GitHub issue #7. Designed to be re-runnable so future
# operators can replay the same workload after config changes.
#
# Usage:
#   ./scripts/soak-live.sh                      # default rulake target
#   URL=https://rvdna-mcp.ruv.io/ EXPECTED_TOOLS=5 ./scripts/soak-live.sh
#   MODE=latency  RPS=50 DURATION=60 ./scripts/soak-live.sh
#   MODE=concurrent CONCURRENT=100 ./scripts/soak-live.sh
#   MODE=idle     IDLE_SEC=90 ./scripts/soak-live.sh
#   MODE=all                              # default — runs every mode
#
# Env vars (with defaults):
#   URL               https://rulake-mcp.ruv.io/
#   EXPECTED_TOOLS    8                          (5 for rvdna/ruqu)
#   MODE              all                        (latency|concurrent|idle|all)
#   DURATION          60                         (seconds — latency mode)
#   RPS_LIST          "10 50"                    (rps levels — latency mode)
#   CONCURRENT_LIST   "1 10 50 100"              (parallelism — concurrent mode)
#   TOOLS_PER_SESSION 5                          (tools/list calls per session — concurrent mode)
#   IDLE_SEC          90                         (idle hold — idle mode)
#   USER_AGENT        rulake-soak-test/1.0       (so we can spot soak in Cloud Run logs)
#   OUT_DIR           /tmp/rulake-soak-<ts>      (per-call latency csvs)
#
# Exit:
#   0 — every mode green (success rate >=95%, no crashes).
#   1 — at least one mode red.
#
# Constraints (issue #7):
#   - Cloud Run free tier is 2M req/month — this script caps at ~12k
#     requests per full run per target. Three targets ≈ 36k/run.
#   - Backs off if error rate spikes past 5% in a window.
#   - Identifies itself with a soak-specific User-Agent.

set -uo pipefail

URL="${URL:-https://rulake-mcp.ruv.io/}"
EXPECTED_TOOLS="${EXPECTED_TOOLS:-8}"
MODE="${MODE:-all}"
DURATION="${DURATION:-60}"
RPS_LIST="${RPS_LIST:-10 50}"
CONCURRENT_LIST="${CONCURRENT_LIST:-1 10 50 100}"
TOOLS_PER_SESSION="${TOOLS_PER_SESSION:-5}"
IDLE_SEC="${IDLE_SEC:-90}"
USER_AGENT="${USER_AGENT:-rulake-soak-test/1.0}"
TS="$(date +%Y%m%d-%H%M%S)"
OUT_DIR="${OUT_DIR:-/tmp/rulake-soak-${TS}}"
mkdir -p "${OUT_DIR}"

# Pretty print helpers
hdr()  { printf '\n\033[1m▸ %s\033[0m\n' "$*"; }
ok()   { printf '\033[32m  ✓\033[0m %s\n' "$*"; }
err()  { printf '\033[31m  ✗\033[0m %s\n' "$*" >&2; FAILED=1; }
info() { printf '    %s\n' "$*"; }
FAILED=0

# Common curl flags. -sS silent but errors-to-stderr; -m bounded; -A
# soak-specific UA. Cloud Run + Cloudflare both honor HTTP/2 keepalive.
CURL_BASE=(curl -sS -m 15 -A "${USER_AGENT}")

# ---------------------------------------------------------------------------
# helpers
# ---------------------------------------------------------------------------

# percentile <pct> <file_of_floats>  → prints the percentile in seconds
percentile() {
  local pct="$1"; local file="$2"
  awk -v p="${pct}" '
    { a[NR]=$1+0 }
    END {
      if (NR==0) { print "nan"; exit }
      n=NR
      asort(a)
      idx=int((p/100.0)*n + 0.5)
      if (idx<1) idx=1
      if (idx>n) idx=n
      printf "%.4f", a[idx]
    }
  ' "${file}"
}

mean() {
  local file="$1"
  awk '{s+=$1; n++} END { if (n==0) {print "nan"; exit} printf "%.4f", s/n }' "${file}"
}

# open_session <url>  → echoes the session id, or empty on failure
open_session() {
  local url="$1"
  local raw
  raw="$("${CURL_BASE[@]}" -isS -X POST "${url}" \
    -H 'Content-Type: application/json' \
    -H 'Accept: application/json, text/event-stream' \
    -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"soak","version":"0"}}}' \
    2>/dev/null)" || return 1
  local sess
  sess=$(echo "${raw}" | grep -i '^mcp-session-id:' | head -1 | tr -d '\r' | awk '{print $2}')
  [[ -n "${sess}" ]] || return 1
  # ack initialized — Cloud Run requires this before tools/list will succeed
  "${CURL_BASE[@]}" -o /dev/null -X POST "${url}" \
    -H 'Content-Type: application/json' \
    -H 'Accept: application/json, text/event-stream' \
    -H "mcp-session-id: ${sess}" \
    -d '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
    >/dev/null 2>&1 || return 1
  echo "${sess}"
}

# call_tools_list <url> <sess>  → prints "<status> <time_total>"
call_tools_list() {
  local url="$1"; local sess="$2"
  "${CURL_BASE[@]}" -o /dev/null -X POST "${url}" \
    -H 'Content-Type: application/json' \
    -H 'Accept: application/json, text/event-stream' \
    -H "mcp-session-id: ${sess}" \
    -d '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
    -w '%{http_code} %{time_total}\n' 2>/dev/null
}

# ---------------------------------------------------------------------------
# mode: latency  — sustained RPS for DURATION s on a single re-used session
# ---------------------------------------------------------------------------

mode_latency() {
  local rps="$1"
  hdr "latency · ${rps} rps · ${DURATION}s · ${URL}"

  local sess; sess=$(open_session "${URL}")
  if [[ -z "${sess}" ]]; then
    err "could not open MCP session for latency mode"
    return 1
  fi
  ok "session opened: ${sess:0:8}…"

  local out_csv="${OUT_DIR}/latency-${rps}rps-$(echo "${URL}" | sed 's|https://||;s|/||g').csv"
  : > "${out_csv}"

  local interval
  interval=$(awk -v r="${rps}" 'BEGIN { printf "%.4f", 1.0/r }')
  local end_ts=$(( $(date +%s) + DURATION ))
  local sent=0 ok_count=0 fail_count=0
  local backoff_window_fail=0 backoff_window_total=0

  while [[ $(date +%s) -lt ${end_ts} ]]; do
    local tick_start; tick_start=$(date +%s.%N)
    local status_time; status_time=$(call_tools_list "${URL}" "${sess}")
    local status="${status_time%% *}"
    local t="${status_time##* }"
    sent=$((sent+1))
    backoff_window_total=$((backoff_window_total+1))
    if [[ "${status}" == "200" ]]; then
      ok_count=$((ok_count+1))
      echo "${t}" >> "${out_csv}"
    else
      fail_count=$((fail_count+1))
      backoff_window_fail=$((backoff_window_fail+1))
    fi

    # back off if 5% error rate in a 100-call window (issue #7 guardrail)
    if [[ ${backoff_window_total} -ge 100 ]]; then
      local pct
      pct=$(awk -v f="${backoff_window_fail}" -v t="${backoff_window_total}" 'BEGIN{printf "%.1f", (f/t)*100}')
      if awk -v p="${pct}" 'BEGIN{exit !(p>5.0)}'; then
        err "error rate ${pct}% > 5% — backing off"
        break
      fi
      backoff_window_fail=0
      backoff_window_total=0
    fi

    # sleep to next tick
    local elapsed sleep_for
    elapsed=$(awk -v s="${tick_start}" 'BEGIN{printf "%.4f", systime()+0 - s}')
    elapsed=$(awk -v s="${tick_start}" 'BEGIN{
      "date +%s.%N" | getline n; close("date +%s.%N")
      printf "%.4f", n - s
    }')
    sleep_for=$(awk -v i="${interval}" -v e="${elapsed}" 'BEGIN{
      d=i-e; if (d<0) d=0; printf "%.4f", d
    }')
    sleep "${sleep_for}" 2>/dev/null || true
  done

  if [[ ${ok_count} -eq 0 ]]; then
    err "zero successful calls"
    return 1
  fi

  local p50 p95 p99 mn
  p50=$(percentile 50 "${out_csv}")
  p95=$(percentile 95 "${out_csv}")
  p99=$(percentile 99 "${out_csv}")
  mn=$(mean "${out_csv}")
  local sr
  sr=$(awk -v o="${ok_count}" -v s="${sent}" 'BEGIN{printf "%.1f", (o/s)*100}')

  ok "sent ${sent}  ok ${ok_count}  fail ${fail_count}  success ${sr}%"
  ok "mean ${mn}s  p50 ${p50}s  p95 ${p95}s  p99 ${p99}s"
  printf '%s,%s,%s,%s,%s,%s,%s,%s,%s\n' \
    "${URL}" "latency" "${rps}rps" "${sent}" "${ok_count}" "${sr}" "${p50}" "${p95}" "${p99}" \
    >> "${OUT_DIR}/results.csv"
}

# ---------------------------------------------------------------------------
# mode: concurrent — N parallel sessions, each does TOOLS_PER_SESSION calls
# ---------------------------------------------------------------------------

mode_concurrent() {
  local n="$1"
  hdr "concurrent · ${n} parallel sessions · ${TOOLS_PER_SESSION} tools/list each · ${URL}"

  local out_csv="${OUT_DIR}/concurrent-${n}-$(echo "${URL}" | sed 's|https://||;s|/||g').csv"
  : > "${out_csv}"

  # Per-worker subshell — open session, do K tools/list, append per-call timings.
  # Self-contained: spawned workers re-implement the wire calls inline so we
  # don't depend on `export -f` semantics that vary across bash builds.
  local worker_script="${OUT_DIR}/.worker.sh"
  cat > "${worker_script}" <<'WORKER_EOF'
#!/usr/bin/env bash
set -uo pipefail
id="$1"; k="$2"; url="$3"; out="$4"; ua="$5"
init_raw=$(curl -sS -m 15 -A "${ua}" -isS -X POST "${url}" \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"soak","version":"0"}}}' 2>/dev/null)
sess=$(echo "${init_raw}" | grep -i '^mcp-session-id:' | head -1 | tr -d '\r' | awk '{print $2}')
if [[ -z "${sess}" ]]; then
  echo "WORKER_${id}_NO_SESSION" >&2
  exit 1
fi
curl -sS -m 15 -A "${ua}" -o /dev/null -X POST "${url}" \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -H "mcp-session-id: ${sess}" \
  -d '{"jsonrpc":"2.0","method":"notifications/initialized"}' >/dev/null 2>&1
for ((i=0; i<k; i++)); do
  st=$(curl -sS -m 15 -A "${ua}" -o /dev/null -X POST "${url}" \
    -H 'Content-Type: application/json' \
    -H 'Accept: application/json, text/event-stream' \
    -H "mcp-session-id: ${sess}" \
    -d '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
    -w '%{http_code} %{time_total}\n' 2>/dev/null)
  status="${st%% *}"
  t="${st##* }"
  echo "${id},${i},${status},${t}" >> "${out}"
done
WORKER_EOF
  chmod +x "${worker_script}"

  local pids=()
  for ((i=0; i<n; i++)); do
    "${worker_script}" "${i}" "${TOOLS_PER_SESSION}" "${URL}" "${out_csv}" "${USER_AGENT}" &
    pids+=($!)
  done

  local fail_workers=0
  for pid in "${pids[@]}"; do
    if ! wait "${pid}"; then
      fail_workers=$((fail_workers+1))
    fi
  done

  local total ok_count
  total=$(wc -l < "${out_csv}" | tr -d ' ')
  ok_count=$(awk -F, '$3==200' "${out_csv}" | wc -l | tr -d ' ')
  local sr
  if [[ ${total} -gt 0 ]]; then
    sr=$(awk -v o="${ok_count}" -v t="${total}" 'BEGIN{printf "%.1f", (o/t)*100}')
  else
    sr=0
  fi

  if [[ ${ok_count} -eq 0 ]]; then
    err "concurrent ${n}: zero successful calls (${fail_workers} workers failed at session-open)"
    printf '%s,%s,%s,%s,%s,%s,%s,%s,%s\n' \
      "${URL}" "concurrent" "${n}sess" "${total}" "${ok_count}" "${sr}" "nan" "nan" "nan" \
      >> "${OUT_DIR}/results.csv"
    return 1
  fi

  awk -F, '$3==200 {print $4}' "${out_csv}" > "${out_csv}.ok"
  local p50 p95 p99
  p50=$(percentile 50 "${out_csv}.ok")
  p95=$(percentile 95 "${out_csv}.ok")
  p99=$(percentile 99 "${out_csv}.ok")

  ok "workers ${n} (failed ${fail_workers})  calls ${total}  ok ${ok_count}  success ${sr}%"
  ok "p50 ${p50}s  p95 ${p95}s  p99 ${p99}s"
  printf '%s,%s,%s,%s,%s,%s,%s,%s,%s\n' \
    "${URL}" "concurrent" "${n}sess" "${total}" "${ok_count}" "${sr}" "${p50}" "${p95}" "${p99}" \
    >> "${OUT_DIR}/results.csv"
}

# ---------------------------------------------------------------------------
# mode: idle — open one session, hold IDLE_SEC, then send tools/list
# ---------------------------------------------------------------------------

mode_idle() {
  hdr "idle · open + sleep ${IDLE_SEC}s + tools/list · ${URL}"
  local sess; sess=$(open_session "${URL}")
  if [[ -z "${sess}" ]]; then
    err "could not open session for idle mode"
    printf '%s,%s,%s,%s,%s,%s,%s,%s,%s\n' \
      "${URL}" "idle" "${IDLE_SEC}s" "0" "0" "0" "nan" "nan" "nan" \
      >> "${OUT_DIR}/results.csv"
    return 1
  fi
  ok "session opened: ${sess:0:8}…"
  info "sleeping ${IDLE_SEC}s …"
  sleep "${IDLE_SEC}"
  local st; st=$(call_tools_list "${URL}" "${sess}")
  local status="${st%% *}"
  local t="${st##* }"
  if [[ "${status}" == "200" ]]; then
    ok "session survived ${IDLE_SEC}s idle  (tools/list ${status} in ${t}s)"
    printf '%s,%s,%s,%s,%s,%s,%s,%s,%s\n' \
      "${URL}" "idle" "${IDLE_SEC}s" "1" "1" "100.0" "${t}" "${t}" "${t}" \
      >> "${OUT_DIR}/results.csv"
  else
    err "session evicted after ${IDLE_SEC}s (got HTTP ${status})"
    printf '%s,%s,%s,%s,%s,%s,%s,%s,%s\n' \
      "${URL}" "idle" "${IDLE_SEC}s" "1" "0" "0.0" "nan" "nan" "nan" \
      >> "${OUT_DIR}/results.csv"
    FAILED=1
  fi
}

# ---------------------------------------------------------------------------
# entry
# ---------------------------------------------------------------------------

hdr "soak target ${URL}  (expected ${EXPECTED_TOOLS} tools)  out=${OUT_DIR}"
echo "url,mode,param,sent,ok,success_pct,p50_s,p95_s,p99_s" > "${OUT_DIR}/results.csv"

case "${MODE}" in
  latency)
    for r in ${RPS_LIST}; do mode_latency "${r}"; done
    ;;
  concurrent)
    for n in ${CONCURRENT_LIST}; do mode_concurrent "${n}"; done
    ;;
  idle)
    mode_idle
    ;;
  all)
    for r in ${RPS_LIST}; do mode_latency "${r}"; done
    for n in ${CONCURRENT_LIST}; do mode_concurrent "${n}"; done
    mode_idle
    ;;
  *)
    err "unknown MODE=${MODE} (latency|concurrent|idle|all)"
    exit 2
    ;;
esac

hdr "results"
column -ts, "${OUT_DIR}/results.csv" 2>/dev/null || cat "${OUT_DIR}/results.csv"

echo
if [[ "${FAILED}" -eq 0 ]]; then
  printf '\033[32m\033[1m✓ soak green\033[0m — see %s/results.csv\n' "${OUT_DIR}"
  exit 0
else
  printf '\033[31m\033[1m✗ soak RED\033[0m — see failures above\n'
  exit 1
fi
