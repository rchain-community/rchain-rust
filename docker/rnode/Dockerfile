# syntax=docker/dockerfile:1

# --- Build stage ---------------------------------------------------------------
FROM rust:1.95.0 AS builder

# `protoc` (prost-build .proto codegen) + the protobuf well-known types
# (`google/protobuf/empty.proto` etc., shipped by libprotobuf-dev) + build-essential
# + perl (ring/lmdb-rkv C builds, already in the full rust image).
RUN apt-get update \
    && apt-get install -y --no-install-recommends protobuf-compiler libprotobuf-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY . .

# The revision this image is built from, so the binary can name it (`/version`, `/status`, `--version`).
#
# **The build argument exists because the context cannot answer it.** `.dockerignore` excludes `.git/`,
# so `node/build.rs`'s `git rev-parse HEAD` finds nothing here and the image served
# `RChain Node 0.1.0 (commit # unknown)` — measured, and the exact question the provenance work
# (issue #32) set out to answer for the artifact that actually ships. `tools/devnet.sh build` passes
# `--build-arg GIT_HEAD_COMMIT=$(git rev-parse HEAD)`; a plain `docker build` without it keeps the
# honest `commit # unknown` rather than inventing one.
ARG GIT_HEAD_COMMIT=""

# Profiling knobs for the memory investigations (#117), all defaulting to the shipped build.
# `CARGO_FEATURES=dhat-heap` installs dhat's global allocator and `CARGO_FEATURES=jemalloc-profiling`
# compiles jemalloc's sampler in; `RUSTFLAGS=-C debuginfo=1` is what makes the resulting profile
# *nameable* — the default release profile is `debug = 0` and the shipped binary carries no debug
# sections at all, so a profile built without it is addresses with no symbols. They are build args
# rather than defaults so the artifact that ships is unchanged.
ARG CARGO_FEATURES=""
ARG RUSTFLAGS=""
# The allocator's own configuration, for a profiling build: `prof:true,lg_prof_sample:20,prof_prefix:…`
# on top of the purge settings `.cargo/config.toml` compiles in. **The empty case is guarded, and that
# is not decoration**: `ARG NAME=""` puts a *set but empty* variable into the `RUN` environment, and
# cargo's `[env]` does not override a variable that already exists — so a bare `export` would strip the
# purge settings from the shipped image and nothing would report it.
ARG JEMALLOC_SYS_WITH_MALLOC_CONF=""
RUN ${JEMALLOC_SYS_WITH_MALLOC_CONF:+export JEMALLOC_SYS_WITH_MALLOC_CONF="$JEMALLOC_SYS_WITH_MALLOC_CONF";} \
    cargo build --release -p rchain-node --bin rnode ${CARGO_FEATURES:+--features "$CARGO_FEATURES"}

# --- Runtime stage -------------------------------------------------------------
# Must match the builder's glibc (rust:1.95.0 = Debian trixie / glibc 2.41).
FROM debian:trixie-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/rnode /usr/local/bin/rnode

# `--profile docker` points `storage.data-dir` at /var/lib/rnode.
RUN useradd --create-home rnode \
    && mkdir -p /var/lib/rnode \
    && chown -R rnode:rnode /var/lib/rnode

# Healthy once the public HTTP API answers /api/status.
HEALTHCHECK --interval=10s --timeout=3s --start-period=30s --retries=5 \
    CMD curl -fsS http://localhost:40403/api/status || exit 1

USER rnode
WORKDIR /var/lib/rnode

ENTRYPOINT ["rnode"]
