/**
 * Parser for `rulake-demo` stdout.
 *
 * The Rust binary prints a fixed, human-readable benchmark table per
 * dataset size (n). Sample (--fast):
 *
 *     === ruLake benchmark harness ===
 *     clustered Gaussian, D=128, 100 clusters, rerank×20, Fresh consistency unless noted
 *     queries are warm (not timing first miss unless labeled 'prime')
 *
 *     ── n = 5000 ──────────────────────────────────────────
 *       direct RaBitQ+ (Haar)    build=    22.0 ms   qps=   19827
 *       direct RaBitQ+ (Hadamard) build=     6.9 ms   qps=   21186   build_speedup=3.17×
 *       ruLake (Fresh)           prime=     4.5 ms   qps=   19161   tax=1.03×
 *       ruLake (Eventual 60s)    prime=     3.1 ms   qps=   19897   tax=1.00×
 *       ruLake federated (2 shards, Eventual)  prime=   3.2 ms   qps=  26221
 *
 *     Notes:
 *       ...
 *
 * Be defensive: regex-match each known row, return what you can, keep
 * unknown lines in `raw` for diagnostics. Do not throw on partial output.
 */

export interface RulakeRow {
  variant: string; // e.g. "direct RaBitQ+ (Haar)"
  build_ms?: number; // build= ... ms
  prime_ms?: number; // prime= ... ms
  qps?: number; // qps= ...
  tax?: number; // tax= ...×  (direct/lake ratio)
  build_speedup?: number; // build_speedup= ...×
}

export interface RulakeBlock {
  /** Dataset size for this block. */
  n: number;
  /** All parsed rows under this block, in source order. */
  rows: RulakeRow[];
}

export interface BatchRow {
  batch_size: number; // 8, 32, 128, 300
  qps: number;
  speedup_vs_per_query: number;
}

export interface ConcurrentRow {
  shards: number;
  clients: number;
  queries_per_client: number;
  wall_ms: number;
  qps: number;
}

export interface BenchmarkReport {
  /** Header line containing "=== ruLake benchmark harness ===". */
  header: string | null;
  /** Configuration descriptor line (D, clusters, rerank, consistency). */
  config: string | null;
  /** One block per `── n = N ──` section. */
  blocks: RulakeBlock[];
  /** Optional `search_batch vs per-query loop` breakdown (omitted in --fast). */
  batch?: { n: number; per_query_qps: number | null; rows: BatchRow[] } | undefined;
  /** Optional concurrent-clients × federation breakdown (omitted in --fast). */
  concurrent?: ConcurrentRow[] | undefined;
  /** Trailing notes block — useful as a sanity check. */
  notes: string[];
  /** Original stdout for diagnostics. */
  raw: string;
}

const RX_BLOCK_HEADER = /^── n = (\d+) ──+/;
const RX_NUM = "(\\d+(?:\\.\\d+)?)";

// "  direct RaBitQ+ (Haar)    build=    22.0 ms   qps=   19827"
const RX_DIRECT_HAAR = new RegExp(`^\\s*direct RaBitQ\\+ \\(Haar\\)\\s+build=\\s*${RX_NUM} ms\\s+qps=\\s*${RX_NUM}`);
// "  direct RaBitQ+ (Hadamard) build=     6.9 ms   qps=   21186   build_speedup=3.17×"
const RX_DIRECT_HADA = new RegExp(
  `^\\s*direct RaBitQ\\+ \\(Hadamard\\)\\s+build=\\s*${RX_NUM} ms\\s+qps=\\s*${RX_NUM}\\s+build_speedup=${RX_NUM}×`,
);
// "  ruLake (Fresh)           prime=     4.5 ms   qps=   19161   tax=1.03×"
// "  ruLake (Eventual 60s)    prime=     3.1 ms   qps=   19897   tax=1.00×"
const RX_RULAKE_SINGLE = new RegExp(
  `^\\s*ruLake \\(([^)]+)\\)\\s+prime=\\s*${RX_NUM} ms\\s+qps=\\s*${RX_NUM}\\s+tax=${RX_NUM}×`,
);
// "  ruLake federated (2 shards, Eventual)  prime=   3.2 ms   qps=  26221"
const RX_RULAKE_FED = new RegExp(
  `^\\s*ruLake federated \\((\\d+) shards,\\s*([^)]+)\\)\\s+prime=\\s*${RX_NUM} ms\\s+qps=\\s*${RX_NUM}`,
);

const RX_BATCH_HEADER = /^── search_batch vs per-query loop \(n=([\d_]+)k?\) ──/;
// "  batch=  8   qps=   19898   speedup vs per-query 1.04×"
const RX_BATCH_ROW = new RegExp(
  `^\\s*batch=\\s*(\\d+)\\s+qps=\\s*${RX_NUM}\\s+speedup vs per-query ${RX_NUM}×`,
);
const RX_BATCH_BASELINE = new RegExp(`^\\s*per-query loop\\s+qps=\\s*${RX_NUM}\\s+\\(baseline\\)`);

const RX_CONC_HEADER = /^── concurrent clients × federation \(n=([\d_]+)k?,\s*(\d+) clients × (\d+) queries\) ──/;
// "  4 shards × 8 clients × 300 queries   wall=  234.5 ms   qps=  10242"
const RX_CONC_ROW = new RegExp(
  `^\\s*(\\d+) shards × (\\d+) clients × (\\d+) queries\\s+wall=\\s*${RX_NUM} ms\\s+qps=\\s*${RX_NUM}`,
);

/**
 * Parse the full stdout from `rulake-demo`. Always returns a report
 * (possibly with empty `blocks`); never throws. Unknown rows are
 * preserved in `raw`.
 */
export function parseBenchmarkOutput(stdout: string): BenchmarkReport {
  const lines = stdout.split(/\r?\n/);

  const report: BenchmarkReport = {
    header: null,
    config: null,
    blocks: [],
    notes: [],
    raw: stdout,
  };

  let currentBlock: RulakeBlock | null = null;
  let mode: "preamble" | "blocks" | "batch" | "concurrent" | "notes" = "preamble";
  let batchN: number | null = null;
  let concN: number | null = null;
  let concClients: number | null = null;
  let concQpc: number | null = null;

  for (const raw of lines) {
    const line = raw;

    if (mode === "preamble") {
      if (/^=== ruLake benchmark harness ===/.test(line)) {
        report.header = line;
        continue;
      }
      if (line.startsWith("clustered Gaussian")) {
        report.config = line;
        continue;
      }
    }

    const blockMatch = line.match(RX_BLOCK_HEADER);
    if (blockMatch) {
      currentBlock = { n: Number(blockMatch[1]), rows: [] };
      report.blocks.push(currentBlock);
      mode = "blocks";
      continue;
    }

    if (RX_BATCH_HEADER.test(line)) {
      const m = line.match(RX_BATCH_HEADER)!;
      batchN = Number(m[1]!.replace(/_/g, ""));
      report.batch = { n: batchN, per_query_qps: null, rows: [] };
      mode = "batch";
      continue;
    }

    if (RX_CONC_HEADER.test(line)) {
      const m = line.match(RX_CONC_HEADER)!;
      concN = Number(m[1]!.replace(/_/g, ""));
      concClients = Number(m[2]);
      concQpc = Number(m[3]);
      report.concurrent = [];
      mode = "concurrent";
      continue;
    }

    if (line.startsWith("Notes:")) {
      mode = "notes";
      continue;
    }

    switch (mode) {
      case "blocks": {
        if (!currentBlock) break;
        let m;
        if ((m = line.match(RX_DIRECT_HAAR))) {
          currentBlock.rows.push({
            variant: "direct RaBitQ+ (Haar)",
            build_ms: Number(m[1]),
            qps: Number(m[2]),
          });
        } else if ((m = line.match(RX_DIRECT_HADA))) {
          currentBlock.rows.push({
            variant: "direct RaBitQ+ (Hadamard)",
            build_ms: Number(m[1]),
            qps: Number(m[2]),
            build_speedup: Number(m[3]),
          });
        } else if ((m = line.match(RX_RULAKE_SINGLE))) {
          currentBlock.rows.push({
            variant: `ruLake (${m[1]})`,
            prime_ms: Number(m[2]),
            qps: Number(m[3]),
            tax: Number(m[4]),
          });
        } else if ((m = line.match(RX_RULAKE_FED))) {
          currentBlock.rows.push({
            variant: `ruLake federated (${m[1]} shards, ${m[2]?.trim()})`,
            prime_ms: Number(m[3]),
            qps: Number(m[4]),
          });
        }
        break;
      }
      case "batch": {
        if (!report.batch) break;
        let m;
        if ((m = line.match(RX_BATCH_ROW))) {
          report.batch.rows.push({
            batch_size: Number(m[1]),
            qps: Number(m[2]),
            speedup_vs_per_query: Number(m[3]),
          });
        } else if ((m = line.match(RX_BATCH_BASELINE))) {
          report.batch.per_query_qps = Number(m[1]);
        }
        break;
      }
      case "concurrent": {
        if (!report.concurrent) break;
        const m = line.match(RX_CONC_ROW);
        if (m) {
          report.concurrent.push({
            shards: Number(m[1]),
            clients: Number(m[2]),
            queries_per_client: Number(m[3]),
            wall_ms: Number(m[4]),
            qps: Number(m[5]),
          });
        }
        break;
      }
      case "notes": {
        if (line.trim().length > 0) report.notes.push(line);
        break;
      }
      default:
        break;
    }
  }

  // Suppress unused-warnings for diagnostic-only state we capture.
  void concN;
  void concClients;
  void concQpc;
  void batchN;

  return report;
}
