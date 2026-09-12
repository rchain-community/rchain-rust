# The channel scheduler (Laws 20–22)

[Effect scheduling](effect-scheduling.md) established the effect-level impossibility: disjoint
*footprints* do not commute (S.3, `Rchain.Effect.effect_reorder_diverges`), only disjoint *closures*
do (S.4, `effect_commute_of_disjoint_closure`), and closures are not statically decidable — so **no
static partition is sound**. This document specifies the schedulers that *are* sound, and the one
that is *usefully parallel*, all formalized in
[`spec/Rchain/Scheduler.lean`](../../../spec/Rchain/Scheduler.lean):

- **Law 20 — channel-task linearization ("1 channel = 1 logical task").** Every channel owns a
  *claim queue*; an effect claims its channels at its DFS path and commits only when it is the head
  of every claimed channel.
- **Law 21 — DFS-gate linearization.** The gate scheduler runs effect `i` only after every earlier
  effect's subtree has completed — provably the sequential fold. The tempting "one-hop" repair
  (let a sibling run once the *next-step* footprint is disjoint) is **unsound**
  (`one_hop_depth2_diverges`).
- **Law 22 — next-step closure is computable at dispatch.** Once a trigger matches, the matched
  datum is concrete, so a continuation's first-step footprint is computable — but (per the depth-2
  pair) that computability does *not* make cross-channel pruning sound.

The Rust realization ships these as three `EffectMode`s — `dfs` (the sequential reference),
`gate` (Law 21), `relaxed` (Law 20) — selected per runtime by the `--effect-scheduler` node flag.
`relaxed` is **off-chain only**: the casper block paths hard-reject it. The extension that makes
the relaxed interleaving *sound on-chain* — speculative execution with a Law 24 serializability
certificate and a gate re-run fallback — is specified in
[On-chain scheduling: validated speculation (Laws 23–25)](onchain-scheduling.md).

## DFS paths: the linearization key

Every effect in a reduction carries a **DFS path** — the sequence of child indices from the root of
the reduction tree (`[4]` is the 4th sibling of a `Par`; `[4, 0]` is its first child). Lexicographic
order on paths is exactly the sequential reducer's depth-first order: `[4] < [4, 0] < [5]`. The path
is therefore the order the scheduler must preserve — the order in which the sequential reducer
*would* have applied the effects, and the order replay re-derives (Law 11).

```
rholang/src/scheduler.rs — DfsPath(Vec<u16>)  (Ord = lexicographic = DFS order)
                          EffectMode { Sequential, ForkJoin, Gate, Relaxed }
```

## Law 20 — the per-channel claim queue

"1 channel = 1 logical task": each channel is a task whose work items are the effects touching it.
An effect is a multi-channel work item (a join touches several channels, Law 7), so the scheduler
holds it to the **per-channel** order, not a global one:

1. An effect **claims** each channel it touches, tagged with its DFS path, at dispatch time.
2. A claim **commits** only when it is the **head** (the path-smallest pending claim) of *every*
   claimed channel.
3. Committing removes the claim from all its queues and wakes the new heads.

`queue_commit_path_ordered` (`Scheduler.lean`) proves the queue invariant: insertion is at path
position (which may be *ahead of* the current head — a continuation enqueued from path `[1, 0]`
sorts before the sibling claimed at `[2]`), commits remove only heads, so a channel's commit
sequence follows its path-sorted claim order. `law20_deadlock_freedom` states the bakery argument:
the globally path-smallest pending claim is head on all its channels, so some claim can always
commit — the queue does not deadlock (stated; the Rust queue's `wait_at_head` park/wake protocol
carries it).

The Rust realization is `rspace/src/concurrent/channel_queue.rs`:

- `ChannelClaimQueue::claim(path, channels) -> ClaimGuard` — the guard is the *real* exclusion: it
  marks the queue's `active` entry and is released on drop, removing the entries and waking the
  next head.
- `ClaimGuard::claim_more(channels)` — extend the guard to newly-discovered channels (a produce
  that matches a join discovers the join's other channels only at phase one, see below).
- `ClaimGuard::wait_at_head() -> HeadLease` — parks until the guard is head on all its channels.
  The lease is **advisory** (exclusion is the guard's `active` mark, not the lease), so a task may
  re-`wait_at_head` after `claim_more` without releasing anything; if it was still `active`, the
  wait is a pass-through.

### The produce phase-one/two split

A `produce` on a join channel `c` does not know at dispatch time *which* join it will feed — the
joined channels are only discovered by consulting the tuple space. The relaxed produce therefore
runs in two phases (`rspace/src/scheduled_space.rs`):

1. **Phase one** holds only the *trigger* channel's tuple-space lock (plus its claim-queue guard):
   log the produce, read the join set, extract the matching candidate. No match → store the datum
   (done, single acquisition). Match → return the matched join channels **without committing**.
2. The reducer then `claim_more`s the join channels, re-`wait_at_head`s (parking with **no rspace
   locks held** — the claim queue serializes per channel while parked), and re-acquires the trigger
   plus *all* join-channel locks as one two-step acquisition. Under the full lock set it
   **re-validates** the candidate (a cross-channel op may have consumed it meanwhile; fall back to
   storing the datum) and commits the COMM.

Phase one's reads are advisory; the phase-two re-extraction under the full lock set is the
atomicity backstop, and the claim queue is what keeps same-channel commits in path order across
the two phases. A `consume` needs no split — its full channel set is static — so it claims all
sources, waits at head, and runs the ordinary locked consume.

## Law 21 — the DFS gate

The **gate** scheduler (`EffectMode::Gate`) runs the task for effect `i` only after the tasks for
effects `0..i−1` have completed. `gate_exec_refines_apply` proves this is *exactly* the sequential
`Effect.apply` fold — sound by construction, but it adds no parallelism beyond Level 1 (pure
resolution). Its role is as the deterministic carrier: the gate mode and the sequential reducer
must produce identical post-state hashes and identical event logs, which
`gate_and_sequential_state_hashes_match` (`rholang/tests/execution.rs`) asserts over the
reduction corpus.

The tempting repair is a **one-hop** scheduler: run a sibling whose *next-step* footprint (the
channels the effect plus its now-computed continuation first steps touch) is disjoint from
everything pending. `one_hop_depth2_diverges` refutes it: the depth-2 pair has next-step
footprints that are disjoint at *every* decision point, yet overlapping closures — so the one-hop
interleaving reaches a state the sequential reducer never reaches. Disjointness must be about the
**closure**, which is exactly the condition that is not computable ahead of time.

## Law 22 — next-step closure at dispatch

What *is* computable, and what the gate and claim queue both rely on, is the **next-step closure**:
once a trigger matches, the matched datum is concrete, so the continuation it selects
(`consumeWith`) has a computable first-step footprint. `next_step_closure_computable` states this;
the Rust reducer computes it in `resolve_children` at dispatch — this is what turns "the
continuation's channels are unknown" into "the continuation's *next* claims are known", and why the
claim queue can enqueue a continuation's effects in the right path order. `depth2_next_step_disjoint`
records the corollary: computability at dispatch does **not** make cross-channel *pruning* sound —
the next-step footprint is not the closure.

## The relaxed contract (the off-chain mode)

`EffectMode::Relaxed` puts Law 20's claim queue to work. Its contract:

- **Per-channel DFS commit order is preserved.** Same-channel *commits* follow the path-sorted
  claim order — the executable form of Law 20 (`queue_commit_path_ordered`), asserted by
  `relaxed_preserves_same_channel_order`: the per-channel **COMM**-event *sequence* equals the
  sequential reference's, strictly, for every non-free corpus term — including the
  persistent-produce pair, whose COMM order used to invert. Only commits are compared at all:
  because dispatch is spawn-only, a claim lands only when its task runs — a later-path sibling
  already at the head may install (log a `Consume`) or store (log a `Produce`) first, so the
  standalone install/store events can interleave around a `Comm` even for a simple same-channel
  produce/consume. The commit order itself is pinned by **dispatch-time pre-claiming**: the
  relaxed arm hoists `claims.claim(...)` above the task-future construction, so a claim is
  enqueued when its effect is *dispatched*, not when its task first runs — same-channel claims
  from one dispatch list land in path order before any later-path task can acquire. The
  cross-dispatch-list race that remains (a DFS-earlier writer committing after a DFS-later
  reader) is exactly the certificate-blind divergence the validated mode's oracle catches
  (`certificate_blind_late_writer_diverges` in
  [On-chain scheduling](onchain-scheduling.md)).
- **Cross-channel interleaving is free.** Effects on disjoint channels claim disjoint queues and
  commit in any order. For terms whose continuations touch channels that later-path siblings also
  touch (the S.3 counterexample), even the *final state* may differ from the sequential
  reference's — the term reduces to `@"out"!(2)` in one interleaving and `@"out"!(1)` in the
  other. The full event log may therefore differ from the sequential one — which is why this mode
  is **off-chain only**.
- **Off-chain only.** The relaxed interleaving must never reach a block's event log (Law 11
  replay / Law 16 content addressing). `casper/src/runtime_manager.rs` hard-rejects
  `EffectMode::Relaxed` at both block-path entry points (`process_deploy`, `play_deploys`); the
  exploratory path (`explore-deploy`) is the relaxed node's only deploy outlet.

Two implementation rules make the relaxed arm correct:

1. **Spawn-only dispatch — never await a continuation while holding claims.** A task that awaited
   its continuation's completion while holding the guard would self-deadlock on re-entrant terms
   (`for (x <- c) { c!(x) }` — the continuation's produce claims `c` whose head is still held by
   its own trigger). Relaxed tasks therefore only *enqueue* children into a shared
   `JoinSet<Result<(), RholangError>>` and return; each child claims its own channels and waits at
   head itself.
2. **Only the root drains.** `evaluate()` — and only `evaluate()` — drains the JoinSet to
   exhaustion after the root reduction (the take-and-swap pattern: the drainer moves the set out of
   its mutex and awaits outside the lock, so continuations spawned *during* the drain land in the
   replacement set and are drained in the next round). The first error propagates.

Every relaxed task still runs the `apply_effect_spawned` cancelled + step-budget checks before
spawning, so the cooperative-cancel and reduction-budget machinery applies unchanged.

## "1 channel = 1 logical task" → M:N mapping

The claim-queue entry is the *logical* task; tokio workers are the *physical* threads. The mapping
is M:N — any worker can run any claim, and a claim that is not head on some channel simply parks
(an async wait, not a blocked thread) until the guard ahead of it commits and wakes it. The
tuple-space maps are **striped** (`rspace/src/hot_store.rs`, `SHARDS = 64` per-key mutexes) so
that distinct channels no longer contend on one store-wide mutex — the claim queue removes the
*semantic* serialization (same-channel order), the stripe removes the *mechanical* one.

## Rust realization map

| Mechanism | Location |
|-----------|----------|
| `DfsPath`, `EffectMode`, `FromStr` ("dfs"/"gate"/"relaxed") | `rholang/src/scheduler.rs` |
| Claim queue (`claim`/`claim_more`/`wait_at_head`, phase-two re-wait) | `rspace/src/concurrent/channel_queue.rs` |
| Scheduled produce/consume + phase-one/two split + `commit_produce` | `rspace/src/scheduled_space.rs` (+ trait defaults in `tuple_space.rs`) |
| Relaxed `reduce_effects` arm, enqueue-only dispatch, root drain | `rholang/src/reduce.rs` |
| Cost charging for the scheduled path | `rholang/src/storage.rs` (`ChargingRSpace`) |
| Striped hot store (`SHARDS = 64`) | `rspace/src/hot_store.rs` |
| Block-path hard-reject + exploratory mode pass-through | `casper/src/runtime_manager.rs` |
| `--effect-scheduler` flag → `casper.effect-scheduler` hocon | `node/src/configuration/{commandline/options.rs,config_mapper.rs,hocon.rs}` |
| Runtime construction under a mode | `rholang/src/runtime.rs` (`create_with_effect_mode`), `node/src/runtime/node_runtime.rs` |
| `--thread-pool-size` → tokio worker threads | `node/src/main.rs` |

## Tests and benchmarks

- `gate_and_sequential_state_hashes_match` — the gate's state hash *and* event log equal the
  sequential reference's, term by term over the reduction corpus (Law 21).
- `relaxed_mode_runs_all_corpus_terms_without_error` — every corpus term (including the S.3
  cross-channel counterexample and the join re-entry terms) reduces to completion under relaxed
  mode, and the post-state checkpoints.
- `relaxed_preserves_same_channel_order` — the per-channel **COMM**-event *sequence* of the
  relaxed event log equals the sequential DFS reference's, strictly, for every non-free corpus
  term (Law 20, pinned by dispatch-time pre-claiming — the persistent-produce pair included),
  with the multiset comparison kept as a cross-check; plus root equality. Only commits are
  compared (spawn-only dispatch lets the standalone install/store events interleave around a
  `Comm`), and the S.3 counterexample and the other free terms are exercised only by the
  no-error test above (their final state is free).
- `block_paths_reject_relaxed_mode` + `exploratory_path_stays_open_in_relaxed_mode`
  (`casper/tests/scheduler.rs`) — the off-chain gate.
- `make bench-scheduler` — `sched/pingpong/{2,4,8,16}` (single-channel serialization),
  `sched/fanout/{8,32,128}` (cross-channel parallelism), the dfs/gate/relaxed sweep at workers
  1–8, and the striped-vs-unstriped store-contention comparison.

### Benchmark actuals

*Recorded after `make bench-scheduler` (see the verification checklist); filled in on the
`feature/channel-scheduler` branch before merge.*

| Workload | dfs | gate | relaxed |
|----------|-----|------|---------|
| pingpong/16 | — | — | — |
| fanout/128 | — | — | — |
| sweep/fanout32, workers 1 | — | — | — |
| sweep/fanout32, workers 8 | — | — | — |
| store_contention (striped vs unstriped) | — | — | — |

> Next: [Substitution and matching](substitution-matching.md).
