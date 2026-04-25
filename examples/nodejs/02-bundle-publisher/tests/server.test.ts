/**
 * Integration tests for the bundle publisher HTTP layer.
 *
 * Uses a real on-disk publish directory (under `os.tmpdir()`) and
 * exercises chokidar end-to-end so we catch any glob / event-routing
 * regressions. Bundles are written via the Node bundle helpers — the
 * Rust crate is not required for the test to run, but the witness is
 * recomputed by the same code that verifies Rust-produced bundles in
 * module 01, so a passing test here implies the wire format is what
 * any ruLake reader expects.
 */

import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import express from "express";
import request from "supertest";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

import { buildApp } from "../src/server.js";
import { computeWitness } from "../src/witness.js";

interface BundleFixture {
  data_ref: string;
  dim: number;
  rotation_seed: number;
  rerank_factor: number;
  generation: number;
  pii_policy: null;
  lineage_id: null;
  format_version: number;
  rvf_witness: string;
}

function buildBundle(opts: {
  data_ref: string;
  dim: number;
  rotation_seed: number;
  rerank_factor: number;
  generation: number;
}): BundleFixture {
  const witness = computeWitness({
    dataRef: opts.data_ref,
    dim: opts.dim,
    rotationSeed: BigInt(opts.rotation_seed),
    rerankFactor: opts.rerank_factor,
    generation: { kind: "Num", value: BigInt(opts.generation) },
  });
  return {
    format_version: 2,
    data_ref: opts.data_ref,
    dim: opts.dim,
    rotation_seed: opts.rotation_seed,
    rerank_factor: opts.rerank_factor,
    generation: opts.generation,
    pii_policy: null,
    lineage_id: null,
    rvf_witness: witness,
  };
}

function writeBundle(rootDir: string, backend: string, collection: string, bundle: BundleFixture): void {
  const dir = join(rootDir, backend, collection);
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, "table.rulake.json"), JSON.stringify(bundle, null, 2));
}

describe("bundle-publisher HTTP", () => {
  let rootDir: string;
  let app: express.Express;
  let cleanup: (() => Promise<void>) | null = null;

  beforeAll(async () => {
    rootDir = mkdtempSync(join(tmpdir(), "rulake-publisher-test-"));
    writeBundle(rootDir, "publisher", "memories", buildBundle({
      data_ref: "local://publisher/memories",
      dim: 8,
      rotation_seed: 42,
      rerank_factor: 20,
      generation: 1,
    }));
    writeBundle(rootDir, "shard-0", "vectors", buildBundle({
      data_ref: "gs://bucket/shard-0.parquet",
      dim: 384,
      rotation_seed: 7,
      rerank_factor: 5,
      generation: 100,
    }));

    const built = await buildApp({ rootDir });
    app = built.app;
    cleanup = () => built.store.stop();
  });

  afterAll(async () => {
    await cleanup?.();
    if (rootDir) rmSync(rootDir, { recursive: true, force: true });
  });

  it("GET /health reports watched dir and bundle count", async () => {
    const res = await request(app).get("/health");
    expect(res.status).toBe(200);
    expect(res.body.ok).toBe(true);
    expect(res.body.bundles).toBe(2);
    expect(res.body.watched).toBe(rootDir);
  });

  it("GET /bundles enumerates published bundles", async () => {
    const res = await request(app).get("/bundles");
    expect(res.status).toBe(200);
    expect(res.body.count).toBe(2);
    const keys: string[] = res.body.bundles.map((b: { key: string }) => b.key);
    expect(keys).toContain("publisher/memories");
    expect(keys).toContain("shard-0/vectors");
  });

  it("GET .../table.rulake.json returns the body with the witness ETag", async () => {
    const res = await request(app).get("/bundles/publisher/memories/table.rulake.json");
    expect(res.status).toBe(200);
    expect(res.headers["content-type"]).toMatch(/application\/json/);
    expect(res.headers["cache-control"]).toBe("no-cache, must-revalidate");
    const body = JSON.parse(res.text);
    expect(body.format_version).toBe(2);
    expect(body.data_ref).toBe("local://publisher/memories");
    expect(body.rvf_witness).toMatch(/^[0-9a-f]{64}$/);
    expect(res.headers.etag).toBe(`"${body.rvf_witness}"`);
    expect(res.headers["x-rulake-witness"]).toBe(body.rvf_witness);
  });

  it("GET .../table.rulake.json honours If-None-Match (304)", async () => {
    const first = await request(app).get("/bundles/publisher/memories/table.rulake.json");
    const etag = first.headers.etag as string | undefined;
    expect(etag).toBeDefined();

    const second = await request(app)
      .get("/bundles/publisher/memories/table.rulake.json")
      .set("If-None-Match", etag!);
    expect(second.status).toBe(304);
    // 304 must not carry a body.
    expect(second.text).toBe("");
  });

  it("GET .../witness returns just the witness probe", async () => {
    const res = await request(app).get("/bundles/publisher/memories/witness");
    expect(res.status).toBe(200);
    expect(res.body.witness).toMatch(/^[0-9a-f]{64}$/);
    expect(res.body.format_version).toBe(2);
    expect(res.body.verified).toBe(true);
    expect(res.body.generation).toBe(1);
    expect(res.headers.etag).toBe(`"${res.body.witness}"`);
  });

  it("GET unknown bundle returns 404", async () => {
    const res = await request(app).get("/bundles/nope/missing/table.rulake.json");
    expect(res.status).toBe(404);
  });

  it("rejects bundles whose witness doesn't verify (poison guard)", async () => {
    // Drop a sidecar with a fabricated witness — must NOT be served.
    const poisoned = buildBundle({
      data_ref: "evil://x",
      dim: 8,
      rotation_seed: 1,
      rerank_factor: 1,
      generation: 1,
    });
    poisoned.rvf_witness = "0".repeat(64); // wrong on purpose
    writeBundle(rootDir, "evil", "poisoned", poisoned);

    // Wait for chokidar to pick it up. awaitWriteFinish in the watcher
    // is set to 75ms; 250ms is comfortable headroom.
    await new Promise((r) => setTimeout(r, 250));

    const res = await request(app).get("/bundles/evil/poisoned/table.rulake.json");
    expect(res.status).toBe(503);
    expect(res.body.error).toMatch(/verification failed/);
  });
});
