# E2 — the state-tamper probe: the tamper is effective, and no targeting holds (2026-10-13)

**What this is.** Phase 1's second experiment of the R3 plan. The reset's trigger needs a
`FailureCause::Divergence`, which the code defines as *"this node's state or replay disagrees"*. A
**record** edit cannot stage one (`#345`: the restore's revalidation *clears* a merely-marked block) and a
**partition** stages nothing (`e1-partition.md`, measured). What is left is an edit of the **state the
merge computes its base from** — and this is the series of runs that tried to make one.

## The first probe, and why it was mis-targeted

`up --validators 4 --fresh` with `--self-reset` armed; `stop devnet-validator-1`; a dev-only helper
(`node/examples/tamper-fringe-claim.rs`) run in a sidecar container over the stopped node's volume,
rewriting the **newest block's** `fringe_state_hash`; restart; 90 s.

```
tampered the claim of block bd4f5648… (height 105, true claim 0e5751c0…)
```

**Nothing happened** — heights advanced, every marker zero. And the printed true claim, `0e5751c0…`, is the
**empty state hash**: on a fresh net *nothing has finalised*, every block's fringe is `∅`, and the merge's
base then comes from a different field entirely — `prev_fringe.is_empty()` → *"take the newest
justification's state"* → `post_state_hash` from the block **body** (`multi_parent_casper.rs:334-338`). The
`fringe_state_hash` tampered is read only in the **other** branch (`:302`). So the probe was mis-targeted,
which corrected the helper rather than closing the route: it now writes **both** fields, at **every block of
the store's greatest height** (the merge selects the greatest `(height, hash)` justification, and the
greatest-height block may be a peer's, which this store also holds).

## The corrected series — four runs, and one of them proves the mechanism

### Run A — every node tampered: **the chain stalls**

All four volumes' greatest-height blocks tampered. The net stopped producing, and every node logged, every
two seconds:

```
ERROR [coop.rchain.node.instances.ProposerInstance] propose failed with an internal error:
      active-bonds leaf not found in state — no block was produced, so the chain will not advance
      until this is fixed
```

**That error string is the merge's own documented consequence of a wrong base**
(`multi_parent_casper.rs:248-252`): *"the base came out one step early: the empty state, early enough to
carry no bonds leaf, and the chain stalled at height 20 with `propose failed … active-bonds leaf not found
in state`"*. So **the tamper works** — it reaches the computation the trigger is about. It stalls the
*proposer* rather than producing a `Divergence`, because with every node tampered nobody publishes a block
and so no peer block is ever *validated* — and validation is where a `Divergence` is recorded.

### Runs B and C — one node, then the three validators: **absorbed**

Run B tampered `devnet-validator-1` alone; run C tampered the three armed validators and left the bootstrap
**clean** so it would keep publishing blocks justifying the tampered tips. Both advanced normally
(heights 197–230, 206–230) with **zero** markers on every node and `unrestorableRecords` 0 throughout.

## The finding, and it is about the trigger rather than about the drill

The tamper is effective (Run A) and the mechanism is sound; **the targeting is what cannot be held.**
Staging the trigger needs the tampered block to be the **greatest `(height, hash)` justification of a block
the subject *validates*** — and on a live net the target moves every block, while a freeze-and-restart
cannot pin it: the snapshot's max is already below the net's (Run B), and a node re-fetching the block
overwrites the tampered body with the true one (Run C).

**The conclusion this plan exists to reach, stated plainly: the reset's trigger is not reachable by an
external instrument at all, and that is by design.** A `Divergence` must be *intrinsic* — the node's own
state or replay against a block everyone else accepts — so any difference applied from outside is either
overwritten by the re-sync or is not the block the merge selects. The narrowness is the *feature*: a trigger
loose enough for an external edit would also fire on a healthy node, and the reset is destructive. It
follows that the arm can only be staged by an **intrinsic** divergence — the perturbed *build* — and that
route, as `#345` recorded, **exercises the re-entry without satisfying F-U5-01's falsifier**, because a
code-diverged node rewinds and fails again rather than healing.

**Where this leaves Phase 1.** One route **closed with a measurement** (`e1-partition.md`); one route
**probed six ways, its mechanism proven effective and its targeting shown unable to hold** (this document).
F-U5-01 remains **unrun**, with the reason now measured rather than assumed: no external edit can stage it.
