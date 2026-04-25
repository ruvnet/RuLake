# 03 — subprocess-wrapper

A pure-Python wrapper around the `rulake-demo` Rust binary. There are
no pyo3 / maturin bindings to ruLake — this package instead spawns the
existing benchmark binary as a child process, parses its stdout, and
exposes the numbers as typed Python objects.

If your Python service needs to invoke ruLake operations (capacity
checks, smoke tests, regression sweeps) without taking on the Rust
toolchain in your own repo, this is the lightweight integration model.

## Install

```bash
cd examples/python/03-subprocess-wrapper
python3 -m venv .venv
. .venv/bin/activate
pip install -e .[dev]
```

No runtime dependencies. The wrapper auto-locates the
`rulake-demo` binary in this order:

1. `binary=` argument to `Rulake(...)`
2. `RULAKE_DEMO_BINARY` env var
3. `target/release/rulake-demo` in the parent crate
4. `target/debug/rulake-demo` in the parent crate
5. fallback to `cargo run --release --bin rulake-demo`

If none resolve, you'll get a `RulakeError` with a clear next step.

## Use as a library

```python
from rulake import Rulake

w = Rulake()
print("crate version:", w.version())          # reads parent Cargo.toml
print("binary at:", w.binary_path())

report = w.benchmark(fast=True)                # ~5 sec
for row in report.rows:
    print(f"n={row.n} {row.label} qps={row.qps:.0f} prime={row.prime_ms} tax={row.tax}")

fresh = report.find(5000, "Fresh")
print(f"cache-hit QPS @ 5k: {fresh.qps:.0f} (tax={fresh.tax}×)")
```

## Run the demo

```bash
# uses --fast by default
python demo.py
```

Output is the parsed table plus an ASCII bar chart of QPS-per-row
scaled to the peak. Useful as a five-second smoke test that the whole
Python -> Rust hop is wired correctly.

## Run the formatted CLI

```bash
# parsed table:
python rulake.py
# raw stdout:
python rulake.py --raw
# full sweep (slow — minutes):
python rulake.py --full --timeout 1200
```

## Tests

```bash
# parser + locator tests (fast, no rulake-demo needed):
pytest tests/ -v

# include the end-to-end test that actually spawns rulake-demo:
RULAKE_RUN_END_TO_END=1 pytest tests/ -v
```

The end-to-end test is gated so the default suite is hermetic — CI
without a Rust toolchain still passes.

## Subprocess hygiene

- Wall-clock timeout (`timeout_sec`, default 600s) prevents a stuck
  child from hanging the caller.
- `start_new_session=True` puts the child in its own process group so
  a Ctrl-C / timeout-kill takes the whole tree down, including a
  `cargo run` that may have spawned `rustc` underneath.
- stderr is captured and (on non-zero exit) attached to the
  `RulakeError` so the failure mode is visible.
- Non-zero exit code is converted to `RulakeError` — the wrapper never
  swallows a failed run.

## Why parse stdout instead of bind?

Building real bindings is weeks of work — `pyo3`, `maturin`, ABI
versioning, wheel publishing for the four major platforms. The
benchmark binary already exists, has a stable text format, and has
been the canonical perf reporter since the crate was first published.
Parsing it gives you the same numbers in a Python dataclass for the
cost of a single `subprocess.run` call.

When you outgrow this — i.e. you need to run a query, not just take a
benchmark — the right next step is real bindings, not a richer
subprocess protocol.
