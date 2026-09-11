# Effect scheduling

[Concurrent reduction](concurrent-reduction.md) established the *process-level* contract: reduction is
permitted anywhere inside `|`. This document moves down one level to the **effect level** — the concrete
`produce`/`consume` operations a reduction step issues against the tuple space — and states what an
*effect scheduler* may and may not do. It is grounded in the Lean model [`Rchain.Effect`](../../../spec/Rchain/Effect.lean),
which is the oracle for this layer.

> **Status: the naive reading of Law 9 is insufficient — and the repair is dynamic, not static.** The
> channel-sharded effect scheduler (partition a `Par`'s effects by *static channel footprint* and run
> disjoint parts concurrently) is **unsound**. This document records the counterexample and the sound
> condition (`Rchain.Effect`). The schedulers that *are* sound — the per-channel claim queue and the DFS
> gate — are Laws 20–22, specified in [The channel scheduler](channel-scheduler.md) and realized as the
> `gate`/`relaxed` effect modes.

## The effect level of reduction

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

## Concurrency profile of the 19 laws (effect level)

**Enables — "you may apply independent effects concurrently":**

| # | Law | What it grants | Rust realization |
|---|-----|----------------|------------------|
| **9** | Merge is a monoid; non-conflicting logs commute | *closure*-disjoint effects commute (see S.4 — the footprint reading S.1 is too weak) | `rspace/src/merger/*` |
| **7** | Join commutativity | a join's channel set is hashed in sorted order, so its identity is order-independent | `rspace/src/hashing/stable_hash_provider.rs` |
| **19** | `Blake2b512Random` associative splittable merge | each effect's RNG is pre-split, so `new`-freshness is schedule-independent | `crypto/src/hash/blake2b512_random.rs` |

**Constrains — "the *order* of same-channel effects is fixed":**

| # | Law | What it fixes | Rust realization |
|---|-----|---------------|------------------|
| **4** | `reduce_deterministic` | the consumed candidate is fixed, not scheduler-chosen | `rholang/src/reduce.rs` |
| **8** | Deterministic COMM | candidate selection is sorted-first by content hash | `rspace/src/space_matcher.rs`, `rspace/src/rspace.rs` |
| **11** | Replay determinism | the effect *order* is fixed — replay must reproduce the recorded trace | `rspace/src/replay_rspace.rs` |
| **10** | Merkle determinism | the trie root is the state — a given effect *set* yields the same root | `rspace/src/history/*` |
| **20** | `queue_commit_path_ordered` (+ bakery deadlock-freedom, stated) | same-channel commits follow the path-sorted claim order; a claim commits only as head of *all* its channels | `rspace/src/concurrent/channel_queue.rs` |
| **21** | `gate_exec_refines_apply` | effect `i` runs only after `0..i−1` complete — exactly the sequential fold; the one-hop variant diverges (`one_hop_depth2_diverges`) | `rholang/src/scheduler.rs` (`EffectMode::Gate`), `rholang/src/reduce.rs` |
| **22** | `next_step_closure_computable` | the matched datum is concrete at dispatch, so the continuation's first-step footprint is computable (`resolve_children`) | `rholang/src/reduce.rs` |

The subtle point the rest of this document makes precise: sorted selection (Law 8) removes the
order-sensitivity of *which stored candidate* a comm consumes, but **not** the *arrival order* — and, more
fundamentally, a continuation's effects are discovered only after its trigger runs, so the *closure* of a
"disjoint-looking" effect can reach a sibling's channel.

## Soundness theorems

### S.1 Footprint commute (the naive Law 9)

```
chans(e₁) ∩ chans(e₂) = ∅   ⇒   apply(e₁; e₂)  ≡  apply(e₂; e₁)
```

This is the natural reading of Law 9 (`Rchain.RSpace.Merge.mergeChanges_comm`): two effects touching
disjoint *channels* commute. **This statement is false at the effect level**, because `chans(e)` is only
the footprint — it does not account for the continuation closure. S.3 is the counterexample.

### S.2 Same-channel order (Law 4/8/11)

For effects sharing a channel, the candidate a comm consumes depends on arrival order. Same-channel
effects must therefore be applied in a **fixed total order** — the reducer's depth-first (DFS) order,
which is exactly the order the sequential scheduler uses and the order replay re-derives (Law 11). This
part is correct and remains a hard constraint.

### S.3 The counterexample: disjoint footprint, overlapping closure, non-commuting

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

### S.4 Closure commute (the *strengthened* Law 9 — the sound condition)

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
therefore **off-chain only**. See [The channel scheduler](channel-scheduler.md).

## Realization map — theorem → mechanism

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

## The "continuation-prepend invariant" is necessary but not sufficient

It is tempting to repair the footprint partition by "prepending" a matched continuation's effects to the
target channel's queue (so they run before the remaining pending effects on that channel). This handles
the *same-channel* ordering, but **not** the cross-channel case: a continuation's effect on channel `d`
is enqueued only *after* the trigger on `c` runs, so a sibling effect already claimed on `d` can be
applied first — exactly the S.3 race. The prepend invariant fixes the order *once both effects are in the
queue*; it cannot stop the sibling from being claimed *before* the continuation is enqueued. This is the
**one-hop enqueue race**: the scheduler may only compare effects that are *both enqueued* — and the
gate scheduler's answer is to compare them by DFS path (Law 21), while the claim queue's answer is to
let the per-channel head rule order them (Law 20). `Scheduler.lean` pins both: `one_hop_depth2_diverges`
is the prepend race as a divergence proof, and `gate_exec_refines_apply` / `queue_commit_path_ordered`
are the two repairs. Only the closure condition (S.4) would justify *more* than per-channel order, and
it is not statically checkable.

> Next: the scheduler itself — [The channel scheduler](channel-scheduler.md), then how the *data* moves
> in a comm — [Substitution and matching](substitution-matching.md).
