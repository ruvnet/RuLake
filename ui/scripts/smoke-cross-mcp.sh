#!/usr/bin/env bash
#
# ruLake Console — cross-component smoke against mcp-rvdna.
#
# Walks the full wire — vite preview Console at :4173 → Connect screen
# → HTTP POST to rvdna-mcp at :17441 → MCP initialize handshake →
# tools/list — and asserts the banner reads "initialize OK · Nms ·
# 5 tools". This is the test that surfaced two real bugs in iter 32:
#   - mcp-rvdna had no CORS layer (Console got Failed-to-fetch)
#   - Console's SSE parser grabbed the `data:\n` keepalive and missed
#     the JSON-bearing line below it (every successful response read
#     as "0 tools")
#
# Companion to:
#   - ./scripts/smoke.sh           (Console e2e in WASM-local mode)
#   - mcp-rvdna/scripts/http-smoke.sh (rvdna-mcp HTTP-only smoke)
#
# Usage:
#   ./scripts/smoke-cross-mcp.sh
#
# Prereqs:
#   - npx --yes agent-browser install   (one-time chrome)
#   - cargo build of mcp-rvdna's rvdna-mcp binary (this script does it)
#
# Exit:
#   0 on green; 1 on any assertion failure.

set -euo pipefail

UI_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO_DIR="$(cd "${UI_DIR}/.." && pwd)"
RVDNA_DIR="${REPO_DIR}/mcp-rvdna"

PREVIEW_PORT=4173
RVDNA_PORT=17441
PREVIEW_URL="http://localhost:${PREVIEW_PORT}/"
RVDNA_URL="http://127.0.0.1:${RVDNA_PORT}/mcp"

ok()   { printf '\033[32m  ✓\033[0m %s\n' "$*"; }
err()  { printf '\033[31m  ✗\033[0m %s\n' "$*" >&2; FAILED=1; }
hdr()  { printf '\n\033[1m▸ %s\033[0m\n' "$*"; }
info() { printf '    %s\n' "$*"; }
FAILED=0

cleanup() {
  pkill -9 -f 'vite preview' 2>/dev/null || true
  pkill -9 -f rvdna-mcp 2>/dev/null || true
  if [[ -n "${PREVIEW_PID:-}" ]]; then kill -9 "$PREVIEW_PID" 2>/dev/null || true; fi
  if [[ -n "${RVDNA_PID:-}" ]];   then kill -9 "$RVDNA_PID"   2>/dev/null || true; fi
}
trap cleanup EXIT

cd "${UI_DIR}"

hdr "build rvdna-mcp"
cargo build --release --manifest-path "${RVDNA_DIR}/Cargo.toml" --bin rvdna-mcp 2>&1 | tail -2
ok "rvdna-mcp built"

hdr "build ui dist"
npm run build 2>&1 | tail -2
ok "ui dist built"

hdr "launch rvdna-mcp"
"${RVDNA_DIR}/target/release/rvdna-mcp" http \
  --bind "127.0.0.1:${RVDNA_PORT}" \
  --capabilities read,internal \
  > /tmp/rvdna-mcp-cross.log 2>&1 &
RVDNA_PID=$!
sleep 1.2
if ! kill -0 "$RVDNA_PID" 2>/dev/null; then
  err "rvdna-mcp died on launch"; cat /tmp/rvdna-mcp-cross.log >&2; exit 1
fi
ok "rvdna-mcp listening on 127.0.0.1:${RVDNA_PORT} (pid ${RVDNA_PID})"

hdr "launch vite preview"
npx --yes vite preview --port "${PREVIEW_PORT}" > /tmp/vite-preview-cross.log 2>&1 &
PREVIEW_PID=$!
sleep 3
if ! curl -sf "${PREVIEW_URL}" -o /dev/null; then
  err "vite preview did not come up"; cat /tmp/vite-preview-cross.log >&2; exit 1
fi
ok "vite preview listening on :${PREVIEW_PORT}"

hdr "open browser + drive Connect"
npx --yes agent-browser open "${PREVIEW_URL}" --args "--no-sandbox" > /dev/null 2>&1
sleep 1.5
RESULT=$(npx --yes agent-browser eval "
  (async function() {
    // Navigate to Connect
    const items = Array.from(document.querySelectorAll('.nav-item-rich'));
    const tgt = items.find(el => /connect/i.test(el.textContent || ''));
    if (tgt) tgt.click();
    await new Promise(r => setTimeout(r, 600));

    // Set the endpoint to rvdna-mcp's URL
    const inputs = Array.from(document.querySelectorAll('input[type=text], input:not([type])'));
    const epIn = inputs.find(i => /rulake|mcp|127\\.0\\.0\\.1/i.test(i.value || i.placeholder || ''));
    if (epIn) {
      const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
      setter.call(epIn, '${RVDNA_URL}');
      epIn.dispatchEvent(new Event('input', { bubbles: true }));
    }

    // Pick 'No auth' — rvdna-mcp v0.0.1 defaults to no JWT/mTLS
    const noAuth = Array.from(document.querySelectorAll('.seg-item')).find(el => /^No auth/.test(el.textContent.trim()));
    if (noAuth) noAuth.click();
    await new Promise(r => setTimeout(r, 300));

    // Click Test
    const test = Array.from(document.querySelectorAll('button')).find(b => /Test only/i.test(b.textContent));
    if (test) test.click();
    await new Promise(r => setTimeout(r, 4500));

    const banner = Array.from(document.querySelectorAll('div, span'))
      .map(el => (el.textContent || '').trim())
      .find(t => /(initialize OK|connect failed)/.test(t)) || '';
    const m = banner.match(/initialize OK · (\\d+)ms · (\\d+) tools/);
    return JSON.stringify({
      url_set: !!epIn,
      no_auth: !!noAuth,
      test_clicked: !!test,
      ok: !!m,
      ms: m && m[1],
      tools: m && m[2],
    });
  })()
" 2>&1 | tail -1 | sed 's/^"//; s/"$//; s/\\\"/"/g')

info "result: ${RESULT}"

if echo "${RESULT}" | grep -q '"ok":true'; then
  ok "Console banner reports initialize OK"
else
  err "Console did not report initialize OK"
fi

TOOLS=$(echo "${RESULT}" | sed -n 's/.*"tools":"\([0-9]*\)".*/\1/p')
if [[ "${TOOLS}" == "5" ]]; then
  ok "tool count = 5 (matches mcp-rvdna's 5 #[tool] handlers)"
else
  err "expected 5 tools, got: ${TOOLS:-<unset>}"
fi

hdr "audit row"
AUDIT=$(npx --yes agent-browser eval "
  (async () => {
    const rows = await window.RuStore.list('audit');
    return rows.filter(r => r.code === 'INIT_OK').length;
  })()
" 2>&1 | tr -d '"' | tail -1)
if [[ "${AUDIT}" -ge 1 ]]; then
  ok "INIT_OK audit row landed (${AUDIT} total)"
else
  err "no INIT_OK audit row found (got: ${AUDIT})"
fi

hdr "console errors"
ERRORS=$(npx --yes agent-browser console --errors 2>&1 | grep -E '^\[(error|warning)\]' || true)
if [[ -z "${ERRORS}" ]]; then
  ok "no browser console errors"
else
  err "browser console errors detected:"
  echo "${ERRORS}" | sed 's/^/      /'
fi

echo
if [[ "${FAILED}" -eq 0 ]]; then
  printf '\033[32m\033[1m✓ cross-mcp smoke green\033[0m — Console at :%d → rvdna-mcp at :%d → 5 tools, INIT_OK, 0 errors\n' \
    "${PREVIEW_PORT}" "${RVDNA_PORT}"
  exit 0
else
  printf '\033[31m\033[1m✗ cross-mcp smoke RED\033[0m — see failures above\n'
  exit 1
fi
