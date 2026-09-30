# #117 — what the live memory is

**The node's memory ceiling is the merge's conflict-resolution search.** A heap profile taken *at* the
ceiling attributes **97 % of the live heap** to `BTreeSet`/`BTreeMap` clones inside
`rchain_sdk::dag::merging::resolve_conflict_set`, reached from `MergeScope::merge` via
`get_pre_state_for_parents`.

## How the profile was taken

`spec/audit/evidence/n117-heap-profile-run.sh` — the frozen reproduction (3 validators, `100,100,50`,
epoch 10, fresh), an 8 GiB cgroup, jemalloc's sampling profiler built in (`profiling`), and the
`LD_PRELOAD` shim calling `prof.dump` **the moment the node's own cgroup `anon` crosses a threshold**, at
1.5 GiB and every 1.5 GiB above.

The trigger is the whole point. jemalloc's `prof_final` and `stats_print` both write at process **exit**,
after the node has freed its world — measured: a node stopped cleanly at 3258 MiB of `anon` printed
`Allocated: 2.9 MiB`. Only a dump taken *while the node is running* reads the live set at the ceiling.

## Result — one dump, at 4.6 GiB of live bytes

| live | share | stack (innermost first) |
|---|---|---|
| 1488 MiB | 37.2 % | `BTreeMap::clone::clone_subtree` |
| 1140 MiB | 28.5 % | `…clone_subtree` ← `resolve_conflict_set` |
| 563 MiB | 14.1 % | `…clone_subtree` |
| 551 MiB | 13.8 % | `…clone_subtree` ← `resolve_conflict_set` |
| 93 MiB | 2.3 % | `…clone_subtree` ← `resolve_conflict_set` |
| 78 MiB | 1.9 % | `…clone_subtree` |
| 46 MiB | 1.1 % | `…clone_subtree` ← `resolve_conflict_set` |
| 39 MiB | 1.0 % | `…clone_subtree` |
| 4 MiB | 0.1 % | `rchain_rspace::merger::seq_diff` (everything else) |

`BTreeSet<T>` wraps `BTreeMap<T, ()>`, so a set clone is reported as a map clone. The three dumps taken
during the run — at 1.5, 3.0 and 4.6 GiB of total live — put this site at 343, 881 and 1488 MiB, so it
accumulates with the node rather than spiking once.

## The mechanism, and a falsifier that runs in seconds

`resolve_conflict_set` calls `compute_rejection_options`, which is the Scala `computeRejectionOptions`
`O(2^n)` search ported literally: a breadth-first enumeration over acceptance orders, whose queue entries
each carry two cloned sets. It is exponential in the number of **forked** chains — chains within a branch
do not conflict while chains across branches conflict completely, so the search enumerates every subset of
a branch: 2^(n/2) states, each holding two cloned sets.

`sdk/tests/merging_scaling.rs` reproduces it with no devnet, as a falsifier that was `#[ignore]`d while the
search was unbounded — **both figures below are pre-fix** and the falsifier is a gate now; see the note at
the end of this file:

- two branches of 10 chains (**20 total**) — **11.3 seconds**;
- two branches of 20 (**40 total**) — 2^20 states, which is the size a devnet fork reaches.

The dense case is *not* the problem: accepting any chain rejects all the others, so it terminates in one
step. That is why the existing 1000-node full-graph oracle passes while the node grows to gigabytes.

## A rewrite was attempted, is wrong, and the counter-example is kept

The obvious fix — compute the least fixed point of `rejected ⊇ ⋃{conflicts(j) : j ∉ rejected}` directly,
since every key not rejected is eventually accepted — **over-approximates**. A key that has been *rejected*
is never accepted afterwards. For `{0: {1}, 1: {0}, 2: {}}` the search yields `{{0}, {1}}` while the
closure also yields `{0, 1}`. `rejection_options_are_not_the_closure` pins that, so the next attempt meets
it first.

The search is left exactly as it was *by this attempt*, and that sentence is history: C177 and C178
changed the enumeration afterwards, and the change was held to the rule in the second sentence — a
differential proof against the enumeration, not an argument about it. See the note at the end of this
file. The merge outcome is consensus-visible (law 17).

## Why this is not simply "make the search polynomial"

**Refuted, in the symmetric case, by what landed.** This section argues the fix space is a cap or a bound
on the conflict-set size and *not* a smarter enumeration of the same function. C178 found the smarter
enumeration: when the conflict relation is symmetric and irreflexive on its keys, the terminal states are
exactly the maximal independent sets, and Bron–Kerbosch enumerates those without visiting the intermediate
states (40 chains: 2^21 states → 60). The paragraph below is kept because its *premise* survives — the
output can be exponential in its own right, and the node's relation is directed, where the rewrite
declines — but its conclusion does not.

The output of the search is the set `{ C(A) : A a realisable accepted set }`, and enumerating realisable
accepted sets is the same problem as enumerating the maximal independent sets of the conflict graph — #P-
hard in general, and exponential *in the output* as well as in the search. The fork shape is the awkward
one: it has only two outcomes while the search visits 2^(n/2) states to find them. So the fix space is a
deterministic bound (a cap, with a fixed fallback — the same on every node, so consensus holds, but a
registered deviation from the Scala's exact search), or a bound on the conflict-set size at the source, not
a smarter enumeration of the same function.

## What the fixes did to these numbers, and what they did not

Added when this file was reviewed for the merge, because everything above describes the search as it was
*before* C177 and C178. Two things changed and one did not:

    pre-fix                    20 chains: 19,728,200 queue entries for 2,046 states (11.3 s)
    after C177 (the dedup)     20 chains: 2,046 states expanded             (13.4 ms)
    after C178 (symmetric)     20 chains: 30 states (0.17 ms), 40 chains 60 states (0.7 ms)
    this file's own falsifier  three plain `#[test]`s, no `#[ignore]`, 200 chains 300 states / 22 ms

**What did *not* change is the reason #117 stays open.** The rewrite C178 landed applies when the conflict
relation is symmetric and irreflexive *on its keys*; `resolve_conflict_set` unions each key's dependencies
into its conflict set, which makes the real relation **directed**. The census the fix added is what
measured that, on the frozen reproduction, 9 node-runs: 376–653 asymmetric pairs among keys at the 33–43
chain widths the devnet reaches, and up to **1,663,395 states expanded on a single merge**, ~2× per chain.
So the node runs on the dedup alone. The devnet numbers and the correction are in
`n117-after-fix-results.md`; the register row is C178, `in progress`.
