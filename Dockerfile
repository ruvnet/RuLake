# syntax=docker/dockerfile:1.7
#
# Multi-stage build for ruLake.
#
#   docker build -t rulake .
#   docker run --rm rulake               # full benchmark (~2 min)
#   docker run --rm rulake --fast        # quick smoke (~5 s)
#   docker run --rm --entrypoint cargo rulake test --release
#
# The build stage compiles cargo deps once via the BuildKit cache, then the
# project. The runtime stage carries only the demo binary on a slim base.

# ---- builder ----
FROM rust:1.83-bookworm AS builder

WORKDIR /build

# Bring the manifest first so dependency compilation is cached separately
# from source edits. The vendored rabitq path-dep needs the submodule, so we
# require COPY of vendor/ before deps will resolve — see Dockerfile.dockerignore
# for what we exclude.
COPY Cargo.toml ./
COPY vendor ./vendor

# Pre-compile deps with a no-op lib + bin. This produces a cached layer that
# survives source-only changes.
RUN mkdir -p src/bin tests examples \
    && echo 'fn main() {}' > src/bin/rulake-demo.rs \
    && echo 'pub fn _placeholder() {}' > src/lib.rs \
    && cargo build --release --bin rulake-demo \
    && rm -rf src target/release/deps/ruvector_rulake* target/release/deps/rulake_demo*

# Now the real source.
COPY src ./src
COPY tests ./tests
COPY examples ./examples
COPY README.md BENCHMARK.md LICENSE-MIT LICENSE-APACHE ./

RUN cargo build --release --bin rulake-demo

# ---- runtime ----
FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/rulake-demo /usr/local/bin/rulake-demo
COPY --from=builder /build/README.md /build/BENCHMARK.md /usr/share/doc/rulake/

ENTRYPOINT ["/usr/local/bin/rulake-demo"]
CMD []
