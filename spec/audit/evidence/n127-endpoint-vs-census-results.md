# Unit 2 — the endpoint now carries the census, measured

Run 2026-09-30 on tree `5187dbdbe`, image built 18:29:12 from that tree. Three validators at
100/100/50, an 8 GiB cgroup, a 90 s window, then **200 scrapes** of `/metrics` on the bootstrap, then the
node's own census line for the same node. The artifacts are
`spec/audit/evidence/n127-endpoint/5187dbdbe-20260930T173236Z/`; the harness is
`n127-endpoint-vs-census-run.sh`, which exists because the previous campaign compared nothing.

## The result

```
census      : 93 merges, width buckets [68, 18, 7, 0, 0] -> cumulative [68, 86, 93, 93, 93]
endpoint    : count 93.0    (first scrape 92.0)
  200 scrapes, endpoint count 93.0 — the defect scaled with this number
    le=16.0    endpoint 68       census 68       ok
    le=32.0    endpoint 86       census 86       ok
    le=64.0    endpoint 93       census 93       ok
    le=128.0   endpoint 93       census 93       ok
  count vs census: 93.0 vs 93 -> ok
```

**Every boundary agrees, and the count is the census's count after two hundred scrapes.** The previous
campaign read `_count 7617` against `101 merges` from the same node at the same instant; the factor was the
scrape count, and the sampler doing the scraping was the measurement's own. The check that would have
caught it — the endpoint beside the census, one quantity rendered twice — is this one, and it is now the
close condition rather than an omission.

The first scrape reads 92 because one more merge landed during the loop. That is the registry growing, not
the endpoint drifting; a live counter's first and last scrapes *should* differ by what happened in between.
What the defect did was different in kind: it multiplied the count by the number of requests.

## What was wrong, in two parts, and what each fix was

**It could not produce a distribution at all.** `HistogramAcc` was `{count, sum, min, max}` — **no bucket
map** — so `snapshot()` emitted exactly one `Bucket { value: h.max, frequency: h.count }`. Every
`_bucket{le=…}` line the endpoint rendered was therefore a function of the **running maximum** and of when
someone happened to scrape. This is the deeper of the two defects and the reason the published distribution
was not a rescaling of the truth: the low bucket read 13 against a census of 38, which no divisor explains.
The fix is a `value -> frequency` map in the accumulator, populated by `record` and emitted by `snapshot`.

**The trigger accumulated.** `report_period_snapshot` merges into a five-year accumulator — correct for
kamon, whose instruments report a *period's* worth and whose `/metrics` route returns a cached string that a
periodic reporter refreshes. The port wired the endpoint straight into it, so every request added the
running totals to themselves. The fix is at the **call site**, not in the accumulator: `/metrics` now calls
a `render` that builds scrape data from the snapshot directly, leaving the ported accumulator and its own
tests untouched. That is where the port changed the oracle's meaning, so that is where the correction goes.

**Both were invisible because every histogram fixture in the tree recorded exactly one sample** — and a
one-sample distribution is rendered correctly by a single-bucket accumulator. The guard is
`a_histogram_renders_every_observation_it_recorded`: two values in, two buckets out, the lower one not the
top.

## The third thing, which is why the first two were findable

The endpoint's bucket boundaries were a **hand-written superset** (`8,16,32,64,128,256`) of the census's
(`WIDTH_EDGES = [16,32,64,128]`), and the registry's test asserted only that the configured set contained
*some* edge at or above 64 — which passes for the right list, for `[64]`, and for any superset. Nothing
compared the two renderings, so nothing could notice them drifting, and they had drifted: the census
publishes its open-ended bucket *at* its last edge, so a width of 200 and a width of 128 both arrive as
`128`, which is the value the configured list must contain for the two to agree. The boundary list is now
**derived** from `search_census::{WIDTH_EDGES, EXPANDED_EDGES}`, and the test asserts equality with those
constants rather than membership.

## What this closes, and what it does not

C182's third defect is fixed and its close condition is now runnable. **What it does not do is produce N**:
the endpoint is trustworthy for measuring the distribution, and the distribution still has to be measured
on the arm that matters — C171's own (`--no-autopropose --propose-on-deploy`), which the previous campaign
did not run. Nothing here bounds or prices anything.
