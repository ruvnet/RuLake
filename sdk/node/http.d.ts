// rulake/http — HTTP client variant (v2 surface, ADR-003).
// Lightweight fetch-based client for talking to a remote `rulake-mcp`
// server. Works in Cloudflare Workers, Deno-deploy, browsers, Bun, and
// Node.js >= 18.

export interface RuLakeHttpOptions {
  /** Bearer token. Either `"Bearer XXX"` or just `"XXX"` (we add `Bearer `). */
  token?: string;
  /** Extra headers to send on every request. */
  headers?: Record<string, string>;
}

export interface RuLakeQueryArgs {
  /** Tool intent. Server must allow this for the auth principal. */
  intent: "search" | "verify" | "explain" | "refresh";
  backend: string;
  collection: string;
  /** Query embedding for `search` / `refresh`. */
  query?: number[];
  /** Top-K. */
  k?: number;
  /** Optional witness to verify against (for `verify`). */
  witness?: string;
  /** Risk envelope: low / medium / high — gates retrieval strictness. */
  risk?: "low" | "medium" | "high";
  /** Freshness budget in milliseconds (0 = Fresh, large = Eventual). */
  freshness_ms?: number;
  /** Server-side policy hint (passed through). */
  policy?: string;
}

export interface RuLakeQueryResult {
  /** Top-K hits. Shape depends on intent; `search` returns hits[]. */
  hits?: Array<{ id: string | number; score: number; backend?: string; collection?: string }>;
  /** Witness of the bundle the result was drawn from. */
  witness?: string;
  /** Server's decision trace — chosen_action, reason_code, etc. */
  decision_trace?: Record<string, unknown>;
  /** Raw passthrough for forward-compat. */
  [key: string]: unknown;
}

export class RuLakeHttpError extends Error {
  status?: number;
  code?: string | number;
  constructor(message: string, opts?: { status?: number; code?: string | number });
}

export class RuLakeHttp {
  constructor(url: string, opts?: RuLakeHttpOptions);
  /** MCP `initialize` + `notifications/initialized`. Stores the session id. */
  connect(): Promise<void>;
  /** Call the `rulake_query` MCP tool. */
  query(args: RuLakeQueryArgs): Promise<RuLakeQueryResult>;
}

export default RuLakeHttp;
