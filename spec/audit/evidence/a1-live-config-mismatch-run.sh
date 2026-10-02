#!/usr/bin/env bash
#
# **The A1 live arm: two bonded validators whose `casper.min-phlo-price` disagree (issue #150).**
#
# The unit arm (`casper/tests/slash_measures.rs`) shows that a strictly-refused block is *not* an
# offence on the refusing node; it does not show what two live nodes do. This does: the bootstrap keeps
# the shipped floor of 1, validator 1 is given a floor of 5, and a deploy priced at 1 — the price
# `tools/devnet.sh deploy` sends — is therefore included by one and refused by the other.
#
# What is measured, and each is a claim the risk plan's A1 rests on:
#
#   1. **no `Slash` system deploy appears in any block**, which is the C198 claim stated where it can be
#      falsified (before C198 the strict node's refusal of that block was an offence, so it slashed the
#      sender of a block it could not evaluate);
#   2. **no validator's bond moves** — `bond-status` on both sides, before and after;
#   3. what *does* happen, stated plainly rather than spun — and it is worse than "the strict node
#      lags": the strict node refuses a block **in the chain's ancestry**, and every block after it
#      carries that deploy's low price on through the autopropose dummy deploy, so it refuses **every
#      subsequent block**. Its view freezes at the height it had (1 in the recorded run) while the
#      permissive node runs on to 37, and finality — which needs >2/3 of the stake, i.e. both — stops
#      for the whole network, which is why `last-finalized-block` answers "Finalized fringe is not
#      available" on both nodes.
#
#      So a mis-set floor is still a way to wedge a network, and C198 does not fix that: it removed the
#      half that *destroys stake*, and the measurement says so rather than implying the hazard is gone.
#
# Run from the repository root:  spec/audit/evidence/a1-live-config-mismatch-run.sh
#
# It tears the default devnet down first (`down -v`) and leaves the network up, with its logs, for
# inspection.

set -uo pipefail
cd "$(dirname "$0")/../../.."

PREFIX="${DEVNET_PREFIX:-devnet}"
BOOTSTRAP="${PREFIX}-bootstrap"
STRICT="${PREFIX}-validator-1"
OUT="spec/audit/evidence/a1-live-config-mismatch.log.txt"

say() { echo "[a1] $*"; }
cli() { docker exec "$1" rnode --grpc-host localhost "${@:2}" 2>&1; }
# What this arm reads off a node: its own view of the chain, and what it has finalised.
#
# **A wildcard to avoid, and how this script fell into it once.** The first version read the bonds with
# `bond-status`, which resolves against the **finalised fringe** — and this network has none, because
# finality is exactly what the mismatch stops. So every bond line read `Finalized fringe is not
# available`, and a `show-blocks -d 30` (a flag that does not exist) made the "no slash" grep run over an
# error message and report `none`. Both are the same defect: an *absence* claim produced by a command
# that failed. The bonds here come from the blocks themselves, and the slash check below reads a dump
# that must be non-empty before it is believed.
sample() {
  local node="$1" label="$2"
  say "-- $label status"
  cli "$node" status | grep -E '"(latestBlockNumber|finalizedBlockNumber|peers|nodes)"' || true
  say "-- $label last-finalized-block"
  cli "$node" last-finalized-block | head -5 || true
}

# The two validators' public keys come from the genesis the containers mount read-only.
bonds_in_container() { docker exec "$BOOTSTRAP" cat /genesis/bonds.txt 2>/dev/null; }

say "tearing down any running devnet"
tools/devnet.sh down -v >/dev/null 2>&1 || true

say "starting 2 validators; validator 1 gets --min-phlo-price 5, the bootstrap keeps 1"
export DEVNET_EXTRA_FLAGS_devnet_validator_1="--min-phlo-price 5"
tools/devnet.sh up --validators 2 --fresh 2>&1 | tail -20

VALIDATOR_0_PK="$(bonds_in_container | sed -n '1p' | awk '{print $1}')"
VALIDATOR_1_PK="$(bonds_in_container | sed -n '2p' | awk '{print $1}')"
say "validator keys: [${VALIDATOR_0_PK:0:16}…] [${VALIDATOR_1_PK:0:16}…]"
say "(the genesis the nodes mount:)"; bonds_in_container | tee -a "$OUT"

say "letting the chain produce for 30 s"
sleep 30
sample "$BOOTSTRAP" "bootstrap (floor 1)" | tee -a "$OUT"
sample "$STRICT" "validator-1 (floor 5)" | tee -a "$OUT"

say "deploying hello.rho (phlo-price 1) — accepted at the bootstrap's floor, below validator-1's"
tools/devnet.sh deploy hello.rho 2>&1 | tail -5 | tee -a "$OUT"

say "letting it be included and the mismatch play out for 60 s"
sleep 60
sample "$BOOTSTRAP" "bootstrap (floor 1), after" | tee -a "$OUT"
sample "$STRICT" "validator-1 (floor 5), after" | tee -a "$OUT"

say "the chain, as blocks (this dump is also where the bonds are read from)"
BLOCKS="spec/audit/evidence/a1-live-blocks.log.txt"
cli "$BOOTSTRAP" show-blocks --depth 40 > "$BLOCKS" 2>&1
lines="$(wc -l < "$BLOCKS")"
say "dump: $lines lines, $(grep -c '^------------- block' "$BLOCKS" || true) blocks"
if [[ "$lines" -lt 100 ]]; then
  say "REFUSING to claim an absence from a $lines-line dump — the command failed:"
  head -3 "$BLOCKS"; exit 1
fi
if grep -qi "slash" "$BLOCKS"; then
  say "FOUND a slash — see $BLOCKS"
else
  say "no Slash system deploy in any of those blocks ($(grep -ci slash "$BLOCKS") matches)"
fi
say "the stakes each block carries (genesis is 100 each, so an unchanged line is an unmoved bond):"
grep -A 2 '"validator"' "$BLOCKS" | grep -E '"stake"' | sort | uniq -c

say "the strict node's refusal, from its own log"
docker logs "$STRICT" 2>&1 | grep -iE "ContainsLowCostDeploy|phlo" | tail -10 | tee -a "$OUT"

say "done; the network is still up (tools/devnet.sh down -v to remove it)"
