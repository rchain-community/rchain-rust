# The reset arm, attempted — the named staging route does not reproduce

**What this is.** §106 and §107 both list the **devnet arm** as owed: *"a diverged running node re-enters
sync"* — F-U5-01's falsifier, the acceptance-level instrument for the self-reset (R3). §101 recorded it as
**stageable but unrun**: *"now stageable with `up --validators 4 --fresh` (the §92 self-reject divergence
needs no instrument), where §100 recorded it blocked behind the retired injection, and unrun."* This is the
attempt, and its result is **negative**, which is why it is written down.

**The run.** One host, one run, 2026-10-13.

```
tools/devnet.sh build                       # image rnode:local, commit 731ac4b1c (this tree)
tools/devnet.sh down -v
DEVNET_EXTRA_FLAGS_devnet_validator_{1,2,3}="--self-reset" \
  tools/devnet.sh up --validators 4 --fresh
```

`--validators 4` is the **bootstrap plus three** (`tools/devnet.sh`'s loop is `i < n`; the bootstrap is the
fourth bonded validator), so the net is the four nodes §92's capture had. `--self-reset` was set on the
three validators and **confirmed present on each container** (`docker inspect … .Config.Cmd`); the
bootstrap was left without it, so the arm is armed on three of four.

**What was observed.** `observe.log` samples every node's `/api/v1/status` every 15 s from 12:12:23 to
12:20:29 — **~9 minutes**, during which the heights ran from `boot 98 · v1 149 · v2 148 · v3 144` to
`boot 270 · v1 277 · v2 275 · v3 271`. Across the whole run, on every node:

| marker | count |
|---|---|
| `[fringe-divergence]` | 0 |
| `Self-created block …` | 0 |
| `reset requested` / `was rewinding to` / `rewound to` | 0 |

and `consecutiveSelfValidationFailures` stayed **0**, `autoproposeTimerHalted` **false**,
`unrestorableRecords` **0** on all four throughout. The net produced ~170 blocks and stayed healthy.

**Why it did not reproduce, and this is the finding.** §92's self-reject was produced by the **fringe-record
store's `min` tie-break**: two honest blocks finalising one fringe from two bases wrote the same key, the
`min` chose a `state_hash`, and the node then refused its own block against the state that value implied.
That store is **gone** — law 66/67/68's record redesign retired `join_fringe_records`, its `min` tie-break
and the in-memory map (`casper/src/dag.rs:812` records the deletion in the `insert` comment;
`models/src/block_metadata.rs:464` and `block-storage/src/test_support.rs:65` the type). The
`[fringe-divergence]` string survives, but as a **reader-side report** of "N distinct claims"
(`casper/src/multi_parent_casper.rs:281-292`) rather than a store write. So the route §101 named stages
nothing on this tree: **the divergence §92 relied on was fixed by the record retirement, and the arm needs a
new instrument rather than a longer run.**

**What this does not claim.** It does not say the reset is untested — its durable half, its boot guard and
its ordering are pinned by `casper/tests/reset.rs` and run by the formal gate (see §106/§107). It does not
say the reset is *correct* either: the property this arm exists to measure, **a diverged running node
re-enters sync**, remains **unrun**, and now has no staging route recorded. That is the residue, stated
rather than implied.

**The files.** `observe.log` (the sampler's output, as written), and the four containers' tails are captured
beside this run; the container tails are ~400 lines per node and are not committed, because the run produced
no fault to point at — the **absence** is the result, and `observe.log`'s zeroes are the evidence for it. The
devnet was torn down with `tools/devnet.sh down -v` after the capture.
