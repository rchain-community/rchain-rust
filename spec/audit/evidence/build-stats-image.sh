#!/usr/bin/env bash
# Put the locally-built, stats-enabled release binary into a thin image over `rnode:local`.
#
# `docker/rnode/Dockerfile` does `COPY . .` then `cargo build`, with no dependency-layer cache, so
# `tools/devnet.sh build` recompiles the entire workspace from scratch for what is a one-line change to
# `node/Cargo.toml`. The thin layer copies the same binary the same Dockerfile would have produced — the
# same package, the same `--release`, the same default features, no `RUSTFLAGS` — so nothing about the
# artifact changes except the allocator feature being measured.
#
# The runtime stage is `debian:trixie-slim` (glibc 2.41) and this host is glibc 2.39; a binary linked
# against the older libc runs on the newer one. Verified by running the devnet, not assumed.
set -eu
cd /home/patrick/RNodeRust
SRC=target/release/rnode
[[ -x "$SRC" ]] || { echo "no $SRC — run: cargo build --release -p rchain-node --bin rnode"; exit 1; }

echo "binary: $SRC  $(date -u -r "$SRC" +%H:%M:%S) UTC  $(du -h "$SRC" | cut -f1)"
echo "tree:   $(git rev-parse --short HEAD)"
echo "the feature this image adds, read back out of the binary itself:"
strings -a "$SRC" | grep -c 'stats.allocated' | sed 's/^/  stats.allocated occurrences: /'

CTX=$(mktemp -d)
trap 'rm -rf "$CTX"' EXIT
cp "$SRC" "$CTX/rnode"
cat > "$CTX/Dockerfile" <<'DOCKER'
FROM rnode:local
COPY rnode /usr/local/bin/rnode
DOCKER
docker build -q -t rnode:local "$CTX"
echo "rnode:local rebuilt -> $(docker inspect rnode:local --format '{{.Id}}')"
