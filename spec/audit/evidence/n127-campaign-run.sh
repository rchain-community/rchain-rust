#!/usr/bin/env bash
# Phase 0's campaign: the storm's rate, the merge shape distribution, and the stall's cause — off one rig.
#
# The pre-registered protocol is `n127-campaign-preregistration.md`: the configuration, the perturbations
# at their fixed offsets, the acceptance rows and the single number per reading were frozen before this
# ran, and an amendment is a new commit rather than an edit to that file.
#
# **Two things this harness does that the Stage 1 one did not**, both provenance (C176's class):
#   * the output directory is `target/n127-campaign/<tree>-<utc>/`, so a file from an earlier run cannot be
#     read as this run's — Stage 1's fixed filenames let a stale artifact nearly be taken for a new one;
#   * a scrape from a node that is not serving writes *that fact*, never a body.
# It also scrapes **before** the kill as well as at the end, because the distribution is a reading from all
# three nodes and the kill is a reading from the survivors.
#
# Usage:  ATTEMPTS=3 spec/audit/evidence/n127-campaign-run.sh
set -u
cd /home/patrick/RNodeRust

ATTEMPTS=${ATTEMPTS:-3}
CAP=${CAP:-8g}
WINDOW_S=${WINDOW_S:-300}
DEPLOY_AT=${DEPLOY_AT:-30}
DEPLOYS=${DEPLOYS:-4}
KILL_AT=${KILL_AT:-120}
DEPLOY_TIMEOUT=${DEPLOY_TIMEOUT:-45}

TREE=$(git rev-parse --short HEAD)
STAMP=$(date -u +%Y%m%dT%H%M%SZ)
OUT="target/n127-campaign/${TREE}-${STAMP}"
# Associative, and it must say so: without `declare -A` a `[bootstrap]=…` literal is an *indexed*
# array whose subscript is an arithmetic expression, so it fails as an unbound variable under `set -u`.
declare -A PORTS=([bootstrap]=40403 [v1]=41403 [v2]=42403)
declare -A CONTAINERS=([bootstrap]=devnet-bootstrap [v1]=devnet-validator-1 [v2]=devnet-validator-2)

mkdir -p "$OUT"
IMAGE=$(docker inspect rnode:local --format '{{.Id}}' 2>/dev/null || echo "none")
FLAGS="--validators 3 --stakes 100,100,50 --epoch-length 10 --fresh"

{
  echo "# tree=$TREE image=$IMAGE"
  echo "# shape: $FLAGS (devnet defaults: autopropose on, propose-on-deploy on, attest-on-new-blocks on)"
  echo "# cap=$CAP window=${WINDOW_S}s attempts=$ATTEMPTS deploys=$DEPLOYS at T+${DEPLOY_AT}s kill=validator-2 at T+${KILL_AT}s"
  echo "# started $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "# binary_sha256=$([ -n "$IMAGE" ] && docker run --rm --entrypoint sha256sum rnode:local /usr/local/bin/rnode 2>/dev/null | cut -d' ' -f1 || echo '?')"
  # The image is built from the working tree, so its binary is only this tree's if no Rust source moved
  # since. Evidence, not a claim: the diff over the sources that go into the binary.
  echo "# image_built_from=$(docker inspect rnode:local --format '{{.Created}}' 2>/dev/null || echo '?')"
  echo "# rust_diff_vs_1732306c7=$([ -n "$(git diff --stat 1732306c7 -- '*.rs' 'Cargo.toml' 'Cargo.lock' '*/Cargo.toml' 2>/dev/null)" ] && echo 'NON-EMPTY — the image does not carry this tree' || echo 'empty — the binary is this tree')"
} > "$OUT/manifest.txt"
cat "$OUT/manifest.txt"

scrape() {  # scrape <node> <attempt> <phase>
  local n=$1 a=$2 phase=$3
  local f="$OUT/metrics-${n}-a${a}-${phase}.txt"
  local now; now=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  if curl -fsS --max-time 10 "http://localhost:${PORTS[$n]}/metrics" -o "$f.body" 2>/dev/null; then
    {
      echo "# tree=$TREE image=$IMAGE node=$n attempt=$a phase=$phase scraped=$now"
      echo "# shape: $FLAGS, cap=$CAP"
      grep -E '^rchain_' "$f.body"
    } > "$f"
  else
    echo "# tree=$TREE node=$n attempt=$a phase=$phase scraped=$now: SCRAPE FAILED — the node is not serving. This file records the failure, not a stale scrape." > "$f"
  fi
  rm -f "$f.body"
  printf '  %-9s %-8s ' "$n" "$phase"
  grep -c '^rchain_merge' "$f" 2>/dev/null | sed 's/^/merge_lines=/'
}

for attempt in $(seq 1 "$ATTEMPTS"); do
  echo "=================== attempt $attempt of $ATTEMPTS  $(date -u +%H:%M:%S) UTC ==================="
  tools/devnet.sh down >/dev/null 2>&1
  DEVNET_NODE_MEMORY="$CAP" timeout 900 \
    tools/devnet.sh up $FLAGS 2>&1 | tail -1

  up=$(docker ps --format '{{.Names}}' 2>/dev/null | grep -c '^devnet-' || true)
  if [[ "$up" -lt 3 ]]; then
    echo "  VOID: $up of 3 containers up"
    tools/devnet.sh down >/dev/null 2>&1
    continue
  fi
  echo "  3 of 3 up at $(date -u +%H:%M:%S)"

  # 0.1's series: height, finality, depth and anon once a second, per node (the #117 sampler — it reads
  # the cgroup from the container id and the peak while the node lives, and writes its own config header).
  python3 spec/audit/evidence/n117-queue-depth.py "$OUT/series-a${attempt}.tsv" \
    > "$OUT/series-a${attempt}.log" 2>&1 &
  sampler=$!

  # The perturbations are at absolute offsets from the attempt's zero, not a chain of sleeps: a deploy
  # that hits its bound would otherwise push the kill later, and the kill's offset is what makes this
  # attempt comparable with the next.
  t0=$(date +%s)
  wait_until() { local d=$(( $1 - $(date +%s) )); (( d > 0 )) && sleep "$d" || true; }
  wait_until $((t0 + DEPLOY_AT))
  # **Every deploy is bounded.** The RPC does not return while the node cannot include it, and an
  # unbounded wait is not a measurement: the first run of this harness sat here for 13 minutes with the
  # chain still growing, which is a fact about the node and a defect in the harness. The count of deploys
  # that returned *within the bound* is recorded, because a deploy that does not land is itself a reading
  # — but it is **not** a pre-registered one, so the results file must say so rather than quietly report
  # the chain's growth as if the four deploys had happened as frozen.
  submitted=0
  timed_out=0
  for _ in $(seq 1 "$DEPLOYS"); do
    if timeout "$DEPLOY_TIMEOUT" tools/devnet.sh deploy examples/hello.rho >/dev/null 2>&1; then
      submitted=$((submitted + 1))
    else
      timed_out=$((timed_out + 1))
    fi
  done
  echo "  deploys: $submitted of $DEPLOYS returned within ${DEPLOY_TIMEOUT}s, $timed_out did not (now T+$(( $(date +%s) - t0 ))s)"
  echo "$submitted	$timed_out	$DEPLOY_TIMEOUT" > "$OUT/deploys-a${attempt}.txt"

  wait_until $((t0 + KILL_AT))
  scrape bootstrap "$attempt" prekill
  scrape v1        "$attempt" prekill
  scrape v2        "$attempt" prekill
  # `stop` resolves a validator *number* (or a container name), not the `v2` label the ports dictionary
  # uses — `stop v2` would try `docker stop v2` and fail.
  tools/devnet.sh stop 2 >/dev/null 2>&1
  echo "  killed validator-2 at T+$(( $(date +%s) - t0 ))s (target ${KILL_AT}s)"

  wait "$sampler"

  scrape bootstrap "$attempt" end
  scrape v1        "$attempt" end
  scrape v2        "$attempt" end

  # The envelope the nodes logged, beside the histogram they published — the two must agree.
  for n in bootstrap v1 v2; do
    docker logs "${CONTAINERS[$n]}" 2>&1 | grep 'merge search' | tail -1 \
      > "$OUT/mergelog-${n}-a${attempt}.txt" || true
    # 0.3: the stall line, verbatim, with the tip it fired at.
    docker logs "${CONTAINERS[$n]}" 2>&1 | grep 'finality did not advance' \
      > "$OUT/stall-${n}-a${attempt}.txt" || true
  done
  echo "  stall lines: $(cat "$OUT"/stall-*-a${attempt}.txt 2>/dev/null | wc -l)"

  tools/devnet.sh down >/dev/null 2>&1
  echo "  attempt $attempt done at $(date -u +%H:%M:%S) UTC"
done

echo
echo "=== the distributions, attempt by attempt ==="
for f in "$OUT"/metrics-*-prekill.txt; do
  [[ -f "$f" ]] || continue
  echo "--- $(basename "$f")"
  grep -E '^rchain_merge_(scope_width|states_expanded)(_bucket|_count|_sum)' "$f" \
    | grep -vE 'le="(0\.0|1\.0|2\.5|5\.0|7\.5|10\.0|0\.005|0\.01|0\.025|0\.05|0\.075|0\.1|0\.25|0\.5|0\.75)"' || true
  grep -E '^rchain_merge_(max_|searches)' "$f" | head -6 || true
done
echo
echo "=== the stalls ==="
for f in "$OUT"/stall-*.txt; do
  [[ -s "$f" ]] || continue
  echo "--- $(basename "$f")"; tail -3 "$f"
done
echo
echo "artifacts: $OUT"
