# Experimental: TreeProc + zero-action ledger concurrent reducer

> **Status: experimental phase plan.** This document lives on the
> `experimental/zfa-concurrent-reducer` branch and tracks issue
> [rchain-community/rchain-rust#8](https://github.com/rchain-community/rchain-rust/issues/8).
> Nothing here is on the `dev` critical path until each phase is demonstrated and CI-green.

## 1. Goal

The stable `dev` reducer already resolves the *pure* parts of a `Par`'s sub-terms
concurrently, but applies tuple-space effects in DFS order
([`rholang/src/reduce.rs`](../../../rholang/src/reduce.rs)). Issue #8 asks whether an
explicit process tree and a local "zero-action" ledger can safely apply **more** of a
deploy's independent work concurrently.

That question is constrained by two existing formal results:

- [`docs/src/formal/concurrent-reduction.md`](../formal/concurrent-reduction.md) C.1 proves
  independent redexes commute.
- [`docs/src/formal/effect-scheduling.md`](../formal/effect-scheduling.md) S.3/S.4 proves that
  **static channel-footprint partitioning is unsound**: the sound criterion is disjoint
  *closure*, which is not statically decidable.

Therefore this branch does **not** claim that "an unmatched branch is dropped" or that
"locks disappear". It tests a narrower, sound hypothesis:

> Can an explicit `TreeProc` plus a dynamic `ZfaLedger` make **closure-disjoint** effect
> groups discoverable and committable concurrently, while every closure-overlapping group
> still executes in deterministic DFS order?

A scheduler that does this would preserve Laws 4, 8, and 11 and still exceed the current
Level-1 parallelism on workloads whose closures are disjoint.

## 2. Baseline vocabulary

| Name | Location | Role |
|---|---|---|
| `Par<S>` | [`models/src/ast.rs`](../../../models/src/ast.rs) | Flat, field-wise process record; canonical order is *not* structural until sorted. |
| `Sorted<S>` / `SortedProc` | [`models/src/sorted.rs`](../../../models/src/sorted.rs) | Refinement type carrying Law 1's canonical order by construction. |
| `sort_par_term` | [`models/src/sorter.rs`](../../../models/src/sorter.rs) | The Law 1 canonicalizer. |
| `OwnedTerm` / `Effect` | [`rholang/src/reduce.rs`](../../../rholang/src/reduce.rs) | Current flat work units and tuple-space effects. |
| `TwoStepLock` | [`rspace/src/concurrent/two_step_lock.rs`](../../../rspace/src/concurrent/two_step_lock.rs) | Per-channel atomicity used by RSpace produce/consume. |
| `RSpace` | [`rspace/src/rspace.rs`](../../../rspace/src/rspace.rs) | Deterministic, replayable tuple space (Laws 7–11). |

Current behavior, in one paragraph:

1. `DebruijnInterpreter::reduce_par` flattens a `Par` into `OwnedTerm`s.
2. `resolve_children` resolves the pure parts (channel/data evaluation, substitution,
   `new` allocation, match selection) concurrently.
3. `reduce_effects` applies the resulting `Effect`s **sequentially in DFS order**.
4. Each RSpace `produce`/`consume` is atomic under a `TwoStepLock` keyed by channel hashes.

There is no effect-level scheduler in `dev`.

## 3. Phase plan

Each phase is self-contained, committed to this branch, and must pass `cargo fmt`,
clippy with the repo allowlist, and `cargo test` before the next phase starts.

### Phase 0 — branch and design plan

**Done by this document.**

- Branch `experimental/zfa-concurrent-reducer` created from `dev`.
- Design/phase plan recorded here.
- `dev` is untouched.

**Acceptance:** branch exists; document is committed; CI on `dev` remains green.

### Phase 1 — `TreeProc` execution representation

A `TreeProc` is an **execution** representation, not a new wire format or a replacement
for `Par`. It is deliberately non-canonical: `Par` children are binary nodes and no
absorption/sorting is performed until the canonicalization boundary.

```rust
// Proposed module: rholang/src/tree_proc.rs
//
// Leaves mirror the current private OwnedTerm in rholang/src/reduce.rs.  The tree keeps
// parallel structure explicit so a scheduler can move whole subtrees to workers without
// first destroying the boundary between independent sub-processes.

pub enum TreeProc {
    /// Parallel composition, kept as an explicit binary spine.
    Par {
        left: Box<TreeProc>,
        right: Box<TreeProc>,
    },
    Nil,
    Send(Send),
    Receive(Receive),
    New(New),
    Match(Match),
    Bundle(Bundle),
    /// Only EVar / EMethod leaves reach this variant.
    Expr(Expr),
}

impl TreeProc {
    /// Build a TreeProc from a flat Par.  Sibling order is taken from the source Par's
    /// field order; no `sort` is applied inside the tree.
    pub fn from_par(par: &Par) -> Self { /* ... */ }

    /// Law 1 boundary: flatten every ParNode and canonicalize once.
    pub fn into_sorted_par(self) -> SortedProc { /* sort_par_term(...) */ }
}
```

Why a tree? The formal concurrent-reduction document notes that the diamond does *not*
hold on the flat `Par`; an explicit `par` node is the place where the calculus grants
`parLeft`/`parRight` reduction permission. The tree does not create new permissions; it
preserves the topology long enough for a scheduler to group effects correctly.

Memory model:

- A `TreeProc` is an **owned, immutable tree**.
- Each worker receives one owned subtree; there is no `Arc<Mutex<TreeNode>>`.
- Only already-shared immutable resources are cloned into workers: `Arc<BTreeMap<String, Par>>`
  (urn map), `Arc<CostAccounting>`, and a deterministic per-branch RNG split.
- At a rejoin, workers return either a canonical tail (`SortedProc`) or resolved
  `Effect`s, never a shared mutable node.

Determinism hooks (needed later, specified now):

- Every `ParNode` edge carries a stable side bit so a path from the root is a
  deterministic `TreePath`.
- `TreePath` is used for `split_rand` and for replay/event ordering so completion order
  cannot change fresh-name allocation or the recorded trace.

**Acceptance criteria (Phase 1):**

1. Property: for every `Par` `p`, `TreeProc::from_par(p).into_sorted_par().as_par() ==
   sort_par_term(&p)`.
2. Property: canonicalization is idempotent (Law 1), using the existing `sort` oracle.
3. Property: swapping `TreeProc::Par` children does not change
   `into_sorted_par()` (Law 2 / Law 1 tie-break).
4. Property: `TreeProc::Par(Nil, p)` canonicalizes equal to `sort(p)` (Nil absorption).
5. No `sort_par_term` call occurs inside `from_par`; the only canonicalization is at
   `into_sorted_par`.

### Phase 2 — `ZfaLedger` pure conflict oracle

The ledger in issue #8 was described as replacing mutexes by "Inject +1, Consume -1,
must sum to zero". Taken literally that is not a complete RSpace semantics: an unmatched
produce legitimately becomes stored data, and an unmatched receive legitimately becomes a
waiting continuation. The safe use of the ledger is therefore **conflict detection**, not
semantics selection.

```rust
// Proposed module: rspace/src/concurrent/zfa_ledger.rs
//
// Generic over the concrete channel key so the crate does not need a models dependency;
// rholang instantiates it with SortedProc (or Blake2b256Hash of SortedProc).

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ZfaLedger<C> {
    /// One unit of "in-flight sibling action" per concrete channel.
    balances: BTreeMap<C, i64>,
}

impl<C: Ord + Clone> ZfaLedger<C> {
    pub fn new() -> Self;

    /// Record one +1 (Inject/produce) or -1 (Consume/receive demand).
    pub fn record(&mut self, channel: C, delta: i64);

    /// All balances are zero.
    pub fn is_zero_action(&self) -> bool;

    /// True when this ledger and `other` touch no channel in common.
    pub fn is_disjoint(&self, other: &Self) -> bool;

    /// Union the two ledgers (used when two components merge).
    pub fn merge(&mut self, other: Self);
}
```

Important distinction from the original issue sketch:

- A **balanced** pair `Inject(x), Consume(x)` represents two sibling effects in the same
  tree that can form a local COMM proposal.
- An **unbalanced** singleton `Inject(x)` or `Consume(x)` is *not invalid*; it is an
  effect that must be stored in RSpace or must wait for an existing RSpace counterpart.
- If two proposed groups overlap on a channel and both try to consume the same physical
  datum, the ledger reports the conflict; the scheduler must then **serialize them in DFS
  order**, not silently drop one.

**Property tests (Phase 2):**

1. `Inject(x); Consume(x)` ⇒ `is_zero_action()`.
2. `Inject(x); Inject(x)` ⇒ not zero.
3. `Inject(x); Consume(y)`, `x != y` ⇒ not zero.
4. For any sequence of deltas, `ZfaLedger` result is independent of record order.
5. Disjoint ledgers: `merge(a, b) == merge(b, a)` and `is_disjoint` is symmetric.
6. The balance type cannot overflow in debug/test builds (checked add in tests).

**Acceptance criteria (Phase 2):** all properties above pass with proptest/quickcheck; no
RSpace or reducer behavior is changed.

### Phase 3 — Dynamic closure scheduler integration

This is the phase where the actual concurrency improvement is measured. It is deliberately
the last phase because it must not start until Phases 1 and 2 are demonstrated.

Scheduler shape (concrete types):

```rust
// Proposed module: rholang/src/scheduler.rs (experimental)

pub struct Component {
    /// Deterministic ordering key: DFS path of the component's first unresolved effect.
    order: TreePath,
    /// Channels touched by effects currently staged in this component.
    ledger: ZfaLedger<SortedProc>,
    /// Effects not yet applied to RSpace.
    staged: VecDeque<Effect>,
}

pub struct ComponentGraph {
    /// Components keyed by a representative channel; overlapping components merge.
    components: BTreeMap<SortedProc, Component>,
}
```

Algorithm sketch:

1. Reduce a `TreeProc` to resolved `Effect`s, exactly as today's pure-resolution phase.
2. Assign each `Effect` to the component containing any of its concrete channels; if no
   component contains those channels, start a new component with its `TreePath` order.
   Effects on multiple channels can merge several components.
3. When two components merge, keep the smaller `TreePath` as the component order and
   **re-serialize** their staged effects by DFS order. This is the enforcement of
   Law 4/8/11 for overlapping closure.
4. Commit **disjoint components concurrently** to RSpace through the existing
   `TwoStepLock`; each produce/consume remains atomic.
5. When an RSpace COMM returns a continuation, the continuation's effects are staged into
   the **same component** (the trigger component). This is the dynamic-closure expansion
   required by effect-scheduling S.4.
6. All newly produced process tails are canonicalized through `Sorted::new` before they
   re-enter the scheduler.

The ledger is used as an invariant/conflict oracle at each merge: if a merged component is
balanced on a channel it means the tree contains both sides of a sibling COMM; if not, the
component may still be valid as stored RSpace state. The ledger never chooses which datum
wins; Law 8's sorted-first candidate selection and DFS order choose that.

**Acceptance criteria (Phase 3):**

1. Differential test: for the existing Scala/oracle-derived rholang test corpus, the
   sharded scheduler's final state hash equals the sequential scheduler's final state hash.
2. Replay test: `ReplayRSpace` accepts the sharded scheduler's trace exactly (Law 11).
3. Adversarial test: the S.3 counterexample from `effect-scheduling.md` produces the same
   state as sequential DFS (i.e. the scheduler does **not** exploit footprint-disjoint
   closure-overlap).
4. Microbenchmark: a synthetic corpus of independent contracts on disjoint channels shows
   measurable throughput/speedup over `set_concurrent(false)` *and* over today's Level-1
   DFS apply loop.
5. No changes to `dev` semantics; this module is gated behind an experimental flag or is
   not wired into the production `eval` path until reviewed.

## 4. Open risks and decisions

| Risk | Mitigation / open question |
|---|---|
| Static footprint is unsound (S.3). | The scheduler must dynamically merge by *resolved* channels and by continuation component, preserving DFS within merged components. Phase 3 has an adversarial test for S.3. |
| A continuation's closure is discovered only after its trigger runs. | Continuation effects join the trigger component; no effect may be committed "early" to a disjoint component before its dynamic closure is known. |
| Replay trace order. | Effects keep `TreePath` order; commit results are re-ordered to the sequential trace before recording (Law 11). |
| RNG / `new` freshness. | Each `TreeProc` edge has a stable path; `split_rand` must use the path, not completion order. |
| Persistent/peek re-produce. | Persistent and peek follow-ons touch the same channels as their original produce/consume; they must be staged in the same component. |
| Naming collision with QuCalc's ZFA. | QuCalc's ZFA is the quantum Pauli closure predicate; issue #8's ZFA is the action-balance ledger. They share only the "zero free action" intuition. This document uses "zero-action ledger" where ambiguity matters. |

## 5. Relationship to existing formal documents

- [Concurrent reduction](../formal/concurrent-reduction.md) — the process-level permission
  (`parLeft`/`parRight`) and the linearization obligation.
- [Effect scheduling](../formal/effect-scheduling.md) — S.3/S.4, the soundness boundary
  this branch must respect.
- [The 19 laws](../formal/the-19-laws.md) — Laws 1, 4, 7, 8, 9, 11 are the invariants each
  phase tests.
- [`spec/INVENTORY.md`](../../../spec/INVENTORY.md) — canonical per-law source of truth.
