/**
 * Tests for the MCP tool layer.
 *
 * The MCP SDK supports an in-memory transport pair so we can drive the
 * server end-to-end (real registration → real dispatch → real response)
 * without going through stdio. We use the real Rust-produced fixture
 * at `/tmp/rulake-fixture` when present; otherwise the snapshot tests
 * are skipped with a clear message so CI on a fresh checkout doesn't
 * fail just because nobody ran the Rust example.
 */

import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import {
  InMemoryTransport,
} from "@modelcontextprotocol/sdk/inMemory.js";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

import { buildServer } from "../src/server.js";
import { loadSnapshot } from "../src/snapshot.js";
import { nearestK } from "../src/search.js";

const FIXTURE_DIR = "/tmp/rulake-fixture";
const HAS_FIXTURE = existsSync(join(FIXTURE_DIR, "table.rulake.json")) && existsSync(join(FIXTURE_DIR, "index.rbpx"));

/** Clone the Rust fixture into a tmpdir so each test can mutate without poisoning the source. */
function cloneFixture(): string | null {
  if (!HAS_FIXTURE) return null;
  const dst = mkdtempSync(join(tmpdir(), "rulake-mcp-fixture-"));
  writeFileSync(join(dst, "table.rulake.json"), readFileSync(join(FIXTURE_DIR, "table.rulake.json")));
  writeFileSync(join(dst, "index.rbpx"), readFileSync(join(FIXTURE_DIR, "index.rbpx")));
  return dst;
}

async function makeClient() {
  const server = buildServer();
  const [clientTransport, serverTransport] = InMemoryTransport.createLinkedPair();
  const client = new Client({ name: "test-client", version: "0.1.0" });
  await Promise.all([server.connect(serverTransport), client.connect(clientTransport)]);
  return { client, server };
}

describe("MCP server tool registration", () => {
  it("lists rulake_search, rulake_verify_witness, rulake_bundle_info", async () => {
    const { client } = await makeClient();
    const tools = await client.listTools();
    const names = tools.tools.map((t: { name: string }) => t.name).sort();
    expect(names).toEqual([
      "rulake_bundle_info",
      "rulake_search",
      "rulake_verify_witness",
    ]);
  });
});

describe.skipIf(!HAS_FIXTURE)("rulake_verify_witness against real snapshot", () => {
  let snapshotDir: string;
  beforeAll(() => {
    snapshotDir = cloneFixture()!;
  });
  afterAll(() => {
    if (snapshotDir) rmSync(snapshotDir, { recursive: true, force: true });
  });

  it("verifies the Rust-produced bundle", async () => {
    const { client } = await makeClient();
    const result = await client.callTool({
      name: "rulake_verify_witness",
      arguments: { snapshot_dir: snapshotDir },
    });
    expect(result.isError).toBeFalsy();
    const sc = result.structuredContent as { ok: boolean; expected: string; actual: string };
    expect(sc.ok).toBe(true);
    expect(sc.expected).toBe(sc.actual);
    expect(sc.expected).toMatch(/^[0-9a-f]{64}$/);
  });

  it("reports mismatch on a tampered witness", async () => {
    const tampered = mkdtempSync(join(tmpdir(), "rulake-mcp-tamper-"));
    try {
      const raw = readFileSync(join(snapshotDir, "table.rulake.json"), "utf8");
      writeFileSync(
        join(tampered, "table.rulake.json"),
        raw.replace(/"rvf_witness":\s*"[^"]+"/, `"rvf_witness": "${"0".repeat(64)}"`),
      );
      writeFileSync(join(tampered, "index.rbpx"), readFileSync(join(snapshotDir, "index.rbpx")));

      const { client } = await makeClient();
      const result = await client.callTool({
        name: "rulake_verify_witness",
        arguments: { snapshot_dir: tampered },
      });
      expect(result.isError).toBeFalsy();
      const sc = result.structuredContent as { ok: boolean; expected: string; actual: string };
      expect(sc.ok).toBe(false);
      expect(sc.actual).not.toBe(sc.expected);
    } finally {
      rmSync(tampered, { recursive: true, force: true });
    }
  });
});

describe.skipIf(!HAS_FIXTURE)("rulake_bundle_info", () => {
  it("returns parsed bundle metadata", async () => {
    const dir = cloneFixture()!;
    try {
      const { client } = await makeClient();
      const result = await client.callTool({
        name: "rulake_bundle_info",
        arguments: { snapshot_dir: dir },
      });
      expect(result.isError).toBeFalsy();
      const sc = result.structuredContent as Record<string, unknown>;
      // Shape-only assertions — the README points users at the
      // `warm_restart` example to produce a fixture, which differs
      // from the older `sidecar_daemon` (different data_ref / dim).
      // Pin the witness contract, not the example-specific values.
      expect(sc.witness_verified).toBe(true);
      expect(sc.format_version).toBe(2);
      expect(typeof sc.data_ref).toBe("string");
      expect(typeof sc.dim).toBe("number");
      expect((sc.dim as number) > 0).toBe(true);
      expect(typeof sc.rotation_seed === "number" || typeof sc.rotation_seed === "string").toBe(true);
      expect(sc.rvf_witness).toMatch(/^[0-9a-f]{64}$/);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
});

describe.skipIf(!HAS_FIXTURE)("rulake_search end-to-end", () => {
  it("returns K hits matching local brute-force ranking", async () => {
    const dir = cloneFixture()!;
    try {
      const snap = loadSnapshot(dir);
      const dim = snap.bundle.dim;
      // A query that matches one of the corpus vectors exactly so we
      // can pin the top-1 score.
      const query = Array.from(snap.index.items[5]!.vector);
      const expected = nearestK(snap.index.items, Float32Array.from(query), 3);

      const { client } = await makeClient();
      const result = await client.callTool({
        name: "rulake_search",
        arguments: { snapshot_dir: dir, query, k: 3 },
      });
      expect(result.isError).toBeFalsy();
      const sc = result.structuredContent as {
        hits: Array<{ id: number; score: number }>;
        n: number;
        dim: number;
        method: string;
      };
      expect(sc.dim).toBe(dim);
      expect(sc.method).toBe("brute-force-l2");
      expect(sc.hits.length).toBe(3);
      expect(sc.hits[0]!.id).toBe(expected[0]!.id);
      // Self-match is exact.
      expect(sc.hits[0]!.score).toBeCloseTo(0, 6);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
});

describe("rulake_search input validation", () => {
  it("returns isError on a missing snapshot dir", async () => {
    const dir = mkdirSync(join(tmpdir(), `rulake-empty-${Date.now()}`), { recursive: true });
    try {
      const { client } = await makeClient();
      const result = await client.callTool({
        name: "rulake_search",
        arguments: { snapshot_dir: dir!, query: [0, 0, 0], k: 1 },
      });
      expect(result.isError).toBe(true);
    } finally {
      if (dir) rmSync(dir, { recursive: true, force: true });
    }
  });
});
