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

# Smoke registry — pairs of (label, command). Add new smokes here.
SMOKES=(
  "Console WASM-local|${REPO_DIR}/ui/scripts/smoke.sh"
  "rvdna-mcp HTTP|${REPO_DIR}/mcp-rvdna/scripts/http-smoke.sh"
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
  "${cmd}"
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
