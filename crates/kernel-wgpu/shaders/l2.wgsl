// L2² distance compute shader for ADR-157 WgpuKernel.
//
// Layout:
//   - `query`      : storage<read>     — `dim` f32s, the query vector
//   - `candidates` : storage<read>     — `n * dim` f32s, row-major
//   - `out`        : storage<read_write> — `n` f32s, distance per candidate
//   - `params`     : uniform           — { dim: u32, n: u32 }
//
// Dispatch: one thread per candidate. Workgroup size 64; the host
// dispatches `ceil(n / 64)` workgroups. The host sorts the resulting
// distances and applies the (lower-index-wins) tie-break.

struct Params {
  dim: u32,
  n  : u32,
};

@group(0) @binding(0) var<storage, read>       query     : array<f32>;
@group(0) @binding(1) var<storage, read>       candidates: array<f32>;
@group(0) @binding(2) var<storage, read_write> out       : array<f32>;
@group(0) @binding(3) var<uniform>             params    : Params;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  let i = gid.x;
  if (i >= params.n) {
    return;
  }
  let base = i * params.dim;
  var acc: f32 = 0.0;
  for (var j: u32 = 0u; j < params.dim; j = j + 1u) {
    let d = query[j] - candidates[base + j];
    acc = acc + d * d;
  }
  out[i] = acc;
}
