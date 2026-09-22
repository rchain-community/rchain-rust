/-!
# Cross-shard transactions: two-phase commit — Laws 26–29

`Rchain.SchedulerOnchain` (Laws 23–25) formalized how *one* reducer's out-of-order commits can still
publish the sequential fold. This module is the next layer up: it formalizes how **two independent
shards** — separate chains/tuple-spaces, each with its own deploy endpoint — coordinate a single
transaction so that it commits on both or aborts on both. The transport is the client-side
remote-deploy primitive (`casper/src/shard_invoke.rs`); what is specified here is the **two-phase
commit** discipline that turns two idempotent legs into one atomic unit
(`docs/src/formal/cross-shard-transactions.md`).

* **Law 26 — shard scope determinism.** A deploy/block's effects bind to exactly one shard; the shard
  id is a validated, ordered value, and the RNG seed + unforgeable names are shard-scoped.
  `shard_scope_deterministic`.

* **Law 27 — cross-shard atomicity (2PC).** A transaction commits on every participant or aborts on
  every participant — no run leaves a strict subset committed. `txn_atomic`.

* **Law 28 — leg idempotency.** `prepare`/`commit`/`abort` are idempotent under the transaction id,
  so re-delivery and client retry are safe. `leg_idempotent`.

* **Law 29 — decision durability & record determinism.** The coordinator's decision is a durable,
  content-addressed record; a prepared participant recovers it (the 2PC *blocking* caveat).
  `commit_record_deterministic`.
-/

namespace Rchain

/-! ## Shard identity — Law 26 -/

/-- A shard id: the name of one chain/tuple-space. The Rust realization is the `ShardId` newtype
    (`shared/src/refined.rs`) — a validated, ordered `/`-separated path (`/root`), with the hierarchy
    `parent-shard-id`/`shard-name` resolved by `ShardSpec` (a node's memberships are the non-empty
    `ShardMemberships` list). The model abstracts it as a
    `String`; `ValidShardId` is the non-empty half of the validation. -/
abbrev ShardId := String

/-- A shard id is valid when it is non-empty (the "validated" half of the newtype). -/
def ValidShardId (s : ShardId) : Prop := s ≠ ""

/-! ## The transaction model -/

/-- The transaction lifecycle (the Git pull-request analogy): `proposed` (open PR) → `prepared`
    (both branches staged and voted ready) → `committed` (merged), or `aborted` (closed without
    merge). -/
inductive TxnState where
  | proposed | prepared | committed | aborted
  deriving DecidableEq, Repr

/-- A phase-one vote: a participant is `ready` to commit, or votes to `abort`. -/
inductive Vote where
  | ready | abort
  deriving DecidableEq, Repr

/-- One leg of a transaction: a shard's effect and its compensating inverse, both opaque
    content-addressed term handles. -/
structure Leg where
  shard : ShardId
  effect : Nat
  compensation : Nat
  deriving Repr

/-- A transaction: a content-addressed id plus its legs (one per participant shard). -/
structure Txn where
  txn_id : Nat
  legs : List Leg
  deriving Repr

/-- A participant's terminal outcome. -/
inductive Outcome where
  | committed | aborted
  deriving DecidableEq, Repr

/-- The coordinator's durable, content-addressed record — the PR and its eventual merge/close
    commit — carrying the transaction, its recorded state, and the votes it collected. -/
structure CoordRecord where
  txn : Txn
  state : TxnState
  votes : List (ShardId × Vote)
  deriving Repr

/-- A run's terminal configuration: the outcome of every participant. -/
abbrev Run := List Outcome

/-! ## Law 26 — shard scope determinism -/

/-- **Law 26.** A leg carries a *valid* shard id, and a deploy/block's effects bind to exactly that
    one shard; the shard id names the unforgeable names and seeds the RNG
    (`casper/src/block_random_seed.rs`), so shard scope is a function of the id. **Stated.** -/
axiom shard_scope_deterministic (l : Leg) : ValidShardId l.shard

/-! ## Law 27 — cross-shard atomicity (2PC) -/

/-- A run is **uniform** when every participant reached the same terminal outcome — all
    `committed` or all `aborted`. -/
def uniform (r : Run) : Prop :=
  (∀ o ∈ r, o = Outcome.committed) ∨ (∀ o ∈ r, o = Outcome.aborted)

/-- **Law 27.** Every run is uniform: the transaction commits on all participants or aborts on
    all; no run leaves a strict subset committed. **Stated.** -/
axiom txn_atomic (r : Run) : uniform r

/-! ## Law 28 — leg idempotency -/

/-- A leg's effect applied to per-shard state: it overwrites its own shard's value with the effect
    handle and leaves other shards untouched. -/
def applyEffect (st : ShardId → Nat) (l : Leg) : ShardId → Nat :=
  fun s => if s = l.shard then l.effect else st s

/-- **Law 28.** Applying a leg's effect twice is applying it once: `prepare`, `commit` and `abort`
    are idempotent under the transaction id, so re-delivery and client retry never re-apply the
    effect or its compensation. **Stated.** -/
axiom leg_idempotent (st : ShardId → Nat) (l : Leg) :
  applyEffect (applyEffect st l) l = applyEffect st l

/-! ## Law 29 — decision durability & record determinism -/

/-- **Law 29.** The coordinator's recorded state is the deterministic decision of its votes —
    `committed` iff every participant voted `ready` — so a `prepared` participant can always recover
    the decision from the durable record, and the merge/close record is re-derivable on replay.
    **Stated** (the recovery window is the 2PC blocking caveat: a prepared participant holds its
    lock until it recovers the decision). -/
axiom commit_record_deterministic (r : CoordRecord) :
  r.state = TxnState.committed ↔ (∀ v ∈ r.votes, v.2 = Vote.ready)

end Rchain
