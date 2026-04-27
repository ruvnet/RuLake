// Loader for the platform-specific .node binary (ADR-003 §5).
//
// On `npm install @ruvector/rulake`, exactly one of the optional
// per-platform packages (@ruvector/rulake-linux-x64-gnu, ...) is
// resolved by npm based on the host's `os` / `cpu` / `libc`. For
// local development we also support a sibling `.node` next to this
// file (the layout `napi build` produces in-tree).
//
// This file is shaped to match what `@napi-rs/cli` generates so
// switching to the CLI later is a no-op.

const { existsSync, readFileSync } = require("node:fs");
const { join } = require("node:path");

const { platform, arch } = process;

let nativeBinding = null;
let loadError = null;

function isMusl() {
  // Best-effort musl detection. On non-Linux this never matters.
  if (process.platform !== "linux") return false;
  if (process.report?.getReport) {
    try {
      const { glibcVersionRuntime } = process.report.getReport().header;
      return !glibcVersionRuntime;
    } catch {}
  }
  try {
    const lddPath = require("node:child_process")
      .execSync("which ldd", { stdio: "pipe" })
      .toString()
      .trim();
    return readFileSync(lddPath, "utf8").includes("musl");
  } catch {
    return false;
  }
}

const candidates = [];

switch (platform) {
  case "linux":
    if (arch === "x64") {
      candidates.push(isMusl()
        ? "rulake.linux-x64-musl.node"
        : "rulake.linux-x64-gnu.node");
      candidates.push("@ruvector/rulake-linux-x64-gnu");
    } else if (arch === "arm64") {
      candidates.push(isMusl()
        ? "rulake.linux-arm64-musl.node"
        : "rulake.linux-arm64-gnu.node");
      candidates.push("@ruvector/rulake-linux-arm64-gnu");
    }
    break;
  case "darwin":
    if (arch === "x64") {
      candidates.push("rulake.darwin-x64.node");
      candidates.push("@ruvector/rulake-darwin-x64");
    } else if (arch === "arm64") {
      candidates.push("rulake.darwin-arm64.node");
      candidates.push("@ruvector/rulake-darwin-arm64");
    }
    break;
  case "win32":
    if (arch === "x64") {
      candidates.push("rulake.win32-x64-msvc.node");
      candidates.push("@ruvector/rulake-win32-x64-msvc");
    }
    break;
}

for (const c of candidates) {
  // Local sibling .node files take precedence — supports the dev loop
  // (`napi build` / `cargo build --release && cp`).
  if (c.endsWith(".node")) {
    const p = join(__dirname, c);
    if (existsSync(p)) {
      try {
        nativeBinding = require(p);
        break;
      } catch (e) {
        loadError = e;
      }
    }
  } else {
    try {
      nativeBinding = require(c);
      break;
    } catch (e) {
      loadError = e;
    }
  }
}

if (!nativeBinding) {
  throw new Error(
    `@ruvector/rulake failed to load a native binary for ${platform}-${arch}. ` +
      `Tried: ${candidates.join(", ")}. ` +
      (loadError ? `Last error: ${loadError.message}` : "")
  );
}

module.exports = nativeBinding;
