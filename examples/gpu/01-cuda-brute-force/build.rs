//! Compile the CUDA L2 kernel to PTX with `nvcc`.
//!
//! cudarc loads the resulting PTX at runtime via the driver API, so we
//! don't need to link against any CUDA libraries at build time. If
//! `nvcc` is unavailable, the build emits a `cargo:warning` and writes
//! a stub PTX that the runtime will refuse to load — this keeps
//! `cargo check` workable on machines without the toolchain while
//! producing a clear failure at run time.
//!
//! We deliberately don't `panic!` on a missing `nvcc`: the README is
//! the authoritative install instruction, and a hard build failure
//! makes the example uninspectable to readers without the toolchain.

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR set by cargo"));
    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR set by cargo"));
    let cu_path = manifest_dir.join("src/kernels/l2_search.cu");
    let ptx_path = out_dir.join("l2_search.ptx");

    println!("cargo:rerun-if-changed={}", cu_path.display());
    println!("cargo:rerun-if-env-changed=NVCC");
    println!("cargo:rerun-if-env-changed=CUDA_PATH");

    // Allow operators to override the nvcc binary location.
    let nvcc = env::var("NVCC").unwrap_or_else(|_| "nvcc".to_string());

    // Target a broad-but-recent compute capability range. sm_75 (Turing)
    // through sm_120 (RTX 5080 / Blackwell). PTX is forward-compatible:
    // the driver JIT-compiles to the host's actual SM at load time, so
    // a single PTX file works across this whole range.
    let mut cmd = Command::new(&nvcc);
    cmd.arg("-ptx")
        .arg("-O3")
        .arg("--use_fast_math")
        .arg("-arch=compute_75")
        .arg("-o")
        .arg(&ptx_path)
        .arg(&cu_path);

    match cmd.output() {
        Ok(out) if out.status.success() => {
            // Good — PTX written.
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            println!(
                "cargo:warning=nvcc failed to compile {}: {}",
                cu_path.display(),
                stderr.trim()
            );
            write_stub_ptx(&ptx_path, "nvcc reported a compilation error");
        }
        Err(e) => {
            println!(
                "cargo:warning=nvcc not invokable ({e}); the binary will fail at runtime \
                 with a clear error. Install the CUDA toolkit (>=12.0) and re-run \
                 `cargo build` to enable GPU execution."
            );
            write_stub_ptx(&ptx_path, &format!("nvcc unavailable: {e}"));
        }
    }
}

fn write_stub_ptx(ptx_path: &std::path::Path, reason: &str) {
    // A syntactically-valid-but-unloadable PTX placeholder. The runtime
    // load will surface this `reason` as the failure cause, so a
    // developer running the binary without nvcc gets an actionable
    // message instead of a confusing PTX parse error.
    let stub = format!(
        "// stub PTX: {reason}\n\
         .version 7.0\n\
         .target sm_50\n\
         .address_size 64\n"
    );
    std::fs::write(ptx_path, stub).expect("write stub PTX");
}
