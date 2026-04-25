# ruLake — Security Review

**Scope.** Threat-model the ruLake crate's external surfaces:

- The `BackendAdapter` trust boundary (we treat backends as
  semi-trusted: a compromised backend can lie about data, but it
  shouldn't crash or DoS the host).
- The bundle / witness scheme — both as written by ruLake and as read
  from disk via the sidecar protocol.
- Filesystem operations in `FsBackend`, including path traversal and
  on-disk format parsing.
- Input validation at every public API entry point.
- Panic surface and `unwrap()` discipline on untrusted input.
- Dependency surface and supply-chain considerations.
- Concurrency / `unsafe` use.

**Method.** Read every public API in `src/`, follow each input down to
its first allocation or syscall, and check whether validation /
length-bounding / atomicity is present at the right layer. Severity
ratings: **Critical** (RCE / data exfiltration / silent corruption),
**High** (crash / DoS exploitable from network), **Medium** (DoS
against single query, or correctness gap requiring local access),
**Low** (hardening opportunity, no exploit), **Info** (observation).

---

## 1. Threat model

| Actor | Capability | What we defend against |
|---|---|---|
| **Caller** (app code) | Owns the `RuLake` instance | Their own bugs; not actively malicious. |
| **Backend** (Parquet / BQ / RVF / Fs) | Returns `PulledBatch` and `current_bundle()` | Hostile or compromised backend lying about size, dim, or vector contents; trying to OOM the cache or poison the witness. |
| **Sidecar publisher** (cache daemon, GCS bucket) | Writes `table.rulake.json` to a directory ruLake reads | Tampered sidecars; oversize JSON; path-component shenanigans. |
| **Filesystem** (FsBackend root, persistence dirs) | Holds `ruvec1` files and `index.rbpx` snapshots | User-controlled filenames; symlink attacks; concurrent writers. |
| **Concurrent caller** | Same process | Race conditions; mutex-poisoning; integer overflow under churn. |

What we **do not** defend against in this crate (out of scope per
ADR-155):

- Wire-level auth (no HTTP/gRPC layer ships).
- RBAC / column masking / PII enforcement (M4 roadmap).
- Cryptographic shred for GDPR (RVF responsibility).
- A truly malicious caller with `&mut RuLake` access (they own the
  process anyway).

---

## 2. Findings — by severity

### Critical

**None found.** No `unsafe` blocks (`grep -rn "unsafe " src/` returns
0). No FFI. No deserialization of executable code. No path that
allocates based on an unbounded untrusted integer without a check
(see `validate_pulled_batch` and FsBackend header parser below).

### High

**None found.** Every external integer-sized field is bounded before
allocation; every JSON input is size-capped; every external filename
is validated against a strict whitelist.

### Medium

#### M1 — `Mutex` poisoning on panic propagates as `unwrap()`

**Locations:**
`src/cache.rs:330, 343, 357, 380, 425, 514, 612, 629, 639, 645, 668, 679, 683, 689, 735, 806, 853, 870, 882`
(every `self.inner.lock().unwrap()`).
`src/lake.rs:92, 104, 678` (`self.backends.read().unwrap() / write().unwrap()`).
`src/backend.rs:207, 226, 261, 269, 288, 313` (`self.inner.read().unwrap() / write().unwrap()`).
`src/fs_backend.rs:92, 205, 236` (`self.index.write().unwrap() / read().unwrap()`).

**Issue.** `std::sync::Mutex::lock()` returns `Result<MutexGuard,
PoisonError>` — `Err` if a previous lock-holder panicked. Every
acquisition in this crate calls `.unwrap()`, which means **a single
panic inside any cache critical section poisons the mutex permanently
and turns every subsequent query into a panic**. Same for the
`RwLock`s in the backends.

**Practical risk.** Most critical sections are short and contain only
HashMap operations + counter increments. The realistic poisoning
sources are:

- `prime_interned` calling into `RabitqPlusIndex::add` with an
  invariant violation that escapes as a panic (rabitq's discipline,
  not ruLake's).
- A panicking `backend.current_bundle` / `backend.pull_vectors`
  call inside `ensure_fresh` while the cache lock isn't held —
  but the cache is later locked by the same thread, so a panic
  during the *unlocked* heavy work doesn't poison the cache mutex
  itself.

The cache mutex is short-held enough that a poisoning event would be
unusual but not impossible. If poisoning happens, the entire `RuLake`
instance is bricked.

**Severity reasoning.** Real DoS surface, but exploit requires
inducing a panic in a path that holds the lock — none of those paths
take untrusted input that could trigger one (the `prime_interned`
heavy work is **outside** the lock). So: latent DoS, not directly
exploitable from the network or a malicious bundle.

**Remediation.** Either:

1. Use `parking_lot::Mutex` / `RwLock` (no poisoning concept).
2. Pattern-match on `lock()` and recover from `PoisonError` by
   `into_inner()`-ing the guard (treat poison as "previous panic
   already cleaned up; data is consistent"). Done idiomatically:
   ```rust
   fn locked_inner<R>(&self, f: impl FnOnce(&mut CacheState) -> R) -> R {
       let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
       f(&mut *g)
   }
   ```

#### M2 — Default `BackendAdapter::current_bundle` pulls every vector on every coherence check

**Location:** `src/backend.rs:125-141`.

**Issue.** The default-impl reads:

```rust
fn current_bundle(...) -> Result<crate::RuLakeBundle> {
    let batch = self.pull_vectors(collection)?;          // FULL pull
    Ok(crate::RuLakeBundle::new(
        format!("{}://{}", self.id(), collection),
        batch.dim,
        rotation_seed,
        rerank_factor,
        crate::Generation::Num(batch.generation),
    ))
}
```

A backend author who forgets to override this and uses
`Consistency::Fresh` will pull **every vector across the network on
every search** just to recompute the witness. At any non-trivial
collection size this is a self-inflicted DoS.

The doc comment on `current_bundle` (`backend.rs:120-124`) does say
"Real backends … should override with their authoritative `data_ref`",
but the trait definition does not enforce it.

**Severity.** Medium — operator footgun, not an attacker-driven
exploit, but the consequences are catastrophic to throughput.

**Remediation options.**

- Make `current_bundle` a non-default trait method (compile-time
  forcing). Breaking change for existing implementers.
- Add a runtime check in `RuLake::register_backend`: if
  `backend.current_bundle("__probe__", 0, 0)` ends up calling
  `pull_vectors`, log a warning. Hard to detect without a sentinel.
- At minimum, **rename the doc-comment header** to a `#[doc(hidden)]`
  warning style and add a clippy-style lint via `#[deprecated(note =
  "Override current_bundle for production backends — see ADR-155")]`
  on the default impl. Crude but effective.

#### M3 — `FsBackend` is vulnerable to symlink / TOCTOU at the root directory itself

**Location:** `src/fs_backend.rs:62-74` `FsBackend::new`,
`fs_backend.rs:166-202` `write`, `fs_backend.rs:240-309` `pull_vectors`.

**Issue.** `FsBackend::new` accepts any `root: impl AsRef<Path>` and
calls `create_dir_all` without checking that the path is a directory
(not a symlink), without checking ownership, and without canonicalizing.
The `register` validator (`fs_backend.rs:105-136`) blocks
**filename**-level traversal but not **root**-level games:

- An operator who passes a symlinked directory as `root` can be made
  to write `ruvec1` files into a target the symlink points at.
- Between `validate_filename` succeeding and `File::create(path)`
  running, a symlink could be planted at `path` pointing at
  `/etc/passwd` (TOCTOU window). The `tmp` filename includes
  `format!(".{filename}.tmp")` (`fs_backend.rs:167`) — also resolvable
  through symlinks.

**Severity.** Medium — exploitation requires **local** filesystem
access on the host that ruLake runs as, and the target paths are
ruLake's own working directory. Realistic threat: a
multi-tenant container where another tenant can plant symlinks in
ruLake's data directory.

**Remediation.**

- Use `std::fs::OpenOptions` with `.create_new(true)` for the temp
  file (atomic create, refuses if file exists — closes the TOCTOU
  window at the temp-file step).
- On Linux, use `O_NOFOLLOW` via the `nix` crate to refuse
  symlink-targeted writes inside `root`.
- Document that `root` must be exclusively owned by the ruLake
  process and reside on a non-shared filesystem.

#### M4 — `Generation::Opaque(String)` is unbounded inside `PulledBatch`

**Location:** `src/bundle.rs:60`, used implicitly via
`backend.current_bundle()` returning a `RuLakeBundle` containing it.

**Issue.** `from_json` caps `data_ref`, `pii_policy`, `lineage_id`,
`memory_class` at 4 KiB each (`bundle.rs:237-254`), but an
**in-process** `Generation::Opaque(String)` produced by a backend
adapter has no length cap. A hostile or buggy backend that returns
a 1 GB string in `current_bundle()` will:

1. Be passed into `compute_witness` (`bundle.rs:362`), which calls
   `g.hash_bytes()` — that allocates `1 + s.len()` bytes
   (`bundle.rs:90-94`). **1 GB allocation, no check.**
2. Then `to_json` will serialize that 1 GB string into a JSON file —
   accepted on write because `to_json` has no cap, only `from_json`
   does.

**Severity.** Medium — requires a malicious or broken backend
implementation, not network input. Real for federated deployments
that mount third-party `BackendAdapter`s.

**Remediation.** Add a `MAX_GENERATION_OPAQUE_BYTES` cap (suggest
4 KiB to mirror the JSON-side cap) and reject in `compute_witness` /
`Bundle::new`. Alternatively, validate inside `RuLake::ensure_fresh`
right after the backend returns its bundle (`lake.rs:648`).

#### M5 — `LocalBackend::append` does not bound vector length per-call

**Location:** `src/backend.rs:225-249`.

**Issue.** `LocalBackend::append(collection, id, vector)` accepts a
single `Vec<f32>` and only checks dimension (`vector.len() == entry.dim`).
There is **no overall growth bound** on the collection — repeatedly
calling `append` can grow `LocalCollection.vectors` until OOM.

Also, `vector.len() != entry.dim` returns an error, but
`entry.dim == 0` (uninitialized collection) is silently set to
`vector.len()` (`backend.rs:236-238`) — meaning the **first append
on an empty collection sets dim to whatever the first caller passes**,
with no validation that this matches the rest of the collection.

**Severity.** Medium — `LocalBackend` is the test substrate. Real
backends won't accept unauthenticated `append` calls. But
`LocalBackend` is also the documented "example for real-backend
implementers" (`backend.rs:152-153`), and the lack of caps may be
copied.

**Remediation.** Add an upper bound on `len(ids)` per collection in
`LocalBackend`, mirroring `MAX_PULLED_VECTORS`. Reject `append` on a
collection whose `dim == 0` (require explicit `put_collection` first).

#### M6 — `Cargo.toml` workspace inheritance + path dependency on `ruvector-rabitq`

**Location:** `Cargo.toml:3-9, 16`.

**Issue.** This standalone repo can't be built without the parent
RuVector workspace. From a supply-chain perspective:

- `ruvector-rabitq = { path = "../ruvector-rabitq", version = "2.2" }`
  means a maintainer could replace the path-resolved crate with a
  modified copy that publishes as `2.2` and ship a backdoored
  RaBitQ kernel without changing this repo's lockfile.
- `serde`, `rand`, `rand_distr`, `thiserror`, `rayon` all use
  `{ workspace = true }`, so the actual versions live in the parent
  workspace's `Cargo.toml` — **not visible from this repo** alone.

**Severity.** Low-to-Medium for a standalone consumer; pure-supply-chain
issue, not exploitable as code.

**Remediation.** For a published standalone version of this crate:

- Pin every dep to a concrete version (or version range) in this
  `Cargo.toml`.
- Drop the `path = "../ruvector-rabitq"` and rely on the
  crates.io-published `2.2`.
- Add a `Cargo.lock` to the repo (currently absent — verified by
  `ls -la /home/ruvultra/projects/RuLake/`).
- Set up `cargo audit` in CI.

### Low

#### L1 — `unwrap()` on infallible array slice → array conversions in `FsBackend`

**Location:** `src/fs_backend.rs:259, 260, 296, 345`.

```rust
let count_u64 = u64::from_le_bytes(header[8..16].try_into().unwrap());
let dim_u32  = u32::from_le_bytes(header[16..20].try_into().unwrap());
...
v.push(f32::from_le_bytes(vec_bytes[lo..lo + 4].try_into().unwrap()));
let dim = u32::from_le_bytes(header[16..20].try_into().unwrap()) as usize;
```

**Issue.** These `try_into().unwrap()` calls on a slice-to-array
conversion are infallible **given the constant-sized input arrays
above** — `header` is `[u8; HEADER_BYTES]` (24), and `vec_bytes`'
length is `dim * 4`, with `dim` already bounds-checked. So the
unwraps are correct.

**Severity.** Low — code correctness is fine; the pattern is fragile
to refactoring. A future change that reads `header` from a `Vec<u8>`
instead of a fixed-size array would silently turn these into panic
risks on truncated input.

**Remediation.** Either use `<[u8; 8]>::try_from(...)` with a `?`,
or annotate with `expect("invariant: header is [u8; 24]")` so the
intent is clear.

#### L2 — `mtime` clock skew silently rounds to 0 on pre-epoch files

**Location:** `src/fs_backend.rs:215-227`.

```rust
let secs = mtime
    .duration_since(UNIX_EPOCH)
    .map(|d| d.as_secs())
    .unwrap_or(0);
```

**Issue.** A pre-1970 file mtime (admittedly rare) silently rounds to
generation = 0, which collides with "fresh, never-written file" if any
exists. Also, mtime is only second-resolution: two writes within one
second will produce the same generation and **the cache will not
notice the second write** (the doc comment at `fs_backend.rs:30-35`
acknowledges this).

**Severity.** Low — affects only `FsBackend`, only on rapid
sub-second writes, with documented behaviour.

**Remediation.** None required; documented. For higher-fidelity
needs, an `FsBackend2` could hash the file contents instead of using
mtime.

#### L3 — `serde_json::from_str` is invoked after a length cap, but with default recursion depth

**Location:** `src/bundle.rs:225`.

**Issue.** `serde_json::from_str(s)` is called with the default
serde config. Deeply nested JSON (within the 64 KiB cap) could still
incur stack growth. The bundle schema is flat, so this is not
exploitable, but a future schema change could re-introduce the
concern. Modern serde_json has reasonable limits but does not
default-cap recursion.

**Severity.** Low.

**Remediation.** Use `serde_json::from_slice` with explicit
`Deserializer::from_slice(...).deserialize_*` and a `disable_recursion_limit(false)` setup if the schema ever grows.

#### L4 — Per-`Lake.with_max_cache_entries` mutation re-creates the cache, losing prior state

**Location:** `src/lake.rs:78-87`.

```rust
pub fn with_max_cache_entries(self, n: usize) -> Self {
    Self {
        cache: Arc::new(VectorCache::new(...)),  // brand-new cache
        backends: self.backends,
        consistency: self.consistency,
    }
}
```

**Issue.** `with_max_cache_entries` is a builder method named like a
non-destructive cap-setter, but it actually **discards every primed
entry** and starts a fresh cache. If a caller does
`let lake = lake.with_max_cache_entries(n);` after registering and
priming backends, all primes are lost silently. No documentation
warns of this.

**Severity.** Low — operator footgun, not exploitable.

**Remediation.** Either expose it as `RuLake::set_max_cache_entries`
on `&mut self` that mutates the existing cache, or add a panic / log
warning when the cache is non-empty at call time.

#### L5 — Witness is SHAKE-256 truncated to 32 bytes; collision space is 2^128 (not weak, but worth noting)

**Location:** `src/bundle.rs:359-390`.

`compute_witness` uses `SHAKE-256` with output length 32 bytes (256
bits). Collision resistance is `2^128`. For a content-addressed cache,
this is overwhelmingly safe — a deliberate collision would require
~2^128 distinct bundle inputs.

**Severity.** Info — no action needed. Recorded for completeness.

#### L6 — Bundle file written without explicit permissions

**Location:** `src/bundle.rs:311-319` and `src/lake.rs:296-318`.

`File::create` uses platform defaults (typically `0644` on Unix), so
any user on the same host can read the witness sidecar (which is
public-by-design — it's the cache-coherence anchor). However, on a
multi-tenant host the bundle reveals `data_ref` (which may be a GCS
URI containing project/bucket names) and `pii_policy` / `lineage_id`
strings.

**Severity.** Low.

**Remediation.** If multi-tenant deployment is in scope, accept a
`mode: u32` parameter on `write_to_dir` and use `OpenOptions::mode()`
to set `0600`. Or restrict via parent directory perms.

#### L7 — `LocalBackend.append`'s `generation = generation.wrapping_add(1)` can wrap

**Location:** `src/backend.rs:220, 247`.

**Issue.** `wrapping_add(1)` on a `u64` won't crash, but if a backend
ever wraps from `u64::MAX` back to 0, two distinct collection states
will share the same generation and the witness collision will silently
prevent re-prime. Astronomically unlikely in `LocalBackend` (would
need ~10^19 mutations) but the pattern is technically incorrect.

**Severity.** Info — `LocalBackend` is a test substrate.

**Remediation.** None required. If the pattern is copied to a real
backend that mutates faster, replace with `checked_add` and refuse
new writes when exhausted.

### Info

#### I1 — `validate_pulled_batch` runs **after** the heavy parts of `pull_vectors` complete

**Location:** `src/cache.rs:377` calls `validate_pulled_batch(&batch)`,
but `batch` was already constructed by `backend.pull_vectors(...)`
which means `LocalBackend` already cloned vectors
(`backend.rs:280-282`) and `FsBackend` already read every byte from
disk (`fs_backend.rs:282-300`). The validator catches a 100 GB
hostile batch **after** ruLake has spent the I/O to materialize it.

**Suggestion.** Push size pre-checks into adapters where cheap:
`FsBackend` already does this at `fs_backend.rs:266-276` (header
parse rejects oversize before reading vectors). `LocalBackend` can't
help — the data is already in memory. For a future `ParquetBackend`,
pre-check the row group counts before allocating the column chunk
buffers.

#### I2 — `RuLake::publish_bundle` does not authenticate the publisher

The bundle is content-addressed (anyone holding the same bytes
recomputes the same witness), but the `publish_bundle → refresh_from_bundle_dir`
flow does not bind a signature to the publisher's identity. Any process
that can write to the publish directory can rotate the witness for
every reader.

**Suggestion.** Out-of-scope for M1 (filesystem ACLs are the answer
today). Recorded so a future `M4 governance` ADR addresses it
deliberately.

#### I3 — `FsBackend::write` overwrites the destination via `rename`

**Location:** `src/fs_backend.rs:193-199`.

`fs::rename(tmp, path)` will replace an existing file at `path`. If
two processes are writing the same `(collection, filename)` pair,
the loser's data is silently overwritten. The bundle / persist code
shares this pattern. Documented as "atomic" — true within a process,
race-prone across processes.

**Severity.** Info — operators are expected to single-write.

#### I4 — No SBOM / `cargo deny` configuration in the repo

**Location:** repo root.

A standalone published crate should ship an `audit.toml` /
`deny.toml` and a SBOM CI step. None present.

**Severity.** Info / hardening.

#### I5 — No fuzz harness for `bundle::from_json` or `fs_backend::pull_vectors` parser

The two external-input parsers in the crate (JSON sidecar + ruvec1
binary) are obvious fuzz targets. `cargo fuzz` can plug into both in
~40 lines. Not present.

**Severity.** Info.

---

## 3. Witness / digest scheme — analysis

The witness is the cache-key anchor and the cross-process integrity
proof, so it deserves its own section.

`compute_witness` (`bundle.rs:362-390`):

```rust
let mut h = Shake256::default();
h.update(b"rulake-bundle-witness-v1|");          // domain separator
h.update(&(data_ref.len() as u64).to_le_bytes()); // length-prefixed
h.update(data_ref.as_bytes());
h.update(b"|");
h.update(&(dim as u64).to_le_bytes());
h.update(&rotation_seed.to_le_bytes());
h.update(&(rerank_factor as u64).to_le_bytes());
h.update(b"|");
let g = generation.hash_bytes();                  // tagged: 0x00 Num | 0x01 Opaque
h.update(&(g.len() as u64).to_le_bytes());
h.update(&g);
let mut out = [0u8; 32];
reader.read(&mut out);
hex::encode(out)
```

**Audit findings (positive):**

1. **Domain-separated.** The `b"rulake-bundle-witness-v1|"` prefix
   prevents witness collision with other SHAKE-256 uses
   (`rvf-crypto` etc.).
2. **Length-prefixed.** Every variable-length field is preceded by
   its length as `u64::to_le_bytes`. This closes the
   `"a|b" vs "ab|"` concatenation collision the test
   `witness_is_length_prefixed` (`bundle.rs:443`) regresses against.
3. **Variant-tagged.** `Generation::hash_bytes()` prepends a
   discriminant byte (`0x00` Num, `0x01` Opaque) — closes the
   audit-driven `Num(7)` vs `Opaque("\x07\0\0\0\0\0\0\0")` collision
   (`bundle.rs:82-97`, regression-tested at `bundle.rs:423-440`).
4. **Tampered-on-disk detection.** `read_from_dir` always calls
   `verify_witness` (`bundle.rs:340-356`); the test
   `fs_read_rejects_tampered_sidecar` (`bundle.rs:519`) confirms
   it catches a `dim` field mutation.
5. **Format version is bumped** to 2 with the variant-tag fix,
   and v99 is rejected (`bundle.rs:227-233`, tested at
   `format_version_downgrade_rejected` in `bundle.rs:478`).

**Audit findings (concerns):**

6. The witness covers `(data_ref, dim, rotation_seed, rerank_factor,
   generation)` — but **not** `format_version`. A malicious actor
   producing a v1 bundle vs a v2 bundle with the same other fields
   gets the same witness (modulo the variant-tag fix). This is
   intentional (to allow format upgrades to leave witnesses stable),
   but a v1 reader and a v2 reader could disagree on what the
   bundle "means" while agreeing on the witness. **Risk:** low —
   v1 readers are deprecated, and the variant-tag fix in v2
   guarantees that any v2-computed witness over post-fix inputs
   differs from any v1-computed witness over the same inputs (per
   the doc comment at `bundle.rs:158-162`).

7. **Rotation kind (Haar vs Hadamard) is NOT in the witness.**
   ADR-158 §"Open Questions" §3 explicitly flags this as a strong
   recommendation pending `WitnessV2`. Today, two bundles built with
   different `RandomRotationKind` values but the same other fields
   produce **identical witnesses**, but their cached codes are
   **different**. A reader that loaded an `index.rbpx` built with
   Hadamard while expecting Haar would silently mis-rank.

   **Mitigation in code:** ruLake's `RuLake::new` only takes seed
   (no rotation kind), so all default-ruLake bundles are Haar.
   Operators explicitly building a Hadamard `RabitqPlusIndex` and
   feeding it through `install_prebuilt` are responsible per ADR-158
   for fixing the rotation kind at bootstrap.

   **Severity.** Low for default users; Medium for Hadamard-opting
   operators. Documented in ADR-158.

8. **Witness is hex (not raw bytes) on the wire.** 64 hex chars vs
   32 raw bytes is a 2× footprint cost, accepted for human
   readability. Fine.

**Verdict on witness:** the scheme is sound, the domain separation
and length prefixing are correctly applied, and the audit-driven
variant-tag fix is in. The rotation-kind gap is documented and
gated behind explicit opt-in.

---

## 4. Path-traversal defenses — `FsBackend::validate_filename`

`src/fs_backend.rs:105-136`:

```rust
fn validate_filename(f: &str) -> Result<()> {
    if f.is_empty() { ... }
    if f.len() > 255 { ... }
    if f == "." || f == ".." { ... }
    for b in f.bytes() {
        if b < 0x20 || b == 0x7f { ... }       // control / DEL
        if b == b'/' || b == b'\\' { ... }     // separators
    }
    if f.contains(':') { ... }                  // Windows drive / UNC
    Ok(())
}
```

Tested by `fs_register_rejects_path_traversal` (`fs_backend.rs:411`)
across 12 attack inputs:

```
"../escape", "../../etc/passwd", "./secret", "", "/absolute",
"sub/foo", "back\\slash", ".", "..", "foo\0bar", "foo\nbar", "C:name"
```

**Verdict:** comprehensive. Covers POSIX, Windows, control bytes,
empty, length cap (POSIX `NAME_MAX = 255`). Two minor observations:

- **No Unicode normalization.** A filename containing
  combining-diacritics will be accepted byte-by-byte (no `/`, no
  `..`, no control byte). This is correct (filesystems handle
  Unicode at the OS layer), but two equivalent-looking but
  byte-different filenames will register as distinct collections.
  Operator-driven, not a security issue.
- **No reserved-name check** (`CON`, `PRN`, `AUX`, `NUL` on
  Windows). On a Linux deployment this doesn't matter; on cross-
  platform deployment a Windows operator could create
  `register("c", "AUX")` and confuse the OS. Low priority.

**Suggestion (low):** add Windows reserved names to the deny list if
cross-platform support is in scope.

---

## 5. Input-validation map (every public entry point)

| API | Inputs | Validation | Verdict |
|---|---|---|---|
| `RuLake::new(rerank_factor, rotation_seed)` | `usize`, `u64` | None | OK — both are forwarded to RaBitQ as configuration. |
| `RuLake::with_consistency` | `Consistency` enum | Type-checked | OK |
| `RuLake::with_max_cache_entries(n)` | `usize` | `n.max(1)` (`cache.rs:317`) | OK — silently raises 0 to 1; doc-friendly. |
| `RuLake::register_backend` | `Arc<dyn BackendAdapter>` | Duplicate-id check (`lake.rs:94-98`) | OK |
| `RuLake::search_one(b, c, q, k)` | `&str`, `&str`, `&[f32]`, `usize` | Dim check inside `search_cached_with_rerank_interned` (`cache.rs:750`); `UnknownBackend` / `UnknownCollection` propagate | OK |
| `RuLake::search_federated(targets, q, k)` | `&[(&str, &str)]`, `&[f32]`, `usize` | Same as `search_one` per shard; `targets.len()` is unchecked but treated benignly | OK |
| `RuLake::search_batch(b, c, &queries, k)` | `&[Vec<f32>]`, ... | Per-query dim check (`cache.rs:822-828`) | OK |
| `RuLake::publish_bundle(key, dir)` | `&CacheKey`, `Path` | None on dir; `create_dir_all` runs (`bundle.rs:297`) | OK — host filesystem is trusted to enforce ACLs. |
| `RuLake::refresh_from_bundle_dir(key, dir)` | `&CacheKey`, `Path` | `from_json` caps + `verify_witness` (`bundle.rs:215-263, 340-356`) | OK |
| `RuLake::save_cache_to_dir(key, dir)` | `&CacheKey`, `Path` | Witness-existence check (`lake.rs:273-279`); index-existence check; cross-checks dim + rerank_factor against bundle | OK |
| `RuLake::warm_from_dir(key, dir)` | `&CacheKey`, `Path` | All of `read_from_dir` checks + cross-checks `idx.dim() == bundle.dim` and `idx.rerank_factor() == bundle.rerank_factor` (`lake.rs:407-420`) + `pos_to_id.len() == idx.len()` (`cache.rs:507-513`) | **Excellent.** Every cross-cutting invariant is checked. |
| `LocalBackend::put_collection` | `&str`, `usize`, `Vec<u64>`, `Vec<Vec<f32>>` | `ids.len() == vectors.len()` + per-vector dim check | OK; no growth cap (M5 above). |
| `LocalBackend::append` | `&str`, `u64`, `Vec<f32>` | Dim check; **silently sets dim if 0** | M5 above. |
| `FsBackend::new(id, root)` | `String`, `Path` | `create_dir_all`; no symlink check | M3 above. |
| `FsBackend::register(c, filename)` | `String`, `String` | `validate_filename` | OK. |
| `FsBackend::write(c, filename, dim, ids, vectors)` | as above + dim/ids/vectors | `validate_filename` + dim/ids checks | OK. |
| `RuLakeBundle::from_json(s)` | `&str` | 64 KiB body cap; 4 KiB per-field cap; 128-byte witness cap; format_version cap | **Excellent.** |
| `RuLakeBundle::read_from_dir(dir)` | `Path` | `from_json` checks + `verify_witness` | OK. |
| `validate_pulled_batch` | `&PulledBatch` | n cap, dim cap, len-match, byte-overflow checked | OK; runs at the right layer (before alloc) per `cache.rs:377`. |

**Coverage verdict:** input validation is **systematically applied at
the boundary** for every external-input path (JSON sidecars, on-disk
binary files, untrusted filenames, untrusted batch sizes). The only
gap is `Generation::Opaque` strings (M4) and the soft `LocalBackend`
caps (M5).

---

## 6. Concurrency & memory safety

- **Zero `unsafe`** in `src/`. Verified by `grep -rn "unsafe " src/`.
- **No `std::mem::transmute`, no `*mut`, no `Box::from_raw`.**
- **All shared state behind `Arc<{Mutex,RwLock}>`**; no `*const`
  raw pointers escaping `Send`/`Sync` boundaries.
- **`RabitqPlusIndex` is wrapped in `Arc`** so concurrent readers
  share an immutable view (`cache.rs:213-214`). The "Arc-drop-lock"
  pattern at `cache.rs:734-762` is correct and idiomatic.
- **`Send + Sync` is enforced on the trait** (`backend.rs:110`), so
  any `BackendAdapter` is safe to share across threads.

**Concurrency test coverage** is real:
`concurrent_searches_are_safe_and_correct` (`tests:559`) spawns 8
threads × 50 queries hitting both single-shard and federated paths
on a shared `RuLake`, asserts no panics, no nan/inf scores, sorted
results, expected hit counts. **Validates the design under
contention.**

---

## 7. Dependency surface & supply chain

`Cargo.toml:15-29` declares 9 direct deps. Notes:

| Dep | Pin | Concern |
|---|---|---|
| `ruvector-rabitq` | `path = "../...", version = "2.2"` | M6 above. Path-dep + workspace-only version means this repo doesn't build standalone. |
| `serde` | workspace | Not visible from this repo. |
| `serde_json` | `1` | Open-ended caret range. Bumping a major version (none expected for serde_json) would be silent. |
| `thiserror` | workspace | Not visible. |
| `sha3` | `0.10` | Pin matched to `rvf-crypto` per inline comment — good discipline. |
| `hex` | `0.4` | OK. |
| `rand` / `rand_distr` | workspace | Used only by the demo binary and tests, not by the library hot path. |
| `rayon` | workspace `1.10` | Pin documented inline as "workspace-pinned 1.10" — good. |

**Recommendations:**

- For a standalone published crate, replace every `workspace = true`
  with concrete versions and drop the `path = "../..."` for
  `ruvector-rabitq`.
- Add a `Cargo.lock` to the repo for reproducible builds.
- Set up `cargo audit` and `cargo deny` in CI.
- Add a `MSRV` (minimum supported Rust version) declaration if
  `rust-version.workspace = true` resolves to one.

---

## 8. Summary table

| ID | Severity | Title | Location | Remediation effort |
|---|---|---|---|---|
| M1 | Medium | Mutex `unwrap()` on poisoning bricks the Lake | cache.rs / lake.rs / backend.rs / fs_backend.rs | Low (helper fn or parking_lot swap) |
| M2 | Medium | Default `current_bundle` does a full pull → operator footgun under `Fresh` | backend.rs:125 | Medium (deprecate / runtime warn / require override) |
| M3 | Medium | `FsBackend` symlink + TOCTOU on `root` and tmp file | fs_backend.rs:62, 167 | Medium (`O_NOFOLLOW` + `create_new`) |
| M4 | Medium | `Generation::Opaque(String)` is unbounded | bundle.rs:60, lake.rs:648 | Low (add cap + check) |
| M5 | Medium | `LocalBackend::append` no growth cap, dim auto-set | backend.rs:225 | Low (mirror `MAX_PULLED_VECTORS`) |
| M6 | Medium | `Cargo.toml` workspace inheritance + path dep | Cargo.toml | Medium (re-pin for standalone) |
| L1 | Low | `try_into().unwrap()` on infallible array slices | fs_backend.rs:259, 260, 296, 345 | Trivial (use `.expect("invariant")`) |
| L2 | Low | mtime second-resolution + epoch-pre rounding | fs_backend.rs:215 | None (documented) |
| L3 | Low | serde_json default recursion | bundle.rs:225 | Low (explicit limit) |
| L4 | Low | `with_max_cache_entries` discards prior cache silently | lake.rs:78 | Low (doc + warn) |
| L5 | Info | Witness 256-bit truncation (collision space 2^128) | bundle.rs:386 | None |
| L6 | Low | Bundle file mode is platform default | bundle.rs / lake.rs | Low (accept mode parameter) |
| L7 | Info | `LocalBackend` generation `wrapping_add` | backend.rs:220, 247 | None |
| I1 | Info | `validate_pulled_batch` runs after pull cost | cache.rs:377 | Backend-side pre-check (per-adapter) |
| I2 | Info | `publish_bundle` does not authenticate publisher | lake.rs:167 | Out-of-scope for M1 (M4 governance) |
| I3 | Info | `fs::rename` overwrites silently across processes | fs_backend.rs:193 | Documented |
| I4 | Info | No SBOM / `cargo deny` config | repo root | CI add |
| I5 | Info | No fuzz harness for JSON / `ruvec1` parsers | repo root | `cargo fuzz` ~40 lines |

**Critical: 0. High: 0. Medium: 6. Low: 6. Info: 5.**

---

## 9. Verdict

ruLake's security posture is **substantially better than typical for
a v1 Rust crate of this scope**. The architectural choices that pay
off here:

- **Zero `unsafe`** keeps the entire crate inside Rust's memory
  safety guarantees.
- **Defense in depth on every external-input parser** (JSON caps,
  `ruvec1` header bounds, filename whitelist, `validate_pulled_batch`).
- **Witness scheme is correctly domain-separated and length-prefixed**,
  with both regression tests against the historic collision findings.
- **Atomic temp+rename for every on-disk write** prevents torn-file
  reads.
- **The Arc-drop-lock pattern** keeps the cache mutex out of the
  scan, which incidentally also keeps the mutex critical sections
  short enough that poisoning risk (M1) is theoretical rather than
  practical.

The medium-severity findings are **operator footguns and hardening
opportunities**, not directly exploitable vulnerabilities. The two
that most deserve action before any production deployment:

- **M3** (FsBackend symlink/TOCTOU) — affects multi-tenant hosts.
- **M4** (`Generation::Opaque` unbounded) — affects deployments that
  load third-party `BackendAdapter`s.

For an audit-grade improvement, address M1 (poison handling), M2
(force `current_bundle` override or runtime-warn), M3, M4, M5, then
add CI fuzz harnesses for the two parsers (I5). The crate's surface
is small enough that this is a realistic week of work, not a quarter.

For a crate that explicitly does **not yet ship** wire auth, RBAC,
PII enforcement, or GDPR shred (all M2-M4 roadmap), the security
boundary is set honestly: trust your backends, trust your filesystem
ACLs, treat the bundle as content-addressed-but-public. Within that
boundary, the implementation is solid.
