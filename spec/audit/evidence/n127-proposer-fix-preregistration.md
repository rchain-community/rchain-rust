# Does finality advance at all once the proposer justifies the round snapshot? — pre-registered

**Status: FROZEN before the run.**

## Why this is the measurement

C185's defect was found in the **proposer**, not in the gate, the arithmetic or the stake split: a block
justified every sender's newest message, and `calculate_next_fringe_support_map` reads `parents ∖ next_layer`
— so the head of each round had an empty remainder and the later movers had seen a prefix, and no candidate
was ever a full partition. In-process, through the production entry point with three live validators at the
devnet's own `100/100/50`, that refused with `Support { supporting: 0, total: 250, full_partitions: 0,
candidates: 2 }`.

The fix is `DagMessageState::round_parents`: a new block justifies the `latest_msgs` as of the last round
boundary. In-process the same chain now publishes a fringe and tracks the tip at a constant 12-height lag
(48/60, 168/180, 348/360). **That is a fixture, not a node.** This run is the node.

The rig, the perturbation and the kill offset are **identical to `n127-liveness-preregistration.md`**, so
this arm is comparable with both earlier arms rather than being a new configuration.

## The reproduction (fixed, and identical to the campaign's)

Three validators, `--stakes 100,100,50 --epoch-length 10 --fresh`, devnet defaults (autopropose on,
propose-on-deploy on, attest-on-new-blocks on), an 8 GiB cgroup, a 300 s window, four deploys at T+30 s, and
`tools/devnet.sh stop 2` (the 50-stake validator) at T+120 s. ≥ 3 attempts, unfiltered, one devnet at a
time. Artifacts under `target/n127-liveness/<tree>-<utc>/`, every file carrying the tree.

## The acceptance rows, frozen

The primary row is **0.1**, and it is deliberately the one the two earlier arms *failed*: they logged
finality 14–17 in the first 120 s and then never moved again, so this arm fails if it repeats that.

| # | observation | verdict |
|---|---|---|
| **0.1** | **finality advances with the height across the pre-kill window**: every survivor's finalised height at T+120 exceeds the 14–17 the two earlier arms reached, and its *last advance* is near T+120 rather than inside the first 35 s | **the pin is gone** — the proposer's parent set was the cause |
| | finality still stops inside the first 40 s and pins | **the fix does not work on the node**: the fixture passes and this rig does not, which is a finding about a *second* stop, and the stall line's variant and numbers say which |
| **0.2** | the survivors' finality keeps advancing after the kill at T+120, in every attempt | the pre-registered row of `n127-liveness-preregistration.md`, now expected to pass rather than fail |
| **0.3** | the `0 of 250` lines are gone, or carry a different support value | the gate is being reached at all, which is the thing the parent set was preventing |
| **0.4** | `SearchBudgetExceeded` does not appear anywhere | the C184 control, unchanged: these are the honest scopes the budget must not touch |

**The single number to report:** per node per attempt, the finalised height at T+120 and at end, the height
at the same instants, and **the instant finality last moved** — the quantity neither earlier arm computed,
and the one that would have shown the pin immediately.

## What this does not settle

- It is three attempts of one configuration on one machine. A fix that holds here is the measurement the
  register asks for before the claim is written down, not a proof.
- It is not a claim about safety: it changes which blocks a node *makes*. An unpatched node accepts a
  patched node's blocks — §6 carries that classification — but this run does not test a mixed network, and
  a mixed network is where a proposer divergence would show.
- It does not re-measure the merged-shape distribution (C182), and it says nothing about N.
