# #144 Stage 1 — soft-checkpoint cost, pre-registered

**Status: FROZEN before the first measurement.** Issue
[#144](https://github.com/rchain-community/rchain-rust/issues/144).

## Question

Candidate 1 in #144 is a full in-memory soft checkpoint on the user-deploy play path. The checkpoint
clones the striped hot-store overlay and snapshots the native overlay. Source inspection shows four
user-deploy sites on the cost-accounted block path, but source count is not a performance result. This
run asks one narrower question before any optimization is allowed:

> **What fraction of a real user-deploy unit is spent taking soft checkpoints, under the default
> sequential scheduler?**

This unit is measurement only. It does **not** remove, defer, reuse, or narrow any checkpoint and it
does not change rollback, scheduling, state, block bytes, or validation verdicts.

## Instrument

`casper/src/runtime_manager.rs` times the five existing play-path call sites separately:

| metric stem | role |
|---|---|
| `deploy_fallback` | pre-deploy rollback checkpoint |
| `deploy_log` | post-deploy event-log checkpoint |
| `cost_unit_fallback` | pre-charge → deploy → refund rollback checkpoint |
| `pre_charge_log` | event-log checkpoint after pre-charge |
| `system_deploy_log` | block-level system deploy; recorded, but excluded from per-user-deploy cost |

The first four are the user-deploy numerator. The same fold also times the enclosing cost-accounted
user-deploy unit. Counters are monotone process totals and are published as `rchain_runtime_*` gauges on
the DAG's existing metrics tick. The harness reads **deltas between two scrapes**, so work before the
arm is not charged to the arm. Process-wide maxima are retained for diagnosis but are not used by the
decision rule because a maximum cannot be scoped to an arm by subtraction.

`tools/devnet-bench.py` derives:

- `snapshots_per_deploy`;
- `checkpoint_ms_per_deploy`;
- `deploy_unit_ms_per_deploy`;
- `checkpoint_fraction_of_deploy_unit`;
- the count and time of each of the four user-deploy sites.

The JSON artifact carries the exact git tree and the harness configuration that produced it.

## Arm

One-validator local devnet, because #144's existing benchmark explicitly scopes itself to the
single-node ceiling and does not claim multi-validator consensus throughput:

```sh
tools/devnet.sh up --validators 1 --fresh --no-autopropose
python3 tools/devnet-bench.py \
  --validators 1 \
  --only deploys \
  --deploys 200 \
  --concurrency 4 \
  --json target/n144-soft-checkpoint/run-<attempt>.json
```

Three attempts, unfiltered, one fresh devnet per attempt. `--no-autopropose` keeps the block path driven
by the submitted work; the harness may use its existing explicit drain proposals after submission so a
backlog is not mistaken for a throughput failure. The reading is the runtime-counter delta, not the
height rate.

The canonical automation for these exact three attempts is `.github/workflows/n144-soft-checkpoint-campaign.yml`. It runs the Docker arm above unchanged on an Ubuntu runner, preserves all three raw JSON files, derives the frozen decision mechanically, and uploads both raw and derived evidence as one artifact. The workflow changes no threshold or void condition.

### Execution wrapper (frozen before the first measurement)

The reference command above uses Docker because that is how `tools/devnet.sh` normally packages the
single validator. The campaign may instead run the **same `rnode` binary directly on the measurement
host** when Docker is unavailable, provided it is built with `cargo build --release` (the Dockerfile's
build profile) and all consensus/runtime flags, genesis inputs, API ports, deployer key, scheduler,
proposal policy and fresh-data condition are identical. In that arm the harness uses
`--native-rnode <path>` so deploys are still signed and submitted by the repository's Rust client over
the external gRPC API; Python does not construct or sign deploy bytes. The JSON records both the path
and SHA-256 of that binary.

This is an execution-wrapper substitution, not a second measurement arm: the decision rule remains the
checkpoint-time fraction inside the same process work unit, and the artifact must record
`native_rnode`, the git tree and node status. Docker-only RSS/image fields may be absent and are not
inputs to the Stage-2 decision. This clause is recorded **before any campaign sample exists**; the
thresholds below are unchanged.

Before each attempt record the image id and preserve the JSON. After the run, copy the three JSON files
under `spec/audit/evidence/n144-soft-checkpoint/<tree>-<utc>/`; `target/` alone is not citable evidence.

## Frozen reading and decision rule

For each attempt, report the four quantities above. The campaign reading is the **median of the three
attempts** for `checkpoint_fraction_of_deploy_unit`, with the three raw values beside it.

| observation | Stage-2 decision |
|---|---|
| `snapshots_per_deploy < 3.5` | **void for candidate 1** — the arm did not exercise the expected successful user-deploy path often enough to price its four sites |
| median checkpoint fraction **>= 10%** | candidate 1 is the first node-local lever to optimize; a behavior-preserving ablation gets its own before/after PR |
| median checkpoint fraction **< 5%** | candidate 1 is not the cheapest first lever; instrument candidate 2 (mempool full decode) next |
| median **5–10%** | inconclusive; repeat at 1,000 deploys before choosing a lever |

The thresholds are relative on purpose. An absolute millisecond threshold would price the developer
machine rather than the algorithm, while the question is whether an O(hot-state) clone is material
inside the work unit that owns it.

## Void conditions

- The JSON does not contain a git tree or the stated configuration.
- Any requested deploy is rejected before entering the pool for a harness/client reason.
- The runtime gauges are absent or move backwards between scrapes.
- Fewer than 90% of submitted deploys are processed inside the bounded drain window.
- The node logs a validation failure, replay mismatch, or self-validation failure during the arm.
  The campaign preserves each attempt's node log and mechanically scans it before applying the frozen decision; a missing log is also void because this condition would otherwise be uncheckable.

A void attempt is preserved and named; it is not silently replaced.

### Post-void rerun amendment — client anchor only, thresholds unchanged

The first three-run arm on `74cda38f14e14ab34f03b75d9989ae1f0c38c94d` is preserved under
`spec/audit/evidence/n144-soft-checkpoint/74cda38f14-20261009T045442Z/` and is **void**: only
95/200, 96/200 and 99/200 deploys processed, and every attempt hit the same self-validation failure at
block 51 (`a deploy has expired`). The harness had captured one `valid_after_block_number` before all
200 submissions, so the earliest deploys were deliberately allowed to age while later submissions kept
advancing the chain. That exposed a real node-side pool/validation boundary defect, but it makes the arm
invalid for pricing checkpoints.

For the rerun, the client refreshes `valid_after_block_number` from the current height **for each signed
deploy**, matching `tools/devnet.sh deploy`. This changes only the load generator's freshness anchor; it
does not change the node, scheduler, checkpoint instrumentation, number of deploys, concurrency, metrics,
void conditions, or any frozen decision threshold above. The original void arm remains evidence and is
not replaced.

## Completed rerun

The corrected rerun on `3b6d7c74a480a27502786fe7db8d44ef025df9c1` processed **200/200** deploys in all three attempts, recorded exactly **4.0 checkpoints/deploy**, and logged no validation/replay/self-validation failure. The checkpoint fractions were **37.270%**, **40.859%**, and **38.673%**; median **38.673%**.

By the frozen rule above, **candidate 1 earns the first behavior-preserving ablation**. Raw JSON, compressed node logs and the mechanically-derived summary are preserved under `spec/audit/evidence/n144-soft-checkpoint/3b6d7c74a4-20261009T050515Z/`.

## Measurement caveat (post-review clarification)

The checkpoint timers include clock-reading overhead, and publishing the census gauges adds work
to the DAG insert path. This observer effect has not been measured separately: the reported
checkpoint fractions describe the instrumented runtime, rather than an exact decomposition of
uninstrumented execution time. This clarification changes no campaign threshold, void condition,
or archived result.

## What this does not claim

- It is not a production TPS number or a multi-validator capacity plan.
- It does not justify widening the 255-deploy consensus ceiling.
- It does not authorize deleting a rollback checkpoint. `RelaxedValidated` still needs the fallback
  path; any later optimization must preserve that behavior and prove equivalence separately.
- It does not rank candidates 2/3/5/6 without measuring them. It only decides whether candidate 1 is
  large enough to earn the first Stage-2 ablation.
