#!/usr/bin/env bash
#
# **The A2 live arm: an equivocation between two live nodes (issue #150).**
#
# The unit arm (`casper/tests/slash_measures.rs`) drives the gate, the proof and the receiver's check
# over two real DAGs in one process; it says of itself that it does **not** exercise the wire. This does.
# Validator 1 is started with `--equivocation-injection`, so every block it creates is followed by an
# equally-signed twin at the same `(sender, seq_num)`; the bootstrap receives both, refuses the second,
# and is therefore the node that can *prove* the offence.
#
# What is measured:
#
#   1. the bootstrap's log records the equivocation (`equivocation detected`), which is its H-1 gate
#      refusing the twin and keeping the header;
#   2. a later block from the bootstrap carries a **`Slash`** for validator 1 — the fix working on a live
#      network, through gossip, on a block a *peer* produced;
#   3. validator 1's stake goes: the blocks after the slash do not carry its 100 (`slash` removes it from
#      the pool), while the bootstrap's own stays;
#   4. **both nodes accept the slashing block**, which is the property that keeps this from being a
#      split: each re-derives the offence from its *own* DAG rather than believing the proposer.
#
# Run from the repository root:  spec/audit/evidence/a2-live-equivocation-run.sh
#
# It tears the default devnet down first (`down -v`) and leaves the network up for inspection.

set -uo pipefail
cd "$(dirname "$0")/../../.."

PREFIX="${DEVNET_PREFIX:-devnet}"
BOOTSTRAP="${PREFIX}-bootstrap"
OFFENDER="${PREFIX}-validator-1"
OUT="spec/audit/evidence/a2-live-equivocation.log.txt"

say() { echo "[a2] $*"; }
cli() { docker exec "$1" rnode --grpc-host localhost "${@:2}" 2>&1; }
bonds_in_container() { docker exec "$BOOTSTRAP" cat /genesis/bonds.txt 2>/dev/null; }

say "tearing down any running devnet"
tools/devnet.sh down -v >/dev/null 2>&1 || true

say "starting 2 validators; validator 1 is told to equivocate"
export DEVNET_EXTRA_FLAGS_devnet_validator_1="--equivocation-injection"
tools/devnet.sh up --validators 2 --fresh 2>&1 | tail -20

OPERATOR_PK="$(bonds_in_container | sed -n '1p' | awk '{print $1}')"
OFFENDER_PK="$(bonds_in_container | sed -n '2p' | awk '{print $1}')"
say "bootstrap key [${OPERATOR_PK:0:16}…]  offender key [${OFFENDER_PK:0:16}…]"

say "letting the injection run for 45 s"
sleep 45

say "the injection, from the offender's own log"
docker logs "$OFFENDER" 2>&1 | grep -c "EQUIVOCATION INJECTION" | tee -a "$OUT"
docker logs "$OFFENDER" 2>&1 | grep "EQUIVOCATION INJECTION" | tail -2 | tee -a "$OUT"

say "the refusals, from the bootstrap's log (its H-1 gate)"
docker logs "$BOOTSTRAP" 2>&1 | grep -ci "equivocation" | tee -a "$OUT"
docker logs "$BOOTSTRAP" 2>&1 | grep -i "equivocation" | tail -3 | tee -a "$OUT"

say "the slash, where it is actually visible"
# **A wildcard this script fell into, and the reason the checks below read what they read.** The first
# version grepped the `show-blocks` dump for "slash" and reported `NO slash in 25 blocks` on exactly the
# run in which the bootstrap logged three slashes. `show-blocks` prints a block's **header and deploys**
# — its `bonds` map and its deploy count — and not the `state.systemDeploys` where a `Slash` lives, so
# that grep could only ever have found nothing. (The A1 driver had the same shape of bug, from a flag
# that did not exist; the lesson is the same one and is recorded in both.)
#
# The slash is read from three places instead, each of which *can* show it:
#   1. the proposer's own line, which names the tier and the evidence;
#   2. the **bonds map** on every later block — `slash` removes the offender from the pool, so the
#      transition from two entries to one is the confiscation, on chain;
#   3. the offender's key, by how many blocks still carry it.
cli "$BOOTSTRAP" show-blocks --depth 40 > "spec/audit/evidence/a2-live-blocks.log.txt" 2>&1
say "1. the proposer, from the bootstrap's log:"
docker logs "$BOOTSTRAP" 2>&1 | grep "\[pos\] slashing" | tee -a "$OUT"
docker logs "$BOOTSTRAP" 2>&1 | grep -c "\[pos\] slashing" | tee -a "$OUT"

BLOCKS="spec/audit/evidence/a2-live-blocks.log.txt"
blocks="$(grep -c '^------------- block' "$BLOCKS" || true)"
lines="$(wc -l < "$BLOCKS")"
say "2. the dump: $lines lines, $blocks blocks"
if [[ "$lines" -lt 100 ]]; then
  say "REFUSING to claim anything from a $lines-line dump — the command failed:"; head -3 "$BLOCKS"; exit 1
fi
say "   validators carried per block (2 = both bonded, 1 = the offender is out of the pool):"
awk '/^------------- block/ { if (n) print n; n = 0 } /"validator":/ { n++ } END { if (n) print n }' "$BLOCKS" \
  | sort | uniq -c | tee -a "$OUT"
say "   the first block carrying a single validator, i.e. where the confiscation lands:"
awk '/^------------- block/ { b = $3 } /"validator":/ { c++ } /^---/ && c { if (c == 1 && !seen) { print "block " b; seen = 1 } }' "$BLOCKS" | head -2 | tee -a "$OUT"
say "3. how many blocks still carry the offender's key:"
grep -c "$OFFENDER_PK" "$BLOCKS" | tee -a "$OUT"

say "both nodes' views at the end"
for n in "$BOOTSTRAP" "$OFFENDER"; do
  say "-- $n"
  cli "$n" status | grep -E '"(latestBlockNumber|peers)"' || true
  cli "$n" last-finalized-block | head -3 || true
done

say "done; the network is still up (tools/devnet.sh down -v to remove it)"
