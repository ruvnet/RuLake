#!/usr/bin/env node
//
// Hermetic IPFS gateway fixture for the Console smoke (issue #19).
//
// Serves `GET /ipfs/<cid>` for a fixed set of local files, each under its
// real CIDv1 (raw codec 0x55, sha256 multihash, base32 "b..." multibase),
// so a CI run never depends on a rate-limited public gateway. Every
// response is re-hashed before it is sent: the gateway can only ever serve
// bytes that match the CID it was asked for. CORS is open (`*`) because
// the Console fetches with `mode: 'cors'` from the vite preview origin.
//
// Usage:
//   node ipfs-fixture-gateway.mjs --cid-of <file>              # print CID, exit
//   node ipfs-fixture-gateway.mjs --port <n> <file> [<file>…]  # serve
//
// The live public-gateway check runs on a schedule instead:
// .github/workflows/ipfs-gateway-probe.yml.

import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { createServer } from 'node:http';

const B32 = 'abcdefghijklmnopqrstuvwxyz234567';

function base32(bytes) {
  let bits = 0;
  let value = 0;
  let out = '';
  for (const byte of bytes) {
    value = (value << 8) | byte;
    bits += 8;
    while (bits >= 5) {
      out += B32[(value >>> (bits - 5)) & 31];
      bits -= 5;
    }
  }
  if (bits > 0) out += B32[(value << (5 - bits)) & 31];
  return out;
}

export function cidV1Raw(bytes) {
  const digest = createHash('sha256').update(bytes).digest();
  // <cid-version 0x01><codec raw 0x55><multihash sha2-256 0x12><len 0x20><digest>
  const cid = Buffer.concat([Buffer.from([0x01, 0x55, 0x12, 0x20]), digest]);
  return 'b' + base32(cid);
}

function usage(msg) {
  if (msg) process.stderr.write(`ipfs-fixture-gateway: ${msg}\n`);
  process.stderr.write('usage: ipfs-fixture-gateway.mjs --cid-of <file> | --port <n> <file>...\n');
  process.exit(2);
}

const args = process.argv.slice(2);
if (args[0] === '--cid-of') {
  if (!args[1]) usage('--cid-of needs a file');
  process.stdout.write(cidV1Raw(readFileSync(args[1])) + '\n');
  process.exit(0);
}

let port = 0;
const files = [];
for (let i = 0; i < args.length; i++) {
  if (args[i] === '--port') {
    port = Number(args[++i]);
    if (!Number.isInteger(port) || port < 1 || port > 65535) usage('invalid --port');
  } else {
    files.push(args[i]);
  }
}
if (!port || files.length === 0) usage();

const objects = new Map();
for (const f of files) {
  const bytes = readFileSync(f);
  objects.set(cidV1Raw(bytes), bytes);
}

const CORS = {
  'access-control-allow-origin': '*',
  'access-control-allow-methods': 'GET, HEAD, OPTIONS',
  'access-control-allow-headers': '*',
};

const server = createServer((req, res) => {
  if (req.method === 'OPTIONS') {
    res.writeHead(204, CORS);
    res.end();
    return;
  }
  const m = /^\/ipfs\/([a-z2-7]{20,100})(?:\?.*)?$/.exec(req.url || '');
  const bytes = m && objects.get(m[1]);
  if ((req.method !== 'GET' && req.method !== 'HEAD') || !bytes) {
    res.writeHead(404, { ...CORS, 'content-type': 'text/plain' });
    res.end('not found\n');
    return;
  }
  if (cidV1Raw(bytes) !== m[1]) {
    // Unreachable unless the in-memory map is corrupted; never serve
    // bytes that do not hash to the requested CID.
    res.writeHead(500, CORS);
    res.end();
    return;
  }
  res.writeHead(200, {
    ...CORS,
    'content-type': 'application/octet-stream',
    'content-length': bytes.length,
    etag: `"${m[1]}"`,
  });
  res.end(req.method === 'HEAD' ? undefined : bytes);
});

server.listen(port, '127.0.0.1', () => {
  for (const cid of objects.keys()) {
    process.stdout.write(`serving /ipfs/${cid} on http://127.0.0.1:${port}\n`);
  }
});
for (const sig of ['SIGINT', 'SIGTERM']) process.on(sig, () => server.close(() => process.exit(0)));
