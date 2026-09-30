#!/usr/bin/env bash
# Name the live structure that fills the cgroup ceiling (#117).
#
# The peak is **live Rust allocations** — measured, not inferred: jemalloc's own `stats.allocated` tracks
# the cgroup's `anon` at 0.968-0.994 over nine nodes in three attempts, with `retained` at 0 in every
# sample. So the fix space is the code, and the question is which allocation site owns the bytes.
#
# jemalloc's sampling profiler answers exactly that: one stack unwind per 2^lg_prof_sample allocations
# (1 in 1,048,576 here), which is light enough not to change the run — unlike `dhat`, which instruments
# every allocation and whose build reproduces no ramp at all.
#
# **The dump is triggered by the shim, not by jemalloc's exit hooks.** `prof_final` and `stats_print` both
# write when the process exits, after the node has freed its world (measured: 3258 MiB of `anon` printed as
# `Allocated: 2.9 MiB`). The shim watches its own cgroup and calls `prof.dump` at 1.5 GiB and every 1.5 GiB
# above that, so the profile shows what dominates at several sizes rather than at one arbitrary moment.
#
# The build must carry the sampler (`jemalloc-profiling`, opt-in since the sampler was made an opt-in —
# `node/Cargo.toml` carries the reasoning) and `-C debuginfo=1` (or the dump is addresses with no names: the
# release profile is `debug = 0`), plus `--export-dynamic` so the shim can reach `mallctl`. The profiling
# keys are no longer in the compiled-in default, so the build is:
#
#   JEMALLOC_SYS_WITH_MALLOC_CONF='background_thread:true,dirty_decay_ms:0,muzzy_decay_ms:0,retain:false,stats_print:true,stats_print_opts:g,prof:true,lg_prof_sample:20,prof_prefix:/var/lib/rnode/jeprof' \
#   RNODE_BUILD_MALLOC_CONF="$JEMALLOC_SYS_WITH_MALLOC_CONF" \
#   RNODE_BUILD_FEATURES=jemalloc-profiling \
#   RNODE_BUILD_RUSTFLAGS='-C debuginfo=1 -C link-arg=-Wl,--export-dynamic' \
#     tools/devnet.sh build
#
# `JEMALLOC_SYS_WITH_MALLOC_CONF` is read by `tikv-jemalloc-sys`'s build script, and jemalloc marks those
# options read-only once it starts, so a runtime `MALLOC_CONF` cannot set them — the conf has to reach the
# build, which is why it is an argument here rather than an environment variable for the container.
set -u
cd /home/patrick/RNodeRust
CAP=${CAP:-8g}
DUMP_AT_MB=${DUMP_AT_MB:-1500}
DUMP_STEP_MB=${DUMP_STEP_MB:-1500}
WINDOW_S=${WINDOW_S:-300}
SHIM=jemalloc-stats-shim.so
declare -A CONTAINERS=([bootstrap]=devnet-bootstrap [v1]=devnet-validator-1 [v2]=devnet-validator-2)

echo "tree=$(git rev-parse --short HEAD) cap=$CAP dump_at=${DUMP_AT_MB}MiB step=${DUMP_STEP_MB}MiB"
echo "image=$(docker inspect rnode:local --format '{{.Id}}')"

if [[ ! -f "spec/audit/evidence/$SHIM" ]]; then
  gcc -shared -fPIC -O2 -o "spec/audit/evidence/$SHIM" \
    spec/audit/evidence/jemalloc-stats-shim.c -ldl -pthread || exit 1
fi
cp -f "spec/audit/evidence/$SHIM" "examples/$SHIM"

rm -f target/n117-audit/jeprof*.heap target/n117-audit/profile-stats-*.txt
tools/devnet.sh down >/dev/null 2>&1
DEVNET_NODE_MEMORY="$CAP" \
DEVNET_NODE_ENV="LD_PRELOAD=/contracts/$SHIM;JEMALLOC_DUMP_ANON_MIB=$DUMP_AT_MB;JEMALLOC_DUMP_STEP_MIB=$DUMP_STEP_MB" \
  timeout 900 tools/devnet.sh up --validators 3 --stakes 100,100,50 --epoch-length 10 --fresh 2>&1 | tail -1

up=$(docker ps --format '{{.Names}}' | grep -c '^devnet-' || true)
echo "containers up: $up of 3"
[[ "$up" -ge 3 ]] || { echo "VOID: devnet did not come up"; tools/devnet.sh down >/dev/null 2>&1; exit 1; }

sleep 5
echo "--- the shim's view of the process, read back out of jemalloc ---"
docker exec devnet-bootstrap sh -c 'head -3 /var/lib/rnode/jemalloc-stats.txt' 2>/dev/null

echo "--- running for ${WINDOW_S}s; dumps land at ${DUMP_AT_MB} MiB and every ${DUMP_STEP_MB} MiB after ---"
for _ in $(seq 1 "$WINDOW_S"); do
  alive=$(docker ps --format '{{.Names}}' | grep -c '^devnet-' || true)
  [[ "$alive" -eq 0 ]] && { echo "  all nodes dead at ${_}s"; break; }
  sleep 1
done

echo "=== dumps taken ==="
for n in bootstrap v1 v2; do
  c=${CONTAINERS[$n]}
  echo "--- $n ---"
  docker logs "$c" 2>&1 | grep -a 'PROF_DUMP' | sed 's/^/  /' || echo "  no dump recorded"
  docker cp "$c:/var/lib/rnode/." target/n117-audit/profile-out-$n/ >/dev/null 2>&1
  docker cp "$c:/var/lib/rnode/jemalloc-stats.txt" "target/n117-audit/profile-stats-$n.txt" >/dev/null 2>&1
done
find target/n117-audit -name 'jeprof*.heap' -o -name '*.heap' 2>/dev/null | grep -v '^$' | head -20
echo
echo "=== node outcome ==="
for c in devnet-bootstrap devnet-validator-1 devnet-validator-2; do
  docker inspect "$c" --format '  {{.Name}} {{.State.Status}} oom={{.State.OOMKilled}} exit={{.State.ExitCode}}' 2>/dev/null
done
rm -f "examples/$SHIM"
echo "(containers left up; 'tools/devnet.sh down' to clean up)"
