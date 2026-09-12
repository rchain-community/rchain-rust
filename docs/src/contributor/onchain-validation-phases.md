# On-chain validation phases (Laws 23–25)

This page is the condensed account of how the on-chain validated-speculation realization
(`EffectMode::RelaxedValidated` on the casper block path) was built, phase by phase. The full
specification is [On-chain scheduling: validated speculation](../formal/onchain-scheduling.md);
the formalization lives in `spec/Rchain/SchedulerOnchain.lean`. Each phase landed green — workspace
build, the rspace/rholang/casper suites, and a clean `lake build`.

## Phase A — the write-record layer

The claim queue gains a second per-channel record: `WriteRecord { path, version, polarity }` in
`rspace/src/concurrent/channel_queue.rs`, with `record_write`/`last_write`/`reset_write_record`
and `set_validation_enabled`. Version = write count (aligned with the Lean `SpecState`); the
record is reset per evaluation, and it is inert until the validation flag is armed. The
`SpeculationInvalid` error variant and its propagation at the three `wait_at_head` call sites
land in the same phase.

## Phase B — the certificate in `try_acquire`

With validation enabled, `try_acquire` performs the prefix-visibility check under the held
channel locks — the gap-free linearization point — rejecting with
`AcquireError::ValidationFailed` (permanent; the claim re-waits are skipped for the channels the
claim already holds, so a produce's own phase-two trigger write cannot false-positive).

## Phase C — dispatch-time pre-claiming

The relaxed arm hoists `claims.claim(...)` above the task-future construction: a claim is
enqueued when its effect is *dispatched*, not when its task first runs. Same-channel claims from
one dispatch list therefore land in path order before any later-path task can acquire — the
persistent-produce COMM-order inversion disappears, and the strict per-channel COMM-sequence
assertion is restored.

## Phase D — fail-fast, oracle backstop, fallback

A certificate failure surfaces as `SpeculationInvalid` out of evaluation; the casper block path
reverts to the deploy's soft checkpoint and re-runs that deploy sequentially (per-deploy
fallback), with a whole-set safety net in `compute_state` re-running the deploy set on a forked
runtime from `start_hash`. The sequential-oracle legs (post-state hash + per-channel COMM
multisets + Law 11 rig-replay) remain on the accept path: certificate and oracle are
complementary — the certificate is the only detector for the C/D pair and the persistent-produce
COMM inversion (oracle-blind), the oracle the only detector for a DFS-earlier writer committing
after a DFS-later reader (`certificate_blind_late_writer_diverges`).

## Phase E — Lean closure

Both Law 24/25 axioms are de-axiomatized with the statements their boundary witnesses force
(each witness is a decided counterexample): the writer chain carries path-nodup and the initial
empty record layer; the publication theorem is the **pinned** form — dispatched + path-nodup +
per-channel path-pinned commit order ⇒ the commit-order fold reaches the gate fold's state and
each commit emits the gate's trace; `validated_speculation_refines_apply` is the coordinator
refinement, proven by cases. INVENTORY rows 24/25 are **proven**; no axioms remain.

## Phase F — docs and verification

This page, the realization map of
[onchain-scheduling.md](../formal/onchain-scheduling.md), the relaxed-contract bullets of
[channel-scheduler.md](../formal/channel-scheduler.md), the Laws 23–25 rows of
[laws-to-rust.md](laws-to-rust.md), and the final sweep: `cargo build`, the
rspace+rholang+casper suites, and `lake build`.
