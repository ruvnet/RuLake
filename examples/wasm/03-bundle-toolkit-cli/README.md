# 03 — bundle-toolkit-cli (Rust → wasm32-wasi)

A small CLI compiled to `wasm32-wasip1` (a.k.a. `wasm32-wasi`), runnable
inside any WASI runtime — `wasmtime`, `wasmer`, `wasmedge`, the Bytecode
Alliance crates, etc. No `wasm-bindgen`; pure WASI std and `clap`.

## What it proves

ruLake bundle verification can run inside an OS-agnostic, sandboxed
runtime — useful for build pipelines, CI gates, and any pipeline that
needs to verify untrusted bundles in a capability-restricted process.
The runtime decides which directories the wasm can read; the wasm itself
has no syscall surface beyond what's been granted.

## Build

```bash
./build.sh           # cargo build --target wasm32-wasip1 --release
```

Output:

```
target/wasm32-wasip1/release/bundle-toolkit.wasm
```

If your toolchain is older than 1.78, the script falls back to
`wasm32-wasi` (the previous alias).

## Install a runtime

Pick one. None is bundled here.

```bash
# wasmtime (Bytecode Alliance)
curl https://wasmtime.dev/install.sh -sSf | bash

# wasmer
curl https://get.wasmer.io -sSfL | sh
```

## Run

WASI sandboxes filesystem access — you must explicitly mount the
directory you want the wasm to read.

```bash
WASM=target/wasm32-wasip1/release/bundle-toolkit.wasm

# verify (exit 0 on match, 1 on mismatch, 2 on error)
wasmtime --dir=/tmp $WASM verify /tmp/rulake-fixture

# dump
wasmtime --dir=/tmp $WASM dump /tmp/rulake-fixture

# witness only (for shell pipelines)
wasmtime --dir=/tmp $WASM witness /tmp/rulake-fixture
# → dea58c64adb1eb4109438f0353a2b1749d4dc29ed7266e9236720ab6cf07d7e4
```

Wasmer is identical with `--mapdir=/tmp::/tmp`:

```bash
wasmer run --mapdir=/tmp::/tmp $WASM -- verify /tmp/rulake-fixture
```

## Subcommands

| command   | exit codes                       | notes                            |
|-----------|----------------------------------|----------------------------------|
| `verify`  | 0 = match, 1 = mismatch, 2 = err | suitable for `&&` chains         |
| `dump`    | 0 = ok, 2 = err                  | pretty-printed JSON. Same load + validation path as `verify`, so a tampered bundle exits 2 (err), not 0 with garbage. |
| `witness` | 0 = ok, 2 = err                  | only the hex digest, no decor    |

## Path inputs

The CLI takes paths via `clap` without an application-level length cap.
This is not a vulnerability — two outer layers already bound it:

- **WASI runtime + kernel** enforce `PATH_MAX` (4096 on Linux); arguments
  longer than that fail in the runtime before reaching the program.
- **WASI capability model** is the actual reachability boundary. The
  binary only sees the directories the host explicitly granted via
  `--dir=…` (wasmtime) or equivalent. Symlinks pointing outside the
  preopened tree are not followed by the WASI VFS unless the runtime
  is invoked with an explicit follow-symlinks flag.

In practice: if you `wasmtime --dir=/tmp run bundle-toolkit.wasm verify
/tmp/foo`, the binary cannot read `/etc/passwd` even if a malicious
sidecar named it as `data_ref`. Run the wasm with the narrowest
preopened-dir set you need.

## Generating a test bundle

```bash
cd /home/ruvultra/projects/RuLake
cargo run --release --example sidecar_daemon
# bundle ends up in /tmp/rulake-sidecar-demo-<pid>/table.rulake.json
```

Or use the captured fixture at `/tmp/rulake-fixture/table.rulake.json`
that was published by an earlier swarm step.

## File layout

```
03-bundle-toolkit-cli/
├── Cargo.toml
├── src/main.rs
├── build.sh
└── target/...      # generated, gitignored
```
