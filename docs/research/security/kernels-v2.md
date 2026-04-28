# Focused security review — `kernel-avx512` and `kernel-wgpu` (ADR-157)

**Date:** 2026-04-27
**Branch:** `main` at `c5031c4` (post-PR #14 merge + bench-sweep commit)
**Reviewer:** /loop directive pass — security review for ADR-157 kernels
**Commit baseline:** the two kernel crates as of v0.1 / v0.0.x

## Scope

This review covers the two ADR-157 kernel crates — `crates/kernel-avx512/`
and `crates/kernel-wgpu/` — that landed after the previous focused
security pass (`v0.0-substrates.md` covered `rvdna-backend` + `ruqu-backend`;
`shipping-substrates-v2.md` covered `gcs-backend`, `ipfs-backend`,
`mcp-server`). Neither kernel has been audited until now.

In scope:
- `crates/kernel-avx512/src/lib.rs` (406 lines, 9 `unsafe` blocks gated by
  runtime CPUID + `#[target_feature]`)
- `crates/kernel-wgpu/src/lib.rs` (649 lines, `#![forbid(unsafe_code)]`)
- `crates/kernel-wgpu/shaders/{l2,popcount}.wgsl` (74 lines combined)
- `WgpuKernel`'s host↔device buffer lifecycle and dispatch driver

Out of scope:
- The `VectorKernel` trait + dispatch policy in `crates/core/src/kernel.rs`
  (covered by ADR-157's own design review)
- Witness chain (kernels do not touch witnesses; data plane only)
- The WGSL compiler stack inside wgpu (we trust wgpu's own audit)
- Side-channel timing attacks against the kernels (not in threat model;
  popcount and L2 are open vector ops, not crypto)

## Threat model

The realistic attacker is **a caller that controls the candidate batch
shape** — i.e. an MCP client whose `rulake_query` happens to land at a
worker thread that's wired to dispatch through one of these kernels.
The kernel itself never sees the trust boundary; the MCP server is the
boundary. So the relevant questions are:

1. Can a malicious batch shape cause **memory unsafety** (OOB read/write,
   UAF) inside the kernel?
2. Can a malicious batch shape cause **resource exhaustion** (OOM, hang,
   GPU device reset) that turns into DoS for the host?
3. Does the kernel **leak data across queries** through residual GPU
   memory or accumulator state?
4. Does the kernel **silently produce wrong answers** that bypass the
   ADR-157 conformance gate?

## Findings — `kernel-avx512`

The crate is small, every `unsafe` block has a SAFETY comment, and the
construction is fail-closed (`Avx512Kernel::new() -> Option<Self>` returns
`None` unless `is_x86_feature_detected!` certifies *all four* of `avx512f`,
`avx512bw`, `avx512vl`, `avx512vpopcntdq` at runtime). The `unsafe`
intrinsics are then locked behind `#[target_feature]` helpers that
the compiler verifies the feature gate against, with `#[deny(unsafe_op_in_unsafe_fn)]`
forcing every intrinsic call to live inside an explicit `unsafe { }`
block (not implicitly inside the `unsafe fn`).

Walked the chunked SIMD loops with adversarial inputs in mind:
- `chunks = len / 16; tail_start = chunks * 16` — `tail_start ≤ len`,
  so the SIMD loop reads `[off..off+16)` for `off ∈ [0, tail_start)`, all
  in bounds.
- Empty input: `len = 0` → `chunks = 0`, SIMD loop never entered, scalar
  tail loop never entered, `sum = 0.0`. No UB.
- Mismatched input (caller passes `query.len() != candidate.len()`):
  cannot happen — the `Avx512Kernel::l2_distance_one` calling site does
  `let len = query.len().min(c.len()); l2_squared_avx512(&query[..len], &c[..len])`
  before the unsafe call, so the helper always sees equal-length subslices.
- Conformance: `tests/conformance_seed_42_matches_reference` runs the
  ADR-157 fixture under both kernels and asserts byte-equality.

### R-AVX-1 (LOW): popcount accumulator can theoretically truncate

In `popcount_xor_avx512`:

```rust
let chunk_sum = _mm512_reduce_add_epi64(acc) as u64;
let mut sum = chunk_sum as u32;     // ← truncation point
for j in tail_start..len {
    sum += (query[j] ^ candidate[j]).count_ones();
}
```

`acc` accumulates per-lane `popcnt_epi64` results across `chunks` SIMD
chunks of 8 `u64` lanes each, so the maximum reachable value is
`chunks * 8 * 64 = chunks * 512 ≤ (len/8) * 512 = len * 64`. For
`sum` to overflow `u32::MAX = 4.29 GB`, the input would need
`len > 4.29G / 64 ≈ 67M u64`s — i.e. RaBitQ codes for a single
candidate of `~67M * 64 = 4.3 G bits` of compressed dimension. RaBitQ
codes are typically `D bits / 64` words (D=768 → 12 words; D=1536 →
24 words), so this is structurally unreachable in any realistic config.

**Recommendation (R-AVX-1):** add a `debug_assert!(len < (1usize << 26))`
at the helper entry to lock the bound in test builds, and document in
the module header that this kernel rejects inputs above 64 M dim at
runtime. Apply when adding the next bench grid (D=768 / n=1M is well
under the limit, but the doc protects against future operators wiring
in arbitrarily large compressed dims). Not a release-blocker; the
overflow is unreachable on any sane workload.

### R-AVX-2 (INFO): `_mm512_mul_ps_local` shim widens unsafe surface by one

The single-line `unsafe fn _mm512_mul_ps_local` exists only to "read
consistently with the rest of the SIMD chunk." It compiles to one
instruction (`vmulps`). Inlining it at the single call site would drop
one `unsafe fn` from the audit surface without changing semantics.

**Recommendation (R-AVX-2):** inline the helper or annotate it with a
`#[doc(hidden)]` + a comment explaining why it must stay separate
(currently the comment doesn't justify the wrapper).

### Memory unsafety: none found

No OOB, no UAF, no aliasing violations. The `unsafe` surface is small
(~50 LOC across 2 helpers) and every `unsafe { }` has a precondition
the caller demonstrably maintains. Conformance fixture pins bit-equality
with the reference kernel, so any silent miscompile is caught at test
time.

### Cross-query state: none

`Avx512Kernel` is `#[derive(Debug, Default)]` with zero fields — purely
a marker type. No accumulator state survives a call. Cannot leak between
queries by construction.

## Findings — `kernel-wgpu`

The crate uses `#![forbid(unsafe_code)]` — the only memory-safety
boundary is whatever wgpu / the underlying Vulkan / Metal / DX12 driver
exposes. Everything else is safe Rust. Construction is fail-closed
(`new_async() -> Result<Self, WgpuKernelError>`, returns `Err` on
no-adapter or device-request failure). Shaders are statically embedded
via `include_str!("../shaders/...")` — operator cannot supply runtime
WGSL.

Both shaders are 35-40 LOC, deliberately tight: per-thread-per-candidate
distance, host-side sort, no shared memory, no atomics. The `if (i >= params.n) { return; }`
guard at line 26-28 of each shader prevents OOB GPU writes when the
host dispatches `ceil(n/64)` workgroups.

### R-WGPU-1 (MEDIUM): no per-call resource cap on host-side allocation

In `run_l2_dispatch` and `run_pop_dispatch`, both helpers compute
buffer sizes from caller-supplied `n` and `dim`:

```rust
let mut flat = vec![0.0f32; dim * n];     // host allocation, n*dim*4 bytes
let out_size = (n as u64) * std::mem::size_of::<f32>() as u64;
let out_buf = self.device.create_buffer(&wgpu::BufferDescriptor { size: out_size, ... });
```

No bound check on `n`, `dim`, or `n * dim`. A malicious caller (or a
buggy upstream wiring that forwards a large batch) can request:
- A 6 GB host vector at `D=1536, n=1M`
- A 4 GB GPU buffer at `n=1G`

`vec![0.0f32; n*dim]` will either succeed (and crowd out other tenants),
panic with allocation failure (and bring down whichever thread the
caller is on), or trigger OOM-killer at the OS level. The GPU side
will fail with a driver error if the requested buffer exceeds
`Limits::max_buffer_size`, but the host vec is allocated *first* so the
driver guard never triggers.

**Recommendation (R-WGPU-1):** add a `KernelCapabilities::max_dispatch_bytes`
field (or a kernel-side `max_n` / `max_dim` pair) and have the
dispatcher in `RuLake::pick_kernel` filter by it. As an interim fix in
this crate alone: check `n.saturating_mul(dim).saturating_mul(4) > 1 << 30`
(1 GiB) at the entry of `run_l2_dispatch` and `run_pop_dispatch`, return
an empty result with a `tracing::warn!` log. The dispatch policy
already has the `min_batch` filter; this is the missing `max_batch_bytes`
counterpart.

### R-WGPU-2 (LOW): no timeout on `device.poll(Maintain::Wait)`

```rust
slice.map_async(wgpu::MapMode::Read, move |r| { let _ = sender.send(r); });
self.device.poll(wgpu::Maintain::Wait);   // ← blocks until callback fires
receiver.recv().expect("wgpu map callback dropped").expect("wgpu map_async failed");
```

A driver hang — most realistic cause: a shader that hits an infinite
loop on a particular driver+GPU combo, or a TDR (Timeout Detection
and Recovery) event that never returns — would block the calling
thread forever. Acceptable in a single-threaded CLI; problematic when
`mcp-server`'s bounded rayon pool calls into this kernel and one stuck
thread permanently consumes a worker slot.

**Recommendation (R-WGPU-2):** wrap the whole dispatch in a host-side
deadline using `std::sync::mpsc::Receiver::recv_timeout` instead of
`recv()`. A 30 s ceiling matches the `Limits::max_compute_workgroups_per_dimension`
upper-bound shader runtime under reasonable workload assumptions and
gives the dispatcher a way to surface a `WgpuKernelError::DeviceTimeout`
that the cache can react to (e.g. unregister the kernel for the
remainder of the session). The caller-side panic from the existing
`expect()` should also be removed in favour of returning the error.

### R-WGPU-3 (LOW): two `expect()` panic points on driver edge cases

```rust
receiver.recv().expect("wgpu map callback dropped").expect("wgpu map_async failed");
```

Two stacked `expect()`s, both of which fire on legitimate driver edge
cases (callback dropped on device loss; map error on driver crash).
In a server context, a single GPU-driver crash takes down the worker
thread that called the kernel, which then causes the bounded rayon
pool to lose capacity until the process restarts.

**Recommendation (R-WGPU-3):** replace both with `?` propagation up to
the public `l2_distance_one` / `rabitq_popcount` returning `Result`,
or — if the trait shape can't change — have the kernel mark itself
`unhealthy` after the first panic and return an empty result for
subsequent calls until the operator re-registers it. The dispatcher
should treat `unhealthy` as a temporary `min_batch = ∞` filter.

### R-WGPU-4 (LOW): silent endianness assumption

```rust
// `bytemuck::cast_slice::<u64,u32>` is the same byte order regardless
// of host endianness on `cfg(target_endian = "little")` platforms
// (every target wgpu actually runs on).
let q_u32: &[u32] = bytemuck::cast_slice(query);
```

The comment is correct today — every wgpu-supported native platform
(Windows / macOS / Linux on x86_64 + ARM64; Android; iOS; WebGPU in
the browser) is little-endian. But this is a structural assumption
the compiler does not enforce.

**Recommendation (R-WGPU-4):** add at the crate root:

```rust
#[cfg(not(target_endian = "little"))]
compile_error!("kernel-wgpu's WGSL shaders assume little-endian u64 -> 2*u32 \
                packing; this target is big-endian");
```

This converts a "silently wrong on big-endian" bug into a build-time
refusal. Free safety, no runtime cost.

### R-WGPU-5 (INFO): per-call buffer allocation, not pooled

Each `run_*_dispatch` call allocates four wgpu buffers (`q_buf`, `c_buf`,
`out_buf`, `read_buf`) and lets them drop at function exit. Steady-state
serving therefore allocates and frees device memory on every query —
a measurable cost that the bench numbers (`docs/research/benchmarks/kernel-l2-three-way.md`)
attribute to "host↔device transfer dominates" but is partly per-call
allocation overhead.

This is **not a security finding** — it's a perf note that interacts with
the dispatch story. Cited here so a future v1.x device-side buffer
cache (mentioned in the accelerator-plane gist's "v1.x roadmap")
isolates the bench-cost hypothesis from the per-call-alloc cost.

### Cross-query state: none in shaders, refcounted in host

The shaders are stateless — every workgroup reads `query`, `candidates`,
`params` and writes `out`. No persistent GPU memory survives the dispatch.
On the host side, `WgpuKernel` holds `Arc<Device>` + `Arc<Queue>` +
two `Arc<ComputePipeline>` + two `Arc<BindGroupLayout>` — all
read-only after construction, no mutable state. Per-call buffers are
created fresh.

**Verdict:** the kernel does not leak data across queries.

### Untrusted-shader path: closed by construction

Both shaders are baked into the binary at compile time via `include_str!`.
A future feature that surfaces user-supplied WGSL would break this
guarantee and the threat model would have to be rewritten. Documenting
this here so future PRs that add a "load-shader-at-runtime" feature
are forced to revisit the audit.

## Summary

| ID | Severity | Crate | Title |
|---|---|---|---|
| R-AVX-1 | LOW | kernel-avx512 | Popcount accumulator can theoretically truncate at len > 64M dim |
| R-AVX-2 | INFO | kernel-avx512 | `_mm512_mul_ps_local` shim widens unsafe surface by one helper |
| R-WGPU-1 | **MEDIUM** | kernel-wgpu | No per-call resource cap on host-side or GPU-side buffer allocation |
| R-WGPU-2 | LOW | kernel-wgpu | No timeout on `device.poll(Maintain::Wait)` — driver hang blocks worker |
| R-WGPU-3 | LOW | kernel-wgpu | Two stacked `expect()` panic points on driver edge cases |
| R-WGPU-4 | LOW | kernel-wgpu | Silent little-endian assumption — should be a `compile_error!` |
| R-WGPU-5 | INFO | kernel-wgpu | Per-call buffer allocation (perf note, not a security finding) |

**Memory safety:** no findings in either kernel.
**Witness bypass:** no findings (kernels do not touch witnesses).
**Cross-query leakage:** no findings.

The MEDIUM finding (R-WGPU-1) is the one that warrants immediate action
when `WgpuKernel` is wired into a production `mcp-server` worker pool —
a cap on `n * dim * 4` bytes converts an OOM/DoS into a structured
refusal. The other findings are defensive-depth recommendations; none
of them are exploitable on the current shipping configuration (both
kernels are off-by-default; operators register them explicitly per
ADR-157).
