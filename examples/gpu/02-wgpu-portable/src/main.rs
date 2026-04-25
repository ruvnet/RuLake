//! Witness-pinned cross-platform GPU L2 kNN over a ruLake snapshot,
//! using `wgpu` so it runs unmodified on Vulkan / Metal / DX12 / WebGPU.
//!
//! See `../01-cuda-brute-force/` for the same example expressed against
//! the CUDA driver API (and ~3-5x more peak throughput on NVIDIA HW
//! because the kernel can use FMA + `--use_fast_math`).
//!
//! Like the CUDA module, this binary does NOT call into ruLake's
//! compressed-scan path. ruLake's native search runs RaBitQ 1-bit
//! popcount on the host CPU; there's no GPU port today (ADR-157
//! remains "Proposed — scaffolding-only"). What this proves is that
//! the bundle protocol gives a clean GPU integration story even
//! before the kernel plane lands. See
//! `../03-rabitq-gpu-design-note/README.md` for the GPU rabitq sketch.

mod ruvec1;
mod witness;

use std::path::{Path, PathBuf};
use std::time::Instant;

use bytemuck::{Pod, Zeroable};
use wgpu::util::DownloadBuffer;

const SHADER_SRC: &str = include_str!("shaders/l2_search.wgsl");
const KERNEL_ENTRY_POINT: &str = "l2_brute_force";
const WORKGROUP_SIZE: u32 = 256;
const DEFAULT_MAX_VECTORS: usize = 1_000_000;

/// Uniform block matching the WGSL `Params` struct. `repr(C)` so the
/// in-memory layout matches the WGSL std140-ish uniform layout.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    n_vectors: u32,
    dim: u32,
    // Pad to 16 bytes — uniform-buffer alignment on every backend
    // requires the struct size be a multiple of 16.
    _pad0: u32,
    _pad1: u32,
}

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
        let snapshot_dir = snapshot_dir.ok_or_else(|| "missing <snapshot-dir>".to_string())?;
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
        "Usage: wgpu-portable <snapshot-dir> [options]\n\
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
         search on the CPU and on the GPU (via wgpu — Vulkan / Metal /\n\
         DX12 / WebGPU) and reports the comparison."
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
        if let Err(e) = materialize_demo_snapshot(&args.snapshot_dir, args.demo_dim, args.demo_n) {
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
    // ---- bundle + witness ----
    let bundle = witness::read_and_verify(&args.snapshot_dir)
        .map_err(|e| format!("witness verify: {e}"))?;
    println!(
        "[bundle] verified witness {} (data_ref={})",
        &bundle.rvf_witness, bundle.data_ref
    );

    // ---- corpus ----
    let data_path = resolve_data_file(&args.snapshot_dir, &bundle)?;
    let corpus =
        ruvec1::read(&data_path, args.max_vectors).map_err(|e| format!("ruvec1 read: {e}"))?;
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

    // ---- query ----
    let query = make_query(corpus.dim, bundle.rotation_seed);
    let k = args.k.min(corpus.count);
    if k == 0 {
        return Err("requested k=0 hits".to_string());
    }

    // ---- CPU baseline ----
    let cpu_t0 = Instant::now();
    let cpu_hits = cpu_topk_l2(&corpus, &query, k);
    let cpu_elapsed = cpu_t0.elapsed();
    println!(
        "[cpu]   top-{k} L2 in {:>8.3} ms ({} candidates × {} dims)",
        cpu_elapsed.as_secs_f64() * 1000.0,
        corpus.count,
        corpus.dim
    );

    // ---- GPU brute-force ----
    let gpu_result = match pollster::block_on(gpu_topk_l2(&corpus, &query, k)) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[gpu]   skipped: {e}");
            return Ok(());
        }
    };
    println!(
        "[gpu]   top-{k} L2 in {:>8.3} ms (dispatch) + {:>6.3} ms (upload+download) = {:>8.3} ms total",
        gpu_result.kernel_ms,
        gpu_result.transfer_ms,
        gpu_result.kernel_ms + gpu_result.transfer_ms
    );

    // ---- agreement ----
    let cpu_ids: Vec<u64> = cpu_hits.iter().map(|h| h.id).collect();
    let gpu_ids: Vec<u64> = gpu_result.hits.iter().map(|h| h.id).collect();
    let agree = top_k_agreement(&cpu_ids, &gpu_ids);

    let speedup_kernel = cpu_elapsed.as_secs_f64() * 1000.0 / gpu_result.kernel_ms.max(1e-6);
    let speedup_total = cpu_elapsed.as_secs_f64() * 1000.0
        / (gpu_result.kernel_ms + gpu_result.transfer_ms).max(1e-6);

    println!();
    println!("=== summary ===");
    println!("  provenance_id (witness):  {}", bundle.rvf_witness);
    println!(
        "  n × dim:                  {} × {}",
        corpus.count, corpus.dim
    );
    println!("  k:                        {}", k);
    println!("  backend:                  {}", gpu_result.backend);
    println!(
        "  speedup (dispatch-only):  {:>6.2}x  (CPU {:>7.3} ms vs GPU {:>7.3} ms)",
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

    Ok(())
}

#[derive(Clone, Copy)]
struct Hit {
    id: u64,
    score: f32,
}

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
    scored.sort_by(|a, b| {
        a.score
            .partial_cmp(&b.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    scored.truncate(k);
    scored
}

struct GpuResult {
    hits: Vec<Hit>,
    kernel_ms: f64,
    transfer_ms: f64,
    backend: String,
}

async fn gpu_topk_l2(
    corpus: &ruvec1::Corpus,
    query: &[f32],
    k: usize,
) -> Result<GpuResult, String> {
    // ---- adapter + device ----
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        })
        .await
        .map_err(|e| format!("request_adapter: {e}"))?;
    let info = adapter.get_info();
    let backend_label = format!("{:?} on {}", info.backend, info.name);
    println!("[gpu]   adapter: {}", backend_label);

    // We need a `max_storage_buffer_binding_size` large enough to hold
    // the corpus vector buffer (n * dim * 4 bytes). The defaults on
    // some backends are only 128 MiB — bump to the adapter's reported
    // ceiling so a 1M × 256-d run (= 1 GiB) succeeds.
    let mut limits = wgpu::Limits::default();
    let adapter_limits = adapter.limits();
    limits.max_storage_buffer_binding_size = adapter_limits.max_storage_buffer_binding_size;
    limits.max_buffer_size = adapter_limits.max_buffer_size;
    limits.max_compute_workgroups_per_dimension =
        adapter_limits.max_compute_workgroups_per_dimension;

    let needed_bytes = (corpus.count as u64) * (corpus.dim as u64) * 4;
    if needed_bytes > limits.max_storage_buffer_binding_size as u64 {
        return Err(format!(
            "vectors buffer = {needed_bytes} bytes exceeds adapter \
             max_storage_buffer_binding_size = {} bytes; lower --max-vectors \
             or pick a smaller --demo-dim",
            limits.max_storage_buffer_binding_size
        ));
    }

    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("rulake-gpu-wgpu-portable"),
            required_features: wgpu::Features::empty(),
            required_limits: limits,
            memory_hints: wgpu::MemoryHints::Performance,
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            trace: wgpu::Trace::Off,
        })
        .await
        .map_err(|e| format!("request_device: {e}"))?;

    // ---- shader + pipeline ----
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("l2_search"),
        source: wgpu::ShaderSource::Wgsl(SHADER_SRC.into()),
    });
    let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("l2_search_bgl"),
        entries: &[
            // query
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            // vectors
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            // out_distances
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            // params (uniform)
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("l2_search_pl"),
        bind_group_layouts: &[Some(&bind_group_layout)],
        // No push-constant / `var<immediate>` use in this shader.
        immediate_size: 0,
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("l2_search_pipeline"),
        layout: Some(&pipeline_layout),
        module: &module,
        entry_point: Some(KERNEL_ENTRY_POINT),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    });

    // ---- buffers ----
    let xfer_t0 = Instant::now();
    let query_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("query"),
        size: (query.len() * 4) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&query_buf, 0, bytemuck::cast_slice(query));

    let vectors_bytes = corpus.vectors.len() as u64 * 4;
    let vectors_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("vectors"),
        size: vectors_bytes,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&vectors_buf, 0, bytemuck::cast_slice(&corpus.vectors));

    let dist_bytes = corpus.count as u64 * 4;
    let distances_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("distances"),
        size: dist_bytes,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });

    let params = Params {
        n_vectors: corpus.count as u32,
        dim: corpus.dim as u32,
        _pad0: 0,
        _pad1: 0,
    };
    let params_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("params"),
        size: std::mem::size_of::<Params>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&params_buf, 0, bytemuck::bytes_of(&params));

    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("l2_search_bg"),
        layout: &bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: query_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: vectors_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: distances_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: params_buf.as_entire_binding(),
            },
        ],
    });

    // Force the queued writes to flush before we measure the kernel —
    // a `submit([])` flushes the staging-belt without queueing any
    // command buffer. On most backends `write_buffer` is already
    // synchronous-on-submit, but this is the documented way to be sure.
    queue.submit(std::iter::empty());
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|e| format!("poll after upload: {e:?}"))?;
    let upload_ms = xfer_t0.elapsed().as_secs_f64() * 1000.0;

    // ---- dispatch ----
    let workgroups = (corpus.count as u32).div_ceil(WORKGROUP_SIZE);
    let kernel_t0 = Instant::now();
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("l2_search_enc"),
    });
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("l2_search_pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(workgroups, 1, 1);
    }
    queue.submit([encoder.finish()]);
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|e| format!("poll after dispatch: {e:?}"))?;
    let kernel_ms = kernel_t0.elapsed().as_secs_f64() * 1000.0;

    // ---- download distances via DownloadBuffer (mappable readback) ----
    let dl_t0 = Instant::now();
    let (sender, receiver) = std::sync::mpsc::channel();
    DownloadBuffer::read_buffer(
        &device,
        &queue,
        &distances_buf.slice(..),
        move |result| {
            // The `Sender` does not need to outlive the callback —
            // wgpu invokes it once, on a worker thread.
            let _ = sender.send(result);
        },
    );
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|e| format!("poll for download: {e:?}"))?;
    let dl = receiver
        .recv()
        .map_err(|e| format!("download channel: {e}"))?
        .map_err(|e| format!("download buffer: {e:?}"))?;
    let raw: &[u8] = &dl;
    let dist_host: Vec<f32> = bytemuck::cast_slice::<u8, f32>(raw).to_vec();
    let download_ms = dl_t0.elapsed().as_secs_f64() * 1000.0;

    // ---- host-side top-K ----
    let mut scored: Vec<Hit> = corpus
        .ids
        .iter()
        .zip(dist_host.iter())
        .map(|(&id, &score)| Hit { id, score })
        .collect();
    scored.sort_by(|a, b| {
        a.score
            .partial_cmp(&b.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    scored.truncate(k);

    Ok(GpuResult {
        hits: scored,
        kernel_ms,
        transfer_ms: upload_ms + download_ms,
        backend: backend_label,
    })
}

fn top_k_agreement(a: &[u64], b: &[u64]) -> usize {
    let bset: std::collections::HashSet<u64> = b.iter().copied().collect();
    a.iter().filter(|id| bset.contains(id)).count()
}

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
            if p.is_file() && p.extension().and_then(|s| s.to_str()) == Some("ruvec1") {
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

fn make_query(dim: usize, rotation_seed: u64) -> Vec<f32> {
    use sha3::digest::{ExtendableOutput, Update, XofReader};
    let mut h = sha3::Shake256::default();
    h.update(b"rulake-gpu-wgpu-portable-query|");
    h.update(&rotation_seed.to_le_bytes());
    h.update(&(dim as u64).to_le_bytes());
    let mut reader = h.finalize_xof();
    let mut bytes = vec![0u8; dim * 4];
    reader.read(&mut bytes);
    let mut q = Vec::with_capacity(dim);
    for k in 0..dim {
        let lo = k * 4;
        let u = u32::from_le_bytes(bytes[lo..lo + 4].try_into().unwrap());
        q.push((u as f64 / (1u64 << 32) as f64 * 2.0 - 1.0) as f32);
    }
    q
}

fn materialize_demo_snapshot(target_dir: &Path, dim: usize, n: usize) -> Result<(), String> {
    use sha3::digest::{ExtendableOutput, Update, XofReader};
    std::fs::create_dir_all(target_dir).map_err(|e| format!("mkdir {}: {e}", target_dir.display()))?;
    let data_path = target_dir.join("vectors.ruvec1");

    let mut h = sha3::Shake256::default();
    h.update(b"rulake-gpu-wgpu-demo-corpus|");
    h.update(&(dim as u64).to_le_bytes());
    h.update(&(n as u64).to_le_bytes());
    let mut reader = h.finalize_xof();
    let mut bytes = vec![0u8; n * dim * 4];
    reader.read(&mut bytes);

    let ids: Vec<u64> = (1000..1000 + n as u64).collect();
    let mut vectors: Vec<Vec<f32>> = Vec::with_capacity(n);
    for i in 0..n {
        let base = (i % 5) as f32;
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
    let mut bundle = witness::RuLakeBundle {
        format_version: 2,
        data_ref: data_ref.clone(),
        dim: dim as u64,
        rotation_seed: 42,
        rerank_factor: 20,
        generation: witness::Generation::Num(1),
        rvf_witness: String::new(),
        pii_policy: None,
        lineage_id: Some("ol://rulake-gpu-wgpu-demo".to_string()),
        memory_class: None,
    };
    bundle.rvf_witness = witness::compute_witness(&bundle);

    let sidecar = target_dir.join(witness::SIDECAR_FILENAME);
    let body = serde_json::to_string_pretty(&bundle)
        .map_err(|e| format!("serialize bundle: {e}"))?;
    std::fs::write(&sidecar, body).map_err(|e| format!("write {}: {e}", sidecar.display()))?;
    Ok(())
}
