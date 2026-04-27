# Deep review — `assets/RuLake dashboard.zip`

**Date.** 2026-04-26
**Branch.** `research/management-ui`
**Companion to.** `docs/adrs/ADR-006-rulake-console-vite-github-pages.md`
**Predecessor.** `docs/research/management-ui.md` (the "conditional yes" note;
this artifact is the resolution of its open Q1).

This is a file-by-file, screen-by-screen, gap-by-gap audit of the design
artifact the user dropped into the repo. Every claim about the design cites a
file in `/tmp/rulake-dashboard-review/` (where the zip is extracted). Every
claim about ruLake's server side cites a real path under
`mcp-server/src/`, `node-wasm/src/`, `node/`, or `vendor/ruvector/`. The
purpose of this document is to be the engineering substrate the
ADR-006 decision rests on; the ADR is the short, normative version,
this is the long, evidentiary version.

---

## Contents

1.  What's in the zip — file inventory
2.  The design system, decoded — tokens, palette, type
3.  Information architecture — six routes, what they do, what backs them
4.  State and data flow — `store.js`, `data.js`, the demo-mode contract
5.  Auth flow — what the design assumes, what `mcp-server` exposes
6.  Witness presentation — the central design choice
7.  Demo-mode story — what happens with no server
8.  Vite migration plan — concrete file layout for `ui/`
9.  Server-side gaps — what `mcp-server v0.9` would have to grow
10. GitHub Pages deployment — workflow, branch strategy, custom domain
11. SEO and canonical URL story
12. Security review — XSS, CSP, token storage, CORS
13. Performance budget — first paint, code-split, wasm lazy-load
14. v0.1 vs v0.2+ — what to ship now, what to defer
15. Open questions for the first production deploy

---

## 1. What's in the zip

The artifact unpacks to 22 files across three groups: source
(JSX/JS/HTML/CSS), realized-design screenshots (`_review/*.png`), and
inspirational mockups (`uploads/*.png`). The realized design is what the
JSX renders and what the four `_review/` screenshots show; the mockups
in `uploads/` are reference imagery from unrelated projects (a fintech
landing page; three "cyberdefense" telemetry dashboards) and are
**inspirational only** — they do not depict ruLake. The console as
implemented does not have a globe or particle field in the sense those
mockups show; it has the receipt-printer aesthetic the screenshots
confirm.

### 1.1 Top-level source

| File | Bytes | Purpose | Notes |
|------|-------|---------|-------|
| `ruLake Console.html` | 5,131 | Entry HTML — head with meta/OG/Twitter/JSON-LD; React + Babel via UMD; loads all JSX with `type="text/babel"`. | This is the SEO surface. The title is *"ruLake Console · Verifiable Vector Memory for AI Agents · MCP-Native"*. JSON-LD declares a `SoftwareApplication`. The `og:image` references `https://github.com/ruvnet/RuLake/raw/main/docs/og.png` — **this file does not exist yet** (flag for §11). |
| `app.jsx` | 9,970 | Root `<App>` — state for route/env/welcome/help/tweaks; the `useLiveStats` tick generator that drives every animated chart on a 1.1 s interval; theme-preset → CSS-variable application. | The entire live-data feel of the demo is `useLiveStats` (`app.jsx:20-63`). The three theme presets (`verifier-green` / `cool-teal` / `iris`) are inlined here with the full token set per theme (`app.jsx:106-164`). |
| `screens.jsx` | 68,298 | The six routes (`Sidebar`, `Topbar`, `Statusbar`, `StatsScreen`, `PlaygroundScreen`, `BrowseScreen`, `BundleScreen`, `AuditScreen`, `ConnectScreen`) plus `SubstrateSettings` and `FederationGraph`. | The heaviest file. 1,303 lines, no class components, all hooks. Density is high but readable — the canvas charts are inline because `<Sparkline>`/`<Histogram>`/`<LineChart>` are in `components.jsx`. |
| `components.jsx` | 11,847 | `Sparkline`, `Histogram`, `LineChart`, `VectorField` (rotating point cloud), `ReceiptPrint` (height-mask reveal), `Hex` (elide+hover), `Bar`. | All canvas-based; no SVG except inline glyphs. DPR-aware. Animated via `requestAnimationFrame`. The `VectorField` is the ambient hero that gives the console its "vector substrate" feel. |
| `modals.jsx` | 9,493 | Modal shell + 5 specialized modals: `SaveQueryModal`, `LoadQueryModal`, `PinBundleModal`, `BundlesListModal`, `ConnectionsModal`. | Every modal writes to IndexedDB via `RuStore.put` and emits a `rulake:store` event for live re-render. Audit-log entries are appended on every action (cite `modals.jsx:36-40, 92-96`). |
| `welcome.jsx` | 24,075 | First-run onboarding — 5-step wizard with rail nav, an animated 5-slide intro slideshow (`IntroSlideshow`, lines 5-207), endpoint config, env explanation, accent picker, and a "where to go next" grid of route shortcuts. | The intro slideshow is the demo's narrative spine: *the problem → how memory works → the usual options → the missing middle → why agents trust it*. Each slide is hand-illustrated SVG with named animation classes (`intro-anim-points`, `intro-anim-flow`, `intro-anim-pop`). |
| `tweaks-panel.jsx` | 18,417 | Reusable Tweaks shell — generic edit-mode protocol (postMessage `__edit_mode_*`), `useTweaks` hook, and form controls (`TweakSlider`, `TweakRadio`, `TweakToggle`, `TweakColor`, `TweakSelect`, `TweakNumber`, `TweakButton`). | This file is the generic Anthropic prototyping harness — it has nothing ruLake-specific in it and **should be dropped** in the Vite migration. The Tweaks panel exists to let the design tool round-trip edits to the JSX; production users don't need it. |
| `debug.jsx` | 5,131 | In-app debug console — patches `console.{log,warn,error,info}` to capture into a fixed bottom panel; seeds with synthetic MCP wire chatter. | Useful pattern for live mode; the wire chatter (`debug.jsx:45-54`) is fictional in the artifact but the slot is real. In production this becomes the actual JSON-RPC over Streamable HTTP request/response log. |
| `help.jsx` | 17,522 | Help-content registry (9 topics: stats, playground, browse, bundle, audit, connect, witness, substrate, refusals), `HelpModal`, `HelpIndexModal`, `HelpIcon`, `HelpStrip`, global `?` hotkey. | This is the most production-finished part of the artifact. The copy is excellent — see §6 on how the witness/refusal explanations carry the cryptographic posture. **Must port verbatim**. |
| `toast.jsx` | 3,008 | Imperative `window.toast.{ok,warn,err,info}` API + `<ToastHost>` renderer. | Minimal pub-sub, four-kind toast variants. Trivially portable. |
| `data.js` | 8,285 | All fixture data — `BACKENDS` (3 lakes × 7 collections), `AUDIT` (12 lines), `SAMPLE_QUERY_RESPONSE` (10 hits + decision_trace + provenance), `REFUSAL_BREAKDOWN`, `mulberry32` PRNG, `hex()` and `elide()` helpers. | The fixture shape is the single source of truth for the type system in the migration. Every type in `ui/src/lib/types.ts` will derive from one of these structures. |
| `store.js` | 3,568 | IndexedDB wrapper — 5 stores (`queries`, `bundles`, `connections`, `audit`, `kv`); `useRuStore` React hook; `appendAudit`, `setKV`, `getKV`. | Cleanly factored. Stays as-is; just re-typed in TS. The `kv` store backs the `welcome_seen` flag so the wizard fires once. |
| `styles.css` | 43,115 | Full design-system CSS. ~1,556 lines. Custom-properties driven; theme-preset overrides on `body[data-theme=...]`. | See §2 for token extraction. |
| `help.css` | 15,844 | Help-modal-specific CSS — `.help-grid`, `.help-card`, `.help-strip`, `.help-doc` typography. | Pulled out for size; merges back in the Vite build. |

### 1.2 `_review/` — realized-design screenshots

| File | Bytes | What it shows |
|------|-------|---------------|
| `01-connect-jwt.png` | 71,545 | Stats screen, lake-prod tab, all six tiles populated, throughput line chart with three series, latency histogram, per-backend rollup table, refusal breakdown. The endpoint pill in the topbar reads `https://rulake.ruv.net/mcp · JWT · scope:read,publish`. |
| `02-connect-jwt.png` | 50,010 | Connect screen — endpoint URL field, four-mode auth segment (none/bearer/jwt/mtls) with JWT selected, JWT textarea, capability matrix table at the bottom showing `read`/`publish` GRANTED and `admin` NOT REQUESTED. |
| `connect-now.png` | 71,465 | A second Stats screenshot, slightly different live values (the 1.1 s tick has progressed). Functionally identical to `01`. |
| `help-stats.png` | 70,233 | The same Stats screen with the Help button clicked (the `?` in the topbar) — minimal visible change at this zoom; the Help index modal would render on top. |

The screenshots are at `1440×888` viewport (the HTML's `meta name="viewport" content="width=1440"` confirms desktop-first).

### 1.3 `uploads/` — inspirational mockups

These are **not** the ruLake design; they're reference imagery from
prior creative-direction work. Listing them so the audit is complete:

| File | Bytes | Subject |
|------|-------|---------|
| `pasted-1777232637007-0.png` | 1,249,965 | Fintech landing page mockup ("Orgun" / "Confidence in every financial choice"). Unrelated to ruLake. |
| `pasted-1777232721505-0.png` | 990,712 | "CYBERDEFEND" dashboard with node graph and a faint globe behind it. Atmospheric reference for "operations console" feel. |
| `pasted-1777232730049-0.png` | 902,167 | Anomaly-detection panel with a globe+telemetry overlay. Same series as above. |
| `pasted-1777232746703-0.png` | 1,527,729 | "CYBERDEFEND" globe with firewall / risk-level overlays. Same series. |

The realized design **discarded** the globe / heavy-particle direction
in favor of the receipt-printer terminal aesthetic. This is the right
call — the cyberdefense reference imagery is high-noise and loud; the
realized console is high-signal and quiet. The screenshots show the
discarded posture is the discarded posture: nothing in the JSX
references a globe or anything resembling those mockups.

---

## 2. The design system, decoded

The CSS is custom-properties throughout. A single root block declares
the default tokens (`styles.css:2-24`); three theme presets
(`verifier-green`, `cool-teal`, `iris`) override them via JS in
`app.jsx:106-164` by calling `root.style.setProperty()`. Bonus: `body[data-theme=...]`
selectors in `styles.css:27-34` add per-theme decorations (e.g. iris gets
faint radial-gradient background washes).

### 2.1 Color tokens (default = verifier-green)

Cited from `styles.css:2-24` and `app.jsx:106-128`.

| Token | Hex | Role |
|-------|-----|------|
| `--ink`     | `#0a0e14` | Page background — deepest near-black |
| `--ink-2`   | `#11161e` | Surface 1 — sidebar, header band, toolbar fills |
| `--ink-3`   | `#1a212c` | Surface 2 — table-row hover, subtle elevation |
| `--ink-4`   | `#242c39` | Surface 3 — scrollbar thumb hover, deepest control |
| `--rule`    | `#2a3340` | All borders / dividers |
| `--fg`      | `#d6dae0` | Body text |
| `--fg-dim`  | `#8a92a0` | Secondary text |
| `--fg-faint`| `#5a6373` | Tertiary / labels / hints |
| `--paper`   | `#f5f1e8` | Receipt panel background — cream |
| `--paper-2` | `#ebe6d8` | Receipt deeper cream |
| `--paper-ink`| `#1a1814` | Receipt text — near-black on cream |
| `--paper-dim`| `#6b6558` | Receipt secondary |
| `--paper-rule`| `#c9c2af` | Receipt dashed-line color |
| `--verifier`| `#1e5b3d` | Forest verifier green — borders, fills |
| `--verifier-bright`| `#2d8c5d` | Bright verifier — text, dots, badges |
| `--refused` | `#c87b2c` | Burnt amber — refusal borders |
| `--refused-bright`| `#e89548` | Bright amber — refusal text/dots |
| `--accent-cyan`| `#4a9eb5` | Information accent — `lake-eu` line color, MCP URI tint |

The other two themes (`cool-teal`, `iris`) keep the same token names but shift
hues. **Every screen renders correctly under all three** because nothing
hard-codes a hex outside these vars except a few SVG illustrations in
`welcome.jsx` (acceptable since those are illustration content, not chrome).

The single most important design principle in the palette: **no red, ever.**
Refusals are amber, not red. This is intentional; the help text says it
explicitly: *"Refusals are not failures. They are the planner declining a
query when the witness, freshness, or capability budget would be violated"*
(`screens.jsx:450-452`). Keeping refusals out of the red channel is what
makes the audit log look like a system working as designed instead of a
system on fire.

### 2.2 Typography

From `styles.css:23, 38-51`:

- **UI font**: Inter, 400/500/600 (loaded via Google Fonts in `ruLake Console.html:53`).
- **Mono font**: JetBrains Mono, 400/500/600 (same source). Used for every numeric
  value, every hex digest, every kbd, every URI, every code-fence block, every
  table cell with data. The console is monospace-heavy by design.
- **Base size**: 13 px body. Tile values 22 px. `font-feature-settings: 'cv11', 'ss01'`
  on body (`styles.css:46`) — Inter's slashed-zero and alternate-1 stylistic sets,
  which matter precisely because the console mixes prose and numbers in tight rows.
- **Font feature off** for mono: `font-feature-settings: 'liga' 0, 'calt' 0`
  (`styles.css:50`) — disables ligatures so `=>` doesn't get prettied into `⇒` in
  hex digests where it would be misleading.

### 2.3 Spacing

Implicit but consistent: 14/18 px gutters; `.tile` is 14×16; `.field-row` is
10×14; `.pane-header` is 32 px tall; `.statusbar` is 28 px; topbar is 36 px.
The grid is `220 px sidebar | 1fr main`; the playground/bundle right pane is 380 px.
Mobile breakpoints at 1100 px (3-tile rows → 3, split → 320 px) and 720 px (sidebar
becomes a drawer; tile rows → 2; split collapses).

### 2.4 Component vocabulary

- **Card**: not used. The console does not paginate into cards; it uses
  full-width tables and bordered tiles inside `border-bottom` rows.
- **Tile** (`.tile`, `.tile-value`, `.tile-label`, `.tile-delta`): the
  six-column KPI row at the top of Stats. Bordered via `border-right`,
  no rounding.
- **Tag** (`.tag-verified`, `.tag-warm`, `.tag-cold`, `.tag-degraded`,
  `.tag-refused`): inline pill, outlined in current color, monospace,
  uppercase. The vocabulary is the audit code surface: `WARM`, `COLD`,
  `VERIFIED`, `DEGRADED`, `MATCH`, `BIT-EXACT MATCH`.
- **Receipt** (`.receipt`, `.receipt-row`, `.receipt-stamp`, `.receipt-foot`):
  cream `--paper` panel with dashed borders, top/bottom serrated edges
  (`::before` / `::after` with radial-gradient background to draw the
  perforation). The witness rendering is *literally* a receipt — 11.5 px
  monospace, k/v rows, dashed dividers, a wax-stamp `.receipt-stamp` that
  rotates -1.2°. This is the single most distinctive UI moment in the
  product and it must port byte-for-byte.
- **Tape** (`.tape-row`): the audit-log line — left-bordered colored stripe
  (`ok` green, `degraded`/`refused` amber dashed), grid of timestamp /
  principal / tool / message / code.
- **Hex** (`.hex`): elided 8-prefix + `…` + 6-suffix on default; full
  hex on hover (CSS-only, no JS). Defined in
  `components.jsx:331-339`.
- **Bar** (`.bar`, `.bar-fill`, `.bar-fill.amber`): 4 px tall progress bar.
- **Sparkline / Histogram / LineChart** (`components.jsx:14-222`): canvas,
  DPR-aware, animated via rAF.
- **Substrate** (`VectorField`, `components.jsx:226-307`): rotating point
  cloud at the bottom of Stats. Ambient — looks like a vector store.

### 2.5 Motion

All animations are `requestAnimationFrame`-driven (charts, substrate, the
endpoint pulse `.conn-dot.pulse`) or short CSS keyframes (`fingerprint-in`,
`introPop`, `pulse`, `navpulse`). No animation library. `prefers-reduced-motion`
is **not** wired up — flag for the v0.1 polish pass (§14).

---

## 3. Information architecture

Six routes, listed in sidebar order, all in `screens.jsx`. The route key
lives in a single React state in `app.jsx:66`.

### 3.1 `stats` — `StatsScreen` (`screens.jsx:237-482`)

**Function.** Live observability dashboard. Six KPI tiles, a 60 s
throughput line chart with three series (one per backend), a latency
histogram (p50/p99 readouts), a per-backend rollup table, the refusal
breakdown, and the ambient vector substrate at the bottom.

**Data the design assumes.**
- Aggregate KPIs: hits, misses, hit-rate, primes, refused, witnesses-verified.
- Per-backend rollup: `{ id, region, hits, misses, hitRate, avgPrimeMs, witness }`.
- Throughput series per backend, 60-sample window.
- Latency histogram bucketed 0/5/10/25/50/100 ms.
- Refusal breakdown: `{ code, count }[]` where codes are `WITNESS_MISMATCH_REFUSED`,
  `STALE_BUNDLE_GUARD`, `STALE_BUNDLE_FALLBACK`, `POLICY_DENIED`, `BUDGET_EXCEEDED`.

**Backed by today.**
- `rulake://stats` (mcp-server/src/server.rs:599, 649-664) — gives hits, misses,
  primes, hit-rate, avg-prime-ms in one rollup. **Maps cleanly.**
- `rulake://stats/by-backend` (`mcp-server/src/server.rs:607, 665-684`) — per-backend
  hits/misses/primes. **Maps cleanly.**

**Gaps.**
- **No latency histogram resource.** The audit log records `duration_ms`
  per call (`AuditEntry` in `mcp-server/src/audit.rs`), but no rolled-up
  p50/p99 surface exists. v0.1 falls back to client-side rolling
  bucketing of audit entries; v0.2 needs `rulake://stats/latency`.
- **No throughput-series resource.** Same shape problem — derive client-side
  from streaming audit; or new `rulake://stats/qps?window=60s`.
- **No refusal-breakdown resource.** Same — derive from audit by counting
  `outcome=refused` grouped by `code`.
- **No witnesses-verified counter** server-side (the design's "0 mismatches"
  tile). v0.1 surfaces this from in-browser verify-loop counters
  (rulake-wasm `verifyBundleJson` results); v0.2 makes it a server resource.

### 3.2 `playground` — `PlaygroundScreen` (`screens.jsx:485-719`)

**Function.** Send `rulake_query` calls; see hits, decision trace,
witness, refusals; save/load query inputs; show the wire payload as
JSON.

**Data the design assumes.**
- Inputs: query string, `target` (e.g. `lake-prod/memories` or
  `federated · prod+eu`), `k`, `risk` (`low`/`medium`/`high`).
- Response shape: matches `SAMPLE_QUERY_RESPONSE` in `data.js:78-118`.
  Has `request`, `data` (10 hits with `id/score/snippet/source/ts`),
  `provenance` (`witness/bundle/generation/issued_at`), `trust_level`,
  `decision` (`chosen_action/reason_code/backends_used/refusals/...`).

**Backed by today.**
- `rulake_query` (`mcp-server/src/server.rs:191-318`) — the public
  decision-layer tool. Returns a `QueryResponse` with `data`,
  `provenance`, `trust_level`, `decision`. **Maps cleanly to the
  design's response shape.** The fixture in `data.js` matches the
  Rust struct field-for-field.
- `rulake/http` (`node/http.mjs:160-184`) — `RuLakeHttp.query(args)` is
  the exact call the playground invokes.

**Gaps.**
- **No "federated" target syntax.** The design dropdown shows
  `federated · prod+eu` (`screens.jsx:566`); ruLake's planner today
  takes a single `target.collection`, not a multi-backend route. v0.1
  hides the federated option in live mode; v0.2 either teaches the
  planner federated routes or removes the option.
- **Embedding model.** The footer caption claims
  *"Embedded with text-embedding-3-small at request time → 1024-d vector"*
  (`screens.jsx:553-555`). The console must do the embedding before
  calling `rulake_query`, because the server takes a vector, not text.
  v0.1 either (a) requires the user to bring their own OpenAI API key
  and we call `text-embedding-3-small` from the browser, or (b) ships a
  small bundled embedder in wasm. (a) is the pragmatic choice for v0.1
  given the cost-of-key issue is the user's; we provide a settings panel.

### 3.3 `browse` — `BrowseScreen` (`screens.jsx:722-918`)

**Function.** List every backend and its collections; show cache
state (warm/cold/degraded), entries, hits, misses, witness; "Refresh"
and "Publish" actions; cache-pressure bars; federation topology
canvas (`FederationGraph` at `screens.jsx:837-918`).

**Data the design assumes.**
- `BACKENDS[]` from `data.js:30-59`: `{ id, kind, region, collections[]
  { id, dim, gen, entries, witness, state, hits, misses, primes, lastPrimeMs } }`.

**Backed by today.**
- `rulake_list_backends` (`mcp-server/src/server.rs:319`) — returns
  backend ids only (`ListBackendsResponse { backends: Vec<String> }` at
  line 526-529). **Insufficient.**

**Gaps (this is the screen with the largest gap).**
- **No `rulake_list_collections` tool.** This is the same gap the prior
  research note flagged at the very end (`docs/research/management-ui.md` §10
  "next concrete commit"). Without it, the design's table cannot be
  populated in live mode. v0.1 needs `rulake_list_collections` returning
  `{ backend, collections: [{ id, dim, generation, entries, witness, state,
    hits, misses, primes, last_prime_ms }] }`.
- **No "kind"/"region" on backends.** The design shows GCS/FS/IPFS as
  `BACKEND` kind tags and `us-east-1`/`eu-west-2`/`global` as regions.
  These are configuration metadata in `mcp.toml` today; surface them
  in `rulake_list_backends` v2.
- **`FederationGraph`** is decorative — pure canvas with hardcoded
  positions (`screens.jsx:847-864`). **Keep as-is for v0.1**; in
  live mode, the node positions auto-layout from the actual
  backend list returned by `rulake_list_collections`.

### 3.4 `bundle` — `BundleScreen` (`screens.jsx:921-1089`)

**Function.** Witness comparator. Two columns of hex side-by-side
(server-published vs browser-recomputed); the bundle JSON
(format_version, data_ref, dim, rotation_seed, rerank_factor, generation,
pii_policy, lineage_id, memory_class, cache_entries, cache_state); a
"Recompute witness" button that reruns SHAKE-256 in the browser; a
generation-chain table showing the lineage; pin/unpin to IndexedDB.

**Data the design assumes.**
- `BundleJson` shape (`screens.jsx:949-959`): `{ format_version, data_ref,
  dim, rotation_seed, rerank_factor, generation, pii_policy, lineage_id,
  memory_class }`.
- A way to fetch bundle bytes (or canonical JSON) from the server.

**Backed by today.**
- `rulake://bundle/{backend}/{collection}` (`mcp-server/src/server.rs:617,
  685-708`) — was added in v0.6. Reads cheaply from `cache_witness_of`
  (no full pull). Returns witness + provenance JSON. **Partial cover.**
- `verifyBundleJson` in `rulake-wasm` (`node-wasm/src/lib.rs:157-209`) —
  the in-browser verifier. Takes a JSON string, returns the recomputed
  witness. **The single most important wasm export for this UI.**
- `computeWitness` (`node-wasm/src/lib.rs:211-237`) — the lower-level
  variant.

**Gaps.**
- **The current `rulake://bundle/{b}/{c}` resource returns witness +
  provenance, but does it return everything the design renders?** The
  design's KV grid shows `format_version`, `data_ref`, `rotation_seed`,
  `rerank_factor`, `pii_policy`, `lineage_id`, `memory_class`. v0.1
  needs to confirm the JSON body of this resource carries all of them —
  if not, extend the resource (small server change).
- **Generation chain.** The design shows a table of the last five
  generations (`screens.jsx:1036-1045`) with witness + Δ-entries. **No
  server surface today returns generation history.** v0.1 hides the
  table in live mode; v0.2 adds `rulake://bundle/{b}/{c}/lineage`
  returning `[{generation, issued_at, delta_entries, witness}]`.
- **"Recompute witness" requires the canonical bundle bytes.** The
  current resource may return a derived JSON that omits the
  rotation seed bytes you'd need to reproduce the witness. v0.1
  must verify by replaying the JSON exactly as the server hashed
  it; if the resource isn't byte-stable, recomputation is meaningless.
  This is the load-bearing detail — flag for the implementation.

### 3.5 `audit` — `AuditScreen` (`screens.jsx:1092-1137`)

**Function.** Tail of structured log lines. Filter by outcome
(all/ok/degraded/refused). Local-only entries (from this console's
own actions) interleave with server-fetched audit. "Clear local"
wipes the IndexedDB store; server audit is unaffected.

**Data the design assumes.**
- `AUDIT[]` from `data.js:62-75`: `{ ts, principal, tool, intent, target,
  outcome, code, k, ms }`.

**Backed by today.**
- The server emits structured `AuditEntry` rows to a JSONL file
  (`mcp-server/src/audit.rs`). **No streaming/tail surface to clients
  today.**

**Gaps.**
- **No streaming audit.** The design assumes a live tail (the screen
  even calls itself "jsonl · tail"). v0.1 polls `rulake://audit/tail`
  on a 1 s interval (need to add this resource — paginated, returns
  the last N=200 entries); v0.2 adds Server-Sent Events for true
  streaming.
- **The audit screen merges local and remote entries.** The local
  marker (`._local: true` at `screens.jsx:1098-1100`) is preserved
  through merge; this works today and stays. Only the remote half
  is the gap.

### 3.6 `connect` — `ConnectScreen` (`screens.jsx:1140-1301`)

**Function.** Manage MCP endpoints. Pick auth mode (none/bearer/jwt/mtls);
paste token; "Connect & initialize" calls `initialize` over the wire and
shows the `tools/list` + `resources/list` results; capability matrix
table at the bottom.

**Data the design assumes.**
- Saved connection: `{ label, endpoint, mode, token (first 8 chars only) }`.
- Capability matrix: `{ capability, tools, resources, status }`.

**Backed by today.**
- `RuLakeHttp.connect()` (`node/http.mjs:124-159`) — initializes the
  MCP session. **Maps cleanly.**
- Server capability tiers (`mcp-server/src/server.rs:455-469`): `Read` /
  `Publish` / `Admin` / `Internal`. The capability matrix in the design
  matches these tiers exactly.

**Gaps.**
- **mTLS is correctly flagged as browser-impossible** in the design
  (`screens.jsx:1237-1245` and `welcome.jsx:260-265`). The TLS handshake
  picks the cert before any JS runs. v0.1 keeps the warning prominent;
  the option appears but disables the connect button.
- **JWT-with-PKCE.** The design shows a JWT textarea and "paste or PKCE
  redirect" hint (`screens.jsx:1249`). v0.1 ships the paste path;
  the PKCE redirect flow is a polish-pass item.
- **Token storage.** The design says *"stored encrypted in IndexedDB"*
  (`screens.jsx:1262`) but the actual code stores `token.slice(0,8)+'…'`
  — i.e. it does NOT persist the full token. **Good.** Live-mode tokens
  must stay in JS memory only (see §12 security).

---

## 4. State and data flow

### 4.1 React state shape

There's no global state library — everything is `useState` in `<App>`
(`app.jsx:66-73`):

```ts
{
  route: 'stats' | 'playground' | 'browse' | 'bundle' | 'audit' | 'connect',
  envTab: 'prod' | 'eu' | 'edge',
  bundleSelection: { backend: string, collection: string },
  mobileNav: boolean,
  welcomeOpen: boolean,
  helpIndexOpen: boolean,
  helpTopic: string | null,
  liveData: ReturnType<typeof useLiveStats>, // animated tick state
  tweaks: { accent, density, showVectorField, motionSpeed, monoFont, showConsole },
}
```

Most child components carry their own state (form fields in playground/connect,
verifying flag in bundle). Inter-component communication is via custom
DOM events (`rulake:open-help`, `rulake:store`) — pragmatic, lightweight,
zero deps. **Keep this pattern in the migration.** It's not worth
introducing Redux for this surface.

### 4.2 IndexedDB persistence (`store.js`)

Five object stores in DB `rulake-console` v1:

| Store | Records |
|-------|---------|
| `queries` | Saved playground queries: `{ label, query, k, risk, target, ts }` |
| `bundles` | Pinned bundle witnesses: `{ backend, collection, generation, witness, note, ts }` |
| `connections` | MCP endpoints: `{ label, endpoint, mode, token-prefix-only, ts }` |
| `audit` | Local console actions (saved query, pinned bundle, etc.) |
| `kv` | Bag for booleans like `welcome_seen` |

The `useRuStore(name)` hook returns `[rows, { put, remove, clear, reload }]`.
Re-renders on `rulake:store` event from `RuStore.put`/`remove`/`clear`. Clean
and small (97 lines). **Port verbatim** to `ui/src/lib/store.ts`.

### 4.3 Mock data in `data.js`

Three lakes (gcs/fs/ipfs), 7 collections in total, 12 audit lines, one
sample query response with 10 hits. Two helpers: `mulberry32(seed)` is a
deterministic PRNG (so the demo looks the same on every reload) and
`hex(n, seed)` produces deterministic hex strings for witnesses. The
`SAMPLE_QUERY_RESPONSE` is the canonical shape for the live response and
its structure is what the type system in the migration is built from.

### 4.4 `useLiveStats` (`app.jsx:20-63`)

A `useState` tick that increments every 1.1 s, feeding a `useMemo` that
returns wave-driven KPIs and 40-sample series. Drives every tile and
chart on Stats. **In live mode, this is replaced by a hook that polls
`rulake://stats` and `rulake://stats/by-backend` on the same cadence,
and rolls a per-tab buffer of the last 60 samples for the throughput
chart.**

---

## 5. Auth flow

The design exposes four auth modes; the server supports three of them
(mTLS works on the server but not from a browser). Mapping:

| Design mode | Wire | Server-side counterpart | v0.1 status |
|-------------|------|-------------------------|-------------|
| `none` | No `Authorization` header | `AuthMode::None` | works against open endpoints |
| `bearer` | `Authorization: Bearer <opaque>` | `AuthMode::Bearer` (validates static token) | works |
| `jwt` | `Authorization: Bearer <JWT>` | `AuthMode::Jwt` (RS256/ES256, JWKS hot rotation, ADR-005-era) | works (paste); PKCE = polish |
| `mtls` | TLS client cert | `AuthMode::Mtls` (server checks cert) | **disabled in browser**; show the warning the design already wrote |

The capability matrix (`screens.jsx:1289-1296`) is keyed by the server's
capability tiers (`mcp-server/src/server.rs:455-469`): `read` /
`publish` / `admin`. The matrix is populated by the server's
`tools/list` response — which already filters by effective capabilities
(`mcp-server/src/server.rs:566-585`), so the client can *infer* the
capability tier from which tool names appeared.

**One subtle thing.** The design's "Connect & initialize" button calls
`initialize` and shows `9 tools, 5 resources` (`screens.jsx:1168`). Today
the server has 7 tools and 3-ish resources (2 stats + N bundle resources
where N is the count of cached collections). The "9 tools" is fictional
in the demo; in live mode we show whatever the server actually returned.

---

## 6. Witness presentation — the central design choice

The design treats witnesses as a **first-class UI element**, not as an
afterthought. The witness shows up in five places:

1.  **Sidebar HUD** (`screens.jsx:71-83`) — bottom of every screen, the
    current bundle's witness in elided 10+8 form, with `● MATCH` badge
    and `SHAKE-256` label. Always visible. The console's promise to the
    user, restated on every page.
2.  **Stats per-backend rollup table** (`screens.jsx:404-427`) — every
    row's last column is a `<Hex>` of the witness. Hover to expand.
3.  **Playground response receipt** (`screens.jsx:627-672`) — the
    witness is rendered as a literal cream-paper receipt with a wax
    stamp `✓ MATCH` (rotated -1.2°). The receipt has top-and-bottom
    perforation (CSS `::before` / `::after` with radial gradient).
    This is the design's icon — once seen, never confused with any
    other product.
4.  **Bundle witness comparator** (`screens.jsx:1008-1027`) — two-column
    side-by-side: server-published vs browser-recomputed. The label "Δ"
    on the third row and the `BIT-EXACT MATCH` tag are the cryptographic
    contract made visual.
5.  **Bundle generation chain** (`screens.jsx:1036-1045`) — every past
    generation has its witness shown next to a verified tag.

The copy that goes with this is excellent. From `help.jsx:123-136`:

> A **witness** is a deterministic hash of a collection's bundle — the
> canonical bytes that produced any answer derived from it. […] Because
> every answer carries the witness it came from, you can *independently
> verify* in the browser that the lake didn't quietly substitute a
> different bundle between publish and read. If the recomputed witness
> disagrees with the published one, the call refuses — silent fallback
> is never an option.

This is the marketing pitch and the technical truth at once. The
production UI **must keep this copy verbatim**, because rewriting it
will only weaken it.

The single load-bearing engineering requirement implied by this design
choice: **`rulake-wasm verifyBundleJson` must run on every response
before "MATCH" is shown.** Today the demo fakes this (700 ms `setTimeout`
in `screens.jsx:506-517`); in production, this `setTimeout` is replaced
by an actual `verifyBundleJson(canonicalJson)` call. If the wasm verify
disagrees with the server's published witness, the badge becomes
`MISMATCH` and the response is **not rendered as trusted** — the receipt
stamp becomes the amber refused stamp, and the audit log gets a
synthetic local row recording the divergence.

---

## 7. Demo-mode story

GitHub Pages is a static host. There is no server. The console has to
work without one. The current artifact handles this via fixture data in
`data.js` and fake-async delays. We can do better.

### 7.1 What the design assumes for demo mode

- `RULAKE.BACKENDS`, `RULAKE.AUDIT`, `RULAKE.SAMPLE_QUERY_RESPONSE`,
  `RULAKE.REFUSAL_BREAKDOWN` are all in `data.js`.
- `useLiveStats` evolves the numbers on a 1.1 s tick.
- Playground "Send" runs a `setTimeout(380)` then `setTimeout(700)` to
  fake network + verify (`screens.jsx:498-519`).
- Bundle "Recompute witness" runs a `setTimeout(850)` (`screens.jsx:928-944`).

### 7.2 What the v0.1 production demo does instead

- **Fixtures stay** — same shape, typed.
- **Verify is real.** The 700 ms fake `setTimeout` becomes an actual
  `verifyBundleJson(bundleJson)` call into `rulake-wasm`. The bundle
  JSON is real (from fixtures); the witness is real; the SHAKE-256 is
  computed in-browser by the same code path live mode would use. **The
  demo's witness is cryptographically real even though the data is
  fixed.** This is the single thing that makes the GitHub Pages URL
  different from a Figma export.
- **"Try a query" mode.** The Playground's `Send` button, in demo
  mode, runs a small fixture-vector search using
  `searchBruteForceL2` from `rulake-wasm` (`node-wasm/src/lib.rs:275-340`).
  10 hits, deterministic order, real cosine ranking. The decision_trace
  comes from a fixture but the search itself is real.
- **Live data stays animated.** `useLiveStats` continues to evolve
  numbers; the demo isn't trying to claim it's connected to a real
  lake. The endpoint pill says `demo · no server` in demo mode, not
  `https://rulake.ruv.net/mcp`. The connect screen's "Connect" button
  asks for an endpoint URL.

### 7.3 What gets removed for demo mode

- The Tweaks panel (it's design-tool plumbing; production users
  don't need to retheme on the fly).
- The synthetic MCP wire chatter in `debug.jsx:45-54`. In demo
  mode the debug console shows the real verify-loop log
  (`wasm: verifyBundleJson(...) → MATCH (4.2 ms)` from real timings).

---

## 8. Vite migration plan

### 8.1 Target file layout

```
ui/
├── package.json
├── tsconfig.json
├── vite.config.ts
├── index.html                           (from `ruLake Console.html`, slimmed)
├── public/
│   ├── og.png                           (NEW — see §11; referenced by meta)
│   └── favicon.svg                      (NEW)
├── src/
│   ├── main.tsx                         (from `app.jsx` mount only)
│   ├── App.tsx                          (from `app.jsx` body, typed)
│   ├── routes/
│   │   ├── StatsRoute.tsx               (from screens.jsx:237-482)
│   │   ├── PlaygroundRoute.tsx          (from screens.jsx:485-719)
│   │   ├── BrowseRoute.tsx              (from screens.jsx:722-918)
│   │   ├── BundleRoute.tsx              (from screens.jsx:921-1089)
│   │   ├── AuditRoute.tsx               (from screens.jsx:1092-1137)
│   │   └── ConnectRoute.tsx             (from screens.jsx:1140-1301)
│   ├── components/
│   │   ├── chrome/
│   │   │   ├── Sidebar.tsx              (from screens.jsx:17-92)
│   │   │   ├── Topbar.tsx               (from screens.jsx:95-132)
│   │   │   └── Statusbar.tsx            (from screens.jsx:135-155)
│   │   ├── charts/
│   │   │   ├── Sparkline.tsx            (from components.jsx:14-89)
│   │   │   ├── Histogram.tsx            (from components.jsx:92-114)
│   │   │   ├── LineChart.tsx            (from components.jsx:117-222)
│   │   │   └── VectorField.tsx          (from components.jsx:226-307)
│   │   ├── primitives/
│   │   │   ├── Hex.tsx                  (from components.jsx:331-339)
│   │   │   ├── Bar.tsx                  (from components.jsx:342-348)
│   │   │   └── ReceiptPrint.tsx         (from components.jsx:310-328)
│   │   ├── modals/
│   │   │   ├── Modal.tsx                (from modals.jsx:5-28)
│   │   │   ├── SaveQueryModal.tsx       (from modals.jsx:31-58)
│   │   │   ├── LoadQueryModal.tsx       (from modals.jsx:61-84)
│   │   │   ├── PinBundleModal.tsx       (from modals.jsx:87-115)
│   │   │   ├── BundlesListModal.tsx     (from modals.jsx:118-140)
│   │   │   └── ConnectionsModal.tsx     (from modals.jsx:143-166)
│   │   ├── help/
│   │   │   ├── HelpModal.tsx            (from help.jsx:176-206)
│   │   │   ├── HelpIndexModal.tsx       (from help.jsx:209-254)
│   │   │   ├── HelpIcon.tsx             (from help.jsx:257-263)
│   │   │   ├── HelpStrip.tsx            (from help.jsx:266-289)
│   │   │   ├── HelpHotkey.tsx           (from help.jsx:292-304)
│   │   │   └── content/                 (one .tsx per topic in HELP{})
│   │   ├── welcome/
│   │   │   ├── WelcomeModal.tsx         (from welcome.jsx:209-438)
│   │   │   └── IntroSlideshow.tsx       (from welcome.jsx:5-207)
│   │   ├── debug/
│   │   │   └── DebugConsole.tsx         (from debug.jsx:5-108)
│   │   └── toast/
│   │       ├── ToastHost.tsx            (from toast.jsx:31-61)
│   │       └── toast.ts                 (from toast.jsx:8-29 — typed bus)
│   ├── lib/
│   │   ├── client.ts                    (wraps `rulake/http` RuLakeHttp)
│   │   ├── verify.ts                    (wraps `rulake-wasm` verifyBundleJson; lazy-loaded)
│   │   ├── search.ts                    (wraps `rulake-wasm` searchBruteForceL2 for demo)
│   │   ├── fixtures.ts                  (typed export of data.js)
│   │   ├── store.ts                     (from store.js, typed)
│   │   ├── liveStats.ts                 (real-mode polling hook)
│   │   ├── demoStats.ts                 (from app.jsx:20-63 useLiveStats)
│   │   ├── mode.ts                      (DemoMode | LiveMode discrimination)
│   │   └── types.ts                     (Backend, Collection, QueryResponse, etc.)
│   └── styles/
│       ├── tokens.css                   (extracted CSS custom properties from styles.css:1-24)
│       ├── chrome.css                   (topbar/sidebar/statusbar)
│       ├── tiles.css                    (.tile-row, .tile)
│       ├── tables.css                   (.tbl, .tape)
│       ├── receipt.css                  (.receipt and the perforation effect)
│       ├── modals.css                   (.modal, .modal-backdrop)
│       ├── help.css                     (from help.css verbatim)
│       └── reset.css                    (from styles.css:36-46)
└── README.md                            (build/run/deploy doc)
```

### 8.2 Tech stack pins (defended)

| Concern | Pick | Rationale |
|---------|------|-----------|
| Build tool | **Vite 5** | The user asked for it; for a static SPA + lazy-loaded wasm, it's the unambiguous best choice. |
| Language | **TypeScript 5.4** | Strict mode. Every fixture in `data.js` becomes a typed export; every server response gets a Zod schema parser. The cost of "convert JSX → TSX" is paid back the first time someone touches the witness rendering. |
| Framework | **React 18.3** | Same as the design. No reason to switch to Solid or Svelte and lose the JSX line-for-line port. |
| Routing | **`wouter`** (~1.5 KB) | The design uses a single string `route` state. We don't need react-router's nested-routes machinery. `wouter` matches our hash-router needs (GitHub Pages doesn't do server-side rewrites cleanly). |
| State | **No store library** | The design uses local `useState` + DOM custom events. Adding Zustand or Jotai is overkill for six routes. The `useRuStore` hook is the only "global" state and it's already an event-driven IndexedDB wrapper. Keep it. |
| Component lib | **None** | The design is bespoke — every primitive is hand-rolled CSS. Adding shadcn/Radix/MUI would either pay for nothing (the design already has its own equivalents) or actively fight the receipt-printer aesthetic. Tailwind would force us to either re-express the tokens twice or make Tailwind respect our CSS vars; neither is worth the friction. **Plain CSS files imported per-component.** |
| Forms | **Native** | Six text inputs and one segmented control across the entire app. No form library needed. |
| Schema validation | **Zod 3** | Every JSON-RPC response from the server gets parsed through a Zod schema before render. This is the type-safe boundary; if the server returns malformed JSON the UI shows a red banner instead of crashing. |
| Wasm loader | **vite-plugin-wasm + dynamic import** | `import('rulake-wasm')` is a top-level chunk; only the routes that need it (Playground, Bundle) trigger the load. ~149 KB compressed; ~370 KB uncompressed. |
| MCP client | **`rulake/http`** (already published as `rulake@2.2.0`) | The exact wire we want to speak. ESM. Works in browsers. No extra dependency surface. |
| Testing | **Vitest + Testing Library** | Vite-native; consistent with the build tool. |
| Lint | **eslint + @typescript-eslint** | Standard. |
| Format | **Prettier** | Standard. |
| Deploy | **`actions/deploy-pages@v4`** | Official, no third-party action. |

The defensible upshot of all of these: **the entire production
dependency closure is React + react-dom + wouter + zod + rulake +
rulake-wasm.** Six packages. The tree-shaken bundle should land
under 250 KB before the wasm chunk.

### 8.3 Migration sequence

1.  **Scaffold `ui/`** with `pnpm create vite@latest ui --template react-ts`.
    Drop in `vite.config.ts` with `base: '/RuLake/'` for GH Pages.
2.  **Port tokens** — `styles/tokens.css` from `styles.css:2-24`.
3.  **Port primitives** — `Hex`, `Bar`, `ReceiptPrint` (no deps).
4.  **Port charts** — `Sparkline`, `Histogram`, `LineChart`,
    `VectorField` (canvas, no deps).
5.  **Port chrome** — `Topbar`, `Sidebar`, `Statusbar`.
6.  **Port the six routes**, in this order: Stats → Playground →
    Browse → Bundle → Audit → Connect. (Stats first because it's the
    "first impression" screen the GitHub Pages URL lands on.)
7.  **Port modals + welcome + help** in parallel with routes.
8.  **Wire demo-mode**: `lib/fixtures.ts`, `lib/demoStats.ts`. Get
    every screen rendering offline before touching live mode.
9.  **Wire live-mode**: `lib/client.ts` (RuLakeHttp wrapper),
    `lib/verify.ts` (rulake-wasm wrapper), `lib/liveStats.ts` (poll).
10. **Wire `verifyBundleJson` into the response paths** — Playground
    response, Bundle recompute. Replace `setTimeout` fakes with real
    awaits. This is where v0.1 earns its name.
11. **Drop the Tweaks panel** — production users don't need it.
12. **Add `.github/workflows/release-ui.yml`** (see §10).

Estimated calendar: 5–7 days for one engineer who knows React +
TypeScript well; the JSX → TSX is mechanical, the typed boundaries
are the careful part.

---

## 9. Server-side gaps

In rough priority order. Each maps to a `mcp-server v0.9` task.

### Gap 1 — `rulake_list_collections` tool

**Blocks.** Browse screen (live mode), Bundle screen target picker.
**Today.** `rulake_list_backends` returns ids only.
**Need.** New tool returning `{ backend, collections: [{ id, dim,
generation, entries, witness, state, hits, misses, primes, last_prime_ms }] }`.
**Effort.** ~150 LOC. The data is already in
`planner.lake.cache_stats_by_collection()` (cited at
`mcp-server/src/server.rs:615`). The tool is a wrapper.
**Capability tier.** `Read`.

### Gap 2 — `rulake://audit/tail` resource

**Blocks.** Audit screen (live mode).
**Today.** Audit emits to JSONL on disk; no client-readable surface.
**Need.** Resource returning the last N=200 entries; pagination via
`?before=<ts>`.
**Effort.** ~200 LOC. Add a ring buffer in `mcp-server/src/audit.rs`
that retains the last N entries in memory, expose via a new
`rulake://audit/tail` resource handler in `server.rs`.
**Capability tier.** `Read` (admin can see all principals; read sees
only their own — RBAC pass).

### Gap 3 — `rulake://stats/latency` resource

**Blocks.** Stats screen latency histogram (live mode).
**Today.** No latency rollup.
**Need.** Resource returning `{ buckets_ms: [0,5,10,25,50,100],
counts: [...], p50, p99 }`.
**Effort.** ~200 LOC. Add an `hdrhistogram` (already a Rust crate)
to the planner; flush to a small struct on each query; expose via a
new resource.
**Capability tier.** `Read`.

### Gap 4 — `rulake://stats/refusals` resource

**Blocks.** Refusal breakdown panel on Stats (live mode).
**Today.** Refusal codes are in audit but not rolled up.
**Need.** Resource returning `[{ code, count, last_ts }]` for the last
1 h window.
**Effort.** ~100 LOC. Counter map keyed by reason_code, decay by
window.
**Capability tier.** `Read`.

### Gap 5 — Confirm `rulake://bundle/{b}/{c}` returns the full JSON

**Blocks.** Bundle screen KV grid + `verifyBundleJson` recompute.
**Today.** Resource exists (`mcp-server/src/server.rs:617`); returns
witness + provenance. Need to verify: does the JSON body include
`format_version`, `data_ref`, `rotation_seed`, `rerank_factor`,
`pii_policy`, `lineage_id`, `memory_class`, AND is the JSON byte-stable
so the wasm verifier reproduces the same SHAKE-256?
**Effort.** Audit-only if it already does; if not, ~50 LOC fix.
**Capability tier.** `Read`.

### Gap 6 — `rulake://bundle/{b}/{c}/lineage` resource

**Blocks.** Generation-chain table on Bundle screen.
**Today.** No lineage history surfaced.
**Need.** Resource returning `[{ generation, issued_at,
delta_entries, witness }]`.
**Effort.** ~150 LOC. Need to retain at least the last 5 generations'
witnesses on the planner side.
**Capability tier.** `Read`.

### Gap 7 — Embedding-as-a-service or client-side embedding

**Blocks.** Playground "Send" in live mode. The user types text;
`rulake_query` takes a vector.
**Today.** No embedding surface.
**Need.** Either (a) Playground asks for an OpenAI API key in
settings and embeds in-browser; or (b) `rulake_embed` tool that
proxies to a configured embedder. (a) is the right v0.1 because
it lets the user keep cost control; (b) is a v0.2 feature for
hosted demos.
**Effort.** (a) is UI-only (~100 LOC of fetch + key storage);
(b) is server-side (~300 LOC + a configured-embedder dep).
**Capability tier.** Whatever the embedder costs; `Read` is fine.

### Gap 8 — CORS preflight allow-list

**Blocks.** Browser → server in any non-trivial deployment.
**Today.** `mcp-server/src/http.rs` may or may not set CORS headers
permissively (need to verify). The GitHub Pages origin
(`https://ruvnet.github.io`) needs to be on the allow-list, OR the
operator sets a wildcard for known consoles.
**Effort.** ~50 LOC + a config field.
**Capability tier.** N/A — transport layer.

The five gaps that block v0.1 live mode: 1, 2, 5, 8, and an embedding
story (7). Gaps 3, 4, 6 are v0.2 polish. None of them block the
GitHub Pages **demo** — demo mode runs against fixtures + rulake-wasm
without any of these.

---

## 10. GitHub Pages deployment

### 10.1 Workflow

`.github/workflows/release-ui.yml`:

```yaml
name: release-ui
on:
  push:
    branches: [main]
    paths: ['ui/**']
  push:
    tags: ['ui-v*']
  workflow_dispatch:

permissions:
  contents: read
  pages: write
  id-token: write

concurrency:
  group: pages
  cancel-in-progress: false

jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: pnpm/action-setup@v3
        with: { version: 9 }
      - uses: actions/setup-node@v4
        with: { node-version: 20, cache: 'pnpm', cache-dependency-path: 'ui/pnpm-lock.yaml' }
      - run: pnpm --dir ui install --frozen-lockfile
      - run: pnpm --dir ui build
      - uses: actions/upload-pages-artifact@v3
        with: { path: ui/dist }

  deploy:
    needs: build
    runs-on: ubuntu-latest
    environment:
      name: github-pages
      url: ${{ steps.deployment.outputs.page_url }}
    steps:
      - id: deployment
        uses: actions/deploy-pages@v4
```

Triggers on `main` push (paths `ui/**`) for the rolling demo and on
`ui-v*` tags for cut releases. Pages deploys via the `id-token` flow —
no `gh-pages` branch, no `peaceiris/actions-gh-pages`, just the
official action.

### 10.2 Branch strategy

- **Source of truth.** `main`. The `ui/` directory lives there.
- **Deploy artifact.** `actions/deploy-pages@v4` ships the static
  artifact directly to Pages — no separate `gh-pages` branch.
- **Tags.** `ui-v0.1`, `ui-v0.2`, etc., for human-visible release
  cuts. The workflow above triggers on either main-push OR ui-tag
  so the demo URL never falls behind main.

### 10.3 Custom domain

The user has not asked for one. The default `ruvnet.github.io/RuLake/`
is a fine canonical surface. **If** a custom domain materializes
(`rulake.ruv.net` would be the natural choice), add a `CNAME` file in
`ui/public/` and update the `og:url` and JSON-LD `url` to match.
Vite's `base` config also flips from `/RuLake/` to `/`.

### 10.4 Cache busting for `rulake-wasm`

Vite hashes filenames by default. The wasm chunk will be
`rulake_wasm_bg.<hash>.wasm`. When the package version bumps, the
hash changes, the import URL changes, no manual bust needed. The
only thing to verify: the wasm chunk's `Content-Type` is
`application/wasm` (Pages does set this by extension since 2021,
verify on first deploy).

---

## 11. SEO and canonical URL story

The HTML head in `ruLake Console.html:1-56` is **production-grade SEO**.
It has:

- `<title>` (89 chars; under the 70-char ideal, but acceptable)
- `<meta name="description">` (380 chars; will get truncated by Google
  at ~160; the front of the description carries the value prop so
  truncation is graceful)
- `<meta name="keywords">` (legacy but harmless; no negative SEO impact)
- `<meta name="author">`, `<meta name="robots">`, `<meta name="theme-color">`
- `<link rel="canonical">` pointing at the **GitHub repo**, not the Pages URL
- Full Open Graph: type, site_name, title, description, url, image, image:alt
- Twitter card with creator handle
- JSON-LD `SoftwareApplication` schema with author, publisher, license, offer

**Two issues to fix in the migration.**

1.  **Canonical should point at the Pages URL.** Once the console is
    live at `https://ruvnet.github.io/RuLake/`, that's the canonical
    surface. The repo is the source code; the URL is the product.
    Update `<link rel="canonical">` and the OG/Twitter `url` fields.
2.  **`og:image` references `https://github.com/ruvnet/RuLake/raw/main/docs/og.png`
    which does not exist.** This is the social-share card every
    Twitter/Slack/LinkedIn preview will fail to render. **Create
    `docs/og.png`** at 1200×630 (Twitter's preferred), with the
    Stats screen background + the witness HUD prominent. The README
    should embed it as the hero image too.

The README hero should link to the Pages URL, big-and-prominent:

```md
[![ruLake Console — try it in your browser](docs/og.png)](https://ruvnet.github.io/RuLake/)
```

That is the single highest-leverage edit to the README the project
will make this quarter.

---

## 12. Security review

### 12.1 XSS

The console renders user-controlled strings in:

- Saved query labels (`screens.jsx:537` via Save modal)
- Saved connection labels and endpoint URLs (`screens.jsx:1156-1164`)
- Pinned bundle notes (`modals.jsx:108-112`)
- Server response snippets in Playground (`screens.jsx:611`)
- Server-returned audit `target` and `code` fields (`screens.jsx:1128-1130`)

All renders today use `{value}` interpolation — React escapes these by
default. **No `dangerouslySetInnerHTML` in the artifact.** Audit confirms
this is safe.

**The migration must keep this discipline.** A grep for
`dangerouslySetInnerHTML` in `ui/src/` should always return zero
matches. Add an ESLint rule (`react/no-danger`).

### 12.2 CSP

The current HTML has no CSP header. Pages serves `Content-Security-Policy`
headers if a `<meta http-equiv="Content-Security-Policy">` tag is present.
For the Vite build, drop in:

```html
<meta http-equiv="Content-Security-Policy" content="
  default-src 'self';
  script-src 'self' 'wasm-unsafe-eval';
  style-src 'self' 'unsafe-inline' https://fonts.googleapis.com;
  font-src 'self' https://fonts.gstatic.com;
  img-src 'self' data: https://github.com;
  connect-src 'self' https://*;
  worker-src 'self' blob:;
" />
```

The `'wasm-unsafe-eval'` directive is required for `rulake-wasm`. The
broad `connect-src https://*` is intentional — users need to connect
to arbitrary `rulake-mcp` endpoints. A stricter CSP would force
operators to fork the build for each endpoint host.

This matches the CSP we already shipped for
`examples/wasm/01-witness-verifier-browser/`; reuse that policy
verbatim with the `connect-src` widened.

### 12.3 Token storage

The design is correct: tokens are **never** persisted to IndexedDB.
`screens.jsx:1152` stores only the prefix:
`token: mode === 'none' ? '' : (token ? token.slice(0,8)+'…' : '')`. **Keep this.**

The full token lives in JS memory only, in a React state inside
`<ConnectScreen>`. When the page reloads, the user pastes again. This
is the right trade-off for a browser console — the alternative
(localStorage / IndexedDB plain) is a 10-line vulnerability writeup
waiting to happen, and the alternative-alternative (Web Crypto +
key-from-passphrase) is too much UX for the v0.1 use case.

For PKCE flows in v0.2, the access token is held in memory and
re-acquired on reload via the refresh token (which, per OAuth 2.0
BCP for browser apps, is also in memory and lost on tab close). This
is fine — the user re-authenticates per session.

### 12.4 CORS

The mcp-server today's HTTP transport (`mcp-server/src/http.rs`) needs
to be audited for CORS handling. For the GitHub Pages origin to call
the user's mcp-server, the server must respond to preflight `OPTIONS`
requests with appropriate `Access-Control-Allow-*` headers. This is
Gap 8 in §9 and is the single transport-layer change blocking live
mode from the demo URL.

---

## 13. Performance budget

### 13.1 Targets

| Metric | Target | Notes |
|--------|--------|-------|
| First contentful paint | < 1.0 s on broadband | Static SPA; should be trivially under |
| Time to interactive | < 1.8 s | After React mounts and hydrates |
| Initial bundle (JS) | < 220 KB gzip | Without wasm |
| `rulake-wasm` chunk | < 180 KB gzip | Lazy-loaded on Playground / Bundle visit |
| Lighthouse perf | > 90 | On the GH Pages URL |

### 13.2 Strategy

- **Code-split per route.** Each `routes/*.tsx` is a `React.lazy()`
  import; the router wrapper holds a `<Suspense>` with a small skeleton.
  Stats lands instantly; Playground/Bundle download their chunks on
  click.
- **Lazy-load `rulake-wasm`.** `lib/verify.ts` does `await import('rulake-wasm')`
  on first call. Stats never triggers it; Audit never triggers it.
  Connect never triggers it.
- **Preload fonts.** The HTML already has the `<link rel="preconnect">`
  for fonts.googleapis.com. Add `<link rel="preload" as="style">` for
  the CSS file.
- **Defer the substrate canvas.** `<VectorField>` should mount with
  `IntersectionObserver` (only animate when in viewport) — saves
  CPU on long scrolls.
- **Throttle `useLiveStats`.** Pause the 1.1 s tick when the tab is
  hidden (`document.visibilityState === 'hidden'`). The artifact
  doesn't do this today.

---

## 14. v0.1 vs v0.2+

### 14.1 v0.1 (the ADR-006 commit)

**Demo mode (default for the GitHub Pages URL).**
- All six routes render with fixtures.
- Witness verification is **real** via `rulake-wasm verifyBundleJson`.
- Playground "Send" runs `searchBruteForceL2` from `rulake-wasm` over
  bundled fixture vectors.
- Bundle "Recompute witness" runs the real SHAKE-256 in-browser.
- Welcome modal fires once; choices persist via IndexedDB.
- Tweaks panel removed.

**Live mode (when an endpoint is configured).**
- Stats: `rulake://stats` + `rulake://stats/by-backend` polling at 1 Hz.
  Latency histogram and refusal breakdown derive from client-side
  audit-buffer rolls (because Gaps 3 and 4 aren't shipped yet).
- Playground: `rulake_query` over `RuLakeHttp`. Embedding via user's
  OpenAI key (Gap 7a). Witness verification via `rulake-wasm`. Refuses
  to render hits if witness mismatches.
- Browse: blocked until Gap 1 lands. Show a placeholder explaining what
  the screen will do once `rulake_list_collections` ships in mcp-server v0.9.
- Bundle: works against `rulake://bundle/{b}/{c}` (Gap 5 must be
  audited; assume green). Generation-chain table hidden until Gap 6.
- Audit: blocked until Gap 2 lands. Same placeholder treatment.
- Connect: full bearer/JWT/none. mTLS shows the warning. PKCE deferred.

### 14.2 v0.2

- Federation builder UI (drag backends into routes).
- RBAC editor (preview the `tools/list` filter).
- Snapshot manager (publish/refresh/save/warm exposed as one-click
  flows with a confirm dialog).
- Streaming audit via SSE.
- PKCE redirect flow.
- Shared-link mode (the Playground encodes inputs in the URL hash so
  a query is forward-able).

### 14.3 v0.3+

- Histogram-driven latency view (after Gap 3 ships).
- Refusal-rate alerting hooks (after Gap 4 ships).
- Generation-chain diff (after Gap 6 ships).
- A "show RVF segment" deep-link (after RVF stabilises its public
  surface — see Q6 in the prior research note).

---

## 15. Open questions

1.  **Is the canonical URL `https://ruvnet.github.io/RuLake/` or
    `https://rulake.ruv.net/`?** If the latter, we set up a CNAME on
    day one. If we leave the question open, we ship to ruvnet.github.io
    and migrate later (cheap).
2.  **Whose OpenAI key embeds the playground query in live mode?** The
    user's, via a settings-stored key — but where do we store it?
    sessionStorage (lost on tab close) is the right answer for the
    privacy posture but the worst for UX. Ask before shipping.
3.  **Should the Pages URL force demo mode, or should it auto-connect
    to a public-demo `rulake-mcp` endpoint?** Hosting a public demo
    server means we own its uptime, abuse detection, and bill. The
    safer answer is "demo mode by default, live mode opt-in via the
    Connect screen." But the demo loses the "click and see real data"
    moment.
4.  **Do we keep three theme presets or ship only `verifier-green`?**
    The cool-teal and iris themes are excellent but the SEO image
    bakes one theme. The conservative call is: ship only verifier-green
    in v0.1 (the screenshots match), keep the theme machinery but
    expose only one preset; flip cool-teal and iris back on in v0.2
    once we know nobody complains about the default.
5.  **What's the wasm size budget?** `rulake-wasm` is ~149 KB compressed
    today. If we add `rulake-wasm-search` (the brute-force-L2 entry
    point is already there at `node-wasm/src/lib.rs:275-340`), no
    growth. If the embedder lands as a wasm bundle (e.g. ONNX
    `text-embedding-3-small` quantized), that's another ~25 MB and
    we don't ship it in the same chunk.
6.  **Does the audit screen need to redact tokens / PII?** The audit
    log in production carries real principal IDs and call targets.
    The console tail is not the place to leak them to a screen-share.
    Need a redaction toggle ("Hide principals"), default on.
7.  **How does the console behave behind a corporate proxy?** PAC
    files, MITM TLS, etc. Probably fine because we use `fetch` over
    HTTPS and the wasm is same-origin, but worth a real-world test
    before claiming "works everywhere."

---

*End of review. Companion ADR: `docs/adrs/ADR-006-rulake-console-vite-github-pages.md`.*
