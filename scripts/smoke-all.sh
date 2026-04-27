#!/usr/bin/env bash
#
# ruLake — run all end-to-end smokes in sequence with a unified summary.
#
# Drop-in for CI or the local dev loop. Each smoke runs in its own
# subprocess so a failure in one doesn't poison the next; the overall
# exit code is non-zero if any one failed. Total wall time on a warm
# build is ~90 s.
#
# Smokes covered (matches the README's End-to-end smoke contracts table):
#
#   1. ui/scripts/smoke.sh                 — Console WASM-local mode.
#                                             7 routes, 5 audit codes,
#                                             0 console errors, App
#                                             store 4 cards.
#   2. ui/scripts/smoke-cross-mcp.sh       — Console + mcp-rvdna full
#                                             wire (CORS, SSE parser,
#                                             Browse refusal, INIT_OK).
#   3. mcp-rvdna/scripts/http-smoke.sh     — rvdna-mcp HTTP transport
#                                             in isolation (handshake
#                                             + 5 tools + refusal path).
#   4. mcp-ruqu/scripts/http-smoke.sh      — ruqu-mcp HTTP transport
#                                             in isolation (same iter 41
#                                             CORS contract; 5 ruqu_*
#                                             tools + RUQU_OPTIMIZE_STUB
#                                             stub-envelope check).
#
# Usage:
#   ./scripts/smoke-all.sh
#   ./scripts/smoke-all.sh --skip-cross    # skip the slower cross-mcp smoke
#
# Exit:
#   0 — all smokes green
#   1 — at least one smoke failed (see per-smoke output above)

set -uo pipefail

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

SKIP_CROSS=0
for arg in "$@"; do
  case "$arg" in
    --skip-cross) SKIP_CROSS=1 ;;
    *)            printf 'unknown flag: %s\n' "$arg" >&2; exit 2 ;;
  esac
done

# Auto-detect mcp-server binary so the Console smoke runs in --live
# mode (which adds INIT_OK + LIST_COLLECTIONS_OK audit codes and the
# live tool-count assertion against mcp-server's 8 tools). If the
# binary isn't built yet, fall back silently to WASM-local-only.
MCP_BIN="${REPO_DIR}/mcp-server/target/release/rulake-mcp"
if [[ -x "${MCP_BIN}" ]]; then
  CONSOLE_SMOKE="${REPO_DIR}/ui/scripts/smoke.sh --live"
  CONSOLE_LABEL="Console WASM-local + live mcp-server"
else
  CONSOLE_SMOKE="${REPO_DIR}/ui/scripts/smoke.sh"
  CONSOLE_LABEL="Console WASM-local (no live; build mcp-server first for --live)"
fi

# Smoke registry — pairs of (label, command). Add new smokes here.
SMOKES=(
  "${CONSOLE_LABEL}|${CONSOLE_SMOKE}"
  "rvdna-mcp HTTP|${REPO_DIR}/mcp-rvdna/scripts/http-smoke.sh"
  "ruqu-mcp HTTP|${REPO_DIR}/mcp-ruqu/scripts/http-smoke.sh"
  "Production rulake-mcp.ruv.io|${REPO_DIR}/scripts/smoke-live.sh"
  "Production rvdna-mcp.ruv.io|URL=https://rvdna-mcp.ruv.io/ EXPECTED_TOOLS=5 ${REPO_DIR}/scripts/smoke-live.sh"
  "Production ruqu-mcp.ruv.io|URL=https://ruqu-mcp.ruv.io/ EXPECTED_TOOLS=5 ${REPO_DIR}/scripts/smoke-live.sh"
)
if [[ "${SKIP_CROSS}" -eq 0 ]]; then
  SMOKES+=("Console + mcp-rvdna cross-component|${REPO_DIR}/ui/scripts/smoke-cross-mcp.sh")
fi

declare -a RESULTS=()

for entry in "${SMOKES[@]}"; do
  label="${entry%%|*}"
  cmd="${entry##*|}"
  printf '\n\033[1m═══ %s ═══\033[0m\n' "${label}"
  printf '    %s\n\n' "${cmd}"

  START=$(date +%s)
  # Word-split cmd so flags like `--live` reach the inner script as
  # separate argv entries (not part of the path). The smoke-script
  # paths are repo-internal and operator-controlled, so word-splitting
  # is safe here.
  # shellcheck disable=SC2086
  bash -c "${cmd}"
  ec=$?
  ELAPSED=$(($(date +%s) - START))

  if [[ "${ec}" -eq 0 ]]; then
    RESULTS+=("${label}|PASS|${ELAPSED}s")
  else
    RESULTS+=("${label}|FAIL (exit ${ec})|${ELAPSED}s")
  fi
done

# Summary
printf '\n\n\033[1m═══════════════════ smoke-all summary ═══════════════════\033[0m\n'
ANY_FAIL=0
for r in "${RESULTS[@]}"; do
  IFS='|' read -r label status elapsed <<< "${r}"
  if [[ "${status}" == "PASS" ]]; then
    printf '  \033[32m✓ %-44s\033[0m %s\n' "${label}" "${elapsed}"
  else
    printf '  \033[31m✗ %-44s\033[0m %s · %s\n' "${label}" "${elapsed}" "${status}"
    ANY_FAIL=1
  fi
done
echo

if [[ "${ANY_FAIL}" -eq 0 ]]; then
  printf '\033[32m\033[1m✓ smoke-all green\033[0m — every script passed\n'
  exit 0
else
  printf '\033[31m\033[1m✗ smoke-all RED\033[0m — see failures above\n'
  exit 1
fi
