// Search dispatcher — picks worker or main-thread path based on the
// user's Workers toggle in the Storage settings card.
//
// The worker is created lazily on first use; subsequent calls reuse it.
// Each call gets a fresh request-id so concurrent requests don't
// collide.

import SearchWorker from '../workers/search.worker.js?worker';

let workerInstance = null;
let nextId = 1;
const pending = new Map();

function getWorker() {
  if (!workerInstance) {
    workerInstance = new SearchWorker();
    workerInstance.addEventListener('message', (e) => {
      const { id, type, hits, message, ms } = e.data || {};
      const pendingHandler = pending.get(id);
      if (!pendingHandler) return;
      pending.delete(id);
      if (type === 'result') pendingHandler.resolve({ hits, ms });
      else pendingHandler.reject(new Error(message || 'worker error'));
    });
  }
  return workerInstance;
}

/**
 * Run an exact-L2 nearest-K search.
 *
 * If `useWorker` is truthy AND the runtime supports `Worker`, the search
 * happens off the main thread (so a 60-fps render keeps running). Vectors
 * are transferred zero-copy via `transferable`s (the caller's buffers
 * are detached on send — pass copies if you still need them on the
 * main thread).
 *
 * Returns `{ hits, ms, transport }` where `transport` is `'worker'` or
 * `'main'` so callers can surface which path ran.
 */
export async function searchOffload({ vectors, ids, dim, query, k, useWorker }) {
  if (useWorker && typeof Worker !== 'undefined') {
    const t0 = performance.now();
    const w = getWorker();
    const id = nextId++;
    const promise = new Promise((resolve, reject) => {
      pending.set(id, { resolve, reject });
    });
    // Don't transfer caller buffers — defensive copy keeps the demo's
    // Playground re-runs cheap. At our problem sizes this is sub-ms.
    const vCopy = new Float32Array(vectors);
    const qCopy = new Float32Array(query);
    const iCopy = ids ? new Float64Array(ids) : undefined;
    w.postMessage(
      { id, type: 'search', vectors: vCopy, ids: iCopy, dim, query: qCopy, k },
      [vCopy.buffer, qCopy.buffer, ...(iCopy ? [iCopy.buffer] : [])],
    );
    const result = await promise;
    return {
      hits: result.hits,
      ms: result.ms,
      roundTripMs: Math.round(performance.now() - t0),
      transport: 'worker',
    };
  }

  // Main-thread path — fall back to the wasm shim if it's loaded,
  // otherwise pure-JS.
  const t0 = performance.now();
  let hits = [];
  if (window.RULakeWasm && window.RULakeWasm.searchL2) {
    hits = await window.RULakeWasm.searchL2(vectors, ids, dim, query, k);
  } else {
    // pure-JS fallback (same as worker code)
    const n = (vectors.length / dim) | 0;
    const cap = Math.min(k | 0, n, 4096);
    const scored = new Array(n);
    for (let i = 0; i < n; i++) {
      let s = 0;
      const off = i * dim;
      for (let d = 0; d < dim; d++) {
        const diff = vectors[off + d] - query[d];
        s += diff * diff;
      }
      scored[i] = { idx: i, score: s };
    }
    scored.sort((a, b) => a.score - b.score);
    hits = scored.slice(0, cap).map((r) => ({
      idx: r.idx,
      id: ids ? ids[r.idx] : r.idx,
      score: r.score,
    }));
  }
  const ms = Math.round(performance.now() - t0);
  return { hits, ms, roundTripMs: ms, transport: 'main' };
}

window.RULakeSearch = { searchOffload };
