# 01 — Getting started

The fastest way to see ruLake working is to open the hosted Console. No
install, no auth, no signup. The page is a Vite-built static SPA hosted on
GitHub Pages and the MCP it talks to is a Cloud Run service in `us-central1`.

> Open: [https://ruvnet.github.io/RuLake/](https://ruvnet.github.io/RuLake/)

![ruLake Console — Stats screen with the LIVE pill in the top right](../../assets/console-hero.png)

## What happens on boot

1. The Console loads in **DEMO mode** with fixture data. The top-right
   endpoint pill reads `○ DEMO · no live MCP`.
2. A fire-and-forget probe runs against `https://rulake-mcp.ruv.io/`. The
   probe is a real MCP `initialize` over Streamable HTTP — same wire shape
   the Connect screen exercises by hand.
3. If the probe succeeds, `window.RULakeActiveClient` is populated with the
   session id and the topbar pill flips to `● LIVE · rulake-mcp.ruv.io`.
4. If the probe fails (cert pending, offline, your firewall, your VPN), the
   pill stays at DEMO and the rest of the Console keeps using fixtures.

The probe lives in `ui/src/components/screens.jsx` inside `Topbar`'s effect.
Failure is silent by design — you should never see a toast for "couldn't
reach the demo MCP".

## What you'll see (left to right)

- **Sidebar** — seven routes (Stats, Playground, Backends, Bundle, Audit,
  Connect, App store). Each row carries a small badge: `live`, `WARM`, the
  current audit row count, the current connection count.
- **Topbar tabs** — three environment scopes (`lake-prod`, `lake-eu`,
  `lake-edge`). The Stats and Browse screens scale their numbers to whichever
  is active. Default is `lake-prod`.
- **Endpoint pill** — top-right. Shows live/demo state and the connected
  host. Hover for the session id.
- **Witness HUD** — bottom of the sidebar. Live SHAKE-256 hex, the bundle
  it belongs to, and a green `● MATCH` indicator.

## Keyboard shortcuts

The Console has a deliberately small shortcut surface. The bindings are in
`ui/src/components/help.jsx`.

| Key | Where it works | What it does |
|---|---|---|
| `?` | Anywhere outside an input | Open the Help index modal |
| `Esc` | Any open modal | Close it |
| `⌘ + Enter` | Playground query box | Send the query (mac) |
| `Ctrl + Enter` | Playground query box | Send the query (win/linux) |

Any time you see a small `?` icon next to a section header, click it for
context-specific docs. The help system is keyed by topic id, so the same
copy is reachable from both the icon and the index.

## Two ways to run real code

The Console can drive **either** of these without you changing anything in
the URL bar:

1. **Through the wire** — when the topbar pill is `● LIVE`, every
   Playground query, every Browse refresh, and every Bundle recompute
   issues a real `tools/call` to the connected MCP. The audit ledger fills
   with rows whose principal is `jules@ruv` (the demo principal — change it
   on your own deploy via JWT claims).
2. **WASM-local** — the bundle of `rulake-wasm` shipped with the Console
   gives the Bundle screen a `computeWitness` function that runs entirely in
   your browser. The IPFS strip on the Bundle screen pairs with this:
   `paste a CID → fetch from a public gateway → recompute the witness
   locally`. No network round-trip to a server.

Pick whichever fits — for the screens that follow, both produce real audit
rows in the local IndexedDB.

## Where state lives

Everything the Console "remembers" is in your browser:

- **IndexedDB** — saved connections, pinned bundles, saved queries, the
  audit tail.
- **localStorage** — the substrate animation settings (density, speed).
- **JS memory only** — bearer tokens, JWT strings. They are never persisted
  past a page reload, by design.

You can wipe the lot from devtools `Application > Storage > Clear site data`
if you want a clean slate. The Connect screen also exposes a `Clear local`
button on the Audit screen.

## Next

When you have the topbar pill green, head to [02 — Stats screen](./02-stats-screen.md)
for a tour of the home view.
