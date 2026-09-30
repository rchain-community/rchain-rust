# #117 Stage A — the measurement, and what it did to the protocol's rule

Three runs of the frozen reproduction with the glibc-accounting shim
(`spec/audit/evidence/mallinfo-shim.c`, the one instrument
`n117-preregistration.md` permits). Raw per-second samples are in `target/n117-audit/stage-a-run{1,2,3}.tsv`
(untracked); this page is the tracked summary.

Configuration, in every run and written into each artifact's header: `DEVNET_NODE_MEMORY=4g`,
`DEVNET_NODE_ENV=LD_PRELOAD=/contracts/mallinfo-shim.so`, 3 validators, stakes `100,100,50`,
epoch-length 10, `--fresh`. Tree `d9a757750`; image `rnode:local` (the shipped plain build).

## The owner, named

Over **760 samples** — every tick at cgroup `anon` > 1 GiB with a successful shim read — glibc's own
accounting (`uordblks + fordblks + hblkhd`) explains the cgroup's anonymous memory at
**92.0–163.9 %, mean 101.0 %**.

That is the audit's central question — *who owns the anonymous bytes at the ceiling* — answered by the
one instrument that can see the allocator's own view: **the memory is glibc's malloc heap.** No prior
measurement could say this; a Rust heap profiler sees only what passed through the global allocator and
never what glibc retained, and `/proc` reports resident pages without saying whether an allocator call
still owns them.

Per-node at the last sample before death (MiB; `U` in use, `F` = fordblks + hblkhd):

| run | node | `anon` | `R` | `R/A` | `U` | `F` | `U+F` vs `anon` |
|---|---|---|---|---|---|---|---|
| 1 | bootstrap | 3958.5 | 1895.4 | 0.479 | 1982.4 | 2001.1 | 100.6 % |
| 1 | v1 | 4024.6 | 611.0 | 0.152 | *read failed* | — | — |
| 1 | v2 | 3964.5 | 842.0 | 0.212 | 3097.1 | 870.7 | 100.1 % |
| 2 | bootstrap | 4085.5 | 1366.8 | 0.335 | *read failed* | — | — |
| 2 | v1 | 3828.1 | 2378.4 | 0.621 | 3530.3 | 445.0 | 103.8 % |
| 2 | v2 | 4043.7 | 679.1 | 0.168 | 3338.9 | 742.3 | 100.9 % |
| 3 | bootstrap | 4084.5 | 2158.5 | 0.528 | 2336.6 | 1861.4 | 102.8 % |
| 3 | v1 | 942.4 | 562.8 | 0.597 | 366.2 | 593.5 | 101.8 % |
| 3 | v2 | 4000.7 | 1405.5 | 0.351 | 2548.4 | 1439.9 | 99.7 % |

The split between in-use and retained-free varies run to run and node to node — sometimes mostly in use
(v2 run 2: 3339 in use against 742 free), sometimes near half (bootstrap run 1: 1982 / 2001). Both
halves are glibc's.

**Two of nine episodes show a failed shim read** (`U` = 0.1 MiB against a 4 GiB `anon`). That is the
container exiting while the read was taken — the same class of defect as the `peak=0` bug the audit
found, and it is recorded rather than averaged.

## What this does to the protocol's rule

The frozen table decides by `R/A`, where `R` is a `smaps`-derived proxy for the arena class. Measured:

| `R/A` band | samples |
|---|---|
| below 0.5 — the table's *"arenas refuted"* row | **280** of 760 |
| 0.5 – 0.8 — the *"partial attribution"* row Amendment 2 had to add | **377** of 760 |
| ≥ 0.8 — the *"arena retention accepted"* row | **103** of 760 |

`R/A` spans **0.000–0.983** across the same runs, on the same process, within one run. So the rule's
verdict is decided by *which instant is sampled*, not by the state of the process: 37 % of samples would
say the arenas are refuted and 14 % would accept them, for a memory that glibc's own accounting
attributes to its heap at ~100 % throughout.

**The protocol's verdict rule is therefore superseded by the measurement it produced.** The table's
"arena retention accepted" row was reaching for the right thing — and the sentence it wanted is the one
above: the ceiling is glibc's heap, half in use and half retained. But the row's *proxy* cannot measure
that, and the audit had already found the same proxy swinging 658–1277 MiB on the preserved snapshot.
Amendment 2 narrowed the classifier; it did not cure it, and this is the measurement that says so.

A reader should take from this: **`R`-based rows of that table should not be used to decide anything.**
A protocol built on `uordblks`/`fordblks`/`hblkhd` — which is directly the quantity the table is about —
would have decided cleanly in every one of these 760 samples.

## Two things the run also settled

- **H6's accepted Θ(N²) ancestry residency is exonerated by direct measurement**, not by arithmetic.
  The node's own DAG gauges were read for the first time: at the ceiling, `logical_bytes` is
  **46–125 MB** and `seen_entries` **749–2584**. Against a 4 GiB anonymous footprint, the DAG's own
  residency is a hundredth of it. The register's accepted residual is real and is not this defect.
- **The "node never dies" case fired for real.** Run 3 ended with a node still alive at `anon` = 942 MiB
  when the 600 s window closed — the case Amendment 3 added after the gate pointed out the table was
  written only for death. It was exercised, not hypothesised.

## What remains unmeasured

The composition of the **in-use** half. glibc reports 2–3.5 GiB `uordblks` in the runs that die at
4 GiB, while the only heap profile of this shape — taken on a run that stopped below the ceiling — put
the live *Rust* heap at ~95 MiB. Whether the rest is Rust-side growth that no profile has seen at the
ceiling, or C-side allocation from `lmdb-rkv-sys` that a Rust-only profiler cannot see, is separable by
no instrument in the tree: the subtraction needs a profile at the ceiling, and the dump requires a clean
exit that a node at its ceiling does not get. That is a named hole, not a conclusion.
