// Lazy-loader for the rulake-wasm package (~149 KB compressed).
// Exposed on window.RULakeWasm so the JSX files (which use globals) can
// call it. Loaded on demand — first call triggers the dynamic import.
//
// The wasm-bindgen module exports:
//   verifyBundleJson(json) → {ok, computed, stored, fields}
//   computeWitness(data_ref, dim, rotation_seed, rerank_factor, generation)
//   searchBruteForceL2(vectors, ids, dim, query, k) → [{idx, id, score}]
//   formatVersion() → 2
//   buildInfo() → "rulake-wasm v2.2.0 (witness-format v2, sha3 = 0.10)"

let modulePromise = null;

function load() {
  if (!modulePromise) {
    modulePromise = import('rulake-wasm').then(async (mod) => {
      // The 'bundler' target export needs no init() (Vite handles it).
      // Some targets export init as default — try it if present, ignore failures.
      try {
        if (typeof mod.default === 'function') {
          await mod.default();
        }
      } catch {
        /* already initialized */
      }
      return mod;
    });
  }
  return modulePromise;
}

const RULakeWasm = {
  /** Recompute and verify a bundle's witness. Returns {ok, computed, stored, fields}. */
  async verifyBundle(jsonOrObj) {
    const m = await load();
    const json = typeof jsonOrObj === 'string' ? jsonOrObj : JSON.stringify(jsonOrObj);
    return m.verifyBundleJson(json);
  },

  /** Compute the SHAKE-256(32) witness directly from fields. */
  async computeWitness(dataRef, dim, rotationSeed, rerankFactor, generation) {
    const m = await load();
    // Generation may be {Num: n} | {Opaque: s} — pass as-is.
    return m.computeWitness(dataRef, dim, BigInt(rotationSeed), rerankFactor, generation);
  },

  /** Brute-force exact-L2 nearest-K. */
  async searchL2(vectors, ids, dim, query, k) {
    const m = await load();
    const v = vectors instanceof Float32Array ? vectors : Float32Array.from(vectors);
    const q = query instanceof Float32Array ? query : Float32Array.from(query);
    const i = ids instanceof Float64Array ? ids : Float64Array.from(ids ?? []);
    return m.searchBruteForceL2(v, i, dim, q, k);
  },

  /** Build / version diagnostic. */
  async info() {
    const m = await load();
    return { version: m.formatVersion(), build: m.buildInfo() };
  },

  /** True once the wasm module has finished loading. */
  loaded() {
    return modulePromise !== null;
  },
};

window.RULakeWasm = RULakeWasm;
export default RULakeWasm;
