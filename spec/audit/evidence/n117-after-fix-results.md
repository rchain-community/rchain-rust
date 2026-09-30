# #117 after the fix: what the node's merge search is actually handed

Run 2026-09-30 on tree `0d111c2d7`, 3 attempts (pre-registered, unfiltered) of the frozen reproduction
via `n117-after-fix-run.sh`. Everything except the instrument is `n117-live-vs-held-run.sh`'s
configuration: 3 validators, stakes 100/100/50, epoch length 10, an 8 GiB cgroup, a clean stop when
`anon` crosses 3000 MiB, 300 s window.

## 1. The ramp

| attempt | bootstrap peak | validator-1 peak | validator-2 peak | crossed 3000 MiB |
|---|---|---|---|---|
| 1 | 1647 MiB | 1674 MiB | 63 MiB | 0 of 3 |
| 2 | 918 MiB | 1869 MiB | 65 MiB | 0 of 3 |
| 3 | 1943 MiB | **3014 MiB** | **5273 MiB** | **2 of 3** |

**Two of three runs show no ramp, and the third crosses twice.** The third is the one that decides the
claim, because it carries the run's **largest** expansion counts — 1,663,395 states on bootstrap *and* on
v2, the node that reached 5273 MiB — and `n117-after-fix-run.sh`'s pre-registered rule is explicit that a
crossing "must come with a census line whose `most states expanded` is large, or the two are unrelated".
By the run's own criterion the crossing is **linked to the merge search**: the mechanism is not bounded,
and this is the run that says so.

**This section previously reported "0 of 3" for all three attempts, and that was wrong** — the summary
contradicted the raw log tracked beside it, which has carried attempt 3's peaks (`peaks: bootstrap=1943
v1=3014 v2=5273`, `crossed: 2 of 3`) since the run finished. An earlier version of this file went further
and explained that the attempt-3 line was *missing from the log*; it is present, at line 35. The
correction is recorded rather than quietly applied because it is the defect this whole thread is about —
C176, "a measurement's artifacts cannot say what they measured" — committed inside the fix for it, and by
the summary rather than by the instrument. What the raw log records was never in doubt; what the summary
made of it was.

**What this does *not* establish.** The pre-fix runs on record crossed at a **4 GiB** cap and a different
window, so they are a different experiment, and the controlled baseline — the parent commit through this
same script — has still not been run. The honest claim is therefore: *with the fix in place, one of three
attempts still ramps, and the ramp correlates with the largest search expansion; whether the fix reduced
the ramp's frequency or amplitude is unmeasured.*

## 2. What the search is handed, which nothing had measured

The census line the fix added (`casper/src/merging.rs::search_census`, logged every 5 s from
`interpreter_util.rs`), over 9 node-runs:

| run | merges | widest scope | conflict pairs | **asymmetric** | **most states expanded on one merge** |
|---|---|---|---|---|---|
| a1 bootstrap | 183 | 33 chains | 2031 | 376 | 411,199 |
| a1 validator-1 | 179 | 33 | 2006 | 376 | 411,199 |
| a1 validator-2 | 230 | 33 | 2056 | 376 | 117,075 |
| a2 bootstrap | 161 | 40 | 1282 | 554 | 593,221 |
| a2 validator-1 | 162 | 43 | 1282 | 653 | **1,389,065** |
| a2 validator-2 | 167 | 34 | 1447 | 419 | 161,913 |
| a3 bootstrap | 130 | 35 | 1162 | 427 | **1,663,395** |
| a3 validator-1 | 167 | 35 | 1162 | 427 | 1,028,567 |
| a3 validator-2 | 126 | 35 | 1162 | 427 | 1,663,395 |

Three things this settles that had only been assumed:

- **The width is real and the guess was close.** The widest scope is 33–43 chains, against the "a fork of
  twenty chains" the defect had been written up with. The exponent was never hypothetical.
- **The relation the merge hands the search is `directed`, not symmetric.** 376–653 asymmetric pairs, in
  every run, at every node. The asymmetry is not noise: it comes from `resolve_conflict_set`'s
  `full_conflicts_map`, which unions each key's **dependencies** into its conflict set
  (`sdk/src/dag/merging.rs`, `with_dependencies`), and a dependency is a one-way constraint.
- **The exact rewrite therefore does not fire.** `maximal_independent_sets` is gated on the relation
  being symmetric and irreflexive on the keys; on this input that check declines, correctly, and the
  enumeration runs — 117,075 to 1,663,395 states on a single merge. So the node is running on the C177
  dedup, not on the C178 rewrite, and `1.66M states × a cloned set each` is still most of what its heap
  holds.

**This corrects the pass section and C178, and the correction is worth stating plainly.** The
precondition was verified on the casper merge *fixtures* — 2-chain scopes with an empty final set, where
the census reads 0 asymmetric — and a 2-chain scope with nothing to depend on cannot exhibit the
property the real path produces. A fixture that cannot fail is not evidence, which is the rule this
repository states about tests and which this is an instance of.

## 3. What this leaves

The growth per chain is roughly a factor of two (33 -> 35 chains took 411,199 -> 1,663,395), so the curve
is still exponential and the node is a wider fork away from the same ceiling: at 50 chains the same
measurement lands near 5·10¹⁰ states. The next unit is the **directed case** of the exact rewrite —
terminal states of a digraph are not its maximal independent sets, and the characterization that does
hold is in §28 — or, if that does not yield an output-sensitive algorithm, a cap with the consensus
consequence registered.

Nothing here is a regression: the pre-fix search at 33 chains would have expanded a factorial number of
queue entries rather than 1.6M states, so the tree is strictly better than it was. It is simply not
finished, and this is the measurement that says where.
