# #144 Stage 2 — remove the unreachable cost-unit fallback snapshot, pre-registered

**Status: FROZEN before the first paired measurement.** Stage 1 selected candidate 1 with a median
soft-checkpoint fraction of **38.673%** across three 200/200-deploy runs. This unit asks whether the
smallest behavior-preserving ablation produces a measurable end-to-end improvement.

## Change under test

The user-deploy cost-accounting wrapper currently takes `CostUnitFallback` before every deploy. That
snapshot is consumed only when the unit returns `SpeculationInvalidated` and must be replayed
sequentially.

The scheduler makes that path mode-dependent:

- `Sequential`: reference execution; no speculative invalidation path;
- `Gate`: DFS-ordered commit gate; no speculative invalidation path;
- `Relaxed` / `RelaxedValidated`: speculative modes; keep the snapshot and rollback unchanged.

The ablation therefore skips **only** `CostUnitFallback` in Sequential/Gate and preserves it in both
relaxed modes. The other three user-deploy checkpoints are unchanged.

Expected mechanical census under the default Sequential arm: **4.0 -> 3.0 snapshots/deploy**. That is
an exercise check, not the performance result.

## Paired arm

One GitHub runner builds two images exactly once:

- baseline: `7df8d642d935683ebd099cb9877f0a378a4d9662` (the committed valid Stage-1 evidence head);
- ablation: the PR head containing this change.

Each image receives three fresh one-validator runs of 200 deploys at concurrency 4 with autopropose
off. Runs are paired on the same runner and the order alternates to reduce first/second-run bias:

1. baseline -> ablation
2. ablation -> baseline
3. baseline -> ablation

Each arm starts from a fresh data volume. The benchmark script comes from the same commit as the image
it measures, so its `tree` field names the binary source rather than the workflow checkout.

## Primary metric

For pair `i`:

```text
reduction_i = 1 - optimized.deploy_unit_ms_per_deploy / baseline.deploy_unit_ms_per_deploy
```

The campaign reading is the **median of the three paired reductions**. The per-arm checkpoint time and
checkpoint fraction are secondary explanatory metrics.

## Frozen acceptance rule

The arm is **VOID** if any of these holds:

- any run does not process 200/200 deploys;
- any required runtime gauge is missing or moves backwards;
- any run logs a validation failure, replay mismatch, or self-validation failure;
- a baseline run does not report 4.0 snapshots/deploy;
- an ablation run does not report 3.0 snapshots/deploy;
- the JSON tree does not equal the commit of the image it is meant to measure.

If valid:

- median paired deploy-unit reduction **>= 5%**, and no pair regresses by more than 5%:
  **the ablation earns merge as #144's first measured node-local lever**;
- otherwise: preserve the result, do not claim a throughput win from the Stage-1 fraction alone, and
  move to the next ablation/candidate with the measured number attached.

The 5% threshold is fixed before the run. It is deliberately below Stage 1's 10% *candidate-selection*
threshold because this unit removes only one of four measured snapshots, not the whole candidate
family.

## Correctness gates

Performance is not sufficient. Before merge:

- the mode predicate test must prove Sequential/Gate skip and Relaxed/RelaxedValidated retain the
  whole-unit fallback;
- existing cost-accounting/state/replay tests remain green;
- full CI is green on the final head;
- the paired evidence artifact and derived summary are committed.

This is family A: no block bytes, state transition, validation verdict, or consensus activation rule is
changed.
