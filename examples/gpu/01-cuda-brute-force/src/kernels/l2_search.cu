// L2 brute-force scan kernel.
//
// One thread per candidate vector. Each thread reads `dim` floats from
// the candidate row and the (replicated) query, accumulates the squared
// L2 distance, and writes one float into `out_distances[idx]`.
//
// We deliberately keep this kernel minimal: top-K reduction is done on
// the host after `cudaMemcpy` D->H. For the corpus sizes targeted by
// this example (≤ ~1M vectors at dim ≤ 1024) the memcpy + host top-K
// is negligible next to the kernel itself, and the kernel reads more
// readably without warp-shuffle reduction smeared in. See
// `examples/gpu/03-rabitq-gpu-design-note/README.md` for the on-device
// top-K sketch we'd need for a production rabitq port.
//
// Memory layout: vectors are laid out row-major in `vectors`, so
// candidate `idx` lives at `vectors + idx * dim` and is read with
// stride 1 by thread `idx`. That gives coalesced loads when adjacent
// threads in a warp scan adjacent candidates (the common case for
// brute-force scan). For scattered top-K rerank passes the layout
// matters less — the pass is bandwidth-limited either way.
//
// `restrict` qualifiers tell the compiler the buffers don't alias,
// which it can't know on its own across kernel launches.

extern "C" __global__ void l2_brute_force(
    const float * __restrict__ query,    // [dim]
    const float * __restrict__ vectors,  // [n_vectors * dim], row-major
    float       * __restrict__ out_distances,  // [n_vectors]
    unsigned int n_vectors,
    unsigned int dim
) {
    unsigned int idx = blockIdx.x * blockDim.x + threadIdx.x;
    if (idx >= n_vectors) {
        return;
    }
    const float * v = vectors + (size_t) idx * (size_t) dim;
    float acc = 0.0f;
    // Loop unrolling beyond 4 hurts register pressure on ampere+ for
    // typical embedding dims. The compiler picks a sensible factor
    // with --use_fast_math.
    #pragma unroll 4
    for (unsigned int k = 0; k < dim; ++k) {
        float d = query[k] - v[k];
        acc = fmaf(d, d, acc);
    }
    out_distances[idx] = acc;
}
