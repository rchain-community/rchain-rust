# The reconciliation drill: the plan is right, and the recovery cannot be demonstrated — with both reasons

**Result: the plan phase is verified; the acceptance is NOT demonstrated, and this file says why.** Two
independent reasons, one about the instrument and one about the design, and the second is the more
important finding.

**Rig.** Four-validator devnet, `--epoch-length 10 --no-autopropose --fresh`, with the C215 divergence
injection armed on the joiners (`--merge-divergence-injection 2/3/4`; the survivor deliberately clean —
see "the first staging" below). The tool is `tools/reconcile-network.sh` on this branch. The raw
material is [`why-no-resync.txt`](why-no-resync.txt).

## What was verified: the plan phase

Run against a genuinely diverged net — heights 3/4/4/3, the clean survivor rejecting the injected
joiners' blocks (14 `state-hash disagreement` lines on it), no node finalising anything:

```
== 1. what each node is showing ==            A h=3   V1 h=4   V2 h=4   V3 h=3   (finalised: none)
== 2. provable equivocation …                 none — every sender has at most one block per height
== 3. the meet (stake-weighted) …             height 0, block c01a6091…, vouched for by A V1 V2 V3
                                              stake: 400 of 400 = 100% — a strict supermajority
== 4. what is above the point …               12 deploy(s), per-block record written
== plan only ==                               exit 0, nothing touched
```

Every part of that is the intended behaviour: no equivocation is invented, the meet is the deepest
height a *strict supermajority of stake* agrees on, and when nothing is finalised that height is
**genesis** — which is the honest answer rather than a winner picked by hand. The tool then exits
without touching anything.

## What was not: the recovery

The joiners were wiped (the devnet's own `reset`, which is the same stop/empty/restart the tool's
`--apply` performs on a droplet) and left to resync from the clean survivor. **They did not converge,
and they did not even validate the survivor's chain:**

```
WARN [casper.blocks.BlockProcessor] Block #1 4ea92991… from 04ea3ce0… failed validation:
     the block's rejected-deploy set does not match its parent
WARN [casper.interpreter.validate] state-hash disagreement on rejected-deploys: block #1 4ea92991… by 04ea3ce0
```

And all four answer `"Finalized fringe is not available."`.

### Reason 1 — the instrument stages *invalidity*, not the ambiguity the incident had

**The injection reaches the block's content.** A proposer's `rejected_deploys` is *computed by its own
merge*, under its own perturbed cache, and then written into the block it publishes. So every block an
injected node produces carries a rejected-deploy set that a clean node cannot reproduce — permanently,
because the set is in the block. The survivor's DAG holds those blocks, so a wiped joiner revalidates
them and refuses each one.

That is **stronger than the incident**, and the difference matters. In the live divergence every node was
honest and each block was valid *to its proposer*; the disagreement was an ambiguity in a derived cache.
Here the chain is un-validatable by anyone who does not share the perturbation. A staging instrument
should not be able to make the chain worse than the thing it stages.

**The first staging made this worse and taught the lesson**: with all four nodes injected, the *survivor
itself* was tainted, and nothing could ever agree with it. Injecting only the joiners was the correction
— the survivor must be clean for a recovery to have a target — and it is not sufficient, for the reason
above.

### Reason 2 — the recovery presupposes a finalised fringe, and TE-1's state may have none

This is the finding, and it is about the design rather than the drill. All four nodes answer
`"Finalized fringe is not available."` — which is *what four divergent heads with frozen finality means*.
A joiner's sync path is keyed on the finalised fringe: it asks the peer for a fringe and restores from
it. With no fringe, the wipe leaves a node with an empty store, a genesis, and peers it cannot sync from.

The TE-1 witness already recorded this dependency in its own words — *"A restoring or joining node syncs
from the approved-genesis / finalised fringe. While finality is stuck that path is closed"* — and this
drill is the measurement of what that costs: **#287's recovery — wipe, resync from the finalised fringe,
verify — has nothing to resync *from* in exactly the state it exists for.** The meet at genesis is
computable, but there is no "sync from genesis" or "sync from an agreed block" path for the joiners to
take.

So the acceptance (*"a network in that state converges to one head, with agreeing block hashes, without a
genesis"*) is not reachable by the mechanism as designed. Closing it needs one of:

1. **a fringe-free restore path** — a joiner that can sync from an arbitrary agreed height (the genesis
   block, or the meet) rather than from a finalised fringe; or
2. **a store-level restore** — the operator copies the survivor's data directory onto the joiners, which
   is what the manual procedure actually did on the old chain (it is not a sync at all); or
3. **a stated limitation** — the tool refuses earlier and says that a net with no finalised fringe cannot
   be reconciled this way, which is at least honest and is close to what this branch's tool already does
   with its meet.

This is a design question for #287 and it is now a measured one rather than an assumed one.

## What this does not show

- **The tool's `--apply` path was not exercised end to end.** The devnet's own `reset` was used for the
  wipe, because the tool's docker control path performs the same three steps; the systemd path is
  untested here and belongs to a droplet.
- **The plan phase's correctness on a *healthy* net** — a meet above genesis, a lagging node that must
  not drag the network back, the pool-disagreement refusal — is unexercised. This staging has no
  finality at all, so every run lands at genesis. Those cases need a net that finalises and then a
  deliberate lag or partition.
- **The equivocation check has never fired.** No injection here produces two signed blocks by one sender
  at one height; the existing `--equivocation-injection` would, and that arm is unrun.
- One host, one attempt per staging.

---

# Second run: the store-level restore (Unit A), and the half of the acceptance the instrument blocks

**What was added.** `--restore-from-master` in the tool: stop the whole network (the survivor included —
a filesystem copy of a live LMDB is a torn snapshot), copy the survivor's *chain state* onto each joiner,
restart. Copied: `blockstorage`, `dagstorage`, `rspace/history`, `rspace/cold` — and `transaction` when it
exists, which on this build it does not, so the copy says so rather than failing. **Never copied**: the
node's identity (`node.key.pem`, `node.certificate.pem`, the validator key — a joiner holding the
survivor's key *is* an equivocator, and no agreement check would see it), and the node-local environments
(the pending deploy pool, `reporting`, `eval/*`, `gateway`).

**What it achieved — the first two clauses of the acceptance, demonstrated.**

```
h=0: c01a6091 | c01a6091 | c01a6091 | c01a6091
h=1: 116d0679 | 116d0679 | 116d0679 | 116d0679
h=2: 9967518a | 9967518a | 9967518a | 9967518a
```

**One head, with agreeing block hashes at every height, on all four nodes, without a genesis.** Before the
restore the four heights were 3/3/4/3 with mutually rejected blocks; after it, every height's hash is
identical across the four. The raw run is [`restore-run.txt`](restore-run.txt) and the reading after it is
[`restore-after.txt`](restore-after.txt).

**What it did not achieve, and this is the honest limit: finality does not advance.** All four still answer
`"Finalized fringe is not available."`, with `finalityStall: none`. **The reason is the instrument, not the
recovery.** The joiners were restarted with `docker start`, which reuses the container's original command —
so they come back with `--merge-divergence-injection` still armed. Their merges stay perturbed, so no node
can validate the chain they now hold, so nothing finalises. A drill that wants that clause has to
**un-inject the joiners**, which a restart cannot do: it needs the containers *recreated* with their
volumes preserved, and the tool's `start_node` deliberately does the least destructive thing it can.

**So this run establishes**: the store-level restore converges a diverged net to one head with agreeing
hashes, without a genesis — which is the acceptance's convergence half, and more than was demonstrated
before it. **It does not establish** the finality clause, and a green reading of that clause under this
instrument would have been the meaningless kind: four identical copies agreeing with each other is a
property of the copy.

**Two bugs the run found, both worth the record.**

- **`local n="$1" host="${HOST[$n]}"` resolves the subscript with the *caller's* `n`.** The words are
  expanded before `local` runs, so `stop_node`/`start_node`/`move_data_dir_aside` used whatever the last
  loop had left in `n`. It went unnoticed in one loop (whose variable and argument coincided) and started
  the wrong container in another. Split into two statements, with the reason in the code.
- **The first copy was destructive.** It `rm -rf`'d each destination directory *before* copying, so a
  failed `cp` — and one failed, on the `transaction` directory this build does not create — left the joiner
  with nothing, while the tool reported "unchanged". It now copies to a staging name and swaps, and skips a
  source that is not there. The tool's own "aside, never gone" rule applies to its copies too.

---

# Third run: disarmed joiners, and the acceptance met

**What changed from the second run.** The second run's joiners kept `--merge-divergence-injection` through
their `docker start`, so their merges stayed perturbed and nothing validated. Here they were **un-injected
before the restore** — which the devnet's own `reset` does, and it also empties their volumes, so the tool's
copy is what fills them. That is the whole difference, and it is a drill-sequencing fix rather than node
code.

**And one more thing the run found, by measuring rather than reading.** After the restore the four nodes
were identical and un-injected, and finality *still* did not advance — because this is a
`--no-autopropose` net and **an idle chain produces nothing**, so there is nothing to finalise. The wipe
path already triggers a block for exactly this reason; the restore path did not. Sending one deploy moved
every reading.

**The result — #287's acceptance, all four clauses:**

```
heights:                17  17  17  17
last-finalized-block:   11 ef05ce68966d09ffc42aeef4e66b8720e31adae3cd059108af400a516190a13c  (all four)
block hashes per height, identical on all four:
  h=0: c01a6091
  h=1: 83795bc0, 99de0848, 9edbbffa, de8a2545
  h=2: a3d2b67a      h=3: 96f6f7e3
  h=4: 132e7f08, d84c9cc6
  h=5: 2e6b09e7, a52fc0d4, ee91a57e
  h=6: 149b34e6      h=7: 142eedca, 4c6c5cbe, 760ad523
```

- **one head** — every height's hash set is identical across the four, including the multi-block heights;
- **agreeing block hashes** — and not merely agreeing heights, which four nodes at zero would also show;
- **without a genesis** — the genesis block is untouched (`c01a6091`, unchanged from before the incident's
  staging) and no chain was re-created;
- **finality advances past the reconciliation point** — the meet was genesis (height 0) and all four
  finalised **height 11 on one hash**, up from "not available" on every node.

Raw: [`restore-disarmed.txt`](restore-disarmed.txt).

**What this is and is not.** It is the store-level path working end to end, and it is a **stopgap**: the
joiners adopted the survivor's store, so the one head they agree on is the survivor's *view* — the operator
asserting a winner, which the tool prints before it applies. It does not make the anchor-based path
(Unit B) unnecessary: that is the one that an operator can run without a data-dir copy, and it is what
would close the partial-divergence case properly. And the instrument's limit stands: this chain's blocks
were produced under perturbation, so its *validity* is not what was demonstrated — its **convergence and
resumed finality** are.

---

# Fourth run: the tool's own transcript, and the three defects the drill found in the tool

**Why a fourth run.** The first three measured the *mechanism*; the acceptance's proof was assembled by hand
afterwards (`restore-disarmed.txt`), because the tool itself could not prove it. #287's review then found two
places where the tool claimed more than it did, and re-running the drill with the fixed tool was the way to
settle both. It settled three more besides — the drill is also a test of the instrument, and the instrument
failed it in three new ways.

**Rig, unchanged except where the run says so.** A four-validator docker devnet (`devnet-bootstrap` +
`devnet-validator-1..3`), `--no-autopropose --epoch-length 10`, driven by faucet transfers so that a chain
that nobody deploys to still produces blocks. The divergence instrument is the C215 injection
(`--merge-divergence-injection`, distinct value per node), armed on the joiners; the survivor stays clean.
The tool is `tools/reconcile-network.sh` on this branch, run with `RECONCILE_CONTROL=docker` and a node file
whose hosts are `local`.

## What was measured

**1. Four heads with no agreed anchor are refused, not resolved** —
[`run-4-refusal-no-anchor.txt`](run-4-refusal-no-anchor.txt). With the injection armed from the start on all
three joiners, the four nodes finished at different heights and froze (57 / 47 / 48 / 48, no node finalising
above its own head). This is TE-1's shape. `== 3.` reports
`REFUSING: finalized heads differ; block observations cannot prove a stake quorum.` and exits **4**, having
touched nothing. The four clauses of the acceptance are not reachable from this state by this tool, and that
is the design rather than a failure of it: an anchor it cannot verify is not an anchor (#287's refusal; C259
owns the mechanism that would recover the no-anchor case).

**2. The write report counted observations** — [`run-4-plan-diverged-prefix.txt`](run-4-plan-diverged-prefix.txt)
against [`run-4-plan-diverged.txt`](run-4-plan-diverged.txt). A second staging took the *same* divergence but
left a usable anchor (three joiners mutually refusing, all three finalised at the same height on one hash,
`b3b161ff…`, while the clean survivor ran ahead). On that state:

```
pre-fix:  #42: 12 block(s)   #43: 12 block(s)   #44: 10 block(s)   → 34 records, one per observation
fixed:    #42: 4 unique block(s) from 12 answer(s) …              → 12 records, one per block,
                                                                    with "observers" and "rejectedDeploys"
```

`/api/blocks/{h}/{h}` is answered per node, so every block came back three times and the counts were the
replication factor. The record is now the block's own: `run-4-deploys.jsonl` is the same run's file.

**3. The finality check printed, and the joiners' block trigger could not reach a `local` host** —
[`run-4-restore-finality-refused.txt`](run-4-restore-finality-refused.txt). `--restore-from-master` against
the anchored divergence: `== 3.` accepts the anchor (height 41), the copy converges the three stores, and
`== 9.` reports `hashes-agree=1` on the first attempt — **the store-level restore worked**. Then `== 10.`
refused:

```
  finality [40] V1=41 V2=41 V3=41  past-the-point=0
  NOT final: V1 finalised at 41, which is not past the point (41)
exit 9
```

The old section would have printed `finalised ?` and exited **0**; this is the reviewer's own negative case
("every node's LFB remains at MEET"), on real nodes. The reason it was 41 was the third defect: `== 8b.` had
printed `no block could be requested`, because the trigger POSTed to `http://${HOST[$MASTER]}:…` — the node
file's token `local`, which does not resolve (`http://local:42403` exits 6) though the tool's header
documents `local` as one of three valid spellings. On a `--no-autopropose` net an unreached trigger means no
blocks, and no blocks means no finality, which reads as the recovery failing.

**4. The acceptance, asserted by the tool itself** — [`run-4-restore.txt`](run-4-restore.txt). With all of
the above fixed, the same run completes:

```
== 9. verification: block hashes per height, not heights ==
  [1] V1=65 V2=65 V3=65  hashes-agree=1
== 10. finality past the point ==
  finality [1] V1=61 V2=61 V3=61  past-the-point=1
  finality advanced past 51 on every node, and every finalized block is one the nodes' own DAGs hold.
exit 0
```

Anchor height **51** (above genesis, unanimously reported), every height's hash set identical across the
three, finality **61** — past the point — and the exit code is the tool's own. This is the first transcript
in this file where the acceptance's clauses are proved by the tool rather than read off by hand.

## The three defects the drill found in the tool

1. **§4 counted observations, not unique blocks** (register **C264**) — §2 above.
2. **§10 named a verification and printed** (register **C265**) — §3 above. The same section's trigger could
   not reach a `local` host, which is why the clause it claims had never been satisfiable on an idle net.
3. **§9 treated a height nobody had produced as a disagreement** (register **C266**) — found between §3 and
   §4 above: the walk runs to `MAXH`, which is `/api/status`'s `latestBlockNumber` and sits one round ahead
   of the highest height the block API serves, so its last step was a height with no blocks on any node. The
   first `--restore-from-master` attempt exited 6, "the nodes still disagree on a block hash at or above 41",
   on a net whose three nodes in fact agreed at every height 40–44.

## What this does not show

- **A frozen four-heads net recovered end to end.** With no node finalising, there is no unanimity for §3 to
  accept, so the tool refuses (measurement 1, and C259's whole subject). The state the acceptance names is
  reachable by this tool only where *some* finality survives.
- **A recovery from a divergent net whose instrument is still armed.** The store copy converges the stores,
  but a node restarting with the injection still armed re-perturbs what it recomputes, and `== 9.` refuses
  the result (this run's third attempt, not committed: `hashes-agree=0` for forty rounds). Removing the
  instrument first — which the devnet's own `reset` does — is the sequencing the third run already found, and
  it is what the successful runs here did.
- **Validity.** As the third run said: these blocks were produced under perturbation, so what is demonstrated
  is convergence and resumed finality, not that the chain they converge on is one anybody would call valid.
