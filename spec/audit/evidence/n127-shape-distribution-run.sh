#!/usr/bin/env bash
# Read the merge search's shape distribution off three live nodes (#127 Stage 1, register row C182).
#
# The pre-registered protocol is `n127-shape-distribution-preregistration.md` — the configuration, the
# acceptance rows and the single number to report were frozen before this ran, and an amendment is a new
# commit rather than an edit to that file.
#
# **The one difference from the census run, and it is deliberate.** `n117-after-fix-run.sh` stops a node the
# moment its `anon` crosses a threshold, which is what a *ramp* measurement needs and what a *distribution*
# cannot survive: the stop truncates exactly the widest scopes out of the sample. Here the threshold is set
# above the cgroup cap, so the nodes run the window and are scraped while they are still alive.
#
# Usage:  ATTEMPTS=3 WINDOW_S=300 spec/audit/evidence/n127-shape-distribution-run.sh
set -u
cd /home/patrick/RNodeRust
ATTEMPTS=${ATTEMPTS:-3}
CAP=${CAP:-8g}
WINDOW_S=${WINDOW_S:-300}
THRESHOLD_MB=${THRESHOLD_MB:-100000}
OUT=target/n127-dist
# The public HTTP API port per node, the same mapping `n117-queue-depth.py` samples.
declare -A PORTS=([bootstrap]=40403 [v1]=41403 [v2]=42403)
declare -A CONTAINERS=([bootstrap]=devnet-bootstrap [v1]=devnet-validator-1 [v2]=devnet-validator-2)

mkdir -p "$OUT"
echo "tree=$(git rev-parse --short HEAD) image=$(docker inspect rnode:local --format '{{.Id}}' 2>/dev/null)"
echo "cap=$CAP window=${WINDOW_S}s threshold=${THRESHOLD_MB}MiB attempts=$ATTEMPTS"

for attempt in $(seq 1 "$ATTEMPTS"); do
  echo "=================== attempt $attempt of $ATTEMPTS  $(date -u +%H:%M:%S) UTC ==================="
  tools/devnet.sh down >/dev/null 2>&1
  DEVNET_NODE_MEMORY="$CAP" timeout 900 \
    tools/devnet.sh up --validators 3 --stakes 100,100,50 --epoch-length 10 --fresh 2>&1 | tail -1

  up=$(docker ps --format '{{.Names}}' 2>/dev/null | grep -c '^devnet-' || true)
  if [[ "$up" -lt 3 ]]; then
    echo "  VOID: $up of 3 containers up"
    tools/devnet.sh down >/dev/null 2>&1
    continue
  fi

  # The window. A node that dies mid-window is recorded as such rather than silently dropped: a
  # distribution from two nodes is a different sample from one taken from three.
  sleep "$WINDOW_S"
  alive=$(docker ps --format '{{.Names}}' 2>/dev/null | grep -c '^devnet-' || true)
  echo "  after ${WINDOW_S}s: $alive of 3 containers still up"

  for n in bootstrap v1 v2; do
    out="$OUT/shape-${n}-a${attempt}.txt"
    {
      echo "# tree=$(git rev-parse --short HEAD) image=$(docker inspect rnode:local --format '{{.Id}}' 2>/dev/null)"
      echo "# shape: --validators 3 --stakes 100,100,50 --epoch-length 10 --fresh, cap=$CAP, window=${WINDOW_S}s, threshold=${THRESHOLD_MB}MiB (disabled)"
      echo "# scraped $(date -u +%Y-%m-%dT%H:%M:%SZ) from :${PORTS[$n]}/metrics, attempt $attempt, node $n"
      curl -fsS --max-time 10 "http://localhost:${PORTS[$n]}/metrics" 2>/dev/null | grep -E '^rchain_merge' || echo "# scrape FAILED (node down?)"
    } > "$out"
    printf "  %s: " "$n"
    grep -c '^rchain_merge_scope_width_bucket' "$out" 2>/dev/null | sed 's/^/buckets=/'
    grep '^rchain_merge_scope_width_count' "$out" 2>/dev/null | head -1
  done

  # And the node's own log line, which carries the same numbers beside the envelope.
  for n in bootstrap v1 v2; do
    docker logs "${CONTAINERS[$n]}" 2>&1 | grep 'merge search' | tail -1 \
      > "$OUT/log-${n}-a${attempt}.txt" || true
  done
  tools/devnet.sh down >/dev/null 2>&1
  echo "  attempt $attempt done"
done

echo
echo "=== the distribution, attempt by attempt ==="
for f in "$OUT"/shape-*-a*.txt; do
  [[ -f "$f" ]] || continue
  echo "--- $f"
  grep -E '^rchain_merge_scope_width(_bucket|_count|_sum)|^rchain_merge_(searches|max_)' "$f" || true
done
