# ADR-001: Standalone repo strategy — vendored submodule, no root workspace

## Status

**Accepted (2026-04-25)** — implemented in commit that introduces this ADR.
Supersedes the implicit "lives inside the RuVector workspace" assumption that
the source carried over from `ruvnet/RuVector/crates/rulake`.

## Context

ruLake was extracted from the [`ruvnet/RuVector`](https://github.com/ruvnet/RuVector)
monorepo into [`ruvnet/RuLake`](https://github.com/ruvnet/RuLake) as the new
primary repo. The crate's `Cargo.toml` as imported still depended on the
parent workspace for almost everything that matters at build time:

```toml
version.workspace = true
edition.workspace = true
license.workspace = true
authors.workspace = true
repository.workspace = true
serde       = { workspace = true }
thiserror   = { workspace = true }
rand        = { workspace = true }
rand_distr  = { workspace = true }
rayon       = { workspace = true }
ruvector-rabitq = { path = "../ruvector-rabitq", version = "2.2" }
```

A fresh clone of just `RuLake` therefore would not build — `cargo build`
fails on the first `*.workspace = true` lookup, and the `path = "../ruvector-rabitq"`
relative dep walks out of the repo. The benchmark numbers in `BENCHMARK.md`
and the test claims in `README.md` were likewise unreachable from this repo
in isolation.

The deep review (`docs/review/capabilities.md`) called this out as the
single biggest gap for a new reader.

## Decision

We adopt three coupled decisions to make the standalone repo `cargo build` /
`cargo test` cleanly while keeping the rabitq kernel a single source of truth.

### 1. Vendor the upstream RuVector workspace as a git submodule

`vendor/ruvector` is a submodule pinned to a specific revision of
`https://github.com/ruvnet/RuVector.git`. This is what makes
`ruvector-rabitq` available to ruLake without re-publishing it on
`crates.io` and without copy-pasting kernel code that the upstream team
continues to evolve.

```toml
ruvector-rabitq = { path = "vendor/ruvector/crates/ruvector-rabitq" }
```

Why submodule (and not the alternatives we considered):

| option | rejected because |
|---|---|
| `crates.io` git dependency (`git = "..."`) | Pulls into Cargo home, not into the repo. CI / Docker / `cargo vendor` flows have to re-fetch on every cold cache. We want the source physically present. |
| Vendored copy under `crates/ruvector-rabitq/` | Drift problem. Upstream rabitq is the same code; copying it forks two histories and we'd silently miss security and perf fixes. |
| `cargo vendor` of all transitive deps | Heavyweight; redundant with `Cargo.lock` for reproducibility. Doesn't help with the workspace-inheritance issue at all. |
| Full RuVector clone, sparse-checkout to rabitq only | Git submodules don't support sparse checkout cleanly across versions. Not worth the operational complexity. |

The cost is repo size (≈10k files in `vendor/ruvector` even though we only
build one of them). We accept this — it makes "clone, run, see numbers"
work in a single command, which is the primary onboarding path.

### 2. Do **not** declare a `[workspace]` at the repo root

This is the non-obvious part. Cargo's workspace resolver, when it sees both
our root manifest and the submodule's `vendor/ruvector/Cargo.toml` declaring
`[workspace]`, tries to claim `vendor/ruvector/crates/ruvector-rabitq` as a
member of *our* workspace because we depend on it via `path = …`. It then
fails to satisfy `version.workspace = true` for rabitq because our root has
no `[workspace.package]` section that matches what rabitq inherits.

We tried two arrangements before settling on this one:

- **A.** Root has `[workspace] members = ["."], exclude = ["vendor"]` plus a
  `[workspace.package]` mirroring upstream's values. **Failed** — cargo still
  claimed rabitq because the path-dep target sits under a directory listed
  in `exclude`. The `exclude` field only suppresses *member discovery*, not
  *path-dep absorption*.
- **B.** Root has `[workspace] members = [".", "vendor/ruvector/crates/ruvector-rabitq"]`
  with mirrored `[workspace.package]`. **Worked but brittle** — every time
  the submodule moves, our `[workspace.package]` and `[workspace.dependencies]`
  must be re-checked against rabitq's inheritance keys. One mismatch and the
  whole workspace fails to load.

The **chosen** arrangement: root has only `[package]`, no `[workspace]`. When
cargo loads the rabitq path-dep, it walks up from
`vendor/ruvector/crates/ruvector-rabitq/` and finds
`vendor/ruvector/Cargo.toml`'s `[workspace]` independently. Rabitq inherits
from upstream's workspace; ruLake uses concrete pins.

`cargo run -p rulake …` still works because cargo accepts `-p NAME`
in a single-package context if `NAME` matches the current package.

### 3. Pin every dependency concretely in the root `Cargo.toml`

No more `*.workspace = true` for ruLake. Versions pinned to match what
upstream RuVector uses today, so we don't double-compile (e.g. two different
`serde` versions if rabitq pulls one and ruLake pulls another):

```toml
serde       = { version = "1.0", features = ["derive"] }
serde_json  = "1.0"
thiserror   = "2.0"
sha3        = "0.10"
hex         = "0.4"
rand        = "0.8"
rand_distr  = "0.4"
rayon       = "1.10"
```

Package-level metadata (`version`, `edition`, `license`, `authors`,
`repository`) is also concrete and points at this repo, not the upstream.

## Consequences

### Positive

- `git clone --recurse-submodules && cargo build` works on a fresh machine.
- `cargo test --release` runs the full 43-test suite (21 unit + 22 integration)
  the way `BENCHMARK.md` and the substrate-acceptance loop require.
- `Dockerfile` and `install.sh` (added in the same commit) become trivial —
  no special toolchain prep beyond a Rust 1.77+ compiler.
- ruLake's metadata (license, authors, repo URL) is now first-class, not
  inherited from a foreign workspace.

### Negative / accepted

- **Repo footprint**: cloning RuLake now drags the full RuVector tree into
  `vendor/ruvector/`. We accept this in exchange for "one command to build".
  Future option: rewrite to a sparse-checkout-friendly layout once the rabitq
  crate splits out of the monorepo upstream (tracked, not blocking).
- **Submodule pin maintenance**: bumping rabitq requires
  `git submodule update --remote vendor/ruvector` plus a manual
  `cargo build` + `cargo test` to confirm nothing regressed. Documented in
  `install.sh` and the README's build section.
- **No root workspace** means tooling that *insists* on a workspace (some
  cargo plugins, certain IDE integrations) sees a bare package. Acceptable —
  none of the standard cargo subcommands (`build`, `test`, `run`, `bench`,
  `publish`, `doc`) require it.

### Verification (run on the same commit that introduces this ADR)

```text
$ cargo build
   ...
   Compiling ruvector-rabitq v2.2.0 (vendor/ruvector/crates/ruvector-rabitq)
   Compiling rulake v2.2.0 (.)
    Finished `dev` profile in 3.62s

$ cargo test --release
   ...
   21 passed; 0 failed   (unit)
   22 passed; 0 failed   (federation_smoke integration)
   Doc-tests rulake: 0 passed; 0 failed

$ cargo run --release --bin rulake-demo -- --fast
   ruLake (Fresh)        prime= 4.4 ms   qps= 19161   tax=1.00×
   ruLake (Eventual)     prime= 3.2 ms   qps= 19673   tax=0.97×
```

The 1.00× tax matches the `≈1.02× raw library speed` claim on the README's
hero block within measurement noise on a non-Ryzen machine.

## References

- Upstream rabitq source: `vendor/ruvector/crates/ruvector-rabitq/`
- Upstream RuVector workspace: `vendor/ruvector/Cargo.toml`
- Capability review: [`docs/review/capabilities.md`](../review/capabilities.md)
- ADRs governing the design surface itself: [ADR-155](ADR-155-rulake-datalake-layer.md)
  (cache-first), [ADR-156](ADR-156-rulake-as-memory-substrate.md) (substrate),
  [ADR-157](ADR-157-optional-accelerator-plane.md) (kernel plane),
  [ADR-158](ADR-158-optional-rotation-and-qvcache-positioning.md) (rotation).
