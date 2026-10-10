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
#      equality is the whole of the computed anchor: no quorum is inferred, and no stake fraction is
#      printed. When no node has a usable finalised block at all — the state this tool's own recovery
#      exists for — the operator may **name** the anchor instead (`RECONCILE_ANCHOR=<hash>`), and the tool
#      states that it is the operator's root rather than a meet it computed (C269);
#   2. **reports** suspected equivocation from the block API's own `sender` field (two distinct blocks by
#      one sender at one height). It is one endpoint's word — this script checks no signature and binds no
#      endpoint to a bonded key — so it is a suspicion worth a human's attention, never an attributable
#      artefact, and **nothing is dropped, reweighted or decided on it** (§2);
#   3. states the anchor — **which** anchor it is, computed or operator-named — and what each node
#      reported, rather than implying a quorum it did not compute;
#   4. enumerates what is above the point, per **unique** block — the nodes that served it, its accepted
#      deploy count, and the signatures of the deploys its merge rejected — and never drops it silently;
#   5. (`--apply`) stops every non-master node, moves each data directory aside — **never deleting** —
#      and restarts them to resync from the master's DAG;
#   5b. (`--restore-from-master`) copies the master's *chain state* onto each joiner instead, for the case
#      step 5 cannot reach — a net with no finalised fringe has nothing to resync to (C259). This is
#      **not a sync**: the joiners adopt the master's view wholesale, which is the operator asserting a
#      winner. It is a labelled stopgap, it never copies node identity, and the tool prints what it is
#      adopting and what it is dropping before it runs;
#   6. verifies the outcome on **block hashes per height** — not on heights, where four nodes at zero
#      are equal — **and** on finality past the point, refusing (exit 9) when either is unmet.
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
#   and, when the nodes cannot agree on a finalised block because none has one:
#         RECONCILE_ANCHOR=<block hash> ... reconcile-network.sh [--apply]
#   The hash is the block the net last agreed on (the operator's call, not this tool's). The joiners must
#   be configured with `--sync-anchor <the same hash>` for the wipe path to resync there; `--restore-from-
#   master` does not need it, because it copies the master's chain state.

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

# Every block a node holds at one height, one line per block:
#   "<blockHash> <sender> <postStateHash> <deployCount> <rejectedDeploys>"
# A height legitimately holds one block per bonded validator, so this returns *all* of them — which is
# what makes the equivocation check possible from the API alone, with no node-internal surface.
#
# **The last field is a compact JSON array and is never empty** (`[]` when a block rejected nothing),
# so the line has a fixed field count with nothing that can vanish. `read` splits on *runs* of
# whitespace with the default IFS, so an absent field would shift every field after it — the shape a
# reader here must not have. The rejected deploys are base16 deploy **signatures** (the deploy id,
# `casper/src/merging.rs`: `deploy_id: d.deploy.sig`), so no element of the array can carry a space.
#
# The bond map was a sixth field until the stake-weighted meet was removed (#305). Nothing reads it
# now — the meet and the `stake_of` helper that consumed it went together — and a field that only
# looks like a denominator is worse than no field: this tool has **no** stake arithmetic (C261).
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
    rejected=json.dumps([str(d) for d in (x.get('rejectedDeploys') or [])], separators=(',',':'))
    print('%s %s %s %s %s' % (x.get('blockHash',''), x.get('sender',''),
                              x.get('postStateHash',''), x.get('deployCount',0), rejected))"
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
# The height of a named block, read from the block itself — `/api/block/{hash}` answers with its own
# `blockInfo`, which carries the number, so an operator-named anchor needs no height from the operator.
anchor_height() { api "$1" "$2" "/api/block/$3" | json_field "b.get('blockNumber','')"; }


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

# Ask the master for one block. A `--no-autopropose` net that nobody deploys to produces nothing, and
# finality is a function of the blocks' justifications, so it cannot move: a converged net can *still*
# report no finality, which reads as the recovery failing when it is the measurement that is idle. §8b
# found this by measuring; §10 needs it repeatedly, because a fringe can take more than one round of new
# blocks to pass the anchor. Best-effort by design — the live net's faucet is mounted only in dev mode,
# so a refusal is a note and the wait still runs.
trigger_block() {
  # **The transport is `api()`'s, and for a while it was not.** This POSTed to
  # `http://${HOST[$MASTER]}:…`, which is the *table's* host token: for a node file that spells a local
  # node `local` — one of the three spellings the header documents — that URL does not resolve, the
  # curl fails, and the section printed "no block could be requested", which reads as a net without the
  # faucet rather than as a broken request. Measured 2026-10-10: `http://local:42403` exits 6 while
  # `http://localhost:42403` answers, and the run that should have driven the chain produced nothing.
  local body='{"address":"11112wWGeUA5qt6MpH9CantYj2UWWt4C3LP4cx8TpQmeM79dyen6Sk"}'
  if is_local "${HOST[$MASTER]}"; then
    curl -s -X POST --max-time 30 -H 'Content-Type: application/json' --data-binary "$body" \
      "http://127.0.0.1:${PORT[$MASTER]}/api/faucet" >/dev/null 2>&1
  else
    $SSH "root@${HOST[$MASTER]}" "curl -s -X POST --max-time 30 -H 'Content-Type: application/json' \
      --data-binary '$body' http://127.0.0.1:${PORT[$MASTER]}/api/faucet" >/dev/null 2>&1
  fi \
    && echo "    a block was requested (a faucet transfer on the master)" \
    || { echo "    NOTE: no block could be requested — on a net without the faucet, deploy something" >&2; return 1; }
}

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
    while read -r bh sender _post _deploys _rejected; do
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

# --- 3. the anchor: computed by agreement, or named by the operator -----------
# A block producer is not a validator vote for the block. The heights API
# cannot prove stake-weighted finalized ancestry. So the anchor is either what the
# nodes **report identically** — no quorum inferred, nothing fabricated — or, when
# they cannot agree because a net's finality is frozen and no node has a usable
# finalised block at all, the block the **operator** names (`RECONCILE_ANCHOR`).
# The two are different answers and the tool says which one it has: a refusal when
# it cannot compute a meet, and an operator's root when nobody can (C269). What it
# never does is present the operator's choice as a computed one.
echo "== 3. the anchor =="
ANCHOR=${RECONCILE_ANCHOR:-}
MEET=""; MEET_HASH=""
if [ -n "$ANCHOR" ]; then
  # **The operator's anchor, and it is labelled as theirs.** Checked here only for the one thing this
  # tool can check without a quorum: that the master holds the block at all (an anchor nobody has is
  # nothing to restore to). Its height comes from the block itself, because §4 enumerates by height.
  MEET=$(anchor_height "${HOST[$MASTER]}" "${PORT[$MASTER]}" "$ANCHOR")
  if ! [[ "$MEET" =~ ^[0-9]+$ ]]; then
    echo "  REFUSING: RECONCILE_ANCHOR $ANCHOR could not be read from ${MASTER} (no such block, or the" >&2
    echo "            node did not answer). An anchor to restore to has to be a block a node holds." >&2
    exit 4
  fi
  MEET_HASH="$ANCHOR"
  echo "  the operator's anchor: height $MEET, hash $MEET_HASH"
  echo "  This is **your** root, not a computed meet: this tool has verified that ${MASTER} holds the block"
  echo "  and nothing else about it — not that stake finalised it, not that any other node agrees."
  echo "  What the nodes report, reported rather than resolved:"
  for n in "${NAMES[@]}"; do
    printf "    %-2s finalised %-6s %s\n" "$n" "${LFB[$n]:-none}" "${LFH[$n]:0:44}"
  done
  echo "  If they disagree, that is the state you are recovering from; the block you named is the point"
  echo "  this recovery starts at, and every block above it is validated rather than installed."
else
  for n in "${NAMES[@]}"; do
    h="${LFB[$n]:-}"; bh="${LFH[$n]:-}"
    if ! [[ "$h" =~ ^[0-9]+$ ]] || [ -z "$bh" ]; then
      echo "  REFUSING: $n has no usable finalized block (height/hash)." >&2
      echo "            Nothing to compute a meet from. If you know the block this net last agreed on," >&2
      echo "            name it: RECONCILE_ANCHOR=<hash> (the operator's root, stated as yours, not ours)." >&2
      exit 4
    fi
    if [ -z "$MEET" ]; then MEET="$h"; MEET_HASH="$bh"
    elif [ "$h" != "$MEET" ] || [ "$bh" != "$MEET_HASH" ]; then
      echo "  REFUSING: finalized heads differ; block observations cannot prove a stake quorum." >&2
      echo "            The nodes do not agree on a point, so this tool cannot compute one. If you know" >&2
      echo "            the block this net last agreed on, name it: RECONCILE_ANCHOR=<hash>." >&2
      exit 4
    fi
  done
  echo "  unanimously reported finalized anchor: height $MEET, hash $MEET_HASH"
  echo "  No stake-weighted quorum or finalized ancestry is inferred from block producers."
fi

# --- 4. what is above the point ----------------------------------------------
echo "== 4. what is above the point =="
echo "  The survivor is ${MASTER}'s DAG, not a chain: every validator proposes its own block each round, so"
echo "  a height legitimately holds one sibling per validator and there is no single line to extract. The"
echo "  joiners resync onto this DAG; the writers re-submit what they can see is missing."
REPORT="${RECONCILE_REPORT:-/tmp/reconcile-deploys.jsonl}"
: > "$REPORT"
total=0; ublocks=0; nobs=0
for (( h=MEET+1; h<=MAXH; h++ )); do
  # **One block is one block, however many nodes serve it.** `/api/blocks/h/h` is answered *per node*,
  # so a block held by all four comes back four times — and the version this replaces counted each
  # *observation*, inflating both the block count and the deploy total by the replication factor. That
  # is #302's double-count (the one that got the stake meet deleted) reappearing one section down, and
  # a write report an operator re-submits from has to be the block's own count, not the network's view
  # of it. The key is the block's **hash**; `obs_seen` also keys on the observing node, so a node that
  # answers with one block twice cannot inflate it either. `observers` keeps where it was seen, so the
  # dedupe is visible rather than silent.
  declare -A obs_seen=() obs_sender=() obs_deploys=() obs_rejected=() obs_nodes=()
  seen_bh=""; nblocks=0; nobs=0
  for n in "${NAMES[@]}"; do
    while read -r bh sender _post deploys rejected; do
      [ -z "${bh:-}" ] && continue
      [ -n "${obs_seen[$bh:$n]:-}" ] && continue
      obs_seen[$bh:$n]=1; nobs=$((nobs+1))
      if [ -z "${obs_nodes[$bh]:-}" ]; then
        nblocks=$((nblocks+1)); seen_bh="$seen_bh $bh"
        obs_sender[$bh]="$sender"; obs_deploys[$bh]="${deploys:-0}"
        obs_rejected[$bh]="${rejected:-[]}"; obs_nodes[$bh]="\"$n\""
      else
        obs_nodes[$bh]="${obs_nodes[$bh]},\"$n\""
      fi
    done < <(blocks_at "${HOST[$n]}" "${PORT[$n]}" "$h")
  done
  for bh in $seen_bh; do
    total=$(( total + ${obs_deploys[$bh]:-0} ))
    printf '{"height":%s,"blockHash":"%s","sender":"%s","deployCount":%s,"observers":[%s],"rejectedDeploys":%s}\n' \
      "$h" "$bh" "${obs_sender[$bh]}" "${obs_deploys[$bh]}" "${obs_nodes[$bh]}" "${obs_rejected[$bh]}" \
      >> "$REPORT"
  done
  ublocks=$((ublocks+nblocks))
  [ "$nblocks" -gt 0 ] && printf "  #%s: %s unique block(s) from %s answer(s)\n" "$h" "$nblocks" "$nobs"
done
echo "  $total deploy(s) above the point across $ublocks unique block(s); the per-block record is $REPORT"
echo "  (a block's *bodies* are not in the heights route — /api/block/{hash} carries the terms, the heights"
echo "   route carries counts. The owners re-submit; this tool does not claim to replay them.)"

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
  echo "  accepted and its peers did not. Section 4 enumerates the blocks this drops, one record per unique"
  echo "  block, with each block's accepted deploy count and the signatures of the deploys its merge"
  echo "  rejected; that record — not its total, which counts accepted deploys — is what the owners"
  echo "  re-submit from. Running this is the operator asserting that ${MASTER} is the chain. It is a"
  echo "  stopgap because it depends on the data-dir layout being movable — which #287's design"
  echo "  deliberately avoided depending on — and not a substitute for an agreed anchor."

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
  # the restore; it was an idle net. This is a `--no-autopropose` network, so a chain that nobody deploys
  # to produces nothing and finality cannot move. §10 asks again while it waits, for both paths — the
  # wipe path has no trigger of its own, which is why its §7 waits on a joiner reaching the anchor and
  # then hands over to §10, whose wait drives the chain rather than only watching it.
  trigger_block || true
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
    # **A height nobody has produced is not a disagreement.** `MAXH` is `/api/status`'s
    # `latestBlockNumber`, which counts the round a node is in and sits one ahead of the highest
    # height the block API serves — so the walk's last step is routinely a height with no blocks on
    # any node. Requiring a non-empty set there failed a **converged** net on 2026-10-10: the three
    # nodes agreed at every height 40–44, height 45 was empty on all three, and this section reported
    # "the nodes still disagree on a block hash at or above 41". Empty on *every* node is the frontier
    # and is skipped; empty on *one* node while another has blocks is the divergence being compared
    # for, and still fails.
    first=""; any_empty=0; any_full=0
    for n in "${NAMES[@]}"; do
      hashes="$(blocks_at "${HOST[$n]}" "${PORT[$n]}" "$h" | awk 'NF {print $1}' | LC_ALL=C sort -u)"
      if [ -z "$hashes" ]; then any_empty=1; continue; fi
      any_full=1
      if [ -z "$first" ]; then first="$hashes"
      elif [ "$hashes" != "$first" ]; then ok=0; break; fi
    done
    if [ "$ok" = "0" ] || { [ "$any_empty" = "1" ] && [ "$any_full" = "1" ]; }; then ok=0; break; fi
  done
  echo "  [$i]$line  hashes-agree=$ok"
  [ "$ok" = "1" ] && break
done
if [ "$ok" != "1" ]; then
  echo "  NOT converged after the wait: the nodes still disagree on a block hash at or above $MEET" >&2
  exit 6
fi

echo "== 10. finality past the point =="
# **This section used to be a print.** It printed each node's last-finalized height and hash and exited
# 0 — including when every node still answered "Finalized fringe is not available.", which is what the
# drill transcript `spec/audit/evidence/n-reconcile-drill/restore-run.txt` shows: four `finalised ?`
# lines and a zero exit, under a closing sentence claiming to be #287's falsifier. Nothing here could
# fail. It asserts the acceptance clause instead — *finality advances past the reconciliation point* —
# from four rules, each refusing with a named reason:
#
#   A. **usable** — every node reports an integer finalized height and a non-empty hash: the same
#      condition §3 refuses on at the anchor, applied at the end. A frozen fringe reads as unusable.
#   B. **advanced** — every node's finalized height is strictly greater than $MEET. This is the clause
#      itself; equal to the anchor is not progress.
#   C. **one head** — no two nodes report *different* hashes at the *same* finalized height. Two
#      finalized blocks at one height is a finality conflict; different finalized *heights* are a lag,
#      which is why this is not §3's unanimity rule.
#   D. **in the common DAG** — each node's finalized block is present at its height in that node's own
#      block answer, and in the answer of every *other* node whose own height reaches that height. The
#      last-finalized endpoint and the block API are different surfaces: a hash one reports and no
#      node's block index holds is a string, not a block.
#
# **D alone is not enough, and C is why.** A DAG legitimately holds every sibling at a height, so
# "each node's finalized block is in every node's answer" is true while two *finalized* heads exist.
# C is what makes the pairwise rule mean one chain, and it cannot fire on an honest net: the fringe's
# `max()` is by (height, id), so nodes holding one fringe finalize the same block.
#
# **What this cannot decide, stated rather than implied.** The heights API carries no ancestry, so
# nothing here proves the finalized blocks lie on one chain — only that they are blocks every node that
# reached their height actually holds. With §9 (identical unordered hash sets at every height
# MEET..MAXH) that is the strongest statement these two endpoints support; the ancestry question is
# #287's, and it is why the meet is still owed rather than declared.
adv_ok=0; why="the finality wait never ran"
for i in $(seq 1 "${RECONCILE_FINALITY_ROUNDS:-40}"); do
  sleep 15
  trigger_block >/dev/null 2>&1 || true
  declare -A FINH=() FINB=() NOWH=()
  adv_ok=1; line=""
  for n in "${NAMES[@]}"; do
    FINH[$n]="$(lfb_num "${HOST[$n]}" "${PORT[$n]}")"
    FINB[$n]="$(lfb_hash "${HOST[$n]}" "${PORT[$n]}")"
    NOWH[$n]="$(height_of "${HOST[$n]}" "${PORT[$n]}")"
    line="$line $n=${FINH[$n]:-?}"
  done
  for n in "${NAMES[@]}"; do
    if ! [[ "${FINH[$n]:-}" =~ ^[0-9]+$ ]] || [ -z "${FINB[$n]:-}" ]; then
      adv_ok=0; why="$n reports no usable finalized block (height '${FINH[$n]:-}', hash '${FINB[$n]:-}')"; break
    fi
  done
  if [ "$adv_ok" = "1" ]; then
    for n in "${NAMES[@]}"; do
      if [ "${FINH[$n]}" -le "$MEET" ] 2>/dev/null; then
        adv_ok=0; why="$n finalised at ${FINH[$n]}, which is not past the point ($MEET)"; break
      fi
    done
  fi
  if [ "$adv_ok" = "1" ]; then
    declare -A fin_seen=()
    for n in "${NAMES[@]}"; do
      k="${FINH[$n]}"
      if [ -n "${fin_seen[$k]:-}" ] && [ "${fin_seen[$k]}" != "${FINB[$n]}" ]; then
        adv_ok=0; why="two finalized blocks at height $k: ${fin_seen[$k]:0:12}… and ${FINB[$n]:0:12}…"; break
      fi
      fin_seen[$k]="${FINB[$n]}"
    done
  fi
  if [ "$adv_ok" = "1" ]; then
    for n in "${NAMES[@]}"; do
      fh="${FINH[$n]}"; fb="${FINB[$n]}"
      for m in "${NAMES[@]}"; do
        # A node's own block index must serve its finalized block whatever its tip says; another
        # node's must, but only once it has reached that height — below it, absence is a lag.
        if [ "$m" != "$n" ] && ! [ "${NOWH[$m]:-0}" -ge "$fh" ] 2>/dev/null; then continue; fi
        if ! blocks_at "${HOST[$m]}" "${PORT[$m]}" "$fh" | awk 'NF {print $1}' | grep -Fqx "$fb"; then
          adv_ok=0; why="$n finalised $fh ${fb:0:12}… but $m (height ${NOWH[$m]:-?}) does not hold it"
          break 2
        fi
      done
    done
  fi
  echo "  finality [$i]$line  past-the-point=$adv_ok"
  [ "$adv_ok" = "1" ] && break
done
if [ "$adv_ok" != "1" ]; then
  echo "  NOT final: $why" >&2
  echo "  Finality has not advanced past $MEET on every node, so the recovery is not verified." >&2
  echo "  (a chain that converged to one head but cannot finalise is C259's state, and this refuses it" >&2
  echo "   rather than reporting it as recovered.)" >&2
  exit 9
fi
echo "  finality advanced past $MEET on every node, and every finalized block is one the nodes' own DAGs hold."
echo "  (this section plus section 9 is #287's falsifier: one head, agreeing block hashes, no genesis.)"
