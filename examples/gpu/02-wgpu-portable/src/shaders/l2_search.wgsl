// L2 brute-force scan compute shader.
//
// One workgroup-thread per candidate vector, exactly the same kernel
// as the CUDA version in module 01 — just expressed in WGSL so it can
// run on Vulkan / Metal / DX12 / WebGPU. Workgroup size 256 is a
// universally-safe choice (every backend supports it) and gives one
// workgroup per 256 candidates, which is a friendly granularity for
// dispatch.
//
// Buffer bindings:
//   @group(0) @binding(0)  query           : [dim]      f32   (read-only)
//   @group(0) @binding(1)  vectors         : [n × dim]  f32   (read-only)
//   @group(0) @binding(2)  out_distances   : [n]        f32   (read-write)
//   @group(0) @binding(3)  params          : Params   (uniform)
//
// Note: WGSL doesn't expose a pointer-cast we can use for SoA<->AoS
// transposition, so we read the same row-major layout the CUDA kernel
// reads. Adjacent workgroup threads scan adjacent candidates, which
// keeps the device's loads coalesced on every backend we tested.

struct Params {
    n_vectors: u32,
    dim: u32,
};

@group(0) @binding(0) var<storage, read>       query         : array<f32>;
@group(0) @binding(1) var<storage, read>       vectors       : array<f32>;
@group(0) @binding(2) var<storage, read_write> out_distances : array<f32>;
@group(0) @binding(3) var<uniform>             params        : Params;

@compute @workgroup_size(256, 1, 1)
fn l2_brute_force(@builtin(global_invocation_id) gid: vec3<u32>) {
    let idx: u32 = gid.x;
    if (idx >= params.n_vectors) {
        return;
    }
    let dim: u32 = params.dim;
    let row_off: u32 = idx * dim;

    var acc: f32 = 0.0;
    for (var k: u32 = 0u; k < dim; k = k + 1u) {
        let d: f32 = query[k] - vectors[row_off + k];
        // fma is available as `fma(...)` in WGSL but the codegen path
        // is identical to the explicit form on every backend we
        // tested. Keep the explicit form so the source reads like the
        // CPU baseline.
        acc = acc + d * d;
    }
    out_distances[idx] = acc;
}
