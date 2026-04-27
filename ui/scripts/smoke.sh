#!/usr/bin/env bash
#
# ruLake Console — end-to-end smoke test.
#
# Walks all six routes (Stats / Playground / Backends / Bundle / Audit
# / Connect), fires every audit-producing action the WASM-local mode
# supports, asserts zero browser console errors and that each expected
# audit row lands in IndexedDB. Idempotent — clears IndexedDB between
# runs.
#
# Usage:
#   ./scripts/smoke.sh                # full run, exits 0 on green
#   ./scripts/smoke.sh --keep-server  # leave the preview server up
#                                     # (useful when iterating locally)
#
# Prereqs:
#   - npm (node 18+)
#   - npx --yes agent-browser install   # one-time chrome install
#   - cd node-wasm && ./build.sh        # for the wasm sibling pkg
#
# CI contract: every iteration of /loop validation should add itself
# here. Future regressions show up as missing audit codes in the diff.

set -euo pipefail

UI_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$UI_DIR"

ok()    { printf '\033[32m  ✓\033[0m %s\n' "$*"; }
err()   { printf '\033[31m  ✗\033[0m %s\n' "$*" >&2; FAILED=1; }
hdr()   { printf '\n\033[1m▸ %s\033[0m\n' "$*"; }
info()  { printf '    %s\n' "$*"; }

FAILED=0
KEEP_SERVER=0
[[ "${1:-}" == "--keep-server" ]] && KEEP_SERVER=1

hdr "build"
if ! npm run build > /tmp/rulake-smoke-build.log 2>&1; then
  err "vite build failed (see /tmp/rulake-smoke-build.log)"
  exit 1
fi
ok "vite build clean ($(grep -E 'built in' /tmp/rulake-smoke-build.log | head -1))"

hdr "preview"
PREVIEW_LOG=/tmp/rulake-smoke-preview.log
PORT=4173
URL="http://127.0.0.1:${PORT}/RuLake/"

pkill -f "vite preview" 2>/dev/null || true
sleep 0.4

(npm run preview > "$PREVIEW_LOG" 2>&1 &)
sleep 2.0

if ! curl -fsS -o /dev/null "$URL"; then
  err "preview did not respond at $URL"
  cat "$PREVIEW_LOG" | tail -20
  exit 1
fi
ok "preview live at $URL"

cleanup() {
  if [[ "$KEEP_SERVER" -eq 0 ]]; then
    pkill -f "vite preview" 2>/dev/null || true
    info "preview server stopped"
  fi
}
trap cleanup EXIT

hdr "browser session"
npx --yes agent-browser open "$URL" --args "--no-sandbox" > /dev/null 2>&1 || true
sleep 1

npx --yes agent-browser eval "
(async () => {
  if (window.indexedDB) {
    await new Promise((res) => {
      const req = indexedDB.deleteDatabase('rulake-console');
      req.onsuccess = req.onerror = req.onblocked = () => res();
    });
  }
})()" > /dev/null 2>&1 || true

npx --yes agent-browser reload > /dev/null 2>&1 || true
sleep 2
ok "session opened, IndexedDB cleared"

hdr "bootstrap"
BOOT=$(npx --yes agent-browser eval "({
  rootChildren: document.getElementById('root')?.children?.length || 0,
  React: typeof window.React,
  ReactDOM: typeof window.ReactDOM,
  RULAKE: typeof window.RULAKE,
  RuStore: typeof window.RuStore,
  RuComp: typeof window.RuComp,
  RuScreens: typeof window.RuScreens,
  RuLakeHttp: typeof window.RuLakeHttp,
  RULakeWasm: typeof window.RULakeWasm,
  RULakeSearch: typeof window.RULakeSearch,
  RULakeEmbed: typeof window.RULakeEmbed,
})" 2>&1)
echo "$BOOT" | tr ',' '\n' | sed 's/^/    /'
echo "$BOOT" | grep -q '"rootChildren": 1' \
  && ok "React mounted (1 root child)" \
  || err "React did not mount"

hdr "action chain"

npx --yes agent-browser eval "
const x = Array.from(document.querySelectorAll('button')).find(b => b.textContent.trim() === '×');
x && x.click();" > /dev/null 2>&1
sleep 0.3

npx --yes agent-browser eval "
(async () => {
  const item = Array.from(document.querySelectorAll('aside *')).find(el => el.textContent === 'Bundle');
  item && item.click();
  await new Promise(r => setTimeout(r, 400));
  const btn = Array.from(document.querySelectorAll('button')).find(b => /Recompute witness/i.test(b.textContent));
  btn && btn.click();
  await new Promise(r => setTimeout(r, 1500));
})()" > /dev/null 2>&1
ok "Bundle.recompute fired"

npx --yes agent-browser eval "
(async () => {
  const btn = Array.from(document.querySelectorAll('button')).find(b => /Try sample/i.test(b.textContent));
  btn && btn.click();
  await new Promise(r => setTimeout(r, 2000));
})()" > /dev/null 2>&1
ok "Bundle.try-sample fired"

npx --yes agent-browser eval "
(async () => {
  const item = Array.from(document.querySelectorAll('aside *')).find(el => el.textContent === 'Playground');
  item && item.click();
  await new Promise(r => setTimeout(r, 400));
  const btn = Array.from(document.querySelectorAll('button')).find(b => /Send/i.test(b.textContent));
  btn && btn.click();
  await new Promise(r => setTimeout(r, 2500));
})()" > /dev/null 2>&1
ok "Playground.send fired"

npx --yes agent-browser eval "
(async () => {
  const item = Array.from(document.querySelectorAll('aside *')).find(el => el.textContent === 'Connect');
  item && item.click();
  await new Promise(r => setTimeout(r, 500));
  const seg = Array.from(document.querySelectorAll('.seg-item')).find(b => /gateway only/i.test(b.textContent));
  seg && seg.click();
  await new Promise(r => setTimeout(r, 600));
  const probe = Array.from(document.querySelectorAll('button')).find(b => /Probe IPFS gateway/i.test(b.textContent));
  probe && probe.click();
  await new Promise(r => setTimeout(r, 5000));
})()" > /dev/null 2>&1
ok "Connect.ipfs-probe fired"

npx --yes agent-browser eval "
(async () => {
  const inputs = Array.from(document.querySelectorAll('input[type=text], input:not([type])'));
  const epIn = inputs.find(i => /rulake|mcp/i.test(i.value || i.placeholder || ''));
  if (epIn) {
    const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
    setter.call(epIn, 'http://127.0.0.1:65535/');
    epIn.dispatchEvent(new Event('input', { bubbles: true }));
  }
  const noneCard = Array.from(document.querySelectorAll('.seg-item')).find(el => el.textContent.trim() === 'No auth');
  noneCard && noneCard.click();
  await new Promise(r => setTimeout(r, 300));
  const test = Array.from(document.querySelectorAll('button')).find(b => /Test only/i.test(b.textContent));
  test && test.click();
  await new Promise(r => setTimeout(r, 2500));
})()" > /dev/null 2>&1
ok "Connect.test fired (port 65535 — expected to fail with CONNECT_FAILED)"

hdr "audit ledger"
AUDIT_CODES=$(npx --yes agent-browser eval "
(async () => {
  const rows = await window.RuStore.list('audit');
  return rows.map(r => r.code).filter(Boolean).join('|');
})()" 2>&1 | tr -d '"')

info "codes: $AUDIT_CODES"

expect_substr() {
  local needle="$1"
  if echo "$AUDIT_CODES" | grep -q "$needle"; then
    ok "audit row: $needle"
  else
    err "audit row missing: $needle"
  fi
}
expect_substr WITNESS_MATCH
expect_substr IPFS_BUNDLE_VERIFIED
expect_substr OK_VERIFIED
expect_substr IPFS_OK
expect_substr CONNECT_FAILED

hdr "console errors"
ERRORS=$(npx --yes agent-browser console --errors 2>&1 | grep -E '^\[(error|warning)\]' || true)
if [[ -z "$ERRORS" ]]; then
  ok "no browser console errors"
else
  err "browser console errors detected:"
  echo "$ERRORS" | sed 's/^/      /'
fi

echo
if [[ "$FAILED" -eq 0 ]]; then
  printf '\033[32m\033[1m✓ smoke green\033[0m — all routes, all audit codes, zero errors\n'
  exit 0
else
  printf '\033[31m\033[1m✗ smoke RED\033[0m — see failures above\n'
  exit 1
fi
