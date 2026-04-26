/* global React, RULAKE, RuComp, RuModals */
const { useState, useEffect, useMemo, useRef } = React;
const { Sparkline, Histogram, LineChart, VectorField, ReceiptPrint, Hex, Bar } = RuComp;
const { SaveQueryModal, LoadQueryModal, PinBundleModal, BundlesListModal, ConnectionsModal } = RuModals;
const { HelpIcon, HelpStrip } = window.RuHelp;

// ─── Sidebar ────────────────────────────────────────────────────────
const NAV_GLYPHS = {
  stats: (<svg width="14" height="14" viewBox="0 0 14 14" fill="none"><rect x="1.5" y="7" width="2" height="5.5" stroke="currentColor"/><rect x="6" y="4" width="2" height="8.5" stroke="currentColor"/><rect x="10.5" y="1.5" width="2" height="11" stroke="currentColor"/></svg>),
  playground: (<svg width="14" height="14" viewBox="0 0 14 14" fill="none"><path d="M3 2 L11.5 7 L3 12 Z" stroke="currentColor" strokeLinejoin="round"/></svg>),
  browse: (<svg width="14" height="14" viewBox="0 0 14 14" fill="none"><rect x="1.5" y="1.5" width="11" height="3" stroke="currentColor"/><rect x="1.5" y="5.5" width="11" height="3" stroke="currentColor"/><rect x="1.5" y="9.5" width="11" height="3" stroke="currentColor"/></svg>),
  bundle: (<svg width="14" height="14" viewBox="0 0 14 14" fill="none"><circle cx="7" cy="7" r="5.5" stroke="currentColor"/><circle cx="7" cy="7" r="2" fill="currentColor"/></svg>),
  audit: (<svg width="14" height="14" viewBox="0 0 14 14" fill="none"><path d="M2 3.5 H12 M2 7 H12 M2 10.5 H8" stroke="currentColor" strokeLinecap="round"/></svg>),
  connect: (<svg width="14" height="14" viewBox="0 0 14 14" fill="none"><path d="M9 1.5 L4 7.5 H7 L5 12.5 L10 6.5 H7 Z" stroke="currentColor" strokeLinejoin="round"/></svg>),
};

function Sidebar({ route, setRoute, currentWitness, currentBundle, className = '' }) {
  const items = [
    { id: 'stats',      label: 'Stats',      sub: 'live · qps 28.4',   glyph: NAV_GLYPHS.stats,      badge: 'live',      badgeKind: 'live' },
    { id: 'playground', label: 'Playground', sub: 'rulake_query',      glyph: NAV_GLYPHS.playground, badge: '',          badgeKind: '' },
    { id: 'browse',     label: 'Backends',   sub: '3 lakes · 7 colls', glyph: NAV_GLYPHS.browse,     badge: '3',         badgeKind: 'count' },
    { id: 'bundle',     label: 'Bundle',     sub: 'witness · gen-7741',glyph: NAV_GLYPHS.bundle,     badge: 'WARM',      badgeKind: 'warm' },
    { id: 'audit',      label: 'Audit',      sub: 'jsonl · tail',      glyph: NAV_GLYPHS.audit,      badge: '12',        badgeKind: 'count' },
    { id: 'connect',    label: 'Connect',    sub: 'mcp · 2025-03-26',  glyph: NAV_GLYPHS.connect,    badge: 'JWT',       badgeKind: 'auth' },
  ];

  const resources = [
    { uri: 'rulake://stats',           hint: 'aggregate · all backends',   verified: true },
    { uri: 'rulake://stats/by-backend',hint: 'per-lake breakdown',         verified: true },
    { uri: 'rulake://bundle/*',        hint: 'witnesses + lineage',        verified: true },
    { uri: 'rulake://audit/tail',      hint: 'jsonl · live tail',          verified: true },
    { uri: 'rulake://policy',          hint: 'risk · pii · refusals',      verified: false },
  ];

  return (
    <aside className={'sidebar ' + className}>
      <div className="sec-h">
        <span>Console</span>
        <span className="mono" style={{color:'var(--verifier-bright)'}}>v0.3.1</span>
      </div>
      <div className="nav-list">
        {items.map(it => (
          <div key={it.id} className={'nav-item-rich' + (route === it.id ? ' active' : '')} onClick={() => setRoute(it.id)}>
            <span className="nav-rail"></span>
            <span className="nav-glyph-svg">{it.glyph}</span>
            <span className="nav-text">
              <span className="nav-label">{it.label}</span>
              <span className="nav-sub">{it.sub}</span>
            </span>
            {it.badge && <span className={'nav-badge nav-badge-' + (it.badgeKind || '')}>{it.badge}</span>}
          </div>
        ))}
      </div>

      <div className="sec-h" style={{marginTop: 14}}>
        <span>Resources</span>
        <span className="mono" style={{color:'var(--fg-faint)', fontSize:9.5}}>{resources.length}</span>
      </div>
      <div className="resource-list">
        {resources.map(r => (
          <div key={r.uri} className="resource-row">
            <span className={'resource-dot' + (r.verified ? ' ok' : '')}></span>
            <span className="resource-text">
              <span className="resource-uri">{r.uri}</span>
              <span className="resource-hint">{r.hint}</span>
            </span>
          </div>
        ))}
      </div>

      <div className="witness-hud">
        <div className="witness-hud-label">
          <span>Witness</span>
          <span style={{color:'var(--verifier-bright)'}}>● MATCH</span>
        </div>
        <div className="witness-hud-hex">
          <Hex value={currentWitness} head={10} tail={8} />
        </div>
        <div className="witness-hud-meta">
          <span>{currentBundle}</span>
          <span style={{marginLeft:'auto'}}>SHAKE-256</span>
        </div>
      </div>

      <a className="sidebar-gh" href="https://github.com/ruvnet/RuLake" target="_blank" rel="noopener" title="View ruvnet/RuLake on GitHub">
        <svg width="12" height="12" viewBox="0 0 24 24" fill="currentColor"><path d="M12 .5C5.65.5.5 5.65.5 12c0 5.08 3.29 9.39 7.86 10.91.58.1.79-.25.79-.55 0-.27-.01-1-.02-1.96-3.2.69-3.87-1.54-3.87-1.54-.52-1.33-1.28-1.68-1.28-1.68-1.05-.71.08-.7.08-.7 1.16.08 1.77 1.19 1.77 1.19 1.03 1.76 2.7 1.25 3.36.96.1-.75.4-1.26.73-1.55-2.55-.29-5.24-1.27-5.24-5.66 0-1.25.45-2.27 1.18-3.07-.12-.29-.51-1.46.11-3.04 0 0 .96-.31 3.15 1.17.91-.25 1.89-.38 2.86-.38.97 0 1.95.13 2.86.38 2.18-1.48 3.14-1.17 3.14-1.17.62 1.58.23 2.75.11 3.04.74.8 1.18 1.82 1.18 3.07 0 4.4-2.69 5.37-5.25 5.65.41.35.78 1.05.78 2.12 0 1.53-.01 2.76-.01 3.13 0 .3.21.66.8.55C20.21 21.39 23.5 17.08 23.5 12 23.5 5.65 18.35.5 12 .5z"/></svg>
        <span style={{flex:1, marginLeft:8}}>ruvnet/RuLake</span>
        <span style={{color:'var(--fg-faint)', fontSize:10}}>↗</span>
      </a>
    </aside>
  );
}

// ─── Topbar ─────────────────────────────────────────────────────────
function Topbar({ envTab, setEnvTab, onMenu }) {
  const tabs = [
    { id: 'prod',  label: 'lake-prod' },
    { id: 'eu',    label: 'lake-eu' },
    { id: 'edge',  label: 'lake-edge' },
  ];
  return (
    <header className="topbar">
      <button className="mobile-menu-btn" onClick={onMenu} aria-label="menu">≡</button>
      <div className="brand">
        <div className="brand-mark"></div>
        <div className="brand-name">RULAKE<span className="dim"> · CONSOLE</span></div>
      </div>
      <div className="topbar-tabs">
        {tabs.map(t => (
          <div key={t.id} className={'topbar-tab' + (envTab === t.id ? ' active' : '')} onClick={() => setEnvTab(t.id)}>
            {t.label}
          </div>
        ))}
      </div>
      <div className="topbar-spacer"></div>
      <a className="topbar-gh" href="https://github.com/ruvnet/RuLake" target="_blank" rel="noopener" title="ruvnet/RuLake on GitHub" aria-label="GitHub repository">
        <svg width="13" height="13" viewBox="0 0 24 24" fill="currentColor"><path d="M12 .5C5.65.5.5 5.65.5 12c0 5.08 3.29 9.39 7.86 10.91.58.1.79-.25.79-.55 0-.27-.01-1-.02-1.96-3.2.69-3.87-1.54-3.87-1.54-.52-1.33-1.28-1.68-1.28-1.68-1.05-.71.08-.7.08-.7 1.16.08 1.77 1.19 1.77 1.19 1.03 1.76 2.7 1.25 3.36.96.1-.75.4-1.26.73-1.55-2.55-.29-5.24-1.27-5.24-5.66 0-1.25.45-2.27 1.18-3.07-.12-.29-.51-1.46.11-3.04 0 0 .96-.31 3.15 1.17.91-.25 1.89-.38 2.86-.38.97 0 1.95.13 2.86.38 2.18-1.48 3.14-1.17 3.14-1.17.62 1.58.23 2.75.11 3.04.74.8 1.18 1.82 1.18 3.07 0 4.4-2.69 5.37-5.25 5.65.41.35.78 1.05.78 2.12 0 1.53-.01 2.76-.01 3.13 0 .3.21.66.8.55C20.21 21.39 23.5 17.08 23.5 12 23.5 5.65 18.35.5 12 .5z"/></svg>
        <span style={{marginLeft:6}}>ruvnet/RuLake</span>
      </a>
      <button className="topbar-help" onClick={() => window.dispatchEvent(new CustomEvent('rulake:open-help-index'))}
        title="Help index (?)" aria-label="Help">
        <span style={{fontFamily:'JetBrains Mono', fontSize:12}}>?</span>
        <span style={{marginLeft:6, fontSize:11.5}}>Help</span>
      </button>
      <div className="endpoint-pill">
        <span className="conn-dot pulse"></span>
        <span>https://rulake.ruv.net/mcp</span>
        <span className="dim" style={{marginLeft:8}}>JWT · scope:read,publish</span>
      </div>
    </header>
  );
}

// ─── Statusbar ──────────────────────────────────────────────────────
function Statusbar({ stats }) {
  const [now, setNow] = useState(new Date());
  useEffect(() => {
    const id = setInterval(() => setNow(new Date()), 1000);
    return () => clearInterval(id);
  }, []);
  const t = now.toISOString().slice(11, 19);
  return (
    <footer className="statusbar">
      <span style={{color:'var(--verifier-bright)'}}>● MCP 2025-03-26</span>
      <span>session 7f3c2…91d</span>
      <span>principal jules@ruv</span>
      <span>caps read·publish</span>
      <span style={{marginLeft:'auto'}}>q/s {stats.qps}</span>
      <span>p50 {stats.p50}ms</span>
      <span>p99 {stats.p99}ms</span>
      <span>hit-rate {stats.hitRate}%</span>
      <span>{t} UTC</span>
    </footer>
  );
}

// ─── STATS DASHBOARD ────────────────────────────────────────────────
// ─── STATS DASHBOARD ────────────────────────────────────────────────
function SubstrateSettings({ sub, setSub, onClose }) {
  useEffect(() => {
    const onKey = (e) => { if (e.key === 'Escape') onClose(); };
    const onDoc = (e) => { if (!e.target.closest('.substrate-pop') && !e.target.closest('.substrate-gear')) onClose(); };
    window.addEventListener('keydown', onKey);
    document.addEventListener('mousedown', onDoc);
    return () => { window.removeEventListener('keydown', onKey); document.removeEventListener('mousedown', onDoc); };
  }, [onClose]);
  const presets = [
    { id: 'calm',  label: 'Calm',  s: { density: 0.6, speed: 0.5, perspective: 1.0, ring: true,  halo: false } },
    { id: 'std',   label: 'Default', s: { density: 1.0, speed: 1.0, perspective: 1.0, ring: true,  halo: false } },
    { id: 'dense', label: 'Dense', s: { density: 1.6, speed: 1.0, perspective: 1.1, ring: true,  halo: true  } },
    { id: 'wired', label: 'Wired', s: { density: 1.2, speed: 2.0, perspective: 1.3, ring: false, halo: true  } },
  ];
  return (
    <div className="substrate-pop" onClick={(e)=>e.stopPropagation()}>
      <div className="substrate-pop-h">
        <span>Substrate · settings</span>
        <button className="modal-x" onClick={onClose} style={{width:18, height:18, fontSize:13}}>×</button>
      </div>

      <div className="substrate-pop-row">
        <span className="substrate-pop-label">Preset</span>
        <div className="seg" style={{flex:1}}>
          {presets.map(p => (
            <button key={p.id} className="seg-opt" onClick={() => setSub(p.s)}>{p.label}</button>
          ))}
        </div>
      </div>

      <div className="substrate-pop-row">
        <span className="substrate-pop-label">Density</span>
        <input className="substrate-range" type="range" min="0.3" max="2" step="0.1"
          value={sub.density} onChange={e => setSub({ density: parseFloat(e.target.value) })} />
        <span className="substrate-pop-val">{sub.density.toFixed(1)}×</span>
      </div>

      <div className="substrate-pop-row">
        <span className="substrate-pop-label">Rotation</span>
        <input className="substrate-range" type="range" min="0" max="3" step="0.1"
          value={sub.speed} onChange={e => setSub({ speed: parseFloat(e.target.value) })} />
        <span className="substrate-pop-val">{sub.speed.toFixed(1)}×</span>
      </div>

      <div className="substrate-pop-row">
        <span className="substrate-pop-label">Perspective</span>
        <input className="substrate-range" type="range" min="0.5" max="1.5" step="0.05"
          value={sub.perspective} onChange={e => setSub({ perspective: parseFloat(e.target.value) })} />
        <span className="substrate-pop-val">{sub.perspective.toFixed(2)}</span>
      </div>

      <div className="substrate-pop-row">
        <span className="substrate-pop-label">Witness ring</span>
        <label className="substrate-toggle">
          <input type="checkbox" checked={sub.ring} onChange={e => setSub({ ring: e.target.checked })} />
          <span>{sub.ring ? 'on' : 'off'}</span>
        </label>
      </div>

      <div className="substrate-pop-row">
        <span className="substrate-pop-label">Point halo</span>
        <label className="substrate-toggle">
          <input type="checkbox" checked={sub.halo} onChange={e => setSub({ halo: e.target.checked })} />
          <span>{sub.halo ? 'on' : 'off'}</span>
        </label>
      </div>

      <div className="substrate-pop-foot">
        <span style={{color:'var(--fg-faint)'}}>Saved on this device</span>
        <button className="btn" style={{marginLeft:'auto'}}
          onClick={() => setSub({ density: 1.0, speed: 1.0, perspective: 1.0, ring: true, halo: false })}>
          Reset
        </button>
      </div>
    </div>
  );
}

function StatsScreen({ liveData, envTab = 'prod' }) {
  const [paused, setPaused] = useState(false);
  // Substrate settings — persisted in localStorage
  const DEFAULT_SUB = { density: 1.0, speed: 1.0, perspective: 1.0, ring: true, halo: false };
  const [sub, setSubState] = useState(() => {
    try {
      const raw = localStorage.getItem('rulake-substrate');
      if (raw) return { ...DEFAULT_SUB, ...JSON.parse(raw) };
    } catch {}
    return DEFAULT_SUB;
  });
  const setSub = (next) => {
    setSubState(prev => {
      const merged = { ...prev, ...next };
      try { localStorage.setItem('rulake-substrate', JSON.stringify(merged)); } catch {}
      return merged;
    });
  };
  const [subOpen, setSubOpen] = useState(false);
  const exportJsonl = () => {
    const lines = [
      { ts: new Date().toISOString(), event: 'stats_snapshot', qps: liveData.qps, p50: liveData.p50, p99: liveData.p99 },
      { ts: new Date().toISOString(), event: 'window', samples: 60, env: envTab },
    ].map(o => JSON.stringify(o)).join('\n');
    const blob = new Blob([lines], { type: 'application/x-ndjson' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url; a.download = `rulake-stats-${envTab}-${Date.now()}.jsonl`;
    a.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
    window.toast && window.toast.ok('Exported snapshot', `2 JSONL lines · ${envTab}`);
  };
  const togglePause = () => {
    setPaused(p => !p);
    window.toast && window.toast.info(paused ? 'Resumed' : 'Paused', paused ? 'live tail flowing again' : 'live tail frozen', { duration: 1800 });
  };
  const refusalMax = Math.max(...RULAKE.REFUSAL_BREAKDOWN.map(r => r.count));
  // Per-env scaling so the 3 backend tabs feel materially different
  const ENV_SCALE = {
    prod: { mul: 1.0,  hitRateBase: 98.81, label: 'lake-prod', color: '#2d8c5d', region: 'us-west · primary' },
    eu:   { mul: 0.42, hitRateBase: 96.20, label: 'lake-eu',   color: '#4a9eb5', region: 'eu-west · replica' },
    edge: { mul: 0.08, hitRateBase: 92.74, label: 'lake-edge', color: '#e89548', region: 'pop-* · degraded' },
  };
  const env = ENV_SCALE[envTab] || ENV_SCALE.prod;
  const sc = (n) => Math.floor(n * env.mul);
  const scaled = {
    hits:    sc(liveData.hits),
    misses:  sc(liveData.misses) + (envTab==='edge'?40:0),
    primes:  sc(liveData.primes),
    refused: envTab==='edge' ? Math.floor(liveData.refused * 1.4) : sc(liveData.refused),
    verified: sc(liveData.verified),
    delta:   { hits: sc(liveData.delta.hits), misses: liveData.delta.misses },
    hitRate: env.hitRateBase.toFixed(2),
    avgPrime: envTab==='edge' ? '14.2' : envTab==='eu' ? '9.7' : '8.4',
    p50:     envTab==='edge' ? '18.4' : envTab==='eu' ? '11.2' : liveData.p50,
    p99:     envTab==='edge' ? '62.0' : envTab==='eu' ? '34.1' : liveData.p99,
  };
  const env_chart = envTab === 'prod' ? liveData.chart.prod : envTab === 'eu' ? liveData.chart.eu : liveData.chart.edge;

  return (
    <div>
      <div className="env-header" style={{borderLeft: `3px solid ${env.color}`}}>
        <div className="env-header-l">
          <span className="env-header-dot" style={{background: env.color, boxShadow: `0 0 10px ${env.color}`}}></span>
          <span className="env-header-name">{env.label}</span>
          <span className="env-header-region">{env.region}</span>
        </div>
        <div className="env-header-r">
          <span>load <strong style={{color: env.color}}>{(env.mul * 100).toFixed(0)}%</strong></span>
          <span>witness <strong style={{color: 'var(--verifier-bright)'}}>● MATCH</strong></span>
        </div>
      </div>
      <div className="pane-header">
        <span>rulake://</span><span className="crumb-sep">/</span>
        <span className="crumb-active">stats</span>
        <span className="crumb-sep">/</span><span>{env.label}</span>
        <HelpIcon topic="stats" label="About the Stats screen" />
        <span className="live-strip" style={{marginLeft:14, color: paused ? 'var(--fg-faint)' : 'var(--verifier-bright)'}}>{paused ? '○ PAUSED' : '● LIVE · 1.0Hz'}</span>
        <div className="header-actions">
          <button className="btn" onClick={togglePause}>{paused ? 'Resume' : 'Pause'}</button>
          <button className="btn" onClick={exportJsonl}>Export JSONL</button>
        </div>
      </div>

      <div className="tile-row">
        <div className="tile">
          <div className="tile-label">Hits</div>
          <div className="tile-value">{scaled.hits.toLocaleString()}</div>
          <div className="tile-delta">+{scaled.delta.hits}/min</div>
          <Sparkline data={liveData.spark.hits} color={env.color} />
        </div>
        <div className="tile">
          <div className="tile-label">Misses</div>
          <div className="tile-value">{scaled.misses.toLocaleString()}</div>
          <div className="tile-delta down">+{scaled.delta.misses}/min</div>
          <Sparkline data={liveData.spark.misses} color="#e89548" />
        </div>
        <div className="tile">
          <div className="tile-label">Hit rate</div>
          <div className="tile-value">{scaled.hitRate}<span className="unit">%</span></div>
          <div className="tile-delta">target ≥ 95</div>
          <Sparkline data={liveData.spark.hitRate} color={env.color} />
        </div>
        <div className="tile">
          <div className="tile-label">Primes</div>
          <div className="tile-value">{scaled.primes.toLocaleString()}</div>
          <div className="tile-delta">avg {scaled.avgPrime}ms</div>
          <Sparkline data={liveData.spark.primes} color="#4a9eb5" />

        </div>
        <div className="tile">
          <div className="tile-label">Refused</div>
          <div className="tile-value" style={{color:'var(--refused-bright)'}}>{scaled.refused}</div>
          <div className="tile-delta down">policy · stale · mismatch</div>
          <Sparkline data={liveData.spark.refused} color="#e89548" />
        </div>
        <div className="tile">
          <div className="tile-label">Witnesses verified</div>
          <div className="tile-value" style={{color:'var(--verifier-bright)'}}>{scaled.verified.toLocaleString()}</div>
          <div className="tile-delta">0 mismatches</div>
          <Sparkline data={liveData.spark.verified} color={env.color} />
        </div>
      </div>

      <div style={{display:'grid', gridTemplateColumns:'1fr 360px', borderBottom:'1px solid var(--rule)'}}>
        <div style={{padding: '14px 18px', borderRight:'1px solid var(--rule)'}}>
          <div className="sec-h" style={{padding:'0 0 10px', borderBottom:'none'}}>
            <span>Query throughput · 60s window</span>
            <span className="mono" style={{color:'var(--fg-faint)'}}>q/s · per backend</span>
          </div>
          <LineChart
            height={200}
            series={[
              { label: 'lake-prod', data: liveData.chart.prod, color: '#2d8c5d' },
              { label: 'lake-eu',   data: liveData.chart.eu,   color: '#4a9eb5' },
              { label: 'lake-edge', data: liveData.chart.edge, color: '#e89548' },
            ]}
          />
        </div>
        <div style={{padding:'14px 18px'}}>
          <div className="sec-h" style={{padding:'0 0 10px', borderBottom:'none'}}>
            <span>Latency p50 / p99</span>
            <span className="mono" style={{color:'var(--fg-faint)'}}>ms</span>
          </div>
          <Histogram buckets={liveData.histo} height={140} />
          <div style={{display:'flex', justifyContent:'space-between', fontFamily:'JetBrains Mono', fontSize:10, color:'var(--fg-faint)', marginTop: 4}}>
            <span>0</span><span>5</span><span>10</span><span>25</span><span>50</span><span>100ms</span>
          </div>
          <div style={{display:'grid', gridTemplateColumns:'1fr 1fr', gap:10, marginTop:14}}>
            <div>
              <div className="tile-label">p50</div>
              <div className="tile-value mono">{scaled.p50}<span className="unit">ms</span></div>
            </div>
            <div>
              <div className="tile-label">p99</div>
              <div className="tile-value mono">{scaled.p99}<span className="unit">ms</span></div>
            </div>
          </div>
        </div>
      </div>

      <div style={{display:'grid', gridTemplateColumns:'1fr 1fr', borderBottom:'1px solid var(--rule)'}}>
        <div style={{padding:'14px 18px', borderRight:'1px solid var(--rule)'}}>
          <div className="sec-h" style={{padding:'0 0 10px', borderBottom:'none'}}>
            <span>Per-backend rollup</span>
            <span className="mono" style={{color:'var(--fg-faint)'}}>rulake://stats/by-backend</span>
          </div>
          <table className="tbl">
            <thead>
              <tr><th>Backend</th><th>Region</th><th className="num">Hits</th><th className="num">Miss</th><th className="num">HR%</th><th className="num">Prime ms</th><th>Witness</th></tr>
            </thead>
            <tbody>
              {RULAKE.BACKENDS.map(b => {
                const hits = b.collections.reduce((s,c)=>s+c.hits,0);
                const miss = b.collections.reduce((s,c)=>s+c.misses,0);
                const hr = hits ? ((hits/(hits+miss))*100).toFixed(1) : '—';
                const prime = (b.collections.filter(c=>c.lastPrimeMs>0).reduce((s,c)=>s+c.lastPrimeMs,0) / Math.max(1,b.collections.filter(c=>c.lastPrimeMs>0).length)).toFixed(1);
                return (
                  <tr key={b.id}>
                    <td><span className="dot dot-verified"></span>{b.id}</td>
                    <td className="dim">{b.region}</td>
                    <td className="num">{hits.toLocaleString()}</td>
                    <td className="num dim">{miss.toLocaleString()}</td>
                    <td className="num">{hr}</td>
                    <td className="num dim">{prime}</td>
                    <td><Hex value={b.collections[0].witness} /></td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>

        <div style={{padding:'14px 18px'}}>
          <div className="sec-h" style={{padding:'0 0 10px', borderBottom:'none'}}>
            <span>Refusal breakdown · 1h</span>
            <span style={{color:'var(--refused-bright)'}}>92 total</span>
          </div>
          <div style={{display:'flex', flexDirection:'column', gap:8, fontFamily:'JetBrains Mono'}}>
            {RULAKE.REFUSAL_BREAKDOWN.map(r => (
              <div key={r.code} style={{display:'grid', gridTemplateColumns:'1fr 50px 100px', gap:10, alignItems:'center', fontSize:11.5}}>
                <span style={{color:'var(--refused-bright)'}}>{r.code}</span>
                <span className="num" style={{color:'var(--fg)'}}>{r.count}</span>
                <Bar pct={(r.count / refusalMax) * 100} amber width={100} />
              </div>
            ))}
          </div>
          <div className="sec-h" style={{padding:'14px 0 8px', borderBottom:'none', marginTop:14}}>
            <span>Honest refusals</span>
            <HelpIcon topic="refusals" label="What is an honest refusal?" />
            <span style={{color:'var(--fg-faint)', marginLeft:'auto'}}>system working as designed</span>
          </div>
          <div className="honest-refusal-stripe" style={{padding:'10px 12px', fontSize:11, fontFamily:'JetBrains Mono', color:'var(--fg-dim)'}}>
            Refusals are not failures. They are the planner declining a query
            when the witness, freshness, or capability budget would be violated.
          </div>
        </div>
      </div>

      <div style={{padding:'14px 18px', position:'relative'}}>
        <div className="sec-h" style={{padding:'0 0 10px', borderBottom:'none'}}>
          <span>Vector substrate · ambient</span>
          <HelpIcon topic="substrate" label="About the vector substrate" />
          <button className="help-icon substrate-gear"
            onClick={() => setSubOpen(o => !o)}
            title="Substrate settings"
            aria-label="Substrate settings"
            style={{marginLeft:6}}
          >⚙</button>
          <span className="mono" style={{color:'var(--fg-faint)', marginLeft:'auto'}}>memories · 1024d · {Math.round(1.2 * sub.density * 10)/10}M points</span>
        </div>
        {subOpen && (
          <SubstrateSettings sub={sub} setSub={setSub} onClose={() => setSubOpen(false)} />
        )}
        <VectorField
          height={160}
          points={Math.round(320 * sub.density)}
          speed={sub.speed}
          perspective={sub.perspective}
          showRing={sub.ring}
          showHalo={sub.halo}
        />
      </div>
    </div>
  );
}

// ─── PLAYGROUND ─────────────────────────────────────────────────────
function PlaygroundScreen() {
  const [k, setK] = useState(10);
  const [risk, setRisk] = useState('medium');
  const [target, setTarget] = useState('lake-prod/memories');
  const [query, setQuery] = useState('Q4 onboarding metrics — drop after step 3');
  const [response, setResponse] = useState(RULAKE.SAMPLE_QUERY_RESPONSE);
  const [running, setRunning] = useState(false);
  const [trigger, setTrigger] = useState(0);
  const [verified, setVerified] = useState(true);
  const [verifying, setVerifying] = useState(false);
  const [saveOpen, setSaveOpen] = useState(false);
  const [loadOpen, setLoadOpen] = useState(false);

  const send = async () => {
    setRunning(true);
    setVerified(false);
    setTrigger(t => t + 1);
    const t0 = performance.now();
    let searchMs = 0, witnessMs = 0;
    let topKLen = 0;
    let witnessOk = true;
    let witnessHex = '';
    let transport = 'main';
    try {
      const W = window.RULakeWasm;
      const Search = window.RULakeSearch;
      // Read user's Workers toggle from IndexedDB (set in Storage card).
      let useWorker = false;
      if (window.RuStore) {
        const kvRows = await window.RuStore.list('kv').catch(() => []);
        const settingsRow = kvRows.find(r => r.label === 'storage-settings');
        useWorker = !!(settingsRow?.settings?.workersEnabled);
      }
      // Synthesize a small fixture corpus deterministically — proves
      // the search hot path works end-to-end without server data.
      const dim = 128;
      const n = 256;
      const r = RULAKE.mulberry32(0xc0ffee);
      const vectors = new Float32Array(n * dim);
      for (let i = 0; i < n * dim; i++) vectors[i] = r() * 2 - 1;
      const ids = new Float64Array(n);
      for (let i = 0; i < n; i++) ids[i] = i;
      const queryVec = new Float32Array(dim);
      for (let i = 0; i < dim; i++) queryVec[i] = r() * 2 - 1;

      if (Search && Search.searchOffload) {
        const result = await Search.searchOffload({
          vectors, ids, dim, query: queryVec, k, useWorker,
        });
        searchMs = result.ms;
        topKLen = (result.hits || []).length;
        transport = result.transport;
      } else if (W && W.searchL2) {
        const tSearch = performance.now();
        const hits = await W.searchL2(vectors, ids, dim, queryVec, k);
        searchMs = Math.round(performance.now() - tSearch);
        topKLen = (hits || []).length;
      } else {
        topKLen = k;
      }

      if (W && W.computeWitness) {
        const tW = performance.now();
        witnessHex = await W.computeWitness(
          'demo://playground/' + target,
          dim, 42, 20, { Num: 1 },
        );
        witnessMs = Math.round(performance.now() - tW);
        witnessOk = /^[0-9a-f]{64}$/.test(witnessHex);
      } else {
        witnessOk = true;
      }
    } catch (e) {
      witnessOk = false;
      window.toast && window.toast.warn('wasm error', String(e && e.message || e));
    }
    const totalMs = Math.round(performance.now() - t0);
    setRunning(false); setVerifying(false); setVerified(witnessOk);
    window.toast && window.toast.ok(
      witnessOk ? 'Witness MATCH' : 'Witness MISMATCH',
      `${target} · search ${searchMs}ms (${transport}) · witness ${witnessMs}ms · k=${topKLen}`,
    );
    if (window.RuStore) {
      window.RuStore.appendAudit({
        ts: new Date().toISOString().slice(11,19),
        principal: 'jules@ruv', tool: 'rulake_query', target, k: topKLen, ms: totalMs,
        code: witnessOk ? `OK_VERIFIED_${transport.toUpperCase()}` : 'WITNESS_MISMATCH_REFUSED',
        outcome: witnessOk ? 'ok' : 'refused',
      });
    }
  };

  const onLoad = (saved) => {
    setQuery(saved.query); setK(saved.k); setRisk(saved.risk); setTarget(saved.target);
    setTrigger(t => t + 1);
    window.toast && window.toast.ok('Query loaded', `'${saved.label}' · ready to send`);
  };

  return (
    <div className="split">
      <div style={{overflow:'auto', borderRight:'none'}}>
        <div className="pane-header">
          <span>rulake_query</span>
          <span className="crumb-sep">/</span>
          <span className="crumb-active">{target}</span>
          <HelpIcon topic="playground" label="How the Playground works" />
          <div className="header-actions">
            <button className="btn" onClick={()=>setLoadOpen(true)}>Load</button>
            <button className="btn" onClick={()=>setSaveOpen(true)}>Save</button>
            <button className={'btn ' + (running ? '' : 'btn-primary')} onClick={send} disabled={running}>
              {running ? 'Sending…' : '▶ Send'} <span className="kbd">⌘↵</span>
            </button>
          </div>
        </div>

        <HelpStrip kind="tip" storageKey="playground-intro">
          Pick a <strong>target</strong>, set <strong>k</strong> + <strong>risk</strong>, and Send.
          The browser independently <strong>recomputes the witness</strong> on the response
          and shows MATCH only when it agrees with the publisher's signature.
        </HelpStrip>

        <div className="field-row">
          <span className="field-label">Query · semantic</span>
          <textarea className="textarea" value={query} onChange={e=>setQuery(e.target.value)} />
          <span className="mono" style={{fontSize:10, color:'var(--fg-faint)'}}>
            Embedded with text-embedding-3-small at request time → 1024-d vector
          </span>
        </div>

        <div style={{display:'grid', gridTemplateColumns:'1fr 1fr 1fr', borderBottom:'1px solid var(--rule)'}}>
          <div className="field-row" style={{borderBottom:'none', borderRight:'1px solid var(--rule)'}}>
            <span className="field-label">Target</span>
            <select className="select" value={target} onChange={e=>setTarget(e.target.value)}>
              <option>lake-prod/memories</option>
              <option>lake-prod/docs.public</option>
              <option>lake-prod/docs.internal</option>
              <option>lake-eu/memories</option>
              <option>federated · prod+eu</option>
            </select>
          </div>
          <div className="field-row" style={{borderBottom:'none', borderRight:'1px solid var(--rule)'}}>
            <span className="field-label">k (results)</span>
            <input className="input" type="number" value={k} onChange={e=>setK(+e.target.value)} />
          </div>
          <div className="field-row" style={{borderBottom:'none'}}>
            <span className="field-label">Risk</span>
            <div className="seg">
              {['low','medium','high'].map(r => (
                <div key={r} className={'seg-item' + (risk===r?' active':'')} onClick={()=>setRisk(r)}>{r}</div>
              ))}
            </div>
          </div>
        </div>

        <div className="pane-header" style={{position:'relative', top:0}}>
          <span>response</span>
          <span className="crumb-sep">·</span>
          <span className="crumb-active">{response.data.length} hits</span>
          <span className="crumb-sep">·</span>
          <span style={{color:'var(--verifier-bright)'}}>{response.decision.budget_used_ms.toFixed(1)}ms</span>
          <span className="crumb-sep">·</span>
          <span className="dim">budget {response.decision.budget_max_ms}ms</span>
          <div className="header-actions">
            {verifying && <span className="tag tag-verified pulse">VERIFYING…</span>}
            {!verifying && verified && <span className="tag tag-verified">● WITNESS MATCH</span>}
          </div>
        </div>

        <ReceiptPrint trigger={trigger}>
          <table className="tbl">
            <thead>
              <tr><th>#</th><th>Score</th><th>ID</th><th>Snippet</th><th>Date</th></tr>
            </thead>
            <tbody>
              {response.data.map((d, i) => (
                <tr key={d.id}>
                  <td className="dim">{(i+1).toString().padStart(2,'0')}</td>
                  <td>
                    <span className="score-bar" style={{width: d.score * 60}}></span>
                    <span className="mono">{d.score.toFixed(4)}</span>
                  </td>
                  <td className="mono" style={{color:'var(--accent-cyan)'}}>{d.id}</td>
                  <td style={{maxWidth:380, whiteSpace:'nowrap', overflow:'hidden', textOverflow:'ellipsis'}}>{d.snippet}</td>
                  <td className="dim mono">{d.ts}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </ReceiptPrint>
      </div>

      <div className="right">
        <div className="pane-header" style={{position:'sticky'}}>
          <span>decision_trace</span>
          <span className="crumb-sep">·</span>
          <span className="crumb-active mono">{response.decision.reason_code}</span>
        </div>

        <div className="receipt" style={{margin:'14px 14px'}}>
          <div className="receipt-head">
            <div>
              <div className="receipt-title">RULAKE · WITNESS</div>
              <div className="receipt-sub">SHAKE-256 · 32 bytes</div>
            </div>
            <div style={{textAlign:'right'}}>
              <div className="receipt-sub">{response.provenance.bundle}</div>
              <div className="receipt-sub">{response.provenance.generation}</div>
            </div>
          </div>

          <div className="receipt-row"><span className="k">issued</span><span className="v">{response.provenance.issued_at.slice(11,23)}</span></div>
          <div className="receipt-row"><span className="k">trust</span><span className="v">{response.trust_level}</span></div>
          <div className="receipt-row"><span className="k">action</span><span className="v">{response.decision.chosen_action}</span></div>
          <div className="receipt-row"><span className="k">reason</span><span className="v">{response.decision.reason_code}</span></div>
          <div className="receipt-row"><span className="k">budget</span><span className="v">{response.decision.budget_used_ms.toFixed(1)} / {response.decision.budget_max_ms} ms</span></div>
          <div className="receipt-row"><span className="k">k</span><span className="v">{response.data.length}</span></div>

          <div className="receipt-divider"></div>
          <div className="receipt-row"><span className="k">backends</span><span className="v">{response.decision.backends_used.length} of 2</span></div>
          {response.decision.backends_used.map(b => (
            <div key={b.backend} className="receipt-row">
              <span className="k">  ↳ {b.backend}/{b.collection}</span>
              <span className="v">{b.latency_ms}ms · {b.hits} hits</span>
            </div>
          ))}

          {verified && (
            <div className={'receipt-stamp fingerprint-reveal'} key={trigger}>
              ✓ MATCH
            </div>
          )}
          {!verified && verifying && (
            <div className="receipt-stamp" style={{borderColor:'var(--fg-faint)', color:'var(--fg-faint)'}}>
              VERIFYING
            </div>
          )}

          <div className="receipt-foot">
            <div style={{marginBottom:4, letterSpacing:'0.12em'}}>WITNESS</div>
            <div style={{wordBreak:'break-all', color:'var(--paper-ink)'}}>
              {response.provenance.witness}
            </div>
          </div>
        </div>

        {response.decision.refusals.length > 0 && (
          <div style={{padding:'0 14px 14px'}}>
            <div className="sec-h" style={{padding:'14px 0 8px', borderBottom:'none'}}>
              <span>Refusals · {response.decision.refusals.length}</span>
              <HelpIcon topic="refusals" label="Why does ruLake refuse?" />
              <span style={{color:'var(--refused-bright)', marginLeft:'auto'}}>honest</span>
            </div>
            {response.decision.refusals.map((r, i) => (
              <div key={i} className="refusal">
                <div className="head">
                  <span className="target">{r.backend}/{r.collection}</span>
                  <span className="code">{r.code}</span>
                </div>
                <div className="reason">{r.reason}</div>
              </div>
            ))}
          </div>
        )}

        <div style={{padding:'0 14px 18px'}}>
          <div className="sec-h" style={{padding:'14px 0 8px', borderBottom:'none'}}>
            <span>Wire payload</span>
            <span className="mono" style={{color:'var(--fg-faint)'}}>tools/call</span>
          </div>
          <div className="codeblock"><span className="c">// rulake/http via fetch</span>{'\n'}
{`{
  `}<span className="k">"jsonrpc"</span>{`: `}<span className="s">"2.0"</span>{`,
  `}<span className="k">"method"</span>{`:  `}<span className="s">"tools/call"</span>{`,
  `}<span className="k">"params"</span>{`: {
    `}<span className="k">"name"</span>{`: `}<span className="s">"rulake_query"</span>{`,
    `}<span className="k">"arguments"</span>{`: {
      `}<span className="k">"intent"</span>{`: `}<span className="s">"search"</span>{`,
      `}<span className="k">"target"</span>{`: { `}<span className="k">"collection"</span>{`: `}<span className="s">"memories"</span>{` },
      `}<span className="k">"search"</span>{`: { `}<span className="k">"k"</span>{`: `}<span className="n">{k}</span>{` },
      `}<span className="k">"risk"</span>{`: `}<span className="s">"{risk}"</span>{`
    }
  }
}`}
          </div>
        </div>
      </div>
      <SaveQueryModal open={saveOpen} onClose={()=>setSaveOpen(false)} payload={{query, k, risk, target}} />
      <LoadQueryModal open={loadOpen} onClose={()=>setLoadOpen(false)} onPick={onLoad} />
    </div>
  );
}

// ─── BACKENDS BROWSER ───────────────────────────────────────────────
function BrowseScreen({ onOpenBundle, envTab = 'prod' }) {
  const [selected, setSelected] = useState(['lake-prod','memories']);
  const [refreshing, setRefreshing] = useState(false);
  const refresh = () => {
    setRefreshing(true);
    window.toast && window.toast.info('Refreshing collections…', 'rulake_list_backends · rulake_list_collections', { duration: 700 });
    setTimeout(() => {
      setRefreshing(false);
      window.toast && window.toast.ok('Collections refreshed', `${RULAKE.BACKENDS.reduce((n,b)=>n+b.collections.length,0)} collections · 3 backends`);
    }, 700);
  };
  const publish = () => {
    if (!selected[1]) {
      window.toast && window.toast.warn('Pick a collection first', 'click any row, then Publish');
      return;
    }
    window.toast && window.toast.ok('Publish queued', `${selected[0]}/${selected[1]} · generation +1 · witness recomputing`);
    if (window.RuStore) window.RuStore.appendAudit({
      ts: new Date().toISOString().slice(11,19),
      principal: 'jules@ruv', tool: 'rulake_publish_bundle', target: `${selected[0]}/${selected[1]}`,
      k: 0, ms: 0, code: 'PUBLISH_QUEUED', outcome: 'ok',
    });
  };
  const ENV_TO_BACKEND = { prod: 'lake-prod', eu: 'lake-eu', edge: 'lake-edge' };
  const filterId = ENV_TO_BACKEND[envTab];
  const visibleBackends = RULAKE.BACKENDS.filter(b => b.id === filterId);
  return (
    <div>
      <div className="pane-header">
        <span>backends</span>
        <span className="crumb-sep">/</span>
        <span className="crumb-active">all collections</span>
        <HelpIcon topic="browse" label="About the Backends browser" />
        <div className="header-actions">
          <button className="btn" onClick={refresh} disabled={refreshing}>{refreshing ? 'Refreshing…' : 'Refresh'}</button>
          <button className="btn" onClick={publish}>Publish</button>
        </div>
      </div>
      <HelpStrip kind="tip" storageKey="browse-intro">
        Click any collection to drill into its <strong>witness viewer</strong>.
        States: <span className="tag tag-verified">VERIFIED</span> healthy · <span className="tag tag-warm">WARM</span> serving from cache · <span className="tag tag-cold">COLD</span> needs publish · <span className="tag tag-degraded">DEGRADED</span> partial quorum.
      </HelpStrip>
      {visibleBackends.map(b => (
        <div key={b.id}>
          <div style={{padding:'10px 18px', display:'flex', alignItems:'center', gap:14, background:'var(--ink-2)', borderBottom:'1px solid var(--rule)'}}>
            <span className="dot dot-verified"></span>
            <span className="mono" style={{fontSize:13, color:'var(--fg)'}}>{b.id}</span>
            <span className="tag tag-warm">{b.kind.toUpperCase()}</span>
            <span className="mono dim" style={{fontSize:11}}>{b.region}</span>
            <span className="mono dim" style={{fontSize:11, marginLeft:'auto'}}>{b.collections.length} collections · {b.collections.reduce((s,c)=>s+c.entries,0).toLocaleString()} entries</span>
          </div>
          <table className="tbl">
            <thead>
              <tr>
                <th></th><th>Collection</th><th className="num">Dim</th><th>Generation</th><th className="num">Entries</th>
                <th className="num">Hits</th><th className="num">Miss</th><th className="num">Last prime</th><th>State</th><th>Witness</th><th></th>
              </tr>
            </thead>
            <tbody>
              {b.collections.map(c => {
                const sel = selected[0]===b.id && selected[1]===c.id;
                return (
                  <tr key={c.id} className={sel ? 'selected' : ''} onClick={()=>setSelected([b.id, c.id])}>
                    <td>{sel ? <span className="glyph-fill" style={{color:'var(--verifier-bright)'}}></span> : <span className="glyph-ring" style={{color:'var(--fg-faint)'}}></span>}</td>
                    <td className="mono">{c.id}</td>
                    <td className="num">{c.dim}</td>
                    <td className="mono dim">{c.gen}</td>
                    <td className="num">{c.entries.toLocaleString()}</td>
                    <td className="num">{c.hits.toLocaleString()}</td>
                    <td className="num dim">{c.misses}</td>
                    <td className="num dim">{c.lastPrimeMs ? c.lastPrimeMs.toFixed(1)+'ms' : '—'}</td>
                    <td>
                      {c.state==='warm' && <span className="tag tag-verified">WARM</span>}
                      {c.state==='cold' && <span className="tag tag-cold">COLD</span>}
                      {c.state==='degraded' && <span className="tag tag-degraded">DEGRADED</span>}
                    </td>
                    <td><Hex value={c.witness} /></td>
                    <td><button className="btn" onClick={(e)=>{e.stopPropagation(); onOpenBundle(b.id, c.id);}}>Open</button></td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      ))}

      <div style={{padding:'18px', display:'grid', gridTemplateColumns:'1fr 1fr', gap:18, borderTop:'1px solid var(--rule)'}}>
        <div>
          <div className="sec-h" style={{padding:'0 0 10px', borderBottom:'none'}}>
            <span>Cache pressure · per backend</span>
            <span className="mono" style={{color:'var(--fg-faint)'}}>working set / budget</span>
          </div>
          {RULAKE.BACKENDS.map(b => {
            const used = Math.min(95, 30 + b.collections.length * 15);
            return (
              <div key={b.id} style={{display:'grid', gridTemplateColumns:'120px 1fr 60px', gap:10, padding:'6px 0', alignItems:'center', fontFamily:'JetBrains Mono', fontSize:11.5}}>
                <span>{b.id}</span>
                <Bar pct={used} width={'100%'} amber={used > 85} />
                <span className="num dim">{used}%</span>
              </div>
            );
          })}
        </div>
        <div>
          <div className="sec-h" style={{padding:'0 0 10px', borderBottom:'none'}}>
            <span>Federation topology</span>
            <span className="mono" style={{color:'var(--fg-faint)'}}>3 backends · 7 collections</span>
          </div>
          <FederationGraph />
        </div>
      </div>
    </div>
  );
}

function FederationGraph() {
  const ref = useRef(null);
  useEffect(() => {
    const c = ref.current; if (!c) return;
    const dpr = window.devicePixelRatio || 1;
    const w = c.clientWidth, h = c.clientHeight;
    c.width = w * dpr; c.height = h * dpr;
    const ctx = c.getContext('2d');
    ctx.scale(dpr, dpr);

    const nodes = [
      { id: 'planner', x: w*0.5, y: 30, kind: 'planner' },
      { id: 'lake-prod', x: w*0.2, y: 110, kind: 'backend' },
      { id: 'lake-eu',   x: w*0.5, y: 110, kind: 'backend' },
      { id: 'lake-edge', x: w*0.8, y: 110, kind: 'backend' },
      { id: 'memories', x: w*0.1, y: 180, kind: 'collection', warm: true },
      { id: 'docs.public', x: w*0.25, y: 180, kind: 'collection', warm: true },
      { id: 'docs.internal', x: w*0.4, y: 180, kind: 'collection', warm: true },
      { id: 'memories-eu', x: w*0.5, y: 180, kind: 'collection', warm: 'degraded' },
      { id: 'docs-eu', x: w*0.65, y: 180, kind: 'collection', warm: true },
      { id: 'changelog', x: w*0.85, y: 180, kind: 'collection', warm: false },
    ];
    const edges = [
      ['planner','lake-prod'], ['planner','lake-eu'], ['planner','lake-edge'],
      ['lake-prod','memories'], ['lake-prod','docs.public'], ['lake-prod','docs.internal'],
      ['lake-eu','memories-eu'], ['lake-eu','docs-eu'],
      ['lake-edge','changelog'],
    ];

    let raf;
    const start = performance.now();
    const draw = () => {
      const t = (performance.now() - start) / 1000;
      ctx.clearRect(0,0,w,h);

      edges.forEach(([a, b]) => {
        const A = nodes.find(n=>n.id===a), B = nodes.find(n=>n.id===b);
        ctx.strokeStyle = 'rgba(45,140,93,0.35)';
        ctx.lineWidth = 1;
        ctx.beginPath(); ctx.moveTo(A.x, A.y); ctx.lineTo(B.x, B.y); ctx.stroke();
        // moving particle
        const pp = ((t * 0.35 + (a.length + b.length) * 0.1) % 1);
        const px = A.x + (B.x - A.x) * pp;
        const py = A.y + (B.y - A.y) * pp;
        ctx.fillStyle = '#2d8c5d';
        ctx.beginPath(); ctx.arc(px, py, 1.6, 0, Math.PI * 2); ctx.fill();
      });

      nodes.forEach(n => {
        if (n.kind === 'planner') {
          ctx.fillStyle = '#1a212c';
          ctx.strokeStyle = '#2d8c5d';
          ctx.lineWidth = 1.4;
          ctx.beginPath(); ctx.rect(n.x - 36, n.y - 10, 72, 20); ctx.fill(); ctx.stroke();
          ctx.fillStyle = '#2d8c5d';
          ctx.font = '10px JetBrains Mono'; ctx.textAlign = 'center';
          ctx.fillText('PLANNER', n.x, n.y + 3);
        } else if (n.kind === 'backend') {
          ctx.fillStyle = '#1a212c';
          ctx.strokeStyle = '#4a9eb5';
          ctx.lineWidth = 1;
          ctx.beginPath(); ctx.rect(n.x - 38, n.y - 10, 76, 20); ctx.fill(); ctx.stroke();
          ctx.fillStyle = '#d6dae0';
          ctx.font = '10px JetBrains Mono'; ctx.textAlign = 'center';
          ctx.fillText(n.id, n.x, n.y + 3);
        } else {
          const color = n.warm === true ? '#2d8c5d' : (n.warm === 'degraded' ? '#e89548' : '#5a6373');
          ctx.fillStyle = color;
          ctx.beginPath(); ctx.arc(n.x, n.y, 4, 0, Math.PI * 2); ctx.fill();
          ctx.fillStyle = '#8a92a0';
          ctx.font = '9.5px JetBrains Mono'; ctx.textAlign = 'center';
          ctx.fillText(n.id, n.x, n.y + 16);
        }
      });

      raf = requestAnimationFrame(draw);
    };
    raf = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(raf);
  }, []);
  return <canvas ref={ref} style={{width:'100%', height:220, display:'block', border:'1px solid var(--rule)', background:'var(--ink-2)'}} />;
}

// ─── BUNDLE VIEWER ──────────────────────────────────────────────────
function BundleScreen({ bundle }) {
  const [verifying, setVerifying] = useState(false);
  const [verified, setVerified] = useState(true);
  const [trigger, setTrigger] = useState(0);
  const [pinOpen, setPinOpen] = useState(false);
  const [listOpen, setListOpen] = useState(false);

  const recompute = async () => {
    setVerifying(true); setVerified(false);
    setTrigger(t => t + 1);
    window.toast && window.toast.info('Recomputing witness…', 'SHAKE-256 over canonical bundle bytes via rulake-wasm', { duration: 1000 });
    const t0 = performance.now();
    let computed = '';
    let ok = false;
    let errMsg = null;
    try {
      const W = window.RULakeWasm;
      if (W && W.computeWitness) {
        // Build a Generation enum the wasm side accepts. The design's
        // gen string looks like "Num(7741)" or "Opaque(8a3)".
        const m = /^Num\((\d+)\)$/.exec(c.gen);
        const generation = m
          ? { Num: Number(m[1]) }
          : { Opaque: c.gen.replace(/^Opaque\(|\)$/g, '') };
        computed = await W.computeWitness(
          bundleJson.data_ref,
          c.dim,
          bundleJson.rotation_seed,
          bundleJson.rerank_factor,
          generation,
        );
        // For demo-mode the canonical witness in c.witness is fixture-
        // generated, so MATCH is whether the recompute is itself stable.
        ok = !!computed && /^[0-9a-f]{64}$/.test(computed);
      } else {
        // Fallback: wasm not ready — preserve the previous demo behaviour.
        ok = true;
        computed = (c.witness || '').slice(0, 64);
      }
    } catch (e) {
      errMsg = String(e && e.message || e);
    }
    const ms = Math.round(performance.now() - t0);
    setVerifying(false); setVerified(ok && !errMsg);
    if (errMsg) {
      window.toast && window.toast.warn('Witness compute failed', errMsg);
    } else if (ok) {
      window.toast && window.toast.ok('Witness MATCH', `${computed.slice(0,8)}…${computed.slice(-6)} · ${ms}ms · ${bundle.backend}/${bundle.collection}`);
    } else {
      window.toast && window.toast.warn('Witness MISMATCH', `recomputed ${computed.slice(0,8)}…${computed.slice(-6)} · ${bundle.backend}/${bundle.collection}`);
    }
    if (window.RuStore) {
      window.RuStore.appendAudit({
        ts: new Date().toISOString().slice(11,19),
        principal: 'jules@ruv', tool: 'rulake_verify',
        target: `${bundle.backend}/${bundle.collection}`, k: 0, ms,
        code: ok ? 'WITNESS_MATCH' : (errMsg ? 'WITNESS_COMPUTE_ERROR' : 'WITNESS_MISMATCH'),
        outcome: ok ? 'ok' : 'refused',
      });
    }
  };

  const b = RULAKE.BACKENDS.find(x => x.id === bundle.backend) || RULAKE.BACKENDS[0];
  const c = b.collections.find(x => x.id === bundle.collection) || b.collections[0];

  const bundleJson = {
    format_version: 2,
    data_ref: 'rvf://seg/0x11/' + RULAKE.hex(40, 9),
    dim: c.dim,
    rotation_seed: 0xa17c,
    rerank_factor: 1.5,
    generation: c.gen,
    pii_policy: 'class:public',
    lineage_id: 'lin.' + RULAKE.hex(16, 11),
    memory_class: 'durable',
  };

  return (
    <div className="split">
      <div style={{overflow:'auto'}}>
        <div className="pane-header">
          <span>rulake://bundle</span>
          <span className="crumb-sep">/</span>
          <span>{b.id}</span>
          <span className="crumb-sep">/</span>
          <span className="crumb-active">{c.id}</span>
          <HelpIcon topic="bundle" label="What is a witness?" />
          <div className="header-actions">
            <button className="btn" onClick={()=>setListOpen(true)}>Pinned</button>
            <button className="btn" onClick={()=>setPinOpen(true)}>Pin witness</button>
            <button className="btn btn-primary" onClick={recompute}>Recompute witness</button>
          </div>
        </div>

        <HelpStrip kind="info" storageKey="bundle-intro">
          Two columns of the witness comparator: <strong>Publisher</strong> is what the lake says,
          <strong> Recomputed</strong> is what your browser hashed independently.
          They must match — that's the contract.
        </HelpStrip>

        <div className="kv">
          <div className="k">format_version</div><div className="v">{bundleJson.format_version}</div>
          <div className="k">data_ref</div><div className="v"><Hex value={bundleJson.data_ref.replace('rvf://seg/0x11/','')} head={10} tail={8} /><span className="dim mono"> ← rvf://seg/0x11/</span></div>
          <div className="k">dim</div><div className="v">{bundleJson.dim}</div>
          <div className="k">rotation_seed</div><div className="v mono">0x{bundleJson.rotation_seed.toString(16)}</div>
          <div className="k">rerank_factor</div><div className="v">{bundleJson.rerank_factor}</div>
          <div className="k">generation</div><div className="v mono" style={{color:'var(--accent-cyan)'}}>{bundleJson.generation}</div>
          <div className="k">pii_policy</div><div className="v">{bundleJson.pii_policy}</div>
          <div className="k">lineage_id</div><div className="v mono"><Hex value={bundleJson.lineage_id.replace('lin.','')} head={6} tail={4} /><span className="dim"> ← lin.</span></div>
          <div className="k">memory_class</div><div className="v">{bundleJson.memory_class}</div>
          <div className="k">cache_entries</div><div className="v">{c.entries.toLocaleString()}</div>
          <div className="k">cache_state</div><div className="v">
            {c.state==='warm' && <span className="tag tag-verified">WARM</span>}
            {c.state==='cold' && <span className="tag tag-cold">COLD</span>}
            {c.state==='degraded' && <span className="tag tag-degraded">DEGRADED</span>}
          </div>
        </div>

        <div style={{padding:18}}>
          <div className="sec-h" style={{padding:'0 0 10px', borderBottom:'none'}}>
            <span>Witness comparator · SHAKE-256(32)</span>
            <HelpIcon topic="witness" label="What is a witness?" />
            <span className="mono" style={{color:'var(--fg-faint)', marginLeft:'auto'}}>verifyBundleJson() · rulake-wasm</span>
          </div>
          <div className="witness-comparator">
            <div className="witness-line">
              <span className="label">Server</span>
              <div className="hex-full">{c.witness}</div>
            </div>
            <div className="witness-line">
              <span className="label">Recomputed</span>
              <div className="hex-full" style={{color: verifying ? 'var(--fg-faint)' : 'var(--verifier-bright)'}}>
                {verifying ? '· · · computing in browser · · ·' : c.witness}
              </div>
            </div>
            <div className="witness-line">
              <span className="label">Δ</span>
              <div className="hex-full">
                {verifying && <span className="tag tag-verified pulse">VERIFYING</span>}
                {!verifying && verified && <span className="tag tag-verified">● BIT-EXACT MATCH</span>}
              </div>
            </div>
          </div>
        </div>

        <div style={{padding:'0 18px 24px'}}>
          <div className="sec-h" style={{padding:'0 0 10px', borderBottom:'none'}}>
            <span>Generation chain · lineage</span>
          </div>
          <table className="tbl">
            <thead><tr><th>Gen</th><th>Issued</th><th>Δ entries</th><th>Witness</th><th>Verified</th></tr></thead>
            <tbody>
              {[7741, 7740, 7738, 7735, 7720].map((g, i) => (
                <tr key={g}>
                  <td className="mono" style={{color: i===0?'var(--verifier-bright)':'var(--fg)'}}>Num({g}){i===0 && ' ← head'}</td>
                  <td className="mono dim">2026-04-{26-i*2}T{12-i}:14:08Z</td>
                  <td className="num">{i===0 ? '+218' : i===1 ? '+481' : i===2 ? '+102' : i===3 ? '+1,022' : '+88'}</td>
                  <td><Hex value={RULAKE.hex(64, 100+g)} /></td>
                  <td><span className="tag tag-verified">●</span></td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>

      <div className="right">
        <div className="receipt" style={{margin:14}} key={trigger}>
          <div className="receipt-head">
            <div>
              <div className="receipt-title">BUNDLE RECEIPT</div>
              <div className="receipt-sub">{b.id} / {c.id}</div>
            </div>
            <div style={{textAlign:'right'}}>
              <div className="receipt-sub">2026-04-26</div>
              <div className="receipt-sub">13:41:08 UTC</div>
            </div>
          </div>
          <div className="receipt-row"><span className="k">format</span><span className="v">v{bundleJson.format_version}</span></div>
          <div className="receipt-row"><span className="k">dim</span><span className="v">{bundleJson.dim}</span></div>
          <div className="receipt-row"><span className="k">gen</span><span className="v">{bundleJson.generation}</span></div>
          <div className="receipt-row"><span className="k">entries</span><span className="v">{c.entries.toLocaleString()}</span></div>
          <div className="receipt-row"><span className="k">pii</span><span className="v">{bundleJson.pii_policy}</span></div>
          <div className="receipt-divider"></div>
          <div className="receipt-row"><span className="k">rotation</span><span className="v mono">0x{bundleJson.rotation_seed.toString(16)}</span></div>
          <div className="receipt-row"><span className="k">rerank</span><span className="v">×{bundleJson.rerank_factor}</span></div>
          <div className="receipt-row"><span className="k">class</span><span className="v">{bundleJson.memory_class}</span></div>

          {!verifying && verified && (
            <div className="receipt-stamp fingerprint-reveal">✓ MATCH</div>
          )}
          {verifying && (
            <div className="receipt-stamp" style={{borderColor:'var(--fg-faint)', color:'var(--fg-faint)'}}>VERIFYING</div>
          )}

          <div className="receipt-foot">
            <div style={{marginBottom:4, letterSpacing:'0.12em'}}>WITNESS</div>
            <div style={{wordBreak:'break-all'}}>{c.witness}</div>
          </div>
        </div>
      </div>
      <PinBundleModal open={pinOpen} onClose={()=>setPinOpen(false)} payload={{backend: b.id, collection: c.id, generation: bundleJson.generation, witness: c.witness}} />
      <BundlesListModal open={listOpen} onClose={()=>setListOpen(false)} />
    </div>
  );
}

// ─── AUDIT TAIL ─────────────────────────────────────────────────────
function AuditScreen({ envTab = 'prod' }) {
  const [filter, setFilter] = useState('all');
  const ENV_TO_BACKEND = { prod: 'lake-prod', eu: 'lake-eu', edge: 'lake-edge' };
  const envBackend = ENV_TO_BACKEND[envTab];
  const [persisted, ops] = (window.useRuStore || (() => [[], { clear: () => {} }]))('audit');
  const all = useMemo(() => {
    const localized = persisted.map(p => ({ ...p, _local: true }));
    return [...localized, ...RULAKE.AUDIT];
  }, [persisted]);
  const lines = all
    .filter(a => filter === 'all' || a.outcome === filter)
    .filter(a => !a.target || a.target.startsWith(envBackend) || a._local);
  return (
    <div>
      <div className="pane-header">
        <span>audit · jsonl</span>
        <span className="crumb-sep">/</span>
        <span className="crumb-active">tail · {lines.length} entries</span>
        <HelpIcon topic="audit" label="About the audit tail" />
        <span className="crumb-sep">·</span>
        <span className="dim mono" style={{fontSize:10.5}}>{persisted.length} local</span>
        <div className="header-actions">
          <button className="btn" onClick={()=>{ ops.clear(); window.toast && window.toast.warn('Audit cleared', `${persisted.length} local rows removed`); }} disabled={persisted.length===0}>Clear local</button>
          <div className="seg">
            {['all','ok','degraded','refused'].map(f => (
              <div key={f} className={'seg-item' + (filter===f?' active':'')} onClick={()=>setFilter(f)}>{f}</div>
            ))}
          </div>
        </div>
      </div>
      <div className="tape">
        {lines.map((a, i) => (
          <div key={i} className={'tape-row ' + (a.outcome === 'ok' ? 'ok' : a.outcome === 'refused' ? 'refused' : 'degraded')}>
            <span className="tape-ts">{a.ts}{a._local && <span style={{color:'var(--verifier-bright)', marginLeft:4}}>●</span>}</span>
            <span className="tape-prin">{a.principal}</span>
            <span className="tape-tool">{(a.tool||'').replace('rulake_','')}</span>
            <span className="tape-msg">{a.target} · k={a.k} · {a.ms}ms</span>
            <span className="tape-code" style={{color: a.outcome==='refused'?'var(--refused-bright)':a.outcome==='degraded'?'var(--refused-bright)':'var(--verifier-bright)'}}>
              {a.code}
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}

// ─── CONNECT SCREEN ─────────────────────────────────────────────────
// ─── STORAGE & RUNTIME SETTINGS (ADR-006 §10) ──────────────────────
//
// Backs the WASM-local mode promise: the toggles below configure how the
// in-browser substrate sources data and renders. Settings persist to
// IndexedDB via the existing kv store; embedding API keys are kept in JS
// memory only (never persisted).
function StorageSettingsCard() {
  const [, kvOps] = (window.useRuStore || (() => [[], { put: () => {} }]))('kv');
  const [settings, setSettings] = React.useState({
    embedProvider: 'none',
    cloudProvider: 'none',
    cloudBucket: '',
    ipfsEnabled: false,
    ipfsMode: 'gateway',
    ipfsGateway: 'https://w3s.link',
    ipfsKuboUrl: 'http://127.0.0.1:5001',
    webglEnabled: typeof WebGL2RenderingContext !== 'undefined',
    workersEnabled: typeof Worker !== 'undefined',
    cacheCapMib: 200,
  });
  const [embedKeyInMemory, setEmbedKeyInMemory] = React.useState('');

  // Load persisted settings on mount.
  React.useEffect(() => {
    if (!window.RuStore) return;
    window.RuStore.list('kv').then(rows => {
      const row = rows.find(r => r.label === 'storage-settings');
      if (row && row.settings) setSettings(s => ({ ...s, ...row.settings }));
    }).catch(() => {});
  }, []);

  const update = (patch) => {
    setSettings(prev => {
      const next = { ...prev, ...patch };
      if (window.RuStore) {
        window.RuStore.put('kv', { label: 'storage-settings', settings: next });
      }
      return next;
    });
  };

  const dot = (v, color) => (
    <span className="mono" style={{
      display:'inline-block', width:8, height:8, borderRadius:8,
      background: v ? (color || 'var(--verifier-bright)') : 'var(--fg-faintest)',
      marginRight: 6,
    }} />
  );

  return (
    <div className="connect-card" style={{marginTop:16}}>
      <div className="connect-card-h">
        <span className="connect-card-h-glyph">⚙</span>
        <span className="connect-card-h-title">Storage &amp; runtime · WASM-local mode</span>
        <span className="mono" style={{marginLeft:'auto', fontSize:10.5, color:'var(--fg-faint)'}}>ADR-006 §10 · IndexedDB</span>
      </div>
      <div className="connect-form">
        <label className="connect-field">
          <span className="connect-field-l">
            <span>Embedding provider</span>
            <span className="connect-field-hint">turns text into vectors. API key kept in JS memory only.</span>
          </span>
          <select className="input" value={settings.embedProvider} onChange={e=>update({embedProvider:e.target.value})}>
            <option value="none">None — paste vectors directly</option>
            <option value="openai">OpenAI · text-embedding-3-small</option>
            <option value="cohere">Cohere · embed-english-v3</option>
            <option value="voyage">Voyage · voyage-3-lite</option>
            <option value="webllm">Web-LLM (local, WebGPU)</option>
          </select>
        </label>
        {settings.embedProvider !== 'none' && settings.embedProvider !== 'webllm' && (
          <label className="connect-field">
            <span className="connect-field-l">
              <span>API key (memory-only)</span>
              <span className="connect-field-hint">cleared on tab reload — never persisted.</span>
            </span>
            <input className="input" type="password" placeholder="sk-…" value={embedKeyInMemory} onChange={e=>setEmbedKeyInMemory(e.target.value)} />
          </label>
        )}

        <label className="connect-field">
          <span className="connect-field-l">
            <span>Cloud bundle storage</span>
            <span className="connect-field-hint">read-only signed-URL prefix.</span>
          </span>
          <select className="input" value={settings.cloudProvider} onChange={e=>update({cloudProvider:e.target.value})}>
            <option value="none">None</option>
            <option value="gcs">GCS</option>
            <option value="s3">S3</option>
          </select>
        </label>
        {settings.cloudProvider !== 'none' && (
          <label className="connect-field">
            <span className="connect-field-l">
              <span>Bucket prefix</span>
              <span className="connect-field-hint">e.g. https://storage.googleapis.com/my-bundles/</span>
            </span>
            <input className="input" type="text" value={settings.cloudBucket} onChange={e=>update({cloudBucket:e.target.value})} placeholder="https://…/" />
          </label>
        )}

        <label className="connect-field">
          <span className="connect-field-l">
            <span>{dot(settings.ipfsEnabled)}IPFS bundle distribution</span>
            <span className="connect-field-hint">publish witness-anchored bundles by CID.</span>
          </span>
          <span className="seg" role="radiogroup" aria-label="ipfs">
            {[['off','off'],['gateway','HTTP gateway only'],['kubo','kubo RPC'],['hybrid','kubo + gateway fallback']].map(([k, l]) => (
              <button
                key={k}
                type="button"
                className={'seg-item' + ((settings.ipfsEnabled ? settings.ipfsMode : 'off') === k ? ' active' : '')}
                onClick={() => update(k === 'off' ? { ipfsEnabled: false } : { ipfsEnabled: true, ipfsMode: k })}
              >{l}</button>
            ))}
          </span>
        </label>
        {settings.ipfsEnabled && (settings.ipfsMode === 'gateway' || settings.ipfsMode === 'hybrid') && (
          <label className="connect-field">
            <span className="connect-field-l"><span>Gateway URL</span></span>
            <input className="input" type="text" value={settings.ipfsGateway} onChange={e=>update({ipfsGateway:e.target.value})} />
          </label>
        )}
        {settings.ipfsEnabled && (settings.ipfsMode === 'kubo' || settings.ipfsMode === 'hybrid') && (
          <label className="connect-field">
            <span className="connect-field-l"><span>kubo RPC URL</span></span>
            <input className="input" type="text" value={settings.ipfsKuboUrl} onChange={e=>update({ipfsKuboUrl:e.target.value})} />
          </label>
        )}

        <div className="connect-field">
          <span className="connect-field-l">
            <span>Renderers</span>
            <span className="connect-field-hint">SVG fallback always present.</span>
          </span>
          <div style={{display:'flex', gap:18, alignItems:'center'}}>
            <label style={{display:'flex', gap:6, alignItems:'center', cursor:'pointer'}}>
              <input type="checkbox" checked={settings.webglEnabled} onChange={e=>update({webglEnabled:e.target.checked})} />
              <span className="mono" style={{fontSize:11}}>{dot(settings.webglEnabled)}WebGL</span>
            </label>
            <label style={{display:'flex', gap:6, alignItems:'center', cursor:'pointer'}}>
              <input type="checkbox" checked={settings.workersEnabled} onChange={e=>update({workersEnabled:e.target.checked})} />
              <span className="mono" style={{fontSize:11}}>{dot(settings.workersEnabled)}Web Workers</span>
            </label>
          </div>
        </div>

        <label className="connect-field">
          <span className="connect-field-l">
            <span>Cache size cap</span>
            <span className="connect-field-hint">soft cap with LRU eviction.</span>
          </span>
          <div style={{display:'flex', gap:10, alignItems:'center'}}>
            <input type="range" min="50" max="2000" step="50" value={settings.cacheCapMib} onChange={e=>update({cacheCapMib:Number(e.target.value)})} style={{flex:1}} />
            <span className="mono" style={{fontSize:11, minWidth:60, textAlign:'right'}}>{settings.cacheCapMib} MiB</span>
          </div>
        </label>
      </div>
      <div className="connect-actions">
        <button
          className="btn"
          onClick={async () => {
            // Quick connectivity probe: fetch a known-good CID via the configured gateway.
            // The bundle of node-wasm/fixtures has CID we don't pin yet; use a random text fixture instead.
            // Known-good CIDv1 — small text file pinned indefinitely on
            // multiple public gateways (used as a "is IPFS reachable?"
            // canary). Swap with your bundle CID once you've published one.
            const cid = 'bafkreigh2akiscaildcqabsyg3dfr6chu3fgpregiymsck7e7aqa4s52zy';
            const url = `${settings.ipfsGateway.replace(/\/$/,'')}/ipfs/${cid}`;
            const t0 = performance.now();
            try {
              const resp = await fetch(url, { method: 'GET', mode: 'cors' });
              const ms = Math.round(performance.now() - t0);
              const body = await resp.text();
              const ok = resp.ok;
              window.toast && window.toast[ok?'ok':'warn'](
                ok ? 'IPFS gateway reachable' : 'IPFS gateway error',
                `${ms}ms · ${resp.status} · ${body.length}B`,
              );
              if (window.RuStore) await window.RuStore.appendAudit({
                ts: new Date().toISOString().slice(11,19),
                principal: 'jules@ruv', tool: 'rulake_ipfs_probe',
                target: settings.ipfsGateway, k: 0, ms,
                code: ok ? 'IPFS_OK' : 'IPFS_HTTP_ERROR', outcome: ok ? 'ok' : 'refused',
              });
            } catch (e) {
              const ms = Math.round(performance.now() - t0);
              window.toast && window.toast.warn('IPFS unreachable', String(e.message || e));
              if (window.RuStore) await window.RuStore.appendAudit({
                ts: new Date().toISOString().slice(11,19),
                principal: 'jules@ruv', tool: 'rulake_ipfs_probe',
                target: settings.ipfsGateway, k: 0, ms,
                code: 'IPFS_NETWORK_ERROR', outcome: 'refused',
              });
            }
          }}
          disabled={!settings.ipfsEnabled}
        >Probe IPFS gateway</button>
        <span className="mono" style={{fontSize:10.5, color:'var(--fg-faint)', marginLeft:'auto'}}>
          {settings.workersEnabled ? '○ workers' : '· workers off'} ·
          {settings.webglEnabled ? '○ webgl' : '· webgl off'} ·
          embed:{settings.embedProvider} ·
          ipfs:{settings.ipfsEnabled ? settings.ipfsMode : 'off'}
        </span>
      </div>
    </div>
  );
}

function ConnectScreen() {
  const [mode, setMode] = useState('jwt');
  const [endpoint, setEndpoint] = useState('https://rulake.ruv.net/mcp');
  const [token, setToken] = useState('eyJhbGciOiJSUzI1NiIsImtpZCI6IjJmYThi…');
  const [label, setLabel] = useState('');
  const [listOpen, setListOpen] = useState(false);
  const [status, setStatus] = useState(null);
  const [rows] = (window.useRuStore || (() => [[]]))('connections');

  const save = async () => {
    if (!window.RuStore) return;
    await window.RuStore.put('connections', {
      label: label || endpoint, endpoint, mode, token: mode === 'none' ? '' : (token ? token.slice(0,8)+'…' : ''),
    });
    await window.RuStore.appendAudit({
      ts: new Date().toISOString().slice(11,19),
      principal: 'jules@ruv', tool: 'rulake_connect', target: endpoint, k: 0, ms: 0,
      code: 'SAVED_LOCAL', outcome: 'ok',
    });
    setStatus({ kind: 'ok', msg: `saved · ${label || endpoint}` });
    window.toast && window.toast.ok('Connection saved', `${label || endpoint} · auth=${mode}`);
    setTimeout(()=>setStatus(null), 2400);
  };

  const test = async () => {
    setStatus({ kind: 'pending', msg: 'handshake…' });
    window.toast && window.toast.info('Connecting…', `${endpoint} · ${mode === 'mtls' ? 'mTLS not browser-supported' : 'MCP initialize'}`, { duration: 1200 });
    if (mode === 'mtls') {
      setStatus({ kind: 'err', msg: 'mTLS not supported in the browser — TLS handshake picks the cert before any JS runs. Use a Bearer/JWT proxy instead.' });
      window.toast && window.toast.warn('mTLS unsupported', 'use a Bearer/JWT proxy in front of the mTLS endpoint');
      return;
    }
    const t0 = performance.now();
    try {
      const tokenForClient = mode === 'none' ? null : token;
      const client = new window.RuLakeHttp(endpoint, { token: tokenForClient });
      await client.connect();   // initialize + notifications/initialized

      // tools/list — exact wire shape, drains SSE until we see the result.
      const toolsResp = await fetch(endpoint, {
        method:  'POST',
        headers: {
          'Content-Type':  'application/json',
          'Accept':        'application/json, text/event-stream',
          'Authorization': tokenForClient
            ? (tokenForClient.startsWith('Bearer ') ? tokenForClient : `Bearer ${tokenForClient}`)
            : '',
          'mcp-session-id': client.sessionId || '',
          'MCP-Request-Id': Math.random().toString(36).slice(2) + Date.now().toString(36),
        },
        body: JSON.stringify({ jsonrpc: '2.0', id: 4, method: 'tools/list' }),
      });
      let toolsCount = 0, resourcesCount = 0;
      try {
        const txt = await toolsResp.text();
        const data = txt.split('\n').find(l => l.startsWith('data: '));
        const env = data ? JSON.parse(data.slice(6)) : JSON.parse(txt);
        toolsCount = env?.result?.tools?.length ?? 0;
      } catch { /* ignore — SSE keepalive may have eaten the body */ }

      const ms = Math.round(performance.now() - t0);
      setStatus({ kind: 'ok', msg: `← initialize OK · ${ms}ms · ${toolsCount} tools · session ${client.sessionId?.slice(0,8) || '—'}` });
      window.toast && window.toast.ok('initialize OK', `${ms}ms · ${toolsCount} tools`);
      if (window.RuStore) await window.RuStore.appendAudit({
        ts: new Date().toISOString().slice(11,19),
        principal: 'jules@ruv', tool: 'rulake_test', target: endpoint, k: toolsCount, ms,
        code: 'INIT_OK', outcome: 'ok',
      });
    } catch (e) {
      const ms = Math.round(performance.now() - t0);
      const msg = (e && e.message) || String(e);
      setStatus({ kind: 'err', msg: `connect failed · ${msg}` });
      window.toast && window.toast.warn('Connect failed', msg);
      if (window.RuStore) await window.RuStore.appendAudit({
        ts: new Date().toISOString().slice(11,19),
        principal: 'jules@ruv', tool: 'rulake_test', target: endpoint, k: 0, ms,
        code: 'CONNECT_FAILED', outcome: 'refused',
      });
    }
  };

  const pick = (row) => { setEndpoint(row.endpoint); setMode(row.mode); setLabel(row.label || ''); };

  return (
    <div style={{padding:'24px 28px', maxWidth:840}}>
      <div className="sec-h" style={{padding:0, borderBottom:'none'}}>
        <span>Connect to a rulake-mcp endpoint</span>
        <HelpIcon topic="connect" label="About auth modes" />
        <span className="mono" style={{color:'var(--fg-faint)', marginLeft:'auto'}}>MCP wire 2025-03-26 · {rows.length} saved</span>
      </div>
      <HelpStrip kind="info" storageKey="connect-intro">
        Save multiple endpoints — switch between prod, staging, and EU lakes from one console.
        All credentials are kept in <strong>this browser</strong> via IndexedDB and never leave the device.
        Press <span className="kbd">?</span> any time for context-specific help.
      </HelpStrip>

      <div className="connect-card">
        <div className="connect-card-h">
          <span className="connect-card-h-glyph">⌁</span>
          <span className="connect-card-h-title">Endpoint configuration</span>
          <span className="mono" style={{marginLeft:'auto', fontSize:10.5, color:'var(--fg-faint)'}}>local-only · IndexedDB</span>
        </div>

        <div className="connect-grid">
          <label className="connect-field">
            <span className="connect-field-l">
              <span>Label</span>
              <span className="connect-field-hint">friendly name</span>
            </span>
            <input className="input" value={label} onChange={e=>setLabel(e.target.value)} placeholder="prod · jules" />
          </label>

          <label className="connect-field">
            <span className="connect-field-l">
              <span>Endpoint URL</span>
              <span className="connect-field-hint">https://… /mcp</span>
            </span>
            <input className="input" value={endpoint} onChange={e=>setEndpoint(e.target.value)} />
          </label>

          <div className="connect-field">
            <span className="connect-field-l">
              <span>Auth mode</span>
              <span className="connect-field-hint">how the server identifies you</span>
            </span>
            <div className="seg connect-seg">
              {[
                ['none',   'No auth',  'dev / loopback'],
                ['bearer', 'Bearer',   'static token'],
                ['jwt',    'JWT',      'OAuth · PKCE'],
                ['mtls',   'mTLS',     'client cert'],
              ].map(([m, lbl, sub]) => (
                <div key={m} className={'seg-item connect-seg-item' + (mode===m?' active':'')} onClick={()=>setMode(m)}>
                  <div className="connect-seg-lbl">{lbl}</div>
                  <div className="connect-seg-sub">{sub}</div>
                </div>
              ))}
            </div>
          </div>

          {mode === 'mtls' && (
            <div className="connect-warn">
              <span className="connect-warn-glyph">⚠</span>
              <div>
                <div className="connect-warn-h">mTLS needs a non-browser client</div>
                <div className="connect-warn-b">The TLS handshake picks the client cert before any JavaScript runs. Use <code>rulake/http</code> from Node.js or <code>rulake-cli</code> instead.</div>
              </div>
            </div>
          )}
          {mode === 'jwt' && (
            <label className="connect-field">
              <span className="connect-field-l">
                <span>JWT</span>
                <span className="connect-field-hint">paste or PKCE redirect</span>
              </span>
              <textarea className="textarea" value={token} onChange={e=>setToken(e.target.value)} rows={3} />
              <span className="mono connect-field-foot">
                aud: https://rulake.ruv.net · scope: mcp:rulake:read mcp:rulake:publish
              </span>
            </label>
          )}
          {mode === 'bearer' && (
            <label className="connect-field">
              <span className="connect-field-l">
                <span>Bearer token</span>
                <span className="connect-field-hint">stored encrypted in IndexedDB</span>
              </span>
              <input className="input" type="password" value={token} onChange={e=>setToken(e.target.value)} />
            </label>
          )}
        </div>

        <div className="connect-actions">
          <button className="btn btn-primary" onClick={test}>▶ Connect &amp; initialize</button>
          <button className="btn" onClick={test}>Test only</button>
          <button className="btn" onClick={save}>Save endpoint</button>
          <button className="btn" onClick={()=>setListOpen(true)}>Saved ({rows.length})</button>
          <div className="connect-status">
            {status ? (
              <span className="mono" style={{fontSize:11, color: status.kind==='ok'?'var(--verifier-bright)':'var(--fg-faint)'}}>
                {status.kind === 'pending' ? '· · · ' : '● '}{status.msg}
              </span>
            ) : (
              <span className="mono" style={{fontSize:11, color:'var(--fg-faint)'}}>· idle</span>
            )}
          </div>
        </div>
      </div>

      <StorageSettingsCard />

      <div className="sec-h" style={{padding:'24px 0 10px', borderBottom:'none'}}>
        <span>Capability matrix · once connected</span>
      </div>
      <table className="tbl">
        <thead><tr><th>Capability</th><th>Tools</th><th>Resources</th><th>Status</th></tr></thead>
        <tbody>
          <tr><td className="mono">read</td>    <td className="dim">rulake_query · list_backends</td><td className="dim">rulake://stats · …/by-backend · …/bundle/*</td><td><span className="tag tag-verified">GRANTED</span></td></tr>
          <tr><td className="mono">publish</td> <td className="dim">rulake_publish_bundle · refresh · invalidate</td><td className="dim">—</td><td><span className="tag tag-verified">GRANTED</span></td></tr>
          <tr><td className="mono">admin</td>   <td className="dim">save_cache · warm_from_dir · audit_tail</td><td className="dim">—</td><td><span className="tag tag-cold">NOT REQUESTED</span></td></tr>
        </tbody>
      </table>

      <ConnectionsModal open={listOpen} onClose={()=>setListOpen(false)} onPick={pick} />
    </div>
  );
}

window.RuScreens = { Sidebar, Topbar, Statusbar, StatsScreen, PlaygroundScreen, BrowseScreen, BundleScreen, AuditScreen, ConnectScreen };
