/* global React */
const { useState: useStH, useEffect: useEffH } = React;

// ─── Help content registry ──────────────────────────────────────────────
// Each entry: short title + body (JSX). Keyed by route or by topic id.
const HELP = {
  stats: {
    title: 'Stats — receipt-printer dashboard',
    body: () => (
      <div className="help-doc">
        <p>The Stats screen prints a continuous receipt of system health. Every tile, sparkline, and refusal row is sourced from the running session — no aggregations older than the live window.</p>
        <h4>Tiles</h4>
        <ul>
          <li><strong>req/s</strong> — ingoing <code>rulake_query</code> rate (1Hz sample)</li>
          <li><strong>p50 / p99</strong> — latency from request received to response sent, in browser-local ms</li>
          <li><strong>hit-rate</strong> — fraction of queries answered from warm cache without recomputing the witness</li>
          <li><strong>refusal-rate</strong> — fraction returned with a <code>REFUSED_*</code> code instead of a result set</li>
        </ul>
        <h4>Refusals panel</h4>
        <p>Honest refusals are <em>not</em> errors — they're a feature. ruLake refuses rather than silently downgrading, so the audit trail is unambiguous about when a query couldn't be served. Common codes:</p>
        <ul>
          <li><code>REFUSED_DRIFT</code> — bundle witness disagrees with publisher signature</li>
          <li><code>REFUSED_QUORUM</code> — federation lost quorum during the read</li>
          <li><code>REFUSED_CAP</code> — caller principal lacks the requested capability</li>
        </ul>
        <h4>Actions</h4>
        <ul>
          <li><strong>Pause</strong> — freeze the live tail (charts hold their last frame)</li>
          <li><strong>Export JSONL</strong> — download a snapshot of the current window as <code>.jsonl</code></li>
        </ul>
      </div>
    ),
  },
  playground: {
    title: 'Playground — verifiable retrieval',
    body: () => (
      <div className="help-doc">
        <p>The Playground sends <code>rulake_query</code> calls and verifies the result's witness in your browser before showing it. The witness is a SHAKE-256 hash of the canonical bundle bytes that produced the response.</p>
        <h4>Inputs</h4>
        <ul>
          <li><strong>target</strong> — backend/collection address (e.g. <code>lake-prod/memories</code>)</li>
          <li><strong>k</strong> — top-k results requested</li>
          <li><strong>risk</strong> — refusal threshold; <em>low</em> tolerates more drift, <em>high</em> refuses on any uncertainty</li>
        </ul>
        <h4>The verify cycle</h4>
        <p>After Send, you'll see <em>Computing witness…</em> for ~700ms. The browser hashes the response's data_ref + bundle pointer and compares to the publisher witness. <strong>MATCH</strong> means cryptographic agreement; anything else surfaces as a refusal in the trace.</p>
        <h4>Save / Load</h4>
        <p>Save persists the inputs (not the response) to IndexedDB on this device. Useful for repro queries during incidents.</p>
        <h4>Keyboard</h4>
        <p><span className="kbd">⌘↵</span> sends · <span className="kbd">Esc</span> closes any open dialog.</p>
      </div>
    ),
  },
  browse: {
    title: 'Backends — collections & federation',
    body: () => (
      <div className="help-doc">
        <p>The Backends screen lists every collection across the lakes you've connected, with cache pressure, witness, and federation health.</p>
        <h4>Columns</h4>
        <ul>
          <li><strong>state</strong> — <span className="tag tag-verified">VERIFIED</span> healthy · <span className="tag tag-warm">WARM</span> serving from cache · <span className="tag tag-cold">COLD</span> needs publish · <span className="tag tag-degraded">DEGRADED</span> partial quorum</li>
          <li><strong>gen</strong> — bundle generation; bumps on every publish</li>
          <li><strong>witness</strong> — first 12 hex chars of the SHAKE-256 witness for this generation</li>
        </ul>
        <h4>Actions</h4>
        <ul>
          <li><strong>Refresh</strong> — re-runs <code>rulake_list_backends</code> + per-backend <code>list_collections</code></li>
          <li><strong>Publish</strong> — queues a publish for the selected collection (requires <code>publish</code> capability)</li>
          <li><strong>Open</strong> — drills into the bundle viewer for a witness diff</li>
        </ul>
        <p className="help-note">The environment tabs (lake-prod / lake-eu / lake-edge) filter this view to one lake at a time.</p>
      </div>
    ),
  },
  bundle: {
    title: 'Bundle — witness comparator',
    body: () => (
      <div className="help-doc">
        <p>The Bundle viewer shows everything needed to independently verify a collection's witness. Three columns: the bundle JSON, the witness comparator, and the federation/cache health.</p>
        <h4>Witness columns</h4>
        <ul>
          <li><strong>Publisher</strong> — what the lake claims the witness is</li>
          <li><strong>Recomputed</strong> — what your browser computed by hashing the canonical bundle</li>
          <li>If they match, you see <strong>MATCH</strong>. If not, every row in the trace tells you which byte ranges differ.</li>
        </ul>
        <h4>Pin witness</h4>
        <p>Pinning saves the current witness + generation locally. Later, you can recompute against the same generation to detect tampering or drift between two reads.</p>
        <h4>Recompute</h4>
        <p>Re-runs the SHAKE-256 over the bundle in browser. Takes ~850ms for typical collections. Good after any ops change to confirm the lake is still self-consistent.</p>
      </div>
    ),
  },
  audit: {
    title: 'Audit — JSONL tail',
    body: () => (
      <div className="help-doc">
        <p>The Audit screen tails the structured request log. Every row is one MCP call: timestamp, principal, tool, target, k, latency, outcome.</p>
        <h4>Filters</h4>
        <p>Filter by outcome to focus: <strong>ok</strong> for successes, <strong>refused</strong> for honest refusals, <strong>degraded</strong> for partial quorum reads.</p>
        <h4>Local rows</h4>
        <p>Actions you take in this console (saves, pins, test connects) append to a local JSONL stream stored in IndexedDB. <strong>Clear local</strong> wipes them — server-side audit is unaffected and lives elsewhere.</p>
      </div>
    ),
  },
  connect: {
    title: 'Connect — endpoints & auth',
    body: () => (
      <div className="help-doc">
        <p>The Connect screen manages MCP endpoints. Save multiple, switch between them, inspect the capability matrix you'd be granted.</p>
        <h4>Auth modes</h4>
        <ul>
          <li><strong>none</strong> — anonymous; only works against open endpoints</li>
          <li><strong>bearer</strong> — opaque API token in <code>Authorization</code> header</li>
          <li><strong>jwt</strong> — signed JWT; we display the verified claims after handshake</li>
          <li><strong>mtls</strong> — client cert; <em>requires</em> a non-browser client (rulake-cli or Node) because the TLS handshake picks the cert before any JS runs</li>
        </ul>
        <h4>Capability matrix</h4>
        <p>After <code>initialize</code> succeeds, the server returns the granted scopes. The matrix shows which tools and resources are reachable for each. <strong>NOT REQUESTED</strong> means you didn't ask for that scope; <strong>GRANTED</strong> means you asked and the server agreed.</p>
        <p className="help-note">All credentials stay in this browser. Nothing is uploaded.</p>
      </div>
    ),
  },
  witness: {
    title: 'What is a witness?',
    body: () => (
      <div className="help-doc">
        <p>A <strong>witness</strong> is a deterministic hash of a collection's bundle — the canonical bytes that produced any answer derived from it.</p>
        <p>ruLake uses <strong>SHAKE-256</strong> truncated to 32 bytes. The hash covers:</p>
        <ul>
          <li>the data_ref (content-addressed pointer to the vector data)</li>
          <li>the rotation seed and rerank factor</li>
          <li>the generation number and publisher signature</li>
        </ul>
        <p>Because every answer carries the witness it came from, you can <em>independently verify</em> in the browser that the lake didn't quietly substitute a different bundle between publish and read. If the recomputed witness disagrees with the published one, the call refuses — silent fallback is never an option.</p>
      </div>
    ),
  },
  substrate: {
    title: 'Vector substrate — the ambient view',
    body: () => (
      <div className="help-doc">
        <p>The substrate visualization is an <strong>ambient health indicator</strong> for the vector store backing the active collection. It's not a real-time scan — it's a stylized projection of the embedding space, rotated continuously so you can spot when something's <em>wrong</em> at a glance.</p>
        <h4>What you're looking at</h4>
        <ul>
          <li><strong>Points</strong> — sampled embeddings from the active collection, projected from <code>1024d → 3d → 2d</code> via random projection and rotation.</li>
          <li><strong>The ring</strong> — a witness pulse synchronized to the collection's witness-recompute clock. When it fades, drift is being checked.</li>
          <li><strong>Color</strong> — matches the active environment (lake-prod / lake-eu / lake-edge).</li>
        </ul>
        <h4>What to look for</h4>
        <p>A healthy substrate looks <em>uniform</em> — points spread across the sphere with smooth rotation. Unusual patterns can indicate:</p>
        <ul>
          <li><strong>Clumping</strong> — embedding collapse; many vectors landing in the same region</li>
          <li><strong>Stalled rotation</strong> — sample feed is stuck (we're showing cached data)</li>
          <li><strong>Holes</strong> — a memory class is missing from the sample (could be policy filter or refused tier)</li>
        </ul>
        <h4>Settings</h4>
        <p>The gear icon next to the section header opens controls for density, rotation speed, and visual style. These are local — they don't change the substrate itself, just how you visualize it.</p>
        <p className="help-note">This is purely visual — every actual integrity check happens at query time via the witness comparator. The substrate just makes anomalies pre-cognitive.</p>
      </div>
    ),
  },
  refusals: {
    title: 'Honest refusals',
    body: () => (
      <div className="help-doc">
        <p>An <strong>honest refusal</strong> is a deliberate <code>REFUSED_*</code> response, with a code and reason, rather than a degraded answer or an empty result.</p>
        <p>Most retrieval systems silently fall back: cache miss → slow path, drift → stale read, quorum loss → partial result. ruLake refuses instead and logs why.</p>
        <p>This makes operations and post-mortems unambiguous: the audit trail says exactly when a query couldn't be served, and the <em>refusal-rate</em> tile is a first-class signal you can alert on.</p>
        <p>Common codes: <code>REFUSED_DRIFT</code>, <code>REFUSED_QUORUM</code>, <code>REFUSED_CAP</code>, <code>REFUSED_RISK</code>, <code>REFUSED_RATE</code>.</p>
      </div>
    ),
  },
};

// ─── Help modal ─────────────────────────────────────────────────────────
function HelpModal({ open, topicId, onClose }) {
  useEffH(() => {
    if (!open) return;
    const onKey = (e) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [open, onClose]);
  if (!open) return null;
  const topic = HELP[topicId] || HELP.stats;
  return (
    <div className="modal-backdrop help-backdrop" onClick={onClose}>
      <div className="modal help-modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <div>
            <div className="welcome-kicker">help · {topicId}</div>
            <div className="modal-title" style={{fontSize:16, marginTop:2}}>{topic.title}</div>
          </div>
          <button className="modal-x" onClick={onClose}>×</button>
        </div>
        <div className="modal-body help-body">{topic.body()}</div>
        <div className="modal-foot">
          <div style={{fontSize:11, color:'var(--fg-faint)'}}>
            Press <span className="kbd">?</span> anywhere to open the help index
          </div>
          <div style={{flex:1}}></div>
          <button className="btn btn-primary" onClick={onClose}>Got it</button>
        </div>
      </div>
    </div>
  );
}

// ─── Help index — list of all topics ────────────────────────────────────
function HelpIndexModal({ open, onClose, onTopic }) {
  useEffH(() => {
    if (!open) return;
    const onKey = (e) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [open, onClose]);
  if (!open) return null;
  const topics = [
    { id: 'stats',      glyph: '▦', label: 'Stats dashboard' },
    { id: 'playground', glyph: '▶', label: 'Query playground' },
    { id: 'browse',     glyph: '◫', label: 'Backends browser' },
    { id: 'bundle',     glyph: '◉', label: 'Bundle viewer' },
    { id: 'audit',      glyph: '≡', label: 'Audit tail' },
    { id: 'connect',    glyph: '⌁', label: 'Connect & auth' },
    { id: 'witness',    glyph: '◈', label: 'What is a witness?' },
    { id: 'substrate',  glyph: '◯', label: 'Vector substrate' },
    { id: 'refusals',   glyph: '⊘', label: 'Honest refusals' },
  ];
  return (
    <div className="modal-backdrop help-backdrop" onClick={onClose}>
      <div className="modal help-modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <div>
            <div className="welcome-kicker">help index</div>
            <div className="modal-title" style={{fontSize:16, marginTop:2}}>What do you want to know?</div>
          </div>
          <button className="modal-x" onClick={onClose}>×</button>
        </div>
        <div className="modal-body help-body" style={{paddingBottom:16}}>
          <div className="help-grid">
            {topics.map(t => (
              <div key={t.id} className="help-card" onClick={() => { onTopic(t.id); }}>
                <div className="help-card-glyph">{t.glyph}</div>
                <div className="help-card-label">{t.label}</div>
              </div>
            ))}
          </div>
          <div style={{marginTop:18, padding:'12px 14px', borderLeft:'2px solid var(--verifier)', background:'rgba(var(--verifier-rgb),0.04)', fontSize:11.5, color:'var(--fg-dim)', lineHeight:1.55}}>
            Tip — every section header has a <strong>?</strong> button next to it. Click it to open contextual help for just that part of the screen.
          </div>
        </div>
      </div>
    </div>
  );
}

// ─── Help icon — tiny ? button ──────────────────────────────────────────
function HelpIcon({ topic, label }) {
  const open = () => window.dispatchEvent(new CustomEvent('rulake:open-help', { detail: { topic } }));
  return (
    <button className="help-icon" onClick={(e) => { e.stopPropagation(); open(); }}
      aria-label={label || 'help'} title={label || 'Help'}>?</button>
  );
}

// ─── Inline guidance strip ──────────────────────────────────────────────
function HelpStrip({ children, kind = 'info', dismissable = true, storageKey }) {
  const [hidden, setHidden] = useStH(() => {
    if (!storageKey) return false;
    try { return localStorage.getItem('rulake-strip-' + storageKey) === '1'; } catch { return false; }
  });
  if (hidden) return null;
  const dismiss = () => {
    setHidden(true);
    if (storageKey) { try { localStorage.setItem('rulake-strip-' + storageKey, '1'); } catch {} }
  };
  return (
    <div className={'help-strip help-strip-' + kind}>
      <div className="help-strip-glyph">
        {kind === 'info' && '◆'}
        {kind === 'tip' && '✱'}
        {kind === 'warn' && '⚠'}
      </div>
      <div className="help-strip-body">{children}</div>
      {dismissable && (
        <button className="help-strip-x" onClick={dismiss} aria-label="dismiss">×</button>
      )}
    </div>
  );
}

// Global ? hotkey
function HelpHotkey({ onOpenIndex }) {
  useEffH(() => {
    const onKey = (e) => {
      // Only trigger when not typing
      const t = e.target;
      const typing = t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable);
      if (!typing && e.key === '?') { e.preventDefault(); onOpenIndex(); }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onOpenIndex]);
  return null;
}

window.RuHelp = { HelpModal, HelpIndexModal, HelpIcon, HelpStrip, HelpHotkey, HELP };
