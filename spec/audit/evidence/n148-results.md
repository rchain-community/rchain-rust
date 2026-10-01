# #148's absent-validator storm: reproduced on `dev`

Run 2026-10-01 on `dev` @ `f1548dec4` (image `sha256:54c4577897…`, `binary_sha256=9a97e3c5…`). Every number
below is `n148-summarise.py`'s output over the committed series, not a reading of a log. The protocol was
frozen first in `n148-preregistration.md`; the two arms are `n148-a-kill/` and `n148-b-all-live/` beside
this file, each with its own `manifest.txt`, `series-a*.tsv`, `readings-n148.txt` and `harness.log`.

```
python3 spec/audit/evidence/n148-summarise.py \
    spec/audit/evidence/n148-a-kill spec/audit/evidence/n148-b-all-live
```

## Verdict against the frozen acceptance row

**#148 is reproduced on `dev`.** Arm A's row was "height runs away after the single deploy while finality
does not advance past its pre-deploy value", and that is what all three attempts did. Part 2 of the issue's
`Closes when` — a bound on post-deploy production, held by a test — is therefore owed.

## Arm A — kill at T+120, one deploy at T+180 (the issue's shape)

| attempt | survivor | height at the deploy → end | gained | rate | finality at the kill → end | finality last moved |
|---|---|---|---|---|---|---|
| 1 | bootstrap | 125 → **243** | +118 | 0.49 /s | 110 → **110** | **T+121** |
| 1 | v1 | 153 → **244** | +91 | 0.38 /s | 110 → **110** | **T+121** |
| 2 | bootstrap | 171 → **213** | +42 | 0.17 /s | 111 → **111** | **T+119** |
| 2 | v1 | 171 → **213** | +42 | 0.17 /s | 111 → **111** | **T+119** |
| 3 | bootstrap | 180 → **221** | +41 | 0.17 /s | 120 → **120** | **T+121** |
| 3 | v1 | 180 → **221** | +41 | 0.17 /s | 120 → **120** | **T+121** |

In **3 of 3** attempts, finality's last movement is **at the kill** and it never moves again; production
continues to the end of the 420 s window. The killed validator (`v2`) has no samples after T+120, as
expected; it is reported as `?` rather than interpolated.

## Arm B — every validator live, one deploy at T+180 (the attribution control)

| attempt | node | height at the deploy → end | gained | finality → end | finality last moved |
|---|---|---|---|---|---|
| 1 | all three | 143 → **402** | +259 | 139 → **398** | T+419 (the window's end) |
| 2 | all three | 200/201 → **461** | +260 | 196 → **457** | T+420 |
| 3 | all three | 207/208 → **470** | +262 | 203 → **466** | T+419 |

With every validator live, **finality tracks the tip at a constant gap of 4 through the end of every
attempt** — nothing freezes, and the single deploy changes nothing.

## What the two arms say together

1. **The absence causes it, and the deploy is incidental.** In arm A finality stopped at T+119–121 — the
   kill — roughly **55 s before** the T+180 deploy, and production was already running by then. The issue
   reads as "one deploy re-arms the storm"; on this rig no deploy is needed for the freeze or for the
   continued production.
2. **The runaway is not *faster* production — it is production that is never released.** Arm B produced
   *faster* (≈1.08 blocks/s) than arm A (0.17–0.49/s) and was harmless, because finality kept up. The
   hazard is the unfinalised set growing without bound, which is the Θ(N²) residency and the replay cost
   `#127` owns.
3. **The mechanism the stall lines give.** The survivors' layer never reaches a supermajority: after the
   kill the line reads **`100 of 250`** (one validator's stake), then lapsing to `0 of 250 (0 full
   partition(s) among 2 candidate(s))` — not the 200 of 250 the two survivors hold. So the two live
   messages do not form one full partition, and finality is stuck exactly as the `#126` fringe derivation
   governs it. This is a node-local liveness result, not an attacker's.

## Correction to the pre-registration, and the run it voided

**Arm B as frozen — "`KILL_AT` beyond the window" — was VOID, and the first attempt was discarded.** The
harness waits for `KILL_AT` *before* the after-kill deploy, so a `KILL_AT` past the window pushes the
**deploy** past the window too: the first arm B attempt deployed at **T+421**, after sampling had stopped
at 420 s, leaving nothing to read. That log is kept as `n148-b-all-live/harness-VOID-first-attempt.log`
so the void is on the record rather than deleted. Arm B was re-run with a `NO_KILL=1` control added to the
harness (skip the kill, keep the timeline), and the arm A results above are unaffected — arm A ran before
the change and its manifest is as frozen.

## What this does not settle

- **The per-deploy minting of #149** is a separate issue and is not measured here.
- **Whether a deploy is *sufficient*** to start a storm on a chain that is otherwise idle. This rig runs
  the devnet defaults (autopropose on), so production continues after the kill without any deploy; #148's
  "chain idle, then one deploy" shape implies its measurement had autopropose **off**. The hazard is the
  same, the trigger is not, and a `--no-autopropose` arm is owed before "a deploy re-arms it" is written
  down as fact.
- **The bound itself** (part 2) — this unit measures; it does not fix.
- Three attempts of one configuration, on one machine, at an 8 GiB cap (the original hosts were 1 GB and
  the run was halted by the cap there). Not a capacity plan and not a proof.
- The merge budget control is empty in both arms (`readings.txt`): these are the honest scopes
  `SearchBudget::NODE` must not touch.
