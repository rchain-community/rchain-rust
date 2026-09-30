# #127 Stage 1 — the shape distribution, pre-registered

**Status: FROZEN before the run.** The point of this file is that the verdict cannot be rationalised after
the fact; the configuration and the acceptance rows below were fixed while no result was known. A later
change is a new commit that says why it was necessary and what invalidated the original.

## What is measured, and what it is for

Not the failure — the *distribution*. C182's instrument publishes the merge search's scope width as a
histogram (`rchain_merge_scope_width`, edges 16/32/64/128 plus an open-ended bucket) beside the envelope as
gauges. This run reads that off each node while it is alive, and its output is the input to two numbers the
change order cannot invent: **Stage 2's threshold N** and **Stage 3's fee schedule**. The record of the
change order is `spec/audit/passes.md` §30.

## The reproduction (fixed)

Three validators, `--stakes 100,100,50 --epoch-length 10 --fresh`, an 8 GiB cgroup with swap off, the node
built from the tree under test. The window is 300 s and **no clean-stop threshold is set** (`THRESHOLD_MB`
above the cgroup cap): the census run stopped a node the moment it crossed 3000 MiB, which is right for
measuring a ramp and wrong for collecting a distribution — it truncates the widest scopes out of the
sample. The run is one devnet at a time, as always.

Read from each node's own `/metrics`, at the end of the window and **before** teardown (a container's
`/metrics` is gone with the container):

- `rchain_merge_scope_width` — the histogram: bucket counts, `_sum`, `_count`;
- `rchain_merge_searches`, `rchain_merge_max_scope_width`, `rchain_merge_max_conflict_pairs`,
  `rchain_merge_max_asymmetric_pairs`, `rchain_merge_max_states_expanded` — the envelope.

The configuration, the tree, and the image id go into the header of every artifact, per C176.

## The acceptance rows, frozen

| observation | verdict | consequence |
|---|---|---|
| the three nodes' `_count` totals agree (same merges) **and** their bucket counts agree | **the instrument is deterministic across nodes** | Stage 2 may key its threshold on this quantity |
| the totals differ, and each node's buckets agree *conditional on its total* | **lag, not disagreement** | the instrument holds; the run had a node behind, which is recorded, not smoothed |
| the nodes agree on `_count` and disagree on the buckets | **the instrument is not deterministic** | C182 reopens: nothing may be gated on a quantity two nodes compute differently |
| every bucket but the first is empty | **the run never reached a wide scope** | no threshold can be chosen from it; the run is void for Stage 2's purpose and says so |

**The single number to report:** the width bucket at which the sample's mass sits — the smallest edge whose
cumulative count is at least half the merges (`median_scope_width`), beside the widest scope the run
reached. A distribution is reported *with* its envelope, never as one number.

**Repeats:** ≥ 3 attempts, unfiltered. The phenomenon is intermittent — the census run showed 0 of 3 nodes
crossing twice and 2 of 3 the third time, in the attempt with the largest expansion counts — so one attempt
decides nothing about the *shape* either.

## What this does not settle

- It does not bound anything, and it does not price anything: it produces the input to those decisions.
- It does not tie the width to the *cost*. Whether the width predicts the state count is a separate
  measurement, and the tree has no cheap general bound to assert it with — only the four closed forms
  `compute_rejection_options`'s doc comment pins.
- It does not measure the proposer's view: every node reports what *it* merged, which is the same set of
  blocks only when no node is behind. That is why the lag row above exists rather than a tolerance.
- It cannot see a scope the node never had to resolve, so a distribution collected on a quiet chain
  understates what a flooded one would produce. The flood is C175/C171's territory, not this run's.
