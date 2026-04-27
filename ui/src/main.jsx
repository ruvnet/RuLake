// ruLake Console — Vite entry.
//
// The design ships as plain-JS / babel-standalone files that each
// declare functions at top level and `Object.assign(window, {...})` at
// the bottom. Their first lines do `const { useState } = React;`, which
// expects React to already be a global. ESM hoists static imports above
// any top-level statements, so we use *dynamic* imports below to delay
// side-effect loading until AFTER React/ReactDOM have been pinned on
// `window`.

import React from 'react';
import * as ReactDOM from 'react-dom/client';

window.React = React;
window.ReactDOM = ReactDOM;

// Styles first so first paint isn't unstyled.
import './styles/styles.css';
import './styles/help.css';

// Load the design files in the same order ruLake Console.html does.
// Each file's body runs synchronously and writes its exports onto
// `window` — later imports can read them.
async function bootstrap() {
  await import('./lib/data.js');
  await import('./lib/store.js');
  // wasm side-loader — populates window.RULakeWasm with lazy-load
  // shims for verifyBundle / computeWitness / searchL2. The rulake-wasm
  // chunk (~149 KB) only fetches when one of those is first called.
  await import('./lib/wasm.js');
  // RuLakeHttp client — pinned on window for the Connect screen's
  // live-mode handshake. fetch-based, SSE-aware.
  const httpMod = await import('./lib/http.js');
  window.RuLakeHttp = httpMod.RuLakeHttp;
  window.RuLakeHttpError = httpMod.RuLakeHttpError;
  // Search dispatcher — picks Worker or main-thread per the user's
  // Storage settings toggle. Pinned as window.RULakeSearch.
  await import('./lib/searchOffload.js');
  // Embedding provider switchboard — pinned as window.RULakeEmbed.
  // Configured at runtime by the Storage card; reads `embedProvider`
  // from IndexedDB and the user-supplied API key from JS memory.
  await import('./lib/embed.js');
  await import('./components/tweaks-panel.jsx');
  await import('./components/components.jsx');
  await import('./components/modals.jsx');
  await import('./components/toast.jsx');
  await import('./components/help.jsx');
  await import('./components/welcome.jsx');
  await import('./components/screens.jsx');
  await import('./components/debug.jsx');
  // app.jsx is the last one — it calls ReactDOM.createRoot itself.
  await import('./components/app.jsx');
}

bootstrap().catch((err) => {
  console.error('[rulake-console] bootstrap failed:', err);
  const root = document.getElementById('root');
  if (root) {
    root.textContent = 'Console failed to start: ' + (err?.message ?? String(err));
  }
});
