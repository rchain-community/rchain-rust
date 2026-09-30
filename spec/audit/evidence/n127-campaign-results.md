# Phase 0 — the campaign: what the rig gave, and the third defect it found in the instrument

Run 2026-09-30 on tree `c5442ee1f`, image `sha256:783fe97d6054…` (built 17:10:49 from this tree; the
manifest's `rust_diff_vs_1732306c7` reads **empty**, so the binary under test is the tree the artifacts
name). Three attempts of the reproduction frozen in `n127-campaign-preregistration.md`, 8 GiB cgroup, 300 s
window, four deploys at T+30 s, `validator-2` stopped at T+120 s. **3 of 3 up in every attempt**, no voids;
artifacts in `target/n127-campaign/c5442ee1f-20260930T163518Z/`.

The short version: **0.2's acceptance row passed while the quantity it guards was wrong.** The endpoint's
histogram and the node's own census disagree by a factor of ~75, and the row I pre-registered checked the
bucket *boundaries*, which were right. The distribution is readable — from the census's log line, not from
`/metrics`. **N cannot be read from this run's endpoint**, and that is a third defect in C182's instrument
rather than a property of the chain.

## 1. The reading that works, and where it has to be read from

The node's own log line carries the census every few seconds, and it is **internally consistent**: the width
buckets sum to `MERGES`, and `widest scope` agrees with the buckets it fills.

```
2026-09-30T16:37:26.709Z WARN [casper.interpreter.validate] merge search: 101 merges · widest scope 36
  chains / 742 conflict pairs / 489 asymmetric · most states expanded on one merge 389977 ·
  width buckets [38, 55, 8, 0, 0] · cost buckets [40, 14, 34, 13, 0]
```

`38 + 55 + 8 = 101` ✓. The buckets are **raw counts per bucket** over the census's edges
`[16, 32, 64, 128, 256]`, so the cumulative distribution is recoverable exactly:

| attempt | node | merges | ≤16 | ≤32 | ≤64 | **median edge** | mean width | most states on one merge |
|---|---|---|---|---|---|---|---|---|
| 1 | bootstrap | 101 | 38 | 93 | 101 | **32** | 19.6 | 389,977 |
| 1 | v1 | 95 | 35 | 87 | 95 | **32** | 19.4 | 389,977 |
| 1 | v2 | 86 | 34 | 78 | 86 | **32** | 19.2 | 389,977 |
| 2 | bootstrap | 138 | 56 | 130 | 138 | **32** | 19.0 | **2,026,511** |
| 2 | v1 | 230 | 57 | 223 | 230 | **32** | 18.3 | 559,535 |
| 2 | v2 | 114 | 50 | 104 | 114 | **32** | 19.0 | 442,202 |
| 3 | bootstrap | 88 | 41 | 88 | 88 | **32** | 16.8 | 1,726,295 |
| 3 | v1 | 84 | 43 | 84 | 84 | **32** | 15.7 | 1,726,295 |
| 3 | v2 | 84 | 40 | 82 | 84 | **32** | 17.2 | 736,023 |

**Nine node-runs, one median bucket: 32 chains.** More than half of every node's merges have a scope of
17–32 chains; the mean is 15.7–19.6; the widest is 32–37; and no merge in any attempt exceeded 64 chains.
**The largest single merge expanded 2,026,511 states** (attempt 2, bootstrap) — **higher than the census
run's 1,663,395**, and on a chain whose block rate is *lower* than the storm's. Load is not the only thing
that moves the cost; this is the widest-scope-fewest-merges case, and it is recorded rather than smoothed.

## 2. The defect: `/metrics` does not carry the census's distribution

The same node, at the same moment (`metrics-bootstrap-a1-prekill.txt`, scraped at T+120):

```
rchain_merge_searches 100.0
rchain_merge_scope_width_total 1957.0          ← mean width 19.6, agrees with the census
rchain_merge_scope_width_bucket{le="8.0"}    0.0
rchain_merge_scope_width_bucket{le="16.0"}  13.0    ← the census says 38 merges are ≤ 16
rchain_merge_scope_width_bucket{le="32.0"} 1373.0
rchain_merge_scope_width_bucket{le="64.0"} 7617.0
rchain_merge_scope_width_count             7617.0   ← the census says 101 merges
```

Three numbers disagree, and each disagreement is a separate fact:

1. **The total is inflated ~75×.** The histogram's `_count` is 7,617 against `MERGES` = 100 in the same
   scrape and 101 in the census. The inflation differs per node (6,763 / 6,996 / 7,617), which is what a
   *per-publish* error looks like and not a per-merge one.
2. **The low bucket is *below* the census.** `le="16"` renders 13 where the census counts 38. So the
   endpoint does not merely over-count uniformly; the distribution it prints is not a rescaling of this one
   and its shape cannot be recovered by dividing by anything.
3. **The cumulative structure is not Prometheus's.** The rendered counts *are* monotonically increasing
   (0, 13, 1373, 7617, 7617, 7617), as a cumulative histogram's must be — but the census's own buckets are
   raw and sum to `MERGES`, so a correct rendering of this census would be `0, 38, 93, 101, 101, 101`. What
   is published is neither.

**Where it is, and what was ruled out.** The quantity is produced by one path and survives two independent
renderings in the artifact: the census's own `summary()` line, which is self-consistent, and `/metrics`,
which is not. So the defect is between the counters and the scrape, and the path has exactly three steps:
`take_width_deltas` hands over `(edge, delta)` pairs; `set_merge_shape_gauges`
(`casper/src/dag.rs:264-274`) records one sample per *bucket*, valued at the bucket's **edge**:

```rust
for (edge, delta) in search_census::take_width_deltas() {
    if delta > 0 { self.metrics.record(&source, "scope_width", count(edge), count(delta)); }
}
```

and the reporter merges each period's snapshot into an accumulator (`merge_distribution_into`,
`node/src/diagnostics/prometheus_reporter.rs:119-129`), whose duration is **five years**
(`NewPrometheusReporter::new`, `:145-148`) — so every snapshot it is given is merged into every previous
one rather than replacing it.

**Two candidates, and the experiment that separated them — run in-process, no devnet needed.** Either
`record` is handed pairs whose frequencies do not sum to the deltas, or the accumulator re-adds a
*cumulative* snapshot each period. A single reported snapshot already renders correctly (the existing
`a_merge_width_renders_into_the_shapes_own_buckets` asserts it), so the second was the candidate, and
`re_reporting_a_snapshot_does_not_double_a_histogram_count` (`node/src/runtime/node_runtime.rs`, ignored
until this is fixed, runnable with `cargo test -p rchain-node --lib -- --ignored re_reporting_a_snapshot`)
**proves it**: one merge, one `record`, **two** `report_period_snapshot` calls, and the endpoint renders

```
rchain_merge_scope_width_count 2.0
```

**So the mechanism is the reporter, not the census and not the publish.** `report_period_snapshot`
(`prometheus_reporter.rs:155-171`) does `accumulator.add(snapshot)` into a **five-year** window and renders
`accumulator.peek()`, and this registry's histograms are cumulative — so every reporting period adds the
running total to itself. That is exactly the observed factor: the devnet's counts are ~75× the census's, and
~75 is the number of reporting periods in a 300 s window.

**The blast radius is exactly two metrics.** Every `Metrics::record` call site in the workspace is
`casper/src/dag.rs:272` and `:283` — the two distributions this stage added. The queue depths C175
observes are gauges, which the accumulator handles by replacement, so they are unaffected. **The repair is
the next unit, not this one**: it is in the reporter's handling of cumulative histograms (or in what the
node reports per period), and it now has a failing test that names it.

**What the row that should have caught this teaches.** The pre-registered check was "the histogram's
boundaries are 16/32/64/128, not the registry's defaults" — and it **passed**, correctly: the boundaries
*are* the shape's. Then Stage 1's defect was that the *boundaries* were the registry's; fixing the
boundaries did not make the counts right, and the row that certified the fix certified the half that was
already visible. A row that guards an instrument must cross-check it against a **second, independent
rendering of the same quantity** — here, `searches` or the census's own log line — not a property of one
rendering's shape.

## 3. 0.1 — the rate, and what the kill does

Read from the series (`n117-queue-depth.py`, ~1.43 s per sample, timestamps used rather than sample
indices — the sampler is slower than one sample a second, and indexing by sample number misplaced the kill
by ~55 s in a first pass at this analysis).

**All three live, before the kill:**

| attempt | height at T+119 s | rate |
|---|---|---|
| 1 | 27 | ~13.6 blocks/min |
| 2 | 32 | ~16.1 blocks/min |
| 3 | 23 | ~11.6 blocks/min |

So the all-live chain runs at **12–16 blocks/min, one block per 4–5 s** — *slower* than the 2 s autopropose
timer, and an order of magnitude below the 276 blocks/min the storm is recorded at. **The storm did not
appear while every validator was live in this configuration**, which is not what the pre-registration
anticipated. Its table has rows for "faster than the timer" and "about the timer"; it has none for
*slower*, and that gap is a correction to the pre-registration rather than a reading.

**After the kill (T+120 → T+300), finality did not resume in 3 of 3 attempts:**

| attempt | survivor | height | finality |
|---|---|---|---|
| 1 | bootstrap | 27 → 27 (**+0**) | 14 → 14 |
| 1 | v1 | 27 → 27 (**+0**) | 14 → 14 |
| 2 | bootstrap | 32 → 34 (+2) | 17 → 17 |
| 2 | v1 | 29 → **82** (+53) | 14 → 18 |
| 3 | bootstrap | 23 → 24 (+1) | 10 → 10 |
| 3 | v1 | 23 → 24 (+1) | 10 → 10 |

Two of three attempts **freeze completely** — the height stops as well as the finality. One attempt keeps
producing at ~18 blocks/min while finality moves by **+4 heights in 180 s**. So the storm's signature
appears **after the kill, not before**, and intermittently: the chain that grows under an absent validator
is the one that runs while finality is pinned. The pre-registered row "the survivors do not resume finality
after the kill" is **met (3 of 3)**, with attempt 2's +4 recorded rather than rounded to zero.

## 4. 0.3 — void, and by construction

Every stall line in the run carries **`at tip 0`**:

```
finality did not advance at tip 0: a layer exists but its supporting stake is not a supermajority —
0 of 250 (0 full partition(s) among 0 candidate(s))
```

The line is rate-limited to **one per 100 heights** (`interpreter_util.rs:197`, `STALL_LOG_INTERVAL`), and
the tip never left the low thirties. So the genesis-time line suppressed every later one and **the
discriminator cannot be read on a chain this short** — a fact about the reading's scale, not about the
instrument. Reading it needs either a longer chain or a smaller interval; the pre-registration did not
foresee that and the results file records it instead of re-running to fit.

## 5. What this changes

- **Stage 2's N cannot come from this run**: the quantity a threshold would be keyed on is published
  wrongly, and the row that guards it was insufficient. C182's `owes` gains a **third defect** — the
  endpoint's counts — and the distribution is readable only from the census log until it is fixed.
- **The honest envelope, from the census**: median edge **32 chains**, mean **15.7–19.6**, widest **37**,
  and a single merge expanding **2,026,511 states**. Those are the numbers a threshold is weighed against,
  with the endpoint caveat attached.
- **The storm is a post-kill phenomenon here**, so Phase 1's pace term targets the state an absent
  validator creates — which is C171's own mechanism ("attesting requires proposing") and #70's second stop,
  not an all-live idle chain.
- **No bound or price is implied by any of this.** This run moved the input's *distribution* into view and
  found the instrument's third defect; it decided nothing about N.

## 6. Harness defect found and fixed in the running

The first launch of this harness sat for **13 minutes** inside `tools/devnet.sh deploy` — the deploy RPC
does not return while the node cannot include the deploy, and the harness had no bound on it. Every deploy
is now `timeout`-bounded (45 s) and the count that returns is written to `deploys-a<N>.txt`; the kill is
scheduled on an **absolute** offset from the attempt's zero, so a slow deploy cannot move it (and it prints
the offset it actually used — `T+121s (target 120s)` in every attempt of the run above). The first launch's
artifacts are kept at `target/n127-campaign/4ff7dff0c-…` as the record of that failure.
