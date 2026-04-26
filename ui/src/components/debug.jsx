/* global React */
const { useState, useEffect, useRef } = React;

// In-app debug console — terminal-styled, captures logs and shows MCP wire activity
function DebugConsole({ visible, onClose }) {
  const [logs, setLogs] = useState([]);
  const [filter, setFilter] = useState('all');
  const [collapsed, setCollapsed] = useState(false);
  const listRef = useRef(null);

  useEffect(() => {
    // Patch console once
    if (window.__rulakeConsolePatched) return;
    window.__rulakeConsolePatched = true;
    const orig = { log: console.log, warn: console.warn, error: console.error, info: console.info };
    const push = (level, args) => {
      const msg = args.map(a => {
        if (a instanceof Error) return a.stack || a.message;
        if (typeof a === 'object') { try { return JSON.stringify(a); } catch { return String(a); } }
        return String(a);
      }).join(' ');
      window.__rulakeLogs = window.__rulakeLogs || [];
      window.__rulakeLogs.push({ ts: new Date(), level, msg });
      if (window.__rulakeLogs.length > 400) window.__rulakeLogs.shift();
      window.dispatchEvent(new CustomEvent('rulake:log'));
    };
    ['log','warn','error','info'].forEach(l => {
      console[l] = (...a) => { push(l, a); orig[l](...a); };
    });
    window.addEventListener('error', e => push('error', [e.message + ' @ ' + (e.filename||'') + ':' + (e.lineno||'')]));
    window.addEventListener('unhandledrejection', e => push('error', ['Unhandled: ' + (e.reason?.message || e.reason)]));

    // Seed
    push('info', ['ruLake console v0.3.1 attached']);
    push('info', ['MCP 2025-03-26 · session 7f3c2…91d · principal jules@ruv']);
    push('info', ['→ initialize { protocolVersion: "2025-03-26" }']);
    push('info', ['← capabilities { tools, resources, logging }']);
    push('info', ['→ tools/list']);
    push('info', ['← 9 tools (rulake_query, list_backends, publish_bundle, ...)']);
    push('info', ['→ resources/list']);
    push('info', ['← 5 resources (rulake://stats, .../by-backend, .../bundle/*)']);
    push('info', ['rulake-wasm initialised · 149 KB · verifyBundleJson ready']);

    // Periodic synthetic wire chatter
    const wires = [
      ['log','→ tools/call rulake_query { target: "lake-prod/memories", k: 10 }'],
      ['log','← 200 OK · 11.2ms · witness=dea58c64…07d7e4 · trust=verified'],
      ['log','→ resources/read rulake://stats'],
      ['log','← {hits: 124330, misses: 1482, hit_rate: 0.988}'],
      ['warn','planner: lake-eu/memories STALE_BUNDLE_GUARD (gen behind by 1)'],
      ['log','wasm: verifyBundleJson(memories) → MATCH (4.2ms)'],
      ['log','→ tools/call rulake_query { target: "lake-prod/docs.public", k: 5 }'],
      ['log','← 200 OK · 6.1ms · witness=8a3f2210…91d004'],
    ];
    let i = 0;
    const id = setInterval(() => {
      const [lvl, m] = wires[i++ % wires.length];
      push(lvl, [m]);
    }, 1800);
    return () => clearInterval(id);
  }, []);

  useEffect(() => {
    const sync = () => setLogs([...(window.__rulakeLogs || [])]);
    sync();
    window.addEventListener('rulake:log', sync);
    return () => window.removeEventListener('rulake:log', sync);
  }, []);

  useEffect(() => {
    if (listRef.current) listRef.current.scrollTop = listRef.current.scrollHeight;
  }, [logs]);

  if (!visible) return null;
  const filtered = filter === 'all' ? logs : logs.filter(l => l.level === filter);
  const counts = logs.reduce((a, l) => { a[l.level] = (a[l.level] || 0) + 1; return a; }, {});

  return (
    <div className={'debug-console' + (collapsed ? ' collapsed' : '')}>
      <div className="debug-head">
        <span className="debug-title">▸ console</span>
        <span className="debug-count" style={{color:'#2d8c5d'}}>log {counts.log || 0}</span>
        <span className="debug-count" style={{color:'#4a9eb5'}}>info {counts.info || 0}</span>
        <span className="debug-count" style={{color:'#e89548'}}>warn {counts.warn || 0}</span>
        <span className="debug-count" style={{color:'#e89548'}}>err {counts.error || 0}</span>
        <div className="debug-filter">
          {['all','log','info','warn','error'].map(f => (
            <span key={f} className={'debug-filter-item' + (filter===f?' active':'')} onClick={()=>setFilter(f)}>{f}</span>
          ))}
        </div>
        <button className="debug-btn" onClick={() => { window.__rulakeLogs = []; setLogs([]); }}>clear</button>
        <button className="debug-btn" onClick={() => setCollapsed(c=>!c)}>{collapsed ? '▲' : '▼'}</button>
        <button className="debug-btn" onClick={onClose}>×</button>
      </div>
      {!collapsed && (
        <div className="debug-body" ref={listRef}>
          {filtered.map((l, i) => (
            <div key={i} className={'debug-line debug-' + l.level}>
              <span className="debug-ts">{l.ts.toISOString().slice(11,23)}</span>
              <span className="debug-lvl">{l.level.toUpperCase()}</span>
              <span className="debug-msg">{l.msg}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

window.RuDebug = { DebugConsole };
