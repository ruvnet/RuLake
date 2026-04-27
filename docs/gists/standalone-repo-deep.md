# ruLake standalone-repo strategy — A Deep Introduction

## TL;DR

ADR-001 makes a single, load-bearing commitment about how ruLake is built: it lives in its own repository, depends on the upstream RaBitQ kernel through a pinned git submodule at `vendor/ruvector/`, and deliberately declines to declare a Cargo `[workspace]` at the root. The result is that `git clone --recurse-submodules && cargo build` produces a working binary on a fresh machine, every dependency version is concrete (no `*.workspace = true` inheritance from a foreign tree), and the upstream kernel stays one source of truth without forking. The decision is structural rather than glamorous, but every other ADR — Python SDK, Node SDK, MCP server, IPFS backend, the substrate ADRs — quietly assumes it.

## Introduction

ruLake started life as a crate inside [`ruvnet/RuVector`](https://github.com/ruvnet/RuVector), the RaBitQ-centred research monorepo. Its first `Cargo.toml` carried over the workspace's habits — `version.workspace = true`, `serde = { workspace = true }`, `ruvector-rabitq = { path = "../ruvector-rabitq", version = "2.2" }` — because that is what was idiomatic in the parent tree. When the project was split into [`ruvnet/RuLake`](https://github.com/ruvnet/RuLake) as a standalone repository, the manifest came along unchanged. A fresh clone of just RuLake therefore did not build. Cargo would fail on the very first workspace-inheritance lookup, and the relative path `../ruvector-rabitq` walked out of the repo entirely. ADR-001 (`docs/adrs/ADR-001-standalone-repo-strategy.md:13`-area) records this state plainly: the benchmark numbers in `BENCHMARK.md` and the test claims in `README.md` were unreachable from the new repo in isolation, and the deep capability review (`docs/review/capabilities.md`) called this out as the single biggest gap for a new reader.

The pressure to fix it cleanly came from three directions. The first was distribution: every downstream piece — the Python SDK that ADR-002 introduces, the Node SDK from ADR-003, the MCP server from ADR-004, the IPFS backend from ADR-005, the GCS backend, the substrate scaffolds for rvDNA and ruQu — depends on `rulake = { path = ".." }` and ultimately on `ruvector-rabitq` underneath it. If the parent crate cannot build standalone, nothing else can either, and the whole sibling-crate pattern collapses. The second pressure was reproducibility. Operators expect a single command (`cargo build`, or `pip install`, or `npm install`) to produce a working artefact; `cargo vendor`-style heroics or copy-paste-and-pray patches against `~/.cargo/config.toml` are user-hostile. The third was correctness drift. RaBitQ is the load-bearing kernel — capability #11 of the project is "RaBitQ + Haar rotation + AVX-512 popcnt at 1.02× direct-library tax". A copy of the kernel pinned in tree, separate from upstream, would silently miss security and performance fixes the upstream team continues to ship. None of these was novel; all three are the standard hazards a research-extracted project has to navigate.

Why answer all three together, in one ADR, rather than incrementally? Because the three answers are coupled. Vendor-the-submodule answers reproducibility but creates a workspace-discovery problem. Pin-versions-concretely answers the inheritance problem but only matters once the rabitq path-dep resolves. Refuse the root workspace answers the discovery problem but only works because the manifests are pinned and the submodule is present. ADR-001 ratifies the trio as one coupled commitment so future contributors do not unpick one side and silently break the other two.

The deeper reason this matters for a vector-cache project specifically is that ruLake's whole pitch (per ADR-155 and `BENCHMARK.md`) is "1.02× tax over the raw library". Any build complication that makes operators nervous about whether they are running the *same* RaBitQ as the benchmark sheet erodes the claim. The standalone-repo strategy is not a build-system curiosity; it is what lets the headline performance number be re-derivable on a contributor's laptop ten minutes after `git clone`.

## The decision in detail

ADR-001 commits to three coupled decisions. None of them is novel in isolation; the load-bearing choice is the *combination*.

The first decision is to vendor upstream RuVector as a git submodule at `vendor/ruvector/`, pinned to a specific revision of `https://github.com/ruvnet/RuVector.git`. The path-dep in the root `Cargo.toml` becomes `ruvector-rabitq = { path = "vendor/ruvector/crates/ruvector-rabitq" }`. The ADR records four alternatives that were considered and rejected. A `git = "..."` Cargo dependency was rejected because it pulls into the user's Cargo home rather than being physically present in the tree, which makes CI / Docker / `cargo vendor` flows re-fetch on every cold cache. A vendored-and-copied rabitq under `crates/ruvector-rabitq/` was rejected because two histories will drift; security and perf fixes upstream would silently pass us by. A `cargo vendor` dump of all transitive deps was rejected as heavyweight and redundant with `Cargo.lock`. A sparse-checkout-only RuVector clone was rejected because git submodules do not support sparse checkout cleanly across versions. The cost of the chosen path is repo footprint — `vendor/ruvector/` carries roughly ten thousand files even though we build exactly one of them — and the ADR accepts this in exchange for the "clone, run, see numbers" property.

The second decision is the non-obvious one: do *not* declare a `[workspace]` at the repo root. Cargo's resolver, when it sees both our root manifest and the submodule's `vendor/ruvector/Cargo.toml` declaring `[workspace]`, will try to claim `vendor/ruvector/crates/ruvector-rabitq` as a member of *our* workspace because we depend on it via a path. It then fails to satisfy `version.workspace = true` for rabitq because our root has no matching `[workspace.package]` section. The ADR documents two arrangements that were tried and abandoned. One used `members = ["."]` and `exclude = ["vendor"]` — this failed because `exclude` only suppresses *member discovery*, not *path-dep absorption*. The other listed rabitq explicitly under `members` and mirrored upstream's `[workspace.package]` — this worked but every submodule bump risked an inheritance-key mismatch. The chosen arrangement is to have only `[package]` at the root, and let cargo discover rabitq's workspace independently when it walks up from the submodule path.

The third decision is to pin every dependency concretely. No more `*.workspace = true` for ruLake. Versions match what upstream RuVector uses today so the build does not double-compile (e.g. two `serde` versions if rabitq pulls one and ruLake pulls another):

| concern | what it pins | why concrete vs inherited |
|---|---|---|
| package metadata | `version`, `edition`, `license`, `authors`, `repository` | This repo is the canonical source now; metadata pointing at upstream RuVector would lie. |
| serialization | `serde = "1.0"`, `serde_json = "1.0"`, `thiserror = "2.0"` | Match upstream's resolved versions to avoid duplicate compiles. |
| crypto / random | `sha3 = "0.10"`, `hex = "0.4"`, `rand = "0.8"`, `rand_distr = "0.4"` | SHAKE-256 (witness) and Haar rotation depend on these; one version of each kernel-affecting crate. |
| parallelism | `rayon = "1.10"` | Federation fan-out (`crates/core/src/lake.rs:521`) walks `(backend, collection)` in parallel; a rayon major bump would be a behaviour change. |

`cargo run -p rulake …` still works because cargo accepts `-p NAME` in a single-package context if `NAME` matches the current package — so existing tooling that uses `-p` does not break.

## Capabilities

What ADR-001 unlocks is mostly invisible — a working build — but the downstream consequences are concrete.

`cargo build` from a fresh `git clone --recurse-submodules` produces `ruvector-rabitq v2.2.0` and `rulake v2.2.0`, in that order, in a few seconds (`docs/adrs/ADR-001-standalone-repo-strategy.md:152`-area records 3.62 s on the reference machine). `cargo test --release` runs the full test suite — 21 unit + 22 integration in the original ADR write-up, growing as substrates and SDKs added their own — without any environment preparation beyond a Rust 1.77+ toolchain. `cargo run --release --bin rulake-demo -- --fast` reproduces the benchmark numbers from `BENCHMARK.md`: the `Fresh` path lands at 4.4 ms prime / 19,161 QPS / 1.00× tax on a non-Ryzen reference machine, which matches the README's hero block within measurement noise.

The sibling-crate pattern is the bigger payoff. Every other ADR points at the same shape: `python/Cargo.toml` (ADR-002) declares `rulake = { path = "..", version = "2.2.0" }`; `node/Cargo.toml` (ADR-003) does the same; `crates/mcp-server/Cargo.toml` (ADR-004) is `ruvector-rulake-mcp` and does the same; the substrate crates `crates/ipfs-backend/`, `crates/gcs-backend/`, `crates/rvdna-backend/`, `crates/ruqu-backend/` all do the same. None of them declares a workspace. None of them re-vendors rabitq. They all reach the kernel through ruLake's transitive dep on `vendor/ruvector/crates/ruvector-rabitq`. A new sibling crate is one `Cargo.toml` away — copy the pattern, build, ship.

A worked example. Suppose an operator wants to build the MCP server binary on a CI runner that has only `rustc` and `git`. The flow is: `git clone --recurse-submodules https://github.com/ruvnet/RuLake.git`, then `cargo build --release -p ruvector-rulake-mcp`. Cargo finds `crates/mcp-server/Cargo.toml`, sees its `rulake = { path = ".." }` dep, walks up into `RuLake/`, finds `Cargo.toml` with `ruvector-rabitq = { path = "vendor/ruvector/crates/ruvector-rabitq" }`, walks down into the submodule, finds the rabitq manifest, walks up *its* tree to `vendor/ruvector/Cargo.toml` (the upstream workspace), and inherits rabitq's metadata from there. No workspace-inheritance failure on our root, because there is no root workspace. Three crates compile, the MCP binary lands in `target/release/rulake-mcp`, the operator distributes it. The same flow works for the Python wheel build (`maturin develop --release` from inside `python/`), the Node native module build (`npx @napi-rs/cli build --release` from inside `node/`), and any future sibling.

## Trust & correctness contract — no implicit workspace coupling

The "trust contract" of ADR-001 is structural rather than cryptographic. Three properties have to hold for the build to be correct, and the manifests pin them.

The first is *no inheritance from a foreign workspace*. The root `Cargo.toml` has no `*.workspace = true` lines. Every package-level field — `version`, `edition`, `license`, `authors`, `repository`, `description` — is concrete and points at this repo. Every dependency line carries a literal version string. The check is mechanical: `grep -n 'workspace = true' Cargo.toml` should return zero hits in the root. If a future contributor reintroduces an inherited field, the build still works *while the submodule is present and pointing at a compatible upstream*, but breaks the moment the upstream `[workspace.package]` field set drifts. ADR-001 documents the prior failure modes (§Decision 2 arrangements A and B, `docs/adrs/ADR-001-standalone-repo-strategy.md:80`-area) so the trap is visible.

The second is *the submodule must be physically present* at build time. The `vendor/ruvector/` path-dep is checked at every `cargo build`; without the submodule contents, cargo fails on `unable to find crate ruvector-rabitq` long before any compilation happens. The ADR documents this in §Consequences ("Submodule pin maintenance"), and the SDK ADRs reinforce it. ADR-002 §Negative carries the constraint over to maturin's sdist build and lists a CI gate that asserts a fresh-container install of the sdist produces a working wheel. ADR-003 §Negative does the same for npm and ships a `prepublishOnly` script (`node/package.json:86`) that exits non-zero if `vendor/ruvector/Cargo.toml` is missing — the literal check `if (!require('fs').existsSync('../vendor/ruvector/Cargo.toml'))` runs before publish and prints an instructional error citing ADR-001.

The third is *no double-compilation of kernel-adjacent crates*. The pinned versions for `serde`, `serde_json`, `thiserror`, `sha3`, `hex`, `rand`, `rand_distr`, and `rayon` match what upstream RuVector resolves today (see `vendor/ruvector/Cargo.toml` for the upstream pins). A mismatch would silently produce two compiled copies of the same crate, doubling build time and — worse — making it possible for the witness recipe in ruLake (`crates/core/src/bundle.rs:362`, which uses `sha3::Shake256`) to disagree with any upstream code path that touches the same primitives. The ADR accepts pin-bump as a manual operation: bumping rabitq is `git submodule update --remote vendor/ruvector` followed by `cargo build` and `cargo test` to confirm nothing regressed, with the steps documented in `install.sh` and the README's build section.

The contract has no run-time enforcement layer; it is a build-time discipline. A regression here would surface as a build failure in CI on the next PR, not as a wrong answer at run time. That is the correct shape for a structural ADR.

## Reference implementation status

The ADR is in `Accepted` state with the verification block on the same commit that introduced it. The repository as it exists today carries the discipline forward:

- `Cargo.toml` at the root has only `[package]` — no `[workspace]` block. Concrete pins for every dep.
- `.gitmodules` at the root pins `vendor/ruvector` to a specific revision of `https://github.com/ruvnet/RuVector.git`.
- `vendor/ruvector/crates/ruvector-rabitq/` is the kernel source, reached through the path dep.
- `python/`, `node/`, `crates/mcp-server/`, `crates/ipfs-backend/`, `crates/gcs-backend/`, `crates/rvdna-backend/`, `crates/ruqu-backend/`, `crates/mcp-rvdna/`, `crates/mcp-ruqu/` are sibling crates. Each declares its own `[package]`, no workspace. Each depends on the parent through `path = ".."` (or `path = "../"` for nested cases) and inherits rabitq transitively.
- `Dockerfile` and `install.sh` (added in the same commit per the ADR's References block) operate on the assumption that `git submodule update --init --recursive` has been run.

The verification block in the ADR (`docs/adrs/ADR-001-standalone-repo-strategy.md:147`-area) records the original acceptance numbers: `cargo build` finishing in ~3.6 s for a clean Debug build, `cargo test --release` passing the 43-test suite (21 unit + 22 integration) the substrate-acceptance loop required at the time, and `cargo run --release --bin rulake-demo -- --fast` reproducing the headline tax number. The test count has grown since (the MCP server alone now ships 65+ tests per `crates/mcp-server/README.md`), but the verification recipe has not changed.

The single-commit acceptance is what makes ADR-001 a structural rather than a perpetual-roadmap ADR. There is no v0.x → v1.0 → v1.5 progression; the discipline either holds or it doesn't, and CI catches the failure mode immediately.

## Composition with the rest of ruLake

ADR-001 is the foundation that the other ADRs in the foundational set silently rest on.

ADR-002 (Python SDK / PyO3) lives at `python/`, ships as `rulake-py` on Cargo and `rulake` on PyPI, and depends on the parent crate via `rulake = { path = "..", version = "2.2.0" }` (`python/Cargo.toml:30`). The maturin sdist explicitly lists the parent crate's `src/`, `Cargo.toml`, `Cargo.lock`, and the vendored rabitq crate under `[tool.maturin].include` (`python/pyproject.toml:77`-area) so a `pip install <sdist>` on a fresh machine builds without network access — the air-gapped install case ADR-002 §Negative calls out. The sibling-crate pattern is the only reason this is a one-config-file question.

ADR-003 (Node SDK / napi-rs) lives at `node/`, ships as `rulake` on npm with five per-platform `optionalDependencies` for the prebuilt `.node` binaries, and depends on the parent through the same `path = ".."` pattern (`node/Cargo.toml:24`). The `prepublishOnly` hook in `node/package.json` enforces the submodule-present check as an npm-publish-time guard, and the per-platform optional packages ride the same submodule-aware build that the umbrella package uses.

ADR-004 (MCP server) lives at `crates/mcp-server/` as `ruvector-rulake-mcp` (`crates/mcp-server/Cargo.toml:19`), again depending on the parent crate via `path = ".."`. ADR-004 §2 ("Crate placement") explicitly cites ADR-001 as the reason it is a sibling crate rather than a workspace member or a `bin/` inside the main crate — the rejected alternatives would each pull tokio + hyper + rustls + rmcp into every library consumer's dep graph or would force a root workspace that ADR-001 §2 rejects.

ADR-005 (IPFS backend) lives at `crates/ipfs-backend/` and follows the same shape. ADR-006 (Console) is a Vite project rather than a Cargo crate but consumes the bundle JSON shape that the standalone-repo build produces. ADR-155 / 156 / 157 / 158 govern the design surface and assume a build system that produces predictable artefacts. Each ADR carries the assumption forward without restating it, which is the right behaviour — the foundational decision is paid for once.

A different way to put it: ADR-001 is the reason every other ADR can talk about "shipping" without first having to talk about "building".

## Open questions

Several genuine unknowns remain. Repo footprint is the loudest: `vendor/ruvector/` drags roughly ten thousand files into every clone, of which we compile exactly one crate. The ADR accepts this in exchange for the "one command to build" property, with a Future option ("rewrite to a sparse-checkout-friendly layout once the rabitq crate splits out of the monorepo upstream") tracked but not blocking. Submodule pin-bump cadence is unsettled — the ADR documents the manual ritual (`git submodule update --remote vendor/ruvector` + a clean rebuild + the 43-test suite) but does not commit to a frequency or an automated PR. Tooling that *insists* on a workspace (some cargo plugins, some IDE integrations) sees a bare package; the ADR notes this and judges it acceptable, but a popular plugin landing in the future could force a revisit. Whether to publish `ruvector-rabitq` to crates.io upstream — which would let ruLake drop the submodule entirely in favour of a `crates.io` version — is an upstream decision, not a ruLake one, and the ADR is honest that the submodule is the right shape *until* that happens.

## References

- ADR-001: `/home/ruvultra/projects/RuLake/docs/adrs/ADR-001-standalone-repo-strategy.md`
- Root manifest with concrete pins: `/home/ruvultra/projects/RuLake/crates/core/Cargo.toml`
- Submodule entry: `/home/ruvultra/projects/RuLake/.gitmodules` → `vendor/ruvector`
- Vendored kernel source: `/home/ruvultra/projects/RuLake/vendor/ruvector/crates/ruvector-rabitq/`
- Upstream workspace the submodule re-uses: `/home/ruvultra/projects/RuLake/vendor/ruvector/Cargo.toml`
- Sibling-crate pattern (each declares its own `[package]`, no `[workspace]`):
  `python/Cargo.toml:30`, `node/Cargo.toml:24`, `crates/mcp-server/Cargo.toml:33`,
  `crates/ipfs-backend/Cargo.toml`, `crates/gcs-backend/Cargo.toml`, `crates/rvdna-backend/Cargo.toml`,
  `crates/ruqu-backend/Cargo.toml`, `crates/mcp-rvdna/Cargo.toml`, `crates/mcp-ruqu/Cargo.toml`
- Capability review that surfaced the gap originally: `/home/ruvultra/projects/RuLake/docs/review/capabilities.md`
- Downstream ADRs assuming this discipline: ADR-002 (`docs/adrs/sdk/ADR-002-python-sdk.md`), ADR-003 (`docs/adrs/sdk/ADR-003-nodejs-typescript-sdk.md`), ADR-004 (`docs/adrs/sdk/ADR-004-rulake-mcp-server.md`), ADR-005 (`docs/adrs/sdk/ADR-005-ipfs-backend-and-deploy.md`), ADR-155 (`docs/adrs/ADR-155-rulake-datalake-layer.md`), ADR-156 (`docs/adrs/ADR-156-rulake-as-memory-substrate.md`), ADR-157 (`docs/adrs/ADR-157-optional-accelerator-plane.md`), ADR-158 (`docs/adrs/ADR-158-optional-rotation-and-qvcache-positioning.md`)
- Prior-art repos using the per-platform-optional-deps pattern that the Node SDK adopts (cited in ADR-003 §5): Prisma, next-swc, parcel-css, lightningcss — each documented in their respective npm registry entries; the ADR does not pin URLs.
