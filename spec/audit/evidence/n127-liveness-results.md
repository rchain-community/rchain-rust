# The after-arm: finality stops 87 seconds before the kill, and the kill is not the variable

Run 2026-09-30 on tree `ef412ef84`, tree `c5442ee1f` for the before-arm. Every number below is
`n127-liveness-summarise.py`'s output over the series, not a reading of a log — the program exists because
the harness computed these rows at run time and printed them to a terminal, which is why the figure in
`#126` and in `C185`'s title came from someone reading a gitignored `target/` directory.

```
python3 spec/audit/evidence/n127-liveness-summarise.py \
    spec/audit/evidence/n127-liveness/*/ \
    spec/audit/evidence/n127-campaign/c5442ee1f-20260930T163518Z
```

**Two changes to the artifacts were made in the same commit, and both are disclosures rather than
readings.** The after-arm's three runs are committed here for the first time — until now the only after-arm
artifact in the tree was one attempt's stall logs, and the series every number was taken from existed only
under `target/`. And each `mergelog-*`/`stall-*`/`deploys-*`/`budget-*` file gained a leading
`# provenance:` line naming its tree, node and attempt: the same files had no tree of their own, which is why
a log copied out of its directory could not say what it measured (C176's class). No log line was altered,
and the summariser's counts are unchanged by the addition — verified by re-running it over both the old and
the new files.

## The pre-registered row, as frozen before the run

`n127-liveness-preregistration.md` fixes one number: **the finalised height at T+120 (the kill) and at
T+300, per node per attempt, beside the height at the same instants**, and one acceptance row: *finality
advances on the survivors after the kill, in every attempt, by more than the **+4** the before-arm
reached*.

**The row is not met, and it is the wrong row.** Three attempts, 3 of 3 up, no voids:

| attempt | survivor | height T+120 → end | finality T+120 → end | finality last moved |
|---|---|---|---|---|
| 1 | bootstrap | 28 → 29 | 13 → 13 | **T+33s** |
| 1 | v1 | 28 → 29 | 13 → 13 | **T+30s** |
| 2 | bootstrap | 65 → 80 | 4 → 4 | **T+19s** |
| 2 | v1 | 65 → 80 | 4 → 4 | **T+16s** |
| 3 | bootstrap | 32 → 36 | 18 → 19 | T+128s |
| 3 | v1 | 31 → 36 | 12 → 12 | **T+24s** |

The kill is at **T+120**. Five of six survivors had finished finalising — permanently — between 87 and 104
seconds *before* it. The sixth moved by one height at T+128.

## The same reading on the before-arm, which was never computed

| attempt | survivor | height T+120 → end | finality T+120 → end | finality last moved |
|---|---|---|---|---|
| 1 | bootstrap | 27 → 27 | 14 → 14 | T+111s |
| 1 | v1 | 27 → 27 | 14 → 14 | **T+90s** |
| 2 | bootstrap | 32 → 34 | 17 → 17 | **T+55s** |
| 2 | v1 | 29 → 82 | 14 → 18 | T+130s |
| 3 | bootstrap | 23 → 24 | 10 → 10 | **T+17s** |
| 3 | v1 | 23 → 24 | 10 → 10 | **T+16s** |

**Sixteen survivor node-runs across both arms. Fourteen stopped advancing before the kill; the other two
moved by one increment within 10 s of it.** The after-arm's fix is real and the derivation does publish a
fringe — but the quantity the acceptance row measures is dominated by *when finality naturally pins*, which
is 16–130 seconds into the run, in both arms, with every validator live.

## The stalls, split at the kill

The pre-registration's 0.3 reading and this one disagree about what the instrument shows, and the split is
the reason:

| run | lines | before the kill | after it |
|---|---|---|---|
| liveness, 18:57 | 75 | **73** | 2 |
| campaign, attempt 1/2/3 | 3 each | 3 each | 0 |
| liveness, 18:38 | 3 | 3 | 0 |

The campaign's three lines are the gate's old one-per-100-heights rate limit firing once at genesis, which
is why §31's run recorded `at tip 0` and nothing else. The 18:57 run has the change-of-reason gate and 75
lines in the same 300 s.

The support values behind them, before the kill:

| value | count |
|---|---|
| `0 of 250 (0 full among 3)` | 15 |
| `100 of 250 (1 full among 3)` | 15 |
| `0 of 250 (0 full among 2)` | 14 |
| `150 of 250 (2 full among 3)` | **10** |
| `0 of 250 (0 full among 0)` | 7 |
| `100 of 250 (1 full among 2)` | 6 |
| `0 of 250 (0 full among 1)` | 3 |
| `50 of 250 (1 full among 3)` | 3 |

## What this refutes

1. **"Finality stops after a validator is killed."** It stops 87–104 seconds earlier, with all three
   validators live and proposing. The kill neither causes it nor changes it. This is in the project's own
   committed artifacts (`n127-campaign/series-a{1,2,3}.tsv`) and had simply never been computed.
2. **`150 = 100 + 50` is not "the bootstrap's stake plus the killed validator's"** (C185's title, `#126`'s
   status block, `passes.md` §34). Ten of the `150 of 250` lines are **before** the kill; the killed
   validator was alive and producing at all ten. `150` is simply "two of three candidates were full
   partitions", and at stakes `100/100/50` that pair is `100 + 50`.
3. **The refusal is not dominantly a shortfall.** `0 of 250` in some form is **39 of the 73 pre-kill
   lines (53 %)**, and `0 full partitions` means *no candidate was seen by the whole live set at all* — which is not
   what `NoAdvance::Support`'s own doc says it reports ("a layer exists whose candidates were seen by the
   whole partition, but the stake behind them is not a supermajority"). The gate reaches its quorum test in
   the minority of refusals.
4. **The pre-registration's 0.3 row maps this outcome to the wrong consequence.** It says a `Support` stall
   means "the partition is satisfiable and the quorum is short → `candidate:inactivity-leak`, a decision".
   The dominant case is the partition *unsatisfied by every candidate*. And `candidate:inactivity-leak`
   (`n127-campaign-preregistration.md:75`, `passes.md:4942`) is not a row in `spec/findings.tsv` — a
   dangling reference, not a plan.

## What this does not settle

- **The mechanism.** Both of these produce `full_partitions = 0` and want different repairs: the gate
  demands that *every* live seer has seen *every* next-layer message
  (`finalizer.rs:222`, `seen_by.values().all(|v| v == &must_be_seen)`), which a forking DAG has no reason to
  satisfy; and `calculate_next_fringe_support_map` resolves `mv.parents` through the **full** `msg_map`
  (`finalizer.rs:185-193`), so a sender outside the live partition can land in a `seen_by` value and drop a
  *live* candidate from the numerator. C185's `owes` names the in-process fixture that separates them.
- **The acceptance row's own status.** Five of six survivors in the after-arm advanced by **0** against the
  before-arm's **+4**, so if the row is read literally the fix "did not work". Read against *when finality
  pins* it is not a comparison of the fix at all. The pre-registration's row is the thing that needs
  restating, not the fix.
- **Provenance, which this file is part of.** The committed liveness artifacts are **one attempt of a
  pre-registered three** (`manifest.txt`: `attempts=1`), and until this commit the series they were read
  from existed only under `target/`. The three-attempt run (`18:07`) has no stall logs at all — the
  instrumentation landed between it and the 18:38 run — so no single run in this campaign carried both the
  pre-registered attempt count and the stall capture. Recorded rather than re-run to fit.
- **The budget control passed**: `budget-*.txt` is empty in both runs, so `SearchBudgetExceeded` never
  fired. C184's number is not too low.
