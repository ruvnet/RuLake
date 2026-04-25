//! Witness-pinned CUDA brute-force L2 kNN over a ruLake snapshot.
//!
//! ## What this binary does
//!
//! 1. Reads `<snapshot-dir>/table.rulake.json`, recomputes the SHAKE-256
//!    witness from the bundle fields, and aborts loudly on mismatch.
//!    The witness algorithm is byte-exactly the one in
//!    `src/bundle.rs` upstream — see `witness.rs` for the local copy.
//! 2. Reads the accompanying `ruvec1` data file (the format documented
//!    in `src/fs_backend.rs`). Caps the corpus at `--max-vectors` so
//!    a hostile or accidentally-huge file can't blow VRAM.
//! 3. Generates a deterministic query vector from a SHAKE-256 stream
//!    seeded by the bundle's `rotation_seed` so reruns are reproducible.
//! 4. Runs a CPU brute-force L2 baseline.
//! 5. Uploads vectors + query to the GPU, launches the L2 kernel,
//!    copies distances back, runs top-K on the host.
//! 6. Prints CPU time, GPU time (kernel-only and end-to-end including
//!    H2D/D2H), the speedup, and the top-K agreement.
//!
//! ## What this binary does NOT do
//!
//! It does NOT call into ruLake's compressed-scan path. ruLake's
//! native search runs RaBitQ 1-bit popcount over compressed codes
//! using the CPU AVX2/AVX-512 kernels in `ruvector-rabitq`, and there
//! is no GPU port of that kernel today (ADR-157 is "Proposed —
//! scaffolding-only"). What this example proves is that the bundle
//! protocol is decoupled enough that a GPU pipeline can verify
//! provenance, consume the same data file, and run an exact L2
//! comparison — no waiting on the M2 kernel plane.
//!
//! See `examples/gpu/03-rabitq-gpu-design-note/README.md` for the
//! sketch of what a GPU port of the actual rabitq scan would look like.

mod ruvec1;
mod witness;

use std::path::{Path, PathBuf};
use std::time::Instant;

use cudarc::driver::{CudaContext, CudaFunction, LaunchConfig, PushKernelArg};

/// PTX produced by `build.rs`. cudarc loads this at runtime via the
/// driver's JIT — no link against libcuda required at build time.
const PTX_SRC: &str = include_str!(concat!(env!("OUT_DIR"), "/l2_search.ptx"));

/// CUDA kernel name as exported in `kernels/l2_search.cu`.
const KERNEL_NAME: &str = "l2_brute_force";

/// Conservative demo cap — fits a 256-d corpus in ≤ ~1 GB VRAM and
/// still gives the GPU plenty to chew on. Override with `--max-vectors`.
const DEFAULT_MAX_VECTORS: usize = 1_000_000;

#[derive(Debug)]
struct Args {
    snapshot_dir: PathBuf,
    k: usize,
    max_vectors: usize,
    materialize_demo: bool,
    demo_dim: usize,
    demo_n: usize,
}

impl Args {
    fn parse() -> Result<Self, String> {
        // Hand-rolled argv loop so we don't pull clap into a CUDA
        // example. Pattern matches the ergonomics of the other ruLake
        // examples (sidecar_daemon, warm_restart) which also avoid
        // clap.
        let mut snapshot_dir: Option<PathBuf> = None;
        let mut k = 10usize;
        let mut max_vectors = DEFAULT_MAX_VECTORS;
        let mut materialize_demo = false;
        let mut demo_dim = 128usize;
        let mut demo_n = 100_000usize;

        let mut argv = std::env::args().skip(1);
        while let Some(a) = argv.next() {
            match a.as_str() {
                "--k" => {
                    k = argv
                        .next()
                        .ok_or("--k requires a value")?
                        .parse()
                        .map_err(|e| format!("--k: {e}"))?;
                }
                "--max-vectors" => {
                    max_vectors = argv
                        .next()
                        .ok_or("--max-vectors requires a value")?
                        .parse()
                        .map_err(|e| format!("--max-vectors: {e}"))?;
                }
                "--materialize-demo" => materialize_demo = true,
                "--demo-dim" => {
                    demo_dim = argv
                        .next()
                        .ok_or("--demo-dim requires a value")?
                        .parse()
                        .map_err(|e| format!("--demo-dim: {e}"))?;
                }
                "--demo-n" => {
                    demo_n = argv
                        .next()
                        .ok_or("--demo-n requires a value")?
                        .parse()
                        .map_err(|e| format!("--demo-n: {e}"))?;
                }
                "-h" | "--help" => {
                    print_usage();
                    std::process::exit(0);
                }
                other if !other.starts_with('-') && snapshot_dir.is_none() => {
                    snapshot_dir = Some(PathBuf::from(other));
                }
                other => return Err(format!("unknown argument: {other}")),
            }
        }
        let snapshot_dir =
            snapshot_dir.ok_or_else(|| "missing <snapshot-dir>".to_string())?;
        Ok(Self {
            snapshot_dir,
            k,
            max_vectors,
            materialize_demo,
            demo_dim,
            demo_n,
        })
    }
}

fn print_usage() {
    eprintln!(
        "Usage: cuda-brute-force <snapshot-dir> [options]\n\
         \n\
         Options:\n\
         \x20 --k <N>              top-K (default: 10)\n\
         \x20 --max-vectors <N>    cap the corpus (default: {DEFAULT_MAX_VECTORS})\n\
         \x20 --materialize-demo   build a synthetic snapshot at <snapshot-dir>\n\
         \x20                      if it doesn't exist\n\
         \x20 --demo-dim <N>       demo vector dimension (default: 128)\n\
         \x20 --demo-n <N>         demo corpus size (default: 100,000)\n\
         \n\
         Reads <snapshot-dir>/table.rulake.json (witness-verified) and\n\
         the accompanying ruvec1 data file, then runs a brute-force L2\n\
         search on the CPU and on the GPU and reports the comparison."
    );
}

fn main() {
    let args = match Args::parse() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}\n");
            print_usage();
            std::process::exit(2);
        }
    };

    if args.materialize_demo
        && !args
            .snapshot_dir
            .join(witness::SIDECAR_FILENAME)
            .exists()
    {
        if let Err(e) =
            materialize_demo_snapshot(&args.snapshot_dir, args.demo_dim, args.demo_n)
        {
            eprintln!("error: materialize-demo: {e}");
            std::process::exit(1);
        }
        println!(
            "[demo] materialized synthetic snapshot at {} ({} vectors × {} dims)",
            args.snapshot_dir.display(),
            args.demo_n,
            args.demo_dim
        );
    }

    if let Err(e) = run(&args) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run(args: &Args) -> Result<(), String> {
    // ---- Step 1: bundle + witness ----
    let bundle = witness::read_and_verify(&args.snapshot_dir)
        .map_err(|e| format!("witness verify: {e}"))?;
    println!(
        "[bundle] verified witness {} (data_ref={})",
        &bundle.rvf_witness, bundle.data_ref
    );

    // ---- Step 2: ruvec1 corpus ----
    let data_path = resolve_data_file(&args.snapshot_dir, &bundle)?;
    let corpus = ruvec1::read(&data_path, args.max_vectors)
        .map_err(|e| format!("ruvec1 read: {e}"))?;
    if corpus.dim as u64 != bundle.dim {
        return Err(format!(
            "snapshot incoherent: bundle.dim={} but ruvec1 dim={}",
            bundle.dim, corpus.dim
        ));
    }
    println!(
        "[corpus] loaded {} vectors × {} dims from {}",
        corpus.count,
        corpus.dim,
        data_path.display()
    );

    // ---- Step 3: query (deterministic per rotation_seed) ----
    let query = make_query(corpus.dim, bundle.rotation_seed);
    let k = args.k.min(corpus.count);
    if k == 0 {
        return Err("requested k=0 hits".to_string());
    }

    // ---- Step 4: CPU baseline ----
    let cpu_t0 = Instant::now();
    let cpu_hits = cpu_topk_l2(&corpus, &query, k);
    let cpu_elapsed = cpu_t0.elapsed();
    println!(
        "[cpu]   top-{k} L2 in {:>8.3} ms ({} candidates × {} dims)",
        cpu_elapsed.as_secs_f64() * 1000.0,
        corpus.count,
        corpus.dim
    );

    // ---- Step 5: GPU brute-force ----
    let gpu_result = match gpu_topk_l2(&corpus, &query, k) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[gpu]   skipped: {e}");
            return Ok(());
        }
    };
    println!(
        "[gpu]   top-{k} L2 in {:>8.3} ms (kernel) + {:>6.3} ms (H2D+D2H) = {:>8.3} ms total",
        gpu_result.kernel_ms,
        gpu_result.transfer_ms,
        gpu_result.kernel_ms + gpu_result.transfer_ms
    );

    // ---- Step 6: agreement + report ----
    let cpu_ids: Vec<u64> = cpu_hits.iter().map(|h| h.id).collect();
    let gpu_ids: Vec<u64> = gpu_result.hits.iter().map(|h| h.id).collect();
    let agree = top_k_agreement(&cpu_ids, &gpu_ids);

    let speedup_kernel = cpu_elapsed.as_secs_f64() * 1000.0 / gpu_result.kernel_ms.max(1e-6);
    let speedup_total = cpu_elapsed.as_secs_f64() * 1000.0
        / (gpu_result.kernel_ms + gpu_result.transfer_ms).max(1e-6);

    println!();
    println!("=== summary ===");
    println!("  provenance_id (witness):  {}", bundle.rvf_witness);
    println!("  n × dim:                  {} × {}", corpus.count, corpus.dim);
    println!("  k:                        {}", k);
    println!(
        "  speedup (kernel-only):    {:>6.2}x  (CPU {:>7.3} ms vs GPU {:>7.3} ms)",
        speedup_kernel,
        cpu_elapsed.as_secs_f64() * 1000.0,
        gpu_result.kernel_ms
    );
    println!(
        "  speedup (incl. transfer): {:>6.2}x  (CPU {:>7.3} ms vs GPU {:>7.3} ms)",
        speedup_total,
        cpu_elapsed.as_secs_f64() * 1000.0,
        gpu_result.kernel_ms + gpu_result.transfer_ms
    );
    println!(
        "  top-{k} set agreement:        {}/{}  ({:.1}%)",
        agree,
        k,
        (agree as f64 / k as f64) * 100.0
    );
    println!();
    println!("  top-{k} (CPU baseline):");
    for (rank, h) in cpu_hits.iter().enumerate().take(k) {
        let gpu_marker = if gpu_ids.contains(&h.id) { " " } else { "*" };
        println!(
            "    {rank:>3}.  id={:>10}  l2_sq={:>14.6}  {gpu_marker}",
            h.id, h.score
        );
    }
    if agree < k {
        println!(
            "    (rows marked * are not in the GPU top-{k}; see README\n\
            \x20    'On float order-of-operations' for why this is expected\n\
            \x20    on ties or near-ties.)"
        );
    }

    Ok(())
}

#[derive(Clone, Copy)]
struct Hit {
    id: u64,
    score: f32,
}

/// CPU top-K by ascending L2_sq. Single-pass linear scan + keep-min
/// heap was overkill at this corpus size — sort-then-truncate is
/// readable and fast enough to be a meaningful baseline for the
/// CPU↔GPU comparison this example is making.
fn cpu_topk_l2(corpus: &ruvec1::Corpus, query: &[f32], k: usize) -> Vec<Hit> {
    let dim = corpus.dim;
    let mut scored: Vec<Hit> = (0..corpus.count)
        .map(|i| {
            let v = &corpus.vectors[i * dim..(i + 1) * dim];
            let mut acc = 0.0f32;
            for j in 0..dim {
                let d = query[j] - v[j];
                acc += d * d;
            }
            Hit {
                id: corpus.ids[i],
                score: acc,
            }
        })
        .collect();
    scored.sort_by(|a, b| a.score.partial_cmp(&b.score).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(k);
    scored
}

struct GpuResult {
    hits: Vec<Hit>,
    kernel_ms: f64,
    transfer_ms: f64,
}

/// Run brute-force L2 on the GPU. Returns the top-K hits along with
/// kernel-only and transfer-only timings (so the README can be honest
/// about what's amortizable across many queries vs what's per-query
/// overhead).
fn gpu_topk_l2(
    corpus: &ruvec1::Corpus,
    query: &[f32],
    k: usize,
) -> Result<GpuResult, String> {
    let ctx = CudaContext::new(0).map_err(|e| format!("CudaContext::new(0): {e}"))?;
    println!(
        "[gpu]   device: {}",
        ctx.name().unwrap_or_else(|_| "<unknown>".to_string())
    );
    let stream = ctx.default_stream();

    // Load PTX. If `build.rs` couldn't find nvcc this surfaces here as
    // a clear "couldn't load module" error rather than a confusing
    // launch failure later.
    let ptx = cudarc::nvrtc::Ptx::from_src(PTX_SRC);
    let module = ctx.load_module(ptx).map_err(|e| {
        format!(
            "load PTX: {e}\n\
            \x20  Did `nvcc` succeed at build time? Re-run\n\
            \x20  `cargo clean && cargo build --release` after installing\n\
            \x20  the CUDA toolkit (>=12.0)."
        )
    })?;
    let func: CudaFunction = module
        .load_function(KERNEL_NAME)
        .map_err(|e| format!("load function {KERNEL_NAME}: {e}"))?;

    // Time H2D + D2H + alloc separately from the kernel — the H2D
    // dominates for a single query, but a real serving setup uploads
    // the corpus once and reuses it across thousands of queries, so
    // separating these matters.
    let xfer_t0 = Instant::now();
    let d_query = stream
        .clone_htod(query)
        .map_err(|e| format!("upload query: {e}"))?;
    let d_vectors = stream
        .clone_htod(corpus.vectors.as_slice())
        .map_err(|e| format!("upload vectors: {e}"))?;
    let mut d_dist = stream
        .alloc_zeros::<f32>(corpus.count)
        .map_err(|e| format!("alloc out_distances: {e}"))?;
    stream
        .synchronize()
        .map_err(|e| format!("sync after H2D: {e}"))?;
    let h2d_ms = xfer_t0.elapsed().as_secs_f64() * 1000.0;

    // ---- launch ----
    let cfg = LaunchConfig::for_num_elems(corpus.count as u32);
    let n = corpus.count as u32;
    let dim = corpus.dim as u32;

    let kernel_t0 = Instant::now();
    {
        let mut builder = stream.launch_builder(&func);
        builder
            .arg(&d_query)
            .arg(&d_vectors)
            .arg(&mut d_dist)
            .arg(&n)
            .arg(&dim);
        // Safety: argument types and order match `l2_brute_force`'s
        // C signature. CUDA bounds-check the per-thread index
        // ourselves inside the kernel via `if (idx >= n_vectors)`.
        unsafe {
            builder
                .launch(cfg)
                .map_err(|e| format!("kernel launch: {e}"))?;
        }
    }
    stream
        .synchronize()
        .map_err(|e| format!("sync after launch: {e}"))?;
    let kernel_ms = kernel_t0.elapsed().as_secs_f64() * 1000.0;

    let dtoh_t0 = Instant::now();
    let dist_host = stream
        .clone_dtoh(&d_dist)
        .map_err(|e| format!("D2H distances: {e}"))?;
    stream
        .synchronize()
        .map_err(|e| format!("sync after D2H: {e}"))?;
    let d2h_ms = dtoh_t0.elapsed().as_secs_f64() * 1000.0;

    // Host-side top-K. For demo corpus sizes (≤ 1M) this is much
    // cheaper than the kernel. A production GPU pipeline would do
    // partial top-K on-device with warp-shuffle reduction or
    // CUB::DeviceRadixSort; see the design note in module 03.
    let mut scored: Vec<Hit> = corpus
        .ids
        .iter()
        .zip(dist_host.iter())
        .map(|(&id, &score)| Hit { id, score })
        .collect();
    scored.sort_by(|a, b| a.score.partial_cmp(&b.score).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(k);

    Ok(GpuResult {
        hits: scored,
        kernel_ms,
        transfer_ms: h2d_ms + d2h_ms,
    })
}

fn top_k_agreement(a: &[u64], b: &[u64]) -> usize {
    let bset: std::collections::HashSet<u64> = b.iter().copied().collect();
    a.iter().filter(|id| bset.contains(id)).count()
}

/// Resolve the data file behind a verified bundle.
///
/// Resolution order:
/// 1. `data_ref` if it's a `file://` URI and the file exists on disk.
/// 2. `<snapshot-dir>/vectors.ruvec1` (this example's convention).
/// 3. The first `*.ruvec1` file in `<snapshot-dir>`.
fn resolve_data_file(
    snapshot_dir: &Path,
    bundle: &witness::RuLakeBundle,
) -> Result<PathBuf, String> {
    if let Some(rest) = bundle.data_ref.strip_prefix("file://") {
        let p = PathBuf::from(rest);
        if p.is_file() {
            return Ok(p);
        }
    }
    let conventional = snapshot_dir.join("vectors.ruvec1");
    if conventional.is_file() {
        return Ok(conventional);
    }
    if let Ok(rd) = std::fs::read_dir(snapshot_dir) {
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_file()
                && p.extension().and_then(|s| s.to_str()) == Some("ruvec1")
            {
                return Ok(p);
            }
        }
    }
    Err(format!(
        "could not locate the data file for bundle at {}; \
         data_ref={:?} (expected a co-located vectors.ruvec1)",
        snapshot_dir.display(),
        bundle.data_ref
    ))
}

/// SHAKE-256-driven query vector. Deterministic per `rotation_seed`,
/// so two runs of this binary against the same snapshot produce the
/// same query and the agreement check is meaningful.
fn make_query(dim: usize, rotation_seed: u64) -> Vec<f32> {
    use sha3::digest::{ExtendableOutput, Update, XofReader};
    let mut h = sha3::Shake256::default();
    h.update(b"rulake-gpu-cuda-brute-force-query|");
    h.update(&rotation_seed.to_le_bytes());
    h.update(&(dim as u64).to_le_bytes());
    let mut reader = h.finalize_xof();
    let mut bytes = vec![0u8; dim * 4];
    reader.read(&mut bytes);
    let mut q = Vec::with_capacity(dim);
    for k in 0..dim {
        let lo = k * 4;
        let u = u32::from_le_bytes(bytes[lo..lo + 4].try_into().unwrap());
        // Map to [-1, 1).
        q.push((u as f64 / (1u64 << 32) as f64 * 2.0 - 1.0) as f32);
    }
    q
}

/// Build a synthetic snapshot at `target_dir` with the same on-disk
/// shape a Rust publisher would emit. Mirrors the Python
/// `materialize_demo_snapshot` helper from `examples/python/04-rag-grounded`.
fn materialize_demo_snapshot(
    target_dir: &Path,
    dim: usize,
    n: usize,
) -> Result<(), String> {
    use sha3::digest::{ExtendableOutput, Update, XofReader};
    std::fs::create_dir_all(target_dir).map_err(|e| {
        format!("mkdir {}: {e}", target_dir.display())
    })?;
    let data_path = target_dir.join("vectors.ruvec1");

    let mut h = sha3::Shake256::default();
    h.update(b"rulake-gpu-cuda-demo-corpus|");
    h.update(&(dim as u64).to_le_bytes());
    h.update(&(n as u64).to_le_bytes());
    let mut reader = h.finalize_xof();
    let mut bytes = vec![0u8; n * dim * 4];
    reader.read(&mut bytes);

    let ids: Vec<u64> = (1000..1000 + n as u64).collect();
    let mut vectors: Vec<Vec<f32>> = Vec::with_capacity(n);
    for i in 0..n {
        let base = (i % 5) as f32; // 5 loose "clusters" so the top-K is interesting.
        let mut v = Vec::with_capacity(dim);
        for k in 0..dim {
            let lo = (i * dim + k) * 4;
            let u = u32::from_le_bytes(bytes[lo..lo + 4].try_into().unwrap());
            let jitter = (u as f64 / (1u64 << 32) as f64) * 0.3 - 0.15;
            v.push(base + jitter as f32);
        }
        vectors.push(v);
    }
    ruvec1::write(&data_path, dim, &ids, &vectors).map_err(|e| format!("ruvec1 write: {e}"))?;

    let data_ref = format!("file://{}", data_path.display());
    let rotation_seed = 42u64;
    let rerank_factor = 20u64;
    let generation = witness::Generation::Num(1);
    let bundle = witness::RuLakeBundle {
        format_version: 2,
        data_ref: data_ref.clone(),
        dim: dim as u64,
        rotation_seed,
        rerank_factor,
        generation,
        rvf_witness: String::new(),
        pii_policy: None,
        lineage_id: Some("ol://rulake-gpu-cuda-demo".to_string()),
        memory_class: None,
    };
    let mut bundle = bundle;
    bundle.rvf_witness = witness::compute_witness(&bundle);

    let sidecar_path = target_dir.join(witness::SIDECAR_FILENAME);
    let body = serde_json::to_string_pretty(&bundle)
        .map_err(|e| format!("serialize bundle: {e}"))?;
    std::fs::write(&sidecar_path, body)
        .map_err(|e| format!("write {}: {e}", sidecar_path.display()))?;
    Ok(())
}
