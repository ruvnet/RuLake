import React, { useEffect, useState } from 'react';

function toolPayload(result) {
  const text = result?.content?.find(item => item.type === 'text')?.text;
  if (result?.isError) throw new Error(text || 'MCP tool failed');
  return result?.structuredContent ?? (text ? JSON.parse(text) : result);
}

async function inventory(client, signal) {
  const call = async (name, args) => toolPayload(await client.rpc('tools/call', { name, arguments: args }, signal));
  const { backends } = await call('rulake_list_backends', {});
  if (!Array.isArray(backends) || !backends.every(id => typeof id === 'string')) throw new Error('Invalid backend list');
  const rows = [];
  // Limit concurrent requests per session and retain the server's order.
  for (let i = 0; i < backends.length; i += 4) {
    signal.throwIfAborted();
    const batch = await Promise.all(backends.slice(i, i + 4).map(async id => {
      try {
        const { collections } = await call('rulake_list_collections', { backend: id });
        if (!Array.isArray(collections)) throw new Error('Invalid collection list');
        const ids = collections.map(c => typeof c === 'string' ? c : c?.id);
        if (!ids.every(name => typeof name === 'string')) throw new Error('Invalid collection ID');
        return { id, ids };
      } catch (error) {
        return { id, error: error.message };
      }
    }));
    rows.push(...batch);
  }
  return rows;
}

function BundleDetails({ client, uri }) {
  const [content, setContent] = useState('Reading bundle…');
  useEffect(() => {
    const controller = new AbortController();
    client.rpc('resources/read', { uri }, controller.signal).then(resource => {
      const text = resource.contents?.find(item => typeof item.text === 'string')?.text;
      if (!text) throw new Error('The server returned no bundle text.');
      if (!controller.signal.aborted) setContent(JSON.stringify(JSON.parse(text), null, 2));
    }).catch(error => { if (!controller.signal.aborted) setContent(error.message); });
    return () => controller.abort();
  }, [client, uri]);
  return <pre className="live-bundle" aria-live="polite">{content}</pre>;
}

export function LiveBackends({ client }) {
  const [revision, reload] = useState(0);
  const [state, setState] = useState({ loading: true });
  const [bundle, setBundle] = useState(null);
  useEffect(() => {
    const controller = new AbortController();
    const signal = controller.signal;
    const start = performance.now();
    setState({ loading: true });
    setBundle(null);
    const report = async () => {
      let rows = [], error;
      try { rows = await inventory(client, signal); }
      catch (e) { error = e.message; }
      if (signal.aborted) return;
      setState({ rows, error });
      const ok = !error && rows.every(row => !row.error);
      await window.RuStore?.appendAudit({
        ts: new Date().toISOString().slice(11,19), principal: 'console', tool: 'rulake_list_collections',
        target: client.url, k: rows.reduce((n, row) => n + (row.ids?.length || 0), 0),
        ms: Math.round(performance.now() - start), code: ok ? 'LIST_COLLECTIONS_OK' : 'LIST_COLLECTIONS_FAILED',
        outcome: ok ? 'ok' : 'refused',
      });
    };
    report().catch(error => console.warn('Could not record inventory audit', error));
    return () => controller.abort();
  }, [client, revision]);

  return <div className="live-inventory">
    <div className="pane-header"><span>Backends</span><span className="crumb-sep">/</span><span>Live inventory</span>
      <div className="header-actions">
        <a className="btn" href={import.meta.env.BASE_URL + 'lakes/'}>Import a lake</a>
        <button className="btn" disabled={state.loading} onClick={() => reload(n => n + 1)}>{state.loading ? 'Refreshing…' : 'Refresh'}</button>
      </div>
    </div>
    <div className="inventory-body" aria-live="polite">
      <p className="dim">{client.url}</p>
      {state.loading && <p>Reading registered backends…</p>}
      {state.error && <p role="alert">Could not read the inventory. {state.error}</p>}
      {state.rows?.length === 0 && !state.error && <p>No registered backends.</p>}
      {state.rows?.map(row => <section key={row.id}>
        <h2>{row.id}</h2>
        {row.error ? <p>Could not list collections. {row.error}</p> :
          row.ids.length === 0 ? <p>No registered collections.</p> : <ul>
            {row.ids.map(id => <li key={id}><span>{id}</span>
              <button className="btn" onClick={() => setBundle('rulake://bundle/' + row.id + '/' + id)}>Read bundle</button>
            </li>)}
          </ul>}
      </section>)}
      {bundle && <section><h2>{bundle}</h2><BundleDetails client={client} uri={bundle} key={bundle} /></section>}
    </div>
  </div>;
}
