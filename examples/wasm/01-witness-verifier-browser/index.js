// Browser entrypoint for the ruLake witness verifier.
//
// Loads the wasm-pack `--target web` ES-module output, then wires up:
//   - drop-zone + file picker
//   - "Try sample" / "Tamper sample" buttons
//   - rendering of fields + witness comparison

import init, { verify_bundle_json } from "./pkg/rulake_witness_verifier_browser.js";

const status = document.getElementById("wasm-status");

let wasmReady = false;
init()
  .then(() => {
    wasmReady = true;
    status.textContent = "wasm ready";
    status.classList.add("ok");
  })
  .catch((err) => {
    status.textContent = `wasm load failed: ${err}`;
    status.classList.add("err");
  });

const dropZone = document.getElementById("drop-zone");
const fileInput = document.getElementById("file-input");

dropZone.addEventListener("click", () => fileInput.click());
dropZone.addEventListener("keydown", (ev) => {
  if (ev.key === "Enter" || ev.key === " ") {
    ev.preventDefault();
    fileInput.click();
  }
});

["dragover", "dragenter"].forEach((evt) => {
  dropZone.addEventListener(evt, (ev) => {
    ev.preventDefault();
    dropZone.classList.add("hover");
  });
});
["dragleave", "drop"].forEach((evt) => {
  dropZone.addEventListener(evt, (ev) => {
    ev.preventDefault();
    dropZone.classList.remove("hover");
  });
});

dropZone.addEventListener("drop", async (ev) => {
  const file = ev.dataTransfer?.files?.[0];
  if (!file) return;
  await readAndVerify(file);
});

fileInput.addEventListener("change", async () => {
  const file = fileInput.files?.[0];
  if (!file) return;
  await readAndVerify(file);
});

document.getElementById("sample-btn").addEventListener("click", async () => {
  const text = await fetchSample();
  runVerify(text);
});

document.getElementById("tamper-btn").addEventListener("click", async () => {
  let text = await fetchSample();
  // Flip the dim from 8 -> 16 without recomputing the witness.
  text = text.replace(/"dim"\s*:\s*8/, '"dim": 16');
  runVerify(text);
});

async function fetchSample() {
  const r = await fetch("./fixtures/known-good-bundle.json");
  if (!r.ok) throw new Error(`sample fetch failed: ${r.status}`);
  return r.text();
}

async function readAndVerify(file) {
  // Defensive cap mirroring the wasm-side 64 KiB limit.
  if (file.size > 64 * 1024) {
    showError(`file too large: ${file.size} bytes (max 64 KiB)`);
    return;
  }
  const text = await file.text();
  runVerify(text);
}

function runVerify(text) {
  if (!wasmReady) {
    showError("wasm not loaded yet, retry in a moment");
    return;
  }
  let result;
  try {
    result = verify_bundle_json(text);
  } catch (err) {
    showError(String(err));
    return;
  }
  render(result);
}

function render(r) {
  const result = document.getElementById("result");
  const banner = document.getElementById("banner");
  const errLine = document.getElementById("error-line");
  result.classList.remove("hidden");

  banner.classList.remove("ok", "fail");
  if (r.ok) {
    banner.textContent = "PASS — witness matches";
    banner.classList.add("ok");
    errLine.classList.add("hidden");
  } else {
    banner.textContent = "FAIL — witness mismatch";
    banner.classList.add("fail");
    errLine.textContent = r.error || "witness mismatch";
    errLine.classList.remove("hidden");
  }

  const f = r.fields;
  document.getElementById("f-format").textContent = f.format_version;
  document.getElementById("f-dref").textContent = f.data_ref;
  document.getElementById("f-dim").textContent = f.dim;
  document.getElementById("f-seed").textContent = f.rotation_seed;
  document.getElementById("f-rerank").textContent = f.rerank_factor;
  document.getElementById("f-gen").textContent = f.generation;
  document.getElementById("f-pii").textContent = f.pii_policy ?? "—";
  document.getElementById("f-lin").textContent = f.lineage_id ?? "—";
  document.getElementById("f-mc").textContent = f.memory_class ?? "—";

  document.getElementById("w-stored").textContent = r.stored || "—";
  document.getElementById("w-computed").textContent = r.computed || "—";
  const matchCell = document.getElementById("w-match");
  if (r.ok) {
    matchCell.textContent = "byte-for-byte equal";
    matchCell.className = "ok";
  } else {
    matchCell.textContent = "DIFFER";
    matchCell.className = "fail";
  }
}

function showError(msg) {
  const result = document.getElementById("result");
  const banner = document.getElementById("banner");
  const errLine = document.getElementById("error-line");
  result.classList.remove("hidden");
  banner.classList.remove("ok");
  banner.classList.add("fail");
  banner.textContent = "ERROR";
  errLine.textContent = msg;
  errLine.classList.remove("hidden");
}
