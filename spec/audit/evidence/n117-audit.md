# Red-team synthesis v3 — issue #117 (memory runaway / cgroup kill)

Read-only audit. Branch `audit/117-memory-hazop`. Produced in response to the round-3 gates' findings
on v2. **The verdict sentence is unchanged and still stands** — §11 states that explicitly and shows
the arithmetic behind it. What changed is the counts, the arithmetic under them, five citations, the
tracking column, and four v2 statements that were wrong.

**Changes from v2, in one place (so this file can be diffed by eye):**

| § | change | driver |
|---|---|---|
| §4 | the smaps snapshot is **pinned** to a named run at a known instant (15:53:39 UTC, `trim.tsv` run 2, cgroup `anon` ≈ 1151 MiB) | new work, this pass |
| §5 | the frozen protocol was **amended** (`7a9f293fa`, Amendment 1) to redefine `R`; §5 now evaluates the amended rule and shows its residual hole | new work, this pass |
| §6 row 10 | the crate-churn figure is **tracked**, not "orphaned" — rule + script now exist (`c0d7fd728`) | coordinator |
| §6 row 19 | verdict **C → S**, downgraded: `docker/rnode/Dockerfile` contains no `MALLOC` at all | coordinator |
| §6 row 34 | the "no arm records a ceiling" replacement also carries `run-profile.sh`'s `12g` vs `6g` self-contradiction | coordinator |
| §6 | `trk` column now reads the **deciding artifact**, not the claim's source. **41 of 48** claims are `untracked evidence` | coordinator |
| §6 | 9 falsifiers made **two-sided**, each with the direction it guards stated | coordinator |
| §1 | K1's "relative" reading shown **vacuous**; K2's reason **inverted** in v2 and corrected; N re-derived by clustering to **27 clusters / 25 growing** | coordinator + this pass |
| §5, §7 | arithmetic fixed: 960.0 MiB, two 128-MiB regions, ~145 threads, `[heap]` 1.96 MiB, `Pss_File` 8.6 MiB, UTC times | coordinator |
| §7 | `N1 file_writeback` reason added; `N2` and `N8` vacuous cells given deviations or explicit scope-outs | coordinator |
| §9 | the amended classifier's residual hole; the `R/A` values the table cannot decide between | this pass |
| §10 | unbounded-channel inventory corrected: **2 in this pipeline, 4 in the tree, 1 of the 4 a test** | coordinator |
| §12 | v2's four retracted statements listed | — |

**Artifact shorthand.** `R5`=`run5-samples.tsv` · `RR`=`rerun.tsv` · `TR`=`trim.tsv` · `TP`=`tp2.tsv`
· `MM`=`mmap.tsv` · `AC`=`arena-control.tsv` · `AM`=`arena-max2.tsv` · `R2/R3/R4`=`run{2,3,4}-samples.tsv`
· `S1`=`samples.tsv` · `PS/PR/PM/PT`=`probe-{smaps,rollup,maps,threads}.txt` · `AL`=`acceptance.log` ·
`EL`=`e2e.log` · `AP`=`arena-probe.log` · `DHB/DHA`=`dhat-heap-before.json`/`dhat-heap-after.json` ·
`L1/L2`=`logs/`,`logs2/` · `PRE`=`spec/audit/evidence/n117-preregistration.md` · `ISS`=`target/n105/issue.md`.

**Clock discipline (v2 got this wrong).** File mtimes on this host are **BST (UTC+1)**; every
`*-samples.tsv` / `*.tsv` `utc` column and every `docker logs` timestamp is **UTC**. v2 compared a
local mtime against UTC timestamps. All times below are UTC, and the conversion is checked against an
independent anchor: `e2e.log` records "run exit: 0 at 17:32:05" (UTC) and `dhat-heap.json`'s mtime is
18:32:05 local — the same instant. Corrected mtimes: `PS`/`PR` **15:53:39 UTC**, `DHB` **16:42:46 UTC**,
`DHA` **17:32:05 UTC**.

---

## 0. The base's standing defects

**D-1 — every artifact is untracked.** `git ls-files target` is empty. 55 files in 2 directories.
**Every claim whose *deciding artifact* is under `target/n105/` is marked `untracked evidence`** in §6.
Marking by the deciding artifact rather than by the claim's source is the v3 correction: **41 of 48
surviving claims.**

**D-2 — the pre-registered measurement was never executed.** `PRE` requires per-second `memory.stat`,
`memory.peak`, `smaps`, `smaps_rollup`, thread count and the node's own gauges, until death.
`grep -l 'Rss\|smaps' *.tsv` → nothing; all seven arm files carry `monitor5.sh`'s header, and
`monitor5.sh` reads no `smaps`. `grep -r 'logical_bytes\|seen_entries' logs/ logs2/` → nothing. The
only `smaps` is one one-shot capture. No `mallinfo` data exists. **`A`, `R`, `F`, `U` therefore exist at
one instant on one node on one run, and `F`/`U` do not exist at all** — so three of the frozen table's
four rows are unreachable. This is the largest defect in the base.

**D-3 — the arm launchers are absent.** `target/n105/` holds the five monitors, the two `probe-maps`
scripts, `check.sh`, `load.sh`, `run-profile.sh`, `wait-and-measure.sh`, `acceptance.sh`,
`arena-probe.sh`, `analyse-dhat.py`. **Nothing launched the `arena-control`, `arena-max2`, `trim`,
`tp2` or `mmap` arms.** Arm→environment is unrecoverable in principle, not merely unrecorded.

**D-4 — no arm file records its own configuration, and the files overlap in time.** All seven arm files
share `monitor5.sh` (+ a `LOG` override), whose only config comment is `:6` "Cap 8g, matching runs 2-3"
— which is wrong for the several 4 GiB arms among them. The files also overlap: `AC`/`AM` share 198 s
of wall-clock window, `TR`/`TP` 233 s, `TP`/`MM` 262 s. **Consequence: the base does not contain the
information needed to count physical runs.** Any such count is a property of the dedup rule, and v3
publishes both the rule and its sensitivity (§2).

---

## 1. Adjudication 1 — finality

**Verdict: the correlation holds under every reading I can construct; the gate is right that the
robustness claim in v2 was false, and the count is smaller and rule-dependent. Causation is not
established. The orchestrator's mechanism claim is contradicted.**

### The published count, with its rule (D1 conflict 3)

**Rule.** Group rows by node; split into `alive=1` runs; split again where `anon` falls from >300 MiB to
<60 MiB; drop runs under 6 samples; then **cluster episodes across files by interval union** (episodes
of the same node whose `[start, end]` intervals overlap are one physical run — necessary because the
arm files overlap, D-4). `anon` for a cluster is the union of its member samples.

**Result: 27 physical-run clusters — 25 growing (peak ≥ 250 MiB), 2 flat.** Raw node-runs before
clustering: 39.

| knee rule | growing clusters | within ±2 s of finality's first advance |
|---|---|---|
| **K1abs** first `anon > max(3·baseline, 150 MiB)` | 25 | **25/25** (deltas −1…+2 s) |
| **K2consec** first consecutive step of ≥50 MiB from the first sample | 25 | **24/25**; one at −3 s (`bootstrap`, cluster @15:53:21) |
| K1 **relative** (the bare `3·baseline` term) | 25 | **not a distinct rule — see below** |
| gate's K2 with its `base ≥ 15 MiB` guard | 6 | fires on 6 of 25 |

**Flat clusters: 2, and both never move finality** — `R5` v2 (peak 27.7 MiB, height 45) and `TR` v2 run0
(peak 39.3 MiB, height 82). **Growing clusters whose finality never moved: 0.**

**Three corrections to v2's framing, all of which the gate is right about:**

1. **K1 and K3 were not "independent rules".** Over the 25 growing clusters the **maximum first-sample
   `anon` is 37.6 MiB**, so `3·baseline` never exceeds 112.8 MiB and K1 is *identically* "first `anon`
   > 150 MiB". K1 and K3 were two absolute thresholds ~1.3× apart on a ramp of ~650 MiB/s. **The
   robustness claim is retracted; the correlation is not.** What survives as a real cross-check is that
   an absolute threshold (K1abs) and a step-size rule (K2consec) — which are methodologically different
   — agree to within one sample.
2. **v2's stated reason K2 could not fire was inverted.** K2 carries a `base ≥ 15 MiB` guard and **19 of
   the 25 growing clusters start below 15 MiB** (range 1.1–37.6 MiB), which is why it cannot fire. v2
   said the monitor starts after the baseline has risen; that is backwards.
3. **v2's N was inflated and `20/20` is not reproducible either.** The gate's clustering gives 28
   distinct runs / 26 growing; mine gives 27 / 25. **The discrepancy is one cluster, and it is caused by
   D-3/D-4, not by the data**: with no launchers and no headers, where one physical run ends and two
   begin is not recoverable, so the N is rule-dependent by construction. **The ratio is robust, the N is
   not**, and v3 publishes the rule so a third party can move the N deliberately rather than by accident.
   One further discrepancy to record rather than paper over: the gate's shared-row figures
   (183/212/237 s) do not match mine (63/145/27 shared *timestamps*; 198/233/262 s of shared
   *wall-clock window*). Both readings agree the dedup key is unsound; neither is authoritative.

### The natural experiment

`TR` 15:49:57–15:52:41. `v2` runs 2 m 45 s to height **82** flat at 14–39 MiB while `bootstrap` and `v1`
on the same devnet, same window, ramp at 511 and 676 MiB/s and die. `v2` was not idle:
`L1/devnet-validator-2.log` shows **94 `close_block`** and **44 `proposed and added`** against
bootstrap's 54 and 14. Its index-call counter never reached the 250 that triggers a log line while
bootstrap reached 750 — the survivor did **less merge/index work**, not less block work.

### What that kills, and what survives

- **"The trigger is forked block processing" — contradicted.** `v2` processed the same forked heights
  1–17 that the other two did, flat at 14–15 MiB, then 65 further heights, flat. Volume of forked-block
  processing is not the operative variable. The orchestrator's reasoning — "the survivor never
  finalized, therefore the trigger is forks" — **inverts its own evidence**: a node that never
  finalized and never grew is the cleanest support for the finality correlation, not against it.
- **The knee is a switch, not a per-event cost.** `R5` bootstrap's `finalized` freezes at 3 at
  15:13:57 while `anon` climbs 930.5 → 8083.9 MiB (650 MiB/s) over 11 s with height frozen at 15.
- **There is a second, slower regime.** `TR` v1's survivor (15:54:59–15:57:25) shows an additional
  finality step 7→8 at 15:55:10 with a discrete floor step 1697 → 2300 MiB, then two minutes of frozen
  finality with the floor creeping 2300 → 2882 and per-second peaks growing 2574 → 3711. Consistent with
  finality as the switch; it also shows the creep is not driven by finality.
- **Causation is not established and the confound is structural**: finality advancing is when the DAG
  begins to be committed and when the index cache becomes prunable (`casper/src/merging.rs:575-597`), so
  at least four things turn on in the same second and the artifacts cannot separate them.

---

## 2. Adjudication 2 — who owns the anonymous bytes at the ceiling

**The snapshot is now pinned** (this pass). `PS`/`PR`/`PM`/`PT` are 16:53:39 local = **15:53:39 UTC**.
`probe-maps2.sh` instruments `devnet-bootstrap`, and only `TR` covers that instant. At 15:53:39 UTC
`TR`'s bootstrap reads **h=14, fin=3, anon=1151.0 MiB, file=1.9 MiB** (neighbours 753.2 at :38 and
1698.0 at :40, so the capture's cgroup `anon` is 0.75–1.7 GiB). **So the only `smaps` snapshot in the
tree is `devnet-bootstrap` in `TR` run 2, taken at `anon` ≈ 1151 MiB, in the run that then peaked at
5847.6 MiB and died at 15:53:55.** The earlier "one instant, unknown run" description is replaced by
that. It also means **`R` is measured at 1151 MiB and the arm died at 5847.6 MiB** — which is the whole
of §9.

At that instant (`PR`): `Rss` 1102.9 MiB, `Anonymous` 1078.9 MiB, `Private_Dirty` **1079.0 MiB**,
`Pss_File` **8.6 MiB**, `Pss_Shmem` 0, `AnonHugePages` 0.

| candidate | shown | not shown | cheapest separator |
|---|---|---|---|
| **page cache / LMDB dirty pages** | **Refuted.** `file` 1.8–2.5 MiB through every ramp (`R5 RR TR TP MM`); `R3` `file_dirty` ≈ 20 MiB and `file_writeback` ≈ 20 MiB at its one sample; `Pss_Shmem 0`; `Pss_File` 8.6 MiB | — | one `memory.stat` read — done |
| **thread stacks** | **Refuted at 1151 MiB.** 29 unnamed regions of exactly 32 MiB holding **5.7 MiB of RSS** (0.43 % of total Rss); 30 threads (`PT`) | whether the pool was larger *at the ceiling* | thread count + `VmStk`, per second |
| **glibc arena retention** | **Present, not proved.** The contiguous `0x6456…` block: thirteen heap-scale VMAs, **1018.9 MiB of Rss**, fill factor 0.99–1.00 (§5, §9) | whether those pages are **free** or **live** — `F` vs `U`, neither measured anywhere | `mallinfo2()` per second |
| **C-side `malloc`** | invisible to `mode=rust-heap`; allocates through **the same arenas**, so `PS` cannot separate it from the row above; **other C libraries (libc, `ld.so`, TLS) allocate there too and are equally unmeasured** | Rust-vs-C attribution inside a shared arena | malloc interposition counting Rust frames — outside the frozen protocol |
| **unbounded channels** | **2 in this pipeline**: `node_runtime.rs:733` (production, full `BlockMessage`s) and `:2557` (the tap). §10 | whether either ever holds more than a handful | one depth counter |
| **a genuinely live Rust structure** | the only profile shows 2.1 MiB live — **void**: the drop is post-shutdown and neither profiled run reached the ceiling (`EL:771`, after arm `anon=302 MiB, exit 0` against a 2500 MiB threshold; before arm unrecorded) | everything about the ceiling | a profile taken at the ceiling |

### The arithmetic gap, corrected

v2's sentence — "13 aligned regions ≈ 832 MiB virtual, 913.2 MiB resident" — was **internally
impossible**: resident cannot exceed virtual, and 13 × 64 MiB is not the virtual size because two of the
thirteen are **128 MiB** VMAs. Corrected, measured: the class is **11 × 64 MiB + 2 × 128 MiB =
960.0 MiB of `Size`, 913.2 MiB of `Rss`** (resident < virtual ✓, and every region's header `Size:`
field agrees with its address range to within one page).

And v2's "~13 heaps at 30 threads at 1.1 GiB → ~62 heaps at death" was wrong under its own linearity:
30 threads → 13 areal heaps is 0.433 heaps/thread, so 4 GiB of heaps (64 heaps) needs
**≈ 145 threads**, not 62. Under the same linearity, the coordinator's check is right and mine is not:
**60 threads gives ~26 heaps ≈ 1.7 GiB**. So the measurement's discriminating power is different from
what v2 said: a thread count of **30 at death refutes** the arena-per-thread route to the ceiling; only
a count **≥ ~145** supports it. Two caveats that keep this honest: glibc's `MALLOC_ARENA_MAX` defaults to
`8 × ncores`, so the linearity itself is an assumption the artifacts do not test (it holds only below
the arena cap); and 145 is comfortably below tokio's 512 default, so the route remains feasible.

---

## 3. Adjudication 3 — what the "8× churn reduction" establishes

**v2 called the figure "true but orphaned". That framing is retracted.** The rule was never written
down, and it now is: **`spec/audit/evidence/dhat-crate-churn.py`, committed `c0d7fd728`**, which states
it (frames traversed in reverse of their listed order; the first frame whose pre-`::` token is not in
`alloc`/`core`/`std` and does not begin with `[`, `<` or `0x`; that program point's `tb` credited to it)
and produces the figures below. I ran it: it reproduces them exactly. **The number is now a
measurement with an instrument, and the instrument is tracked.** The choice of rule moves the answer —
crediting *any* frame in the crate gives 14,474.7 → 2,034.8 MiB — which is why the script, not the
number, is the citation.

| quantity | before (`DHB`) | after (`DHA`) | ratio |
|---|---|---|---|
| `Σtb` `rchain_casper` (ever allocated) | 8,796.1 MiB | 1,126.0 MiB | **7.81×** |
| `Σtb` whole process | 16,391.4 MiB | 3,300.4 MiB | 4.97× |
| `Σgb` (held at global peak) | 410.8 MiB | 94.5 MiB | 4.35× |
| max per-site `mb` (peak live at one site) | 56.9 MiB | 19.6 MiB | 2.90× |
| `Σeb` (live when the profiler dropped) | 2.0 MiB | 2.1 MiB | 1.05× |
| `pps` / `ftbl` | 164,734 / 13,094 | 158,238 / 12,582 | different binaries |

**Established:** the churn reduction itself, on two different binaries, measured by a tracked
instrument.

**Inference, not established:** *"different ceilings"* — no dump records one (`DHB`/`DHA` metadata is
only `mode/cmd/pid/tg/te`; `cmd` is identical). *"the before arm never reached the ceiling"* —
unrecorded. *"the profiled runs never reached the ceiling"* — supported for the after arm only.
*"the before-profile's producing run is unrecorded"* — supported: the preserve step in
`wait-and-measure.sh` never fired (`e2e.log` never prints "pre-fix profile preserved"); `DHB` covers
603.2 s of process life against `DHA`'s 531.9 s; no tree or cap is recorded.

**Struck:** any treatment of the two profiles as one experiment. The after run is a node that **did not
exhibit the failure at all**. The pair establishes exactly one thing: *the merge path's cumulative
allocation on that shape fell ~7.8×.*

---

## 4. Adjudication 4 — instrument defects and what they touch

| # | defect | artifact | claims it touches | correction |
|---|---|---|---|---|
| I-1 | `acceptance.sh` reports `peak=0` for exactly the nodes it exists to measure: it resolves `/proc/$PID/cgroup` with `PID` read *after* the container exited, and its comment claims `memory.peak` "cannot miss a spike" | `AL:746-747`; `acceptance.sh` | every "peak MiB" for a dead node | those two peaks are **not measured**; the outcome survives via `status=exited oom=true exit=137` |
| I-2 | `analyse-dhat.py` does not compute what it was cited for: headline is `max(gb)` under a `Σgb` label (off by 23.6×/5.4×), crate table sums `mb` | `dhat-after.txt:49-60` | "tens of MiB"; every per-crate number | use `dhat-crate-churn.py` (§3) |
| I-3 | the snapshot's own totals drift **219.4 MiB** (`Σ`smaps `Rss` 1322.4 vs `smaps_rollup` 1102.9) across files written ~90 ms apart | `PS` vs `PR` | every single-point total; `R`; every ratio | all such figures carry ≈20 % |
| I-4 | `arena-probe.sh`'s comparison figure is a literal in an `echo`; its own classifier is a third definition, `size >= 63 MiB` (`:44`) | `arena-probe.sh` | the doc's and `5d3c55022`'s "29 regions of exactly 64 MiB — one per worker — fully resident" | **29 regions of exactly 32 MiB, 5.7 MiB RSS** — the stack class; the 64-MiB class is 11/658.2; 29 × 64 MiB = 1856 MiB exceeds the whole anonymous RSS (1298.4 summed) |
| I-5 | `monitor5.sh`'s `alive` is a cgroup-readability proxy | all seven arm files | none material | read it as "the container's cgroup was resolvable" |
| I-6 | `run-profile.sh` contradicts itself: `:4` says "the cap is deliberately generous (12g)", `:17` defaults `PROFILE_MEMORY` to `6g` | `run-profile.sh:4,17,20` | the profiled arm's ceiling | the arm is **6 GiB by code / 12 GiB by its own comment**; unresolvable from the artifact |
| I-7 | `monitor3.sh`'s byte/KiB and host-root fallback | `R3` | only withdrawn readings | `R3` unusable |
| I-8 | **v2 compared a local mtime to UTC timestamps** | §0 | the "different runs" justification | corrected: 15:53:39 / 16:42:46 / 17:32:05 **UTC**; the verdict holds and is now checkable |

---

## 5. The frozen protocol: what it now is, and where it is still inadequate

**The protocol was amended after the artifacts were produced** — `7a9f293fa` (19:32 on 2026-09-29, the
same timestamp as the round-1 brief), *Amendment 1 — the region classifier and the same-instant rule*,
at `PRE:89-120`. The freeze permits amendment only with a stated reason, and the amendment gives two:
the class-confusion the artifacts exposed (the count from the 32-MiB stack class, the size from the
64-MiB arena class) and the 219 MiB same-capture drift. **It changes no threshold**, and it makes the
measurement *harder* to satisfy. v2 did not know of it and criticised the pre-amendment text; v3
evaluates the amended rule.

**v2's criticism 2 (single-point totals) is answered by the amendment.** Criticism 1 is not:

| # | status | why |
|---|---|---|
| `A` is not collectable as written | **open** | `PRE` still defines `A` as "peak `anon` of the dying node, from `memory.peak`". `memory.peak` is `memory.current`'s high-water (it *includes* `file`), and `PRE:41-43` itself says a dead node's cgroup is gone. Both halves are the I-1 defect. Corrected `A` = `max` over the per-second `anon` samples while alive |
| the same-instant rule | **added, and unsatisfiable by the preserved snapshot** | the amendment requires `smaps` totals paired with a cgroup read *at the same instant*. The snapshot has no cgroup read: the nearest are `TR`'s 1-second samples (753.2 / 1151.0 / 1698.0 MiB at :38/:39/:40). At ~650 MiB/s, a one-second offset is ±650 MiB, so `R/A` from this artifact carries **≈±57 %**. The amendment's own consistency check cannot be met by the artifact the amendment was written about |
| `R` is not single-valued | **narrowed, not closed** | below |

**The amended `R`, evaluated.** Amendment 1 defines the arena class by three signatures — 64-MiB-aligned
start, `Rss ≈ Size`, adjacent to other 64 MiB blocks — and excludes stacks (4 KiB guard, `Rss ≪ Size`)
and reservations (`Rss < Size`, unstable between snapshots). Applying it to `PS`:

| reading | n | `R` (Rss) | `R/Rss` 1102.9 | `R/anon` 1151 | note |
|---|---|---|---|---|---|
| pre-amendment, size exactly 64 MiB | 11 | 658.2 MiB | 0.597 | 0.572 | below the table's lowest row |
| pre-amendment, aligned (start & size ≡ 0 mod 64 MiB) | 13 | 913.2 MiB | 0.828 | 0.793 | straddles 0.8 |
| **amended, literal** (no size floor) | 17 | 1030.1 MiB | 0.934 | 0.895 | **includes a 0.766 MiB region** that is 100 % filled and adjacent to a big block |
| **amended + heap-scale floor (Size ≥ 60 MiB)** | 13 | **1018.9 MiB** | 0.924 | 0.885 | the thirteen contiguous `0x6456…` VMAs, fill 0.99–1.00 |

**Residual hole, reported as a hole:** the amended rule has no size floor and no definition of "≈" or of
"adjacent", so read literally it admits a 0.766 MiB region purely because it is full and near a big one,
while it *excludes* the three 100 %-filled 64-MiB heaps in the `0x7370…` block (`73701c`, `737034`,
`7370c0`, Rss 64.0 MiB each) because their neighbours are reserves rather than filled heaps. The
amendment fixed the class confusion it names; a floor and an adjacency tolerance are still owed.

**And the table still cannot be applied to its own best artifact.** `R` is measured at `anon` ≈ 1151 MiB;
the arm died at `anon` = 5847.6 MiB. So:

- the **frozen** reading (`R/A` with `A` = the arm's peak at death): 1018.9 / 5847.6 = **0.174** →
  the table's *"`R/A < 0.5` → arenas refuted"* row;
- the **same-instant** reading (1018.9 / 1151) = **0.885** → the table's *"arena retention accepted"* row.

The same artifact, the same rule, opposite rows, and the difference is entirely whether `A` is the
instant `R` was taken or the peak the arm reached. That is not a judgement call about evidence quality;
it is a gap in the frozen table, and it is the strongest single argument for §9's measurement.

---

## 6. The claim ledger

Columns: claim · source · deciding artifact · verdict · **falsifier (direction stated)** · tracking.
Verdicts: **S** supported · **U** unsupported · **C** contradicted · **P** pending. `UNT` = untracked
evidence, read off the **deciding artifact**. Where a verdict is **S**, the falsifier is evidence that
would overturn it; where **C**, the falsifier is evidence that would establish the claim.

| # | claim | source | deciding artifact | verdict | falsifier (direction) | trk |
|---|---|---|---|---|---|---|
| 1 | "A node that stops advancing while a bonded peer keeps producing accumulates anonymous memory without bound, until its own cgroup limit kills it." | `ISS:9-11` | `R5` (`anon` 8083.9 on an 8 GiB cap), `TR` | **S** | a run of the shape with `anon` flat to the cap, **or** a killing node whose `anon` was bounded | UNT |
| 2 | "The two nodes that stalled are the two that blew up; the node that raced ahead stayed flat." | `ISS` table | `R5` | **S** | a stalling node that stays flat, **or** a racing node that grows | UNT |
| 3 | v2 "height 41 … peak 25.1 MiB" | `ISS` table | `R5` last sample: height 45, peak 27.7 | **C** by 4 heights / 2.6 MiB | a re-read of the same run giving 41 / 25.1 — i.e. a run where the monitor stopped earlier | UNT |
| 4 | "Peaks are `memory.peak` (monotonic per cgroup)" | `ISS:20` | `R5` col 8 vs col 5 (8108.9 vs 8083.9) | **C** | a `peak_mb` column equal to the `anon` column | UNT |
| 5 | "`file` stays at ~2 MiB while `anon` reaches 8 GiB" | `ISS` | `R5`, `PR` | **S** | any arm with `file` > 100 MiB during a ramp | UNT |
| 6 | "A lone validator is flat: 72.2 MiB at height 16, 72.3 at height 36" | `ISS` | `R4` (72.1→72.3 MiB over 39 s) | **S** | a lone validator that ramps, **or** a lone-validator run whose height advances without `anon` moving above 100 MiB *and* a three-validator run at the same height staying flat | UNT |
| 7 | "It begins as finality begins" | `ISS` § | `R5 RR TR TP MM AC AM`: **25/25 growing clusters within 0–2 s** (§1 rule) | **S** | **two-sided**: a growing cluster whose knee precedes finality by >2 s, **or** one whose knee lags finality by >2 s | UNT |
| 8 | "`validator-2` stays at 25 MiB while reaching a higher height" | `ISS` | `R5` (v2 peak 27.7 MiB at h=45, `finalized` never leaves `none`) | **S** | a `last-finalized-block` reading for v2 that is not `none`, **or** a v2 peak above 100 MiB | UNT |
| 9 | "the same shape was OOM-killed at 4 GiB (17 s) and at 8 GiB (35 s)" | `ISS` | `AC` 17–19 s ✓; `R5` 24–28 s | **C** on the 8 GiB figure by ~11 s | a 35 s death at 8 GiB, **or** an 8 GiB arm dying at 24–28 s from a first sample taken at container start | UNT |
| 10 | "reduced allocation churn in the merge path by ~8× (8,796 → 1,126 MiB in `rchain_casper`)" | `PRE` | `dhat-crate-churn.py` (tracked, `c0d7fd728`) on `DHB`/`DHA` | **S** | the script ceasing to produce 8,796.1 / 1,126.0 | **tracked** |
| 11 | "and changed time-to-death by about two seconds" | `PRE` | none | **U — struck** | a repeatable time-to-death with the spread characterised | — |
| 12 | "29 anonymous regions of exactly 64 MiB — `HEAP_MAX_SIZE`, one per worker — fully resident" | `docs/src/node/validator-requirements.md:120-121`; `5d3c55022` | `PS`: 29 regions are 32 MiB, 5.7 MiB RSS; 64-MiB class 11/658.2 | **C** | a snapshot showing 29 regions of 64 MiB with `Rss ≈ Size` **and** 29 worker threads ≠ 29 | UNT |
| 13 | "the main `[heap]` sat at 1.9 MiB" | same | `PS` (`[heap]` `Rss` 1.926 MiB, `Size` 1.961 MiB) | **S** | a snapshot with `[heap]` `Rss` above 100 MiB | UNT |
| 14 | "file-backed RSS at 26.8 MiB" | same | `PS`: 24.7 MiB excluding kernel-named `[heap]`(1.9)+`[stack]`(0.2); 26.8 includes them | **C** by 2.1 MiB | a region list in which all 26.8 MiB is genuinely file-backed | UNT |
| 15 | "A heap profile of **that run** accounts for 16.4 GiB allocated against 2 MiB still live at exit" | same | `DHB` = 16,391.4 MiB = **16.007 GiB**; `DHB` 16:42:46 UTC ≠ `PS` 15:53:39 UTC | **C** twice | a shared run id linking the profile to the snapshot, **or** a 16.4 GiB total from the raw JSON | UNT |
| 16 | hence "nothing is leaked" | same | the drop is post-shutdown (`run-profile.sh`'s own comment) | **U** | a dump taken before the shutdown drain showing the same live figure | UNT |
| 17 | "`MALLOC_ARENA_MAX=2` … 1, 2 and 1 surviving nodes of three" | `validator-requirements.md:126-129` | `AL`: 1 of 3 reproduced; the 2-of-3 run is not on disk | **P** | the 2-of-3 run | UNT |
| 18 | "a probe … could not sample the nodes that mattered because two had already been killed" | same | `AP:4-5` | **S** | a probe run sampling all three | UNT |
| 19 | "Capping the arenas is unproven and is **deliberately not shipped**" | `validator-requirements.md:126` | **`docker/rnode/Dockerfile`: `grep -ic malloc` = 0** | **S** (downgraded from C) | a `MALLOC_*` or `ENV` stanza in the image | **tracked** |
| 19a | (v2's inference from this: the acceptance run therefore contradicts the docs) | v2 | `AL:744` = `MALLOC_ARENA_MAX=2` in the *container* | **C** — but it is a **test-time env override**, not a shipped default, so it contradicts the *arm's label*, not the docs' claim about the image | an `AL` run whose env dump shows no `MALLOC_*`, i.e. `DEVNET_NODE_ENV` empty | UNT |
| 20 | "fixed without the arena cap: 1 of 3" | `acceptance.sh` header; `AL:749` | `AL:744` | **C** | as 19a | UNT |
| 21 | "no commit ever put `MALLOC_ARENA_MAX` in the Dockerfile; `5d3c55022` is docs-only" | B4 | `git show 5d3c55022 --name-only`; `grep -ic malloc docker/rnode/Dockerfile` = 0 | **S** | a MALLOC stanza in a Dockerfile at any commit | **tracked** |
| 22 | "`memory.peak` … cannot miss a spike" | `acceptance.sh` comment | `AL:746-747` | **C** | a spike visible in a `peak=0` row | UNT |
| 23 | "the profile explains the OOM" | round-1 target | `EL:771` | **U** | a profile of a dying node | UNT |
| 24 | "~750 requests **over one stall** with 711 of them hits" | `casper/src/merging.rs:457` | `L1` bootstrap 379: 750 = cumulative `INDEX_CALLS` since process start; cache 39; pruned 0 | **C** on "over one stall"; **S** on 711 = 750 − 39 | a per-stall counter in the log line, **or** a run where 750 − cache ≠ hits (i.e. any pruned > 0) | **tracked** |
| 25 | R15 "unbounded block-validation pipeline … done" | `spec/findings.tsv:84` | `node_runtime.rs:733`, `:2557` | **C** as to completeness (§10) | a bound at either site | **tracked** |
| 26 | "the memory knee is within 0–2 s of finality leaving none in **6/6** growing nodes" | A4 | **25/25 growing clusters** under K1abs and **24/25** under K2consec (§1) | **S**, count corrected from 6/6 | one growing cluster outside ±2 s under both rules | UNT |
| 27 | "the single flat node never finalized (**1/1**)" | A4 | **2/2** | **S**, count corrected from 1/1 | a flat cluster whose finality moves | UNT |
| 28 | "the trigger is forked block processing" | orchestrator prose | `TR` v2 run0 (h=82, 39.3 MiB, 94 `close_block`, 44 proposals) | **C** as stated | a node processing forks with finality frozen that *does* grow | UNT |
| 29 | "29 × 64 MiB = 1.86 GiB does not reach 4 GiB" | A3 | `PS`: the 29-region class is 32 MiB (5.7 MiB RSS); the 64-MiB class is 11/658.2 or 13/913.2 | **C** as a premise; the real gap is ~3.1 GiB ≈ 49 further full heaps | a snapshot showing 29 regions of 64 MiB at `Rss ≈ Size` (same falsifier as 12) | UNT |
| 30 | "post-fix runs 2 and 3"; "37 GiB free"; "~130 s vs 132 s"; "the ~9 MB ancestry set" | various | none on disk | **U — struck** | the artifacts themselves | — |
| 31 | the 219 MiB smaps-vs-rollup drift | B3 | reproduced **219.4 MiB** (`PS` 1322.4 vs `PR` 1102.9) | **S** | a capture with drift < 10 MiB | UNT |
| 32 | `Σgb` total live heap = 410.8 pre / 94.5 post MiB | B1 | `dhat-crate-churn.py` output over `DHB`/`DHA` | **S** | a re-sum that disagrees | UNT |
| 33 | "peak live heap fell only 4.4× (411 → 94)" | B2 | 410.8/94.5 = 4.35× | **S** | a ratio outside 4.0–4.7 from a re-sum | UNT |
| 34 | "the 8× and the OOM behaviour come from different builds at different ceilings" | B2 | builds: **S** (`pps` 164,734 vs 158,238). **Ceilings: retracted** | split | a recorded ceiling per arm | UNT |
| 35 | "the profiled runs never reached the ceiling" | B1 | `EL:770-772` (`anon=302 MiB`, exit 0, 2500 MiB threshold) | **S** for the after arm; **P** for the before arm | the before arm's record, **or** an after-arm run that reached 2500 MiB | UNT |
| 36 | the before-profile's producing run is unrecorded | A1 | `DHB` metadata carries no tree/ceiling; preserve step never fired | **S** | a record naming it | UNT |
| 37 | `analyse-dhat.py` does not compute the figures it appeared to | B4 | `dhat-after.txt:49-60`; script source | **S** | a corrected run of *that script* reproducing 8,796.1/1,126.0 | UNT |
| 38 | `acceptance.log`'s `peak=0` entries are a read-path artifact | A1 | `AL:746-747`; `acceptance.sh`'s `PID=0` → `CG=/sys/fs/cgroup` | **S** | a nonzero peak on a dead node | UNT |
| 39 | the 64-MiB class is 11 regions / 658 MiB / 61 % of anon RSS | B3 | `PS`: 658.2 MiB; 61.0 % of rollup `Anonymous` | **S** | a re-count disagreeing | UNT |
| 40 | 29 × 32 MiB regions hold 5.71 MiB (0.5 %) | B3 | `PS`: 5.7 MiB over regions of size exactly 32 MiB | **S** | a re-count giving > 50 MiB, i.e. stacks that are actually resident | UNT |
| 41 | file-backed RSS is 24.7 MiB, not 26.8 | B3 | `PS`: 24.7 MiB over path-carrying regions excluding `[heap]`(1.9)+`[stack]`(0.2) | **S** | a region list in which all 26.8 MiB is genuinely file-backed | UNT |
| 42 | "death times at the same ceiling span ~24 s to ~7.5 min (a 20× spread)" | B2 | measured 17–186 s from first sample = 11× | **C** — overstated | a 450 s run on disk, **or** a first sample at container start | UNT |
| 43 | the smaps snapshot and the dhat profiles are different runs | B3 | `PS` **15:53:39 UTC**; `DHB` **16:42:46 UTC**; `DHA` **17:32:05 UTC**; only `TR` covers 15:53 | **S** | a shared run id | UNT |
| 44 | C-side allocations are invisible to a Rust-only profiler | A2 | `mode=rust-heap` in `DHB`/`DHA` | **S** | a C-side site appearing in the profile | UNT |
| 45 | blocking pool up to 512, 32 MiB stack each, **kept once grown** | A2 | `typed_store.rs:66,83,94,105,116,133`; `max_blocking_threads` set nowhere; `main.rs:71` | **S** as mechanism; the count at ceiling is a *different* claim | **mechanism**: a `max_blocking_threads` call in the tree, **or** a tokio version whose blocking threads are reaped — the thread count at death does **not** bear on this row (v2's was wrong) | **tracked** |
| 46 | growth continues when finality stops, 136–860 MiB/s | A4 | measured **215–918 MiB/s** (maximum 3-sample ramp per cluster) | **S**, bounds off by 79/58 MiB/s | a re-derivation outside **215–918** MiB/s (v2's 200–950 band was wider than the claim and guarded neither endpoint) | UNT |
| 47 | the flat survivor shows no transient dips | A4 | **C**: `R5` v2 swings 14.6→27.7 MiB (~90 %) and drifts 0.32 MiB/s; `TR` v2 run0 drifts 0.23 MiB/s | **C** | for this **C** verdict, evidence *for* the claim: a flat cluster whose peak-to-floor ratio is below 1.05 while growing clusters exceed 100× — i.e. survivors whose transients are categorically, not merely proportionally, smaller | UNT |
| 48 | H6 / C55 / C56 | register | — | **registered** — not re-reported anywhere in this document | — | — |
| 49 | `validated_blocks` is unbounded, carrying full `BlockMessage`s, and is the sibling of R15 | source | `node_runtime.rs:733`; `:2557` | **S** | a bound at either site | **tracked** |
| 50 | the log line's 750 is cumulative `INDEX_CALLS` (hits included), and `cache_len` grew 20→30→39 with 0 pruned | source/`L1` | `merging.rs`'s `INDEX_CALLS.fetch_add` runs before the cache lookup; `L1` 227/263/379 | **S** | a call counter incremented only on misses | UNT |
| 51 | the knee's causal edge is unidentified | this audit | the fault tree's `D` node (§8) | **S** (by the absence of any measurement that could identify it) | the measurement in §9 | — |

**Tracking count: 41 of 48 surviving claims are `untracked evidence`** (48 = 51 rows − 2 struck − 1
registered; the 7 tracked are rows 10, 19, 21, 24, 25, 45, 49).

---

## 7. The merged HAZOP table

Guide words are the house set at **`spec/audit/passes.md:498-504`** (v2 cited `:2944-2957`, which is the
tripwire passage). **Every `(node, parameter)` pair in §7.1 ends with a deviation or an `n/a: <reason>`.**
A `n/a` carrying a bare value instead of a reason is a defect and is fixed below.

| node | parameter | guide word | deviation | consequence | existing control | evidence class |
|---|---|---|---|---|---|---|
| N1 cgroup | `memory.max` | Less | 4 GiB/6 GiB/8 GiB/16 GiB per arm, recorded in the *script comment* (`monitor2.sh:9` 8g; `monitor4.sh:5` 16g; `monitor5.sh:6` 8g; `run-profile.sh:17` 6g vs its own `:4` "12g"; `monitor.sh`/`monitor3.sh` none), never in the arm file | no two arm files comparable; the ceiling only decides *when* (`AC` 17 s at 4 GiB vs `R5` 24 s at 8 GiB) | none | measured (`AL`, scripts) |
| N1 cgroup | `memory.swap.max` | n/a: pinned equal to `memory.max` (`devnet.sh:322-325`), so the node cannot swap out — no deviation exists because the pair is deliberate | — | — | measured (`devnet.sh`) |
| N1 cgroup | `memory.current` | Other than | sampled, but in `R3` the cgroup path fell back to the host root and read 15–30 GiB | a 1024×/host-wide reading was published before correction | `docker-` guard, `monitor4.sh:12-20` | measured (`R3`,`R4`) |
| N1 cgroup | `memory.peak` | Other than | quoted as the *anon* peak (`ISS` table); it is `memory.current`'s high-water — `R5` 15:14:08 `anon=8083.9`, `peak=8108.9` | 25.0 MiB of overstatement; not material | none | measured (`R5` cols 5 vs 8) |
| N1 cgroup | `memory.peak` read after death | No | returns 0 for exactly the nodes that died | two of three acceptance peaks missing | none | measured (`AL:746-747`) |
| N1 cgroup | `memory.high` | No | never set; the only control is the hard limit, so the node is killed rather than throttled | the failure mode is SIGKILL, not reclaim | none | measured (absent from `devnet.sh`, `AL`) |
| N1 cgroup | `oom_group` / `oom.kill` | n/a: not set, and there is only one process in the container, so group-kill semantics have nothing to act on | — | — | measured (absent) |
| N1 cgroup | `anon` | More | 8083.9 MiB peak on an 8 GiB cap (`R5` 15:14:08) | the top event | container limit only | measured (`R5`) |
| N1 cgroup | `file` | n/a: flat 1.6–2.5 MiB through every ramp, three orders of magnitude below the growth, so no deviation exists to report | — | — | measured (`R5 RR TR TP MM`) |
| N1 cgroup | `file_dirty` | n/a: ≈20 MiB at its one sample, two orders of magnitude below the ramp | — | — | measured (`R3`) |
| N1 cgroup | `file_writeback` | n/a: ≈20 MiB at its one sample — **the same fact and the same order-of-magnitude reason as `file_dirty` above**, and it is one of the two terms that makes `file` a non-owner | — | — | measured (`R3`) |
| N1 cgroup | `shmem` | n/a: `Pss_Shmem 0` (`PR`) — nothing is charged through tmpfs or `/dev/shm` | — | — | measured (`PR`) |
| N2 Rust allocator | `Σtb` ever allocated | More | 16,391.4 → 3,300.4 MiB (4.97×) on `devnet-bootstrap` | churn moved, not residency | none | measured (`DHB`,`DHA`) |
| N2 Rust allocator | `Σgb` held at global peak | More | 410.8 → 94.5 MiB (4.35×) | the visible live heap never approaches 4 GiB | none | measured (`DHB`,`DHA`) |
| N2 Rust allocator | max per-site `mb` (peak live at one site) | More | 56.9 → 19.6 MiB (2.90×) | no single site owns a GiB; the transient is diffuse | none | measured (`DHB`,`DHA`) |
| N2 Rust allocator | `Σeb` live at drop | No | 2.0 / 2.1 MiB at the end of `main` | **void as "no leak"**: the drop is post-shutdown | none | measured, and void (`run-profile.sh`'s comment) |
| N2 Rust allocator | return-to-OS (`MADV_FREE`, trim) | No | no `malloc_trim`/`mallopt` anywhere in the crate graph, and `unsafe` is forbidden | freed memory is retained by glibc | none | measured (repo grep) |
| N2 Rust allocator | peak-resident footprint per thread | More | **the deviation is that the quantity the fix must move is not any measured one**: `Σgb` fell 4.35× and `anon` still reached the ceiling, so whatever is retained does not scale with live heap. The only per-thread figures in the base are the 32-MiB stack *reservation* and the 5.7 MiB *residency* of the stack class | the fix space (allocator configuration) is inferred, not measured | none | inferred; the quantity itself is unmeasured |
| N2 Rust allocator | profile at the ceiling | No | no profile of a dying node exists; the after run stopped at `anon=302 MiB, exit 0` against a 2500 MiB threshold, and the before run is unrecorded | "the profile explains the OOM" is an inference across runs | none | measured (`EL:770-772`) |
| N3 glibc arenas | arena count | More | 11 regions of exactly 64 MiB / 13 at alignment, vs 29 threads — **not** one per worker as the docs say | the doc's mechanism story is wrong in its count | none | measured (`PS`) |
| N3 glibc arenas | heaps per arena | More | **two** VMAs of exactly 128.000 MiB and **one** of 127.996 MiB — the kernel merges adjacent heaps, so VMA count < heap count (v2 said "3 regions of 128 MiB"; the artifact has two of 128.000 and one of 127.996) | "64-MiB VMA = one arena" is false in general | none | measured (`PS`) |
| N3 glibc arenas | `HEAP_MAX_SIZE` | Other than | the 64 MiB constant is asserted **only in `arena-probe.sh`'s comment (`:6,:25`)**, never from the image or a measurement; and the classifier resting on it has three incompatible forms (`= 64 MiB`, `≡ 0 mod 64 MiB`, `≥ 63 MiB`, the last at `arena-probe.sh:44`) | `R` swings 658–1277 MiB and the frozen thresholds sit inside | none | measured (`arena-probe.sh:44`) + asserted-only |
| N3 glibc arenas | `fordblks` (`F` term 1) | No | **never measured** — no `mallinfo`/`malloc_info` anywhere | the frozen table's `F` cannot be evaluated | none | **unknown** |
| N3 glibc arenas | `hblkhd` (`F` term 2) | No | **never measured**; `PRE` defines `F = fordblks + hblkhd`, and `hblkhd` is the mmap'd-chunk term a 64-MiB heap contributes to | dropping it biases "arena retention accepted" (`F ≥ 0.8·R`) *against* acceptance | none | **unknown** |
| N3 glibc arenas | `uordblks` (`U`) | No | never measured | live-vs-retained cannot be separated | none | **unknown** |
| N3 glibc arenas | trim policy (`M_TRIM_THRESHOLD`, `M_MMAP_THRESHOLD`) | No | env-only; not recorded in any file; the `mmap` arm is named for it and nothing says what it set | no arm comparable; the arm named for this is unattributable | none | **unknown** |
| N3 glibc arenas | `MALLOC_ARENA_MAX` | Other than | the tree says "not shipped" and the **image agrees** (`grep -ic malloc docker/rnode/Dockerfile` = 0); the acceptance *container* had `=2` | the acceptance arm is not the arm it is labelled as; the docs' shipping claim survives | none | measured (`AL:744`, Dockerfile) |
| N3 glibc arenas | retention across a sawtooth | More | survivor `TR` v1 floors at exactly 2300.4 MiB and re-climbs 9× while peaks reach 3711 MiB; nothing returns | each transient raises a floor | none | measured (`TR` 15:54:59–15:57:25) |
| N4 C-side malloc | `lmdb-rkv-sys` allocations | No | invisible to `mode=rust-heap`; they land in **the same arenas** as N3, so `PS` cannot separate them | "C-side" and "arena" are one measurement, not two | none | **unknown** |
| N4 C-side malloc | other C libraries (libc, `ld.so`, TLS) | No | same arenas, same invisibility; unmeasured and unlisted in round 1 | a third contributor to the same bytes is unaccounted for | none | **unknown** |
| N4 C-side malloc | Rust-vs-C attribution inside an arena | No | nothing in the tree attributes bytes within a heap to a caller | the fix space cannot be narrowed to Rust or C | none | **unknown** |
| N5 LMDB | map size per environment | More | 1 TiB × 6 envs + 10 TiB `reporting` + 100 GiB `dagstorage` = 15.8 TiB virtual | virtual only; RSS 24.7 MiB | none | measured (`PM`) |
| N5 LMDB | dirty pages | n/a: `file_dirty` ≈ 20 MiB at its one sample, three orders of magnitude below the ramp | — | — | measured (`R3`) |
| N5 LMDB | read transactions (reader table) | No | one `MDB_txn` slot per concurrent reader in the lock mmap; no artifact counts them, and `spawn_blocking` per operation makes readers the default state | cannot be excluded | none | **unknown** |
| N5 LMDB | map size vs ceiling | Other than | a 1 TiB reservation inside a 4 GiB cgroup is legal but means the limit is on resident pages, not on the store | none | measured (`PM`) |
| N6 threads | worker thread count | n/a: `default_worker_threads()`; 29 stacks + main observed, matching the design | — | — | measured (`PT` = 30, `PS`) |
| N6 threads | worker stack reservation | n/a: `thread_stack_size(32 MiB)` at `main.rs:71`, 32 MiB × 29 = 928 MiB virtual, exactly as designed | — | — | measured (`PS`, source) |
| N6 threads | worker stack residency | n/a: 29 × 32 MiB regions hold **5.7 MiB of RSS**, so stacks are refuted as an owner at 1151 MiB | — | — | measured (`PS`) |
| N6 threads | blocking-pool thread count | More | every LMDB op is `spawn_blocking` (`typed_store.rs:66,83,94,105,116,133`); `max_blocking_threads` is set nowhere → tokio's 512 default | unbounded × 32 MiB stack × one arena each | none | inferred from source; count **unknown** |
| N6 threads | blocking-pool stack | More | the same 32 MiB `thread_stack_size` applies to blocking threads | 512 × 32 MiB = 16 GiB of *virtual* reservation | none | inferred from source |
| N6 threads | arena-per-thread | More | glibc gives a contending thread its own arena, up to `MALLOC_ARENA_MAX` (default `8 × ncores`) | the only measured route from 913 MiB of heaps to 4 GiB | none | inferred; the cap is untested here |
| N6 threads | thread count at the ceiling | No | sampled once, at `anon` ≈ 1151 MiB: 30 | **the number that would settle §2** — under the linearity, 30 refutes the arena-per-thread route and **≥ ~145** supports it | none | measured at 1151 MiB only (`PT`) |
| N7 block/merge | merge/index calls per block | More | `INDEX_CALLS` 250 at h≈13 → 750 at h≈15 (`L1` 227, 379), counted on **every** call including hits | per-block merge work grows with the DAG — **candidate `D2`** | none | measured (`L1`) |
| N7 block/merge | index cache entries | More | 20 → 30 → 39 across four log lines | unpruned while finality is frozen | `prune_cache` | measured (`L1`,`L2`) |
| N7 block/merge | index cache pruned | Less | **0 pruned in all four log lines**, including at the moment finality first advances | the prune path never fires on this reproduction | `merging.rs:575-597` | measured (`L1`) |
| N7 block/merge | index cache bytes | No | entries are counted, never sized; `merging.rs` says "kilobytes to megabytes" | a Θ(N)-entry cache with unknown bytes cannot be excluded | none | **unknown** |
| N7 block/merge | replay fallbacks | More | 1 fallback / 603 ms cumulative (`L1` 379) — a full block replay, persisted | cost, and a per-block Θ(block) allocation | none | measured (`L1`) |
| N7 block/merge | fork fan-in (justifications per block) | As well as | 3-way fork at every height 2–13; `L2` shows duplicate `close_block 14` lines; `R2`/`S1` count `InvalidBlockNumber` but **no artifact counts justifications per block** | the merge scope's size is unmeasured; candidate `D3` | none | partial (`L2`, `R5` height traces) |
| N7 block/merge | fork fan-out (proposals per height) | As well as | every validator proposes at every height 2–13 (`L1`: bootstrap 14, v1 12, v2 44 `proposed and added`) | the reproduction's defining condition | none | measured (`L1`) |
| N7 block/merge | `validated_blocks` channel | More | production `mpsc::unbounded_channel()` at `node_runtime.rs:733`, carrying full `BlockMessage`s | **the sibling of R15 survives** — §10 | none | measured (source) |
| N7 block/merge | tap channel | More | second `unbounded_channel()` at `:2557`, live whenever autopropose or attest-on-new-blocks is on | same class, one layer down | none | measured (source) |
| N7 block/merge | `incoming_blocks` channel | n/a: bounded at `MAX_PENDING_BLOCKS = 1024` (`node_running.rs:455`, used `:732`) — the bound exists, so no deviation | — | R15's fix | measured (source) |
| N7 block/merge | `processor_input` channel | n/a: bounded at 1024 (`node_runtime.rs:764-781`), comment cites R15 | — | R15's fix | measured (source) |
| N7 block/merge | `:2090` channel | n/a: **inside `#[cfg(test)]` (`node_runtime.rs:2014`)**, so it is not reached by any node | — | — | measured (source) |
| N7 block/merge | proposer loop | Before/After | `v2` proposed 44 blocks against bootstrap's 14 in the same window and stayed at 27.7 MiB | proposal is not the driver | none | measured (`L1`) |
| N8 DAG gauges | `logical_bytes` | No | **never recorded in any artifact**, though `PRE` names it as the instrument that settles DAG residency directly | the DAG-residency question is unsettled by construction | exists as a gauge, unread | **unknown** |
| N8 DAG gauges | `seen_entries` | No | likewise never recorded | as above | none | **unknown** |
| N8 DAG gauges | `message_count` / `fringe_states` / `index_entries` | No | likewise never recorded (`dag.rs` publishes all of them) | as above | none | **unknown** |
| N8 DAG gauges | child/height maps | No | likewise never recorded | as above | none | **unknown** |
| N8 DAG gauges | `Message.seen` residency | More | **scope-out, with the reason:** H6 registers this residency as an accepted residual, and #117's artifact set contains no replay-time footprint at all (D-2), so this audit can neither measure nor exclude it. It is **not** claimed as a contributor and **not** claimed as excluded — the fault tree draws it as a registered edge rather than leaving it off | the fault tree must not be read as having eliminated H6 | H6 | registered |
| N9 page cache | file RSS | n/a: 24.7 MiB of true file-backed mapping, two orders below any ramp, so no deviation exists to report | — | — | measured (`PS`,`PR`) |
| N9 page cache | kernel-named anon (`[heap]`, `[stack]`) | Other than | `[heap]` `Rss` 1.926 MiB / `Size` 1.961 MiB and `[stack]` 0.2 MiB are anonymous but carry a path, so a path-based classifier miscounts them as file-backed (the 26.8 figure) | 2.1 MiB misattributed | none | measured (`PS`) |
| N9 page cache | `Private_Dirty` / `Pss_File` | n/a: `Private_Dirty` 1079.0 MiB ≈ `Anonymous` 1078.9 MiB, and `Pss_File` 8.6 MiB — essentially all of `Rss` is anonymous private dirty, so no deviation exists to report | — | — | measured (`PR`) |
| N9 page cache | readahead / `MDB_NORDAHEAD` | No | not set or measured; it could inflate `file`, which is 1.8 MiB | immaterial by measurement | measured by absence of effect (`R5`) |
| N10 kernel | `AnonHugePages` / THP | No | one sample, `PR` = **0**, but no per-second series exists. **Relevant in two ways:** THP would (i) inflate `anon` for the same page count and (ii) block return-to-OS, since a partially-used 2 MiB page cannot be freed; and a THP region is `Rss ≈ Size` at 2 MiB granularity, which is exactly the signature Amendment 1's arena classifier keys on | `R` could be measuring THP rather than arenas, and no artifact rules it out over time | none | measured once (`PR`), **unknown** over time |
| N10 kernel | page tables / kernel stacks / slab (`kmem`) | No | not sampled; for a 30-thread process these are 10s of MiB, not GiB — but that is an estimate | cannot be *excluded* from the artifacts | none | **unknown** |
| N10 kernel | socket buffers | No | not sampled; ~20 peer connections, immaterial by scale | none | **unknown** |
| N11 instrumentation | container-id → cgroup path | Other than | `acceptance.sh` resolves via `/proc/$PID/cgroup` with `PID` read after death | the `peak=0` defect | `monitor4/5.sh` re-resolve each iteration; `acceptance.sh` does not | measured (`AL:746-747`) |
| N11 instrumentation | cgroup unit (bytes vs KiB) | Other than | `monitor3.sh` divided v2 bytes by 1024 → a 1024× "41 GiB anon" | a withdrawn reading | corrected in `monitor4.sh` | measured (`R3`,`R4`) |
| N11 instrumentation | host-root fallback | Other than | `monitor3.sh`'s unresolved path fell back to `/sys/fs/cgroup` and read the host | 15–30 GiB readings are the machine | `docker-` guard (`monitor4.sh`) | measured (`R3`) |
| N11 instrumentation | smaps sampling interval | No | one one-shot capture, threshold-triggered; `PRE` requires it per second | `R` exists at one instant and not at death | none | measured (D-2) |
| N11 instrumentation | smaps region classifier | Other than | three incompatible rules in the base plus an amendment (`PRE:89-120`) that narrows but does not close it (§5) | `R` ∈ 658.2 / 913.2 / 1018.9 / 1030.1 MiB on **one** snapshot | none | measured (`PS`, `arena-probe.sh:44`, `PRE:89-120`) |
| N11 instrumentation | smaps totals | Other than | `Σ`(smaps `Rss`) exceeds `smaps_rollup Rss` by **219.4 MiB** across files written ~90 ms apart | single-point totals soft by ≈20 % | same-instant rule added (`PRE`) but unsatisfiable by this artifact (§5) | measured (`PS` vs `PR`) |
| N11 instrumentation | clock discipline | Other than | artifact mtimes are **local (BST)**, sample columns and logs are **UTC**; v2 compared the two directly | a timestamp comparison that put the snapshot in no run, when it is in `TR` run 2 | none | measured (`e2e.log` anchor) |
| N11 instrumentation | thread sampling | No | once, at 1151 MiB | §2's discriminator is unavailable | none | measured (`PT`) |
| N11 instrumentation | node gauge collection | No | `PRE` requires it; nothing collects it | the direct DAG-residency instrument was never read | none | measured (D-2) |
| N11 instrumentation | per-arm config recording | No | `PRE` requires the config in filename and header; no file does it, and D-3 shows the launchers are absent | arms are incomparable in principle; the run count itself is rule-dependent (D-4) | none | measured (D-3, D-4) |
| N11 instrumentation | profile preserve step | No | `wait-and-measure.sh`'s `cp` guard never fired | the before-profile's producing run is unrecorded | none | measured (`EL`, `DHB`) |
| N11 instrumentation | `dhat-heap.json` overwrite | Other than | `md5` identical to `dhat-heap-after.json` | the default path now holds the after run | none | measured |
| N11 instrumentation | profile drop timing | Early/Late | the dump is written when the `Profiler` guard drops at the end of `main` — **after** the shutdown drain | a leak freed at shutdown reads identically to no leak | none | measured (`run-profile.sh`, `DHB`/`DHA` `Σeb`) |
| N11 instrumentation | analysis script | Other than | `analyse-dhat.py` emits `42.4 MiB 0x61ff475ee30e: tokio`; headline is `max(gb)`; crate table sums `mb` | the crate numbers were never produced by an on-disk script — **now fixed by `dhat-crate-churn.py`** | `spec/audit/evidence/dhat-crate-churn.py` | measured (`dhat-after.txt:49-60`) |
| N11 instrumentation | `arena-probe.sh` arena count | No | "29 regions of exactly 64 MiB, 13 with RSS" is a literal in a trailing `echo`; the probe could not sample 2 of 3 nodes | the probe's own output is a constant | none | measured (`arena-probe.sh`, `AP:4-5`) |

### 7.1 Pair index (for mechanical completeness checking)

`N1 cgroup`: memory.max ✓ · memory.swap.max ✓ · memory.current ✓ · memory.peak ✓ · memory.peak-after-death ✓ · memory.high ✓ · oom_group ✓ · anon ✓ · file ✓ · file_dirty ✓ · file_writeback ✓ · shmem ✓
`N2 Rust allocator`: Σtb ✓ · Σgb ✓ · max per-site mb ✓ · Σeb ✓ · return-to-OS ✓ · peak-resident-footprint-per-thread ✓ · profile-at-ceiling ✓
`N3 glibc arenas`: arena count ✓ · heaps per arena ✓ · HEAP_MAX_SIZE ✓ · fordblks ✓ · hblkhd ✓ · uordblks ✓ · trim policy ✓ · MALLOC_ARENA_MAX ✓ · retention across a sawtooth ✓
`N4 C-side malloc`: lmdb-rkv-sys ✓ · other C libraries ✓ · Rust-vs-C attribution ✓
`N5 LMDB`: map size per env ✓ · dirty pages ✓ · read transactions ✓ · map size vs ceiling ✓
`N6 threads`: worker count ✓ · worker stack reservation ✓ · worker stack residency ✓ · blocking-pool count ✓ · blocking-pool stack ✓ · arena-per-thread ✓ · thread count at ceiling ✓
`N7 block/merge`: merge/index calls per block ✓ · index cache entries ✓ · index cache pruned ✓ · index cache bytes ✓ · replay fallbacks ✓ · fork fan-in ✓ · fork fan-out ✓ · validated_blocks channel ✓ · tap channel ✓ · incoming_blocks channel ✓ · processor_input channel ✓ · `:2090` channel ✓ · proposer loop ✓
`N8 DAG gauges`: logical_bytes ✓ · seen_entries ✓ · message_count/fringe_states/index_entries ✓ · child/height maps ✓ · Message.seen ✓
`N9 page cache`: file RSS ✓ · kernel-named anon ✓ · Private_Dirty/Pss_File ✓ · readahead ✓
`N10 kernel`: AnonHugePages/THP ✓ · page tables/kernel stacks/slab ✓ · socket buffers ✓
`N11 instrumentation`: cgroup path resolution ✓ · units ✓ · host-root fallback ✓ · smaps interval ✓ · smaps classifier ✓ · smaps totals ✓ · clock discipline ✓ · thread sampling ✓ · gauge collection ✓ · per-arm config ✓ · preserve step ✓ · dhat overwrite ✓ · profile drop timing ✓ · analysis script ✓ · arena-probe constant ✓

**82 pairs** (N1 12 · N2 7 · N3 9 · N4 3 · N5 4 · N6 7 · N7 13 · N8 5 · N9 4 · N10 3 · N11 15), **82 filled.**
Every pair ends in a deviation or an `n/a: <reason>`; no guide word is forced; every parameter named in
§8's fault tree has a row.

---

## 8. The merged fault tree

Top event: **`rnode` is killed by its cgroup memory controller.** Every edge is `measured` (artifact
named), `inferred` (with the measurement that would settle it), or `unknown` (with why).

```
TOP  killed by cgroup memory controller         [measured: AL:746-747 exit=137 oom=true;
 │                                                         R5 alive=0 rows]
 │
 ├─ G1 anon reaches memory.max with nothing reclaimable    [measured: R5 8083.9 MiB vs 8 GiB;
 │   │                                                                file 1.9 MiB]
 │   │
 │   ├─ G1.1 anon grows                     [measured]
 │   │    ├─ A per-event transient far above the floor  [measured: TR v1 floor 2300.4 MiB,
 │   │    │                                               per-second peaks to 3711 MiB, 9 cycles,
 │   │    │                                               TR 15:54:59–15:57:25]
 │   │    ├─ B a sustained ramp, height-independent     [measured: R5 650 MiB/s with height frozen
 │   │    │                                               at 15, 15:13:57–15:14:08]
 │   │    └─ C A/B begin within 0–2 s of finality's first advance
 │   │           [measured: 25/25 physical-run clusters, K1abs; 24/25 under K2consec, §1]
 │   │         └─ D what the knee represents            [ **UNKNOWN** — no artifact in the base]
 │   │              ├─ D1 finality switches on the expensive path (commit / final scope / merge)
 │   │              │      [inferred — artifact: none. Settled by: the frozen reproduction with
 │   │              │       finality held frozen (validators below the fringe quorum) while blocks
 │   │              │       keep arriving; flat anon would establish D1]
 │   │              ├─ D2 the merge scope goes super-linear at that depth
 │   │              │      [inferred — artifact: L1 bootstrap 227/379 and L2 15:47:06, index calls
 │   │              │       250@h13 → 750@h15. Settled by: INDEX_CALLS logged as a rate per block]
 │   │              ├─ D3 both are symptoms of a third event (the DAG becoming a fork fan)
 │   │              │      [unknown — no artifact counts justifications per block. Settled by:
 │   │              │       fork fan-in sampled per block, or a 1-validator arm that also finalizes]
 │   │              └─ D4 the node was already diverging and finality is a marker
 │   │                     [unknown — settled by: the survivor's and the victim's last finalised
 │   │                      block hashes at the failing height, the check spec/audit/passes.md §25
 │   │                      already proposes for #105]
 │   │
 │   ├─ G1.2 freed memory is not returned to the OS  [inferred — no artifact measures return-to-OS]
 │   │    ├─ E glibc arena retention
 │   │    │      [inferred; measured *signature*: the contiguous 0x6456… block, 13 heap-scale VMAs,
 │   │    │       1018.9 MiB Rss, fill 0.99–1.00, against Rss 1102.9 (PS/PR, §5, §9); sawtooth
 │   │    │       floors at TR v1. The ONE unmeasured step is F vs U — settled by mallinfo2, §9]
 │   │    ├─ F a live Rust structure
 │   │    │      [evidence VOID: DHB/DHA drop post-shutdown (Σeb 2.0/2.1 MiB) and neither profiled
 │   │    │       run reached the ceiling (EL:771). Settled by: a profile taken at the ceiling]
 │   │    ├─ G thread stacks  [REFUTED: 29×32 MiB regions, 5.7 MiB RSS — PS]
 │   │    ├─ H LMDB C-side malloc (shares E's arenas)  [unknown — not separable by smaps;
 │   │    │       settled by malloc interposition]
 │   │    ├─ I the two unbounded channels  [unknown — node_runtime.rs:733, :2557; no depth gauge.
 │   │    │       Settled by: one depth counter]
 │   │    ├─ J BLOCK_INDEX_CACHE  [measured entries 39 / pruned 0 (L1); bytes never measured.
 │   │    │       Settled by: sizing the cache once]
 │   │    └─ K DAG Message.seen residency  [registered H6 residual — drawn, not eliminated (§7 N8)]
 │   │
 │   └─ G1.3 nothing caps the growth                 [measured]
 │        ├─ L no bound stated anywhere for anon     [measured: ISS:1-3 "Closes when … a stated
 │        │                                            bound"; no bound in the tree]
 │        └─ M blocking pool unbounded, one arena + 32 MiB stack per thread
 │               [inferred from source: typed_store.rs ×6 spawn_blocking; max_blocking_threads
 │                unset; main.rs:71. Settled by: thread count at the ceiling — 30 refutes the
 │                route to 4 GiB, ≥ ~145 supports it (§2)]
 │
 └─ G2 file/page cache counted toward the limit     [REFUTED: measured — file 1.8–2.5 MiB
        throughout; Pss_File 8.6 MiB; file_dirty ≈ 20 MiB; Pss_Shmem 0]
```

**Critical path, shipped as such:** `TOP ← G1 ← G1.1 ← C ← D`, with **`D` `unknown`**, and in parallel
`TOP ← G1 ← G1.2 ← E` with the single step `F`-vs-`U` `unknown`. **The path is not resolvable from the
artifacts.** Two edges are unknown for two different reasons — one measurement was never taken
(`mallinfo2`) and one dimension was never sampled (thread count; `logical_bytes`/`seen_entries` over
time). Nothing here promotes either to `inferred`.

**A named secondary path that is fully measured until it is not:** `G1.1 ← B` ← `N7 merge/index calls
per block` (`INDEX_CALLS` 250@h13 → 750@h15, `L1`) ← `N7 fork fan-in`, which has **no artifact**. Even
the best-measured chain ends at an unmeasured parameter.

Registered cut-set edges (H6, C55, C56, R15's two bounded channels) are drawn and are **not** on this
critical path.

---

## 9. The single discriminating measurement

**It is the pre-registration's Stage A, as amended by `7a9f293fa`. It has never been run (D-2).**
Procedure, with the corrections §5 shows are still owed:

1. The frozen reproduction: `DEVNET_NODE_MEMORY=4g DEVNET_NODE_ENV= tools/devnet.sh up --validators 3
   --stakes 100,100,50 --epoch-length 10 --fresh` — **and the container's env is dumped and logged
   before the run**, because `AL:744` shows an inherited `MALLOC_ARENA_MAX=2` reaching a run labelled
   "without the arena cap".
2. Per second, until the container dies: `anon`, `file`, `file_dirty`; `memory.current`; `memory.peak`
   **read while alive**; `smaps_rollup`; `smaps` with the per-region list emitted; thread count and the
   count of 32-MiB regions; the node's own `logical_bytes` and `seen_entries`; and the cgroup read
   **at the same instant as the smaps read**, per the amendment.
3. `F = fordblks + hblkhd` and `U = uordblks` from `mallinfo2()` via a host-compiled `LD_PRELOAD` shim
   — the one piece of new machinery `PRE` already permits, and the only thing that separates the
   table's first row from its second.

**The frozen criteria, verbatim (`PRE`, unamended by `7a9f293fa`):** `R/A ≥ 0.8 ∧ F ≥ 0.8·R` → arena
retention accepted. `R/A ≥ 0.8 ∧ F ≪ R ∧ U ≥ 0.8·R` → refuted as "freed". `R/A ≥ 0.8 ∧ F ≪ R ∧ U ≪ R` →
mislabelled. `R/A < 0.5` → arenas refuted. **Report `R/A`**, with its sample time, `memory.peak` at
death, and the arm's recorded configuration.

**What is still owed, and why the table cannot decide its own artifact.** On the one snapshot that
exists: `R` = 1018.9 MiB (amended rule + heap-scale floor), taken at `anon` ≈ 1151 MiB; the arm died at
`anon` = 5847.6 MiB. So `R/A` is **0.885** if `A` is the same-instant `anon` and **0.174** if `A` is
`memory.peak` at death — the "accepted" row and the "refuted" row from one artifact and one rule. Four
things must be fixed in a v2 pre-registration before this measurement can produce a verdict, none of
which invents a threshold:

1. **`A` must be defined as the same-instant `anon`** (or `R/A` must be defined as a same-instant ratio
   and the peak-at-death reported separately). The frozen text mixes them.
2. **`R`'s classifier needs a size floor and a stated adjacency tolerance** — read literally, the
   amendment admits a 0.766 MiB full region and excludes three 64-MiB full heaps (§5).
3. **The `[0.5, 0.8)` band has no row** — a strict reading of the pre-amendment rule (0.597) lands
   there and produces no verdict.
4. **The thread count and the 32-MiB region count, per second**, which the frozen table does not carry
   and which §2 shows is the cheap discriminator between "arenas grow per thread" and "a fixed set of
   arenas retains more". Measured with `ls`, not with a profiler.

**Thresholds are not invented here.** Items 1–4 are corrections to collectability and definition; the
gap in item 3 is reported as a hole rather than filled with a number chosen after seeing the data.

---

## 10. Register context, and the unbounded-channel inventory

**R15's citation is stale, and the sibling is confirmed present.** `spec/findings.tsv:84` cites
`node_runtime.rs:531`. **`let mut admin = tokio::spawn({` is at `:531`** — v2's sentence calling `:529`
the spawn contradicted itself; the correct statement is that `:531` opens the admin-server spawn and
the C112/C121 comment follows it. The unbounded channels in this pipeline are at **`:733`** (production,
full `BlockMessage`s) and **`:2557`** (the tap, live whenever autopropose or attest-on-new-blocks is on).
**The correct register row is R15 (C1, section 13)** — same class, same file, other half of the same
pipeline. R15's fix is real and its comment cites the row at `:764-781`; it bounded the *processor-input*
channel and left the output side unbounded.

**Inventory, exactly (the coordinator's correction is right and worth stating precisely).** A whole-tree
`grep` for `unbounded_channel` finds five sites:

| site | type | production? |
|---|---|---|
| `node/src/runtime/node_runtime.rs:733` | `BlockMessage` | **yes** — this pipeline's output side |
| `node/src/runtime/node_runtime.rs:2557` | `BlockMessage` | **yes** — the tap |
| `casper/src/blocks/block_receiver.rs:538` | `BlockHash` | yes, but **not** this pipeline and not `BlockMessage` |
| `casper/src/engine/lfs_block_requester.rs:226` | `BlockHash` | yes, same caveat |
| `node/src/runtime/node_runtime.rs:2090` | `BlockHash` | **no** — inside `#[cfg(test)]` (`:2014`) |

So: **two in the pipeline under audit, four in the tree, one of the four a test fixture.** "Two in the
tree" would be false, and v2 said "two in this pipeline", which is correct but was written without the
distinction — it is stated here so no register row is drafted from the wrong count. The two `BlockHash`
channels are recorded as unexamined, not as clean.

**Not re-reported:** H6 (`spec/findings.tsv:30`, the Θ(N²) `Message.seen` **residency** as a
deliberately accepted residual); C55 and C56 (`:161-162`, both `done`). None appears as a finding above;
C56's merge-scope copying is referenced only as one reason the `INDEX_CALLS` counter exists.

---

## 11. The verdict — stated, with the corrections applied

`AL:740-749` (v2 quoted this as starting at `:743`; the block starts at `:740`):

```
build exit: 0 at 17:50:31
devnet up at 17:50:39 — watching for 180 s
--- the image's allocator default actually reached the container? ---
MALLOC_ARENA_MAX=2
--- per-node peaks (cgroup memory.peak; monotonic, authoritative) ---
  devnet-bootstrap       peak=  0 MiB  status=exited oom=true exit=137
  devnet-validator-1     peak=  0 MiB  status=exited oom=true exit=137
  devnet-validator-2     peak= 18 MiB  status=running oom=false exit=0
--- still up: 1 of 3 (pre-fix: 0 of 3; fixed without the arena cap: 1 of 3) ---
```

**The verdict sentence, unchanged, and it still stands after every correction in this file:**

> **The symptom is confirmed. The fix is not. The mechanism is unmeasured.**

**Why each clause survives, after the counts and the arithmetic moved:**

- **The symptom is confirmed**, and the strongest evidence for it is *not* the acceptance arm. The
  acceptance arm is compromised as a statement about the frozen reproduction (below); what is
  uncompromised is the body of 25 growing physical-run clusters, all of which ramp to 3.0–8.1 GiB and
  die, on five of the seven arms, at ceilings of 4 and 8 GiB. The acceptance arm adds a further death
  at 4 GiB on the tip.
- **The fix is not [shown].** Two things must both be said. First, the acceptance arm is **not the
  frozen reproduction**: `acceptance.sh` never sets `DEVNET_NODE_ENV`, so the run inherited an
  environment, and `AL:744` proves it carried `MALLOC_ARENA_MAX=2` — an override the *image* does not
  ship (`grep -ic malloc docker/rnode/Dockerfile` = 0, row 19 downgraded to **S** for that reason).
  **So the honest frame is: either the verdict is not about the frozen reproduction, or the frozen
  reproduction was not run — and the second is the case, because no artifact shows it being run.**
  Second, that error runs *in favour of* the negative verdict rather than against it: the arm that died
  was the arm **most favourable to the fix** (the arena cap set), and it still lost 2 of 3 nodes. A
  negative claim survives an arm that was more favourable to the positive one.
- **The mechanism is unmeasured.** The fault tree's `D` is `unknown` with four undistinguished
  candidates, and `F`-vs-`U` at the ceiling has never been taken (D-2). §5 now adds a third: the frozen
  table cannot be applied to its own best artifact, because `R` is measured at 1151 MiB and `A` is
  defined at death, and the two readings land in opposite rows.
- **The issue's own closure criterion is unmet on both disjuncts** (`ISS:1-3`: a re-run showing `anon`
  flat, **or a stated bound**, across a stalled period): no run shows `anon` flat under a stall, and no
  bound exists anywhere in the tree.

**Would any correction change the verdict?** No, and the one that came closest is stated rather than
absorbed: **row 19's downgrade from C to S removes one leg of v2's argument that the audit's own
acceptance run contradicted the docs.** That leg was doing real work — v2 used it to say "the arm is not
the arm it claims". The replacement is *narrower and still sufficient*: the image ships no cap (row 19,
S), the container had one (`AL:744`), so the arm's own label "fixed without the arena cap" is false even
though the docs' shipping claim is true. The verdict rests on the deaths, not on the label, and the
deaths are unaffected. **Nothing else in the correction list touches the sentence**: the arithmetic
fixes change magnitudes (960.0 not 832; 1018.9 not 913.2; ~145 threads not ~62), the count fixes change
N (25 not 31 not 16) and not the ratio, the citation fixes change addresses, the tracking column changes
provenance, and the amendment to `R` makes the measurement harder rather than easier.

---

## 12. What I killed, and what v2 got wrong

**Killed as claims (all sources):**

1. **"The trigger is forked block processing"** (orchestrator). Contradicted by `TR` v2 run0: 82 heights,
   forked through 17, flat at 39.3 MiB, 94 `close_block` and 44 proposals. The reasoning that produced
   it inverts its own evidence.
2. **"The survivor's flatness refutes the finality correlation."** It supports it; the two flat clusters
   are the only two that never moved finality.
3. **"29 anonymous regions of exactly 64 MiB — `HEAP_MAX_SIZE`, one per worker — fully resident"**
   (`validator-requirements.md:120-121`, repeated in `5d3c55022`). 29 regions are **32 MiB**, hold
   **5.7 MiB**; the count is from the stack class and the size from the arena class.
4. **"`[heap]` 1.9 MiB and file-backed RSS 26.8 MiB."** `[heap]` ✓ (`Rss` 1.926 MiB); file-backed is
   24.7 MiB.
5. **"16.4 GiB allocated against 2 MiB still live at exit" for "that run".** 16,391.4 MiB = 16.007 GiB,
   and the profile (16:42:46 UTC) and the snapshot (15:53:39 UTC) are different runs.
6. **"nothing is leaked"** from 2 MiB live — void: the drop is post-shutdown.
7. **"changed time-to-death by about two seconds"** — no artifact; struck.
8. **"over one stall with 711 of them hits"** — cumulative `INDEX_CALLS`, not per stall.
9. **"death times span ~24 s to ~7.5 min (a 20× spread)"** — artifacts give 17–186 s from first sample.
10. **"the fixed without the arena cap: 1 of 3"** — the container's env had `MALLOC_ARENA_MAX=2`.
11. **"`memory.peak` cannot miss a spike"** — it missed exactly the two nodes that died.
12. **"the flat survivor shows no transient dips"** — the survivors swing ~90 % on their own scale.
13. **v2 "height 41, peak 25.1 MiB"** — the artifact reads 45 and 27.7.
14. **R15 "unbounded block-validation pipeline … done"** — half of it is done.
15. **`analyse-dhat.py`'s headline and crate table** — `max(gb)` under a `Σgb` label; pseudo-crates.
    (Superseded by `dhat-crate-churn.py`, which is now the instrument of record.)
16. **Every "peak MiB" for a dead node in `acceptance.sh`** — not measured.
17. **`arena-probe.sh`'s "pre-fix measurement: 29 regions of exactly 64 MiB, 13 with RSS"** — a literal.
18. **Any single-point total from `PS`** as a precise figure — 219.4 MiB (≈20 %) drift in the same capture.
19. **A3's `29 × 64 MiB` gap analysis** — right arithmetic on the wrong premise; the real gap points at
    the thread count.
20. **"post-fix runs 2 and 3"; "37 GiB free"; "~130 s vs 132 s"; "the ~9 MB ancestry set"; the profiled
    comparison as one experiment** — no artifact; struck.

**Retracted from v2 (six statements, this is the full list):**

- **v2's "true but orphaned"** for the 7.81× figure. **Retracted.** The rule was unwritten, not absent;
  `spec/audit/evidence/dhat-crate-churn.py` (`c0d7fd728`) states it and reproduces 8,796.1 → 1,126.0 and
  16,391.4 → 3,300.4, and I ran it.
- **v2's "two independent knee rules (K1 relative, K3 absolute)". Retracted.** K1's relative term is
  vacuous — max baseline 37.6 MiB, so `3·baseline ≤ 112.8` and K1 is identically an absolute 150 MiB
  threshold. The surviving cross-check is K1abs against K2consec.
- **v2's N = 31 growing / 2 flat. Replaced** with **27 physical-run clusters / 25 growing / 2 flat**
  under a stated interval-union rule, with the rule-dependence stated: the gate's clustering gives
  28/26, mine 27/25, and the difference is one cluster caused by D-3/D-4.
- **v2's reason for K2's failure. Retracted** — 19 of 25 growing clusters start below 15 MiB, which is
  why the guard blocks them; v2 blamed the monitor's start point.
- **v2's four arithmetic errors. Corrected**: 960.0 MiB of `Size` (not 832 MiB, which was impossible);
  two 128-MiB VMAs plus one 127.996-MiB VMA (not three of 128); **~145 threads** for 4 GiB (not ~62);
  `[heap]` 1.96 MiB (not 2,008); `Pss_File` 8.6 MiB (not 8.8, which was a /1000 slip); and the
  local-vs-UTC clock error that made the snapshot look like it fell in no run.
- **v2's row 19 verdict C. Downgraded to S**, with the narrow replacement kept in 19a.

**Holes reported as holes, not filled:** the frozen table's `[0.5, 0.8)` band; the amended `R`
classifier's missing size floor and adjacency tolerance; `F`/`U` never measured; `HEAP_MAX_SIZE`
asserted only in a script comment; fork fan-in never counted; `logical_bytes`/`seen_entries` never read;
the before-profile's producing run unrecorded and its launcher absent; THP sampled once and never ruled
out over time; the same-instant rule unsatisfiable by the artifact it was written about; and the
physical-run count itself, which the base cannot make determinate.
