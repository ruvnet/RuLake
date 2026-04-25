#!/usr/bin/env tsx
/**
 * Verify the witness in a ruLake snapshot or publish directory.
 *
 * Usage:
 *   tsx src/cli.ts <dir>
 *
 * Where `<dir>` contains a `table.rulake.json` written by the Rust
 * crate (e.g. `examples/sidecar_daemon` or `save_cache_to_dir`).
 *
 * Exit codes:
 *    0  witness matches  (prints "MATCH")
 *    1  witness mismatch (prints "MISMATCH" + expected vs actual)
 *    2  bundle missing or malformed
 */

import { readFileSync, statSync } from "node:fs";
import { join, resolve } from "node:path";

import { parseBundle } from "./bundle.js";
import { verifyBundle } from "./witness.js";

function fail(code: number, msg: string): never {
  process.stderr.write(`${msg}\n`);
  process.exit(code);
}

function main(argv: string[]): void {
  const args = argv.slice(2);
  if (args.length !== 1) {
    fail(2, "usage: tsx src/cli.ts <dir-containing-table.rulake.json>");
  }

  const dir = resolve(args[0]!);
  const bundlePath = join(dir, "table.rulake.json");

  let st;
  try {
    st = statSync(bundlePath);
  } catch (err) {
    fail(2, `bundle missing: ${bundlePath} (${(err as Error).message})`);
  }
  if (!st.isFile()) {
    fail(2, `bundle path is not a regular file: ${bundlePath}`);
  }

  let raw: string;
  try {
    raw = readFileSync(bundlePath, "utf8");
  } catch (err) {
    fail(2, `cannot read bundle: ${(err as Error).message}`);
  }

  let bundle;
  try {
    bundle = parseBundle(raw);
  } catch (err) {
    fail(2, `cannot parse bundle: ${(err as Error).message}`);
  }

  const { ok, expected, actual } = verifyBundle(bundle);

  process.stdout.write(`bundle: ${bundlePath}\n`);
  process.stdout.write(`format_version: ${bundle.format_version}\n`);
  process.stdout.write(`data_ref:       ${bundle.data_ref}\n`);
  process.stdout.write(`dim:            ${bundle.dim}\n`);
  process.stdout.write(`rotation_seed:  ${bundle.rotation_seed.toString()}\n`);
  process.stdout.write(`rerank_factor:  ${bundle.rerank_factor}\n`);
  const genStr =
    bundle.generation.kind === "Num"
      ? `Num(${bundle.generation.value.toString()})`
      : `Opaque(${JSON.stringify(bundle.generation.value)})`;
  process.stdout.write(`generation:     ${genStr}\n`);
  process.stdout.write(`witness:        ${actual}\n`);

  if (ok) {
    process.stdout.write(`\nMATCH — recomputed SHAKE-256(32) agrees with on-disk rvf_witness.\n`);
    process.exit(0);
  } else {
    process.stdout.write(`\nMISMATCH\n  expected: ${expected}\n  actual:   ${actual}\n`);
    process.exit(1);
  }
}

main(process.argv);
