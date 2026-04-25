/**
 * MCP server exposing a ruLake snapshot as agent-callable tools.
 *
 * Tools:
 *   - rulake_search           — brute-force exact L2 K-NN against the snapshot.
 *   - rulake_verify_witness   — recompute SHAKE-256 witness, report match/mismatch.
 *   - rulake_bundle_info      — return the bundle's metadata (witness, generation, …).
 *
 * Transport: stdio. The client (e.g. Claude Code, an inspector,
 * @modelcontextprotocol/inspector) launches this server and speaks
 * JSON-RPC over stdin/stdout.
 *
 * Snapshot directories are provided per call so a single server can
 * serve many tables; load results are cached in-process keyed by the
 * absolute path so repeated calls don't re-parse the .rbpx.
 */

import { resolve } from "node:path";

import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod";

import { nearestK } from "./search.js";
import { loadSnapshot, type SnapshotLoadResult } from "./snapshot.js";

const VERSION = "0.1.0";

/**
 * Light in-process cache so an agent can call us repeatedly without
 * re-parsing the same `.rbpx`. Bounded by `MAX_CACHED_SNAPSHOTS`: an
 * MCP client could otherwise pass thousands of unique paths and grow
 * the map (and the Float32Array buffers it holds) without bound.
 * LRU eviction: when full, drop the least-recently-inserted entry.
 */
const MAX_CACHED_SNAPSHOTS = 16;
const snapshotCache = new Map<string, SnapshotLoadResult>();

function cachedLoad(snapshotDir: string): SnapshotLoadResult {
  const abs = resolve(snapshotDir);
  const existing = snapshotCache.get(abs);
  if (existing) {
    // Refresh recency: re-insert at the end of the iteration order.
    snapshotCache.delete(abs);
    snapshotCache.set(abs, existing);
    return existing;
  }
  const entry = loadSnapshot(abs);
  snapshotCache.set(abs, entry);
  if (snapshotCache.size > MAX_CACHED_SNAPSHOTS) {
    const oldest = snapshotCache.keys().next().value;
    if (oldest !== undefined) snapshotCache.delete(oldest);
  }
  return entry;
}

function bigintToJson(n: bigint): number | string {
  return n <= BigInt(Number.MAX_SAFE_INTEGER) ? Number(n) : n.toString();
}

function generationJson(g: import("./bundle.js").Generation): number | string {
  return g.kind === "Num" ? bigintToJson(g.value) : g.value;
}

/**
 * Construct the MCP server with all three ruLake tools registered.
 * Exposed for tests so they can drive the tool handlers directly
 * without spinning up the stdio transport.
 */
export function buildServer(): McpServer {
  const server = new McpServer({
    name: "rulake-mcp-tool",
    version: VERSION,
  });

  server.registerTool(
    "rulake_search",
    {
      title: "ruLake search",
      description:
        "Brute-force exact L2 K-nearest-neighbour search against a ruLake snapshot directory. " +
        "Verifies the bundle witness before serving results — refuses to search if it fails.",
      inputSchema: {
        snapshot_dir: z
          .string()
          .min(1)
          .describe("Absolute path to a ruLake snapshot dir containing table.rulake.json + index.rbpx"),
        query: z
          .array(z.number())
          .min(1)
          .describe("Query vector, length must equal the snapshot's dim"),
        k: z.number().int().min(1).max(1000).default(10).describe("Number of nearest neighbours to return"),
      },
    },
    async ({ snapshot_dir, query, k }) => {
      let snap: SnapshotLoadResult;
      try {
        snap = cachedLoad(snapshot_dir);
      } catch (err) {
        return errorResult(`failed to load snapshot: ${(err as Error).message}`);
      }
      if (!snap.witnessOk) {
        return errorResult(
          `witness verification failed for ${snapshot_dir} — refusing to search. expected=${snap.expectedWitness} actual=${snap.bundle.rvf_witness}`,
        );
      }
      const q = Float32Array.from(query);
      if (q.length !== snap.bundle.dim) {
        return errorResult(`query length ${q.length} does not match bundle dim ${snap.bundle.dim}`);
      }
      const hits = nearestK(snap.index.items, q, k);
      return jsonResult({
        snapshot_dir: snap.paths.bundle.replace(/\/table\.rulake\.json$/, ""),
        witness: snap.bundle.rvf_witness,
        data_ref: snap.bundle.data_ref,
        n: snap.index.n,
        dim: snap.index.dim,
        method: "brute-force-l2",
        hits: hits.map((h) => ({ id: h.id, score: h.score })),
      });
    },
  );

  server.registerTool(
    "rulake_verify_witness",
    {
      title: "Verify ruLake bundle witness",
      description:
        "Recompute the SHAKE-256(32) witness of a ruLake snapshot's table.rulake.json and report whether " +
        "it matches the on-disk rvf_witness. Use this as a precondition before trusting any cache entry.",
      inputSchema: {
        snapshot_dir: z.string().min(1).describe("Absolute path to the ruLake snapshot directory"),
      },
    },
    async ({ snapshot_dir }) => {
      try {
        const snap = cachedLoad(snapshot_dir);
        return jsonResult({
          ok: snap.witnessOk,
          expected: snap.expectedWitness,
          actual: snap.bundle.rvf_witness,
          format_version: snap.bundle.format_version,
          data_ref: snap.bundle.data_ref,
          dim: snap.bundle.dim,
          generation: generationJson(snap.bundle.generation),
        });
      } catch (err) {
        return errorResult(`failed to load snapshot: ${(err as Error).message}`);
      }
    },
  );

  server.registerTool(
    "rulake_bundle_info",
    {
      title: "Bundle metadata",
      description:
        "Return the parsed bundle sidecar metadata (witness, data_ref, dim, generation, optional " +
        "policy / lineage tags). Cheap; doesn't load the .rbpx payload.",
      inputSchema: {
        snapshot_dir: z.string().min(1).describe("Absolute path to the ruLake snapshot directory"),
      },
    },
    async ({ snapshot_dir }) => {
      try {
        const snap = cachedLoad(snapshot_dir);
        return jsonResult({
          format_version: snap.bundle.format_version,
          data_ref: snap.bundle.data_ref,
          dim: snap.bundle.dim,
          rotation_seed: bigintToJson(snap.bundle.rotation_seed),
          rerank_factor: snap.bundle.rerank_factor,
          generation: generationJson(snap.bundle.generation),
          rvf_witness: snap.bundle.rvf_witness,
          witness_verified: snap.witnessOk,
          pii_policy: snap.bundle.pii_policy,
          lineage_id: snap.bundle.lineage_id,
          memory_class: snap.bundle.memory_class ?? null,
          sizes: snap.sizes,
          n: snap.index.n,
        });
      } catch (err) {
        return errorResult(`failed to load snapshot: ${(err as Error).message}`);
      }
    },
  );

  return server;
}

/** Serialize a JSON object as an MCP `text` content block. */
function jsonResult(payload: unknown): {
  content: { type: "text"; text: string }[];
  structuredContent?: Record<string, unknown>;
} {
  return {
    content: [{ type: "text", text: JSON.stringify(payload, null, 2) }],
    structuredContent: payload as Record<string, unknown>,
  };
}

function errorResult(message: string): {
  content: { type: "text"; text: string }[];
  isError: true;
} {
  return {
    content: [{ type: "text", text: message }],
    isError: true,
  };
}

// Stdio entry — only when invoked directly.
const isMain = import.meta.url === `file://${process.argv[1]}`;
if (isMain) {
  const transport = new StdioServerTransport();
  const server = buildServer();
  server.connect(transport).catch((err) => {
    process.stderr.write(`mcp server error: ${(err as Error).message}\n`);
    process.exit(1);
  });
}
