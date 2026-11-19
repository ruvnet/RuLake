import init, { searchBruteForceL2 } from 'rulake-wasm';

let snapshot;
let wasm;

function decode(buffer) {
  const size = buffer.byteLength;
  const view = new DataView(buffer);
  if (size < 24 || size > 16 * 1024 * 1024 ||
      view.getBigUint64(0, true) !== 0x316365767572n || view.getUint32(20, true) !== 0) {
    throw new Error('Choose a valid ruvec1 snapshot up to 16 MiB.');
  }
  const rows = Number(view.getBigUint64(8, true));
  const dim = view.getUint32(16, true);
  const stride = 8 + dim * 4;
  if (!rows || !dim || dim > 8192 || 24 + rows * stride !== size) {
    throw new Error('Snapshot dimensions and file size disagree.');
  }
  const vectors = new Float32Array(rows * dim);
  for (let row = 0; row < rows; row++) {
    const start = 24 + row * stride + 8;
    for (let col = 0; col < dim; col++) {
      const value = view.getFloat32(start + col * 4, true);
      if (!Number.isFinite(value)) throw new Error('Snapshot contains a nonfinite component.');
      vectors[row * dim + col] = value;
    }
  }
  return { rows, dim, stride, view, vectors };
}

self.onmessage = async ({ data: { query, file, sampleUrl } }) => {
  try {
    if (!snapshot) {
      let bytes;
      if (file) bytes = await file.arrayBuffer();
      else {
        const response = await fetch(sampleUrl);
        if (!response.ok) throw new Error('Could not load the sample snapshot.');
        bytes = await response.arrayBuffer();
      }
      snapshot = decode(bytes);
    }
    const { rows, dim, stride, view, vectors } = snapshot;
    if (query.length !== dim) throw new Error('This snapshot requires ' + dim + ' query components.');
    wasm ??= init();
    await wasm;
    // Let WASM return the row index. Read the external uint64 ID without a float64 round trip.
    const [hit] = searchBruteForceL2(vectors, new Float64Array(0), dim, query, 1);
    if (!Number.isFinite(hit.score)) throw new Error('Squared distance exceeds the float32 range.');
    const id = view.getBigUint64(24 + hit.idx * stride, true).toString();
    self.postMessage({ rows, id, score: hit.score });
  } catch (error) {
    self.postMessage({ error: error.message || String(error) });
  }
};
