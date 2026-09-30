#!/usr/bin/env bash
# Is the memory that reaches the cgroup ceiling **live** or **held**? One number decides it.
#
# `stats.allocated` is what jemalloc believes the application currently holds; `stats.resident` is what it
# has resident on the application's behalf. Read at the ceiling:
#
#   allocated ~ anon                    -> live.  A structure, and a heap profile can name it.
#   allocated << anon, resident ~ anon  -> held.  Purge rate; allocator configuration is the fix space.
#
# =====================================================================================================
# A CONTROLLED EXPERIMENT. Every variable is fixed before the first attempt and recorded. Two earlier
# versions of this measurement were not tests: one changed the image's build path *and* added an
# instrument that wrote ~150k log lines per tick, then diagnosed the wreckage instead of fixing the
# variables; the next read jemalloc's numbers at process exit, after the node had already freed its world
# (3258 MiB of anon at the ceiling, `Allocated: 2.9 MiB` at exit — different moments, not comparable).
#
#   tree            recorded by `git rev-parse HEAD`; the image is built from it by `tools/devnet.sh build`
#   image           the CANONICAL Dockerfile build — not a thin layer over a stale base
#   build flags     `RNODE_BUILD_RUSTFLAGS='-C link-arg=-Wl,--export-dynamic'`. This is the ONE difference
#                   from the shipped artifact, and it changes only the dynamic symbol table: jemalloc is
#                   linked statically, so without it `_rjem_mallctl` is unreachable by `dlsym` and no
#                   instrument can read jemalloc's accounting while the process runs. It does not change
#                   allocation behaviour. The shim records the resolved symbol in its own header, so an
#                   artifact cannot be mistaken for one taken without it.
#   allocator conf  the shipped default, compiled in (JEMALLOC_SYS_WITH_MALLOC_CONF): purge settings plus
#                   `stats_print:true,stats_print_opts:g`. No runtime allocator override.
#   instrumentation `LD_PRELOAD=/contracts/jemalloc-stats-shim.so` — 1 Hz, one line per sample, taking
#                   jemalloc's stats lock once a second. NOT `stats_interval`, which dumps per-arena blocks
#                   at ~150k lines per tick and perturbs the run it measures.
#   cgroup          8 GiB with swap off. Not 4 GiB: with a 4 GiB cap a 2500 MiB threshold leaves too little
#                   headroom and the cgroup OOM-killer wins the race — measured, 8 of 9 nodes died before
#                   the clean stop landed. 8 GiB is the configuration the reference clean-stop runs used.
#   shape           --validators 3 --stakes 100,100,50 --epoch-length 10 --fresh
#   load            none beyond the chain's own operation. One devnet at a time, never two.
#   endpoint        the node is stopped cleanly (SIGTERM, 180 s grace) when `anon` first exceeds
#                   THRESHOLD_MB. The shim means the endpoint no longer has to be a clean exit.
#   attempts        N=3, fixed in advance and unfiltered. The phenomenon is INTERMITTENT — three earlier
#                   unprofiled runs of this reproduction ramped 3 of 3, and an earlier set of three ramped
#                   2 of 3 — so one run decides nothing and reporting one would be cherry-picking.
#   void            fewer than three containers up, or the shim failing to resolve `mallctl`, voids the
#                   attempt: it measured the devnet or the instrument, not the node.
#
# PRE-REGISTERED INTERPRETATION (a = anon at peak, A = allocated, R = resident):
#   A/a >= 0.7                        -> LIVE
#   A/a <  0.3  and  R/a >= 0.7       -> HELD
#   otherwise                         -> AMBIGUOUS
# =====================================================================================================
set -u
cd /home/patrick/RNodeRust
ATTEMPTS=${ATTEMPTS:-3}
THRESHOLD_MB=${THRESHOLD_MB:-3000}
CAP=${CAP:-8g}
WINDOW_S=${WINDOW_S:-300}
SHIM=jemalloc-stats-shim.so
declare -A CONTAINERS=([bootstrap]=devnet-bootstrap [v1]=devnet-validator-1 [v2]=devnet-validator-2)

echo "tree=$(git rev-parse --short HEAD) cap=$CAP threshold=${THRESHOLD_MB}MiB window=${WINDOW_S}s attempts=$ATTEMPTS"
echo "image=$(docker inspect rnode:local --format '{{.Id}}' 2>/dev/null)"
RUN_DATE=$(date -u +%Y-%m-%d)

# Built from the tracked source if absent, so the artifact is reproducible from the repository alone —
# the `.so` is a build product and is not committed, the `.c` is.
if [[ ! -f "spec/audit/evidence/$SHIM" ]]; then
  echo "compiling $SHIM from spec/audit/evidence/jemalloc-stats-shim.c"
  gcc -shared -fPIC -O2 -o "spec/audit/evidence/$SHIM" \
    spec/audit/evidence/jemalloc-stats-shim.c -ldl -pthread || exit 1
fi
cp -f "spec/audit/evidence/$SHIM" "examples/$SHIM"

for attempt in $(seq 1 "$ATTEMPTS"); do
  echo "=================== attempt $attempt of $ATTEMPTS  $(date -u +%H:%M:%S) UTC ==================="
  tools/devnet.sh down >/dev/null 2>&1
  # Only one variable is injected, and it contains no commas — the separator that has shredded a
  # MALLOC_CONF in this work before. The allocator configuration itself is compiled into the image.
  DEVNET_NODE_MEMORY="$CAP" DEVNET_NODE_ENV="LD_PRELOAD=/contracts/$SHIM" timeout 900 \
    tools/devnet.sh up --validators 3 --stakes 100,100,50 --epoch-length 10 --fresh 2>&1 | tail -1

  up=$(docker ps --format '{{.Names}}' 2>/dev/null | grep -c '^devnet-' || true)
  if [[ "$up" -lt 3 ]]; then
    echo "  VOID: $up of 3 containers up"
    tools/devnet.sh down >/dev/null 2>&1
    continue
  fi
  sleep 5
  hdr=$(docker exec devnet-bootstrap sh -c 'head -1 /var/lib/rnode/jemalloc-stats.txt' 2>/dev/null)
  case "$hdr" in
    *"symbol=_rjem_mallctl"*|*"symbol=mallctl"*) echo "  three containers up; shim resolved: $hdr" ;;
    *) echo "  VOID: the shim did not resolve mallctl — this attempt would measure nothing"
       echo "        (header: ${hdr:-<no file>})"
       tools/devnet.sh down >/dev/null 2>&1
       continue ;;
  esac

  out="target/n117-audit/live-vs-held-attempt${attempt}.tsv"
  python3 target/n117-audit/queue-depth.py "$out" &
  sampler=$!

  declare -A stopped=()
  for _ in $(seq 1 $((WINDOW_S + 60))); do
    for n in bootstrap v1 v2; do
      [[ -n "${stopped[$n]:-}" ]] && continue
      c=${CONTAINERS[$n]}
      pid=$(docker inspect "$c" --format '{{.State.Pid}}' 2>/dev/null)
      [[ -z "$pid" || "$pid" == 0 ]] && continue
      cg="/sys/fs/cgroup$(sed -n 's/^0:://p' "/proc/$pid/cgroup" 2>/dev/null)"
      anon=$(awk '/^anon /{print int($2/1048576)}' "$cg/memory.stat" 2>/dev/null)
      if [[ -n "$anon" && "$anon" -gt "$THRESHOLD_MB" ]]; then
        echo "  $n crossed at anon=${anon} MiB — stopping cleanly"
        docker stop -t 180 "$c" >/dev/null
        stopped[$n]=1
      fi
    done
    [[ ${#stopped[@]} -ge 3 ]] && break
    kill -0 $sampler 2>/dev/null || break
    sleep 1
  done
  wait $sampler 2>/dev/null

  for n in bootstrap v1 v2; do
    docker cp "${CONTAINERS[$n]}:/var/lib/rnode/jemalloc-stats.txt" \
      "target/n117-audit/stats-${n}-a${attempt}.txt" >/dev/null 2>&1 \
      || echo "  $n: no stats file to copy"
  done
  tools/devnet.sh down >/dev/null 2>&1
  echo "  attempt $attempt done"
done

rm -f "examples/$SHIM"
echo
echo "=== analysis (pre-registered bands) ==="
python3 target/n117-audit/align-live-vs-held.py "$RUN_DATE" \
  target/n117-audit/live-vs-held-attempt*.tsv target/n117-audit/stats-*-a*.txt 2>&1
