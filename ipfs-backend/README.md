# `ruvector-rulake-ipfs` — IPFS bundle distribution backend

Implements [ADR-005](../docs/adrs/sdk/ADR-005-ipfs-backend-and-deploy.md):
a `BackendAdapter` that publishes and resolves ruLake bundles
(`table.rulake.json`) over IPFS, addressed by CIDv1.

This is the **bundle distribution layer**, not the bulk vector-data
layer. Vector bodies stay on the original backend
([`gcs-backend/`](../gcs-backend/), `FsBackend`, etc.); IPFS handles
the cross-process witness anchor.

Sibling Cargo package per [ADR-001](../docs/adrs/ADR-001-standalone-repo-strategy.md).

## What v0.1 ships

- `IpfsBackend` — `BackendAdapter` trait impl over kubo HTTP RPC.
- **Three operating modes** (ADR-005 §2):
  - `Mode::Kubo` — talks to a kubo daemon (default `http://127.0.0.1:5001`); read + write + pin.
  - `Mode::Gateway` — read-only via a public gateway (ipfs.io / dweb.link / w3s.link). CI / sandbox / no-egress shape.
  - `Mode::KuboWithGatewayFallback` — kubo first, gateway on miss; audit-loud per ADR-005 §2.
- **Witness ↔ CID mapping** (ADR-005 §3 — "two anchors, one cache key"):
  - `rvf_witness` (SHAKE-256(32) hex) stays the cache anchor.
  - `data_ref` becomes `ipfs://<cid>`.
  - `Generation::Opaque(cid)` carries the CID — variant-tag domain
    separation at `src/bundle.rs:91-94` keeps GCS-`Num` and IPFS-`Opaque`
    generations from colliding by construction.
- **Cheap `current_bundle()` override** (ADR-004 §Resources contract,
  ADR-005 §4) — fast path is operator-supplied CID + dim → no IPFS
  round trip. Fallback path is a single-block `ipfs cat` (bundles are
  ≤ 64 KiB by `RuLakeBundle::from_json`'s cap → single block, no DAG walk).
- **Bearer auth** for the kubo HTTP API via kubo's native
  `API.Authorizations` `HTTPAuthSecrets` mechanism (ADR-005 §10).
- **Witness-fail-closed** — the existing `RuLakeBundle::read_from_dir`
  guard at `src/bundle.rs:349` rejects maliciously-pinned bundles
  whose body doesn't hash to its claimed witness. IPFS doesn't bypass
  the existing posture.

## What v0.1 deliberately doesn't do

- **No vector-body retrieval.** `pull_vectors()` errors with the
  ADR-005 §1 "bundle-only" message. Wire a body-store backend
  (`gcs-backend`, `FsBackend`, etc.) for vectors and use this backend
  for the witness-distribution.
- **No pinning-service mode** (Storacha / Pinata / Filebase) — v0.2.
- **No AES-256-GCM envelope** for private bundles — operators today
  can encrypt the bundle JSON themselves before `publish_bundle`.
- **No `ipfs-api-backend-hyper` SDK** — pulls a yanked transitive
  (`multihash 0.17` → `core2 0.4`). v0.1 talks plain `reqwest` to
  kubo's `/api/v0/{add,cat,pin/add}` endpoints. ~50 LOC, ~40 fewer
  transitive deps, no upstream maintenance dependency.

## Build + test

```bash
git clone --recurse-submodules https://github.com/ruvnet/RuLake
cd RuLake/ipfs-backend
cargo build --release
cargo test --release          # 6 offline tests (1 unit + 5 smoke)

# Live test against a real kubo daemon:
RULAKE_IPFS_LIVE_TEST=1 \
RULAKE_IPFS_KUBO_API=http://127.0.0.1:5001 \
cargo test --release -- --ignored ipfs_live
```

## Usage

```rust
use std::sync::Arc;
use rulake::{cache::Consistency, RuLake, BackendAdapter};
use ruvector_rulake_ipfs::{IpfsBackend, IpfsCollection, Mode, KuboApiUrl};

// Default — local kubo, no auth.
let ipfs = IpfsBackend::kubo_local("ipfs-prod")?;
ipfs.register(IpfsCollection {
    name: "memories".into(),
    cid:  None,             // populated by publish_bundle below
    dim:  Some(768),
})?;

// Production — kubo with bearer auth + gateway fallback.
let prod = IpfsBackend::new(
    "ipfs-prod",
    Mode::KuboWithGatewayFallback {
        api:       KuboApiUrl("http://10.128.0.5:5001".into()),
        api_token: Some("kubo-bearer-token-from-secret-manager".into()),
        gateway:   ruvector_rulake_ipfs::GatewayUrl("https://w3s.link".into()),
    },
)?;
```

## Wire into ruLake

```rust
// IPFS as the bundle layer; GCS as the body layer.
let lake = RuLake::new(20, 42)
    .with_consistency(Consistency::Eventual { ttl_ms: 5_000 });

let bodies = ruvector_rulake_gcs::GcsParquetBackend::open_gcs("gcs-bodies", "my-bucket")?;
lake.register_backend(Arc::new(bodies) as Arc<dyn BackendAdapter>)?;

let bundles = IpfsBackend::kubo_local("ipfs-bundles")?;
lake.register_backend(Arc::new(bundles) as Arc<dyn BackendAdapter>)?;
```

## Deployment (Compute Engine, `ruv-dev`)

ADR-005 §9 walks through the deployment annex: `e2-small` VM,
`pd-balanced` 50 GB, no public IP, IAP-tunneled SSH, kubo bound to
localhost (private-IP via firewall `--source-tags rulake-serving`),
bearer auth via kubo's native `HTTPAuthSecrets`, daily PD snapshots.
**~$20-25/month at steady state.** See ADR-005 §9 for the concrete
`gcloud` commands.

## License

MIT OR Apache-2.0, matching the parent crate.
