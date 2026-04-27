/* global React */
const { useEffect: useEff, useState: useSt } = React;

// ─── Modal shell ──────────────────────────────────────────────────────────
function Modal({ open, onClose, title, subtitle, children, width = 520, footer }) {
  useEff(() => {
    if (!open) return;
    const onKey = (e) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [open, onClose]);
  if (!open) return null;
  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal" style={{ width }} onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <div>
            <div className="modal-title">{title}</div>
            {subtitle && <div className="modal-sub">{subtitle}</div>}
          </div>
          <button className="modal-x" onClick={onClose}>×</button>
        </div>
        <div className="modal-body">{children}</div>
        {footer && <div className="modal-foot">{footer}</div>}
      </div>
    </div>
  );
}

// ─── Save Query modal ─────────────────────────────────────────────────────
function SaveQueryModal({ open, onClose, payload }) {
  const [label, setLabel] = useSt('');
  useEff(() => { if (open) setLabel(`query · ${new Date().toISOString().slice(11,19)}`); }, [open]);
  const save = async () => {
    await window.RuStore.put('queries', { label, ...payload });
    await window.RuStore.appendAudit({
      ts: new Date().toISOString().slice(11,19),
      principal: 'jules@ruv', tool: 'rulake_query', target: payload.target,
      k: payload.k, ms: 0, code: 'SAVED_LOCAL', outcome: 'ok',
    });
    window.toast && window.toast.ok('Query saved', `'${label}' · stored locally`);
    onClose();
  };
  return (
    <Modal open={open} onClose={onClose} title="Save query" subtitle="Persists to IndexedDB · stays on this device"
      footer={<><button className="btn" onClick={onClose}>Cancel</button><button className="btn btn-primary" onClick={save} disabled={!label}>Save</button></>}>
      <div className="field-row" style={{borderBottom:'none', padding:0}}>
        <span className="field-label">Label</span>
        <input className="input" value={label} onChange={e=>setLabel(e.target.value)} autoFocus />
      </div>
      <div style={{marginTop:12, fontSize:11.5, fontFamily:'JetBrains Mono', color:'var(--fg-faint)'}}>
        target · <span style={{color:'var(--accent-cyan)'}}>{payload.target}</span><br/>
        k={payload.k} · risk={payload.risk}<br/>
        query · <span style={{color:'var(--fg-dim)'}}>{(payload.query || '').slice(0,80)}{(payload.query||'').length>80?'…':''}</span>
      </div>
    </Modal>
  );
}

// ─── Load Query modal ─────────────────────────────────────────────────────
function LoadQueryModal({ open, onClose, onPick }) {
  const [rows, ops] = (window.useRuStore || (() => [[], { remove: () => {} }]))('queries');
  return (
    <Modal open={open} onClose={onClose} title="Saved queries" subtitle={`${rows.length} on this device`} width={620}>
      {rows.length === 0 && (
        <div style={{padding:'40px 0', textAlign:'center', color:'var(--fg-faint)', fontFamily:'JetBrains Mono', fontSize:11.5}}>
          ∅ no saved queries yet — Save a query from the playground
        </div>
      )}
      {rows.map(r => (
        <div key={r.id} className="store-row" onClick={() => { onPick(r); onClose(); }}>
          <div style={{flex:1, minWidth:0}}>
            <div className="store-row-label">{r.label}</div>
            <div className="store-row-meta">
              <span style={{color:'var(--accent-cyan)'}}>{r.target}</span> · k={r.k} · risk={r.risk}
              <span style={{marginLeft:10, color:'var(--fg-faint)'}}>{new Date(r.ts).toLocaleString()}</span>
            </div>
          </div>
          <button className="btn" onClick={(e)=>{e.stopPropagation(); ops.remove(r.id); window.toast && window.toast.info('Query deleted', r.label, { duration: 1800 });}}>Delete</button>
        </div>
      ))}
    </Modal>
  );
}

// ─── Pin Bundle modal ─────────────────────────────────────────────────────
function PinBundleModal({ open, onClose, payload }) {
  const [note, setNote] = useSt('');
  useEff(() => { if (open) setNote(''); }, [open]);
  const save = async () => {
    await window.RuStore.put('bundles', { ...payload, note });
    await window.RuStore.appendAudit({
      ts: new Date().toISOString().slice(11,19),
      principal: 'jules@ruv', tool: 'rulake_pin', target: `${payload.backend}/${payload.collection}`,
      k: 0, ms: 0, code: 'PINNED_LOCAL', outcome: 'ok',
    });
    window.toast && window.toast.ok('Witness pinned', `${payload.backend}/${payload.collection} @ ${payload.generation}`);
    onClose();
  };
  return (
    <Modal open={open} onClose={onClose} title="Pin bundle witness" subtitle="Snapshot the current witness for later diff"
      footer={<><button className="btn" onClick={onClose}>Cancel</button><button className="btn btn-primary" onClick={save}>Pin</button></>}>
      <div style={{fontSize:11.5, fontFamily:'JetBrains Mono', marginBottom:12}}>
        <div><span style={{color:'var(--fg-faint)'}}>backend  </span><span>{payload.backend}</span></div>
        <div><span style={{color:'var(--fg-faint)'}}>coll     </span><span>{payload.collection}</span></div>
        <div><span style={{color:'var(--fg-faint)'}}>gen      </span><span style={{color:'var(--accent-cyan)'}}>{payload.generation}</span></div>
        <div style={{marginTop:8, wordBreak:'break-all', color:'var(--verifier-bright)'}}>{payload.witness}</div>
      </div>
      <div className="field-row" style={{borderBottom:'none', padding:0}}>
        <span className="field-label">Note (optional)</span>
        <input className="input" value={note} onChange={e=>setNote(e.target.value)} placeholder="e.g. before Q4 reindex"/>
      </div>
    </Modal>
  );
}

// ─── Pinned Bundles list ─────────────────────────────────────────────────
function BundlesListModal({ open, onClose }) {
  const [rows, ops] = (window.useRuStore || (() => [[], { remove: () => {} }]))('bundles');
  return (
    <Modal open={open} onClose={onClose} title="Pinned bundles" subtitle={`${rows.length} witness pins`} width={680}>
      {rows.length === 0 && (
        <div style={{padding:'40px 0', textAlign:'center', color:'var(--fg-faint)', fontFamily:'JetBrains Mono', fontSize:11.5}}>
          ∅ no pinned bundles
        </div>
      )}
      {rows.map(r => (
        <div key={r.id} className="store-row">
          <div style={{flex:1, minWidth:0}}>
            <div className="store-row-label">{r.backend}/{r.collection} <span style={{color:'var(--accent-cyan)', marginLeft:6}}>{r.generation}</span></div>
            <div className="store-row-meta" style={{wordBreak:'break-all', color:'var(--verifier-bright)'}}>{r.witness}</div>
            {r.note && <div className="store-row-meta" style={{color:'var(--fg-dim)'}}>“{r.note}”</div>}
            <div className="store-row-meta">{new Date(r.ts).toLocaleString()}</div>
          </div>
          <button className="btn" onClick={()=>{ops.remove(r.id); window.toast && window.toast.info('Bundle unpinned', `${r.backend}/${r.collection} · ${r.generation}`, { duration: 1800 });}}>Unpin</button>
        </div>
      ))}
    </Modal>
  );
}

// ─── Saved Connections list ──────────────────────────────────────────────
function ConnectionsModal({ open, onClose, onPick }) {
  const [rows, ops] = (window.useRuStore || (() => [[], { remove: () => {} }]))('connections');
  return (
    <Modal open={open} onClose={onClose} title="Saved connections" subtitle={`${rows.length} endpoints · stored locally`} width={620}>
      {rows.length === 0 && (
        <div style={{padding:'40px 0', textAlign:'center', color:'var(--fg-faint)', fontFamily:'JetBrains Mono', fontSize:11.5}}>
          ∅ no saved endpoints
        </div>
      )}
      {rows.map(r => (
        <div key={r.id} className="store-row" onClick={()=>{ onPick && onPick(r); onClose(); }}>
          <div style={{flex:1, minWidth:0}}>
            <div className="store-row-label">{r.label || r.endpoint}</div>
            <div className="store-row-meta">
              <span style={{color:'var(--accent-cyan)'}}>{r.endpoint}</span> · auth={r.mode}
              <span style={{marginLeft:10, color:'var(--fg-faint)'}}>{new Date(r.ts).toLocaleString()}</span>
            </div>
          </div>
          <button className="btn" onClick={(e)=>{e.stopPropagation(); ops.remove(r.id); window.toast && window.toast.info('Connection deleted', r.label || r.endpoint, { duration: 1800 });}}>Delete</button>
        </div>
      ))}
    </Modal>
  );
}

window.RuModals = { Modal, SaveQueryModal, LoadQueryModal, PinBundleModal, BundlesListModal, ConnectionsModal };
