# Concurrent reduction

[Structural congruence and reduction](congruence-reduction.md) defined the *sequential* dynamics of the
ρ-calculus: one `⟶` step contracts a single COMM redex, and reduction is a congruence under `|`. This
document extends that to the *concurrent* dynamics.

The ρ-calculus is a **concurrent** calculus: `|` is parallel composition, and the laws already grant
the permission to reduce independent sub-processes simultaneously. The node's reducer
(`rholang::reduce::DebruijnInterpreter`) exercises the Level-1 permission — it resolves a `Par`'s
*pure* sub-terms (substitution, spatial matching, `new`-allocation) concurrently, then applies the
tuple-space effects in DFS order — plus the effect-level modes of Laws 20–22 (`gate` and `relaxed`,
see [The channel scheduler](channel-scheduler.md)). *Static* effect-level partitioning is
**unsound** — see [Effect scheduling](effect-scheduling.md) S.3 — so the reducer deliberately does
not attempt it. This document is the specification of that concurrent execution model, **founded in
the 25 laws** ([`spec/INVENTORY.md`](../../../spec/INVENTORY.md)).

Throughout, "sequential" and "concurrent" are about the *scheduler*: both reduce the same `⟶` relation
and must land on the same canonical state. The difference is whether independent redexes fire one-at-a-time
or simultaneously.

## The concurrency contract in one line

> **Reduction is permitted everywhere inside `|` (`parLeft`/`parRight`), but the result must be the
> same canonical state regardless of schedule** (`reduce_deterministic`).

Everything below is a spelling-out of that sentence: which laws *grant* the permission, which laws
*fix* the result, and the theorems that say the two are compatible.

---

## A. Concurrency profile of the 25 laws

Each law is classified by its role for concurrent reduction. The citation is the Rust realization that
currently carries it (from [`contributor/laws-to-rust.md`](../contributor/laws-to-rust.md)).

### The enabling set — "you may parallelize"

| # | Law | What it grants | Rust realization |
|---|-----|----------------|------------------|
| **1** | `Par` commutative; canonicalization idempotent | Sub-process *order* is irrelevant; parallel results canonicalize identically | `Sorted<Par<S>>` (`models/src/sorted.rs`) |
| **2** | α/name equivalence: `\|` associative/commutative with `Nil`, a congruence | Reassociate the flat `Par` into independent work units | `Par<S>` structural equality (`models/src/ast.rs`) |
| **4** (core) | Reduction: `parLeft`/`parRight` | Reduction happens *anywhere inside* `\|` | `Reduce` in `spec/Rchain/Rho.lean` |
| **7** | Join commutativity (channel keys hashed in sorted order) | A multi-channel receive fires independent of message arrival order | `rspace::hashing::StableHashProvider::hash_seq` |
| **9** | Merge is a monoid; non-conflicting logs commute | Effects of *non-conflicting* COMMs compose order-independently | `rspace::merger::StateChange`/`ChannelChange` |
| **19** | `Blake2b512Random` associative splittable merge | Parallel branches carry independent RNG streams that merge deterministically | `crypto::hash::Blake2b512Random` |

### The constraining set — "the result must be deterministic"

| # | Law | What it fixes | Rust realization |
|---|-----|---------------|------------------|
| **1** (tie-break) | canonical total order | *Which* candidate is "first" when several match | `Sorted<Par>` + `space_matcher.rs` |
| **4** (full) | `reduce_deterministic`, first-match-wins | The send/receive pairing is fixed, not scheduler-chosen | `spec/Rchain/Reduce.lean` (stated) |
| **8** | Deterministic COMM (produce refs sorted; content-addressed events) | Reproducible candidate selection | `rspace::space_matcher` first-match-in-insertion-order |
| **11** | Replay determinism (recomputed COMM ⊆ recorded trace) | A re-execution — concurrent or not — reproduces the recorded trace | `rspace::ReplayRSpace` |
| **12** | Actor atomicity (single-threaded `mbox.nextMsg`) | One actor/message at a time; the *analog* here is per-channel serialization | (orphaned; carried by `TwoStepLock`) |
| **17** | Merge determinism (unique min-cost rejection); RNG merge commutative | Conflicting merges resolve to a unique winner | `NonNegI64` + `Blake2b512Random::merge` |
| **20** | Channel-task linearization (claim queue, per-channel DFS commit order) | Same-channel commits are path-ordered; a claim commits only as head of all its channels | `rspace::concurrent::channel_queue::ChannelClaimQueue` |
| **21** | DFS-gate linearization (`gate_exec_refines_apply`) | Effect `i` waits on `0..i−1` — the sequential fold, deterministically | `rholang::scheduler::EffectMode::Gate` |
| **22** | Next-step closure computable at dispatch | The matched datum is concrete, so the continuation's first-step footprint is computed at dispatch (`resolve_children`) | `rholang::reduce` |

### The supporting set — "neutral, but load-bearing"

- **Law 3** — `sort(subst t) = subst(sort t)`: substitution commutes with canonical order, so a branch
  may substitute into an already-sorted subterm without re-sorting.
- **Law 5** — a free variable is bound at most once (`BindsAtMostOnce`): a pattern's substitution is
  well-defined regardless of the order parallel-matched sub-patterns bind variables.
- **Law 6** — no globally free variables (`Closed`): preserved under `|`, `≡`, and `⟶`, so splitting a
  closed program into branches and rejoining keeps everything total.
- **Law 10** — Merkle determinism: the content-addressed trie root *is* the state, so two merges of the
  same deltas converge on the same root.
- **Laws 14–16, 18** — consensus/storage invariants; they constrain the *block* layer, not the reducer.

---

## B. Formal definitions

The grammar and `≡` are those of [`congruence-reduction.md`](congruence-reduction.md). We add:

**Channel footprint.** `chans(P)` is the finite set of channels that `P` produces on or consumes from
(the *sources* of its top-level sends and receives). For a `Par` this is the union over its terms.

**Independence.** Two processes are independent when they do not contend for any channel:

```
indep(P, Q)  ⇔  chans(P) ∩ chans(Q) = ∅
```

**Parallel step `⟹`.** A single concurrent step reduces a set of pairwise-independent redexes at once:

```
P ⟹ Q   when   P ≡ R₁ | … | Rₖ | S
               each Rᵢ ⟶ Rᵢ'   (a redex contraction)
               pairwise indep(Rᵢ, Rⱼ)  for i ≠ j
               Q ≡ R₁' | … | Rₖ' | S
```

The sequential step `⟶` is the special case `k = 1`. `⟹` subsumes `⟶`.

**Schedule.** A finite sequence of parallel steps `P = P₀ ⟹ P₁ ⟹ … ⟹ Pₙ`. The sequential reducer is the
schedule that always takes `k = 1` with the redex chosen in canonical order.

---

## C. Soundness theorems

These are the statements that make "linear scaling" *sound* — that concurrency is a scheduling freedom,
not a semantics change. Statements marked **proven** are already discharged in
[`spec/Rchain/Rho.lean`](../../../spec/Rchain/Rho.lean); **stated** are the obligations this document
adds (to be formalized as `Rchain/Concurrent.lean`).

### C.1 Independent-redex commute (**proven** — `parStep_comm`)

```
Reduce p p'  ∧  Reduce q q'   ⇒   Reduce (p'|q) (p'|q')  ∧  Reduce (p|q') (p'|q')
```

*Proof sketch.* A redex on the left and a redex on the right of `|` have disjoint channel footprints,
so the two contractions neither read nor write a channel the other touches; they commute. `≡` (Law 2)
reassociates the resulting `Par`s.

**The diamond does *not* hold on the flat `Par`.** The general statement `P ⟹ Q₁ ∧ P ⟹ Q₂ ⇒ Q₁ ≡ Q₂` is
**false**: a term with one receive and two sends on the same channel is a redex in two ways, reducing to
two inert, non-`≡` send-only terms (see `spec/Rchain/Concurrent.lean`, `reduce_not_deterministic`).
Confluence is a property of the *tree* model (explicit `par` nodes), not of the field-wise flat `Par`.
Determinism is instead a property of the **chosen schedule** (the sequential reducer's canonical order,
Law 1/4/8) — the "same normal form" invariant below holds for *that* schedule, not for arbitrary `⟹`.

### C.2 Linearization (**stated**)

The sequential reducer — one redex at a time, in canonical order (Law 1), first-match-wins (Law 4/8) —
is a valid refinement of `⟹`: for every sequential run there is a schedule of `⟹` steps with the same
normal form, and vice versa (every `⟹` schedule linearizes to some sequential run reaching a `≡`-equal
state). This is what makes the sequential `eval` loop a *correct* scheduler, and what any (future)
concurrent scheduler must preserve.

### C.3 Commutative merge (**stated**, the Law 9/17 pair)

The *effect* of a set of COMM events on the tuple space is a commutative monoid over non-conflicting
events; conflicting events resolve to a unique min-cost winner (Law 17). Hence parallel branches may
apply their effects in any order and merge to the same state. Realized by the `StateChange` monoid +
`compute_trie_actions` in `rspace/src/merger/`, and the sorted distinct-branch RNG merge in
`rholang/src/merging.rs`.

### C.4 RNG determinism (**stated**, Law 19)

Splitting a seed by index (`split_byte`/`split_short`) then merging branch RNGs **associatively and
sorted** is schedule-independent. Consequently `new`-name freshness does not depend on the interleaving
of parallel branches. The merge associativity/commutativity is already an **axiom by design** in
[`spec/Rchain/Crypto/Random.lean`](../../../spec/Rchain/Crypto/Random.lean).

---

## D. Realization map — theorem → mechanism

| Theorem | Mechanism carrying it (today, reused) | Location |
|---------|----------------------------------------|----------|
| C.1 Diamond | `≡` reassociation (`StrCong`) + per-channel atomicity | `spec/Rchain/Rho.lean`; `rspace/src/concurrent/{multi_lock,two_step_lock}.rs` |
| C.2 Linearization | canonical order `Sorted<Par>` + sorted-first candidate selection | `models/src/sorted.rs`; `rspace/src/space_matcher.rs` |
| C.3 Commutative merge | `StateChange`/`ChannelChange` monoid + `compute_trie_actions` | `rspace/src/merger/*` |
| C.4 RNG determinism | `Blake2b512Random::{split_byte,split_short,merge}` | `crypto/src/hash/blake2b512_random.rs` |
| C.5 Gate refinement | `EffectMode::Gate` (effect `i` after `0..i−1`) | `rholang/src/scheduler.rs`; `rholang/src/reduce.rs` |
| C.6 Path-ordered commit | `ChannelClaimQueue` + the phase-one/two produce split | `rspace/src/concurrent/channel_queue.rs`; `rspace/src/scheduled_space.rs` |

What is *not* present is a **static** effect-level partition — and, per
[Effect scheduling](effect-scheduling.md) S.3/S.4, a sound one does not exist at the effect level (the
sound condition is disjoint *closure*, which is not statically decidable). The sound effect-level
mechanisms are dynamic instead: the DFS gate (C.5) and the per-channel claim queue (C.6), whose
**relaxed** mode trades cross-channel interleaving freedom for a non-sequential event log and is
therefore off-chain only (the casper block paths hard-reject it). On-chain, the reducer's parallelism
is Level 1 (pure resolution) plus the sequential-equivalent gate.

---

## Scope and deferrals

This document specifies **within-deploy** concurrency: independent sub-processes of a *single* program
(`|`-composition) reducing concurrently, synchronizing only at the tuple space. It deliberately does
**not** cover:

- **Deploy-level parallelism** — deploys in a block share the whole tuple space and are *not* statically
  independent; RChain's own model treats intra-block deploys as ordered/dependent
  (`casper/src/merging.rs`), so parallelizing them changes block semantics.
- **Sharding / namespace partitioning** ("nth sharding") — a separate horizontal-scaling design, not
  present anywhere in the current tree.
- **Actor runtime** (Laws 12–13) — the Rosette VM is orphaned in this port; its fork-join barrier is
  *prior art* for the scheduler's rejoin discipline, not a component to rebuild.

> Next: the effect level of the scheduler — [Effect scheduling](effect-scheduling.md).
