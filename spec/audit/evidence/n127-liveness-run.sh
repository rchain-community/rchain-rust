#!/usr/bin/env bash
# Does finality resume after a validator is killed? — the "after" arm of the 2026-09-30 campaign.
#
# The protocol is `n127-liveness-preregistration.md`, frozen before this ran: the same rig as the campaign
# (which found finality stopping in 3 of 3 attempts, at most +4 heights), the same kill at T+120, and one
# acceptance row — finality advances on the survivors by more than +4.
#
# The merge budget (C184) is read here as a **control**, not as a reading: this run's scopes are the honest
# ones, so `SearchBudgetExceeded` must NOT appear. If it does, the budget is set too low and its number is
# wrong.
#
# Usage:  ATTEMPTS=3 spec/audit/evidence/n127-liveness-run.sh
set -u
# The repo root is overridable so a probe can run against a `dev` checkout in a worktree without moving
# the primary tree. It has to be overridable because `tools/devnet.sh build` builds the image from the
# **current directory**, and the image under test must be the tree under test. Default unchanged, so the
# protocol `n127-liveness-preregistration.md` froze is what runs when nothing is set.
cd "${REPO:-/home/patrick/RNodeRust}"

ATTEMPTS=${ATTEMPTS:-3}
CAP=${CAP:-8g}
WINDOW_S=${WINDOW_S:-300}
DEPLOY_AT=${DEPLOY_AT:-30}
DEPLOYS=${DEPLOYS:-4}
KILL_AT=${KILL_AT:-120}
DEPLOY_TIMEOUT=${DEPLOY_TIMEOUT:-45}
# Off by default, so the protocol `n127-liveness-preregistration.md` froze is what runs unchanged. Set it
# to a T+ offset to deploy **again after the kill**, which is the discriminator between the two readings of
# a chain that goes quiet once a validator is stopped: "there is nothing left to finalise" (the proposer's
# documented idle contract, so blocks resume when there is) versus a second stop (they do not).
DEPLOY_AGAIN_AT=${DEPLOY_AGAIN_AT:-0}
# How many deploys the after-kill arm submits. Defaults to `$DEPLOYS`, so the frozen protocol — in which
# the after-kill arm is off entirely — is unchanged. A probe that wants **one** deploy after the kill sets
# this to 1 and `DEPLOYS=0` to send none before it, which is the idle-then-one-deploy shape of #148.
DEPLOYS_AGAIN=${DEPLOYS_AGAIN:-$DEPLOYS}

TREE=$(git rev-parse --short HEAD)
STAMP=$(date -u +%Y%m%dT%H%M%SZ)
OUT="target/n127-liveness/${TREE}-${STAMP}"
declare -A PORTS=([bootstrap]=40403 [v1]=41403 [v2]=42403)
declare -A CONTAINERS=([bootstrap]=devnet-bootstrap [v1]=devnet-validator-1 [v2]=devnet-validator-2)

mkdir -p "$OUT"
IMAGE=$(docker inspect rnode:local --format '{{.Id}}' 2>/dev/null || echo "none")
FLAGS="--validators 3 --stakes 100,100,50 --epoch-length 10 --fresh"

{
  echo "# tree=$TREE image=$IMAGE"
  echo "# shape: $FLAGS, cap=$CAP, window=${WINDOW_S}s, kill=validator-2 at T+${KILL_AT}s (KILL_AT>window = no kill), attempts=$ATTEMPTS"
  echo "# deploys: ${DEPLOYS} at T+${DEPLOY_AT}s; after the kill: ${DEPLOYS_AGAIN} at T+${DEPLOY_AGAIN_AT}s (0 = arm off)"
  echo "# started $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "# binary_sha256=$(docker run --rm --entrypoint sha256sum rnode:local /usr/local/bin/rnode 2>/dev/null | cut -d' ' -f1 || echo '?')"
  # **Is the binary under test this tree?** Two facts, both checked rather than asserted: the working tree
  # is HEAD (a dirty Rust tree means the image may be neither), and when the image was built — which a
  # reader compares against when this harness was started. The line this replaces named a fixed commit
  # (`80782e184`) and so answered a question nobody was asking after that commit stopped being the tip.
  echo "# image_created=$(docker inspect rnode:local --format '{{.Created}}' 2>/dev/null || echo 'none')"
  echo "# rust_diff_vs_HEAD=$([ -n "$(git diff --stat HEAD -- '*.rs' 2>/dev/null)" ] && echo 'NON-EMPTY — the image may be neither' || echo 'empty — the Rust tree is HEAD')"
} > "$OUT/manifest.txt"
cat "$OUT/manifest.txt"

for attempt in $(seq 1 "$ATTEMPTS"); do
  echo "=================== attempt $attempt of $ATTEMPTS  $(date -u +%H:%M:%S) UTC ==================="
  tools/devnet.sh down >/dev/null 2>&1
  DEVNET_NODE_MEMORY="$CAP" timeout 900 tools/devnet.sh up $FLAGS 2>&1 | tail -1

  up=$(docker ps --format '{{.Names}}' 2>/dev/null | grep -c '^devnet-' || true)
  if [[ "$up" -lt 3 ]]; then
    echo "  VOID: $up of 3 containers up"
    tools/devnet.sh down >/dev/null 2>&1
    continue
  fi
  echo "  3 of 3 up at $(date -u +%H:%M:%S)"

  python3 spec/audit/evidence/n117-queue-depth.py "$OUT/series-a${attempt}.tsv" \
    > "$OUT/series-a${attempt}.log" 2>&1 &
  sampler=$!

  t0=$(date +%s)
  wait_until() { local d=$(( $1 - $(date +%s) )); (( d > 0 )) && sleep "$d" || true; }

  wait_until $((t0 + DEPLOY_AT))
  submitted=0
  for _ in $(seq 1 "$DEPLOYS"); do
    timeout "$DEPLOY_TIMEOUT" tools/devnet.sh deploy examples/hello.rho >/dev/null 2>&1 \
      && submitted=$((submitted + 1))
  done
  echo "  deploys: $submitted of $DEPLOYS"

  wait_until $((t0 + KILL_AT))
  tools/devnet.sh stop 2 >/dev/null 2>&1
  echo "  killed validator-2 at T+$(( $(date +%s) - t0 ))s (target ${KILL_AT}s)"

  if (( DEPLOY_AGAIN_AT > 0 )); then
    wait_until $((t0 + DEPLOY_AGAIN_AT))
    again=0
    for _ in $(seq 1 "$DEPLOYS_AGAIN"); do
      timeout "$DEPLOY_TIMEOUT" tools/devnet.sh deploy examples/hello.rho >/dev/null 2>&1 \
        && again=$((again + 1))
    done
    echo "  deploys after the kill: $again of $DEPLOYS at T+$(( $(date +%s) - t0 ))s (target ${DEPLOY_AGAIN_AT}s)"
  fi

  wait "$sampler"

  # **The node's own reason**, read before the containers go: the stall line now fires on a change of
  # variant as well as per 100 heights (`interpreter_util.rs`), because the previous campaign measured the
  # pin in 3 of 3 attempts and could not say why — the instrument that explains it was gated out of range.
  # Each file carries its own provenance, not the directory's: a log grepped out of here and read elsewhere
  # otherwise cannot say which tree, node and attempt it came from (C176's class).
  for n in bootstrap v1 v2; do
    printf '# provenance: tree=%s node=%s attempt=%s — `docker logs` grepped at the end of the attempt (%s)\n' \
      "$TREE" "$n" "$attempt" "n127-liveness-run.sh" > "$OUT/stall-${n}-a${attempt}.txt"
    docker logs "${CONTAINERS[$n]}" 2>&1 | grep 'finality did not advance' \
      >> "$OUT/stall-${n}-a${attempt}.txt" || true
    printf '# provenance: tree=%s node=%s attempt=%s — the C184 control: `SearchBudgetExceeded` must be absent here (empty below = the budget refused nothing)\n' \
      "$TREE" "$n" "$attempt" > "$OUT/budget-${n}-a${attempt}.txt"
    docker logs "${CONTAINERS[$n]}" 2>&1 | grep 'exceeded its budget' \
      >> "$OUT/budget-${n}-a${attempt}.txt" || true
    # **Everything the node said at WARN or above.** The 2026-09-30 run found a chain that stops
    # proposing once a validator is killed and keeps stopping with four deploys in the pool — and the
    # proposer logs nothing on the path that suppresses it (`proposer.rs` logs only a self-created block
    # that fails validation), so the artifact that would explain the halt was never captured. It is here
    # now: this is the instrument, and the missing log line is the next unit's first change.
    printf '# provenance: tree=%s node=%s attempt=%s — `docker logs`, WARN and above, verbatim\n' \
      "$TREE" "$n" "$attempt" > "$OUT/logs-${n}-a${attempt}.txt"
    docker logs "${CONTAINERS[$n]}" 2>&1 | grep -E ' (WARN|ERROR) ' \
      >> "$OUT/logs-${n}-a${attempt}.txt" || true
  done
  echo "  stall lines: $(cat "$OUT"/stall-*-a${attempt}.txt 2>/dev/null | grep -vc '^#')"
  tools/devnet.sh down >/dev/null 2>&1
  echo "  attempt $attempt done at $(date -u +%H:%M:%S) UTC"
done

echo
echo "=== the pre-registered reading, computed by the committed program ==="
# One implementation of the reading, and it writes an artifact: the inline version this replaces printed
# the rows to a terminal, which is why the number in the tracker came from a person reading a gitignored
# `target/` directory rather than from a file anyone could check.
python3 spec/audit/evidence/n127-liveness-summarise.py "$OUT" | tee "$OUT/readings.txt"

echo
echo "=== the budget control: SearchBudgetExceeded must not appear on honest scopes ==="
# Read from the artifacts this run wrote, not from `docker logs` again — the containers are down by now, and
# a control computed a second way is a second implementation of one reading (`Split`, law 51b).
hits=0
for f in "$OUT"/budget-*-a*.txt; do
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
echo "artifacts: $OUT"
