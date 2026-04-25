/**
 * Tests for the rulake-demo stdout parser.
 *
 * The parser is deliberately defensive — these tests pin the expected
 * shape against real (or near-real) stdout fixtures. If the Rust binary
 * changes its output, update the fixtures here AND the regexes in
 * `src/parser.ts`.
 */

import { describe, expect, it } from "vitest";

import { parseBenchmarkOutput } from "../src/parser.js";

const FAST_FIXTURE = `\
=== ruLake benchmark harness ===
clustered Gaussian, D=128, 100 clusters, rerank×20, Fresh consistency unless noted
queries are warm (not timing first miss unless labeled 'prime')

── n = 5000 ──────────────────────────────────────────
  direct RaBitQ+ (Haar)    build=    22.0 ms   qps=   19827
  direct RaBitQ+ (Hadamard) build=     6.9 ms   qps=   21186   build_speedup=3.17×
  ruLake (Fresh)           prime=     4.5 ms   qps=   19161   tax=1.03×
  ruLake (Eventual 60s)    prime=     3.1 ms   qps=   19897   tax=1.00×
  ruLake federated (2 shards, Eventual)  prime=   3.2 ms   qps=   26221

Notes:
  - 'tax' is the direct-QPS / lake-QPS ratio (1.0 = free, 2.0 = lake is 2× slower).
  - Fresh calls LocalBackend::generation() on every query (one hash-map read —
    on a real backend this is a network round-trip, expect materially higher tax).
`;

const FULL_FIXTURE = `\
=== ruLake benchmark harness ===
clustered Gaussian, D=128, 100 clusters, rerank×20, Fresh consistency unless noted

── n = 5000 ──────────────────────────────────────────
  direct RaBitQ+ (Haar)    build=    22.0 ms   qps=   19827
  ruLake (Fresh)           prime=     4.5 ms   qps=   19161   tax=1.03×
  ruLake federated (2 shards, Eventual)  prime=   3.2 ms   qps=   26221
  ruLake federated (4 shards, Eventual)  prime=   3.5 ms   qps=   27800

── search_batch vs per-query loop (n=100000) ──
  batch=  8   qps=   19898   speedup vs per-query 1.04×
  batch= 32   qps=   21500   speedup vs per-query 1.13×
  batch=128   qps=   22000   speedup vs per-query 1.15×
  batch=300   qps=   22500   speedup vs per-query 1.18×
  per-query loop   qps=   19124   (baseline)

── concurrent clients × federation (n=100000, 8 clients × 300 queries) ──
  1 shards × 8 clients × 300 queries   wall=  812.4 ms   qps=    2954
  2 shards × 8 clients × 300 queries   wall=  431.0 ms   qps=    5568
  4 shards × 8 clients × 300 queries   wall=  234.5 ms   qps=   10242

Notes:
  - 'tax' is the direct-QPS / lake-QPS ratio (1.0 = free).
`;

describe("parseBenchmarkOutput (fast mode)", () => {
  const r = parseBenchmarkOutput(FAST_FIXTURE);

  it("extracts the header + config lines", () => {
    expect(r.header).toContain("ruLake benchmark harness");
    expect(r.config).toContain("clustered Gaussian");
  });

  it("captures the single n=5000 block with five rows", () => {
    expect(r.blocks).toHaveLength(1);
    expect(r.blocks[0]!.n).toBe(5000);
    expect(r.blocks[0]!.rows).toHaveLength(5);
  });

  it("parses direct (Haar) row", () => {
    const row = r.blocks[0]!.rows[0]!;
    expect(row.variant).toBe("direct RaBitQ+ (Haar)");
    expect(row.build_ms).toBe(22.0);
    expect(row.qps).toBe(19827);
  });

  it("parses Hadamard row including build_speedup", () => {
    const row = r.blocks[0]!.rows[1]!;
    expect(row.variant).toBe("direct RaBitQ+ (Hadamard)");
    expect(row.build_ms).toBe(6.9);
    expect(row.qps).toBe(21186);
    expect(row.build_speedup).toBe(3.17);
  });

  it("parses ruLake (Fresh) and (Eventual) with tax", () => {
    const fresh = r.blocks[0]!.rows[2]!;
    expect(fresh.variant).toBe("ruLake (Fresh)");
    expect(fresh.prime_ms).toBe(4.5);
    expect(fresh.qps).toBe(19161);
    expect(fresh.tax).toBe(1.03);

    const eventual = r.blocks[0]!.rows[3]!;
    expect(eventual.variant).toBe("ruLake (Eventual 60s)");
    expect(eventual.tax).toBe(1.0);
  });

  it("parses the federated row", () => {
    const fed = r.blocks[0]!.rows[4]!;
    expect(fed.variant).toBe("ruLake federated (2 shards, Eventual)");
    expect(fed.prime_ms).toBe(3.2);
    expect(fed.qps).toBe(26221);
  });

  it("collects notes", () => {
    expect(r.notes.length).toBeGreaterThan(0);
    expect(r.notes.join("\n")).toContain("'tax' is the direct-QPS");
  });

  it("does not include batch/concurrent sections", () => {
    expect(r.batch).toBeUndefined();
    expect(r.concurrent).toBeUndefined();
  });
});

describe("parseBenchmarkOutput (full mode)", () => {
  const r = parseBenchmarkOutput(FULL_FIXTURE);

  it("captures both federation rows in n=5000", () => {
    expect(r.blocks).toHaveLength(1);
    const fedRows = r.blocks[0]!.rows.filter((row) => row.variant.startsWith("ruLake federated"));
    expect(fedRows).toHaveLength(2);
    expect(fedRows[0]!.variant).toBe("ruLake federated (2 shards, Eventual)");
    expect(fedRows[1]!.variant).toBe("ruLake federated (4 shards, Eventual)");
  });

  it("parses the batch section with baseline", () => {
    expect(r.batch).toBeDefined();
    expect(r.batch!.n).toBe(100000);
    expect(r.batch!.rows).toHaveLength(4);
    expect(r.batch!.rows[0]).toEqual({ batch_size: 8, qps: 19898, speedup_vs_per_query: 1.04 });
    expect(r.batch!.per_query_qps).toBe(19124);
  });

  it("parses the concurrent-clients section", () => {
    expect(r.concurrent).toBeDefined();
    expect(r.concurrent).toHaveLength(3);
    expect(r.concurrent![2]).toEqual({
      shards: 4,
      clients: 8,
      queries_per_client: 300,
      wall_ms: 234.5,
      qps: 10242,
    });
  });
});

describe("parseBenchmarkOutput (defensive)", () => {
  it("returns an empty report on empty input", () => {
    const r = parseBenchmarkOutput("");
    expect(r.blocks).toHaveLength(0);
    expect(r.notes).toHaveLength(0);
    expect(r.header).toBeNull();
  });

  it("does not throw on garbage", () => {
    const r = parseBenchmarkOutput("hello\nworld\n── n = 5000 ──\n  total nonsense\n");
    expect(r.blocks).toHaveLength(1);
    expect(r.blocks[0]!.rows).toHaveLength(0);
  });

  it("preserves raw stdout for diagnostics", () => {
    const r = parseBenchmarkOutput(FAST_FIXTURE);
    expect(r.raw).toBe(FAST_FIXTURE);
  });
});
