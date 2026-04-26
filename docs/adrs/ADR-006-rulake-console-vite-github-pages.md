# ADR-006 — ruLake Console: Vite + React, hosted on GitHub Pages, dual-mode (demo + live)

**Status.** Proposed
**Date.** 2026-04-26
**Author.** rUv (ruv.io)
**Supersedes.** Resolves the conditional in `docs/research/management-ui.md` ("conditional yes")
**Companion.** `docs/research/console-deep-review.md` — the file-by-file evidentiary audit
**Branch of record.** `research/management-ui`

---

## Context

ruLake has reached a state where shipping a UI is the next obvious move.

**What's done.**

- `rulake@2.2.0` is published on **crates.io** (commit `f04eb95`).
- The npm umbrella `rulake@2.2.0` is rolling out — the `node-v2.2.0`
  tag is pushed and CI is building the 5 native binaries; the
  `rulake-wasm@2.2.1` package is queued behind a 24 h npm cooldown.
- `mcp-server` is at **v0.8** (commit `67fc821`), shipping
  Streamable HTTP, JSON-RPC, four authentication modes
  (`none`/`bearer`/`jwt`/`mtls`), per-collection RBAC via JWT scopes,
  per-call capability gates with `tools/list` visibility filtering
  (`mcp-server/src/server.rs:566-585`), JWKS hot rotation, IPFS-aware
  bundle resources, and an in-process JSONL audit log.
- `node-wasm/` exposes `verifyBundleJson`, `computeWitness`,
  `searchBruteForceL2`, `formatVersion`, `buildInfo`
  (`node-wasm/src/lib.rs:157, 211, 275, 343`). ~149 KB compressed.
- `node/http.mjs` exports `RuLakeHttp` — the fetch-based MCP-Streamable-HTTP
  client every browser caller will use (`node/http.mjs:107-184`).
- `gcs-backend/`, `ipfs-backend/` (ADR-005) and the on-disk `rvf-*`
  workspace under `vendor/ruvector/crates/rvf/` are all live.

**What isn't done.**

- There is no UI. The closest thing the project has is
  `examples/wasm/01-witness-verifier-browser/` — a one-page proof of
  concept of the cryptographic verify path.

**The previous research note** (`docs/research/management-ui.md`,
1,457 lines, commit `a703a19`, on this same branch) put the verdict at
**"conditional yes"** with three gating conditions:

> 1. The operator persona has asked for a console, in writing, in the last 90 days.
> 2. The agent-developer persona is bottlenecked on onboarding friction in measurable ways.
> 3. The team has 3 engineer-weeks of capacity that isn't better spent on cloud backends or `mcp-brain`.

**The new fact** is that the user has now produced a substantial
design artifact at `assets/RuLake dashboard.zip` (5.0 MB, 22 files,
extracted to `/tmp/rulake-dashboard-review/` for review). The artifact
is not a sketch — it is a **working in-browser console** with six
routes, an IndexedDB-backed local store, a multi-step onboarding
wizard, contextual help on every screen, three theme presets, an
ambient vector substrate, a receipt-printer witness aesthetic, and
production-grade SEO meta. It is the resolution of the prior note's
gating Q1 — the operator-and-evangelist persona has shown up, with a
brief, in code. The ADR's job is to convert that artifact into a
production deployment with the lowest-friction stack.

The user's directive, exactly:

> "deep review '/home/ruvultra/projects/RuLake/assets/RuLake dashboard.zip'
>  this will be build in vitejs and hosted on github pages.
>  create adr, this will be the complete management ui for rulake and demo"

So the ADR records two decisions in one: (a) we will build the UI, and
(b) we will build it in Vite + React + TypeScript and host it on
GitHub Pages, dual-mode (demo + live) from a single static build.

---

## Decision

We will:

1.  **Ship `ui/` as a Vite + React + TypeScript SPA, deployed to
    GitHub Pages at `https://ruvnet.github.io/RuLake/` as the
    canonical management UI for ruLake.** The ADR-006 commit
    creates the `ui/` directory; the first ship is `ui-v0.1`.

2.  **Adopt the design from `assets/RuLake dashboard.zip` as the
    v0.1 source of truth.** The file inventory and migration map
    are in `docs/research/console-deep-review.md` §1 and §8. The
    JSX → TSX port is mechanical for routes/components; the
    vanilla CSS migrates as plain `.css` files imported per
    component (no Tailwind, no CSS-in-JS, no component library —
    defended in §8.2 of the deep review). The receipt-printer
    aesthetic, the verifier-green palette, the contextual help
    copy, and the welcome-modal intro slideshow port verbatim. The
    Tweaks panel (`tweaks-panel.jsx`) is the design tool's
    edit-mode harness and is dropped from production.

3.  **Dual-mode UI: demo + live, both shipped from the same Vite
    build.**
    - **Demo mode** is the default for the GitHub Pages URL. Six
      routes render with bundled fixtures (from `data.js`). The
      Playground's "Send" runs `searchBruteForceL2` from
      `rulake-wasm` over a fixture vector set; the Bundle's
      "Recompute witness" runs `verifyBundleJson` from
      `rulake-wasm` over a fixture bundle. **Cryptography is
      real.** The 700 ms / 850 ms `setTimeout` placeholders in the
      artifact are replaced by actual wasm calls. The demo's
      witness "MATCH" badge is the same SHAKE-256 verdict the
      production console shows — it's the **data** that's
      synthetic, not the **proof**.
    - **Live mode** activates when the user enters an endpoint URL
      on the Connect screen. The MCP transport is `RuLakeHttp` from
      `node/http.mjs`. Authentication: `none`, `bearer`, or `jwt`
      (paste). mTLS is correctly disabled in-browser; the design
      already shows the warning (`screens.jsx:1237-1245`).

4.  **SEO posture: keep the title, meta, OG, Twitter, and JSON-LD
    blocks from `ruLake Console.html` verbatim.** Two corrections:
    `<link rel="canonical">` flips from the GitHub repo URL to the
    Pages URL once live; the missing `docs/og.png` (referenced by
    the OG / Twitter image fields and currently 404) is created at
    1200×630 and committed to `docs/`. The README hero image
    becomes a click-through to the Pages URL — that single edit is
    the highest-leverage README change of the quarter.

5.  **Tech-stack pins.** Defended one-line each, full justification in
    deep-review §8.2.
    - **Vite 5** — the user asked for it; right tool for static SPA + lazy wasm.
    - **React 18.3** — same as the design; mechanical JSX → TSX port.
    - **TypeScript 5.4 strict** — every fixture and every server response
      gets a typed boundary; pays back the first time someone touches witness rendering.
    - **`wouter`** for routing — 1.5 KB; the design is a single string
      route state and we don't need react-router's machinery.
    - **No state library** — local `useState` + DOM custom events +
      the existing `useRuStore` IndexedDB hook is enough; adding
      Zustand/Jotai is overkill for six routes.
    - **No component library** — the design is bespoke; CSS modules
      or per-component plain CSS imports; Tailwind would either pay
      for nothing or fight the receipt-printer aesthetic.
    - **`zod`** for response-schema validation at the wire boundary.
    - **`vite-plugin-wasm`** + dynamic `import('rulake-wasm')` so the
      ~149 KB wasm chunk only loads on routes that need it
      (Playground, Bundle).
    - **Production dependency closure: 6 packages** — react,
      react-dom, wouter, zod, rulake, rulake-wasm. Tree-shaken bundle
      should land under 250 KB pre-wasm.

6.  **Build + deploy pipeline.**
    - `pnpm --dir ui build` produces a static `ui/dist/`.
    - New `.github/workflows/release-ui.yml` (full content in deep-review §10.1):
      checkout → setup-pnpm → setup-node → install → build →
      `actions/upload-pages-artifact@v3` → `actions/deploy-pages@v4`.
    - Triggers: `push` to `main` with paths `ui/**` (rolling demo);
      `push` of tags matching `ui-v*` (cut releases);
      `workflow_dispatch` (manual).
    - **No `gh-pages` branch.** The official action publishes
      directly via the `id-token` flow.
    - Vite `base: '/RuLake/'` for the default Pages path; flips to
      `'/'` if a custom domain is added later.

7.  **Authentication in the browser.**
    - Demo mode: no auth.
    - Live mode: `none` / `bearer` / `jwt` paste. The Connect
      screen's existing UI (`screens.jsx:1140-1301`) is the surface;
      it populates a `RuLakeHttp` client created on
      "Connect & initialize". JWT-with-PKCE is a v0.2 polish item.
      mTLS is **out of scope** for the browser UI — the TLS
      handshake picks the cert before any JS runs. The design
      already disables this option correctly; we keep that exact
      warning (`screens.jsx:1237-1245`) and copy it to the welcome
      modal.
    - Token storage: full tokens **never** persisted; the design
      already correctly stores only the first 8 chars in IndexedDB
      for display (`screens.jsx:1152`). Full token lives in JS
      memory only and is re-pasted on tab reload. This is the
      correct trade-off for a browser console.

8.  **Witness-trust boundary — the load-bearing decision.** Every
    server response that carries a witness is **re-verified locally
    with `verifyBundleJson` from `rulake-wasm` before the response
    is rendered as trusted.** If the recomputed SHAKE-256 disagrees
    with the published witness, the response is **not** rendered —
    the receipt stamp turns amber, an audit row is appended locally
    recording the divergence, and a toast surfaces the mismatch.
    The network is untrusted; the cryptography is the only trust
    boundary the user is asked to extend. This is what makes the
    console different from every other vector-DB dashboard. The
    design treats this exactly right (deep-review §6); ADR-006 is
    the contract that says the production build will not weaken it.

9.  **The console's role in the demo.** The GitHub Pages URL is
    the project's **first 30-second pitch**. A first-time visitor
    lands on the Stats screen, sees live numbers, scrolls to the
    vector substrate, opens Playground, sends a query against
    fixture vectors, watches `verifyBundleJson` actually compute
    in the browser, and sees `● WITNESS MATCH` cryptographically
    proven by code that ran in their tab. Five clicks, no install,
    zero accounts, all open source. **This is what no other
    vector-database product offers.** ADR-006 commits to making
    this experience real in v0.1.

---

## Consequences

### Positive

- **Sub-30-second "get it" moment.** A visitor lands on the URL and
  understands the value prop without reading docs. Today they have
  to clone the repo and `cargo run`.
- **Agent-developer onboarding shortens** from "clone, build, run,
  configure MCP, write a query" to "open URL, click Send." The
  Playground encodes the `tools/call` payload visibly so the
  developer can copy it into their own client.
- **Operators get the dashboard the prior note proposed.** Stats,
  Browse, Bundle, Audit cover the read-mostly observability
  scope (2)+(3)+subset-of-(4) from `docs/research/management-ui.md` §1.
- **SEO presence outside the GitHub repo.** The Pages URL becomes
  an indexed canonical surface with full JSON-LD schema. Searches
  for "MCP vector cache", "verifiable agent memory", "SHAKE-256
  witness retrieval" have a landing page to find.
- **The receipt-printer / witness-stamp aesthetic is a brand
  asset.** No competing product looks anything like it. The
  cream-paper receipt with the rotated wax stamp is recognisable in
  a screenshot share.
- **The cryptographic claim is in-product, not in-readme.** The
  user can see the witness recomputed in their browser; trust comes
  from the experience, not the marketing.

### Negative

- **A Vite build pipeline to maintain.** New CI workflow, new
  pnpm-lock to keep current, new dependency tree to security-scan.
  Mitigated by the deliberately small dependency closure (6 packages).
- **Auth UX work for live mode.** PKCE in v0.2; settings panel for
  the user's OpenAI key (for embedding) in v0.1. Not free.
- **The design assets become the canonical look-and-feel and
  constrain future changes.** This is a feature, not a bug — design
  consistency is what makes the product feel finished — but it
  means every future screen has to land in the same visual
  language. We accept this constraint.
- **`rulake-wasm` is ~149 KB on first wasm load.** Lazy-loaded
  per-route (Playground/Bundle only); Stats/Audit/Connect never
  trigger it. Acceptable.
- **Five mcp-server gaps block parts of live mode.** Listed in
  deep-review §9 (Gaps 1, 2, 5, 7, 8 are v0.1-blockers; 3, 4, 6
  are v0.2). The first concrete server-side commit AFTER ADR-006
  lands is `mcp-server v0.9 — rulake_list_collections + audit/tail
  resource + CORS allow-list`, which unblocks Browse and Audit live
  modes simultaneously.
- **Demo mode requires us to ship a small fixture vector set in
  the static bundle** (a few hundred random unit vectors of the
  appropriate dimensions) so `searchBruteForceL2` has something to
  rank. ~30 KB; acceptable.

### Neutral

- We are **not** taking on the federation builder, RBAC editor, or
  snapshot manager in v0.1 (deferred to v0.2 per deep-review §14.2).
- We are **not** taking on a hosted public-demo `rulake-mcp` endpoint
  in v0.1 — the GH Pages URL defaults to demo mode, never live, until
  the user pastes their own endpoint. (Open question 3 in the deep
  review.)

---

## Verification

### How we'll know this worked

**Quantitative.**

- Lighthouse Performance ≥ 90 on the GH Pages URL (mobile profile).
- Initial JS bundle (gzip) ≤ 220 KB.
- `rulake-wasm` lazy-loaded chunk (gzip) ≤ 180 KB.
- Time-to-interactive ≤ 1.8 s on a fresh Chrome on broadband.
- Zero `dangerouslySetInnerHTML` in `ui/src/` (ESLint rule, CI fail).
- The Vite `base` config matches the deploy URL (assert in workflow).

**Qualitative.**

- A first-time visitor lands on the URL, follows the Welcome modal
  through to the Playground, hits Send, and sees a real
  `verifyBundleJson` MATCH within 60 seconds. We verify by sitting
  three people who haven't used ruLake in front of a stopwatch.
- A `rulake-mcp` operator can paste their endpoint into the Connect
  screen, click "Connect & initialize", see the capability matrix
  populate based on their JWT scopes, and read live `rulake://stats`
  on the Stats screen within 10 seconds. We verify with an
  internally-deployed mcp-server.
- The witness rendering on Playground and Bundle screens is
  byte-for-byte the receipt aesthetic from `_review/01-connect-jwt.png`
  and the artifact's `screens.jsx:627-672` (the wax-stamp `✓ MATCH`
  rotated -1.2°).

### Test plan

- **Unit tests** (Vitest): every fixture in `lib/fixtures.ts` parses
  through its Zod schema; `verifyBundleJson` over a known-good
  bundle returns the expected SHAKE-256; the IndexedDB hooks return
  empty arrays before any writes.
- **Integration tests** (Vitest + jsdom): Welcome modal flows
  through all 5 steps; saving a query persists across re-mount;
  switching environments updates the per-backend rollup.
- **End-to-end smoke** (Playwright, optional): one test that loads
  the GH Pages preview build, navigates each route, and screenshots
  for visual diff against `_review/*.png`. Regressions caught early.
- **Manual** before each release tag: Welcome → Playground →
  Bundle, verifying every witness MATCH lights up green.

---

## References

### Files in this repo

- `assets/RuLake dashboard.zip` — the design artifact (5.0 MB, 22 files)
- `docs/research/console-deep-review.md` — the deep-review companion
- `docs/research/management-ui.md` — the prior "conditional yes" note (1,457 lines)
- `mcp-server/src/server.rs` — server-side capability surface
  - `:191-318` — `rulake_query`
  - `:319-446` — list/publish/refresh/save/warm/invalidate tools
  - `:455-469` — capability tier mapping
  - `:566-585` — `tools/list` visibility filter
  - `:597-624` — `list_resources`
  - `:649-708` — `read_resource`
- `mcp-server/src/audit.rs` — `AuditEntry` shape
- `mcp-server/src/http.rs` — Streamable HTTP transport
- `node-wasm/src/lib.rs` — the wasm exports
  - `:157-209` — `verifyBundleJson` (the central wasm call)
  - `:211-237` — `computeWitness`
  - `:275-340` — `searchBruteForceL2`
  - `:342-414` — `formatVersion`, `buildInfo`
- `node/http.mjs` — `RuLakeHttp` browser client
- `examples/wasm/01-witness-verifier-browser/` — the prior PoC; CSP
  template; reference for the bare verify flow

### Adjacent ADRs

- **ADR-001** — Standalone-repo strategy. Why ruLake is its own
  repo separate from RuVector.
- **ADR-004** — MCP-server capability tiers and tool visibility
  filtering. The model the Connect screen's capability matrix
  surfaces.
- **ADR-005** — Bundle distribution over IPFS. The `lake-edge`
  backend in the demo data.

### Commits on this branch

- `a703a19` — `docs/research/management-ui.md` (the prior research note)
- `f04eb95` — `rulake@2.2.0` published to crates.io
- `5f9cd2c` — README hero refresh; the README link to the Pages URL
  lands on top of this baseline

### External

- Vite 5 — https://vitejs.dev/
- `actions/deploy-pages@v4` — https://github.com/actions/deploy-pages
- MCP wire protocol 2025-03-26 — https://spec.modelcontextprotocol.io/

---

## Implementation order (informative)

The first concrete commit after ADR-006 lands:

```
ui v0.0 — Vite + React + TS scaffold; styles/tokens.css extracted;
          Stats route renders fixture data offline.
```

That single commit proves the deployment pipeline (CI builds, Pages
deploys, the URL serves the screen) and unblocks every subsequent
route migration. The full migration sequence is in deep-review §8.3.

The first concrete server-side commit after ADR-006 lands:

```
mcp-server v0.9 — rulake_list_collections + rulake://audit/tail +
                  CORS allow-list for the GH Pages origin.
```

That unblocks Browse and Audit live mode simultaneously and is the
smallest change that closes the most v0.1 gaps. Estimated ~500 LOC,
one PR.

---

*Status remains Proposed until the first `ui-v0.1` deploy succeeds
to GitHub Pages, at which point this ADR transitions to Accepted.*
