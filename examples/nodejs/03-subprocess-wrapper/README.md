# 03 — subprocess-wrapper

TypeScript class that wraps the Rust `rulake-demo` binary as a
subprocess, parses its stdout, and returns structured results. This is
the simplest interop story for teams that aren't ready to write
language bindings: ship the Rust binary in your container, talk to it
from Node.

## Install

```bash
cd examples/nodejs/03-subprocess-wrapper
npm install
```

The Rust binary is found via, in order:

1. `RULAKE_DEMO_BIN` env var (absolute path)
2. `target/release/rulake-demo` relative to the repo root
3. `cargo run --release --bin rulake-demo` (slow on first run)

So you usually want to build it once first:

```bash
cd /path/to/RuLake
cargo build --release --bin rulake-demo
```

## Run

```bash
# Quick (one block, n=5000):
npx tsx src/cli.ts benchmark --fast
# Full sweep (n = 5k / 50k / 100k + batch + concurrent):
npx tsx src/cli.ts benchmark --full
# Print the underlying crate version:
npx tsx src/cli.ts version
```

Sample output (`--fast`):

```
ruLake 2.2.0 — running --fast benchmark
binary: /path/to/RuLake/target/release/rulake-demo

clustered Gaussian, D=128, 100 clusters, rerank×20, Fresh consistency unless noted

── n = 5000 ──
                                 variant   build_ms   prime_ms       qps     tax   speedup
                  direct RaBitQ+ (Haar)       22.0          -     19827       -        -
              direct RaBitQ+ (Hadamard)        6.9          -     21186       -     3.17
                          ruLake (Fresh)         -        4.5     19161    1.03        -
                  ruLake (Eventual 60s)         -        3.1     19897    1.00        -
   ruLake federated (2 shards, Eventual)         -        3.2     26221       -        -
```

## Library use

```ts
import { Rulake } from "./src/rulake.js";

const r = new Rulake();
const report = await r.benchmark({ fast: true });
for (const block of report.blocks) {
  for (const row of block.rows) {
    console.log(block.n, row.variant, row.qps);
  }
}
```

## Test

```bash
npm test
```

The parser tests use captured `rulake-demo` stdout fixtures and check
both the `--fast` and full-mode shapes plus defensive behaviour
(empty input, garbage input). They do NOT spawn the Rust binary, so
they run in milliseconds and don't require a build.

## Design notes

- The class is intentionally one-shot per call. `rulake-demo` is a
  one-shot benchmark; spawning a fresh subprocess per call is fine.
- The parser never throws. Unknown rows are dropped, partial output
  yields a partial report, and the raw stdout is preserved on the
  `raw` field for diagnostics.
- The wall-clock cost lives almost entirely in the spawned subprocess
  (the parser is a few µs per line). For a long-running benchmark,
  subscribe to `stdout` directly via the lower-level child_process
  spawn — the `parseBenchmarkOutput` helper takes the full string at
  the end.
