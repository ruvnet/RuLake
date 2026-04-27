# ADR-005: ruLake IPFS backend — content-addressed bundles, kubo daemon on GCP

## Status

**Accepted (2026-04-26 → v0.1 as of 2026-04-27)** — `ipfs-backend/`
crate is shipping. v0.1 carries the kubo client + gateway-fallback
mode, witness-anchored bundle distribution by CIDv1, and the offline
publish/fetch round-trip that the smoke suite exercises. Security
review (commit `8ce3689`) surfaced **R-IPFS-1**: `fetch_bundle` was
warn-only on `data_ref ≠ ipfs://{cid}` mismatch — closed by hard
refuse with code `IPFS_BUNDLE_CID_MISMATCH` in commit `56b497b`.
Sister ADR to [ADR-004](./ADR-004-rulake-mcp-server.md); the GCS
backend (`gcs-backend/`, commit `c706dc6`) was the first cloud-shaped
backend, IPFS is the second and the first whose addressing model is
content-hash-native.

**Originally proposed (2026-04-26)** — fixed the shape before the
first PR opened so the client / pinning / public-gateway / deployment
questions wouldn't relitigate at code-review time.

## Date

2026-04-26

## Authors

ruv.io · RuVector engineering. Drafted alongside the operator's
ask for a single-node kubo on the `ruv-dev` GCP project so the
deployment annex (§9–§11) and the backend design land in one
artefact — the IPFS layer is operationally inseparable from the
node it talks to.

## Relates To

- [ADR-001](../ADR-001-standalone-repo-strategy.md) — sibling crate
  layout, no root workspace, vendored submodule discipline. The new
  crate slots in next to `gcs-backend/` under the same rules.
- [ADR-155](../ADR-155-rulake-datalake-layer.md) — the cache-first /
  witness-as-anchor framing that this backend has to honour. The
  M2-M5 backend roadmap explicitly anticipates content-addressed
  backends; IPFS is the first one in the family.
- [ADR-004](./ADR-004-rulake-mcp-server.md) — `rulake://bundle/{backend}/{collection}`
  resource contract requires a cheap `current_bundle()` override.
  That contract is the load-bearing one for this backend because IPFS
  body fetches are slow.
- [ADR-002](./ADR-002-python-sdk.md) / [ADR-003](./ADR-003-nodejs-typescript-sdk.md)
  — sibling-crate layout precedent. The IPFS backend follows the same
  no-workspace, no-`*.workspace = true` discipline.
- Prior art in this repo: `gcs-backend/` (Parquet on GCS via
  `object_store`). Mirror its shape — `with_store(...)` for tests,
  cached `(dim, generation)` table, `block_on` bridge from the sync
  `BackendAdapter` trait into an async client.
- Prior art outside this repo: kubo HTTP RPC v0
  (`POST /api/v0/{cat,add,pin/add,pin/rm,pin/ls,id}`), the
  vendor-agnostic IPFS Pinning Services API
  (`github.com/ipfs/pinning-services-api-spec`).

---

## Context

ruLake's `BackendAdapter` trait (`src/backend.rs:110`) is what every
data lake plugs into. Today we have `LocalBackend`, `FsBackend`, and
the freshly-shipped `GcsParquetBackend`. The next natural backend is
content-addressed storage: a Parquet (or, more relevantly for this
backend, **bundle-only**) artefact addressed by its hash, served
from a public network of nodes that the operator may or may not run.
That ecosystem is IPFS, and the dominant daemon is kubo.

There are three forces pushing this backend onto the v0.5 list:

1. **The bundle is already content-addressed.** `RuLakeBundle::new`
   produces a `rvf_witness` that is `SHAKE-256(32)` over the four
   content-shaping fields plus the variant-tagged `Generation`
   (`src/bundle.rs:166-187` for `new`, `src/bundle.rs:362-390` for
   `compute_witness`). The witness IS the cache-key anchor; two
   bundles with the same witness are interchangeable. IPFS gives us
   a second, network-level anchor (the CID) that maps onto
   `Generation::Opaque(String)` at `src/bundle.rs:60` without changing
   the witness format. Two anchors, one cache key — the IPFS variant
   adds a *naming* axis without disturbing the *trust* axis.

2. **The bundle is small.** A `table.rulake.json` is a few hundred
   bytes (the on-disk size cap is 64 KiB at `src/bundle.rs:218`,
   but that's 100× headroom). At that size the IPFS performance
   story is *gateway-round-trip-bound*, not body-bandwidth-bound. A
   bundle fetched through the public ipfs.io gateway is a single
   small block; latency is in the same league as a GCS HEAD.

3. **The operator wants a private node.** `gcloud config get-value
   project` is `ruv-dev`; GKE is not enabled; there's no kubo today.
   The operator's ask in this ADR is "give me a single-node kubo
   on ruv-dev so I can pin bundles myself, plus a backend that
   resolves CIDs through it." The deployment annex (§9–§11) is
   half this ADR.

The MCP server's `transfer_ipfs-resolve` tool is *not* a substitute
— it resolves IPNS names via public gateways and returns a CID, full
stop. Using it from inside ruLake would mean every bundle read goes
through a public gateway with no auth and no SLA. We need a real
backend with a real client.

### What "the IPFS backend stores" — sharpening the scope

This ADR commits to a **bundle-only** IPFS backend in v0.1. Vector
bodies stay on whatever backend already serves them (GCS+Parquet,
local FS, BigQuery). The IPFS layer carries the *sidecar*: the
`table.rulake.json` that anchors the cache. The `data_ref` inside
the bundle still points at the underlying byte stream
(`gs://bucket/x.parquet`, `iceberg://catalog/db/table`, etc.), and
ruLake's federation behaviour is unchanged — what IPFS adds is a
**portable, content-addressed location for the witness itself**.

Two concrete shapes this enables:

| Use case | What's on IPFS | What's still on the original backend |
|---|---|---|
| **Bundle-publishing** — operator wants any agent that knows the CID to be able to verify and refresh its cache against the canonical bundle | the `table.rulake.json` only | the Parquet bytes (GCS, S3, etc.) |
| **Cross-org cache priming** — partner teams cache-share via the witness without sharing storage credentials | the bundle, possibly an HMAC-encrypted variant | each org's own copy of the vector bytes |

What we are **not** building in v0.1 is "vector bodies on IPFS." That
shape is technically possible — encode a Parquet file as a UnixFS
DAG, pin its root CID, fetch it through `ipfs cat` — but the
performance story is bad: 100k vectors × 128 dim × 4 bytes is ~50 MB,
which is `~10–60 s` on a public gateway cold-fetch and incurs the
gateway's 5 GiB range-request cap. ADR-155's cache-first
positioning says backends are interchangeable so long as their
generation tokens are reliable; IPFS's role here is "carry the
generation token cheaply", not "serve the body". Body-on-IPFS is a
v0.3 reopener, behind the dim-1024 / count-100k workload that v0.1
cannot serve usefully.

### Spec & ecosystem snapshot (April 2026)

| Component | Status | Notes |
|---|---|---|
| **kubo (Go IPFS daemon)** | `v0.40.1` (2026-02-27) | Patch over v0.40.0 to dodge a Go 1.26 net-IO bug on Windows. The reference daemon. |
| **CIDv1 default base** | base32 (lowercase, multibase prefix `b`) | Switched in kubo 0.5+ for browser/case-insensitivity reasons; 2026 default. |
| **Public gateway**, IPFS Foundation | `ipfs.io`, `dweb.link`, `trustless-gateway.link` | Cloudflare's `cloudflare-ipfs.com` was decommissioned 2024-08; do not include in any default fallback list. |
| **Pinning Services API** | OpenAPI 1.0.0 at `github.com/ipfs/pinning-services-api-spec` | Vendor-agnostic; Pinata, Storacha (ex-web3.storage), and Filebase all implement it. |
| **Pinata** | active | Most-cited managed pin service; pricing tiered, auth via API key. |
| **Storacha** (formerly web3.storage) | active, post-rebrand | UCAN-based auth, free tier exists but reduced from the 2023 era. |
| **NFT.Storage** | active but NFT-only-positioning; backed by Filecoin long-term | Less suitable as a generic ruLake pin target. |
| **Filebase** | active | S3-compatible API plus an IPFS Pinning Service API endpoint; 5 GB free tier. |
| **Cloudflare IPFS gateway** | **decommissioned** 2024-08 | Excluded from the default gateway list. |

### Rust-side library landscape

Real Rust crates for IPFS as of April 2026:

| Library | Status | Verdict |
|---|---|---|
| `ipfs-api` + `ipfs-api-backend-hyper` | Mature, kubo HTTP RPC client. `0.17.x` family. Hyper-based, supports rustls, used by a long tail of Rust IPFS tooling | **Pick.** The smallest dep tree that talks to a real kubo node. |
| `ipfs-api-backend-actix` | Same façade, actix-web transport | Reject. Forces the operator's serving binary to drag actix in even when nothing else needs it. |
| `rust-ipfs` (`rs-ipfs`, embedded full node) | "early alpha" per the crate's own docs; libp2p + DHT + pubsub + HTTP API stubs in one box | Reject for v0.1. We don't want a libp2p node inside the ruLake serving process — DHT routing tables, NAT traversal state, peer-id key material, all of it is operational weight with no upside when the operator already has a kubo daemon. |
| `iroh` (n0-computer) | Active, `v0.98.0` (2026-04-17). Modern Rust networking stack | Reject for v0.1. Per `iroh.computer/docs/ipfs`: "you can't use Iroh as an embedded Rust IPFS implementation" today. The IPFS-compat layer is roadmap, not shipped. |
| `iroh-blobs` + `rust-libp2p` directly | Build-your-own | Reject. We'd own the libp2p stack as a maintenance surface. |
| Plain `reqwest` against the kubo HTTP RPC | Pragmatic | Reject as the *only* path; kept as a v0.1 fallback behind a Cargo feature so an operator who can't take the `ipfs-api` dep still has a way out. |

The decision: **kubo HTTP RPC via `ipfs-api-backend-hyper`** is the
default. The Rust ecosystem has settled the same way the Python and
JS ones have — most users aren't running a libp2p node, they're
talking to one over HTTP.

### The MCP `transfer_ipfs-resolve` tool — what it actually does

The MCP-side tool is a one-trick pony: takes an IPNS name string,
returns a CID. It's an IPNS resolver, not a content fetcher. We
treat it the same way we'd treat any other public resolver: a useful
helper for users who want to publish IPNS-named bundles, **but not
part of this backend's read path**. Resolving an IPNS name to a
CID is something the operator does once at config time; the
backend reads CIDs.

## Decision

We ship a Rust-native `ipfs-backend/` sibling crate that implements
`BackendAdapter` for **bundle metadata addressed by CID**, defaults
to read+optional-pin against a **kubo HTTP RPC** endpoint, falls
back to **public IPFS Foundation gateways** for read-only operation
when no kubo is configured, maps the CID into the existing
`Generation::Opaque(String)` slot so the witness contract is
unchanged, overrides `current_bundle()` with a single-block
`ipfs cat` so the ADR-004 resource contract is honoured cheaply,
and pairs the crate with a one-page deployment annex for a single
e2-small kubo node on the operator's `ruv-dev` GCP project.

The crate is `ruvector-rulake-ipfs`; the binary surface is the trait
impl plus a small CLI helper for `pin` / `unpin` / `publish`. The
deployment shape is one VM, one persistent disk, IAP-tunneled SSH,
no public IP on the kubo HTTP API.

```text
ipfs-backend/
├── Cargo.toml          # ruvector-rulake-ipfs, no workspace
├── README.md           # install, configure kubo endpoint, gateway fallback
├── src/
│   ├── lib.rs          # public exports
│   ├── backend.rs      # IpfsBundleBackend impl of BackendAdapter
│   ├── client/         # kubo HTTP client + gateway fallback
│   │   ├── mod.rs
│   │   ├── kubo.rs     # ipfs-api-backend-hyper wrapper
│   │   └── gateway.rs  # reqwest against ipfs.io / dweb.link
│   ├── cid.rs          # CID parsing + URL helpers
│   └── pin.rs          # publish_bundle → kubo add+pin (or pinning-service)
├── examples/
│   └── publish_bundle.rs
└── tests/
    └── smoke.rs        # offline against a fake kubo + gateway
```

```bash
# Read-only, public-gateway only (what works without any kubo at all)
$ cargo run --example publish_bundle -- \
    --gateway https://ipfs.io \
    read bafkrei…

# Read+pin against a private kubo
$ export RULAKE_IPFS_KUBO_RPC=http://10.0.0.5:5001
$ export RULAKE_IPFS_KUBO_AUTH_FILE=/etc/rulake/kubo-bearer
$ cargo run --example publish_bundle -- publish ./snapshot/
   pinned bafkrei… (45 ms, kubo)
```

### 1. Crate placement — sibling `ipfs-backend/`, mirroring `gcs-backend/`

Per ADR-001 we have no root workspace; the IPFS backend slots in as
the third sibling backend crate (`gcs-backend/` shipped, an Iceberg
backend lives on the M5 list). One binary, `cargo install`-able, ships
as a library that the operator's serving binary registers via
`Arc<dyn BackendAdapter>` — same shape as `gcs-backend/`'s
`open_gcs(...) → Result<GcsParquetBackend>`.

| Option | Verdict |
|---|---|
| `ipfs-backend/` sibling crate | **Pick.** Mirrors `gcs-backend/`. No workspace, no `*.workspace = true`. Operator opts in by adding `ruvector-rulake-ipfs = { path = "ipfs-backend" }` (or the published version) to their serving binary. |
| `src/ipfs.rs` inside the main crate | Reject. Forces `ipfs-api`, hyper-rustls, and the kubo client into every ruLake consumer's dep graph. ~30 transitive crates; the library crate stays pure-sync, no-network for a reason. |
| `crates/rulake-ipfs/` workspace member | Reject. Requires a root workspace, which ADR-001 §2 explicitly rejects. |
| `examples/rust/06-ipfs-bundle/` | Reject for production; OK as a hand-on demo *in addition*. We follow the same split as `mcp-server/` vs `examples/nodejs/04-mcp-tool/`. |

### 2. The IPFS client — `ipfs-api-backend-hyper`, with a gateway fallback

The shape is the same `Inner { client, runtime, schema_cache }`
pattern that `GcsParquetBackend` uses, with a small twist: there
are two clients, picked by config.

```toml
# ipfs-backend/Cargo.toml — illustrative, not final
[package]
name = "ruvector-rulake-ipfs"
version = "0.1.0"
edition = "2021"
license = "MIT OR Apache-2.0"
authors = ["Ruvector Team"]
repository = "https://github.com/ruvnet/RuLake"
description = "IPFS BackendAdapter for ruLake — content-addressed bundles via kubo"

[dependencies]
rulake = { path = ".." }

# Default kubo RPC client. with-hyper-rustls feature gives us a TLS
# client that works against a kubo behind Caddy/nginx with TLS;
# with-builder gives us the request-builder API we use for pin add/rm.
ipfs-api-backend-hyper = { version = "0.6", default-features = false, features = ["with-hyper-rustls", "with-builder"] }
ipfs-api-prelude       = "0.6"

# Plain HTTP client for the public-gateway fallback path.
# Gateway reads are a single GET on `https://gateway/ipfs/<cid>?format=raw`;
# we don't need the kubo RPC for that.
reqwest                = { version = "0.12", default-features = false, features = ["rustls-tls", "stream", "json"] }

# Async runtime — single-threaded current-thread, owned by the backend
# (matches gcs-backend/src/backend.rs:74-77 exactly).
tokio                  = { version = "1.39", default-features = false, features = ["rt", "macros"] }

# CID parsing — multihash-aware, multibase-aware. cid-rs is the
# canonical Rust impl (used by rust-libp2p, rust-ipfs, iroh).
cid                    = "0.11"
multibase              = "0.9"
multihash              = "0.19"

# Errors + bytes + futures glue.
anyhow                 = "1"
bytes                  = "1"
futures                = "0.3"

# Logging.
tracing                = "0.1"

# Serde for the pinning-services-api JSON payloads.
serde                  = { version = "1", features = ["derive"] }
serde_json             = "1"

[features]
default              = ["kubo"]
# Behind the feature flag in case an operator can't take the
# ipfs-api transitive set; gateway-only mode still works.
kubo                 = []
gateway-only         = []
# Pinning-services-api client (Pinata / Storacha / Filebase).
# Off by default to keep the v0.1 surface narrow.
pinning-service      = []

[dev-dependencies]
tempfile = "3"
mockito  = "1"
tokio    = { version = "1.39", features = ["rt-multi-thread", "macros"] }

[profile.release]
opt-level     = 3
lto           = "thin"
codegen-units = 1
strip         = "symbols"
```

The `Cargo.toml` mirrors `gcs-backend/Cargo.toml` line-for-line on
the runtime + bytes + futures + tracing block; the only swap is
`object_store + parquet + arrow` becoming `ipfs-api-backend-hyper +
reqwest + cid`. Operators with both backends pay the union; in
practice the dep trees overlap heavily on hyper / rustls / tokio so
the marginal cost is small.

#### Operating modes — one binary, three runtime shapes

| Mode | Reads | Writes (pin) | Auth | Use case |
|---|---|---|---|---|
| **`kubo` (default)** | kubo HTTP RPC, single-block `ipfs cat` | kubo HTTP RPC, `pin/add` + `add` | bearer / basic via `kubo-bearer-file` (see §4) | Operator runs the kubo on `ruv-dev` per §9. |
| **`gateway-only`** | public IPFS Foundation gateways (`ipfs.io`, `dweb.link`, `trustless-gateway.link`) with constant-time round-robin | refused — backend is read-only | none (gateways are unauthenticated) | Quick eval; CI; partners reading published CIDs without standing up a node. |
| **`kubo` + `pinning-service`** | kubo HTTP RPC | also pushes to a pinning service (Pinata / Storacha / Filebase) via the OpenAPI 1.0.0 endpoint | kubo bearer + service API key | Belt-and-braces: own node holds the data hot, the service holds it cold so a node-loss event is recoverable. |

The gateway path is **explicitly read-only**. Public gateways don't
accept writes, and an operator who's writing through a gateway is
either confused or being attacked. The backend rejects `publish_bundle`
calls in `gateway-only` mode at trait boundary, with
`RuLakeError::InvalidParameter("ipfs gateway-only mode is read-only")`.

### 3. Witness ↔ CID mapping — two anchors, one cache key

The bundle's `rvf_witness` is `SHAKE-256(32)` (hex). IPFS CIDs are
typically `sha2-256` multihashes, base32-encoded under multibase.
**These are different hash functions.** They cannot be made equal,
and we shouldn't try.

The mapping is:

| Field | What it is | Used for |
|---|---|---|
| `RuLakeBundle.rvf_witness` | `SHAKE-256(32)` over `(data_ref, dim, rotation_seed, rerank_factor, generation_with_variant_tag)`, `src/bundle.rs:362-390` | The cache-key anchor. Two backends with the same witness share the cache pointer — that's the cross-backend dedup story from ADR-155 §3.6. |
| `RuLakeBundle.data_ref` | URI of the authoritative byte stream — `gs://...`, `iceberg://...`, or now **`ipfs://<cid>`** | The pull source. The witness includes this string verbatim, so two different IPFS CIDs (or an IPFS CID and a GCS URI) addressing different bytes naturally produce different witnesses. |
| `RuLakeBundle.generation` | `Generation::Opaque(<cid>)` for IPFS-resident bundles, `src/bundle.rs:60` | The coherence token. Each new pin (≡ new CID) is a new generation. The variant tag (`0x01` for `Opaque`, `src/bundle.rs:91-94`) keeps it from colliding with `Num` mtimes from other backends, per the witness security audit at `src/bundle.rs:71-81`. |

Concrete construction in `IpfsBundleBackend::current_bundle`:

```rust
// Illustrative — actual code in ipfs-backend/src/backend.rs.
fn current_bundle(
    &self,
    collection: &str,
    rotation_seed: u64,
    rerank_factor: usize,
) -> Result<RuLakeBundle> {
    // 1. Resolve (collection name) → CID. Source is operator config
    //    (a static map) or, in v0.2, an IPNS name resolved via the
    //    same kubo. v0.1 is config-only — no IPNS in the read path.
    let cid: Cid = self.resolve_collection_to_cid(collection)?;

    // 2. Single-block fetch. Bundle is ≤ 64 KiB by contract (§4).
    //    No DAG walk, no UnixFS, just `cat` the raw block.
    let bytes = self.client.cat_one_block(&cid)?;

    // 3. Parse + verify the witness on the bundle's own terms.
    //    `from_json` enforces the size + field caps already
    //    (src/bundle.rs:215-263). We do NOT recompute the witness
    //    here — `read_from_dir` does that for the FS sidecar
    //    case; we mirror it for IPFS in `IpfsBundleBackend::read`.
    let bundle: RuLakeBundle = RuLakeBundle::from_json(
        std::str::from_utf8(&bytes)
            .map_err(|e| RuLakeError::Backend { backend: self.id.clone(), detail: format!("ipfs cat {cid}: not utf-8 ({e})") })?
    )?;
    if !bundle.verify_witness() {
        return Err(RuLakeError::Backend {
            backend: self.id.clone(),
            detail: format!("ipfs witness mismatch on cid={cid}"),
        });
    }

    // 4. Re-anchor the bundle: data_ref becomes ipfs://<cid> if it
    //    wasn't already; generation becomes Opaque(cid). The original
    //    witness is preserved — re-anchoring is a *naming* operation,
    //    not a payload change. (Strictly: we VERIFY the bundle as it
    //    arrived from IPFS, then publish a *re-anchored* bundle through
    //    `RuLakeBundle::new` whose witness reflects the IPFS-side
    //    fields. See §3.4 below for why both shapes have to exist.)
    Ok(RuLakeBundle::new(
        format!("ipfs://{cid}"),
        bundle.dim,
        rotation_seed,
        rerank_factor,
        Generation::Opaque(cid.to_string()),
    ))
}
```

#### 3.4 The two-bundles question — answered

A subtle but real question: when we pin a bundle on IPFS, does its
witness change? Two bundles, two views:

| View | `data_ref` | `generation` | Witness |
|---|---|---|---|
| **Pre-pin** (the bundle the operator computed locally before pushing it to IPFS) | `gs://bucket/x.parquet` (or wherever the bytes live) | `Generation::Num(<gcs-gen>)` | `W_pre` |
| **Post-pin** (the bundle the IPFS backend serves, after the bytes have been pinned and the CID is known) | `ipfs://<cid>` | `Generation::Opaque(<cid>)` | `W_post` |

`W_pre` and `W_post` are **different witnesses** because both
`data_ref` and `generation` differ. That's correct — they describe
the same vectors but they pin them via different addressing schemes.
Cache sharing across the two requires a witness-level alias table,
which we deliberately do **not** build in v0.1 because it adds a
trust layer (someone has to assert the alias) on top of a system
designed to need none.

The operator's mental model: pinning a bundle to IPFS *publishes*
it — the IPFS-anchored bundle is a new artefact whose lineage points
back at the pre-pin version via the `lineage_id` field
(`src/bundle.rs:141`), but they are separate cache entries. v0.2
opens the alias-table conversation if a customer asks; v0.1 keeps
the trust surface flat.

### 4. Read path — single-block `ipfs cat`, no DAG walk

A `table.rulake.json` is bounded by the existing 64 KiB JSON size
cap (`src/bundle.rs:218`). IPFS's default block size is 256 KiB. A
bundle therefore lives in **exactly one block** — no UnixFS DAG, no
chunking, no link traversal. The read is one HTTP request:

```text
# kubo (RPC):
POST http://kubo:5001/api/v0/cat?arg=<cid>&length=65536

# gateway (raw block path):
GET https://ipfs.io/ipfs/<cid>?format=raw   # IPIP-412 raw-block format
GET https://ipfs.io/ipfs/<cid>              # default deserialised path also works
```

The `length=65536` cap on the kubo path mirrors `MAX_JSON_BYTES` —
a bundle larger than that is rejected by `from_json` anyway, so
fetching past the cap wastes bandwidth on data we're going to
discard. The gateway path can't pass `length` (the gateway protocol
doesn't expose it), so we cap at parse time via the existing
`from_json` check.

**This is the cheap-`current_bundle()` story for IPFS.** The
ADR-004 §Resources contract requires every backend that backs the
`rulake://bundle/{backend}/{collection}` resource to override the
default `current_bundle` impl (which does a full `pull_vectors`).
For GCS+Parquet the override is "HEAD + footer read"; for IPFS it
is "single-block cat" — actually cheaper, because there's no schema
to learn. The bundle *is* the schema.

#### Public-gateway fallback — on / off / how

The default is **off** for the kubo-mode build, **on** as the only
read path for the `gateway-only` build. The mode is picked by
operator at construction:

```rust
// Illustrative.
let backend = IpfsBundleBackend::open_kubo("ipfs-prod", "http://10.0.0.5:5001")?
    // Allow read-only fallback to public gateways when the kubo is
    // unreachable (network partition, daemon down). Off by default
    // — fallback hides outages and complicates the audit story.
    .with_gateway_fallback(GatewayPolicy::OnKuboError {
        gateways: vec![
            "https://ipfs.io".into(),
            "https://dweb.link".into(),
            "https://trustless-gateway.link".into(),
        ],
        per_gateway_timeout: Duration::from_secs(3),
    });
```

The fallback policy is opt-in and *audit-loud*: every gateway-served
read emits a structured `tracing::warn!` line with `cid`, `gateway`,
`reason: "kubo_unreachable"` so the operator can see the
degradation. `cloudflare-ipfs.com` is **not** in the default list
because it was decommissioned in August 2024.

### 5. Write path — `publish_bundle` → kubo `add` + `pin/add`, optionally also a pinning service

`RuLake::publish_bundle` (`src/lake.rs:167`) calls
`backend.current_bundle(...)` and then `bundle.write_to_dir(dir)` —
that's the FS-sidecar shape. For IPFS we keep the same trait surface
but provide a backend-specific helper:

```rust
// Illustrative.
impl IpfsBundleBackend {
    /// Pin a freshly-computed bundle on IPFS and return its CID.
    /// Wraps two kubo RPC calls: `add` (uploads the bundle bytes,
    /// returns a CID) and `pin/add` (asks the local node to keep
    /// the block from being garbage-collected).
    ///
    /// In `kubo + pinning-service` mode also POSTs to the configured
    /// pinning service per the IPFS Pinning Services API spec.
    pub fn publish(&self, bundle: &RuLakeBundle) -> Result<Cid> { … }
}
```

The contract:

- **Idempotent.** Re-publishing the same bundle bytes returns the
  same CID (content addressing means the network does the dedup for
  us). The kubo `add` RPC's response carries the CID; we never
  guess.
- **Atomic in the `add` step.** kubo's `add` either fully ingests
  the block or fails — there's no half-pinned state. The subsequent
  `pin/add` is what makes the pin durable; if the pinning step fails
  the operator gets a structured error and can retry without
  re-uploading (the block is already in the local store).
- **Pinning-service push is best-effort.** When the
  `pinning-service` feature is enabled and a service is configured,
  the post-`pin/add` push is asynchronous; the function returns the
  CID as soon as the kubo pin lands. Failures on the service push
  surface as a `tracing::warn!` line with retry queueing — losing
  the redundant pin shouldn't fail the publish call when the local
  pin succeeded.

#### Pinning-service auth — the existing discipline applies

ADR-004 §5 enforces a `--bearer-token-file` discipline for the MCP
HTTP transport. We mirror it here: pinning-service API keys are
read from a file, not an env var or a CLI argument. The file path
is the value the operator passes; the contents are the key.
Rationale matches ADR-004's: env vars leak through `/proc/<pid>/environ`,
CLI args leak through `ps aux`, files have proper FS permissions
and audit trails.

```toml
# operator config — illustrative
[ipfs]
kubo_rpc           = "http://10.0.0.5:5001"
kubo_auth_file     = "/etc/rulake/ipfs/kubo-bearer"

[ipfs.pinning_service]
endpoint           = "https://api.pinata.cloud/psa"   # any PSA-compliant URL
auth_file          = "/etc/rulake/ipfs/pinata-key"
service_label      = "pinata-prod"                    # for audit logging
```

### 6. Lifecycle & garbage collection — the operator's call

A pin lives until somebody unpins it. There is no automatic eviction
in v0.1. Concretely:

- **`publish_bundle` always pins.** Operators who don't want a pin
  can use the lower-level `add_only(...)` helper, but the default is
  pin-everything. Cache invariant: every CID the backend has ever
  returned is still resolvable on the local kubo unless the operator
  explicitly removed it.
- **`unpin(cid)` is exposed as a backend method**, not as a
  `BackendAdapter` trait member. Pin removal is an operational
  action, not a query-path action.
- **Quotas are kubo's problem, not ruLake's.** The kubo daemon's
  `Datastore.StorageMax` config (default 10 GB) limits the local
  blockstore; we don't shadow that limit. When a publish call would
  push the daemon past `StorageMax`, the kubo RPC returns a
  structured error and we surface it as
  `RuLakeError::Backend { detail: "ipfs storage full: ..." }`.
- **Pinning-service quotas** are likewise the operator's problem.
  v0.1 does not implement the Pinning Services API's `pins?status=...`
  list-and-prune workflow; it's a v0.2 reopener once a customer
  hits a quota.

### 7. Performance contract — gateway-bound, not body-bound

A bundle is small, so the latency story is the *round trip*, not the
*throughput*. Numbers we anchor against, with citations:

| Path | Expected p50 | Expected p99 | Source / rationale |
|---|---|---|---|
| **Local kubo, hot block** | 5–20 ms | 50 ms | Loopback HTTP RPC + in-memory blockstore hit. Dominated by JSON-RPC framing and serde. |
| **Local kubo, cold block (DHT walk needed)** | 200–1000 ms | 2–10 s | Bitswap + DHT discovery. Highly variable; dominated by network conditions and DHT health. ProbeLab tracks this on a weekly basis. |
| **Public gateway (`ipfs.io`/`dweb.link`), hot at gateway** | 50–200 ms | 500 ms | Anchored against the IPFS Foundation public gateways and the ProbeLab Tiros measurement series; precise weekly numbers live at `probelab.io/ipfs/gateways/`. Gateway-side caches absorb the popular content. |
| **Public gateway, cold** | 250–1500 ms | 3–10 s | Same gateway has to do its own DHT walk; first request after a publish is the worst case. |
| **GCS HEAD (for comparison; the gcs-backend baseline)** | 20–60 ms | 200 ms | What `GcsParquetBackend::generation` does today; the local-kubo-hot-block target sits within an order of magnitude. |

Practical implications baked into the v0.1 design:

1. **`current_bundle()` MUST stay a single block fetch.** The ADR-004
   resource contract is not negotiable; if it grew to a DAG walk we'd
   give back an order of magnitude.
2. **Cache `Consistency::Eventual { ttl_ms }` is the recommended
   default for IPFS backends.** Per ADR-155 §"Strict freshness, or
   10× throughput?", `Eventual` is the latency-friendly mode, and on
   IPFS the gap matters because `Fresh` triggers a generation check
   on every search — through the network. v0.1 ships a worked
   example pinning `ttl_ms = 30_000` against a kubo node, which
   amortises the network hop over 30 seconds of cache hits.
3. **Body fetches are out of scope.** A 50 MB Parquet through a
   public gateway is a tens-of-seconds operation and breaks the
   1.02× tax envelope from `BENCHMARK.md`. The IPFS backend serves
   the witness only.

### 8. Security — public-by-default, witness-fail-closed

The IPFS threat model has one big surprise relative to GCS: **pinned
CIDs are public**. Anyone who knows the CID can fetch the bytes
through any public gateway. This is a property of content addressing,
not a bug. The ADR commits to making this loud:

| Threat | Mitigation |
|---|---|
| **Public visibility of bundle contents** | The README and the constructor's doc-comment both state, in the first paragraph, that pinning a bundle on IPFS makes its contents network-public. Operators who need confidentiality must encrypt before publish — see "Encryption envelope" below. |
| **Adversarial pinned bundle** (malicious CID claims to be the next bundle for our collection) | Witness-fail-closed at `src/bundle.rs:349`. The bundle's own SHAKE-256 anchor catches any field-level tampering; an attacker who flips a field has to also recompute and re-publish, which is fine — it's a different bundle (different CID, different witness, different cache entry). What they cannot do is poison an existing trusted entry. |
| **Squatting on a CID** | Impossible by construction. CIDs are content-derived; you can't claim someone else's CID. |
| **DNS rebinding on the gateway path** | The gateway client refuses to follow redirects to non-`ipfs/` paths; gateways that misbehave are ejected from the rotation for the process lifetime. |
| **DoS via unbounded fetch** | `length=65536` cap on the kubo path; parse-time cap on the gateway path. A bundle that exceeds the cap fails before allocation. |
| **Public-gateway abuse / rate-limiting** | Gateway operators rate-limit; the backend respects `429` with exponential backoff and rotates to the next gateway. The operator's own kubo node has no such limit. |
| **Pinning-service credential leak** | `--auth-file` discipline (§5) keeps keys off `argv`/`env`. |
| **Witness-format downgrade** | `RuLakeBundle::from_json` rejects `format_version > 2` (`src/bundle.rs:227-233`); a future v3 bundle on IPFS can't trick a v2 reader. |

#### Encryption envelope — the small-payload escape hatch

Bundles are bounded at 64 KiB. AES-256-GCM over a 64 KiB blob with a
per-collection key is a single primitive call; the on-IPFS object
becomes `(nonce || ciphertext || tag)`, the cleartext bundle's
witness is computed *before* encryption, and decryption keys live in
whatever secret manager the operator already runs (GCP Secret
Manager on `ruv-dev`, KMS, sops, your call). v0.1 doesn't ship the
envelope code itself — that's an `examples/` exhibit, not a backend
feature — but the design is documented because operators will ask
on day one.

What we explicitly do **not** ship in v0.1:

- **A managed key-server.** Out of scope; operator owns key
  custody.
- **A re-encryption / proxy-decryption mode.** Out of scope;
  v0.2 reopener if a multi-org sharing case appears.
- **Per-bundle ACLs on the kubo HTTP API.** kubo doesn't natively
  do per-CID ACLs; the daemon-level auth in §11 is the v0.1
  control.

### 9. Deployment annex (Compute Engine, `ruv-dev`)

This section walks the operator from "no kubo" to "kubo answering
my backend's RPC" on the existing `ruv-dev` GCP project. The
project's account is `ruv@ruv.net`, GKE is **not** enabled, and
there is no existing IPFS deployment. Decisions made:

#### 9.1 Where to run kubo

| Option | Verdict | Why |
|---|---|---|
| **Compute Engine `e2-small` VM, attached PD-balanced, IAP-tunneled SSH, no public IP** | **Pick.** | Minimal. Kubo is a stateful daemon with a libp2p swarm port that wants a stable network identity; Cloud Run's stateless cold-start model fights it. Compute Engine gives us a private IP, a persistent block device, and SSH-via-IAP for ops. ~$15-25/mo at sustained-use discount. |
| Compute Engine `e2-micro` (free-tier eligible in `us-west1` / `us-central1` / `us-east1`) | Reject as v0.1 default; OK as a development/eval shape. | The free tier's 0.25 vCPU shared and 1 GB RAM is enough to *boot* kubo but not enough to handle a sustained DHT walk (kubo's GC + Bitswap can spike RAM); the v0.1 production target is `e2-small` (2 vCPU shared, 2 GB RAM). The free tier is a great "try it" path and we document it but don't recommend it for any pinning workload. |
| Cloud Run with GCS-FUSE backing the blockstore | Reject. | GCS-FUSE doesn't provide concurrency control — kubo's blockstore is a write-heavy store with `flatfs` semantics that assume POSIX rename atomicity. Cloud Storage FUSE is "not fully POSIX compliant," explicitly. Cloud Run's request-driven container lifecycle also fights kubo's long-lived swarm connections. Wrong tool. |
| Cloud Run with Filestore-mounted blockstore | Reject. | Filestore solves the POSIX problem but Filestore's minimum spend ($150-200/mo for the smallest tier) blows the cost envelope before we've pinned anything. |
| GKE Autopilot | Reject. | GKE API is not enabled on `ruv-dev` per the operator's note. Enabling it for one daemon is overkill — control-plane cost alone (~$72/mo until Autopilot's free-tier credit applies) exceeds the entire VM-based shape. |
| Self-managed k8s on a Compute Engine VM | Reject. | A k8s control plane to run one pod is operational debt for no upside. |
| ipfs-cluster / multi-node | Reject for v0.1. | Single-node is the operator's ask. Cluster shape is a v0.2+ reopener once we have a workload that justifies the second node. |

#### 9.2 Concrete VM shape

```bash
# ─── Variables, edit before running ────────────────────────────────
PROJECT=ruv-dev
ACCOUNT=ruv@ruv.net
REGION=us-central1            # free-tier eligible; cheap PDs; close to ADC.
ZONE=us-central1-a
VM_NAME=kubo-1
NETWORK=default               # use the project's default VPC
DISK_SIZE=50GB                # blockstore + OS; PD-balanced ~$5.50/mo
SA_NAME=kubo-sa
SA_EMAIL="${SA_NAME}@${PROJECT}.iam.gserviceaccount.com"

# ─── 1. Service account with the minimum scopes ───────────────────
gcloud iam service-accounts create "${SA_NAME}" \
    --project "${PROJECT}" \
    --display-name "kubo IPFS daemon"

# Logging + monitoring (ops). No storage write needed unless we
# decide to back up the blockstore to GCS — see §9.5.
gcloud projects add-iam-policy-binding "${PROJECT}" \
    --member "serviceAccount:${SA_EMAIL}" \
    --role   "roles/logging.logWriter"
gcloud projects add-iam-policy-binding "${PROJECT}" \
    --member "serviceAccount:${SA_EMAIL}" \
    --role   "roles/monitoring.metricWriter"

# ─── 2. The VM itself ─────────────────────────────────────────────
gcloud compute instances create "${VM_NAME}" \
    --project       "${PROJECT}" \
    --zone          "${ZONE}" \
    --machine-type  "e2-small" \
    --image-family  "debian-12" \
    --image-project "debian-cloud" \
    --boot-disk-size      "${DISK_SIZE}" \
    --boot-disk-type      "pd-balanced" \
    --boot-disk-device-name "kubo-boot" \
    --service-account "${SA_EMAIL}" \
    --scopes          "https://www.googleapis.com/auth/cloud-platform" \
    --no-address \
    --network         "${NETWORK}" \
    --tags            "kubo-node" \
    --shielded-secure-boot \
    --shielded-vtpm \
    --metadata enable-oslogin=TRUE

# ─── 3. Firewall: allow the libp2p swarm in (so peers can dial us)
#     and IAP-tunneled SSH only. RPC stays internal — never open
#     port 5001 to the world. ─────────────────────────────────────
gcloud compute firewall-rules create kubo-swarm-in \
    --project   "${PROJECT}" \
    --direction INGRESS \
    --network   "${NETWORK}" \
    --target-tags kubo-node \
    --allow     tcp:4001,udp:4001 \
    --source-ranges 0.0.0.0/0

gcloud compute firewall-rules create kubo-iap-ssh \
    --project   "${PROJECT}" \
    --direction INGRESS \
    --network   "${NETWORK}" \
    --target-tags kubo-node \
    --allow     tcp:22 \
    --source-ranges 35.235.240.0/20            # IAP TCP forwarding range

# (No firewall rule for 5001/RPC or 8080/gateway. Both stay
# bound to localhost on the VM and reachable only via SSH-tunnel
# or a private peering, per §11.)

# ─── 4. Get on it ────────────────────────────────────────────────
gcloud compute ssh "${VM_NAME}" \
    --project "${PROJECT}" \
    --zone    "${ZONE}" \
    --tunnel-through-iap
```

A few of these choices are load-bearing — call them out explicitly:

- **`--no-address`** drops the public IP. We never want kubo's RPC
  (port 5001) reachable from the internet, and we don't need an
  inbound public IP for the libp2p swarm: kubo will hole-punch /
  use AutoNAT through the firewall-allowed UDP/TCP 4001 just fine.
  No public IP = no IAM-leak path to a misconfigured admin endpoint.
- **`--tunnel-through-iap`** is how we SSH in without a public IP.
  IAP TCP forwarding tunnels SSH over a TLS connection to Cloud
  IAP, applies any context-aware-access policies, then forwards to
  the VM. Zero bastion cost.
- **`--shielded-secure-boot --shielded-vtpm`** is free hardening;
  no reason to skip it.
- **`enable-oslogin=TRUE`** ties SSH access to the project's IAM
  rather than per-VM SSH keys. Single source of truth.
- **`pd-balanced` over `pd-ssd`**: a single-node IPFS workload's
  IOPS pattern (sequential block writes, random small-block reads)
  is fine on balanced; SSD is 4× the price for IOPS we won't use.

#### 9.3 Install + configure kubo (one-time, on the VM)

```bash
# On the VM, after `gcloud compute ssh ...`.
KUBO_VERSION=v0.40.1
curl -L "https://dist.ipfs.tech/kubo/${KUBO_VERSION}/kubo_${KUBO_VERSION}_linux-amd64.tar.gz" \
    | tar xz
sudo install kubo/ipfs /usr/local/bin/

sudo useradd --system --home /var/lib/ipfs --shell /usr/sbin/nologin ipfs
sudo install -d -o ipfs -g ipfs -m 0750 /var/lib/ipfs

# `server` profile disables local-network discovery (we're in a
# datacentre VPC, not a lab). `lowpower` is wrong here — we want
# the daemon responsive.
sudo -u ipfs IPFS_PATH=/var/lib/ipfs/.ipfs ipfs init --profile server

# Tighten the blockstore cap at OS level — we only have 50 GB of
# disk and the OS / logs need their share.
sudo -u ipfs IPFS_PATH=/var/lib/ipfs/.ipfs \
    ipfs config Datastore.StorageMax 30GB

# RPC bound to localhost (default), gateway also localhost. Both
# get exposed only via the SSH tunnel from operator workstations
# or via the in-VPC private IP from ruLake serving processes
# (see §11 for the auth story).
sudo -u ipfs IPFS_PATH=/var/lib/ipfs/.ipfs \
    ipfs config Addresses.API   /ip4/127.0.0.1/tcp/5001
sudo -u ipfs IPFS_PATH=/var/lib/ipfs/.ipfs \
    ipfs config Addresses.Gateway /ip4/127.0.0.1/tcp/8080

# Run it under systemd.
sudo tee /etc/systemd/system/ipfs.service >/dev/null <<'EOF'
[Unit]
Description=IPFS daemon (kubo)
After=network-online.target
Wants=network-online.target

[Service]
Type=notify
User=ipfs
Environment=IPFS_PATH=/var/lib/ipfs/.ipfs
ExecStart=/usr/local/bin/ipfs daemon --init=false --migrate=true
Restart=on-failure
RestartSec=5

# Hardening
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true
NoNewPrivileges=true
ReadWritePaths=/var/lib/ipfs

[Install]
WantedBy=multi-user.target
EOF

sudo systemctl daemon-reload
sudo systemctl enable --now ipfs
```

#### 9.4 Cost at steady state

Back-of-envelope, `us-central1`, sustained-use pricing as of
April 2026 (operator should re-check `cloud.google.com/compute/all-pricing`
before commitment):

| Line item | Monthly |
|---|---|
| `e2-small` VM, 730 hr, sustained-use discount | ~$13 |
| `pd-balanced` 50 GB | ~$5.50 |
| Egress (IPFS swarm + the occasional gateway hit; modest) | ~$2-5 |
| Logging (default-tier, ~50 MB/day from kubo) | <$0.50 |
| **Total** | **~$20-25/mo** |

Add another ~$5/mo for a snapshot schedule (§9.5). The free-tier
`e2-micro` knocks the VM cost to $0 for evaluation; we don't
recommend it for production pinning.

A pinning-service account for redundancy (Pinata's $20/mo "Picnic"
or Filebase's $15/mo "Starter") roughly doubles the budget but is
the right v0.2 hardening once the workload matters.

#### 9.5 Persistence + backup

`/var/lib/ipfs/.ipfs` is the blockstore. Two backup paths:

1. **PD snapshots** (recommended). One snapshot per day, keep 7,
   stored in the same region for free egress. ~$0.026/GB·month for
   incremental storage.

   ```bash
   gcloud compute resource-policies create snapshot-schedule kubo-daily \
       --project       "${PROJECT}" \
       --region        "${REGION}" \
       --max-retention-days 7 \
       --on-source-disk-delete keep-auto-snapshots \
       --daily-schedule \
       --start-time 03:00 \
       --storage-location "${REGION}"

   gcloud compute disks add-resource-policies "${VM_NAME}" \
       --project "${PROJECT}" \
       --zone    "${ZONE}" \
       --resource-policies kubo-daily
   ```

2. **GCS sync of the `.ipfs` directory** (backup of last resort).
   `gsutil rsync` the blockstore to a private GCS bucket nightly.
   Slower to restore (you have to re-stand-up a kubo and rsync
   back) but immune to a zone-wide PD failure.

3. **Re-pin from a pinning service.** The cheapest disaster-recovery
   path, *if* the operator opted into the `pinning-service` mode
   (§5). On VM rebuild, the kubo points at the same set of CIDs
   from the pinning service and Bitswap-pulls them back. No
   blockstore backup needed; tradeoff is recovery time
   (proportional to data size and gateway availability).

v0.1 ships an example `examples/operator/snapshot.sh` that wraps the
`gcloud compute resource-policies` recipe. Restore is documented in
the deployment README.

### 10. Auth on the kubo HTTP API

kubo's RPC is **admin-level access to the daemon** and is bound to
localhost by default — for very good reason. Per `docs.ipfs.tech/how-to/kubo-rpc-tls-auth`,
the recommended way to expose it is **TLS termination upstream + HTTP
auth**, and kubo's own `API.Authorizations` config supports basic
or bearer auth tokens via `HTTPAuthSecrets`. The `AuthSecret` value
is parsed from `type:value` strings: `basic:user:pass` or
`bearer:<opaque>`.

| Option | Verdict |
|---|---|
| **kubo's own `API.Authorizations` with a bearer token, plus IAP TCP forwarding for the network layer** | **Pick.** Bearer token over an IAP-tunneled connection to a private IP. Two layers (Google identity at the network edge, kubo bearer at the daemon edge); the bearer token lives in a file on the serving host with mode 0600. Mirrors ADR-004's `--bearer-token-file` posture. |
| Caddy / nginx in front of kubo with TLS + basic auth | Acceptable but unnecessary — adds another moving piece. Reach for it if the operator already runs a reverse proxy on the VM for other services. |
| GCP Identity-Aware Proxy in front of the RPC | Reject for v0.1. IAP for HTTP needs a load balancer + a managed certificate + a domain — operationally heavier than the bearer-token path and the operator hasn't asked for browser-based access. |
| mTLS on the kubo RPC | Reject for v0.1. kubo doesn't natively do mTLS on the RPC; we'd be terminating TLS in a sidecar. Reopen if a customer's mesh requires it. |
| **No auth, RPC bound to localhost, ssh-tunnel to use** | Acceptable for the operator's own ops sessions; not acceptable as the way the ruLake serving process talks to kubo from a sibling VM. |

Bearer-token shape, written to kubo's config:

```bash
# On the VM. Generate a 32-byte random token, store it in a
# 0600 file owned by `ipfs`, point the daemon at it.
TOKEN=$(openssl rand -hex 32)
sudo -u ipfs IPFS_PATH=/var/lib/ipfs/.ipfs \
    ipfs config --json API.Authorizations \
    "{\"rulake-prod\":{\"AuthSecret\":\"bearer:${TOKEN}\",\"AllowedPaths\":[\"/api/v0\"]}}"
sudo systemctl restart ipfs

# Stash the token where the serving host can pull it (Secret
# Manager is the right home; example shown for a manual sync).
gcloud secrets create rulake-ipfs-bearer --replication-policy automatic
echo -n "${TOKEN}" | gcloud secrets versions add rulake-ipfs-bearer --data-file -
```

The serving host fetches the secret on boot and writes it to
`/etc/rulake/ipfs/kubo-bearer` (mode 0600, owner = the serving-binary
user). The backend reads from `RULAKE_IPFS_KUBO_AUTH_FILE` —
**never** from `RULAKE_IPFS_KUBO_AUTH` (env), per ADR-004 §5
discipline.

### 11. Network shape — RPC stays private, swarm goes public

| Port | Bound to | Reachable from | Purpose |
|---|---|---|---|
| 4001 (TCP+UDP) | `0.0.0.0` | internet (firewall rule `kubo-swarm-in`) | libp2p swarm. Public reachability dramatically improves the daemon's DHT health and Bitswap latency. |
| 5001 (TCP) | `127.0.0.1` | only via SSH-tunnel (ops) or in-VPC private IP via the bearer-authed RPC (serving) | kubo HTTP RPC — admin-level. Never public. |
| 8080 (TCP) | `127.0.0.1` | only via SSH-tunnel (ops) | kubo gateway. Useful for local debugging; never exposed to the internet because it would let anyone fetch any CID through *our* IP. |

For the serving binary (the host running ruLake + the IPFS backend)
to reach 5001 from a *different* VM in the same VPC, the kubo VM's
firewall + the kubo `Addresses.API` config has to allow it. The
straightforward path is an additional rule:

```bash
gcloud compute firewall-rules create kubo-rpc-internal \
    --project       "${PROJECT}" \
    --direction     INGRESS \
    --network       "${NETWORK}" \
    --target-tags   kubo-node \
    --source-tags   rulake-serving \
    --allow         tcp:5001

# And re-bind the API to the private IP:
sudo -u ipfs IPFS_PATH=/var/lib/ipfs/.ipfs \
    ipfs config Addresses.API /ip4/0.0.0.0/tcp/5001
sudo systemctl restart ipfs
```

The `--source-tags rulake-serving` constraint plus the bearer-token
auth gives us **two independent gates** between the network and the
RPC: the network must be from a tagged VM, and the request must
carry the right bearer. Either gate alone is the v0.1 minimum; both
gates is the v0.1 recommendation.

### 12. Distribution & tests

Mirroring `gcs-backend/`'s split:

- **Offline tests** (the default `cargo test`) use a fake kubo
  client that stores blocks in a `HashMap<Cid, Vec<u8>>`. Same shape
  as `GcsParquetBackend::with_store`'s in-memory `ObjectStore`. The
  fake covers `add`, `cat`, `pin/add`, `pin/rm`, `pin/ls`. Tests
  assert: round-trip publish-then-read; witness-fail-closed on a
  block whose JSON has been tampered with; size-cap rejection;
  `current_bundle()` issues exactly one `cat` call (no DAG walk).
- **Live tests** (`RULAKE_IPFS_LIVE_TEST=1 cargo test --
  --ignored ipfs_live`) run against an operator-supplied endpoint:
  either a kubo URL (`RULAKE_IPFS_KUBO_RPC=...
  RULAKE_IPFS_KUBO_BEARER=...`) or a public gateway
  (`RULAKE_IPFS_GATEWAY=https://ipfs.io`). Same gate shape as
  `gcs-backend/tests/smoke.rs`'s `RULAKE_GCS_LIVE_TEST=1`.
- **Conformance test** against the IPFS Pinning Services API spec —
  behind the `pinning-service` feature, runs against an operator-
  supplied PSA endpoint. Asserts the request/response shapes match
  the OpenAPI 1.0.0 schema using `serde_json` golden parses; we do
  not bring in a full openapi-validator dep.

## Alternatives considered

### A. Embed iroh as a library (no kubo)

Reject for v0.1. Per `iroh.computer/docs/ipfs` (April 2026): "you
can't use Iroh as an embedded Rust IPFS implementation" today. The
IPFS-compat layer is on the roadmap but not shipped. v0.2 reopener:
when iroh's IPFS shape lands, an `iroh` Cargo feature could swap in
an embedded node and skip the daemon-deployment annex entirely. That
day is not today.

### B. Embed `rust-ipfs` as a full node

Reject. The crate's own docs flag it as "early alpha." Bringing a
libp2p stack inside the ruLake serving process means owning DHT
routing tables, NAT traversal, Bitswap, peer-id key material, and
the GC loop — all operational weight that the operator's own kubo
already carries. Deploy-the-daemon is the cheap path even though it
costs $20/mo; embed-the-stack is the expensive path even though it
costs $0.

### C. Public-gateway-only, no kubo

Reject as the v0.1 default; **kept** as an opt-in feature
(`gateway-only` Cargo feature). For an operator who only wants to
read CIDs that someone else has published, a kubo is overkill. For
an operator who wants to *publish*, the gateways are read-only —
there's no choice.

### D. Pinata-only (skip kubo, push directly to a pinning service)

Reject. The pinning-service path is great as a redundant pin target
but bad as the primary read path: every read goes through the
service's gateway, which is a third-party SLA we don't control. The
managed-only shape also means no ability to read private network
content; the kubo gives us a peer in the swarm, the service gives us
a vendor.

### E. Vector bodies on IPFS, not just bundles

Reject for v0.1. A 50 MB Parquet through a public gateway is
tens-of-seconds latency and breaks the 1.02× tax envelope; the same
file through a private kubo is bound by the local blockstore IOPS,
which we'd then have to size for the workload (back to "this is just
GCS+Parquet but slower"). The bundle-only shape preserves the
existing GCS+Parquet body path and adds witness portability — that's
the v0.1 product. Bodies-on-IPFS reopens at v0.3 with a real
workload to size against.

### F. Make the witness equal to the CID

Reject — they're different hash functions. The witness is
SHAKE-256(32) over a bundle-domain-separated concatenation that
includes a variant tag for `Generation` (`src/bundle.rs:71-97`).
The CID is a multihash of the *encoded bundle bytes* (UnixFS or
raw). Forcing them to be equal would mean either (a) changing the
witness format to be a multihash — which breaks every existing
ruLake deployment and abandons the SHAKE security argument — or (b)
encoding the bundle as a multihash-of-SHAKE blob, which is a
non-standard CID format that no IPFS tool can read. Two anchors,
one cache key, is the right shape.

### G. IPNS in the read path

Reject for v0.1. IPNS resolves a long-lived name to a (changing)
CID, with sub-second to multi-second latency depending on republish
frequency and the resolving node's record cache. For a backend
whose `current_bundle()` is *expected* to be cheap, adding an IPNS
resolution every read is a regression. Keep IPNS for the publish
side (operator-facing tooling that emits a stable name); read from
CIDs.

### H. Skip the deployment annex; let the operator figure out kubo themselves

Reject. The single-node-on-`ruv-dev` shape is the operator's
explicit ask, and ruLake has done this for every backend so far —
GCS got the `gcloud auth application-default login` story baked
into the README. Without the annex the backend is dead weight on
day one.

### I. Run kubo in Docker on the VM instead of bare-systemd

Acceptable; we don't pick it for v0.1 because the bare-systemd path
is one fewer moving piece (no Docker daemon, no image registry, no
container restart policy that fights systemd). Operators with an
existing Docker discipline can swap; the kubo image at
`ipfs/kubo:v0.40.1` is well-maintained.

### J. Run an `ipfs-cluster` for HA

Reject for v0.1. Single-node is the ask. Cluster shape is a v0.2+
reopener once a workload that justifies the second node arrives;
the cluster brings a real consensus layer (raft) and a meaningful
config surface that we shouldn't design speculatively.

## Consequences

### Positive

- **Witness portability.** A bundle published via this backend has a
  canonical CID; any agent that knows the CID can verify the witness
  and refresh its cache without holding storage credentials. That's
  a real cross-team / cross-org primitive ruLake didn't have before.
- **Cheap `current_bundle()`** — single-block fetch per the §4
  contract. Honours the ADR-004 §Resources requirement
  ("MUST NOT call default impl") with the smallest possible read.
- **Zero new trust surface in the witness contract.** The CID is a
  *naming* anchor that lands in `Generation::Opaque`; the witness
  format is unchanged, so `verify_witness()` and the
  `read_from_dir`-style fail-closed posture (`src/bundle.rs:349`)
  apply verbatim. The IPFS backend cannot poison the witness chain.
- **Deployment that fits in one VM.** $20-25/mo, no GKE, no Cloud
  Run, no Filestore. The annex turns the operator's environment
  (account `ruv@ruv.net`, project `ruv-dev`) into a working node
  with a copy-paste sequence of `gcloud` commands.
- **Public-gateway fallback for CI / partners.** A `gateway-only`
  build that doesn't need any infra is a real onboarding path —
  partner reads the CID through `ipfs.io` and gets the same bundle
  the operator published.
- **`rmcp`-style spec discipline.** kubo's HTTP RPC is versioned
  and stable; the `ipfs-api-backend-hyper` crate tracks it; the
  Pinning Services API is OpenAPI-generated. We don't ship a
  hand-rolled IPFS client.

### Negative / accepted

- **Pinned-bundle confidentiality is not a default.** Anyone with
  the CID can read it through any public gateway. Documented in
  the README and the constructor's doc-comment, but it's a
  meaningful posture change relative to the GCS backend (where the
  bytes live behind the bucket's IAM). Operators with sensitive
  bundles must encrypt before publish; the encryption envelope is an
  example, not a feature.
- **Cold gateway reads can be slow.** A first read of a freshly-
  published CID through a public gateway can be multi-second while
  the gateway DHT-walks. The `Eventual { ttl_ms = 30_000 }` default
  amortises the cost; operators who need sub-100 ms cold reads must
  run their own kubo and pay the $20-25/mo.
- **Operator now runs a daemon.** kubo on the VM is one more piece
  of long-lived infra to monitor, snapshot, patch. ADR-001's
  "clone-and-run" promise applies to ruLake itself, not to the IPFS
  backend's deployment dependencies. The annex is mandatory reading.
- **Two anchors, one cache key.** `W_pre` (the GCS-flavoured bundle)
  and `W_post` (the IPFS-flavoured bundle) are different witnesses
  for the same vectors. v0.1 doesn't alias them; cache sharing
  across the GCS and IPFS views of the same data is *not*
  automatic. Documented as a v0.2 reopener.
- **Pin lifecycle is the operator's job.** No automatic GC. Bundles
  pile up on the kubo blockstore until somebody removes them. Ships
  with a `Datastore.StorageMax 30GB` cap so the daemon at least
  fails loud rather than filling the disk; the operator owns the
  prune script.

### Neutral

- **Sibling-crate count goes from 4 to 5.** `python/`, `node/`,
  `mcp-server/`, `gcs-backend/`, `ipfs-backend/`. CI gains one more
  matrix row; ADR-001's "no workspace" rule absorbs it.
- **Cargo dep tree gains `ipfs-api`, `cid`, `multihash`, `multibase`,
  `reqwest`.** Most of those are pure-Rust, MIT/Apache-licensed; no
  novel licence concerns. The hyper / rustls / tokio overlap with
  `gcs-backend/` and `mcp-server/` means most of the bytes are
  already in the binary if those crates are also linked.
- **The MCP `transfer_ipfs-resolve` tool stays out of the read
  path.** It's an IPNS resolver and remains useful as an operator-
  facing helper for naming bundles; it's not a substitute for the
  backend's kubo client.

### Verification (acceptance for the PR that lands `ipfs-backend/`)

```text
$ cargo build --release -p ruvector-rulake-ipfs
   Compiling ipfs-api-backend-hyper v0.6.x
   Compiling reqwest v0.12.x
   Compiling cid v0.11.x
   Compiling ruvector-rulake-ipfs v0.1.0 (ipfs-backend)
    Finished `release` profile in 22.x s

$ cargo test -p ruvector-rulake-ipfs
   test backend::publish_then_read_roundtrips_through_fake_kubo                  ok
   test backend::current_bundle_issues_exactly_one_cat_call                      ok
   test backend::current_bundle_returns_opaque_generation_with_cid               ok
   test backend::tampered_bundle_bytes_fail_witness_check                        ok
   test backend::oversized_bundle_rejected_before_parse                          ok
   test backend::gateway_only_mode_refuses_publish                               ok
   test backend::gateway_fallback_emits_warn_line                                ok
   test client::kubo_bearer_header_is_sent                                       ok
   test client::gateway_429_triggers_rotation_to_next_endpoint                   ok
   test client::cloudflare_gateway_not_in_default_list                           ok
   test cid::cidv1_base32_default_roundtrips_with_kubo_response                  ok
   test pin::publish_pins_then_optionally_pushes_to_pinning_service              ok
   ... 16 passed; 0 failed

# Live tests, operator runs once.
$ RULAKE_IPFS_LIVE_TEST=1 \
  RULAKE_IPFS_KUBO_RPC=http://10.0.0.5:5001 \
  RULAKE_IPFS_KUBO_BEARER=... \
  cargo test --release -- --ignored ipfs_live
   test ipfs_live::publish_a_real_bundle_then_read_it_back                       ok (1.7s)
   test ipfs_live::current_bundle_under_100ms_p50_against_local_kubo             ok
   test ipfs_live::cross_check_via_public_gateway_reads_same_bytes               ok
```

A bench gate, mirroring the GCS-backend §M2 acceptance:

> `IpfsBundleBackend::current_bundle()` against a freshly-published
> bundle on a local kubo daemon completes in `≤ 50 ms p50, ≤ 200 ms
> p99` over 1 000 calls. Against a public gateway, the same call
> completes in `≤ 500 ms p50, ≤ 5 s p99` (variance dominated by
> gateway-side DHT health, hence the wide p99).

The cache-hit path through `RuLake::search_one` against a collection
backed by an IPFS bundle stays within ADR-155's 1.02× tax envelope —
because once the bundle has been read once and the cache is primed,
the IPFS layer never participates in the search hot path. Cache-miss
paths are bounded by the gateway numbers above.

## Open questions

### Resolved by this ADR

- **Crate placement.** `ipfs-backend/` sibling, no workspace.
- **Library.** `ipfs-api-backend-hyper` for kubo RPC; `reqwest` for
  the public-gateway path; `cid` / `multihash` / `multibase` for
  hash arithmetic.
- **What's on IPFS.** Bundles only in v0.1. Vector bodies are out
  of scope until v0.3 with a real workload.
- **Witness vs CID.** Different hashes; the witness is the cache
  anchor (SHAKE-256(32) per the existing format), the CID lands in
  `Generation::Opaque(...)` and the bundle's `data_ref` becomes
  `ipfs://<cid>`. Two-witness model documented; aliasing deferred.
- **Operating modes.** kubo (default), gateway-only,
  kubo+pinning-service. Pinning-service is a Cargo feature so it's
  off in the minimal build.
- **Public-gateway fallback.** Off by default; opt-in via
  `with_gateway_fallback(GatewayPolicy::OnKuboError { ... })`;
  Cloudflare's decommissioned gateway is excluded from the default
  list. Every fallback hit is a structured `tracing::warn!`.
- **Auth on kubo RPC.** Bearer via kubo's
  `API.Authorizations`/`HTTPAuthSecrets`, plus IAP-tunneled SSH for
  ops, plus a private-IP-only RPC binding with a source-tag
  firewall rule.
- **Deployment shape.** Compute Engine `e2-small`, `pd-balanced`
  50 GB, no public IP, IAP TCP forwarding for SSH, public swarm
  port, private RPC. ~$20-25/mo all-in.
- **Kubo version pin.** v0.40.1 (current as of 2026-04-26).
  Operator bumps with `apt`-style atomicity; ADR refresh happens
  if a future kubo introduces a new RPC version we want.
- **Pin lifecycle.** Operator-managed. v0.1 ships
  `Datastore.StorageMax 30GB` so the daemon fails loud; we don't
  ship an auto-prune.

### v0.2 (post-first-real-deployment)

1. **Witness aliasing across naming axes.** When the same vectors
   live behind both a GCS bundle (`W_pre`) and an IPFS bundle
   (`W_post`), should the cache treat them as the same entry? Likely
   yes, but the alias mechanism is a trust boundary (someone has to
   assert the equivalence) and we don't speculatively design those.
2. **IPNS in the publish path.** Operator-facing helper to publish a
   stable `/ipns/<key>` that points at the latest bundle CID. Read
   path still resolves CIDs only. Useful when the operator wants
   external consumers to subscribe to "the bundle for collection X"
   without re-broadcasting CIDs.
3. **Pinning-service `pins?status=...` listing + automated prune.**
   Once the operator has pinned more than they meant to.
4. **`ipfs-cluster` mode.** When single-node availability isn't
   enough.
5. **Body-on-IPFS** as an opt-in shape with a sized workload.
6. **Encryption envelope** as a first-class backend feature (not
   just an example) with a key-handle abstraction so different
   secret-managers slot in.
7. **Source-tag-only RPC binding** as a hardening default — today
   the operator chooses between localhost-only (safe) and
   private-IP-with-firewall (convenient); v0.2 could bake the
   IAP-fronted private path as a recipe.

### v1.0 (orthogonal but adjacent)

1. **iroh as an embedded backend.** When iroh's IPFS-compat layer
   lands, an `iroh` Cargo feature could remove the kubo dependency
   for operators who'd rather embed than deploy. The ADR-005
   shape lets us swap the client without changing the trait impl.
2. **A read-only public CID resolver in `mcp-server`.** A new
   `intent: "fetch_bundle_by_cid"` could let agents pull a known
   bundle through ruLake's MCP surface, with the planner enforcing
   the witness check. Belongs in an ADR-004 amendment, not here.
3. **Multi-pinning-service pushes.** Pinata + Filebase + Storacha
   as a redundancy set. The Pinning Services API spec is uniform
   enough that a `Vec<PinningService>` config + parallel push is
   a small change.

## References

- IPFS pinning services API spec (OpenAPI 1.0.0):
  [`github.com/ipfs/pinning-services-api-spec`](https://github.com/ipfs/pinning-services-api-spec),
  rendered at [`ipfs.github.io/pinning-services-api-spec/`](https://ipfs.github.io/pinning-services-api-spec/)
- Kubo RPC API v0 reference: [`docs.ipfs.tech/reference/kubo/rpc/`](https://docs.ipfs.tech/reference/kubo/rpc/)
- Kubo TLS + HTTP auth guide (the `HTTPAuthSecrets` shape used in §10):
  [`docs.ipfs.tech/how-to/kubo-rpc-tls-auth/`](https://docs.ipfs.tech/how-to/kubo-rpc-tls-auth/)
- Kubo v0.40.x release notes: [`github.com/ipfs/kubo/releases`](https://github.com/ipfs/kubo/releases)
- IPFS public utilities (current public gateway list, post-Cloudflare-decommission):
  [`docs.ipfs.tech/concepts/public-utilities/`](https://docs.ipfs.tech/concepts/public-utilities/)
- ProbeLab gateway analytics (the source for the gateway latency
  bands in §7): [`probelab.io/ipfs/gateways/`](https://probelab.io/ipfs/gateways/)
- IPFS Foundation gateway operator note (Interplanetary Shipyard):
  [`about.ipfs.io/`](https://about.ipfs.io/)
- Cloudflare's IPFS-gateway decommission notice (May → August 2024):
  [`blog.cloudflare.com/cloudflares-public-ipfs-gateways-and-supporting-interplanetary-shipyard/`](https://blog.cloudflare.com/cloudflares-public-ipfs-gateways-and-supporting-interplanetary-shipyard/)
- `ipfs-api-backend-hyper` crate: [`crates.io/crates/ipfs-api-backend-hyper`](https://crates.io/crates/ipfs-api-backend-hyper)
- `cid` crate (CIDv1 base32 by default in kubo since 0.5+):
  [`crates.io/crates/cid`](https://crates.io/crates/cid),
  context PR [`github.com/ipfs/kubo/pull/6300`](https://github.com/ipfs/kubo/pull/6300)
- iroh's "embedded IPFS" status note (April 2026): [`iroh.computer/docs/ipfs`](https://www.iroh.computer/docs/ipfs)
- Storacha (post-rebrand of web3.storage) docs: [`docs.storacha.network/`](https://docs.storacha.network/)
- Pinata pricing + PSA endpoint: [`pinata.cloud/`](https://pinata.cloud/)
- Filebase (S3-compatible IPFS, PSA-compatible): [`filebase.com/`](https://filebase.com/),
  pricing [`filebase.com/pricing/`](https://filebase.com/pricing/)
- Cloud IAP TCP forwarding (the SSH-without-bastion shape used in §9):
  [`docs.cloud.google.com/iap/docs/using-tcp-forwarding`](https://docs.cloud.google.com/iap/docs/using-tcp-forwarding)
- Cloud Storage FUSE limitations (why Cloud Run was rejected for the
  blockstore in §9.1): [`docs.cloud.google.com/storage/docs/cloud-storage-fuse/overview`](https://docs.cloud.google.com/storage/docs/cloud-storage-fuse/overview)
- Existing GCS backend this ADR mirrors structurally: `gcs-backend/`
  (commit `c706dc6`); see `gcs-backend/Cargo.toml`,
  `gcs-backend/src/backend.rs`, `gcs-backend/README.md`.
- Public Rust surface this backend implements: `BackendAdapter` at
  `src/backend.rs:110-146`; `current_bundle` default impl at
  `src/backend.rs:125-141` (the one we override).
- Bundle protocol this backend serves over the wire: `src/bundle.rs`
  — witness format `src/bundle.rs:362-390`, variant tag for the
  `Generation` enum `src/bundle.rs:71-97`, witness-fail-closed
  guard `src/bundle.rs:349`, JSON size + field caps
  `src/bundle.rs:215-262`.
- Related decisions: ADR-155 §M2-M5 backend roadmap, ADR-004 §Resources
  contract, ADR-001 sibling-crate discipline.
