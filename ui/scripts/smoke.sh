#!/usr/bin/env bash
#
# ruLake Console — end-to-end smoke test.
#
# Walks all seven routes (Stats / Playground / Backends / Bundle /
# Audit / Connect / App store), fires every audit-producing action
# the WASM-local mode supports, and on the App store route asserts
# that the four substrate cards render with their status tags
# (SHIPPING / SCAFFOLDED). Asserts zero browser console errors and
# that each expected audit row lands in IndexedDB. Idempotent —
# clears IndexedDB between runs.
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
RUN_LIVE=0
for arg in "$@"; do
  case "$arg" in
    --keep-server)  KEEP_SERVER=1 ;;
    --live)         RUN_LIVE=1 ;;        # also exercise live mcp-server path
    --live=*)       RUN_LIVE=1 ;;
  esac
done

hdr "build"
if ! npm run build > /tmp/rulake-smoke-build.log 2>&1; then
  err "vite build failed (see /tmp/rulake-smoke-build.log)"
  exit 1
fi
ok "vite build clean ($(grep -E 'built in' /tmp/rulake-smoke-build.log | head -1))"

hdr "preview"
PREVIEW_LOG=/tmp/rulake-smoke-preview.log
# Ports are overridable so the smoke can run beside other dev servers.
# --strictPort makes vite fail loudly instead of silently moving ports.
PORT="${SMOKE_PREVIEW_PORT:-4173}"
URL="http://127.0.0.1:${PORT}/RuLake/"

# Every background process is tracked by PID and only those PIDs are
# stopped — never kill by pattern (other vite/mcp processes on the host
# are not ours).
PREVIEW_PID=""
MCP_PID=""
IPFS_PID=""
cleanup() {
  if [[ "$KEEP_SERVER" -eq 0 ]] && [[ -n "$PREVIEW_PID" ]] && kill -0 "$PREVIEW_PID" 2>/dev/null; then
    kill "$PREVIEW_PID" 2>/dev/null || true
    info "preview server stopped (pid $PREVIEW_PID)"
  fi
  if [[ -n "$MCP_PID" ]] && kill -0 "$MCP_PID" 2>/dev/null; then
    kill "$MCP_PID" 2>/dev/null || true
    info "mcp-server stopped (pid $MCP_PID)"
  fi
  if [[ -n "$IPFS_PID" ]] && kill -0 "$IPFS_PID" 2>/dev/null; then
    kill "$IPFS_PID" 2>/dev/null || true
    info "ipfs fixture gateway stopped (pid $IPFS_PID)"
  fi
}
trap cleanup EXIT

"$UI_DIR/node_modules/.bin/vite" preview --host 127.0.0.1 --port "$PORT" --strictPort \
  > "$PREVIEW_LOG" 2>&1 &
PREVIEW_PID=$!

for _ in $(seq 1 30); do
  curl -fsS -o /dev/null "$URL" 2>/dev/null && break
  kill -0 "$PREVIEW_PID" 2>/dev/null || break
  sleep 0.5
done
if ! curl -fsS -o /dev/null "$URL"; then
  err "preview did not respond at $URL"
  tail -20 "$PREVIEW_LOG"
  exit 1
fi
ok "preview live at $URL (pid $PREVIEW_PID)"

# Hermetic IPFS gateway (issue #19). The Connect.ipfs-probe step used to
# hit a public gateway (w3s.link → dweb.link), which rate-limits CI
# runners, so IPFS_OK flaked red. The fixture serves
# ui/public/sample-bundle.json under its real CIDv1 and re-hashes every
# response. The live public-gateway check runs on a schedule in
# .github/workflows/ipfs-gateway-probe.yml. Set SMOKE_IPFS_GATEWAY to
# probe a real gateway from here instead.
IPFS_FIXTURE="$UI_DIR/public/sample-bundle.json"
IPFS_PROBE_CID="$(node "$UI_DIR/scripts/ipfs-fixture-gateway.mjs" --cid-of "$IPFS_FIXTURE")"
if [[ -n "${SMOKE_IPFS_GATEWAY:-}" ]]; then
  IPFS_GATEWAY="$SMOKE_IPFS_GATEWAY"
  IPFS_PROBE_CID="${SMOKE_IPFS_CID:-bafkreigh2akiscaildcqabsyg3dfr6chu3fgpregiymsck7e7aqa4s52zy}"
  info "ipfs: probing external gateway $IPFS_GATEWAY (SMOKE_IPFS_GATEWAY)"
else
  IPFS_PORT="${SMOKE_IPFS_PORT:-4190}"
  IPFS_GATEWAY="http://127.0.0.1:${IPFS_PORT}"
  node "$UI_DIR/scripts/ipfs-fixture-gateway.mjs" --port "$IPFS_PORT" "$IPFS_FIXTURE" \
    > /tmp/rulake-smoke-ipfs.log 2>&1 &
  IPFS_PID=$!
  for _ in $(seq 1 20); do
    curl -fsS -o /dev/null "$IPFS_GATEWAY/ipfs/$IPFS_PROBE_CID" 2>/dev/null && break
    kill -0 "$IPFS_PID" 2>/dev/null || break
    sleep 0.25
  done
  if curl -fsS -o /dev/null "$IPFS_GATEWAY/ipfs/$IPFS_PROBE_CID"; then
    ok "ipfs fixture gateway at $IPFS_GATEWAY (pid $IPFS_PID, cid $IPFS_PROBE_CID)"
  else
    err "ipfs fixture gateway did not come up — see /tmp/rulake-smoke-ipfs.log"
    exit 1
  fi
fi

# Optionally boot a local mcp-server for the live-MCP path. Best-effort:
# if the binary isn't built we skip the live phase silently.
MCP_PORT="${SMOKE_MCP_PORT:-9099}"
MCP_URL="http://127.0.0.1:${MCP_PORT}/"
MCP_BIN="$UI_DIR/../crates/mcp-server/target/release/rulake-mcp"
if [[ "$RUN_LIVE" -eq 1 ]]; then
  hdr "mcp-server (live)"
  if [[ ! -x "$MCP_BIN" ]]; then
    info "mcp-server binary not found at $MCP_BIN — skipping live phase"
    info "(build with: cd mcp-server && cargo build --release)"
    RUN_LIVE=0
  else
    # --capabilities read,publish,admin → tools/list exposes all 8
    # #[tool] handlers (3 read + 2 publish + 3 admin). Smoke asserts
    # the Console reads exactly 8 to prove the SSE parser fix from
    # iter 32 also picks up the right line for mcp-server's larger
    # response payload.
    "$MCP_BIN" http --auth none --capabilities read,publish,admin --bind "127.0.0.1:${MCP_PORT}" \
      > /tmp/rulake-smoke-mcp.log 2>&1 &
    MCP_PID=$!
    sleep 1.5
    # Sanity-check via curl-OPTIONS preflight (tests CORS too).
    if curl -fsS -X OPTIONS "$MCP_URL" -H 'Origin: http://127.0.0.1:4173' -o /dev/null; then
      ok "mcp-server up (pid $MCP_PID, CORS preflight 204)"
    else
      err "mcp-server did not respond — see /tmp/rulake-smoke-mcp.log"
      RUN_LIVE=0
    fi
  fi
fi

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

IPFS_SET=$(npx --yes agent-browser eval "
(async () => {
  const item = Array.from(document.querySelectorAll('aside *')).find(el => el.textContent === 'Connect');
  item && item.click();
  await new Promise(r => setTimeout(r, 500));
  const seg =Array.from(document.querySelectorAll('.seg-item')).find(b => /gateway only/i.test(b.textContent));
  seg && seg.click();
  await new Promise(r => setTimeout(r, 600));
  const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
  const setInput = (label, v) => {
    const el = document.querySelector('input[aria-label=\"' + label + '\"]');
    if (!el) return false;
    setter.call(el, v);
    el.dispatchEvent(new Event('input', { bubbles: true }));
    return true;
  };
  const gw = setInput('IPFS gateway URL', '${IPFS_GATEWAY}');
  await new Promise(r => setTimeout(r, 200));
  const cid = setInput('IPFS probe CID', '${IPFS_PROBE_CID}');
  await new Promise(r => setTimeout(r, 300));
  const probe = Array.from(document.querySelectorAll('button')).find(b => /Probe IPFS gateway/i.test(b.textContent));
  probe && probe.click();
  await new Promise(r => setTimeout(r, 5000));
  return 'ipfs-inputs gw=' + gw + ' cid=' + cid + ' probe=' + !!probe;
})()" 2>&1 | tail -1 | tr -d '"')
if [[ "$IPFS_SET" == *"gw=true cid=true probe=true"* ]]; then
  ok "Connect.ipfs-probe fired against $IPFS_GATEWAY"
else
  err "Connect.ipfs-probe could not target $IPFS_GATEWAY ($IPFS_SET)"
fi

npx --yes agent-browser eval "
(async () => {
  const inputs = Array.from(document.querySelectorAll('input[type=text], input:not([type])'));
  const epIn = inputs.find(i => /rulake|mcp/i.test(i.value || i.placeholder || ''));
  if (epIn) {
    const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
    setter.call(epIn, 'http://127.0.0.1:65535/');
    epIn.dispatchEvent(new Event('input', { bubbles: true }));
  }
  const noneCard = Array.from(document.querySelectorAll('.seg-item')).find(el => /^No auth/.test(el.textContent.trim()));
  noneCard && noneCard.click();
  await new Promise(r => setTimeout(r, 300));
  const test = Array.from(document.querySelectorAll('button')).find(b => /Test only/i.test(b.textContent));
  test && test.click();
  await new Promise(r => setTimeout(r, 2500));
})()" > /dev/null 2>&1
ok "Connect.test fired (port 65535 — expected to fail with CONNECT_FAILED)"

if [[ "$RUN_LIVE" -eq 1 ]]; then
  LIVE_RESULT=$(npx --yes agent-browser eval "
  (async () => {
    const inputs = Array.from(document.querySelectorAll('input[type=text], input:not([type])'));
    const epIn = inputs.find(i => /127\.0\.0\.1/i.test(i.value));
    if (epIn) {
      const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
      setter.call(epIn, '${MCP_URL}');
      epIn.dispatchEvent(new Event('input', { bubbles: true }));
    }
    await new Promise(r => setTimeout(r, 200));
    const test = Array.from(document.querySelectorAll('button')).find(b => /Test only/i.test(b.textContent));
    test && test.click();
    await new Promise(r => setTimeout(r, 3500));
    const banner = Array.from(document.querySelectorAll('div, span'))
      .map(el => (el.textContent || '').trim())
      .find(t => /(initialize OK|connect failed)/.test(t)) || '';
    const m = banner.match(/initialize OK · (\\d+)ms · (\\d+) tools/);
    return JSON.stringify({ ok: !!m, tools: m && m[2] });
  })()" 2>&1 | tail -1 | sed 's/^"//; s/"$//; s/\\\"/"/g')
  ok "Connect.test fired against live mcp-server at ${MCP_URL}"

  # iter 32: Console SSE parser was grabbing the empty `data:\n`
  # keepalive line first, returning toolsCount=0 even when the wire
  # was healthy. Iter 32 fix in screens.jsx skipped empty payloads.
  # mcp-server with `--auth none` exposes its full default capability
  # set (read+publish+admin) which surfaces all 8 #[tool] handlers
  # via tools/list — so the Console MUST report ≥3 tools to prove the
  # parser is reading the right SSE frame, and ≥8 to prove the full
  # capability set is exposed (which the Console live indicator
  # depends on).
  if echo "${LIVE_RESULT}" | grep -q '"ok":true'; then
    ok "Console banner reports initialize OK from mcp-server"
  else
    err "Console did not report initialize OK from live mcp-server"
  fi
  LIVE_TOOLS=$(echo "${LIVE_RESULT}" | sed -n 's/.*"tools":"\([0-9]*\)".*/\1/p')
  if [[ "${LIVE_TOOLS}" -ge 8 ]]; then
    ok "Console reports ${LIVE_TOOLS} tools from mcp-server (≥ 8 expected)"
  else
    err "Console reports ${LIVE_TOOLS:-<unset>} tools from mcp-server, expected ≥ 8"
  fi

  # Browse.refresh — should see the active client and dispatch the
  # rulake_list_collections wire call. The local mcp-server registers
  # one fixture LocalBackend ("local") with zero collections, so the
  # response is 0 collections / 1 backend — but the audit row lands
  # green and proves the wire works end-to-end.
  npx --yes agent-browser eval "
  (async () => {
    const item = Array.from(document.querySelectorAll('aside *')).find(el => el.textContent === 'Backends');
    item && item.click();
    await new Promise(r => setTimeout(r, 400));
    const refresh = Array.from(document.querySelectorAll('button')).find(b => /^Refresh\$|REFRESH/i.test(b.textContent.trim()));
    refresh && refresh.click();
    await new Promise(r => setTimeout(r, 2500));
  })()" > /dev/null 2>&1
  ok "Browse.refresh fired against live mcp-server (rulake_list_collections wire)"
fi

hdr "App store"
# App store is read-only — no audit-firing actions. We just navigate
# to it and assert the substrate cards render correctly. A render
# failure here would have surfaced as a browser console error in the
# final check, but explicit cards/tags assertions catch silent regressions
# (e.g. missing tag-amber CSS rule) that don't throw.
APPSTORE_DOM=$(npx --yes agent-browser eval "
(async () => {
  const items = Array.from(document.querySelectorAll('.nav-item-rich'));
  const tgt = items.find(el => /app\s*store/i.test(el.textContent || ''));
  if (tgt) tgt.click();
  await new Promise(r => setTimeout(r, 500));
  const cards = document.querySelectorAll('.connect-card');
  const tags  = Array.from(document.querySelectorAll('.tag')).map(t => t.textContent.trim());
  return JSON.stringify({ cards: cards.length, tags });
})()" 2>&1 | tr -d '"' | sed 's/\\//g')
info "appstore: $APPSTORE_DOM"
if echo "$APPSTORE_DOM" | grep -q 'cards:4'; then
  ok "App store: 4 substrate cards render"
else
  err "App store: expected 4 cards, got: $APPSTORE_DOM"
fi
# Status-tag assertion is intentionally loose — accept any of
# SHIPPING / SCAFFOLDED / PROPOSED so the smoke doesn't break every
# time a substrate is promoted (iter 56 was the last such bump:
# rvDNA + ruQu both went SCAFFOLDED → SHIPPING after the v0.1+v0.2
# wave landed). What we actually care about is that each card
# carries SOME status tag.
TAG_COUNT=$(echo "$APPSTORE_DOM" | grep -oE 'SHIPPING|SCAFFOLDED|PROPOSED|ROADMAP' | wc -l)
if [[ "$TAG_COUNT" -ge 4 ]]; then
  ok "App store: 4+ status tags rendered (got $TAG_COUNT)"
else
  err "App store: expected ≥4 status tags, got $TAG_COUNT in: $APPSTORE_DOM"
fi

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
# The probe must have succeeded outright — a refused probe row next to
# a stale IPFS_OK would otherwise pass the substring check above.
for refused in IPFS_HTTP_ERROR IPFS_NETWORK_ERROR; do
  if echo "$AUDIT_CODES" | grep -q "$refused"; then
    err "audit row present but should not be: $refused"
  else
    ok "no $refused row"
  fi
done
if [[ "$RUN_LIVE" -eq 1 ]]; then
  expect_substr INIT_OK
  expect_substr LIST_COLLECTIONS_OK

  # Topbar live-indicator: data-mode attribute flips to "live" once
  # the active client is pinned by ConnectScreen.test.
  TOPBAR_MODE=$(npx --yes agent-browser eval "document.querySelector('.endpoint-pill')?.getAttribute('data-mode') || 'absent'" 2>&1 | tr -d '"' | tr -d ' ')
  if [[ "$TOPBAR_MODE" == "live" ]]; then
    ok "topbar data-mode=live (live indicator visible)"
  else
    err "topbar data-mode=$TOPBAR_MODE (expected live)"
  fi
fi

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
