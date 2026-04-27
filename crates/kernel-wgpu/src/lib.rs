//! Portable GPU implementation of the ADR-157 [`VectorKernel`] trait
//! via [wgpu](https://wgpu.rs).
//!
//! This crate provides [`WgpuKernel`], which runs the L2² distance and
//! rabitq Hamming popcount inner loops on whichever compute-capable
//! backend wgpu finds: Vulkan, Metal, DX12, GL, or WebGPU. The same
//! source compiles for native and (eventually) WASM.
//!
//! # Construction is fail-closed
//!
//! Both constructors return `Result<Self, WgpuKernelError>`:
//!
//! - [`WgpuKernel::new_async`] — async, for callers that already drive
//!   their own runtime.
//! - [`WgpuKernel::new_blocking`] — synchronous, wraps `new_async` in
//!   `pollster::block_on`. Convenience for the typical sync caller.
//!
//! On a host without a compatible adapter the constructor returns
//! [`WgpuKernelError::NoAdapter`], so a binary linked against this
//! crate still loads on headless CI; the operator falls back to
//! whatever non-GPU kernel they have registered (typically
//! `CpuNaiveKernel`).
//!
//! # Determinism caveats
//!
//! Per ADR-157 §"Determinism as a hard gate":
//!
//! 1. **Scan / popcount** must be byte-equal across kernels. The
//!    rabitq path is exact integer math — XOR + popcount on packed
//!    1-bit codes — so we get bit-equality "for free", and the
//!    conformance fixture in `rulake::kernel::assert_kernel_conformant`
//!    exercises that path against [`WgpuKernel`].
//! 2. **Rerank / L2** may diverge in the last ULP because WGSL's `f32`
//!    arithmetic is IEEE-754 but per-driver compilers may reorder
//!    floating ops. We therefore advertise `simd_width: 1,
//!    popcount_native: false, gpu: true` and document that the L2
//!    path is *recall-equivalent*, not bit-equal. Operators routing
//!    `Consistency::Fresh` / `Consistency::Frozen` queries through
//!    this kernel must also wire a `caps().deterministic = false`
//!    filter once that field lands on `KernelCapabilities` in v3.0.
//!
//! # Example
//!
//! ```no_run
//! use ruvector_rulake_kernel_wgpu::WgpuKernel;
//! use rulake::kernel::VectorKernel;
//!
//! // Returns `Err` on hosts without a GPU adapter.
//! if let Ok(k) = WgpuKernel::new_blocking() {
//!     assert_eq!(k.id(), "wgpu");
//!     let q  = vec![0u64; 4];
//!     let cs = vec![vec![1u64; 4], vec![2u64; 4]];
//!     let _top = k.rabitq_popcount(&q, &cs, 1);
//! }
//! ```
#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::cmp::Ordering;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use rulake::kernel::{KernelCapabilities, VectorKernel};
use wgpu::util::DeviceExt;

/// Errors that can occur when constructing a [`WgpuKernel`].
#[derive(Debug)]
pub enum WgpuKernelError {
    /// `wgpu` could not find a compatible adapter — typically the
    /// host has no GPU or no suitable backend driver. Callers should
    /// fall back to a non-GPU kernel.
    NoAdapter,
    /// `wgpu` found an adapter but failed to request a device. The
    /// underlying error is preserved so the operator can see the
    /// driver / feature mismatch.
    RequestDevice(wgpu::RequestDeviceError),
}

impl std::fmt::Display for WgpuKernelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoAdapter => write!(f, "no compatible wgpu adapter found on this host"),
            Self::RequestDevice(e) => write!(f, "wgpu request_device failed: {e}"),
        }
    }
}

impl std::error::Error for WgpuKernelError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::RequestDevice(e) => Some(e),
            _ => None,
        }
    }
}

/// Portable GPU [`VectorKernel`] implementation backed by `wgpu`.
///
/// Holds a refcounted `wgpu::Device` + `Queue` plus the two cached
/// compute pipelines (L2 + popcount) so per-call dispatch only pays
/// the buffer-allocation + bind-group + submit cost.
///
/// Cheap to clone: every field is an `Arc`.
///
/// # Example
///
/// ```no_run
/// use ruvector_rulake_kernel_wgpu::WgpuKernel;
/// use rulake::kernel::VectorKernel;
///
/// let k = WgpuKernel::new_blocking().expect("no GPU adapter");
/// assert_eq!(k.id(), "wgpu");
/// ```
#[derive(Clone)]
pub struct WgpuKernel {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    l2_pipeline: Arc<wgpu::ComputePipeline>,
    pop_pipeline: Arc<wgpu::ComputePipeline>,
    l2_layout: Arc<wgpu::BindGroupLayout>,
    pop_layout: Arc<wgpu::BindGroupLayout>,
}

impl std::fmt::Debug for WgpuKernel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WgpuKernel").finish_non_exhaustive()
    }
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct ParamsU32x2 {
    a: u32,
    b: u32,
}

impl WgpuKernel {
    /// Async constructor. Requests an adapter + device and compiles
    /// the L2 / popcount compute pipelines.
    ///
    /// # Errors
    ///
    /// - [`WgpuKernelError::NoAdapter`] if no compatible adapter exists
    ///   (headless CI is the typical case).
    /// - [`WgpuKernelError::RequestDevice`] if the adapter is found
    ///   but the device request fails (e.g. driver doesn't support
    ///   the requested feature set).
    ///
    /// # Example
    ///
    /// ```no_run
    /// # async fn demo() {
    /// use ruvector_rulake_kernel_wgpu::WgpuKernel;
    /// let _ = WgpuKernel::new_async().await;
    /// # }
    /// ```
    pub async fn new_async() -> Result<Self, WgpuKernelError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::default());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
            .ok_or(WgpuKernelError::NoAdapter)?;

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("ruvector-rulake-kernel-wgpu/device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::downlevel_defaults(),
                    memory_hints: wgpu::MemoryHints::Performance,
                },
                None,
            )
            .await
            .map_err(WgpuKernelError::RequestDevice)?;

        let l2_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("kernel-wgpu/l2.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/l2.wgsl").into()),
        });
        let pop_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("kernel-wgpu/popcount.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/popcount.wgsl").into()),
        });

        let bind_layout = make_bind_group_layout(&device);

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("kernel-wgpu/pipeline-layout"),
            bind_group_layouts: &[&bind_layout],
            push_constant_ranges: &[],
        });

        let l2_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("kernel-wgpu/l2-pipeline"),
            layout: Some(&pipeline_layout),
            module: &l2_module,
            entry_point: "main",
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        let pop_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("kernel-wgpu/popcount-pipeline"),
            layout: Some(&pipeline_layout),
            module: &pop_module,
            entry_point: "main",
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        let bind_layout = Arc::new(bind_layout);
        Ok(Self {
            device: Arc::new(device),
            queue: Arc::new(queue),
            l2_pipeline: Arc::new(l2_pipeline),
            pop_pipeline: Arc::new(pop_pipeline),
            l2_layout: bind_layout.clone(),
            pop_layout: bind_layout,
        })
    }

    /// Blocking convenience over [`Self::new_async`] — wraps the
    /// async constructor in `pollster::block_on`.
    ///
    /// # Errors
    ///
    /// Same as [`Self::new_async`].
    ///
    /// # Example
    ///
    /// ```no_run
    /// use ruvector_rulake_kernel_wgpu::WgpuKernel;
    /// let _ = WgpuKernel::new_blocking();
    /// ```
    pub fn new_blocking() -> Result<Self, WgpuKernelError> {
        pollster::block_on(Self::new_async())
    }

    /// Run the L2 compute shader against `query` and `candidates_flat`
    /// (row-major `n × dim` `f32`), returning the per-candidate
    /// distance buffer downloaded back to host memory.
    ///
    /// `dim` and `n` are passed as a uniform; both must fit in `u32`.
    fn run_l2_dispatch(&self, query: &[f32], candidates_flat: &[f32], dim: u32, n: u32) -> Vec<f32> {
        let q_buf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("kernel-wgpu/l2/query"),
                contents: bytemuck::cast_slice(query),
                usage: wgpu::BufferUsages::STORAGE,
            });
        let c_buf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("kernel-wgpu/l2/candidates"),
                contents: bytemuck::cast_slice(candidates_flat),
                usage: wgpu::BufferUsages::STORAGE,
            });

        let out_size = (n as u64) * std::mem::size_of::<f32>() as u64;
        let out_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("kernel-wgpu/l2/out"),
            size: out_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let read_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("kernel-wgpu/l2/read"),
            size: out_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let params = ParamsU32x2 { a: dim, b: n };
        let params_buf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("kernel-wgpu/l2/params"),
                contents: bytemuck::bytes_of(&params),
                usage: wgpu::BufferUsages::UNIFORM,
            });

        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("kernel-wgpu/l2/bind"),
            layout: &self.l2_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: q_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: c_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: out_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: params_buf.as_entire_binding(),
                },
            ],
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("kernel-wgpu/l2/encoder"),
            });
        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("kernel-wgpu/l2/pass"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(&self.l2_pipeline);
            cpass.set_bind_group(0, &bind_group, &[]);
            let groups = n.div_ceil(64);
            cpass.dispatch_workgroups(groups, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&out_buf, 0, &read_buf, 0, out_size);
        self.queue.submit(Some(encoder.finish()));

        let slice = read_buf.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = sender.send(r);
        });
        // Drive the device until the map callback fires.
        self.device.poll(wgpu::Maintain::Wait);
        receiver
            .recv()
            .expect("wgpu map callback dropped")
            .expect("wgpu map_async failed");

        let data = slice.get_mapped_range();
        let out: Vec<f32> = bytemuck::cast_slice(&data).to_vec();
        drop(data);
        read_buf.unmap();
        out
    }

    /// Run the popcount compute shader against `query` and
    /// `candidates_flat` (row-major `n × dim_u32` `u32`), returning the
    /// per-candidate Hamming distance buffer downloaded back to host
    /// memory.
    fn run_pop_dispatch(
        &self,
        query: &[u32],
        candidates_flat: &[u32],
        dim_u32: u32,
        n: u32,
    ) -> Vec<u32> {
        let q_buf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("kernel-wgpu/pop/query"),
                contents: bytemuck::cast_slice(query),
                usage: wgpu::BufferUsages::STORAGE,
            });
        let c_buf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("kernel-wgpu/pop/candidates"),
                contents: bytemuck::cast_slice(candidates_flat),
                usage: wgpu::BufferUsages::STORAGE,
            });

        let out_size = (n as u64) * std::mem::size_of::<u32>() as u64;
        let out_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("kernel-wgpu/pop/out"),
            size: out_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let read_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("kernel-wgpu/pop/read"),
            size: out_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let params = ParamsU32x2 { a: dim_u32, b: n };
        let params_buf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("kernel-wgpu/pop/params"),
                contents: bytemuck::bytes_of(&params),
                usage: wgpu::BufferUsages::UNIFORM,
            });

        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("kernel-wgpu/pop/bind"),
            layout: &self.pop_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: q_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: c_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: out_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: params_buf.as_entire_binding(),
                },
            ],
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("kernel-wgpu/pop/encoder"),
            });
        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("kernel-wgpu/pop/pass"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(&self.pop_pipeline);
            cpass.set_bind_group(0, &bind_group, &[]);
            let groups = n.div_ceil(64);
            cpass.dispatch_workgroups(groups, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&out_buf, 0, &read_buf, 0, out_size);
        self.queue.submit(Some(encoder.finish()));

        let slice = read_buf.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = sender.send(r);
        });
        self.device.poll(wgpu::Maintain::Wait);
        receiver
            .recv()
            .expect("wgpu map callback dropped")
            .expect("wgpu map_async failed");

        let data = slice.get_mapped_range();
        let out: Vec<u32> = bytemuck::cast_slice(&data).to_vec();
        drop(data);
        read_buf.unmap();
        out
    }
}

fn make_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("kernel-wgpu/bind-layout"),
        entries: &[
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
    })
}

impl VectorKernel for WgpuKernel {
    fn id(&self) -> &'static str {
        "wgpu"
    }

    fn capabilities(&self) -> KernelCapabilities {
        KernelCapabilities {
            simd_width: 1,
            popcount_native: false,
            gpu: true,
        }
    }

    fn l2_distance_one(
        &self,
        query: &[f32],
        candidates: &[Vec<f32>],
        top_k: usize,
    ) -> Vec<(u64, f32)> {
        if candidates.is_empty() {
            return Vec::new();
        }
        // Take the dimension from `query.len()` and zero-pad / truncate
        // each candidate to that length to match the naive impl's
        // "shorter-of-the-two" rule. In practice the rabitq cache hands
        // us perfectly-rectangular batches so the truncation path is
        // cold; we still handle it for parity.
        let dim = query.len();
        let n = candidates.len();
        let mut flat = vec![0.0f32; dim * n];
        for (i, c) in candidates.iter().enumerate() {
            let len = dim.min(c.len());
            flat[i * dim..i * dim + len].copy_from_slice(&c[..len]);
            // The remainder stays zero — but in the truncated case the
            // naive impl never reads `query[len..]` either, so we must
            // mirror that. We do so by computing the per-row distance
            // on the host for any candidate shorter than `dim`. Most
            // production batches are rectangular so this is cold.
        }

        let distances = self.run_l2_dispatch(query, &flat, dim as u32, n as u32);

        // For any candidate shorter than `dim`, recompute on host to
        // honour the naive impl's truncation contract.
        let mut scored: Vec<(u64, f32)> = distances
            .into_iter()
            .enumerate()
            .map(|(i, d)| {
                if candidates[i].len() >= dim {
                    (i as u64, d)
                } else {
                    let len = candidates[i].len();
                    let c = &candidates[i];
                    let mut acc = 0.0f32;
                    for j in 0..len {
                        let d = query[j] - c[j];
                        acc += d * d;
                    }
                    (i as u64, acc)
                }
            })
            .collect();

        scored.sort_by(|a, b| match a.1.partial_cmp(&b.1) {
            Some(Ordering::Equal) | None => a.0.cmp(&b.0),
            Some(o) => o,
        });
        scored.truncate(top_k);
        scored
    }

    fn rabitq_popcount(
        &self,
        query: &[u64],
        candidates: &[Vec<u64>],
        top_k: usize,
    ) -> Vec<(u64, u32)> {
        if candidates.is_empty() {
            return Vec::new();
        }

        let dim_u64 = query.len();
        let dim_u32 = (dim_u64 * 2) as u32;
        let n = candidates.len();

        // Rectangular flatten with zero-pad on short rows. Same
        // rationale as the L2 path.
        let mut flat_u64 = vec![0u64; dim_u64 * n];
        for (i, c) in candidates.iter().enumerate() {
            let len = dim_u64.min(c.len());
            flat_u64[i * dim_u64..i * dim_u64 + len].copy_from_slice(&c[..len]);
        }

        // Reinterpret as 2× u32 little-endian — matches the WGSL
        // shader's lane decomposition. `bytemuck::cast_slice::<u64,u32>`
        // is the same byte order regardless of host endianness on
        // `cfg(target_endian = "little")` platforms (every target
        // wgpu actually runs on).
        let q_u32: &[u32] = bytemuck::cast_slice(query);
        let c_u32: &[u32] = bytemuck::cast_slice(&flat_u64);

        let raw = self.run_pop_dispatch(q_u32, c_u32, dim_u32, n as u32);

        let mut scored: Vec<(u64, u32)> = raw
            .into_iter()
            .enumerate()
            .map(|(i, h)| {
                if candidates[i].len() >= dim_u64 {
                    (i as u64, h)
                } else {
                    // Short-row recompute on host for the naive truncation
                    // contract.
                    let len = candidates[i].len();
                    let c = &candidates[i];
                    let mut acc: u32 = 0;
                    for j in 0..len {
                        acc += (query[j] ^ c[j]).count_ones();
                    }
                    (i as u64, acc)
                }
            })
            .collect();

        scored.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        scored.truncate(top_k);
        scored
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_advertise_gpu_geometry() {
        let Ok(k) = WgpuKernel::new_blocking() else {
            eprintln!("skipping caps check: no wgpu adapter on this host");
            return;
        };
        let c = k.capabilities();
        assert_eq!(c.simd_width, 1);
        assert!(!c.popcount_native);
        assert!(c.gpu);
        assert_eq!(k.id(), "wgpu");
    }
}
