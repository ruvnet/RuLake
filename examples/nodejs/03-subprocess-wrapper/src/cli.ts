#!/usr/bin/env tsx
/**
 * CLI driver for the subprocess wrapper.
 *
 * Usage:
 *   tsx src/cli.ts benchmark [--fast]   (default: --fast)
 *   tsx src/cli.ts version
 *
 * Pretty-prints the parsed benchmark output as a table.
 */

import { Rulake, RulakeBinaryError } from "./rulake.js";

function pad(s: string | number, n: number): string {
  const str = String(s);
  return str.length >= n ? str : " ".repeat(n - str.length) + str;
}

async function main(argv: string[]): Promise<void> {
  const args = argv.slice(2);
  const cmd = args[0] ?? "benchmark";

  if (cmd === "version") {
    const r = new Rulake();
    process.stdout.write(`ruvector-rulake ${r.version()}\n`);
    process.stdout.write(`binary: ${r.binary ?? "(cargo fallback)"}\n`);
    return;
  }

  if (cmd === "benchmark") {
    const fast = !args.includes("--full"); // default to fast
    const r = new Rulake();
    process.stdout.write(`ruLake ${r.version()} — running ${fast ? "--fast" : "full"} benchmark\n`);
    process.stdout.write(`binary: ${r.binary ?? "(cargo fallback — slower)"}\n\n`);

    let report;
    try {
      report = await r.benchmark({ fast });
    } catch (err) {
      if (err instanceof RulakeBinaryError) {
        process.stderr.write(`benchmark failed: ${err.message}\n`);
        if (err.result.stderr) process.stderr.write(`stderr:\n${err.result.stderr}\n`);
        process.exit(1);
      }
      throw err;
    }

    if (report.config) process.stdout.write(`${report.config}\n\n`);
    for (const block of report.blocks) {
      process.stdout.write(`── n = ${block.n} ──\n`);
      process.stdout.write(
        `  ${pad("variant", 38)}  ${pad("build_ms", 9)}  ${pad("prime_ms", 9)}  ${pad("qps", 8)}  ${pad("tax", 6)}  ${pad("speedup", 8)}\n`,
      );
      for (const row of block.rows) {
        process.stdout.write(
          `  ${pad(row.variant, 38)}  ${pad(row.build_ms?.toFixed(1) ?? "-", 9)}  ${pad(row.prime_ms?.toFixed(1) ?? "-", 9)}  ${pad(row.qps ?? "-", 8)}  ${pad(row.tax?.toFixed(2) ?? "-", 6)}  ${pad(row.build_speedup?.toFixed(2) ?? "-", 8)}\n`,
        );
      }
      process.stdout.write("\n");
    }

    if (report.batch) {
      process.stdout.write(`── batch (n=${report.batch.n}) ──\n`);
      for (const row of report.batch.rows) {
        process.stdout.write(
          `  batch=${pad(row.batch_size, 4)}  qps=${pad(row.qps, 8)}  speedup×${row.speedup_vs_per_query.toFixed(2)}\n`,
        );
      }
      process.stdout.write(`  per-query-loop qps=${report.batch.per_query_qps ?? "-"}\n\n`);
    }

    if (report.concurrent) {
      process.stdout.write(`── concurrent ──\n`);
      for (const row of report.concurrent) {
        process.stdout.write(
          `  shards=${row.shards} × clients=${row.clients} × queries=${row.queries_per_client}  wall=${row.wall_ms.toFixed(1)} ms  qps=${row.qps}\n`,
        );
      }
      process.stdout.write("\n");
    }
    return;
  }

  process.stderr.write(`unknown command: ${cmd}\n`);
  process.stderr.write(`usage: tsx src/cli.ts [version | benchmark [--fast | --full]]\n`);
  process.exit(2);
}

main(process.argv).catch((err) => {
  process.stderr.write(`${(err as Error).stack ?? String(err)}\n`);
  process.exit(1);
});
