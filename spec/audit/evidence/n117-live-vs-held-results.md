# #117 — is the memory at the ceiling live or held?

**Verdict: LIVE. Nine of nine nodes.** The bytes reaching the cgroup ceiling are handed out by `alloc` and
not yet returned by `dealloc`. They are not retention, not purge lag, and not allocator configuration —
jemalloc accounts for 97–99 % of the cgroup's anonymous memory, so there is no un-attributed remainder.

This matters because it decides the fix space: a live peak is a structure in the node, and the next unit is
a heap profile rather than an allocator knob.

## Method

`spec/audit/evidence/n117-live-vs-held-run.sh` — the frozen reproduction (3 validators, `100,100,50`,
epoch 10, fresh, no external load), an 8 GiB cgroup with swap off, no runtime allocator override, three
attempts fixed in advance and reported unfiltered. One instrument is added:
`jemalloc-stats-shim.c`, `LD_PRELOAD`, 1 Hz, reading `stats.allocated` / `active` / `resident` / `mapped` /
`retained` through `mallctl` from inside the node, alongside the cgroup's `memory.stat` by an independent
sampler. The two 1 Hz series are joined on wall-clock time by `align-live-vs-held.py`.

Pre-registered interpretation, fixed before the first attempt (a = anon at peak, A = allocated,
R = resident): `A/a >= 0.7` → LIVE; `A/a < 0.3` and `R/a >= 0.7` → HELD; otherwise AMBIGUOUS.

Two things the artifact records about itself, because both have been wrong in this work: the shim writes
the resolved symbol into its header (and `# UNRESOLVED: … this file contains no data` if it cannot resolve
one), and the sampler reads the run's configuration out of the running container instead of restating it.

## Result

| attempt | node | cgroup `anon` (MiB) | jemalloc `allocated` | `active` | `resident` | `mapped` | `retained` | A/a | R/a | verdict |
|---|---|---|---|---|---|---|---|---|---|---|
| 1 | bootstrap | 7704 | 7661.0 | 7667.4 | 7741.7 | 7817.4 | 0.0 | 0.994 | 1.005 | LIVE |
| 1 | v1 | 3097 | 3040.6 | 3053.0 | 3100.0 | 3175.0 | 0.0 | 0.982 | 1.001 | LIVE |
| 1 | v2 | 4436 | 4367.6 | 4377.4 | 4435.3 | 4503.4 | 0.0 | 0.985 | 1.000 | LIVE |
| 2 | bootstrap | 7911 | 7836.9 | 7843.0 | 7931.1 | 8003.0 | 0.0 | 0.991 | 1.002 | LIVE |
| 2 | v1 | 7941 | 7687.6 | 7697.3 | 7764.8 | 7841.3 | 0.0 | 0.968 | 0.978 | LIVE |
| 2 | v2 | 3958 | 3916.1 | 3922.6 | 3968.6 | 4032.6 | 0.0 | 0.989 | 1.003 | LIVE |
| 3 | bootstrap | 6138 | 6059.0 | 6065.0 | 6131.7 | 6207.0 | 0.0 | 0.987 | 0.999 | LIVE |
| 3 | v1 | 8162 | 8050.7 | 8183.1 | 8255.7 | 8325.1 | 0.0 | 0.986 | 1.012 | LIVE |
| 3 | v2 | 3143 | 3113.7 | 3122.8 | 3173.2 | 3244.8 | 0.0 | 0.991 | 1.010 | LIVE |

`resident / anon` is ≈ 1.000, and `mapped >= resident >= active >= allocated` in every row — the internal
ordering jemalloc guarantees, which is a check that the numbers are real rather than a constant.

## What this refutes

- **"The remainder is reclaimable, allocator-held memory, and the question is purge rate."** Recorded in
  `.cargo/config.toml` and in a #117 comment as a correction to an earlier claim, on the strength of a node
  whose `anon` fell 2352 → 79 MiB mid-run. That observation is real and is consistent with live memory
  being *released* when the node drops its structures; it is not evidence for retention. This measurement
  supersedes it, and the correction was itself wrong.
- **"`dhat` suppresses the failure."** Not shown. `dhat`'s run never ramped, and the phenomenon is
  intermittent; the 600-second absence was one draw. With the peak now known to be live Rust allocations,
  a heap profiler is the correct instrument for the next unit.

## Instrument defects found and fixed on the way

Each of these produced plausible-looking data rather than an error, which is why they are recorded:

- `stats_print` reports at process **exit**, after the node has freed its world — 3258 MiB of `anon` at the
  ceiling printed as `Allocated: 2.9 MiB`. Two different moments, not comparable.
- `stats_interval` prints per-arena blocks at ~150,000 lines per tick, perturbing the run it measures.
- `tikv-jemalloc-sys` passes `--enable-stats` and `--enable-prof` only for the corresponding Cargo
  features; with neither on, `stats_print` printed an option echo and no numbers, and `prof:true` was
  ignored without complaint.
- jemalloc's merged `stats.*` are **cached and refresh only when `epoch` is advanced**. Without the bump the
  shim produced 319 byte-identical samples — which the pre-registered bands scored as `A/a = 0.000`,
  AMBIGUOUS, the exact shape of a striking negative result. Caught by reading the raw series instead of the
  verdict. (The bands themselves had no row for it: they assumed the memory would be jemalloc's.)
- `opt.*` must be read into a buffer of the option's native type; jemalloc writes the value and returns
  EINVAL if the length differs, so a `bool` read into a `size_t` yields the right byte and a confident `-1`.
