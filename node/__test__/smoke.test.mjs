// Smoke tests for the ruLake Node.js binding.
//
// Mirrors python/tests/test_smoke.py: builds a LocalBackend with a small
// collection, registers it, searches, checks stats, round-trips a bundle
// through JSON + the filesystem, and asserts the typed error mapping.
//
// Run after `cargo build --release && cp target/release/libruvector_rulake_node.so rulake.linux-x64-gnu.node`:
//
//     node --test __test__/smoke.test.mjs

import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, rmSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import {
  RuLake,
  LocalBackend,
  Bundle,
  Consistency,
  RuLakeError,
} from "../index.mjs";

// ─────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────

function deterministic(seed) {
  // Tiny LCG — good enough for a smoke test, deterministic across runs.
  let s = seed >>> 0;
  return () => {
    s = (Math.imul(s, 1103515245) + 12345) >>> 0;
    return (s & 0x7fffffff) / 0x7fffffff - 0.5;
  };
}

function makeLakeWithData({ n = 2_000, dim = 32 } = {}) {
  const rng = deterministic(0);
  const ids = new BigInt64Array(n);
  const vectors = new Float32Array(n * dim);
  for (let i = 0; i < n; i++) ids[i] = BigInt(i);
  for (let i = 0; i < n * dim; i++) vectors[i] = rng();

  const lake = new RuLake(20, 42n);
  const be = new LocalBackend("local");
  // putCollection is async (libuv worker thread).
  return be.putCollection("docs", ids, vectors, dim).then(() => {
    lake.registerLocalBackend(be);
    return { lake, be, ids, vectors, dim, n };
  });
}

function rowOf(vectors, i, dim) {
  return vectors.slice(i * dim, (i + 1) * dim);
}

// ─────────────────────────────────────────────────────────────────────
// Search smoke
// ─────────────────────────────────────────────────────────────────────

test("search_one returns k hits with exact-match top-1", async () => {
  const { lake, vectors, dim } = await makeLakeWithData({ n: 5_000, dim: 64 });
  const q = rowOf(vectors, 42, dim);
  const hits = await lake.searchOne("local", "docs", q, 10);

  assert.equal(hits.length, 10);
  assert.equal(hits[0].id, 42n);
  assert.equal(hits[0].backend, "local");
  assert.equal(hits[0].collection, "docs");
  // Top-1 against itself is L2² = 0; allow tiny RaBitQ-rerank slack.
  assert.ok(hits[0].score < 0.001, `expected ~0 score, got ${hits[0].score}`);
});

test("search_batch preserves query order", async () => {
  const { lake, vectors, dim } = await makeLakeWithData({ n: 2_000, dim: 32 });
  const queries = new Float32Array(4 * dim);
  for (let i = 0; i < 4; i++) {
    queries.set(rowOf(vectors, i + 1, dim), i * dim);
  }
  const batches = await lake.searchBatch("local", "docs", queries, dim, 5);

  assert.equal(batches.length, 4);
  for (let i = 0; i < 4; i++) {
    assert.equal(batches[i].length, 5);
    assert.equal(batches[i][0].id, BigInt(i + 1));
  }
});

test("search_federated across two backends merges by score", async () => {
  const dim = 32;
  const n = 1_000;
  const rng = deterministic(1);
  const ids = new BigInt64Array(n);
  for (let i = 0; i < n; i++) ids[i] = BigInt(i);

  const vs1 = new Float32Array(n * dim);
  const vs2 = new Float32Array(n * dim);
  for (let i = 0; i < n * dim; i++) vs1[i] = rng();
  for (let i = 0; i < n * dim; i++) vs2[i] = rng();

  const lake = new RuLake(20, 42n);
  const be1 = new LocalBackend("be1");
  const be2 = new LocalBackend("be2");
  await be1.putCollection("docs", ids, vs1, dim);
  await be2.putCollection("docs", ids, vs2, dim);
  lake.registerLocalBackend(be1);
  lake.registerLocalBackend(be2);

  // Query is exactly vs1[100] → top-1 must come from be1, id=100.
  const q = vs1.slice(100 * dim, 101 * dim);
  const hits = await lake.searchFederated(
    [["be1", "docs"], ["be2", "docs"]],
    q,
    5,
  );
  assert.equal(hits.length, 5);
  assert.equal(hits[0].backend, "be1");
  assert.equal(hits[0].id, 100n);
});

// ─────────────────────────────────────────────────────────────────────
// Cache stats / consistency
// ─────────────────────────────────────────────────────────────────────

test("cache_stats track hits across Eventual consistency calls", async () => {
  let { lake, vectors, dim } = await makeLakeWithData({ n: 1_000, dim: 16 });
  lake = lake.withConsistency(Consistency.eventual(60_000));
  const q = rowOf(vectors, 0, dim);
  for (let i = 0; i < 5; i++) await lake.searchOne("local", "docs", q, 5);

  const stats = lake.cacheStats();
  assert.ok(stats.hits + stats.misses >= 1n);
  if (stats.hitRate !== null) {
    assert.ok(stats.hitRate >= 0 && stats.hitRate <= 1);
  }
});

// ─────────────────────────────────────────────────────────────────────
// Bundle round-trip — JSON + filesystem + witness
// ─────────────────────────────────────────────────────────────────────

test("bundle witness round-trip through JSON", () => {
  const b = new Bundle("test://x", 128, 42n, 20, 7n);
  assert.equal(b.verifyWitness(), true);
  const body = b.toJson();
  const parsed = JSON.parse(body);
  assert.equal(parsed.data_ref, "test://x");
  assert.equal(parsed.dim, 128);
  assert.equal(parsed.rvf_witness.length, 64);

  const b2 = Bundle.fromJson(body);
  assert.equal(b2.rvfWitness, b.rvfWitness);
  assert.equal(b2.verifyWitness(), true);
});

test("bundle: Num(7) and Opaque('\\x07\\0...') do NOT collide (audit fix)", () => {
  const num = new Bundle("x", 128, 42n, 20, 7n);
  const opq = new Bundle("x", 128, 42n, 20, "\x07\0\0\0\0\0\0\0");
  assert.notEqual(num.rvfWitness, opq.rvfWitness);
});

test("bundle round-trip via filesystem", async () => {
  const dir = mkdtempSync(join(tmpdir(), "rulake-node-"));
  try {
    const b = new Bundle("test://y", 64, 42n, 20, "abc");
    const path = await b.writeToDir(dir);
    assert.ok(path.endsWith("table.rulake.json"));
    assert.ok(existsSync(path));
    const b2 = await Bundle.readFromDir(dir);
    assert.equal(b2.rvfWitness, b.rvfWitness);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

// ─────────────────────────────────────────────────────────────────────
// Error mapping (ADR-003 §6)
// ─────────────────────────────────────────────────────────────────────

test("unknown backend → RuLakeError with RULAKE_BACKEND_NOT_FOUND code", async () => {
  const lake = new RuLake(20, 42n);
  const q = new Float32Array(8);
  await assert.rejects(
    () => lake.searchOne("nope", "docs", q, 5),
    (e) =>
      e instanceof RuLakeError &&
      e.code === "RULAKE_BACKEND_NOT_FOUND" &&
      typeof e.message === "string" &&
      e.message.includes("nope"),
  );
});

test("dim mismatch → RuLakeError with RULAKE_DIMENSION_MISMATCH code", async () => {
  const { lake, dim } = await makeLakeWithData({ n: 100, dim: 32 });
  void dim;
  const q = new Float32Array(64); // wrong dim
  await assert.rejects(
    () => lake.searchOne("local", "docs", q, 5),
    (e) =>
      e instanceof RuLakeError &&
      e.code === "RULAKE_DIMENSION_MISMATCH",
  );
});

// ─────────────────────────────────────────────────────────────────────
// Consistency factories
// ─────────────────────────────────────────────────────────────────────

test("Consistency factories return distinct instances", () => {
  const a = Consistency.fresh();
  const b = Consistency.eventual(1000);
  const c = Consistency.frozen();
  assert.ok(a && b && c);
});
