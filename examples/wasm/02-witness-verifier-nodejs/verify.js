// CLI: node verify.js path/to/table.rulake.json
//
// Loads the wasm-pack `--target nodejs` artifact and prints a structured
// PASS/FAIL plus the field summary.

"use strict";

const fs = require("node:fs");
const path = require("node:path");

const PKG_PATH = path.join(__dirname, "pkg", "rulake_witness_verifier_nodejs.js");

function loadPkg() {
  if (!fs.existsSync(PKG_PATH)) {
    console.error(`error: ${PKG_PATH} not found. Run ./build.sh first.`);
    process.exit(2);
  }
  return require(PKG_PATH);
}

function main(argv) {
  if (argv.length < 1) {
    console.error("usage: node verify.js <path-to-table.rulake.json>");
    process.exit(2);
  }
  const file = argv[0];
  const text = fs.readFileSync(file, "utf8");
  const { verify_bundle_json } = loadPkg();

  let r;
  try {
    r = verify_bundle_json(text);
  } catch (err) {
    console.error(`ERROR: ${err}`);
    process.exit(1);
  }

  const status = r.ok ? "PASS" : "FAIL";
  console.log(`${status}  ${file}`);
  console.log(`  format_version : ${r.fields.format_version}`);
  console.log(`  data_ref       : ${r.fields.data_ref}`);
  console.log(`  dim            : ${r.fields.dim}`);
  console.log(`  rotation_seed  : ${r.fields.rotation_seed}`);
  console.log(`  rerank_factor  : ${r.fields.rerank_factor}`);
  console.log(`  generation     : ${r.fields.generation}`);
  console.log(`  pii_policy     : ${r.fields.pii_policy ?? "—"}`);
  console.log(`  lineage_id     : ${r.fields.lineage_id ?? "—"}`);
  console.log(`  memory_class   : ${r.fields.memory_class ?? "—"}`);
  console.log(`  stored witness : ${r.stored}`);
  console.log(`  computed       : ${r.computed}`);
  if (!r.ok) {
    console.log(`  error          : ${r.error}`);
  }

  process.exit(r.ok ? 0 : 1);
}

main(process.argv.slice(2));
