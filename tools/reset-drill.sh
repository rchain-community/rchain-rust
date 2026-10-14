#!/usr/bin/env bash
# reset-drill — stage the self-reset's trigger on a devnet, and watch for the reset.
#
# Why a store edit and not a divergence of nature: `FailureCause::Divergence` is defined as *"this node's
# state or replay disagrees"* (`casper/src/block_status.rs:130-161`). A **record** edit cannot stage one —
# `restore_divergent_justifications` re-runs `validate_checks` and *clears* a merely-marked block, so the
# restore's own revalidation recomputes the truth (register C285). A **network partition** stages nothing
# either, measured: the merge is a pure function of the parent set (`casper/tests/merge_determinism.rs`),
# so a node with a different block set computes the same answer
# (`spec/audit/evidence/reset-arm/e1-partition.md`). What is left is an edit of the **state the merge
# computes its base from** — and this script makes it.
#
# The two fields, because the merge reads one or the other: while nothing has finalised the base is the
# **newest justification's** `post_state_hash` (`multi_parent_casper.rs:334-338`); once a fringe exists it
# is the greatest `(height, hash)` member's per-block `fringe_state_hash` claim (`:302`). The helper
# (`node/examples/tamper-fringe-claim.rs`) writes **both**, on the store's newest block.
#
# **Only the node under test is tampered, and the net is frozen first.** Tampering *every* node stalls the
# whole net instead of staging a divergence: each node's own proposer reads the wrong base and fails
# (`propose failed … active-bonds leaf not found in state`, the stall the merge's own comment names) — and
# with nobody producing, no peer block is ever validated, which is what the reset needs. So the tamper goes
# on `$SUBJECT` alone, at every block of its store's **greatest height** (the helper's job), and the other
# three keep producing blocks that justify the tampered tip.
#
# Freezing the net first is what keeps those blocks on the tips when the nodes restart.
#
# Usage: tools/reset-drill.sh [--observe-seconds N] [--warm-heights H]
# Exit: 0 the reset was observed · 3 the run staged nothing (the falsifier — see the gate below).
#
# **STATUS 2026-10-13: this does not stage the reset, and the runs say why.** The tamper is *effective* —
# applied to every node it stalls the chain with `propose failed … active-bonds leaf not found in state`,
# the merge's own documented consequence of a wrong base (`multi_parent_casper.rs:248-252`) — but the
# trigger needs the tampered block to be the greatest `(height, hash)` justification of a block the subject
# **validates**, and on a live net the target moves every block while a freeze-and-restart cannot pin it.
# The runs, their markers and the conclusion are in `spec/audit/evidence/reset-arm/e2-tamper.md`. The
# conclusion is that the trigger is not reachable *by an external instrument at all*, and that its
# narrowness is the feature: a trigger loose enough for an external edit would fire on a healthy node, and
# the reset is destructive. The script is kept because the mechanism is proven and a *perturbed build*
# (the route §109/#345 recorded) is the only remaining way to reach the drive loop's third exit.

set -euo pipefail
cd "$(dirname "$0")/.."

OBSERVE=240
WARM=40
while [[ $# -gt 0 ]]; do
  case "$1" in
    --observe-seconds) OBSERVE="${2:?}"; shift 2 ;;
    --warm-heights)    WARM="${2:?}";    shift 2 ;;
    *) echo "usage: tools/reset-drill.sh [--observe-seconds N] [--warm-heights H]" >&2; exit 2 ;;
  esac
done

OUT=target/reset-drill
mkdir -p "$OUT"
NODES=(devnet-bootstrap devnet-validator-1 devnet-validator-2 devnet-validator-3)
VOLS=(devnet-bootstrap-data devnet-validator-1-data devnet-validator-2-data devnet-validator-3-data)
# The node under test — the only one tampered (see above), and one of the three that can be armed.
SUBJECT=devnet-validator-1
SUBJECT_VOL=devnet-validator-1-data
# `--validators 4` is the bootstrap plus three (the loop is `i < n`), so the bootstrap is the fourth
# bonded validator — and it is launched `-s`/standalone, where `check_self_reset` *refuses* the flag at
# config load. So the switch goes on the three validators only.
PORTS=(40403 41403 42403 43403)

status() { curl -s --max-time 4 "http://localhost:$1/api/v1/status" 2>/dev/null; }
height() { status "$1" | python3 -c "import json,sys; print(json.load(sys.stdin).get('latestBlockNumber'))" 2>/dev/null || echo 0; }

echo "== 1. a fresh four-validator net, --self-reset on the validators =="
tools/devnet.sh down -v >/dev/null 2>&1 || true
DEVNET_EXTRA_FLAGS_devnet_validator_1="--self-reset" \
DEVNET_EXTRA_FLAGS_devnet_validator_2="--self-reset" \
DEVNET_EXTRA_FLAGS_devnet_validator_3="--self-reset" \
  tools/devnet.sh up --validators 4 --fresh > "$OUT/up.log" 2>&1 || true
# `up` gives up on its own wait when a joiner is slow to sync genesis, while the node itself often forms a
# minute later — so the drill checks the ports rather than trusting the exit code. Measured: a run whose
# `up` timed out on validator 2 still had all four healthy and level at height 65 a minute afterwards.
for _ in $(seq 1 24); do
  answer=$(status 40403 | head -c1)
  [[ -n "$answer" ]] && break
  sleep 5
done
for n in devnet-validator-1 devnet-validator-2 devnet-validator-3; do
  docker inspect "$n" --format '{{join .Config.Cmd " "}}' | grep -q -- "--self-reset" \
    || { echo "REFUSING: $n is not armed with --self-reset" >&2; exit 4; }
done

echo "== 2. warming to height $WARM =="
for _ in $(seq 1 60); do
  h=$(height 40403); echo "   height $h"
  [[ "$h" -ge "$WARM" ]] && break
  sleep 5
done

echo "== 3. freezing the net, and perturbing $SUBJECT's greatest-height block(s) only =="
docker stop "${NODES[@]}" >/dev/null
docker run --rm --entrypoint /drill/tamper-fringe-claim \
  -v "$SUBJECT_VOL":/data -v "$PWD/target/debug/examples":/drill:ro rnode:local /data
docker start "${NODES[@]}" >/dev/null

echo "== 4. observing for ${OBSERVE}s =="
: > "$OUT/observe.log"
for _ in $(seq 1 $((OBSERVE / 15))); do
  ts=$(date -u +%H:%M:%S)
  line="$ts"
  for i in "${!PORTS[@]}"; do
    s=$(status "${PORTS[$i]}")
    if [[ -n "$s" ]]; then
      line="$line | ${NODES[$i]}: $(printf '%s' "$s" | python3 -c "
import json,sys
d=json.load(sys.stdin)
print('h=%s unrest=%s sf=%s halt=%s' % (d.get('latestBlockNumber'), d.get('unrestorableRecords'),
      d.get('consecutiveSelfValidationFailures'), d.get('autoproposeTimerHalted')))" 2>/dev/null)"
    else
      line="$line | ${NODES[$i]}: (no answer)"
    fi
  done
  echo "$line" >> "$OUT/observe.log"
  tail -1 "$OUT/observe.log"
  sleep 15
done

echo "== 5. the chain, in the order it must appear =="
for n in "${NODES[@]}"; do
  docker logs "$n" > "$OUT/$n.log" 2>&1 || true
done
{
  for pat in "state-hash disagreement" "failed validation:" "cannot restore block" \
             "reset requested" "rewound [0-9]* block" "reset complete" "catch-up against"; do
    printf '%-28s' "$pat"
    for n in "${NODES[@]}"; do printf ' %s' "$(grep -cE "$pat" "$OUT/$n.log")"; done
    echo
  done
} | tee "$OUT/markers.txt"

# **The falsifier**, so the run cannot be read as a success by default: the staging precondition is that a
# divergence line appears. Absent it, nothing was staged and the run is void — exit 3, as E1 and the first
# E2 probe both had to.
staged=$(grep -cE 'state-hash disagreement|failed validation: (Invalid(State|PreState)Hash|InvalidRejectedDeploy|InvalidBondsCache|UnjustifiedSlash)' "$OUT"/*.log | awk -F: '{s+=$2} END {print s+0}')
if [[ "$staged" -eq 0 ]]; then
  echo "STAGED NOTHING: no divergence line on any node — the run is void (see $OUT/markers.txt)" >&2
  exit 3
fi
echo "staged: $staged divergence line(s); markers in $OUT/markers.txt"
