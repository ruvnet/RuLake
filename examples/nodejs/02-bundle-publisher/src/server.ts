/**
 * HTTP publisher for ruLake bundle sidecars.
 *
 * Endpoints:
 *
 *   GET /bundles/:backend/:collection/table.rulake.json
 *       The bundle JSON. ETag = "<witness>". Cache-Control: no-cache,
 *       must-revalidate. 200 (full body) or 304 (witness unchanged).
 *
 *   GET /bundles/:backend/:collection/witness
 *       Lightweight witness probe — { witness, generation, format_version }.
 *       Use this when you only need to decide whether to refresh.
 *
 *   GET /bundles
 *       Index of currently-served bundles { bundles: [...] }.
 *
 *   GET /health
 *       { ok, watched, bundles }.
 *
 * Run:
 *   tsx src/server.ts <publish-dir> [port]
 *
 * Where `<publish-dir>` is the same directory the Rust publisher's
 * `publish_bundle(key, dir)` writes into; the watcher discovers
 * `<backend>/<collection>/table.rulake.json` files automatically.
 */

import { resolve } from "node:path";

import express from "express";
import type { NextFunction, Request, Response } from "express";

import { applyBundleHeaders, ifNoneMatchHits } from "./etag.js";
import { BundleStore } from "./watcher.js";

export interface ServerOptions {
  rootDir: string;
  port?: number;
  /** Pass an existing `BundleStore` (e.g. for tests) instead of constructing one. */
  store?: BundleStore;
}

export interface RunningServer {
  app: express.Express;
  store: BundleStore;
  close(): Promise<void>;
  /** Set when listen() succeeded. */
  port?: number;
}

/** Construct (but do not listen) the express app + bundle store. */
export async function buildApp(opts: ServerOptions): Promise<{ app: express.Express; store: BundleStore }> {
  const store = opts.store ?? new BundleStore(resolve(opts.rootDir));
  if (!opts.store) {
    await store.start();
  }

  const app = express();
  app.disable("x-powered-by");

  app.get("/health", (_req: Request, res: Response) => {
    res.json({ ok: true, watched: store.rootDir, bundles: store.size() });
  });

  app.get("/bundles", (_req: Request, res: Response) => {
    res.json({
      watched: store.rootDir,
      count: store.size(),
      bundles: store.list().map((e) => ({
        key: e.key,
        backend: e.backend,
        collection: e.collection,
        witness: e.bundle.rvf_witness,
        format_version: e.bundle.format_version,
        verified: e.verified,
        size: e.size,
        mtime_ms: e.mtimeMs,
      })),
    });
  });

  // GET the canonical sidecar.
  app.get(
    "/bundles/:backend/:collection/table.rulake.json",
    (req: Request, res: Response, next: NextFunction) => {
      try {
        const { backend, collection } = req.params as { backend: string; collection: string };
        const entry = store.get(`${backend}/${collection}`);
        if (!entry) {
          res.status(404).json({ error: "bundle not found", key: `${backend}/${collection}` });
          return;
        }
        if (!entry.verified) {
          // Don't serve a sidecar whose witness doesn't verify — that's
          // exactly the cache-poisoning surface this whole protocol
          // exists to close.
          res.status(503).json({ error: "bundle present but witness verification failed", key: entry.key });
          return;
        }

        const witness = entry.bundle.rvf_witness;
        applyBundleHeaders(res, witness);

        if (ifNoneMatchHits(req.header("if-none-match"), witness)) {
          res.status(304).end();
          return;
        }

        res.type("application/json");
        // Round-trip via JSON.stringify — the on-disk file is what the
        // store loaded; serving the parsed-then-stringified form
        // guarantees we never echo trailing garbage from disk.
        res.send(serializeBundle(entry.bundle));
      } catch (err) {
        next(err);
      }
    },
  );

  // Lightweight witness-only probe. Useful when a peer wants to ask
  // "should I refetch?" without paying for the full body.
  app.get(
    "/bundles/:backend/:collection/witness",
    (req: Request, res: Response) => {
      const { backend, collection } = req.params as { backend: string; collection: string };
      const entry = store.get(`${backend}/${collection}`);
      if (!entry) {
        res.status(404).json({ error: "bundle not found", key: `${backend}/${collection}` });
        return;
      }
      applyBundleHeaders(res, entry.bundle.rvf_witness);

      if (ifNoneMatchHits(req.header("if-none-match"), entry.bundle.rvf_witness)) {
        res.status(304).end();
        return;
      }

      res.json({
        witness: entry.bundle.rvf_witness,
        generation: serializeGeneration(entry.bundle.generation),
        format_version: entry.bundle.format_version,
        verified: entry.verified,
      });
    },
  );

  // Final error handler — keep messages bland; no stack traces over the wire.
  app.use((err: unknown, _req: Request, res: Response, _next: NextFunction) => {
    // eslint-disable-next-line no-console
    console.error(`[bundle-publisher] unhandled error: ${(err as Error).message}`);
    if (!res.headersSent) {
      res.status(500).json({ error: "internal error" });
    }
  });

  return { app, store };
}

/** Bundle → wire JSON. Mirrors the Rust serde output (memory_class skipped when null/undefined). */
function serializeBundle(b: import("./bundle.js").RuLakeBundle): string {
  const out: Record<string, unknown> = {
    format_version: b.format_version,
    data_ref: b.data_ref,
    dim: b.dim,
    rotation_seed: bigintToJson(b.rotation_seed),
    rerank_factor: b.rerank_factor,
    generation: serializeGeneration(b.generation),
    rvf_witness: b.rvf_witness,
    pii_policy: b.pii_policy,
    lineage_id: b.lineage_id,
  };
  if (b.memory_class !== undefined && b.memory_class !== null) {
    out.memory_class = b.memory_class;
  }
  return JSON.stringify(out, null, 2);
}

function serializeGeneration(g: import("./bundle.js").Generation): number | string {
  if (g.kind === "Num") {
    return bigintToJson(g.value);
  }
  return g.value;
}

/** Render a u64 as a JSON-safe number when it fits, otherwise as a string. */
function bigintToJson(n: bigint): number | string {
  if (n <= BigInt(Number.MAX_SAFE_INTEGER)) {
    return Number(n);
  }
  return n.toString();
}

/** Start listening; returns a handle for graceful shutdown. */
export async function startServer(opts: ServerOptions): Promise<RunningServer> {
  const { app, store } = await buildApp(opts);
  const port = opts.port ?? 0;
  return await new Promise<RunningServer>((resolveServer, reject) => {
    const httpServer = app.listen(port, () => {
      const addr = httpServer.address();
      const boundPort = typeof addr === "object" && addr ? addr.port : port;
      resolveServer({
        app,
        store,
        port: boundPort,
        async close() {
          await new Promise<void>((r) => httpServer.close(() => r()));
          await store.stop();
        },
      });
    });
    httpServer.on("error", reject);
  });
}

// CLI entry — only when invoked directly.
const isMain = import.meta.url === `file://${process.argv[1]}`;
if (isMain) {
  const args = process.argv.slice(2);
  if (args.length < 1) {
    process.stderr.write("usage: tsx src/server.ts <publish-dir> [port]\n");
    process.exit(2);
  }
  const rootDir = args[0]!;
  const port = args[1] ? Number(args[1]) : 8787;
  startServer({ rootDir, port }).then(
    (s) => {
      process.stdout.write(`bundle-publisher listening on :${s.port}, watching ${s.store.rootDir}\n`);
      const stop = async () => {
        await s.close();
        process.exit(0);
      };
      process.on("SIGINT", () => void stop());
      process.on("SIGTERM", () => void stop());
    },
    (err) => {
      process.stderr.write(`failed to start: ${err.message}\n`);
      process.exit(1);
    },
  );
}
