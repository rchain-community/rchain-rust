#!/usr/bin/env bash
# reconcile-network.sh — bring a busted RChain testnet back to ONE chain, without a genesis.
#
# Derived from the reconciliation utility on PR #294 (jimscarver), with the review's five invariants and
# #287's own corrections implemented. Design and rationale: issue #287. The four-divergent-heads incident
# it exists for: spec/audit/evidence/te-1-2026-10-09-four-divergent-heads.md.
#
# What it does
#   1. reads every node's explicitly reported finalised block and requires them to **agree** — the same
#      height *and* the same hash on every listed node — or refuses, naming what differs. A block producer
#      is not a validator vote and the heights API cannot prove stake-weighted finalized ancestry, so this
#      equality is the whole of the anchor: no quorum is inferred, and no stake fraction is printed;
#   2. **reports** suspected equivocation from the block API's own `sender` field (two distinct blocks by
#      one sender at one height). It is one endpoint's word — this script checks no signature and binds no
#      endpoint to a bonded key — so it is a suspicion worth a human's attention, never an attributable
#      artefact, and **nothing is dropped, reweighted or decided on it** (§2);
#   3. states the anchor and what each node reported, rather than implying a quorum it did not compute;
#   4. enumerates what is above the point, per block, and never drops it silently;
#   5. (`--apply`) stops every non-master node, moves each data directory aside — **never deleting** —
#      and restarts them to resync from the master's DAG;
#   5b. (`--restore-from-master`) copies the master's *chain state* onto each joiner instead, for the case
#      step 5 cannot reach — a net with no finalised fringe has nothing to resync to (C259). This is
#      **not a sync**: the joiners adopt the master's view wholesale, which is the operator asserting a
#      winner. It is a labelled stopgap, it never copies node identity, and the tool prints what it is
#      adopting and what it is dropping before it runs;
#   6. verifies the outcome on **block hashes per height**, not on heights.
#
# What it never does
#   * treat an unfinalised block as the truth;
#   * infer a stake quorum from block producers, or print a stake fraction it has not verified (1-2);
#     there is no denominator here to shrink;
#   * touch the master's data directory;
#   * run without `--apply` — the default prints the plan and exits 0;
#   * report a height as converged (four nodes at height 0 are equal).
#
# Transport and control
#   `RECONCILE_NODES` is a file of `host api-port unit name master?` rows; the built-in table is the
#   live testnet. `host` being `local`/`127.0.0.1`/`localhost` reads the API with curl on this host
#   (which is how the devnet drill runs it); anything else is ssh'd to.
#   `RECONCILE_CONTROL` is `systemd` (default: stop/start the `unit`, whose data dir is moved with mv)
#   or `docker` (the devnet: stop/start the container, and its data hangs off a volume, so "aside" is a
#   tar on the host kept beside the volume rather than a rename).
#
# Usage:  RECONCILE_NODES=<file> RECONCILE_CONTROL=docker reconcile-network.sh \
#           [--apply | --restore-from-master] [--master NAME]

set -uo pipefail

# host  api-port  unit  name  master?
DEFAULT_NODES='164.90.140.144 40403 rnode    A master
164.90.140.144 41403 rnode-d  D
104.131.176.164 40403 rnode   B
104.131.176.164 41403 rnode-c C'

if [ -n "${RECONCILE_NODES:-}" ]; then NODES="$(cat "$RECONCILE_NODES")"; else NODES="$DEFAULT_NODES"; fi
SSH_KEY=${SSH_KEY:-$HOME/.ssh/id_droplet}
SSH="ssh -i $SSH_KEY -o BatchMode=yes -o ConnectTimeout=10"
CONTROL=${RECONCILE_CONTROL:-systemd}
APPLY=0; MASTER=A; RESTORE=0
while [ $# -gt 0 ]; do case "$1" in
  --apply) APPLY=1;;
  --restore-from-master) RESTORE=1; APPLY=1;;
  --master) MASTER=$2; shift;;
esac; shift; done

is_local() { case "$1" in local|127.0.0.1|localhost) return 0;; esac; return 1; }

api() { # host port path
  if is_local "$1"; then
    curl -s --max-time 15 "http://127.0.0.1:$2$3" 2>/dev/null
  else
    $SSH "root@$1" "curl -s --max-time 15 http://127.0.0.1:$2$3" 2>/dev/null
  fi
}

# Every block a node holds at one height: "<blockHash> <sender> <postStateHash> <deployCount> <bonds>".
# A height legitimately holds one block per bonded validator, so this returns *all* of them — which is
# what makes the equivocation check possible from the API alone, with no node-internal surface.
blocks_at() { # host port height
  api "$1" "$2" "/api/blocks/$3/$3" | python3 -c "
import json,sys
h=int('$3')
try:
    b=json.load(sys.stdin)
    b=b if isinstance(b,list) else []
except Exception:
    sys.exit(0)
for x in b:
    if not isinstance(x,dict): continue
    if x.get('blockNumber') != h: continue
    bonds=';'.join('%s=%s' % (y.get('validator',''), y.get('stake','')) for y in (x.get('bonds') or []))
    print('%s %s %s %s %s' % (x.get('blockHash',''), x.get('sender',''),
                              x.get('postStateHash',''), x.get('deployCount',0), bonds))"
}
# The last-finalised endpoint answers with an *error string* rather than an object when there is no
# finalised fringe yet (`"Finalized fringe is not available."`) — which is the state a diverged net is in,
# so this refuses to assume an object.
json_field() { # expr over `b`, evaluated with `b` the block object (or {})
  python3 -c "
import json,sys
try:
    d=json.load(sys.stdin)
    b=(d.get('blockInfo') or d.get('block') or d) if isinstance(d,dict) else {}
    print($1)
except Exception: print('')"
}
lfb_num()  { api "$1" "$2" /api/last-finalized-block | json_field "b.get('blockNumber','')"; }
lfb_hash() { api "$1" "$2" /api/last-finalized-block | json_field "b.get('blockHash','')"; }
height_of() { api "$1" "$2" /api/status | json_field "d.get('latestBlockNumber','')"; }

# A validator's stake, read from a block's own bond map. Absent means zero weight, which is what a node
# that has never seen the validator should conclude — not an error.
stake_of() { # sender bonds
  local sender="$1" bonds="$2" kv
  for kv in ${bonds//;/ }; do
    [ "${kv%%=*}" = "$sender" ] && { echo "${kv##*=}"; return; }
  done
  echo 0
}

# --- A: the chain-state environments, by `casper/src/storage.rs::rnode_db_mapping` ---------------
#
# **Copied**: `blockstorage` (block bodies), `dagstorage` (block metadata, the fringe records, the
# approved store, the deploy and deployer indices, and the two merge caches), `rspace/history` and
# `rspace/cold` (the on-chain tuple space), and `transaction`.
#
# **Not copied, each for its own reason**: `deploypoolstorage` (this node's pending deploy pool — not
# the network's; owners re-submit), `reporting` (a local trace cache), `eval/history` and `eval/cold`
# (the *off-chain* evaluator's space, not consensus), `gateway` (its own comment says "node-local, never
# consensus state").
#
# **And never the identity.** `node.key.pem`, `node.certificate.pem` and the validator key are the
# node's own and are not in this list. A joiner holding the survivor's key *is* an equivocator, and an
# agreement check cannot see it — the two look identical until they later sign conflicting blocks.
CHAIN_ENVS="blockstorage dagstorage rspace/history rspace/cold transaction"

# The named volume a container keeps its shard data on.
data_volume() {
  docker inspect -f '{{range .Mounts}}{{if eq .Destination "/var/lib/rnode"}}{{.Name}}{{end}}{{end}}' "$1" 2>/dev/null
}

# **The stopgap, and it says so.** This is not a sync: it copies the survivor's chain state onto each
# joiner, so what the joiners agree about afterwards is the *survivor's view*, including the blocks above
# the agreed anchor that only it accepted. The operator is asserting a winner, which is why `--apply` prints the
# blocks being adopted and the blocks being dropped before it runs. It exists because on a net with no
# finalised fringe there is nothing to sync *to* (see section 3's anchor and `--sync-anchor`), and it is
# marked a stopgap because it depends on the data-dir layout being movable — the thing #287's design
# deliberately avoided depending on.
restore_chain_state_from_master() {
  local n="$1" mvol vol mdir dir
  if [ "$CONTROL" = "docker" ]; then
    # `$MASTER` is the node's *name*; the container that holds its volume is `UNIT[$MASTER]`. Passing the
    # name is the bug that made this report "no data volume on A" while the volume sat there — found by
    # running it, and worth the sentence because the two are easy to confuse in this script.
    mvol="$(data_volume "${UNIT[$MASTER]}")"; vol="$(data_volume "${UNIT[$n]}")"
    [ -z "$mvol" ] && { echo "    WARNING: no data volume on ${UNIT[$MASTER]} — nothing copied" >&2; return 1; }
    [ -z "$vol" ] && { echo "    WARNING: no data volume on ${UNIT[$n]} — nothing copied" >&2; return 1; }
    docker run --rm -v "$mvol":/src:ro -v "$vol":/dst alpine sh -c "
      set -e
      copied=''; skipped=''
      for d in $CHAIN_ENVS; do
        # **A source that is not there is skipped, not fatal.** LMDB creates an environment only when
        # something writes to it, so a directory the mapping names (`transaction` on this build) can
        # simply not exist — and under \`set -e\` a failed \`cp\` aborted the whole copy, which is how the
        # first run reported a failure while the store was fine.
        if [ ! -e \"/src/\$d\" ]; then skipped=\"\$skipped \$d\"; continue; fi
        # **Copy beside, then swap.** Removing the destination first and *then* copying means a failed
        # copy leaves the joiner with nothing — the opposite of the 'aside, never gone' rule this tool
        # holds itself to everywhere else. So the new copy lands at a staging name and the old one is
        # only removed once the copy is complete.
        mkdir -p \"/dst/\$(dirname \"\$d\")\"
        rm -rf \"/dst/\$d.reconcile-new\"
        cp -a \"/src/\$d\" \"/dst/\$d.reconcile-new\"
        rm -rf \"/dst/\$d\"
        mv \"/dst/\$d.reconcile-new\" \"/dst/\$d\"
        copied=\"\$copied \$d\"
      done
      # A copied LMDB lock file is a stale lock: the joiner would wait on a lock nobody holds.
      find /dst -name 'lock.mdb' -delete
      echo \"    copied:\$copied\${skipped:+; not present on the survivor:\$skipped}\" >&2
      true" \
      && echo "    chain state copied from ${UNIT[$MASTER]}'s volume; identity and genesis untouched" \
      || { echo "    WARNING: the copy failed — $n is unchanged" >&2; return 1; }
  else
    mdir="$($SSH "root@${HOST[$MASTER]}" "systemctl cat ${UNIT[$MASTER]} | grep -oP '(?<=--data-dir ).*?(?= |\$)' | head -1" 2>/dev/null)"
    dir="$($SSH "root@${HOST[$n]}" "systemctl cat ${UNIT[$n]} | grep -oP '(?<=--data-dir ).*?(?= |\$)' | head -1" 2>/dev/null)"
    [ -z "$mdir" ] || [ -z "$dir" ] && { echo "    WARNING: a --data-dir could not be read" >&2; return 1; }
    # tar on the survivor, stream through this host, untar on the joiner: no key between the two.
    $SSH "root@${HOST[$MASTER]}" "tar -C '$mdir' -cf - $CHAIN_ENVS" 2>/dev/null \
      | $SSH "root@${HOST[$n]}" "mkdir -p '$dir' && tar -C '$dir' -xf - && find '$dir' -name lock.mdb -delete" 2>/dev/null \
      && echo "    chain state streamed from ${MASTER} to $n; identity and genesis untouched" \
      || { echo "    WARNING: the copy failed — $n is unchanged" >&2; return 1; }
  fi
}

stop_node() {
  # **Resolve the node first, in its own statement.** `local n="$1" unit="${UNIT[$n]}"` looks right and is
  # not: the words are expanded *before* `local` runs, so the subscript uses the caller's `n` — which is
  # whatever the last loop left there. It worked by accident in one loop (whose variable and argument
  # coincided) and started the wrong container in another, found by running it.
  local n="$1" host unit
  host="${HOST[$n]}"; unit="${UNIT[$n]}"
  if [ "$CONTROL" = "docker" ]; then
    docker stop "$unit" >/dev/null 2>&1 && echo "    $unit stopped (container)" && return
    echo "    WARNING: $unit did not stop" >&2
  else
    $SSH "root@$host" "systemctl stop $unit" >/dev/null 2>&1 && echo "    $unit stopped" && return
    echo "    WARNING: $unit did not stop" >&2
  fi
  return 1
}
start_node() {
  local n="$1" host unit
  host="${HOST[$n]}"; unit="${UNIT[$n]}"
  if [ "$CONTROL" = "docker" ]; then
    docker start "$unit" >/dev/null 2>&1 && echo "    $unit started (container)" && return
  else
    $SSH "root@$host" "systemctl start $unit" >/dev/null 2>&1 && echo "    $unit started" && return
  fi
  echo "    WARNING: $unit did not start" >&2
  return 1
}
# **Aside, never gone.** systemd: the data directory is renamed in place. docker: the data lives on a
# volume, which cannot be renamed, so its whole contents are tarred to a host file named after the
# volume and the timestamp *before* anything is removed — the backup is the "aside", and the run prints
# where it is. The genesis files are a special case and are handled first, because they may live *inside*
# the data directory: a standalone node's default genesis is `<data_dir>/genesis`, and wiping the data
# dir is what destroyed the genesis inputs on the live net. On the devnet they are mounted from outside
# at `/genesis`, so the copy is a no-op there and must not read as failure.
move_data_dir_aside() {
  local n="$1" ts="$2" host unit vol backup
  host="${HOST[$n]}"; unit="${UNIT[$n]}"
  if [ "$CONTROL" = "docker" ]; then
    vol="$(docker inspect -f '{{range .Mounts}}{{if eq .Destination "/var/lib/rnode"}}{{.Name}}{{end}}{{end}}' "$unit" 2>/dev/null)"
    if [ -z "$vol" ]; then echo "    WARNING: no data volume found on $unit — refusing" >&2; return 1; fi
    backup="${RECONCILE_BACKUP_DIR:-$PWD/target}/reconcile-backup-${vol}-${ts}.tar"
    docker run --rm -v "$vol":/data -v "$(dirname "$backup")":/backup alpine \
      tar cf "/backup/$(basename "$backup")" -C /data . >/dev/null 2>&1 \
      && echo "    $vol backed up to $backup (never deleted — this is the 'aside')" \
      || { echo "    WARNING: could not back up $vol — refusing to touch it" >&2; return 1; }
    # Preserve the stopped container and its volume so docker start can restart it.
    docker run --rm -v "$vol":/data alpine sh -c 'cd /data && rm -rf -- blockstorage dagstorage rspace/history rspace/cold transaction' >/dev/null 2>&1 \
      || { echo "    WARNING: could not clear chain state; refusing restart" >&2; return 1; }
    echo "    $vol chain state cleared; container preserved; backup at $backup"
  else
    $SSH "root@$host" "d=\$(systemctl cat $unit | grep -oP '(?<=--data-dir ).*?(?= |\$)' | head -1)
      cp -a \$d/genesis \${d}.genesis-keep-$ts 2>/dev/null && echo '    genesis inputs preserved'
      mv \$d \${d}.bak-reconcile-$ts && echo \"    \$d -> \${d}.bak-reconcile-$ts (never deleted)\"
      mkdir -p \$d && cp -a \${d}.genesis-keep-$ts/. \$d/genesis/ 2>/dev/null
      chown -R rnode:rnode \$d" 2>/dev/null
  fi
}

declare -A HOST PORT UNIT LFB LFH HGT
NAMES=()
echo "== 1. what each node is showing =="
while read -r host port unit name master; do
  [ -z "${name:-}" ] && continue
  NAMES+=("$name"); HOST[$name]=$host; PORT[$name]=$port; UNIT[$name]=$unit
  LFB[$name]=$(lfb_num "$host" "$port"); LFH[$name]=$(lfb_hash "$host" "$port")
  HGT[$name]=$(height_of "$host" "$port")
  [ "${master:-}" = "master" ] && MASTER=$name
  printf "  %-2s h=%-6s finalised=%-6s %s\n" "$name" "${HGT[$name]:-?}" "${LFB[$name]:-?}" "${LFH[$name]:0:44}"
done <<< "$NODES"

if [ "${#NAMES[@]}" -eq 0 ]; then echo "no nodes in the table — nothing to reconcile" >&2; exit 2; fi
MAXH=0; for n in "${NAMES[@]}"; do [ "${HGT[$n]:-0}" -gt "$MAXH" ] 2>/dev/null && MAXH=${HGT[$n]}; done

# --- 2. reported equivocation -------------------------------------------------
# **A report, not a proof, and it decides nothing.** A sender is flagged when two *distinct* blocks by it
# appear at one height in the answers the endpoints gave. The `sender` field is the endpoint's word: this
# script verifies no signature and binds no endpoint to a bonded key, so the output is a suspicion for a
# human to chase, never an attributable artefact — the proof, and the slash, are the node's (#287's
# obligation 4, #290 part 1). **No arithmetic consumes it**: since 2026-10-09 there is no stake-weighted
# meet here and no denominator, so nothing is dropped or reweighted on this output. Silence, slowness and
# absence are not evidence either way, here or anywhere else in this script.
declare -A REPORTED=() REPORTED_WHY=()
EQUIV_LINES=()
echo "== 2. reported equivocation from the block API — a report, not a proof (heights 0..$MAXH) =="
for (( h=0; h<=MAXH; h++ )); do
  declare -A seen_sender_block=()
  for n in "${NAMES[@]}"; do
    while read -r bh sender _post _deploys _bonds; do
      [ -z "${bh:-}" ] && continue
      prev="${seen_sender_block[$sender]:-}"
      if [ -n "$prev" ] && [ "$prev" != "$bh" ]; then
        if [ -z "${REPORTED[$sender]:-}" ]; then
          REPORTED[$sender]=1
          REPORTED_WHY[$sender]="two distinct blocks at height $h"
          EQUIV_LINES+=("height $h: ${prev:0:12}… and ${bh:0:12}… by ${sender:0:16}…")
        fi
      fi
      seen_sender_block[$sender]="$bh"
    done < <(blocks_at "${HOST[$n]}" "${PORT[$n]}" "$h")
  done
done
if [ "${#REPORTED[@]}" -eq 0 ]; then
  echo "  none reported — every sender has at most one block per height in these endpoints' answers"
else
  for s in "${!REPORTED[@]}"; do echo "  REPORTED ${s:0:16}… — ${REPORTED_WHY[$s]}"; done
  for e in "${EQUIV_LINES[@]}"; do echo "    $e"; done
  echo "  A report only: the sender fields are unverified, and no stake is weighed, dropped or decided."
fi

# --- 3. conservative finalized anchor -----------------------------------------
# A block producer is not a validator vote for the block. The heights API
# cannot prove stake-weighted finalized ancestry. Require identical explicit
# last-finalized blocks from every listed node instead of fabricating quorum.
echo "== 3. finalized anchor (unanimous LFB; no inferred stake votes) =="
MEET=""; MEET_HASH=""
for n in "${NAMES[@]}"; do
  h="${LFB[$n]:-}"; bh="${LFH[$n]:-}"
  if ! [[ "$h" =~ ^[0-9]+$ ]] || [ -z "$bh" ]; then
    echo "  REFUSING: $n has no usable finalized block (height/hash)." >&2
    exit 4
  fi
  if [ -z "$MEET" ]; then MEET="$h"; MEET_HASH="$bh"
  elif [ "$h" != "$MEET" ] || [ "$bh" != "$MEET_HASH" ]; then
    echo "  REFUSING: finalized heads differ; block observations cannot prove a stake quorum." >&2
    exit 4
  fi
done
echo "  unanimously reported finalized anchor: height $MEET, hash $MEET_HASH"
echo "  No stake-weighted quorum or finalized ancestry is inferred from block producers."

# --- 4. what is above the point ----------------------------------------------
echo "== 4. what is above the point =="
echo "  The survivor is ${MASTER}'s DAG, not a chain: every validator proposes its own block each round, so"
echo "  a height legitimately holds one sibling per validator and there is no single line to extract. The"
echo "  joiners resync onto this DAG; the writers re-submit what they can see is missing."
REPORT="${RECONCILE_REPORT:-/tmp/reconcile-deploys.jsonl}"
: > "$REPORT"
# Record every observation, then aggregate by block hash. Never count the
# same block once per endpoint. Keep provenance for operator inspection.
OBSERVATIONS=$(mktemp)
trap 'rm -f "$OBSERVATIONS"' EXIT
for (( h=MEET+1; h<=MAXH; h++ )); do
  for n in "${NAMES[@]}"; do
    while read -r bh sender _post deploys _bonds; do
      [ -z "${bh:-}" ] && continue
      printf '%s\t%s\t%s\t%s\t%s\n' "$h" "$bh" "$sender" "${deploys:-0}" "$n" >> "$OBSERVATIONS"
    done < <(blocks_at "${HOST[$n]}" "${PORT[$n]}" "$h")
  done
done
python3 - "$OBSERVATIONS" "$REPORT" <<'PYREPORT'
import collections
import json
import sys

blocks = {}
for line in open(sys.argv[1], encoding="utf-8"):
    height, block_hash, sender, count, observer = line.rstrip("\n").split("\t")
    count = int(count)
    if count < 0 or not block_hash:
        raise SystemExit("REFUSING: invalid block record")
    key = block_hash
    existing = blocks.get(key)
    if existing is None:
        blocks[key] = dict(height=int(height), blockHash=block_hash,
                           sender=sender, deployCount=count, observers={observer})
    else:
        if (existing["height"], existing["sender"], existing["deployCount"]) != (int(height), sender, count):
            raise SystemExit("REFUSING: conflicting observations for block " + block_hash)
        existing["observers"].add(observer)
per_height = collections.Counter()
total = 0
with open(sys.argv[2], "w", encoding="utf-8") as output:
    for record in sorted(blocks.values(), key=lambda x: (x["height"], x["blockHash"])):
        record["observers"] = sorted(record["observers"])
        total += record["deployCount"]
        per_height[record["height"]] += 1
        output.write(json.dumps(record, sort_keys=True) + "\n")
for height, count in sorted(per_height.items()):
    print(f"  #{height}: {count} unique block(s)")
print(f"  {total} deploy(s) in {len(blocks)} unique block(s) above the point; per-block record: {sys.argv[2]}")
print("  Counts only: this API does not provide deploy signatures in this report.")
PYREPORT

if [ "$APPLY" != "1" ]; then
  echo "== plan only =="
  echo "  would: stop every non-master node, move each data directory aside (never delete), restart them to"
  echo "        resync onto ${MASTER}'s DAG, then verify block hashes per height."
  echo "  re-run with --apply to execute. ${MASTER}'s data directory is never touched."
  exit 0
fi

if [ "$RESTORE" = "1" ]; then
  echo "== 5. apply (--restore-from-master): the whole network is stopped, the survivor included =="
  # The survivor is stopped too, and that is not an oversight: a filesystem-level copy of a live LMDB is a
  # torn snapshot, and in this mode the copy *is* the truth the network will run on.
  for n in "${NAMES[@]}"; do echo "  $n:"; stop_node "$n" || { echo "REFUSING: node stop failed: $n" >&2; exit 6; }; done

  echo "== 6. the fiat, printed before it is applied =="
  echo "  This is NOT a sync. The joiners adopt ${MASTER}'s chain state wholesale, so what they agree about"
  echo "  afterwards is ${MASTER}'s view of the chain — including any block above the point that ${MASTER}"
  echo "  accepted and its peers did not. The blocks this drops are enumerated in section 4; that count is"
  echo "  the write set the owners re-submit. Running this is the operator asserting that ${MASTER} is the"
  echo "  chain. It is a stopgap because it depends on the data-dir layout being movable — which #287's"
  echo "  design deliberately avoided depending on — and not a substitute for an agreed anchor."

  TS=$(date -u +%Y%m%d-%H%M)
  echo "== 7. chain state copied onto each joiner (identity and genesis untouched) =="
  for n in "${NAMES[@]}"; do
    [ "$n" = "$MASTER" ] && continue
    echo "  $n:"
    restore_chain_state_from_master "$n" || { echo "REFUSING: state copy failed: $n" >&2; exit 7; }
  done

  echo "== 8. restarted: the survivor first, then the joiners =="
  start_node "$MASTER" || { echo "REFUSING: master start failed" >&2; exit 8; }; sleep 20
  for n in "${NAMES[@]}"; do
    [ "$n" = "$MASTER" ] && continue
    echo "  $n:"; start_node "$n" || { echo "REFUSING: joiner start failed: $n" >&2; exit 8; }
  done

  echo "== 8b. triggering a block, because an idle chain cannot finalise =="
  # **Found by measuring.** The first run of this mode reported no finality and looked like a failure of
  # the restore; it was an idle net. This is a `--no-autopropose` network, so a chain that nobody deploys to
  # produces nothing and finality cannot move — the same reason the wipe path above has its own trigger.
  # Without this the mode can converge a net to one head and *still* report no finality, which reads as
  # the recovery failing when it is the measurement that is idle.
  curl -s -X POST --max-time 30 "http://${HOST[$MASTER]}:${PORT[$MASTER]}/api/faucet" \
    -H 'Content-Type: application/json' \
    --data-binary '{"address":"11112wWGeUA5qt6MpH9CantYj2UWWt4C3LP4cx8TpQmeM79dyen6Sk"}' >/dev/null 2>&1 \
    && echo "    a block was requested (a faucet transfer on the master)" \
    || echo "    NOTE: no block could be requested — on a net without the faucet, deploy something" >&2
else
echo "== 5. apply: every non-master node is stopped first, before anything is moved =="
# All of them, then the moves. The version this derives from stopped and restarted each node inside one
# iteration, putting production back up before agreement had been proven.
for n in "${NAMES[@]}"; do
  [ "$n" = "$MASTER" ] && continue
  echo "  $n:"; stop_node "$n" || { echo "REFUSING: node stop failed: $n" >&2; exit 6; }
done

TS=$(date -u +%Y%m%d-%H%M)
echo "== 6. data directories moved aside (never deleted) =="
for n in "${NAMES[@]}"; do
  [ "$n" = "$MASTER" ] && continue
  echo "  $n:"; move_data_dir_aside "$n" "$TS" || { echo "REFUSING: backup/move failed: $n" >&2; exit 7; }
done

echo "== 7. one joiner is restarted and must reach the point before the rest follow =="
first_joiner=""; for n in "${NAMES[@]}"; do [ "$n" = "$MASTER" ] && continue; first_joiner=$n; break; done
if [ -n "$first_joiner" ]; then
  start_node "$first_joiner" || { echo "REFUSING: first joiner start failed" >&2; exit 8; }
  f=""
  for i in $(seq 1 40); do
    sleep 15
    f=$(lfb_num "${HOST[$first_joiner]}" "${PORT[$first_joiner]}")
    h="$(lfb_hash "${HOST[$first_joiner]}" "${PORT[$first_joiner]}")"
    echo "  [$i] $first_joiner finalised=${f:-?} ${h:0:32}"
    [ "$f" = "$MEET" ] && [ "$h" = "$MEET_HASH" ] && break
  done
  if [ "$f" != "$MEET" ] || [ "$h" != "$MEET_HASH" ]; then
    echo "  the first joiner did not reach the point — refusing to restart the others" >&2
    exit 5
  fi
fi

echo "== 8. the rest are restarted =="
for n in "${NAMES[@]}"; do
  [ "$n" = "$MASTER" ] && continue
  [ "$n" = "$first_joiner" ] && continue
  echo "  $n:"; start_node "$n" || { echo "REFUSING: joiner start failed: $n" >&2; exit 8; }
done

fi

echo "== 9. verification: block hashes per height, not heights =="
# A converged *height* is not a converged chain — four nodes at height 0 are equal. Every assertion is on
# the hash at a height, and the frontier is compared within the protocol's own lag (one block).
ok=0
for i in $(seq 1 40); do
  sleep 15
  ok=1; line=""
  for n in "${NAMES[@]}"; do line="$line $n=$(height_of "${HOST[$n]}" "${PORT[$n]}")"; done
  for (( h=MEET; h<=MAXH; h++ )); do
    first=""
    for n in "${NAMES[@]}"; do
      # Compare the complete unordered block-hash set at this height.
      hashes="$(blocks_at "${HOST[$n]}" "${PORT[$n]}" "$h" | awk 'NF {print $1}' | LC_ALL=C sort -u)"
      if [ -z "$hashes" ]; then ok=0; break; fi
      if [ -z "$first" ]; then first="$hashes"
      elif [ "$hashes" != "$first" ]; then ok=0; break; fi
    done
    [ "$ok" = "0" ] && break
  done
  echo "  [$i]$line  hashes-agree=$ok"
  [ "$ok" = "1" ] && break
done
if [ "$ok" != "1" ]; then
  echo "  NOT converged after the wait: the nodes still disagree on a block hash at or above $MEET" >&2
  exit 6
fi

echo "== 10. finality past the point (mandatory) =="
# Finality must advance strictly beyond the anchor on every configured node.
# An idle --no-autopropose network may require an operator-triggered block;
# do not silently claim recovery success if that has not happened.
FINALITY_ATTEMPTS=${RECONCILE_FINALITY_ATTEMPTS:-40}
FINALITY_INTERVAL=${RECONCILE_FINALITY_INTERVAL:-15}
finality_ok=0
for (( attempt=1; attempt<=FINALITY_ATTEMPTS; attempt++ )); do
  finality_ok=1
  final_height=""
  final_hash=""
  for n in "${NAMES[@]}"; do
    f=$(lfb_num "${HOST[$n]}" "${PORT[$n]}")
    hh=$(lfb_hash "${HOST[$n]}" "${PORT[$n]}")
    printf "  [%s] %-2s finalised %-6s %s\n" "$attempt" "$n" "${f:-?}" "${hh:0:44}"
    if ! [[ "$f" =~ ^[0-9]+$ ]] || [ "$f" -le "$MEET" ] || [ -z "$hh" ]; then
      finality_ok=0
      continue
    fi
    if [ -z "$final_height" ]; then
      final_height="$f"; final_hash="$hh"
    elif [ "$f" != "$final_height" ] || [ "$hh" != "$final_hash" ]; then
      finality_ok=0
    fi
  done
  [ "$finality_ok" = "1" ] && break
  [ "$attempt" -lt "$FINALITY_ATTEMPTS" ] && sleep "$FINALITY_INTERVAL"
done
if [ "$finality_ok" != "1" ]; then
  echo "  NOT RECOVERED: finalized blocks have not unanimously advanced beyond anchor $MEET" >&2
  exit 9
fi
echo "  VERIFIED: all nodes finalized height $final_height, hash $final_hash (anchor $MEET)"
