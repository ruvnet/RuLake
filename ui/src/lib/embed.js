// Embedding provider — turns the Playground's query text into a vector.
//
// Providers wired here:
//   none      — deterministic mulberry32 fake (default; no network).
//   openai    — text-embedding-3-small via /v1/embeddings (1536-d).
//   cohere    — embed-english-v3 via /v1/embed (1024-d).
//   voyage    — voyage-3-lite via /v1/embeddings (512-d).
//   webllm    — local Web-LLM via WebGPU (not wired this iteration).
//
// API keys live in JS memory only. The provider/key choice is exposed
// at runtime via `window.RULakeEmbed.configure({...})` — populated by
// the Storage settings card on the Connect screen.

const PROVIDERS = {
  openai: {
    url: 'https://api.openai.com/v1/embeddings',
    body: (text, model) => ({ model: model || 'text-embedding-3-small', input: text }),
    pickVector: (j) => j?.data?.[0]?.embedding,
    auth: (key) => ({ Authorization: `Bearer ${key}` }),
    dim: 1536,
  },
  cohere: {
    url: 'https://api.cohere.ai/v1/embed',
    body: (text, model) => ({
      model: model || 'embed-english-v3.0',
      texts: [text],
      input_type: 'search_query',
    }),
    pickVector: (j) => j?.embeddings?.[0],
    auth: (key) => ({ Authorization: `Bearer ${key}` }),
    dim: 1024,
  },
  voyage: {
    url: 'https://api.voyageai.com/v1/embeddings',
    body: (text, model) => ({ model: model || 'voyage-3-lite', input: [text] }),
    pickVector: (j) => j?.data?.[0]?.embedding,
    auth: (key) => ({ Authorization: `Bearer ${key}` }),
    dim: 512,
  },
};

let config = { provider: 'none', apiKey: null, model: null };

function configure(patch) {
  config = { ...config, ...patch };
}

/**
 * Embed a text query and return a `{ vector: Float32Array, dim, ms,
 * provider, error }` result. Always resolves — errors are returned in
 * the result rather than thrown so the caller can record them in the
 * audit ledger.
 */
async function embed(text) {
  const t0 = performance.now();
  if (!text || typeof text !== 'string') {
    return { error: 'empty query' };
  }
  if (config.provider === 'none' || !config.apiKey) {
    // Fallback: deterministic random vector seeded from the query.
    let seed = 0x9e3779b9;
    for (let i = 0; i < text.length; i++) {
      seed = (seed * 31 + text.charCodeAt(i)) >>> 0;
    }
    const dim = 128;
    const vec = new Float32Array(dim);
    let s = seed || 1;
    for (let i = 0; i < dim; i++) {
      s = (s * 1664525 + 1013904223) >>> 0;
      vec[i] = ((s & 0xffff) / 0x8000) - 1;
    }
    const ms = Math.round(performance.now() - t0);
    return { vector: vec, dim, ms, provider: 'fixture' };
  }

  const def = PROVIDERS[config.provider];
  if (!def) {
    return { error: `unknown provider: ${config.provider}` };
  }
  try {
    const resp = await fetch(def.url, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', ...def.auth(config.apiKey) },
      body: JSON.stringify(def.body(text, config.model)),
    });
    const ms = Math.round(performance.now() - t0);
    if (!resp.ok) {
      let detail = '';
      try { detail = (await resp.text()).slice(0, 200); } catch {}
      return { error: `${resp.status} ${resp.statusText}${detail ? ` · ${detail}` : ''}`, ms, provider: config.provider };
    }
    const j = await resp.json();
    const arr = def.pickVector(j);
    if (!Array.isArray(arr)) {
      return { error: `unexpected response shape from ${config.provider}`, ms, provider: config.provider };
    }
    return {
      vector: Float32Array.from(arr),
      dim: arr.length,
      ms,
      provider: config.provider,
    };
  } catch (e) {
    const ms = Math.round(performance.now() - t0);
    return { error: String(e && e.message || e), ms, provider: config.provider };
  }
}

window.RULakeEmbed = { configure, embed, get config() { return { ...config, apiKey: config.apiKey ? '***' : null }; } };
export { configure, embed };
