// Web Worker — exact L2 nearest-K, off the main thread.
//
// Mirrors `rulake-wasm::searchBruteForceL2` semantics (recall = 1.0,
// k capped at min(n, 4096)) but runs in pure JS so the worker boots
// without needing wasm in worker-scope (Vite + wasm-bindgen require
// extra plumbing for that — a v0.2 polish). The win we ship today is
// "the search runs off the main thread" — which is what the Workers
// toggle in the Storage settings card is advertising.
//
// Protocol:
//   inbound: { id, type: 'search', vectors: Float32Array, ids: Float64Array,
//              dim, query: Float32Array, k }
//   outbound: { id, type: 'result', hits: [{idx, id, score}], ms }
//   on error: { id, type: 'error', message }

self.addEventListener('message', (e) => {
  const { id, type, vectors, ids, dim, query, k } = e.data || {};
  if (type !== 'search') return;
  const t0 = performance.now();
  try {
    if (!Number.isInteger(dim) || dim <= 0) throw new Error('dim must be a positive integer');
    if (!(vectors instanceof Float32Array)) throw new Error('vectors must be Float32Array');
    if (!(query instanceof Float32Array)) throw new Error('query must be Float32Array');
    if (vectors.length % dim !== 0) throw new Error(`vectors.length ${vectors.length} not divisible by dim ${dim}`);
    if (query.length !== dim) throw new Error(`query.length ${query.length} != dim ${dim}`);
    const n = (vectors.length / dim) | 0;
    const haveIds = ids instanceof Float64Array && ids.length === n;
    const cap = Math.min(k | 0, n, 4096);

    const scored = new Array(n);
    for (let i = 0; i < n; i++) {
      const off = i * dim;
      let s = 0;
      for (let d = 0; d < dim; d++) {
        const diff = vectors[off + d] - query[d];
        s += diff * diff;
      }
      scored[i] = { idx: i, score: s };
    }
    scored.sort((a, b) => a.score - b.score);
    const top = scored.slice(0, cap).map((row) => ({
      idx: row.idx,
      id: haveIds ? ids[row.idx] : row.idx,
      score: row.score,
    }));
    const ms = Math.round(performance.now() - t0);
    self.postMessage({ id, type: 'result', hits: top, ms });
  } catch (err) {
    self.postMessage({ id, type: 'error', message: String(err && err.message || err) });
  }
});
