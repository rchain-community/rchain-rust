#!/usr/bin/env bash
# Does the cgroup ramp still happen, now that the merge's conflict search is bounded? (#117)
#
# This is `n117-live-vs-held-run.sh` — the frozen reproduction, same constants, same pre-registered
# interpretation — pointed at a different question. That one asked **is the memory at the ceiling live or
# held**; this one asks **is there still a ceiling**, which is the only question that decides whether
# C177/C178 were the defect.
#
# =====================================================================================================
# CONTROLLED EXPERIMENT. The variables are the ones `n117-live-vs-held-run.sh` fixed, unchanged, so the
# two runs are comparable:
#
#   tree            `git rev-parse HEAD`; the image is a thin layer over `rnode:local` containing the
#                   locally built release binary of this tree (`build-stats-image.sh`) — the same package,
#                   profile and default features the Dockerfile would produce, so the only difference
#                   from the shipped artifact is the allocator feature the binary already carried.
#   allocator conf  the shipped default, compiled in. No runtime allocator override.
#   instrumentation `LD_PRELOAD=/contracts/jemalloc-stats-shim.so` — 1 Hz.
#   cgroup          8 GiB with swap off (NOT 4 GiB: at 4 GiB the OOM-killer wins the race against the
#                   clean stop — measured, 8 of 9 nodes died before it landed).
#   shape           --validators 3 --stakes 100,100,50 --epoch-length 10 --fresh
#   load            none beyond the chain's own operation. One devnet at a time, never two.
#   endpoint        a node is stopped cleanly when `anon` first exceeds THRESHOLD_MB, so a node that
#                   still ramps is still caught rather than killed mid-write.
#   attempts        N=3, fixed in advance and unfiltered. The phenomenon is INTERMITTENT, so one run
#                   decides nothing.
#   void            fewer than three containers up, or the shim failing to resolve `mallctl`.
#
# WHAT IS READ OFF IT (this is the new half, and it is a *count* rather than a byte):
#
#   `merge search: …` — the census line the node now logs every five seconds from `interpreter_util.rs`,
#   reporting the widest merge scope it has run, how dense that scope's conflicts were, how many pairs
#   were asymmetric, and the most states one search expanded. This is the measurement the fix did not
#   have: the shape had been *assumed* (a fork of twenty chains) and never read off a running node.
#
# PRE-REGISTERED OUTCOME: with the search bounded, no node's `anon` reaches THRESHOLD_MB within the
# window, i.e. **0 of 3 nodes cross**, against 2 of 3 on the same reproduction before the fix (and 3 of 3
# on the runs before that). A crossing is not automatically a failure of the fix — a node can grow for
# reasons that are not this search — but it must come with a census line whose `most states expanded` is
# large, or the two are unrelated.
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
mkdir -p target/n117-audit

if [[ ! -f "spec/audit/evidence/$SHIM" ]]; then
  echo "compiling $SHIM from spec/audit/evidence/jemalloc-stats-shim.c"
  gcc -shared -fPIC -O2 -o "spec/audit/evidence/$SHIM" \
    spec/audit/evidence/jemalloc-stats-shim.c -ldl -pthread || exit 1
fi
# The jemalloc shim is **not** used here, and that is a deliberate difference from
# `n117-live-vs-held-run.sh`: this run asks whether `anon` ramps and what the search's census says, and
# neither needs allocator accounting. Requiring it would force a full release rebuild for
# `-C link-arg=-Wl,--export-dynamic` (a `RUSTFLAGS` change invalidates every crate) to satisfy an
# instrument this question does not read. Set USE_SHIM=1 to restore it.
USE_SHIM=${USE_SHIM:-0}
if [[ "$USE_SHIM" == "1" ]]; then cp -f "spec/audit/evidence/$SHIM" "examples/$SHIM"; fi

for attempt in $(seq 1 "$ATTEMPTS"); do
  echo "=================== attempt $attempt of $ATTEMPTS  $(date -u +%H:%M:%S) UTC ==================="
  tools/devnet.sh down >/dev/null 2>&1
  if [[ "$USE_SHIM" == "1" ]]; then
    DEVNET_NODE_MEMORY="$CAP" DEVNET_NODE_ENV="LD_PRELOAD=/contracts/$SHIM" timeout 900 \
      tools/devnet.sh up --validators 3 --stakes 100,100,50 --epoch-length 10 --fresh 2>&1 | tail -1
  else
    DEVNET_NODE_MEMORY="$CAP" timeout 900 \
      tools/devnet.sh up --validators 3 --stakes 100,100,50 --epoch-length 10 --fresh 2>&1 | tail -1
  fi

  up=$(docker ps --format '{{.Names}}' 2>/dev/null | grep -c '^devnet-' || true)
  if [[ "$up" -lt 3 ]]; then
    echo "  VOID: $up of 3 containers up"
    tools/devnet.sh down >/dev/null 2>&1
    continue
  fi
  sleep 5
  if [[ "$USE_SHIM" != "1" ]]; then
    echo "  three containers up; no shim (USE_SHIM=0)"
  else
  hdr=$(docker exec devnet-bootstrap sh -c 'head -1 /var/lib/rnode/jemalloc-stats.txt' 2>/dev/null)
  case "$hdr" in
    *"symbol=_rjem_mallctl"*|*"symbol=mallctl"*) echo "  three containers up; shim resolved: $hdr" ;;
    *) echo "  VOID: the shim did not resolve mallctl — this attempt would measure nothing"
       echo "        (header: ${hdr:-<no file>})"
       tools/devnet.sh down >/dev/null 2>&1
       continue ;;
  esac
  fi

  out="target/n117-audit/after-fix-attempt${attempt}.tsv"
  python3 target/n117-audit/queue-depth.py "$out" &
  sampler=$!

  declare -A stopped=()
  declare -A peak=()
  for _ in $(seq 1 $((WINDOW_S + 60))); do
    for n in bootstrap v1 v2; do
      [[ -n "${stopped[$n]:-}" ]] && continue
      c=${CONTAINERS[$n]}
      pid=$(docker inspect "$c" --format '{{.State.Pid}}' 2>/dev/null)
      [[ -z "$pid" || "$pid" == 0 ]] && continue
      cg="/sys/fs/cgroup$(sed -n 's/^0:://p' "/proc/$pid/cgroup" 2>/dev/null)"
      anon=$(awk '/^anon /{print int($2/1048576)}' "$cg/memory.stat" 2>/dev/null)
      if [[ -n "$anon" && "$anon" -gt "${peak[$n]:-0}" ]]; then peak[$n]=$anon; fi
      if [[ -n "$anon" && "$anon" -gt "$THRESHOLD_MB" ]]; then
        echo "  $n CROSSED at anon=${anon} MiB — stopping cleanly"
        docker stop -t 180 "$c" >/dev/null
        stopped[$n]=1
      fi
    done
    [[ ${#stopped[@]} -ge 3 ]] && break
    kill -0 $sampler 2>/dev/null || break
    sleep 1
  done
  wait $sampler 2>/dev/null

  echo "  peaks: bootstrap=${peak[bootstrap]:-?} v1=${peak[v1]:-?} v2=${peak[v2]:-?} MiB (threshold ${THRESHOLD_MB})"
  echo "  crossed: ${#stopped[@]} of 3"

  # The census, from the node's own log. This is the measurement the fix did not have.
  for n in bootstrap v1 v2; do
    docker logs "${CONTAINERS[$n]}" > "target/n117-audit/after-fix-log-${n}-a${attempt}.txt" 2>&1
    line=$(grep 'merge search' "target/n117-audit/after-fix-log-${n}-a${attempt}.txt" | tail -1)
    echo "  $n census: ${line:-<none — the merge search never ran, or the log level hid it>}"
  done
  if [[ "$USE_SHIM" == "1" ]]; then
    for n in bootstrap v1 v2; do
      docker cp "${CONTAINERS[$n]}:/var/lib/rnode/jemalloc-stats.txt" \
        "target/n117-audit/after-fix-stats-${n}-a${attempt}.txt" >/dev/null 2>&1 \
        || echo "  $n: no stats file to copy"
    done
  fi
  tools/devnet.sh down >/dev/null 2>&1
  echo "  attempt $attempt done"
done

rm -f "examples/$SHIM"
echo
echo "=== summary ==="
for attempt in $(seq 1 "$ATTEMPTS"); do
  for n in bootstrap v1 v2; do
    f="target/n117-audit/after-fix-log-${n}-a${attempt}.txt"
    [[ -f "$f" ]] || continue
    echo "a${attempt} ${n} $(grep 'merge search' "$f" | tail -1)"
  done
done
