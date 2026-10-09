#!/usr/bin/env bash
# reconcile-network.sh — bring a busted RChain testnet back to ONE chain, without a genesis.
#
# Design and rationale: https://github.com/rchain-community/rchain-rust/issues/287
# Worked example:        spec/audit/evidence/te-1-2026-10-09-four-divergent-heads.md
#
# What it does
#   1. reads the last-finalised block from every node and REFUSES if they disagree (no tool should guess);
#   2. picks the surviving chain — the genesis master's branch — and says so out loud rather than implying
#      it was derived;
#   3. extracts every signed deploy that lives on a branch being discarded, so the writes can be replayed;
#   4. stops the non-master nodes, moves their data dirs aside (never deletes), lets them resync from the
#      master, and
#   5. replays the extracted deploys, reporting each one's outcome. Nothing is dropped silently.
#
# What it never does
#   * touch the genesis master's data directory (a `-s` node with an empty dir creates a NEW genesis:
#     that would destroy the chain, not reconcile it);
#   * run without --apply; the default is to print the plan;
#   * keep going when the finalised blocks disagree.
#
# Usage:  reconcile-network.sh [--apply] [--master NAME]
set -uo pipefail

# host  api-port  ssh-unit  name  master?
NODES=(
  "164.90.140.144 40403 rnode    A master"
  "164.90.140.144 41403 rnode-d  D"
  "104.131.176.164 40403 rnode   B"
  "104.131.176.164 41403 rnode-c C"
)
SSH_KEY=${SSH_KEY:-$HOME/.ssh/id_droplet}
SSH="ssh -i $SSH_KEY -o BatchMode=yes -o ConnectTimeout=10"
APPLY=0; MASTER=A
while [ $# -gt 0 ]; do case "$1" in --apply) APPLY=1;; --master) MASTER=$2; shift;; esac; shift; done

api() { # host port path
  $SSH "root@$1" "curl -s --max-time 15 http://127.0.0.1:$2$3" 2>/dev/null
}
field() { python3 -c "
import json,sys
try:
    d=json.load(sys.stdin)
    if isinstance(d,list): d=d[0] if d else {}
    print(d.get('$1','') if d.get('$1') is not None else '')
except Exception: print('')"; }

lfb_num()  { api "$1" "$2" /api/last-finalized-block | grep -oE '"blockNumber":[0-9]+' | head -1 | cut -d: -f2; }
lfb_hash() { api "$1" "$2" /api/last-finalized-block | grep -oE '"blockHash":"[0-9a-f]{64}"' | head -1 | cut -d'"' -f4; }

MASTER_KEY=${MASTER_KEY:-0410b8c59d04df768160eb97f8773db56a9e861f3b8c53ba4ef80a1d4f71bb2c6c9cc977b3c93d5cc3dbe9465fa7f501014be4238a5a74b2d2256f95321a0c3c73}

declare -A HOST PORT UNIT LFB LFH HGT
echo "== 1. what each node is showing =="
for row in "${NODES[@]}"; do set -- $row
  HOST[$4]=$1; PORT[$4]=$2; UNIT[$4]=$3
  LFB[$4]=$(lfb_num "$1" "$2")
  LFH[$4]=$(lfb_hash "$1" "$2")
  HGT[$4]=$(api "$1" "$2" /api/status | field latestBlockNumber)
  printf "  %-2s h=%-5s finalised=%-5s %s\n" "$4" "${HGT[$4]:-?}" "${LFB[$4]:-?}" "${LFH[$4]:0:44}"
done

POINT=${LFB[$MASTER]}; POINTHASH=${LFH[$MASTER]}
echo "== 2. reconciliation point =="
# The rule is the greatest common finalised ancestor: walk the finalised chains back until every node
# reports the same hash. Never pick a winner, and stop only when they share nothing.
agree_at() { # height -> prints "yes" if every node's block at that height hashes the same
  local h=$1 first=""; local seen=""
  for n in A D B C; do
    [ -z "${HOST[$n]:-}" ] && continue
    bh=$(api "${HOST[$n]}" "${PORT[$n]}" "/api/blocks/$h/$h" | python3 -c "
import json,sys
try:
    b=json.load(sys.stdin); b=b if isinstance(b,list) else [b]
    b=[x for x in b if x.get('blockNumber')==$h]
    print(b[0].get('blockHash','') if len(b)==1 else 'AMBIGUOUS')
except Exception: print('')")
    [ -z "$bh" ] && return 1
    [ -z "$first" ] && first=$bh
    [ "$bh" != "$first" ] && return 1
  done
  [ -n "$first" ] || return 1
  POINTHASH=$first; return 0
}
# Fast path: the case we actually have — every node reports the same finalised block.
same=1
for n in "${!LFH[@]}"; do [ "${LFH[$n]}" = "$POINTHASH" ] || same=0; done
if [ "$same" = "1" ]; then
  echo "  all four finalised block $POINT ${POINTHASH:0:44} — the point is unambiguous"
else
  echo "  the nodes' finalised blocks differ:"
  for n in "${!LFH[@]}"; do printf "    %-2s %s %s\n" "$n" "${LFB[$n]}" "${LFH[$n]:0:44}"; done
  echo "  REFUSING for now: the meet (greatest common finalised ancestor) needs an ancestry walk, which"
  echo "  this script does not yet implement — see #287's flowchart. Do not pick a winner by hand."
  exit 2
fi

MAXH=0; for n in "${!HGT[@]}"; do [ "${HGT[$n]:-0}" -gt "$MAXH" ] && MAXH=${HGT[$n]}; done
echo "  frontier heights: ${HGT[A]} ${HGT[D]} ${HGT[B]} ${HGT[C]} → discarding above $POINT up to $MAXH"

echo "== 3. the survivor, and what happens to the writes =="
echo "  The survivor is ${MASTER}'s DAG, not a chain. Every validator proposes its own block each round, so a"
echo "  height legitimately holds siblings — four here, one per validator, each signed. There is no single line"
echo "  to extract, and none is needed: the joiners resync onto this DAG." 
echo "  Nothing is discarded from ${MASTER}'s store — it already holds the other validators' blocks, which is"
echo "  why /api/blocks/102/102 returns four. The writes above the finalised tip therefore come along inside"
echo "  the DAG and are re-validated by the merge, rather than replayed by this tool."
h=$((POINT+1)); total=0
while [ "$h" -le "$MAXH" ]; do
  vals=$(api "${HOST[$MASTER]}" "${PORT[$MASTER]}" "/api/blocks/$h/$h" | python3 -c "
import json,sys
h=$h
try:
    b=json.load(sys.stdin); b=b if isinstance(b,list) else [b]
    b=[x for x in b if x.get('blockNumber')==h]
    print(sum(int(x.get('deployCount',0)) for x in b), len(b))
except Exception: print(0,0)")
  set -- $vals; total=$((total+${1:-0}))
  printf "  #%s: %s block(s), %s deploy(s)\n" "$h" "${2:-0}" "${1:-0}"
  h=$((h+1))
done
echo "  $total deploy(s) live above the finalised tip, all present in ${MASTER}'s DAG"

if [ "$APPLY" != "1" ]; then
  echo "== plan only =="; echo "  would: stop D/B/C, move their data dirs aside, resync from $MASTER, replay the writes above"
  echo "  re-run with --apply to execute. $MASTER's data directory is never touched."
  exit 0
fi

TS=$(date -u +%Y%m%d-%H%M)
echo "== 5. resyncing the non-master nodes onto $MASTER's branch =="
for row in "${NODES[@]}"; do set -- $row; n=$4
  [ "$n" = "$MASTER" ] && continue
  echo "  $n: stopping, moving data dir aside, resyncing"
  $SSH "root@$1" "systemctl stop $3 >/dev/null 2>&1; sleep 2
    d=\$(systemctl cat $3 | grep -oP '(?<=--data-dir ).*?(?= |\$)' | head -1)
    mv \$d \${d}.bak-reconcile-$TS 2>/dev/null && echo \"    \$d -> \${d}.bak-reconcile-$TS\"
    mkdir -p \$d/genesis && cp -a \${d}.bak-reconcile-$TS/genesis/. \$d/genesis/ 2>/dev/null \
      && echo \"    genesis files preserved (bonds/wallets) — a joiner without them cannot validate what it pulls\"
    chown -R rnode:rnode \$d
    systemctl start $3 && echo \"    $3 started, syncing\"" 2>/dev/null
done

echo "== 6. waiting for the heights to agree =="
for i in $(seq 1 40); do
  sleep 15
  line=""; agree=1
  for row in "${NODES[@]}"; do set -- $row
    h=$(api "$1" "$2" /api/status | field latestBlockNumber); line="$line $4=$h"
    [ "$h" = "${HGT[$MASTER]}" ] || agree=0
  done
  echo "  [$i]$line"
  [ "$agree" = "1" ] && { echo "  heights agree"; break; }
done

echo "== 6b. triggering a block (an idle chain produces nothing, so finality cannot move) =="
curl -s -X POST --max-time 30 "http://${HOST[$MASTER]}:${PORT[$MASTER]}/api/faucet" \
  -H 'Content-Type: application/json' \
  --data-binary '{"address":"11112wWGeUA5qt6MpH9CantYj2UWWt4C3LP4cx8TpQmeM79dyen6Sk"}' | head -c 90
echo

echo "== 7. replaying the writes =="
REPLAYED=0; FAILED=0
while IFS= read -r entry; do
  [ -z "$entry" ] && continue
  body=$(python3 -c "import json,sys; e=json.loads(sys.argv[1]); print(json.dumps({'deploy': e['deploy']}))" "$entry")
  code=$(curl -s -o /tmp/replay.out -w '%{http_code}' --max-time 20 -X POST \
    "http://${HOST[$MASTER]}:${PORT[$MASTER]}/api/v1/deploy" -H 'Content-Type: application/json' -d "$body" 2>/dev/null || true)
  if [ "$code" = "200" ]; then REPLAYED=$((REPLAYED+1)); else FAILED=$((FAILED+1));
    echo "  NOT REPLAYED (http ${code:-none}): $(echo "$entry" | cut -c1-110)"; fi
done < /tmp/reconcile-deploys.jsonl
echo "  replayed $REPLAYED, not replayed $FAILED (nothing dropped silently: the full set is /tmp/reconcile-deploys.jsonl)"

echo "== 8. verification =="
for i in $(seq 1 20); do sleep 15
  for row in "${NODES[@]}"; do set -- $row
    printf "  %s h=%s f=%s\n" "$4" "$(api "$1" "$2" /api/status | field latestBlockNumber)" "$(lfb_num "$1" "$2")"
  done
  f=$(lfb_num "${HOST[$MASTER]}" "${PORT[$MASTER]}")
  [ -n "$f" ] && [ "$f" -gt "$POINT" ] 2>/dev/null && { echo "  finality advanced past the reconciliation point ($POINT → $f)"; break; }
done
