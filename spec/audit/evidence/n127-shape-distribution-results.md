# #127 Stage 1 — the shape distribution: what the run gives, and what the instrument does not

Run 2026-09-30 on tree `18ab4e4af`, image `sha256:c2e56e912d629…`, 3 attempts of the frozen reproduction,
8 GiB cgroup, 300 s window, clean-stop threshold disabled so the sample is not truncated (the protocol and
its acceptance rows are `n127-shape-distribution-preregistration.md`, frozen before the run). Nine node-runs
scraped from `/metrics` while all three nodes were alive — **3 of 3 up in every attempt**, no voids. The raw
scrapes are `shape-<node>-a<attempt>.txt`, each carrying its own configuration header.

## 1. What is valid: the envelope, and determinism on the same history

| attempt | node | merges | widest scope | most conflict pairs | most asymmetric | most states expanded |
|---|---|---|---|---|---|---|
| 1 | bootstrap | 282 | 27 | 3536 | 247 | 13,443 |
| 1 | validator-1 | 222 | 28 | 2562 | 259 | 20,503 |
| 1 | validator-2 | 180 | 32 | 2660 | 370 | 61,511 |
| 2 | bootstrap | 193 | **39** | 1191 | 391 | **364,009** |
| 2 | validator-1 | 190 | **39** | 1183 | 391 | **364,009** |
| 2 | validator-2 | 296 | 8 | 1191 | 15 | 33 |
| 3 | bootstrap | 190 | **43** | 1155 | 459 | **899,236** |
| 3 | validator-1 | 187 | **43** | 1155 | 459 | **899,236** |
| 3 | validator-2 | 294 | 5 | 740 | 3 | 9 |

**Determinism, on the nodes that saw the same blocks.** In attempts 2 and 3, bootstrap and validator-1 report
**identical** maxima — 39 chains / 1191 pairs / 391 asymmetric / 364,009 states, then 43 / 1155 / 459 /
899,236 — from independently computed statistics. That is the pre-registration's first acceptance row met on
the strongest available evidence, and it is stronger than the bucket comparison the row asked for, because
the maxima are exact numbers rather than counts in a bucket.

**And validator-2 is not lagging, it is elsewhere.** It reports *more* merges (296, 294) with a *much
smaller* shape (widest scope 8 and 5 chains; 33 and 9 states). Two runs in three, the third node resolved a
long flat chain while the other two resolved a wide one — the same split the census run showed, where
validator-2 sat at 63 MiB while the others climbed. This is recorded rather than smoothed: a distribution
collected from one node is not the network's, and the divergence itself is the `Void`/`Drift` family #126
owns rather than a Stage 1 concern.

**The widest scope this quiet devnet reached is 43 chains** — the same width the census run's storm reached —
and it cost **899,236 states expanded** in one merge. That is Stage 2's territory: on an unloaded chain, one
merge in a 300 s window already paid ~9·10⁵ states.

## 2. What is broken: my own histogram, and precisely how

**The distribution's shape is not readable, and the fault is in the instrument, not the run.** The scraped
buckets are:

```
rchain_merge_scope_width_bucket{le="0.005"} 0    …    {le="7.5"} 0
rchain_merge_scope_width_bucket{le="10.0"} 0
rchain_merge_scope_width_bucket{le="+Inf"} 282
```

Those boundaries — 0.005 … 7.5, 10.0 — are **the registry's defaults**, not the 16/32/64/128 edges
`search_census::WIDTH_EDGES` defines. `MetricsRegistry::record` resolves its own bucket configuration
(`scrape_data_builder.rs`), and every width recorded is ≥ 5, so every sample lands in `+Inf` and the
distribution collapses to its total. **The failure is exactly the class this pass exists to name** — an
instrument that cannot say what it measured — and it is worth stating that it was found by *reading the
artifact*, not by trusting the code that produced it.

**And the same defect invalidates the mean.** The publish records the bucket's *edge* as the value
(`record(&source, "scope_width", edge, delta)`), because `record` takes a value and a count and there is no
bucket-label primitive. So `rchain_merge_scope_width_sum / _count` is a mean **edge**, not a mean width — it
takes the values 16 and 32 and nothing between. An earlier draft of this file computed "mean width 30.2 /
19.1 / 19.0 / 16.0" from it and drew a conclusion from the number; that reading is withdrawn here rather
than left to be found. What the sums *do* support is coarse: most merges in attempts 2 and 3 sat at or below
the 16-chain edge, and attempt 1 was a mix at or below 32.

## 3. The acceptance rows, evaluated as frozen

| row as frozen | verdict |
|---|---|
| the three nodes' `_count` totals agree and their buckets agree → deterministic | **partly met, and better evidence than the row asked for**: the totals do *not* agree (282/222/180, 193/190/296, 190/187/294) so the first half fails, but the two nodes on the same history report **identical maxima**, which is the determinism the row is about |
| the totals differ and the buckets agree conditional on the total → lag | **not decidable as written**: the buckets are the registry's, so "the buckets agree" tests nothing. On the evidence that *is* readable, validator-2 is a different history, not a lagging one |
| the nodes agree on `_count` and disagree on the buckets → the instrument is not deterministic | not reached |
| every bucket but the first is empty → the run never reached a wide scope | **this is what the artifact shows, and for the wrong reason**: the buckets are empty because they are the registry's, not because the scopes were narrow. The envelope says the opposite — 43 chains wide, 899,236 states |

**So the run's verdict is: the envelope and cross-node determinism are measured; the distribution is not,
and cannot be with this instrument.** C182 stays `in progress` and its `owes` gains two named defects rather
than a re-run.

## 4. What Stage 2 needs from the fix

The threshold **N** and the fee schedule are read off a distribution, so the fix is a prerequisite rather
than a follow-up: record the **actual width** per merge (a value the registry can bucket itself), or give the
registry a bucket configuration that covers 1–256 chains, or publish the cumulative buckets as gauges
(`scope_width_le_16`, …) the way Prometheus histograms are actually consumed. Any of the three makes the
distribution readable; the first is the smallest change and the most informative, and it also makes
`_sum/_count` a true mean.

**What the run already gives Stage 2, for the record:** the widest scope in a *quiet* 300 s window is 43
chains and ~9·10⁵ states; the median sits near or below the 16-chain edge (coarsely, and to be re-measured);
and the maximum is identical across nodes on the same history, so a threshold keyed on width is a quantity
two nodes compute the same way. The census run's storm reached 1,663,395 states at 35 chains — so the
*load* moves the cost more than the width does, which is itself an argument for measuring the distribution
under load before setting N.
