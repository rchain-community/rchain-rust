# Effect scheduling: from the effect level to validated speculation

The scheduler is where "the result must be deterministic" becomes an *implementation*: which effects may
run concurrently, which must be ordered, and what a reordering has to prove before its result may be
published. This is the document for laws **20–25**, in the order the design layers them.

**What this document carries.** The **effect level** of reduction and the soundness theorems it targets
(S.1–S.4) — including the counterexample that kills the obvious Law 9 reading ("disjoint footprints
commute"): *disjoint footprint, overlapping closure* does **not** commute, so the sound condition is the
strengthened one. Then the three tiers built on it: the **channel scheduler** (law 20's per-channel claim
queue, law 21's DFS gate, law 22's dispatch-time closure, and the relaxed off-chain mode); and the
**on-chain** extension (laws 23–25: read-determinism, the versioned write-record certificate, and
validated speculation, where an invalidated subtree aborts and gate-re-runs so the published log is the
sequential fold's).

The conceptual frame — the four levels, and what may parallelize at each — is
[Concurrency: the model](concurrency.md); the block-level determinism question is
[Determinism of the block state transition](determinism.md).

> The per-law **status** is the register's (`spec/LAWS.md`, emitted from `Rchain/Laws.lean` and refused
> stale by the gate). Nothing on this page restates one; every theorem named below is a pointer into
> `spec/Rchain/`.

---

## Effect scheduling

[Concurrent reduction](concurrency.md) established the *process-level* contract: reduction is
permitted anywhere inside `|`. This document moves down one level to the **effect level** — the concrete
`produce`/`consume` operations a reduction step issues against the tuple space — and states what an
*effect scheduler* may and may not do. It is grounded in the Lean model [`Rchain.Effect`](../../../spec/Rchain/Effect.lean),
which is the oracle for this layer.

> **Status: the naive reading of Law 9 is insufficient — and the repair is dynamic, not static.** The
> channel-sharded effect scheduler (partition a `Par`'s effects by *static channel footprint* and run
> disjoint parts concurrently) is **unsound**. This document records the counterexample and the sound
> condition (`Rchain.Effect`). The schedulers that *are* sound — the per-channel claim queue and the DFS
> gate — are Laws 20–22, specified in the channel scheduler (below) and realized as the
> `gate`/`relaxed` effect modes.

### The effect level of reduction

A reduction step is realized by a set of **effects**:

- a **produce** `send(c, d, persistent)` — a send of datum `d` on channel `c`;
- a **consume** `receive(C, p, persistent)` — a receive on a channel set `C` (a join, Law 7) with patterns
  `p`; on a match its **continuation** (the receive body) runs, and the continuation's own effects are
  emitted only *after* the trigger matches.

Each effect has two notions of "which channels it touches":

- **footprint** `chans(e)` — the channels the effect *directly* touches (the trigger channels);
- **closure** `closure(e)` — the channels reachable through `e`'s *transitive continuation descent*
  (the trigger channels, plus everything the continuation can send/receive on, recursively).

Two effects are **independent** only when their **closures** are disjoint, not merely their footprints.

### Concurrency profile of the laws (effect level)

**Enables — "you may apply independent effects concurrently":**

| # | Law | What it grants | Rust realization |
|---|-----|----------------|------------------|
| **9** | Merge is a monoid; non-conflicting logs commute | *closure*-disjoint effects commute (see S.4 — the footprint reading S.1 is too weak) | `rspace/src/merger/*` |
| **7** | Join commutativity | a join's channel set is hashed in sorted order, so its identity is order-independent | `rspace/src/hashing/stable_hash_provider.rs` |
| **19** | `Blake2b512Random` associative splittable merge | each effect's RNG is pre-split, so `new`-freshness is schedule-independent | `crypto/src/hash/blake2b512_random.rs` |

**Constrains — "the *order* of same-channel effects is fixed":**

| # | Law | What it fixes | Rust realization |
|---|-----|---------------|------------------|
| **4** | `reduce_redex_unique` (**proven**) — an isolated redex is unique up to `≡`; and `reduce_not_deterministic` (**proven**) — the *flat* calculus is not confluent | an isolated redex produces one thing; which redex fires is this module's business. The consumed candidate is fixed by content-addressed selection (law 8) and the claim queue (law 20), not by the calculus | `rholang/src/reduce.rs`, `rspace/src/space_matcher.rs` |
| **8** | Deterministic COMM | candidate selection is sorted-first by content hash | `rspace/src/space_matcher.rs`, `rspace/src/rspace.rs` |
| **11** | Replay determinism | the effect *order* is fixed — replay must reproduce the recorded trace | `rspace/src/replay_rspace.rs` |
| **10** | Merkle determinism | the trie root is the state — a given effect *set* yields the same root | `rspace/src/history/*` |
| **20** | `queue_commit_path_ordered` (+ `pathSorted_head_minimal`, the bakery argument's core) | same-channel commits follow the path-sorted claim order; a claim commits only as head of *all* its channels; a sorted queue's head is its path-smallest element | `rspace/src/concurrent/channel_queue.rs` |
| **21** | `gate_await_closure_orders` | the immediate-predecessor await chain is *transitively* complete, so a linear chain of awaits suffices (the Rust's "not the quadratic all-predecessors join"); the one-hop variant diverges (`one_hop_depth2_diverges`) | `rholang/src/scheduler.rs` (`EffectMode::Gate`), `rholang/src/reduce.rs` |
| **22** | `depth2_next_step_disjoint` | the matched datum is concrete at dispatch, so the continuation's first-step footprint is computable (`resolve_children`, whose signature carries no store) — and that computability does *not* license cross-channel pruning | `rholang/src/reduce.rs` |

The subtle point the rest of this document makes precise: sorted selection (Law 8) removes the
order-sensitivity of *which stored candidate* a comm consumes, but **not** the *arrival order* — and, more
fundamentally, a continuation's effects are discovered only after its trigger runs, so the *closure* of a
"disjoint-looking" effect can reach a sibling's channel.

### Soundness theorems

#### S.1 Footprint commute (the naive Law 9)

```
chans(e₁) ∩ chans(e₂) = ∅   ⇒   apply(e₁; e₂)  ≡  apply(e₂; e₁)
```

This is the natural reading of Law 9 (`Rchain.RSpace.Merge.mergeChanges_comm`): two effects touching
disjoint *channels* commute. **This statement is false at the effect level**, because `chans(e)` is only
the footprint — it does not account for the continuation closure. S.3 is the counterexample.

#### S.2 Same-channel order (Law 4/8/11)

For effects sharing a channel, the candidate a comm consumes depends on arrival order. Same-channel
effects must therefore be applied in a **fixed total order** — the reducer's depth-first (DFS) order,
which is exactly the order the sequential scheduler uses and the order replay re-derives (Law 11). This
part is correct and remains a hard constraint.

#### S.3 The counterexample: disjoint footprint, overlapping closure, non-commuting

`Rchain.Effect.effect_reorder_diverges` proves there exist two effects with **disjoint footprints** but
**overlapping closures** that do **not** commute. Concretely (channels `c`, `d`, `"join"`, `"out"`; both
`c` and `d` hold a datum):

```
e₁ = receive d { receive c { @"join"!() } }     -- footprint {d}, closure {d, c, "join"}
e₂ = receive c { @"out"!() }                    -- footprint {c}, closure {c, "out"}
```

`footprint_disjoint` shows `chans(e₁) ∩ chans(e₂) = ∅`; `closure_overlap` shows `closure(e₁) ∩ closure(e₂)
≠ ∅`. Applying `e₁` then `e₂` yields `"join"` filled and `"out"` empty; applying `e₂` then `e₁` yields the
opposite. Hence a scheduler that partitions by *static footprint* and runs the parts concurrently may
reach a state the sequential scheduler never reaches — it is **unsound**.

#### S.4 Closure commute (the *strengthened* Law 9 — the sound condition)

```
closure(e₁) ∩ closure(e₂) = ∅   ⇒   apply(e₁; e₂)  ≡  apply(e₂; e₁)
```

This is `Rchain.Effect.effect_commute_of_disjoint_closure`, the correct soundness criterion for
concurrent effect scheduling: two effects may run concurrently only when their **closures** are disjoint.
Because a continuation's closure is discovered only by running the trigger, this condition is not
statically decidable. Consequently **no static footprint partition is sound**. The sound schedulers
replace prediction with *dynamic* ordering: the DFS gate runs effects strictly in path order (Law 21),
and the per-channel claim queue (Law 20) enforces same-channel DFS order while cross-channel commits
interleave freely — the **relaxed** mode, whose event log may differ from the sequential one and is
therefore **off-chain only**. See the channel scheduler (below).

### Realization map — theorem → mechanism

| Theorem | Mechanism | Location |
|---------|-----------|----------|
| S.1 Footprint commute | *insufficient* — not a valid concurrency permission | — |
| S.2 Same-channel DFS order | the sequential reducer's DFS order (effects applied in order) | `rholang/src/reduce.rs` |
| S.3 Counterexample | `effect_reorder_diverges` | `spec/Rchain/Effect.lean` |
| S.4 Closure commute | the sound condition (not statically decidable) | `spec/Rchain/Effect.lean` |
| Law 7 (join key) | channel-set keys (`Vec<C>`) | `rholang/src/reduce.rs` |
| Law 8 (sorted selection) | candidate sort by content hash | `rspace/src/space_matcher.rs` |
| Atomicity | per-channel `TwoStepLock` | `rspace/src/concurrent/{multi_lock,two_step_lock}.rs` |
| Law 11 oracle | `ReplayRSpace` trace check | `rspace/src/replay_rspace.rs` |
| Law 20 (claim queue) | `ChannelClaimQueue` (claim/claim_more/wait_at_head) | `rspace/src/concurrent/channel_queue.rs` |
| Law 21 (gate) | `EffectMode::Gate` reduce_effects arm | `rholang/src/scheduler.rs`, `rholang/src/reduce.rs` |
| Law 20 (relaxed, off-chain) | `EffectMode::Relaxed` + scheduled produce/consume + casper block-path hard-reject | `rspace/src/scheduled_space.rs`, `rholang/src/reduce.rs`, `casper/src/runtime_manager.rs` |

### The "continuation-prepend invariant" is necessary but not sufficient

It is tempting to repair the footprint partition by "prepending" a matched continuation's effects to the
target channel's queue (so they run before the remaining pending effects on that channel). This handles
the *same-channel* ordering, but **not** the cross-channel case: a continuation's effect on channel `d`
is enqueued only *after* the trigger on `c` runs, so a sibling effect already claimed on `d` can be
applied first — exactly the S.3 race. The prepend invariant fixes the order *once both effects are in the
queue*; it cannot stop the sibling from being claimed *before* the continuation is enqueued. This is the
**one-hop enqueue race**: the scheduler may only compare effects that are *both enqueued* — and the
gate scheduler's answer is to compare them by DFS path (Law 21), while the claim queue's answer is to
let the per-channel head rule order them (Law 20). `Scheduler.lean` pins both: `one_hop_depth2_diverges`
is the prepend race as a divergence proof, and `gate_await_closure_orders` / `queue_commit_path_ordered`
are the two repairs. Only the closure condition (S.4) would justify *more* than per-channel order, and
it is not statically checkable.

> Next: the scheduler itself — the channel scheduler (below), then how the *data* moves
> in a comm — [Substitution and matching](substitution-matching.md).

---

## The channel scheduler (Laws 20–22)

The effect level (above) established the effect-level impossibility: disjoint
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
The on-chain extension (below).

### DFS paths: the linearization key

Every effect in a reduction carries a **DFS path** — the sequence of child indices from the root of
the reduction tree (`[4]` is the 4th sibling of a `Par`; `[4, 0]` is its first child). Lexicographic
order on paths is exactly the sequential reducer's depth-first order: `[4] < [4, 0] < [5]`. The path
is therefore the order the scheduler must preserve — the order in which the sequential reducer
*would* have applied the effects, and the order replay re-derives (Law 11).

```
rholang/src/scheduler.rs — DfsPath(Vec<u16>)  (Ord = lexicographic = DFS order)
                          EffectMode { Sequential, ForkJoin, Gate, Relaxed }
```

### Law 20 — the per-channel claim queue

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

#### The produce phase-one/two split

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

### Law 21 — the DFS gate

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

### Law 22 — next-step closure at dispatch

What *is* computable, and what the gate and claim queue both rely on, is the **next-step closure**:
once a trigger matches, the matched datum is concrete, so the continuation it selects
(`consumeWith`) has a computable first-step footprint. `next_step_closure_computable` states this;
the Rust reducer computes it in `resolve_children` at dispatch — this is what turns "the
continuation's channels are unknown" into "the continuation's *next* claims are known", and why the
claim queue can enqueue a continuation's effects in the right path order. `depth2_next_step_disjoint`
records the corollary: computability at dispatch does **not** make cross-channel *pruning* sound —
the next-step footprint is not the closure.

### The relaxed contract (the off-chain mode)

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
  the on-chain extension (below)).
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

### "1 channel = 1 logical task" → M:N mapping

The claim-queue entry is the *logical* task; tokio workers are the *physical* threads. The mapping
is M:N — any worker can run any claim, and a claim that is not head on some channel simply parks
(an async wait, not a blocked thread) until the guard ahead of it commits and wakes it. The
tuple-space maps are **striped** (`rspace/src/hot_store.rs`, `SHARDS = 64` per-key mutexes) so
that distinct channels no longer contend on one store-wide mutex — the claim queue removes the
*semantic* serialization (same-channel order), the stripe removes the *mechanical* one.

### Rust realization map

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

### Tests and benchmarks

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

#### Benchmark actuals

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

---

## On-chain scheduling: validated speculation (Laws 23–25)

[The channel scheduler](channel-scheduling.md) (Laws 20–22) ships the sound schedulers — the
per-channel claim queue and the DFS gate — and the *relaxed* mode, whose free cross-channel
interleaving may diverge from the sequential reducer's final state and event log. That is why the
casper block paths hard-reject `Relaxed`: a relaxed trace must never reach a block's event log.
This page specifies the extension that makes effect-level concurrency *sound on-chain*, formalized
in [`spec/Rchain/SchedulerOnchain.lean`](../../../spec/Rchain/SchedulerOnchain.lean):

- **Law 23 — read-determinism.** An effect's chosen candidate, commit outcome, and event trace
  are a deterministic function of the state it reads: states agreeing on the effect's *closure*
  yield identical traces (`read_state_determines_outcome`). This is the executable counterpart of
  Law 8's content-addressed COMM and the strengthened Law 9 (`effect_commute_of_disjoint_closure`):
  it is what lets a *checked* interleaving stand in for the sequential run.

- **Law 24 — DFS-order serializability.** A concurrent execution is sound for the block path iff
  every commit read exactly the state the DFS-earlier effects produced. The Bool-presence `State`
  of `Rchain.Effect` cannot state this — the depth-2 pair's stale read has the same Bool value as
  the correct one — so the law adds the **versioned write-record layer**
  `SpecState := Chan → Option (DfsPath × Nat × Bool)`: each write records its writer's DFS path,
  a per-channel version, and the value the channel held *after* the write (the polarity makes the
  record witness the data: `record_determines_value`). A commit validates when every channel it
  reads is **prefix-visible** — its newest write was recorded by a DFS-earlier path.

- **Law 25 — validated speculation.** A scheduler may commit effects in any order iff each commit
  validates Law 24; a run that fails validation falls back to the whole-run gate re-run, so the
  published state is the sequential fold's (`validated_speculation_refines_apply`, proven). The
  gate re-run is total (`gate_replay_terminates`), a published run commits each path at most once
  (`Published`: each path validates, or aborts at most once and then commits via the path-ordered
  re-run queue), and the fallback's sorted re-run queue preserves that invariant
  (`fallback_rerun_published`, proven).

### The versioned write-record layer

The data layer (`State := Chan → Bool`) tracks datum presence. The record layer tracks, per
channel, its newest write — writer path, version, and post-value:

```lean
abbrev Write     := DfsPath × Nat × Bool
abbrev SpecState := Chan → Option Write
```

A produce records an addition write (post-value `true`); a matched consume records a removal write
(post-value `false`); a missing consume writes nothing. Versions increment per channel, so the
newest write is the latest committer. The two layers are kept coherent by construction:

- `applyAt_preserves_invariant` / `record_determines_value` (proven): in any reachable state, a
  channel's value **is** its newest write's recorded post-value (or the initial value if
  unwritten). This is what makes the record a *witness* — where two Bool-equal reads differ in
  which data they matched, their records differ in writer path.

### Prefix visibility: the check

```lean
def prefixVisible (s : SpecState) (p : DfsPath) (c : Chan) : Prop :=
  (s c).elim True (fun w => PathLt w.1 p)

def ValidCommit (sRead : SpecState) (p : DfsPath) (reads : Finset Chan) : Prop :=
  ∀ c ∈ reads, prefixVisible sRead p c

def DFSSerializable (st0 : State × SpecState) : List (DfsPath × Effect) → Prop
  | [] => True
  | (p, e) :: rest =>
      ValidCommit st0.2 p (Effect.footprint e) ∧ DFSSerializable (applyAt p e st0) rest
```

`DFSSerializable` steps the state in *commit* order and validates each commit against the record
the commit actually read — no separate snapshot is carried, so the check is faithful by
construction. The Rust realization computes exactly this certificate on the block path: the
committed effect's read set is its claimed footprint, and the record layer is the write-record
layer of `rspace::concurrent::channel_queue::ChannelQueue` — `record_write`/`last_write` over a
per-channel `WriteRecord` (writer path = claim path, version = write count, polarity = post-value),
armed by `set_validation_enabled` for `RelaxedValidated` only. `try_acquire` performs the prefix
visibility check under the held channel locks (the gap-free linearization point), rejecting with
`AcquireError::ValidationFailed` — the fail-fast certificate.

The certificate is **not** the acceptance gate by itself: it is a per-commit detector, and the
sequential-oracle legs remain on the accept path. The two checks are complementary, each with a
decided witness: the certificate is the only detector for the C/D pair and the persistent-produce
COMM inversion (oracle-blind — identical multisets and state), while the oracle is the only
detector for a DFS-earlier writer committing *after* a DFS-later reader
(`certificate_blind_late_writer_diverges` — both commits validate, yet the commit-order fold
diverges from the gate fold). A certificate failure fails *fast* into the sequential fallback
without running the oracle; certificate-clean runs still pass all three oracle legs.

### The witnesses

The witness theorems pin the check's shape, all proven in
`spec/Rchain/SchedulerOnchain.lean`:

- `s3_pair_fails_validation` — the depth-2 pair's B-first interleaving is **not** serializable:
  `[0,0,0]`'s produce reads channel `0` whose newest write is `[1]`'s removal — a DFS-later path.
  The scheduler aborts A's subtree and re-runs it under the gate.
- `s3_sequential_order_valid` — the DFS order itself validates: this is the run the sequential
  reducer produces, and the run the gate re-run restores.
- `later_write_pollution_unsound` — the draft "every DFS-earlier write is reflected" check is
  blind to a DFS-earlier read observing a DFS-*later* write that committed first (the C/D pair);
  only prefix visibility rejects it.
- `trace_equality_without_serializability` — the publication check is **one-directional**: the
  produce-only pair publishes the same trace as the path-order fold yet fails validation. Log
  equality is a necessary consequence, never a sufficient certificate.

Three further boundary witnesses — each a decided counterexample — settle the corrected
statements below:

- `dispatched_serializable_log_inequality` — literal log equality between the commit-order fold
  and the path-order fold is false even for dispatched, serializable runs (two independent
  produces committed out of path order). The published log is the *path-ordered drain*; the
  publication theorem must relate each commit's trace to the gate's per-path trace.
- `certificate_blind_late_writer_diverges` — the certificate's blind spot above.
- `writer_chain_needs_nodup` — one path committing twice on different channels is serializable,
  yet the writer path list repeats; the writer chain needs the path-nodup hypothesis (the
  coordinator's `Published` invariant).

### The publication theorems (proven)

- `serializable_writer_chain` — prefix visibility forces each channel's writers to commit in
  strictly increasing path order, from the initial (empty) record layer and with path-nodup.
- `dfs_serializable_implies_log_equal` — the pinned publication theorem: a dispatched,
  path-nodup run whose commits are **per-channel path-pinned** (`Pinned`: each channel's commits
  already appear in path order — what dispatch-time pre-claiming gives within one dispatch list)
  reaches the gate fold's state, and each commit emits exactly the gate's trace at its path; the
  published log — the path-ordered drain — is the gate's trace, event for event. The pin is the
  load-bearing hypothesis.
- `validated_speculation_refines_apply` — the coordinator refinement: on the accept path the
  oracle's own verdict is the state equality, on the fallback path the deploy set re-runs under
  the gate from the start state (which is the gate fold by construction); either way the
  sequential fold ships. The supporting lemmas — `record_determines_value`, `dispatched_record_at`
  (a dispatched commit only ever writes its own channel), `gate_replay_terminates`, and
  `fallback_rerun_published` (the path-sorted re-run queue keeps `Published`) — are proven. No
  axioms remain.

### Rust realization map

| Law element | Lean | Rust |
|---|---|---|
| write-record layer | `SpecState` / `applyAt` | per-channel `WriteRecord` (path, version = write count, polarity) in `rspace::concurrent::channel_queue.rs`: `record_write`/`last_write`/`reset_write_record`, armed by `set_validation_enabled` |
| prefix visibility | `prefixVisible` / `ValidCommit` | the certificate in `try_acquire` — `AcquireError::ValidationFailed` rejects a commit whose read is not prefix-visible, under the held channel locks (the gap-free linearization point) — **plus** the sequential-oracle legs on the accept path (`RuntimeManager::validate_relaxed_block`: post-state hash + per-channel COMM multisets + Law 11 rig-replay) |
| DFS-serializability | `DFSSerializable` | the certificate + oracle pair above: the certificate catches the C/D pair and the persistent-produce COMM inversion (oracle-blind); the oracle catches the late-earlier-writer divergence (certificate-blind) |
| abort / inverse-replay / gate re-run | Law 25 (`Published`, `gate_replay_terminates`, `fallback_rerun_published`) | per-deploy fallback: `SpeculationInvalid` reverts to the deploy's soft checkpoint and re-runs it sequentially; whole-set safety net: `compute_state` re-runs the deploy set on `fork_play_runtime` from `start_hash` |
| published log = sequential fold | `validated_speculation_refines_apply` | block carries the sequential trace either way: the oracle-verified relaxed trace on accept, the gate re-run's trace on fallback |

The block path therefore runs: speculate under the claim queue (Law 20) with dispatch-time
pre-claiming pinning per-channel commit order, fail fast into the sequential fallback on a
certificate rejection (Law 24's check), validate certificate-clean runs against the sequential
reference (the oracle legs), and on divergence ship the reference's results (Law 25's gate
re-run) — pure `Relaxed` remains off-chain only. The realization (the `RelaxedValidated`
block-path mode, the write-record layer with its `try_acquire` certificate, dispatch-time
pre-claiming, per-deploy fail-fast with the whole-set safety net, and the `validate_relaxed_block`
oracle) is implemented on `feature/channel-scheduler`; the phase-by-phase account is
[the on-chain validation phase plan](../contributor/onchain-validation-phases.md).
