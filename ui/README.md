# ruLake Console — Vite + React SPA

Verifiable vector memory dashboard for AI agents. Per
[ADR-006](../docs/adrs/ADR-006-rulake-console-vite-github-pages.md).

Deployed to **https://ruvnet.github.io/RuLake/** on every push to `main`
that touches `ui/` or `sdk/node-wasm/` (CI: `.github/workflows/release-ui.yml`).

## Run locally

```bash
cd ../sdk/node-wasm && ./build.sh   # one-time: build the wasm sibling
cd ../../ui
npm install
npm run dev                  # http://127.0.0.1:5173/RuLake/
```

`npm run build` produces `ui/dist/`; `npm run preview` serves it at
`http://127.0.0.1:4173/RuLake/`.

## Validate end-to-end

```bash
npx --yes agent-browser install
npm run preview &
npx --yes agent-browser open http://127.0.0.1:4173/RuLake/
npx --yes agent-browser snapshot -i
npx --yes agent-browser console --errors
```

## Layout

| Path | Purpose |
|---|---|
| `index.html` | Entry — meta + JSON-LD; loads `src/main.jsx` as ESM module |
| `src/main.jsx` | Bootstraps React-on-window, dynamic-imports the design files in dependency order |
| `src/components/` | Ported design (JSX) — `app.jsx`, `screens.jsx`, `components.jsx`, `modals.jsx`, `welcome.jsx`, `help.jsx`, `toast.jsx`, `debug.jsx`, `tweaks-panel.jsx` |
| `src/lib/data.js` | Demo fixtures (`RULAKE.BACKENDS`, `AUDIT`, `SAMPLE_QUERY_RESPONSE`, …) |
| `src/lib/store.js` | IndexedDB store (`RuStore` + `useRuStore` hook) — persistent across reloads |
| `src/styles/` | `styles.css` (43 KB design system) + `help.css` |

## Tri-mode

Per ADR-006 §10, the console runs in three modes from the same build:

1. **Demo** — fixture data only, witness verification still cryptographically real via `rulake-wasm`. Default for the GitHub Pages URL.
2. **WASM-local** — full ruLake substrate in the browser via IndexedDB + Web Workers + WebGL + WASM. Optional cloud + IPFS for storage.
3. **Live (MCP)** — connect to a remote `rulake-mcp` Streamable HTTP server.

Mode picker on the Connect screen.

## License

MIT OR Apache-2.0, matching the parent crate.
