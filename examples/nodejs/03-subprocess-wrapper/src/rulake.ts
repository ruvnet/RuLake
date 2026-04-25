/**
 * `Rulake` — a thin async wrapper around the Rust `rulake-demo` binary.
 *
 * The Node side does NOT link against ruLake. It spawns the cargo
 * binary as a subprocess, captures stdout, and parses the human-readable
 * benchmark output. This is the path most teams adopt first: ship the
 * Rust binary in a container, talk to it from your existing services.
 *
 * Binary discovery, in order:
 *   1. `RULAKE_DEMO_BIN` env var (absolute path)
 *   2. `target/release/rulake-demo` relative to the repo root
 *   3. fall back to `cargo run --release --bin rulake-demo` (slow on first run)
 *
 * The class is intentionally small: `benchmark()` runs once, returns
 * the parsed report. Long-lived service callers can construct one
 * instance and reuse it; spawning a fresh subprocess per call is fine
 * because the binary itself is a one-shot benchmark.
 */

import { spawn } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";

import { parseBenchmarkOutput, type BenchmarkReport } from "./parser.js";

export interface RulakeOptions {
  /** Path to the rulake-demo binary. Overrides discovery. */
  binary?: string;
  /** Repo root (folder containing Cargo.toml). Used for cargo fallback + version reads. */
  repoRoot?: string;
  /** Optional process env overrides for the spawned binary. */
  env?: NodeJS.ProcessEnv;
}

export interface BenchmarkOptions {
  /** Pass `--fast` (one block, n=5000). Default true — full mode is slow. */
  fast?: boolean;
  /** Hard timeout in milliseconds. Default 600 000 (10 minutes). */
  timeoutMs?: number;
}

export interface SpawnResult {
  stdout: string;
  stderr: string;
  exitCode: number | null;
  signal: NodeJS.Signals | null;
  /** Wall time spent inside the subprocess. */
  durationMs: number;
}

export class RulakeBinaryError extends Error {
  constructor(
    message: string,
    public readonly result: SpawnResult,
  ) {
    super(message);
    this.name = "RulakeBinaryError";
  }
}

export class Rulake {
  /** Resolved path to the binary OR `null` when the cargo fallback is in play. */
  public readonly binary: string | null;
  public readonly repoRoot: string;
  public readonly env: NodeJS.ProcessEnv;

  constructor(opts: RulakeOptions = {}) {
    this.repoRoot = resolve(opts.repoRoot ?? findRepoRoot());
    this.binary = opts.binary ?? discoverBinary(this.repoRoot);
    this.env = { ...process.env, ...opts.env };
  }

  /** Read the Rust crate version from Cargo.toml. */
  version(): string {
    const path = resolve(this.repoRoot, "Cargo.toml");
    try {
      const body = readFileSync(path, "utf8");
      // Tiny grep — avoid pulling in a TOML parser for one field.
      const m = body.match(/^version\s*=\s*"([^"]+)"/m);
      return m?.[1] ?? "unknown";
    } catch {
      return "unknown";
    }
  }

  /** Run `rulake-demo` once, parse its stdout, return the report. */
  async benchmark(opts: BenchmarkOptions = {}): Promise<BenchmarkReport> {
    const fast = opts.fast ?? true;
    const args = fast ? ["--fast"] : [];
    const result = await this.spawnBinary(args, opts.timeoutMs ?? 600_000);
    if (result.exitCode !== 0) {
      throw new RulakeBinaryError(
        `rulake-demo exited with code=${result.exitCode} signal=${result.signal}`,
        result,
      );
    }
    // Parser is defensive — even partial stdout becomes a (possibly empty) report.
    return parseBenchmarkOutput(result.stdout);
  }

  /** Lower-level: just spawn and capture; do not parse. */
  private spawnBinary(args: string[], timeoutMs: number): Promise<SpawnResult> {
    const t0 = process.hrtime.bigint();
    return new Promise<SpawnResult>((resolveResult, reject) => {
      const cmd = this.binary ?? "cargo";
      const fullArgs = this.binary
        ? args
        : ["run", "--release", "--quiet", "--bin", "rulake-demo", "--", ...args];

      const child = spawn(cmd, fullArgs, {
        cwd: this.repoRoot,
        env: this.env,
        stdio: ["ignore", "pipe", "pipe"],
      });

      let stdout = "";
      let stderr = "";
      child.stdout!.setEncoding("utf8");
      child.stderr!.setEncoding("utf8");
      child.stdout!.on("data", (chunk: string) => {
        stdout += chunk;
      });
      child.stderr!.on("data", (chunk: string) => {
        stderr += chunk;
      });

      const timer = setTimeout(() => {
        child.kill("SIGKILL");
      }, timeoutMs).unref();

      child.on("error", (err) => {
        clearTimeout(timer);
        reject(err);
      });

      child.on("close", (code, signal) => {
        clearTimeout(timer);
        const durationMs = Number((process.hrtime.bigint() - t0) / 1_000_000n);
        resolveResult({ stdout, stderr, exitCode: code, signal, durationMs });
      });
    });
  }
}

/** Walk up from `import.meta.url` looking for `Cargo.toml`. */
export function findRepoRoot(startDir?: string): string {
  let dir = startDir ? resolve(startDir) : resolve(dirname(new URL(import.meta.url).pathname), "..");
  for (let i = 0; i < 12; i++) {
    if (existsSync(resolve(dir, "Cargo.toml"))) {
      return dir;
    }
    const parent = dirname(dir);
    if (parent === dir) break;
    dir = parent;
  }
  // Fall back to the start dir; downstream code will surface a clearer error.
  return startDir ? resolve(startDir) : process.cwd();
}

/** Locate `target/release/rulake-demo` next to Cargo.toml; null if missing. */
export function discoverBinary(repoRoot: string): string | null {
  const env = process.env.RULAKE_DEMO_BIN;
  if (env && existsSync(env)) return env;
  const candidate = resolve(repoRoot, "target/release/rulake-demo");
  return existsSync(candidate) ? candidate : null;
}
