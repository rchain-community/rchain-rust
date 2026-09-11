# The concurrency model

> This document is the **specification of the node's concurrency model** — the statement of what may run
> concurrently, what must serialize, and *why*, founded in the 22 laws. It is the target the Lean
> formalization (`spec/Rchain/`) proves. The prior documents spell out the machinery:
> [Concurrent reduction](concurrent-reduction.md) (the reducer),
> [Effect scheduling](effect-scheduling.md) (the tuple space), and
> [The channel scheduler](channel-scheduler.md) (Laws 20–22). This page ties them together and adds the
> block level.

## The invariant in one line

> **Concurrency is a scheduling freedom, never a semantics change.** Every concurrent execution — at
> any layer — must reach the same canonical state as the purely-sequential execution. The 22 laws make
> this a *theorem*, not a convention: the reducer picks a **canonical, deterministic schedule** (DFS +
> content-sorted selection, Laws 4/8/11), and the state is content-addressed (Law 10). The *raw*
> nondeterministic `Reduce` relation is not even single-step deterministic up to `≡`, and is not
> confluent on the flat `Par` — determinism is a property of the chosen schedule, not of the relation.

## The three levels

Concurrency appears at three levels, each with its own enabling and constraining laws.

| Level | What runs concurrently | What serializes | Enabling laws | Constraining laws |
|-------|------------------------|-----------------|---------------|-------------------|
| **Reducer** (within a deploy) | the *pure* resolution of a `Par`'s terms — substitution, spatial matching, name allocation | the tuple-space effects, in DFS order | 1, 2, 19 | 4, 8 |
| **Effect** (matching + scheduling) | effects on **disjoint channels** (per-channel claim queues, Laws 20–22) | the per-channel commit order (DFS path order) | 20, 21, 22 | 4, 8, 11 |
| **Block** (validation) | replay of dependency-free blocks | DAG insertion, in topological order | 11 | 14, 15, 16, 18 |

## Level 1 — the reducer (fork-join over `|`)

A `Par` is a parallel composition (`|`, Law 2), so its sub-terms are *concurrent by construction*. The
reducer realizes this with a **fork-join**: `expand_par` resolves every term's *pure* part concurrently
(`rholang/src/reduce.rs`), then applies the *effects* (the `produce`/`consume` calls that touch the
tuple space) in DFS order.

- **Why the pure part is concurrent.** Substitution, spatial matching, and `new`-name allocation are
  side-effect-free with respect to the tuple space; cost charges are atomic and each term's RNG is
  pre-split (`split_byte`/`split_short`, Law 19). So concurrent resolution is schedule-independent.
- **Why the effects are serial.** The effect *order* fixes which datum/continuation a comm consumes
  (Law 4/8), and a continuation's channel footprint is only known after its trigger effect runs — so
  the reducer keeps effects in DFS order to reproduce the sequential candidate choices.

See [Concurrent reduction](concurrent-reduction.md) for the theorems (independent-redex commute,
linearization).

## Level 2 — effect selection (content-addressed matching)

When a channel has more than one matching datum or continuation, the space must pick one. Selection is
**sorted-first by content hash** (`rspace/src/space_matcher.rs`, Law 8) rather than newest-first. This
removes the order-sensitivity of *which stored candidate* a comm consumes. See
[Sorted matching](../node/sorted-matching.md).

## Level 3 — effect scheduling (footprint is not enough; the claim queue is)

Two effects on **disjoint channels** touch disjoint state and commute (Law 9), so they may apply
concurrently. Two effects on the **same channel** must apply in DFS order (Law 4/8/11). But the
naive reading of "disjoint" is **insufficient**: a continuation's effects are discovered only after
its trigger runs, so an effect's *closure* can reach a "disjoint-looking" sibling's channel. The
channel-sharded scheduler (partition a `Par`'s effects by static footprint, run disjoint parts
concurrently) is therefore **unsound** — see [Effect scheduling](effect-scheduling.md) S.3, and the
proved counterexample `Rchain.Effect.effect_reorder_diverges`.

The repair is the **channel scheduler** (Laws 20–22, see [The channel
scheduler](channel-scheduler.md)): do not predict anything statically. Every effect *claims* the
channels it touches at its DFS path, and commits only when it is the head of every claimed channel
(Law 20) — per-channel order is enforced *dynamically*, by the queue, not by a static partition.
The **gate** variant runs effect `i` only after all earlier effects complete (Law 21) and is
provably the sequential fold; the **relaxed** variant applies the claim queue and lets
cross-channel commits interleave freely, at the cost of a non-sequential event log — which is
precisely why it is **off-chain only** (the casper block paths hard-reject it).

## Level 4 — cross-block validation (replay is verify-only)

Block validation *replays* a block's deploys against its recorded trace and checks the recomputed
post-state root (Law 11). Replay is **verify-only**: it does not change the committed state, and each
block's replay is self-contained (it starts from the block's `pre_state_hash`, a parent's committed
root). So **dependency-free blocks** — siblings whose parents are already in the DAG — can be replayed
concurrently, each on a freshly-forked `ReplayRhoRuntime` (`casper/src/runtime_manager.rs`
`fork_replay_runtime`), and the `block_processor` then inserts them serially in topological order.

This is the node's block-throughput scaling: it uses the *same* Laws 4/8/11 determinism as the other two
levels, but at the granularity of whole blocks.

## The limits of concurrency

Three levels of concurrency are sound and implemented:

1. **Level 1 — pure resolution.** A `Par`'s sub-terms (substitution, spatial matching, `new`-allocation)
   resolve concurrently; only the tuple-space effects serialize in DFS order. Sound because pure
   resolution is side-effect-free w.r.t. the tuple space and the RNG is pre-split (Law 19).
2. **Level 3 — the channel scheduler** (Laws 20–22): the **gate** mode (provably the sequential
   fold) and the **relaxed** mode (per-channel DFS order via the claim queue, free cross-channel
   interleaving — off-chain only, since its event log differs from the sequential one).
3. **Level 4 — cross-block replay.** Dependency-free blocks replay concurrently; insertion is serial.

**Static effect-level sharding is unsound** (`Rchain.Effect.effect_reorder_diverges`). Two distinct
obstacles stand in the way, and they are different in kind — the claim queue sidesteps both by
never predicting:

- **Obstacle A — the flat `Par` is not confluent.** The node's process is the flat field-wise `Par`;
  `parMerge` erases the tree structure that records *which* send pairs with *which* receive, so reduction
  is not even single-step deterministic up to `≡` (`Concurrent.lean`, `reduce_not_deterministic`).
  Confluence is recovered only in the **tree model** (`Tree.lean`, `reduceT_confluent`), where `par` is an
  injective constructor.
- **Obstacle B — a continuation's closure is dynamic.** The sound independence criterion is disjoint
  *closure*, not disjoint *footprint* (`Rchain.Effect.effect_commute_of_disjoint_closure`). But a
  continuation's closure depends on the datum the trigger matches: a receive `for (@x ← c) { @[x, *y]!(…) }`
  only reveals its output channel at match time. So closure is not statically decidable, and no *static*
  partition (by footprint, or even by closure) is sound. The claim queue does not need it: effects
  claim what they *actually* touch, at the path they *actually* run at, and the per-channel order is
  enforced at commit time.

## The path to pure ρ-calculus thread-level concurrency

The ρ-calculus *theoretically* permits `P | Q` to reduce `P` and `Q` on independent threads, confluently.
The channel scheduler realizes the *sound subset* of that: per-channel DFS order (Law 20) plus free
cross-channel interleaving, with the block paths gated to the sequential-equivalent modes. What remains
for full confluence — each still a research project, not an incremental scheduler tweak:

1. **Represent processes as trees, not the flat `Par`.** Adopt the tree model's `Proc` (explicit,
   injective `par` nodes) as the *execution* representation, so reduction is confluent
   (`reduceT_confluent`). The flat `Par` is the field-wise quotient that *causes* the non-confluence; a
   concurrent reducer must operate on the tree and flatten only at the canonicalization boundary
   (Obstacle A).

2. **Make closures static.** Effect-level independence is disjoint closure (Obstacle B). A
   **restricted calculus** whose channel positions are statically decidable — a receive body's channels
   computable without the matched datum — would make a closure-aware sharded scheduler sound, at the
   cost of the reflection that makes the ρ-calculus higher-order. (The dynamic alternative — tracking
   closures incrementally at runtime — is exactly the claim queue's specialization to per-channel order,
   and its parallelism collapses toward the DFS order in the general case, as the relaxed contract
   states.)

3. **Canonicalize after concurrent reduction.** Confluence yields "the same result up to `≡`"; consensus
   (Law 10) needs *one* canonical state. A concurrent reducer must end with a canonicalization step
   (Law 1 sort) that flattens the tree and orders its fields, turning the `≡`-class into a single
   content-addressed state.

Until these three are discharged formally (in `spec/Rchain/`), the reducer's on-chain concurrency is
bounded to Level 1, the gate mode, and Level 4; the relaxed mode carries the off-chain parallel
case.

## Soundness theorems (the Lean targets)

The model is sound if these hold. Each is the statement that a concurrent execution matches the
sequential one.

1. **Independent-redex commute** — two parallel steps on *independent* redexes commute
   (`parStep_comm`: `Reduce p p' → Reduce q q' → Reduce (p'|q) (p'|q') ∧ Reduce (p|q') (p'|q')`).
   The full **diamond/confluence does not hold** on the flat `Par`: a term with one receive and two
   sends on one channel is a redex in two ways, reducing to two inert, non-`≡` terms (see
   `spec/Rchain/Concurrent.lean`, `reduce_not_deterministic`). Confluence is a property of the *tree*
   model (explicit `par` nodes); the flat `Par` is its field-wise quotient.
2. **Linearization** — the sequential reducer (DFS, canonical order) is a valid refinement of the
   concurrent one; both reach the same `≡`-canonical state.
3. **Disjoint commute (footprint)** — `chans(e₁) ∩ chans(e₂) = ∅ ⇒ apply(e₁; e₂) ≡ apply(e₂; e₁)`.
   **False at the effect level** (`Rchain.Effect.effect_reorder_diverges`): the footprint reading of Law 9
   ignores the continuation closure.
4. **Closure commute (the sound condition)** — `closure(e₁) ∩ closure(e₂) = ∅ ⇒ apply(e₁; e₂) ≡
   apply(e₂; e₁)` (`Rchain.Effect.effect_commute_of_disjoint_closure`). Not statically decidable, so no
   static sharded scheduler is sound — the claim queue enforces the achievable special case (per-channel
   order) dynamically instead.
5. **Gate refinement** — running effect `i` only after effects `0..i−1` complete is exactly the
   sequential `apply` fold (`Scheduler.lean`, `gate_exec_refines_apply`); the one-hop (next-step
   footprint) variant is unsound (`one_hop_depth2_diverges`).
6. **Path-ordered commit** — a claim queue's commit sequence per channel follows the path-sorted
   claim order (`Scheduler.lean`, `queue_commit_path_ordered`; deadlock-freedom stated by the bakery
   argument, `law20_deadlock_freedom`).
7. **Replay determinism** — recomputed COMM ⊆ recorded trace (Law 11), so concurrent re-validation
   reaches the recorded root.

## Formalization plan (`spec/Rchain/`)

- Already in `Rho.lean`: `StrCong` (`comm`/`assoc`/`ident`/`par`) and `Reduce` (`comm`/`parLeft`/
  `parRight`) — the *permission* for concurrent reduction.
- Already in `Random.lean` (axiom): the associative splittable RNG merge (Law 19).
- **Done** (`Concurrent.lean`): a parallel-step relation `⟹` (`ParStep`), the independent-redex commute
  (`parStep_comm`), linearization of `⟹` to `⟶`-sequences (`parStep_to_reduce`), the field-wise
  decomposition + inertness lemmas, `reduce_redex_unique` (an isolated redex is deterministic up to
  `StrCong`), and the counterexample `reduce_not_deterministic` showing the flat `Par` is not confluent.
- **Done** (`Tree.lean`): the **tree model** — `Proc` with explicit (injective) `par` nodes, `ReduceT`/
  `StrCongT`, and `reduceT_confluent` (the diamond holds up to `StrCongT`). `flatten : Proc → Par`
  bridges the two (`flatten_reduce`/`flatten_strCong`): tree confluence is a sound refinement of the
  flat `Reduce`, whose non-confluence is precisely the loss of tree structure under `parMerge`.
- **Done** (`Effect.lean`): the **effect-level model** — `Effect` (produce/consume with continuation),
  `State`, `apply`, `footprint`, `closure`. Proves the naive "disjoint footprint" lift to the tuple
  space is **unsound** (`effect_reorder_diverges`) and states the sound condition
  (`effect_commute_of_disjoint_closure`: disjoint *closure* ⇒ commute). This is what rules out the
  *static* channel-sharded effect scheduler.
- **Done** (`Scheduler.lean`): the **channel scheduler** — `DfsPath`/`PathLt` (lexicographic = DFS
  order), the claim queue with `queue_commit_path_ordered` (proven) and `law20_deadlock_freedom`
  (stated, bakery), the gate with `gate_exec_refines_apply` (proven) and the one-hop counterexample
  `one_hop_depth2_diverges` (proven), and `next_step_closure_computable` /
  `depth2_next_step_disjoint` for Law 22.

> **Formal.** The full per-law catalog is [The 22 laws](the-19-laws.md) /
> [`spec/INVENTORY.md`](../../../spec/INVENTORY.md). The machine realization of each law is
> [The 22 laws → Rust code](../contributor/laws-to-rust.md).
