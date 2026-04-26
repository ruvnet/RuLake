// rulake/http — HTTP client variant (v2 surface, ADR-003 §"What's not yet here").
//
// Lightweight fetch-based client for talking to a remote `rulake-mcp`
// server (the Streamable HTTP transport added in mcp-server v0.2). Lets
// edge runtimes that can't load the native binary — Cloudflare Workers,
// Deno-deploy, browser ServiceWorkers — still consume a remote ruLake
// memory layer through one MCP tool.
//
// Pairs with `rulake-wasm` for the local witness-verification side: any
// hit returned by the remote can be re-verified locally against its
// bundle witness with `rulake-wasm`'s `verifyBundleJson`.

const ACCEPT  = "application/json, text/event-stream";
const VERSION = "2025-03-26"; // MCP protocol version

class RuLakeHttpError extends Error {
  constructor(message, { status, code } = {}) {
    super(message);
    this.name   = "RuLakeHttpError";
    this.status = status;
    this.code   = code;
  }
}

function uuidLike(seed) {
  // Cheap deterministic-ish nonce per request — the server's replay
  // guard rejects duplicates.
  const r = (seed * 0x9e3779b9) >>> 0;
  const t = (Date.now() & 0xffffffff) >>> 0;
  return [r, t, Math.floor(Math.random() * 0xffffffff) >>> 0]
    .map((n) => n.toString(16).padStart(8, "0"))
    .join("");
}

async function postJson(url, body, headers = {}) {
  const resp = await fetch(url, {
    method:  "POST",
    headers: { "Content-Type": "application/json", Accept: ACCEPT, ...headers },
    body:    JSON.stringify(body),
  });
  if (!resp.ok) {
    let code;
    try {
      const j = await resp.json();
      code = j?.error?.code;
    } catch {
      // ignore
    }
    throw new RuLakeHttpError(`HTTP ${resp.status} ${resp.statusText}`, {
      status: resp.status,
      code,
    });
  }
  return resp;
}

/**
 * Read SSE body until we get a JSON-RPC envelope with `result`. Returns
 * `null` if the stream closes without one.
 */
async function readSseUntilResult(resp) {
  if (!resp.body) {
    const text = await resp.text();
    try { return JSON.parse(text); } catch { return null; }
  }
  const reader  = resp.body.getReader();
  const decoder = new TextDecoder();
  let buf = "";
  try {
    while (true) {
      const { value, done } = await reader.read();
      if (done) return null;
      buf += decoder.decode(value, { stream: true });
      let idx;
      while ((idx = buf.indexOf("\n")) !== -1) {
        const line = buf.slice(0, idx);
        buf = buf.slice(idx + 1);
        if (!line.startsWith("data: ")) continue;
        const data = line.slice(6).trim();
        if (!data) continue;
        try {
          const env = JSON.parse(data);
          if (env.result !== undefined || env.error !== undefined) return env;
        } catch {
          // keep reading — partial frame
        }
      }
    }
  } finally {
    try { reader.cancel(); } catch { /* ignore */ }
  }
}

/**
 * Thin client for `rulake-mcp` over Streamable HTTP.
 *
 * Usage:
 *   const c = new RuLakeHttp("https://rulake.example/mcp", { token: "Bearer …" });
 *   await c.connect();
 *   const res = await c.query({ intent: "search", backend: "docs", collection: "memories", query: [...], k: 10 });
 *   // res.hits, res.witness, res.decision_trace, ...
 *
 * The server-side scope→capability mapping (mcp-server v0.8) gates which
 * intents this token may invoke.
 */
class RuLakeHttp {
  constructor(url, opts = {}) {
    this.url           = url;
    this.token         = opts.token  ?? null;        // "Bearer X" or "X" (we add Bearer)
    this.headers       = opts.headers ?? {};
    this.sessionId     = null;
    this.requestSeed   = 1;
  }

  _headers() {
    const h = { ...this.headers };
    if (this.token) {
      h.Authorization = this.token.startsWith("Bearer ") ? this.token : `Bearer ${this.token}`;
    }
    if (this.sessionId) h["mcp-session-id"] = this.sessionId;
    return h;
  }

  async connect() {
    const initId = uuidLike(this.requestSeed++);
    const resp = await postJson(
      this.url,
      {
        jsonrpc: "2.0",
        id:      1,
        method:  "initialize",
        params:  {
          protocolVersion: VERSION,
          capabilities:    {},
          clientInfo:      { name: "rulake/http", version: "2.2.0" },
        },
      },
      { ...this._headers(), "MCP-Request-Id": initId },
    );
    this.sessionId = resp.headers.get("mcp-session-id") || this.sessionId;

    // Drain initialize response (don't actually need its body — the
    // session id from headers is the load-bearing artifact).
    try { await resp.body?.cancel?.(); } catch { /* ignore */ }

    // Fire `notifications/initialized` (no response expected).
    try {
      const r2 = await fetch(this.url, {
        method:  "POST",
        headers: { ...this._headers(), "Content-Type": "application/json", Accept: ACCEPT, "MCP-Request-Id": uuidLike(this.requestSeed++) },
        body:    JSON.stringify({ jsonrpc: "2.0", method: "notifications/initialized" }),
      });
      try { await r2.body?.cancel?.(); } catch { /* ignore */ }
    } catch {
      // notifications are best-effort
    }
  }

  /** Call the `rulake_query` MCP tool. Returns the parsed result body. */
  async query(args) {
    if (!this.sessionId) await this.connect();
    const reqId = uuidLike(this.requestSeed++);
    const resp = await postJson(
      this.url,
      {
        jsonrpc: "2.0",
        id:      Date.now() & 0xffff,
        method:  "tools/call",
        params:  { name: "rulake_query", arguments: args },
      },
      { ...this._headers(), "MCP-Request-Id": reqId },
    );
    const env = await readSseUntilResult(resp);
    if (!env) throw new RuLakeHttpError("server closed stream without a result");
    if (env.error) {
      throw new RuLakeHttpError(env.error.message ?? "error", {
        status: 200,
        code:   env.error.code,
      });
    }
    return env.result;
  }
}

export { RuLakeHttp, RuLakeHttpError };
export default RuLakeHttp;
