/* IndexedDB store for ruLake console — saved queries, bundle pins, connections, audit log */
(function () {
  const DB_NAME = 'rulake-console';
  const DB_VERSION = 1;
  const STORES = ['queries', 'bundles', 'connections', 'audit', 'kv'];

  let dbPromise = null;
  function openDB() {
    if (dbPromise) return dbPromise;
    dbPromise = new Promise((resolve, reject) => {
      const req = indexedDB.open(DB_NAME, DB_VERSION);
      req.onupgradeneeded = (e) => {
        const db = e.target.result;
        STORES.forEach((s) => {
          if (!db.objectStoreNames.contains(s)) {
            const store = db.createObjectStore(s, { keyPath: 'id', autoIncrement: true });
            if (s === 'audit') store.createIndex('ts', 'ts');
          }
        });
      };
      req.onsuccess = () => resolve(req.result);
      req.onerror = () => reject(req.error);
    });
    return dbPromise;
  }

  function tx(store, mode = 'readonly') {
    return openDB().then((db) => db.transaction(store, mode).objectStore(store));
  }

  const wrap = (req) => new Promise((res, rej) => { req.onsuccess = () => res(req.result); req.onerror = () => rej(req.error); });

  const RuStore = {
    async list(store) {
      const s = await tx(store);
      return wrap(s.getAll()).then((rows) => rows.sort((a, b) => (b.ts || 0) - (a.ts || 0)));
    },
    async put(store, value) {
      const s = await tx(store, 'readwrite');
      const row = { ...value, ts: value.ts || Date.now() };
      const id = await wrap(s.put(row));
      window.dispatchEvent(new CustomEvent('rulake:store', { detail: { store } }));
      console.log(`[idb] put → ${store}#${id} (${row.label || row.name || row.endpoint || row.target || ''})`);
      return id;
    },
    async remove(store, id) {
      const s = await tx(store, 'readwrite');
      await wrap(s.delete(id));
      window.dispatchEvent(new CustomEvent('rulake:store', { detail: { store } }));
      console.log(`[idb] del → ${store}#${id}`);
    },
    async clear(store) {
      const s = await tx(store, 'readwrite');
      await wrap(s.clear());
      window.dispatchEvent(new CustomEvent('rulake:store', { detail: { store } }));
      console.log(`[idb] clear → ${store}`);
    },
    async getKV(key) {
      const s = await tx('kv');
      const row = await wrap(s.get(key));
      return row ? row.value : null;
    },
    async setKV(key, value) {
      const s = await tx('kv', 'readwrite');
      await wrap(s.put({ id: key, value, ts: Date.now() }));
      console.log(`[idb] kv ← ${key}`);
    },
    async appendAudit(entry) {
      return RuStore.put('audit', entry);
    },
  };

  // React hook (defined when React is ready)
  function defineHook() {
    if (!window.React) { setTimeout(defineHook, 30); return; }
    const { useState, useEffect, useCallback } = window.React;
    window.useRuStore = function (storeName) {
      const [rows, setRows] = useState([]);
      const reload = useCallback(() => RuStore.list(storeName).then(setRows), [storeName]);
      useEffect(() => {
        reload();
        const onChange = (e) => { if (e.detail.store === storeName) reload(); };
        window.addEventListener('rulake:store', onChange);
        return () => window.removeEventListener('rulake:store', onChange);
      }, [storeName, reload]);
      return [rows, {
        put: (v) => RuStore.put(storeName, v),
        remove: (id) => RuStore.remove(storeName, id),
        clear: () => RuStore.clear(storeName),
        reload,
      }];
    };
  }
  defineHook();

  window.RuStore = RuStore;
})();
