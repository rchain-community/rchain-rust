#!/usr/bin/env bash
# Blocks per deploy as a function of the validator count — the #149 sweep.
#
# The protocol is `n149-preregistration.md`, frozen before this ran. It states the one thing that
# shapes the rig: the attestation guard is only consulted when the pool is empty, and the dev-mode
# dummy deploy fills it — so the primary arm is `--no-autopropose` and the devnet default is the
# control. Nothing here chooses that at run time; it is a fixed property of the two arms.
#
# Usage:  spec/audit/evidence/n149-sweep-run.sh
set -u

REPO=${REPO:-$(cd "$(dirname "$0")/../../.." && pwd)}
cd "$REPO"

ATTEMPTS=${ATTEMPTS:-3}
NS=${NS:-"2 3 5 8"}
CAP=${CAP:-8g}
SETTLE_S=${SETTLE_S:-60}
IDLE_S=${IDLE_S:-60}
READ_S=${READ_S:-180}
# The autopropose control (the campaign's configuration) is a bridge to the existing 12–16/min number,
# not a second sweep: N=3, one attempt.
CONTROL_N=${CONTROL_N:-3}
CONTROL_ATTEMPTS=${CONTROL_ATTEMPTS:-1}
JOIN_TIMEOUT=${JOIN_TIMEOUT:-180}
DEPLOY_TIMEOUT=${DEPLOY_TIMEOUT:-45}

TREE=$(git rev-parse --short HEAD)
STAMP=$(date -u +%Y%m%dT%H%M%SZ)
# `OUT_ROOT` exists so the sampler and reading can be exercised end to end on a short-window pre-flight
# without that run landing among the sweep's artifacts. It changes where the run is written and nothing
# else — the protocol is the windows and the flags, and neither reads this.
OUT="${OUT_ROOT:-target/n149-blocks}/${TREE}-${STAMP}"
mkdir -p "$OUT"

IMAGE=$(docker inspect rnode:local --format '{{.Id}}' 2>/dev/null || echo "none")
{
  echo "# tree=$TREE image=$IMAGE"
  echo "# sweep: N in {$NS}, primary arm=--no-autopropose, control arm=defaults at N=$CONTROL_N"
  echo "# windows: settle=${SETTLE_S}s idle=${IDLE_S}s reading=${READ_S}s, cap=$CAP, attempts=$ATTEMPTS"
  echo "# started $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "# image_created=$(docker inspect rnode:local --format '{{.Created}}' 2>/dev/null || echo 'none')"
  echo "# rust_diff_vs_HEAD=$([ -n "$(git diff --stat HEAD -- '*.rs' 2>/dev/null)" ] && echo 'NON-EMPTY — the image may be neither' || echo 'empty — the Rust tree is HEAD')"
} > "$OUT/manifest.txt"
cat "$OUT/manifest.txt"

# One run: a devnet at `n` validators, one sampler covering all three windows, one deploy.
run_one() {
  local n="$1" arm="$2" attempt="$3"
  local extra="" label="$arm"
  [[ "$arm" == "noauto" ]] && extra="--no-autopropose"
  local dir="$OUT/n${n}-${label}-a${attempt}"
  mkdir -p "$dir"

  echo "=================== N=$n $label a$attempt  $(date -u +%H:%M:%S) UTC ==================="
  tools/devnet.sh down >/dev/null 2>&1
  DEVNET_NODE_MEMORY="$CAP" timeout 900 tools/devnet.sh up --validators "$n" --fresh \
    --epoch-length 10 $extra 2>&1 | tail -1

  up=$(docker ps --format '{{.Names}}' 2>/dev/null | grep -c '^devnet-' || true)
  if [[ "$up" -lt "$n" ]]; then
    echo "  VOID: $up of $n containers up" | tee "$dir/void.txt"
    tools/devnet.sh down >/dev/null 2>&1
    return
  fi

  # Void condition (pre-registration): every node must have committed genesis, or the network did not
  # form and no reading below is meaningful. `latestBlockNumber` is `max_height + 1`, so genesis alone
  # reports 1 — this waits for *that*, not merely for the HTTP server (devnet.sh:290-301 says why).
  local deadline=$(( $(date +%s) + JOIN_TIMEOUT )) joined=0
  while (( $(date +%s) < deadline )); do
    joined=0
    for (( i = 0; i < n; i++ )); do
      local port=$(( 40403 + i * 1000 ))
      local bn
      bn=$(curl -fsS --max-time 3 "http://localhost:${port}/api/v1/status" 2>/dev/null \
           | sed -n 's/.*"latestBlockNumber":\([0-9]*\).*/\1/p')
      [[ -n "$bn" && "$bn" -gt 0 ]] && joined=$((joined + 1))
    done
    (( joined >= n )) && break
    sleep 2
  done
  if (( joined < n )); then
    echo "  VOID: $joined of $n nodes committed genesis within ${JOIN_TIMEOUT}s" | tee "$dir/void.txt"
    tools/devnet.sh down >/dev/null 2>&1
    return
  fi
  echo "  $n of $n up and past genesis at $(date -u +%H:%M:%S)"

  N149_N="$n" N149_WINDOW_S=$(( SETTLE_S + IDLE_S + READ_S )) \
    python3 spec/audit/evidence/n149-sample.py "$dir" > "$dir/sampler.log" 2>&1 &
  local sampler=$!

  t0=$(date +%s)
  wait_until() { local d=$(( $1 - $(date +%s) )); (( d > 0 )) && sleep "$d" || true; }

  # Window 2 (idle) opens here and closes at the deploy; nothing is done in it — that is the point.
  wait_until $(( t0 + SETTLE_S + IDLE_S ))
  local deploy_epoch
  deploy_epoch=$(date +%s)
  if timeout "$DEPLOY_TIMEOUT" tools/devnet.sh deploy examples/hello.rho >/dev/null 2>&1; then
    echo "  deployed at T+$(( deploy_epoch - t0 ))s (target $(( SETTLE_S + IDLE_S ))s)"
  else
    echo "  DEPLOY FAILED — the reading window has no driver" | tee "$dir/void.txt"
  fi
  printf 'arm\t%s\nn\t%s\nattempt\t%s\nt0\t%s\nsettle_end\t%s\nidle_end_and_deploy\t%s\nread_end\t%s\n' \
    "$arm" "$n" "$attempt" "$t0" "$(( t0 + SETTLE_S ))" "$deploy_epoch" \
    "$(( t0 + SETTLE_S + IDLE_S + READ_S ))" > "$dir/marks.tsv"

  wait "$sampler"

  # The C184 control, read from the artifacts this run wrote rather than from `docker logs` again: a
  # control computed a second way is a second implementation of one reading.
  for name in $(seq 0 $(( n - 1 ))); do
    local c="devnet-validator-${name}"
    [[ "$name" == "0" ]] && c="devnet-bootstrap"
    docker logs "$c" 2>&1 | grep 'exceeded its budget' >> "$dir/budget.txt" || true
    docker logs "$c" 2>&1 | grep -E ' (WARN|ERROR) ' >> "$dir/logs-${c}.txt" || true
  done
  tools/devnet.sh down >/dev/null 2>&1
  echo "  done at $(date -u +%H:%M:%S) UTC"
}

for n in $NS; do
  for attempt in $(seq 1 "$ATTEMPTS"); do
    run_one "$n" noauto "$attempt"
  done
done

for attempt in $(seq 1 "$CONTROL_ATTEMPTS"); do
  run_one "$CONTROL_N" auto "$attempt"
done

echo
echo "=== the pre-registered reading, computed by the committed program ==="
python3 spec/audit/evidence/n149-summarise.py "$OUT" | tee "$OUT/readings.txt"

echo
echo "=== the C184 control: SearchBudgetExceeded must not appear on honest scopes ==="
hits=0
for f in "$OUT"/n*/budget.txt; do
  [ -e "$f" ] || continue
  c=$(grep -vc '^#' "$f" || true)
  hits=$((hits + c))
done
if [[ "$hits" -eq 0 ]]; then
  echo "  ok: the budget refused nothing — these are the honest scopes it must not touch"
else
  echo "  *** $hits budget refusal(s): SearchBudget::NODE is set too low, and its number is wrong"
fi
echo
echo "void attempts: $(ls "$OUT"/n*/void.txt 2>/dev/null | wc -l)"
echo "artifacts: $OUT"
