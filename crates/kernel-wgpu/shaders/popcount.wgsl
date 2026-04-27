// rabitq popcount-XOR Hamming compute shader for ADR-157 WgpuKernel.
//
// WGSL has no native u64. We pack each `u64` query/candidate word as
// two consecutive `u32`s (little-endian, matching `bytemuck::cast_slice::<u64,u32>`
// on the host); both halves are XORed and `countOneBits` is applied to
// each half independently, then summed. This is bit-equal to the host
// `(q ^ c).count_ones()` over the full `u64` because popcount
// distributes over disjoint bit-spans.
//
// Layout:
//   - `query`      : storage<read>     — `dim_u32` u32s   (= 2 * dim_u64)
//   - `candidates` : storage<read>     — `n * dim_u32` u32s, row-major
//   - `out`        : storage<read_write> — `n` u32s, hamming distance per candidate
//   - `params`     : uniform           — { dim_u32: u32, n: u32 }

struct Params {
  dim_u32: u32,
  n      : u32,
};

@group(0) @binding(0) var<storage, read>       query     : array<u32>;
@group(0) @binding(1) var<storage, read>       candidates: array<u32>;
@group(0) @binding(2) var<storage, read_write> out       : array<u32>;
@group(0) @binding(3) var<uniform>             params    : Params;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  let i = gid.x;
  if (i >= params.n) {
    return;
  }
  let base = i * params.dim_u32;
  var acc: u32 = 0u;
  for (var j: u32 = 0u; j < params.dim_u32; j = j + 1u) {
    acc = acc + countOneBits(query[j] ^ candidates[base + j]);
  }
  out[i] = acc;
}
