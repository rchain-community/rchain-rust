# #117 — pre-registration of the discriminating measurement

**Status: FROZEN before the run.** This file exists so the measurement's verdict cannot be
rationalised after the fact. The thresholds and the arm design below were fixed while no result was
known. Any later change is a new commit that says why it was necessary and what invalidated the
original — silently editing this file would destroy the only thing it is for.

The convention it follows is the register's own for a measured bound (`spec/TEST-COVERAGE.md:412-416`:
a bound is *falsified before it is believed*, expressed relative to a same-process baseline so it
survives a different machine), plus `spec/audit/passes.md:2943-2945`: a bound calibrated against a tree
that no longer exists is prose, not a bound.

## Why this measurement

A node on a three-validator devnet is OOM-killed by its cgroup's memory controller within one to three
minutes, on a chain of ~15–90 blocks — three orders of magnitude below the project's own sizing model
(`docs/src/node/validator-requirements.md:68` budgets ~4 GB for a *1,000-block* chain). Two fixes
reduced allocation churn in the merge path by ~8× (8,796 → 1,126 MiB in `rchain_casper`) and changed
time-to-death by about two seconds.

**So the open question is not how much is allocated but who owns the bytes at the ceiling.** No
instrument currently in the tree answers it: the Rust heap profiler sees only the Rust global
allocator; the cgroup accounts for everything but attributes nothing; and the per-second monitors
record `anon` as a single number.

## The reproduction (fixed)

```bash
DEVNET_NODE_MEMORY=4g DEVNET_NODE_ENV= \
  tools/devnet.sh up --validators 3 --stakes 100,100,50 --epoch-length 10 --fresh
```

The configuration — ceiling, environment, thread count, image id — is written into the **filename and
the header** of every artifact this produces. The arm that misled the earlier work was labelled "no
arena cap" while its container environment contained `MALLOC_ARENA_MAX=2`; nothing in the tree records
which environment an arm ran under, so no existing arm file can be compared with another.

## What is read, per node, per second, until it dies

1. cgroup: `memory.stat` (`anon`, `file`), `memory.current`, `memory.peak` — read from the cgroup
   **path resolved from the container id**, not from `/proc/<pid>/cgroup` after exit, because a dead
   node's cgroup is gone and the previous instrument reported `peak=0` for exactly the nodes it existed
   to measure.
2. `/proc/<pid>/smaps`: every unnamed `rw-p` region, classified by 64-MiB alignment, with summed
   virtual size and summed Rss. This yields `R`.
3. `/proc/<pid>/smaps_rollup`: `Rss`, `Anonymous`.
4. thread count from `/proc/<pid>/task`.
5. the node's own gauges (`logical_bytes`, `seen_entries`) — which settle the DAG-residency question
   directly rather than by inference.

## Quantities and the frozen decision table

- `A` = peak `anon` of the dying node (MiB), from `memory.peak`.
- `R` = Rss in 64-MiB-aligned unnamed `rw-p` regions at the last sample before death.
- `F` = `fordblks + hblkhd` from glibc's allocator accounting (Stage B, or its `/proc` equivalent).
- `U` = `uordblks`, likewise.

| observation | verdict | consequence |
|---|---|---|
| `R/A ≥ 0.8` **and** `F ≥ 0.8·R` | arena retention **accepted** | the cap in use is the wrong knob; the fix space is allocator configuration, not the merge path |
| `R/A ≥ 0.8` **and** `F ≪ R` **and** `U ≥ 0.8·R` | **refuted as "freed"** — those regions hold live bytes | the heap profile's live figure is wrong or blind, and the fix space is unidentified |
| `R/A ≥ 0.8` **and** `F ≪ R` **and** `U ≪ R` | **mislabelled** — the bytes sit in a mapping no allocator call owns | the class name is wrong; a reservation, a dirty-page map or a runtime mapping owns them |
| `R/A < 0.5` | arenas **refuted** | the owner is outside this class and the same instrument names it |

**The single number to report: `R/A`**, with its sample time, `memory.peak` at death, and the arm's
recorded configuration. One number, one artifact, one configuration.

## The companion comparison for the fix verdict

A separate run, never concurrent with the above, and never rebuilt between its arms (a rebuild changes
the binary under test, which is part of what this audit is correcting):

- **arms**: pre-fix `rnode:control-94ea0a1d2` (verify the image still exists locally before committing
  to this design; do **not** rebuild that commit mid-audit) and the current tree;
- **one** stated ceiling and **one** stated environment for both;
- **≥ 3 repeats per arm**;
- **endpoint: continuous** — peak `anon` at a fixed time — *not* a survival count. The observed spread
  in survival counts at nominally identical settings was 1, 2 and 1 of 3; separating that endpoint at
  80 % power would need about eight runs per arm, whereas a continuous endpoint separates in three.

## What this does not settle

The C-side allocations that the Rust heap profiler cannot see. If Stage A is ambiguous about ownership,
a `LD_PRELOAD` shim calling `mallinfo2()` once per second is the one piece of new machinery permitted —
it is compiled on the host and changes nothing about the node binary.

---

## Amendment 1 — the region classifier and the same-instant rule

**Stated, and why: this refinement is forced by evidence produced after the freeze, and it changes no
threshold in the decision table.** The audit's third seeded attack recomputed the only preserved
`smaps` snapshot and found that "64-MiB-aligned unnamed `rw-p`" is ambiguous as a definition of the
arena class: the same snapshot holds **29 regions of exactly 32 MiB** that are the worker thread stacks
(`thread_stack_size(32 MiB)`) and **11 regions of exactly 64 MiB** that are the arena heaps, and a
figure that merged the count from one class with the size of the other produced a claim the artifact
contradicts. With the original definition, `R` could be dominated by a class that is not the one under
test, and a misclassification would decide the verdict. The thresholds `0.8` / `0.5` and the four-row
table are unchanged.

**`R` is redefined as the Rss of the _arena_ class only**, identified by all three signatures below
rather than by alignment alone. A region counts toward `R` only if it matches the arena row.

| class | signature in one `smaps` read |
|---|---|
| **arena heap** (counts toward `R`) | starts on a 64-MiB boundary; `Rss ≈ Size` (the used portion is resident); adjacent to other 64 MiB blocks |
| **thread stack** (excluded from `R`) | immediately preceded by a 4 KiB `---p` guard; `Rss ≪ Size`; one per worker thread by count |
| **reservation / transient buffer** (excluded from `R`, reported separately) | 64-MiB-aligned possible, but `Rss < Size`, and its presence changes between snapshots |

**The same-instant rule.** Every `smaps`-derived total must be paired with a `smaps_rollup` and a
cgroup `memory.stat` read **at the same instant**, and the three must be reported together. The reason
is measured: across the four files of the preserved snapshot, written ~90 ms apart, the `smaps`-summed
Rss exceeds the `smaps_rollup` Rss by **219 MiB**. The process is mid-runaway and non-monotonic
(anon dips of 30–78% recur), so a single-point total taken alone is not trustworthy — and a verdict
that turns on `R/A` must not turn on which of two reads of the same instant was used.

Both changes make the measurement *harder* to satisfy, not easier: they exclude a class that could
have inflated `R`, and they add a consistency check that can fail. Nothing here relaxes a criterion.
