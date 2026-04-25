/**
 * Filesystem watcher for ruLake publish directories.
 *
 * The Rust `publish_bundle(key, dir)` writes
 *
 *     <root>/<backend>/<collection>/table.rulake.json
 *
 * (and may also place an `index.rbpx` next to it). This watcher walks
 * the root, finds every `table.rulake.json` under a two-deep
 * `<backend>/<collection>` layout, parses each one, and keeps a hot
 * `Map<bundleKey, BundleEntry>` for the HTTP layer.
 *
 * Atomic-write friendly: the Rust writer renames a temp file into
 * place, so we either see no file or the new complete file — never a
 * truncated one. We still defensively re-parse on every change event
 * and drop the entry on parse failure rather than serving a stale one.
 */

import { readFileSync, statSync } from "node:fs";
import { join, relative, sep } from "node:path";

import chokidar, { type FSWatcher } from "chokidar";

import { parseBundle, type RuLakeBundle } from "./bundle.js";
import { verifyBundle } from "./witness.js";

const SIDECAR = "table.rulake.json";

export interface BundleEntry {
  key: string; // "<backend>/<collection>"
  backend: string;
  collection: string;
  bundle: RuLakeBundle;
  /** Absolute on-disk path of the table.rulake.json. */
  path: string;
  /** Size in bytes, for diagnostics. */
  size: number;
  /** Whether the on-disk witness verifies — clients should NOT trust unverified bundles. */
  verified: boolean;
  /** mtime in millis when we last reloaded. */
  mtimeMs: number;
}

export class BundleStore {
  private readonly entries = new Map<string, BundleEntry>();
  private watcher: FSWatcher | null = null;

  constructor(public readonly rootDir: string) {}

  list(): BundleEntry[] {
    return [...this.entries.values()].sort((a, b) => a.key.localeCompare(b.key));
  }

  get(key: string): BundleEntry | undefined {
    return this.entries.get(key);
  }

  size(): number {
    return this.entries.size;
  }

  /**
   * Start watching. Resolves once the initial scan settles so HTTP
   * tests can trust `list()` immediately after `await store.start()`.
   */
  async start(): Promise<void> {
    // Match `<root>/*/*/table.rulake.json` exactly. chokidar's glob
    // dialect supports the `**/table.rulake.json` form too, but the
    // two-deep layout is what `publish_bundle` produces — keep it
    // strict so unrelated files don't show up as bundles.
    const pattern = join(this.rootDir, "**", SIDECAR);
    this.watcher = chokidar.watch(pattern, {
      ignoreInitial: false,
      awaitWriteFinish: { stabilityThreshold: 75, pollInterval: 25 },
      depth: 5,
    });

    const ready = new Promise<void>((resolve) => {
      this.watcher!.on("ready", () => resolve());
    });

    this.watcher
      .on("add", (p) => this.reload(p))
      .on("change", (p) => this.reload(p))
      .on("unlink", (p) => this.drop(p))
      .on("error", (err) => {
        // eslint-disable-next-line no-console
        console.error(`[bundle-publisher] watcher error: ${err.message}`);
      });

    await ready;
  }

  async stop(): Promise<void> {
    await this.watcher?.close();
    this.watcher = null;
    this.entries.clear();
  }

  /** Compute the `<backend>/<collection>` key from a sidecar path. */
  private deriveKey(absPath: string): { key: string; backend: string; collection: string } | null {
    const rel = relative(this.rootDir, absPath);
    const parts = rel.split(sep);
    // Expected: [backend, collection, "table.rulake.json"]
    if (parts.length !== 3 || parts[2] !== SIDECAR) {
      return null;
    }
    const backend = parts[0]!;
    const collection = parts[1]!;
    return { key: `${backend}/${collection}`, backend, collection };
  }

  private reload(absPath: string): void {
    const meta = this.deriveKey(absPath);
    if (!meta) return; // Ignore files outside the <backend>/<collection> layout.

    let raw: string;
    let st;
    try {
      st = statSync(absPath);
      raw = readFileSync(absPath, "utf8");
    } catch {
      // Reader race with writer / file vanished — let the next event
      // sort it out. Don't drop the prior entry on a transient error.
      return;
    }

    let bundle: RuLakeBundle;
    try {
      bundle = parseBundle(raw);
    } catch (err) {
      // Drop the entry — serving an unparseable bundle is worse than
      // 404. Log so an operator sees what happened.
      // eslint-disable-next-line no-console
      console.error(`[bundle-publisher] parse failed for ${absPath}: ${(err as Error).message}`);
      this.entries.delete(meta.key);
      return;
    }

    const { ok } = verifyBundle(bundle);
    this.entries.set(meta.key, {
      key: meta.key,
      backend: meta.backend,
      collection: meta.collection,
      bundle,
      path: absPath,
      size: st.size,
      verified: ok,
      mtimeMs: st.mtimeMs,
    });
  }

  private drop(absPath: string): void {
    const meta = this.deriveKey(absPath);
    if (!meta) return;
    this.entries.delete(meta.key);
  }
}
