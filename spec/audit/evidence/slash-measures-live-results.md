# The live arms: A1 and A2 on a two-validator devnet

**What this is.** `slash-measures-results.md` records the two measurements the risk plan owed and says of
each what it does **not** cover: the wire. It runs two `BlockDagKeyValueStorage`s in one process, so a
block reaches the second node by a direct call rather than by gossip. These are the live arms — a
two-validator devnet, real gossip, the built image — and they close that residue. One of them also turned
up **C201**, below.

**The runs.** Two drivers, both in the tree, both re-runnable:

| arm | driver | raw output |
|---|---|---|
| A1 — two floors that disagree | `spec/audit/evidence/a1-live-config-mismatch-run.sh` | `a1-live-config-mismatch.log.txt`, `a1-live-blocks.log.txt` |
| A2 — an equivocation between live nodes | `spec/audit/evidence/a2-live-equivocation-run.sh` | `a2-live-equivocation.log.txt`, `a2-live-blocks.log.txt` |

They need one tool change, also in the tree: `tools/devnet.sh` now takes **per-node** extra `rnode run`
flags (`DEVNET_EXTRA_FLAGS_<container name, dashes → underscores>`, and `DEVNET_EXTRA_FLAGS` for every
node). Every arm here is about one node set differently from its peers, and the shared flag string is
shared *on purpose* — a genesis parameter given to one node and not another is a different chain (AUDIT
C46) — so the escape hatch had to be per node.

## A2 — the fix works between two live nodes

Validator 1 is started with `--equivocation-injection`, which broadcasts a twin of every block it creates:
the same block, equally signed, at the same `(sender, seq_num)`.

**The injection.** 6 twins, each announced by the offender itself:

```
WARN [casper.blocks.Proposer] EQUIVOCATION INJECTION: broadcasting a second block #6 (seq 5) from this
node's own key. Every peer will record this node as having equivocated and may slash it.
```

**The refusals.** 65 `equivocation` lines in the bootstrap's log — its H-1 gate refusing the twins before
any write, and keeping their headers.

**The slashes.** 59 of the bootstrap's proposer's own lines, one per block, at a chain height of 62:

```
[pos] slashing 1 bonded validator(s) whose block failed validation here (tier and share):
  04dbe32c Malicious/10000bps (equivocation evidence)
```

That is C200 working end to end on a live network: the **`Malicious` tier** (10 000 bps), with the
**equivocation evidence attached**, for a validator whose offending block the bootstrap never stored —
decided by a node that had only the header to go on.

**On chain.** `show-block`'s bonds map for a recent block carries **one** validator where genesis carried
two, and the transition is at block 22 — the confiscation, in the state, read off a block header:

```
block 43 … 48:  "validator": 04f700a4…(the bootstrap)  "stake": 100      (only)
first block carrying a single validator: block 22
```

**No split.** Both nodes end at the same height with the same view; the offender stops proposing once it
is out of the pool (6 injections, then silence) and the bootstrap carries on alone.

**A correction worth keeping.** The driver's first version grepped the `show-blocks` dump for `slash` and
reported *"NO slash in 25 blocks"* on exactly the run in which the bootstrap logged three slashes.
`show-blocks` prints a block's header and its deploys — its `bonds` map, its `deployCount` — and **not**
`state.systemDeploys`, which is where a `Slash` lives, so that grep could only ever find nothing. The
checks now read the proposer's own line and the bonds transition, and the driver says why. The A1 driver
had the same shape of bug (a flag that does not exist), and both carry the note: **an absence claim
produced by a command that failed is not an absence.**

## A1 — a floor that disagrees wedges the network, and no stake is destroyed

The bootstrap keeps the shipped `casper.min-phlo-price` of 1; validator 1 is given 5. A deploy priced at 1
— what `tools/devnet.sh deploy` sends — is then accepted by one and refused by the other.

| | bootstrap (floor 1) | validator 1 (floor 5) |
|---|---|---|
| height before the deploy | 8 | 1 |
| height 60 s after | 26 | **1** |
| finality | `Finalized fringe is not available` | the same |

Validator 1's log refuses **every** block from #16 onward, not one:

```
WARN [casper.blocks.BlockProcessor] Block #16 … from 04f700a4… failed validation: a deploy's phlo price
  is below the minimum        (and #17, #18, … #25, and on)
```

The deploy is included once; what keeps the violation alive is the **autopropose dummy deploy**, which the
bootstrap keeps adding to every block at the same price. So the strict node refuses a block in the chain's
ancestry and therefore every descendant of it: its view freezes while the permissive node runs on, and
finality — which needs >2/3 of the stake, i.e. both — stops for the whole network.

**No stake is destroyed.** Over 26 blocks: **0** matches for `slash` anywhere, and every block's bonds map
carries both validators at **100** — 52 `"stake": 100` lines for 26 blocks, unchanged from genesis.

So C198 does what it claims — the disagreement costs no one their bond — and the measurement says plainly
what it does not fix: a mis-set floor is still a way to wedge a network. It is the same hazard C198 was
aimed at, minus the confiscation.

## C201 — a slash re-nominated for a validator that is no longer bonded

The A2 run turned up something neither arm was looking for, and it is registered as **C201** rather than
explained away here.

The bootstrap's proposer emits a `Slash` for the offender on **every** block it produces — 59 lines and
still counting when the run was stopped, at a chain height of 62. But the offender is gone from the active
set: `show-block` on any recent block carries one validator, and the transition is block 22.

`add_recorded_equivocations` filters on `bonded`, and `bonded` is `compute_bonds(pre_state_hash)` —
`pos:active` read at the pre-state — so it should exclude an offender slashed out of the pool *and* the
active set. The observation and the reading of the code cannot both be right.

**One half of it is now measured and the read is clean.**
`a_slashed_validator_is_absent_from_the_bonds_at_the_post_state` (`casper/src/runtime_manager.rs`) slashes
a bonded validator in a block and then asks `compute_bonds` at that block's **post-state**: the victim is
absent, the operator untouched. So the leaf is written and read correctly at a post-state, and what
differs in the live run is *which hash* the proposer reads — the **merged** pre-state, whose native view
is reconstructed from the native-changes sidecar (`MergeScope::merge`, issue #74).

**Impact, as far as this run shows.** Bounded. The repeated slash is idempotent — the offender is already
out of the pool, so `slash` confiscates nothing and both nodes replay it identically; the chain advanced
to 62 and the two nodes never disagreed. But the read that is stale here is the *proposer's* own view of
who is bonded, and the same read feeds `check_active_validator` and the attestation quorum, so the class
is worth naming even though this instance is harmless.

**What would close it.** The same test as the one above, with a **merge** in the middle: build a
two-parent pre-state whose parents include a slashing block, and assert the victim is absent from
`compute_bonds(merged_root)`. That is one test and it needs a merge fixture, which is the unit this row
asks for.
