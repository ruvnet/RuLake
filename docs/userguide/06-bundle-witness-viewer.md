# 06 — Bundle / witness viewer

The Bundle screen is the load-bearing trust surface. Every cached entry in
ruLake is anchored by a SHAKE-256(32) witness over a small canonical
descriptor — the bundle. This screen renders that descriptor, recomputes
the witness in your browser, and compares the two. They must match. That
is the contract.

![Bundle viewer with key-value descriptor on the left, witness comparator showing BIT-EXACT MATCH, generation chain, and a printed receipt on the right](../../assets/console-bundle.png)

## What is a bundle?

`RuLakeBundle` is a ~300-byte JSON sidecar called `table.rulake.json`. It
travels with the data — in object storage next to the Parquet, on IPFS
under its own CID, or inline in the MCP response. The struct (in
`crates/core/src/bundle.rs`) carries:

| Field | What it pins | Notes |
|---|---|---|
| `format_version` | The bundle schema | Currently `2`; included in the witness preimage so a v1 and v2 bundle with otherwise-identical fields produce different witnesses. |
| `data_ref` | URL of the actual vector bytes | e.g. `rvf://seg/0x11/<hex>`, `gs://bucket/key`, `ipfs://<cid>`. |
| `dim` | Vector dimensionality | |
| `rotation_seed` | Hadamard rotation seed | `u32`; pinned because it changes how vectors are pre-projected. |
| `rerank_factor` | Per-shard re-rank multiplier | Float; affects recall-vs-latency. |
| `generation` | Either `Num(u64)` or `Opaque(<bytes>)` | Numeric for monotonic stores (mtime, GCS gen); opaque for content-addressed. |
| `pii_policy` | e.g. `class:public`, `class:internal` | Surfaced to MCP policy hooks. |
| `lineage_id` | Stable id for the vector lineage | Unchanged across re-publish if the source pipeline is the same. |
| `memory_class` | `durable`, `genomic`, `quantum`, ... | Substrates use this to tag their bundles. |

The witness is a SHAKE-256 hash to 32 bytes over a deterministic
serialisation of all the above plus the `format_version` tag byte. It is
the load-bearing contract: identical bundle inputs ⇒ identical witness ⇒
identical query results.

## Reading the screen

### Header strip

Breadcrumb shows `rulake://bundle / <backend> / <collection>`. The header
has three buttons:

- **Pinned** — open the saved-bundles modal.
- **Pin witness** — save the current `(backend, collection, generation,
  witness)` tuple to IndexedDB. Useful as an immutable handle to compare
  against later.
- **Recompute witness** — runs `rulake-wasm`'s `computeWitness(...)`
  in-browser, then sets `verified` based on whether the result matches the
  server-reported value. The audit tail gains `WITNESS_MATCH`,
  `WITNESS_MISMATCH`, or `WITNESS_COMPUTE_ERROR`.

### IPFS fetch strip

Below the header sits a paste-a-CID strip. Workflow:

1. Paste an IPFS CID (e.g. `bafy…`).
2. Click **Fetch from gateway**.
3. The Console fetches the bundle JSON from the configured public gateway
   (default `https://ipfs.io/ipfs/<cid>`).
4. `rulake-wasm` recomputes the witness over the fetched bytes.
5. The audit tail gains `IPFS_BUNDLE_VERIFIED` or `IPFS_WITNESS_MISMATCH`.

This is the smallest end-to-end demonstration of the cross-process trust
chain: the bytes came from an untrusted network, your browser hashed them,
and you got either MATCH or MISMATCH locally without trusting the gateway.

### Field table

The middle of the page renders the bundle JSON as a key/value table, one
row per field listed above. Click any value to copy it.

### Witness comparator

Three rows, all on the same canonical hex format:

- **Server** — what the cache reported.
- **Recomputed** — what `rulake-wasm` produced from the bundle bytes.
- **Δ** — `● BIT-EXACT MATCH` (green) or `MISMATCH` (amber).

The recomputed row goes grey with `· · · computing in browser · · ·`
during the recompute, then snaps back to the canonical hex.

### Generation chain · lineage

A small table of the most recent five generations of this collection:

| Column | Meaning |
|---|---|
| `Gen` | `Num(N)` or `Opaque(<hex>)`; head row marked `← head` |
| `Issued` | UTC timestamp |
| `Δ entries` | Net entries added/removed since the previous gen |
| `Witness` | First 16 hex of the SHAKE-256 |
| `Verified` | Always `●` (green) if it was successfully verified at publish time |

This is the audit chain operators care about: a generation is published,
its witness is checkpointed, and the chain is the immutable record of
"what bytes did this collection have, and when".

### Bundle receipt (right pane)

The receipt on the right is a printable summary — `format`, `dim`, `gen`,
`entries`, `pii`, then a divider, then `rotation`, `rerank`, `class`. The
big stamp at the bottom is `✓ MATCH` (green) when verified, `VERIFYING`
(grey) during recompute. The witness footer prints the full 64-hex.

## Verifying a bundle outside the Console

The same recompute path is one function call from anywhere
`rulake-wasm` runs:

```js
import init, { computeWitness } from 'rulake-wasm';
await init();

const witness = computeWitness(
  'rvf://seg/0x11/9ad...e72',  // data_ref
  1024,                         // dim
  0xa17c,                       // rotation_seed
  1.5,                          // rerank_factor
  { Num: 7741 },                // generation
);
// witness is a 64-char hex string. Compare to the server-reported value.
```

From Rust, the same is `RuLakeBundle::new(...).rvf_witness()`.

## What MISMATCH actually means

A mismatch is not "the data is wrong". It means *the bundle the cache is
serving was anchored to different bytes than the descriptor says*. Three
common causes:

1. **Tampered sidecar** — somebody edited `table.rulake.json` after publish.
2. **Schema mismatch** — the publisher used a different `format_version`,
   or a different rotation seed/rerank factor than what the server
   advertised.
3. **CID-substitution** — for IPFS bundles, the bundle was re-pinned under
   a different CID. `IpfsBackend::fetch_bundle` hard-refuses this with
   `IPFS_BUNDLE_CID_MISMATCH` (see CHANGELOG R-IPFS-1).

In all three cases the right move is to refuse, audit, and surface the
collection's `(backend, collection)` to the operator. The Console does the
first two automatically.
