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

/-- A transaction record's state, as the port has it: **three** variants
    (`rholang/src/native_state.rs:106-111`). The coordinator's PR-style `proposed` this model used to
    carry was not a record state at all — a transaction with no record is `Ledger.record = none`, which
    is how the verbs below spell it (they return `"txn commit: unknown transaction"`). -/
inductive TxnState where
  | prepared | committed | aborted
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

/-- Applying a leg's effect twice is applying it once — `funext` and a case split, which is what makes
    it a theorem rather than the axiom it was. -/
theorem leg_idempotent (st : ShardId → Nat) (l : Leg) :
    applyEffect (applyEffect st l) l = applyEffect st l := by
  funext s
  by_cases h : s = l.shard <;> simp [applyEffect, h]

/-! ### The ledger the port's verbs actually write

`applyEffect` is the per-shard view, and the port's `prepare`/`commit`/`abort` are not pointwise
updates: they read a record, decide, and write both a vault balance and a record
(`rholang/src/native_state.rs:873-966`). Modelled here so the law is about those verbs: idempotence
comes from the **early return on an existing record** (which the port takes *before* the balance check,
`:881-883`), and the fences — commit after abort is an error, abort after commit is an error
(`:912`, `:944`) — are stated too, because idempotence alone would permit both. -/

/-- A transaction record (the port's `TxnRecord`, `native_state.rs:135-141`). The coordinator key is
    part of the port's record and is not modelled: no rule below reads it. -/
structure TxnRecord where
  state : TxnState
  amount : Nat
  src : String
  dst : String
  deriving Repr

/-- The store the port keeps: vault balances under `PREFIX_VAULT` and transaction records under
    `PREFIX_TXN`, both keyed by a hash (`native_state.rs:838-869`) — modelled as maps, with the
    transaction id a `Nat` as it is in `Txn.txn_id`. -/
structure Ledger where
  vault : String → Nat
  txn : List (Nat × TxnRecord)

/-- The record for a transaction id, if any (the port's `txn`, `native_state.rs:859-864`). -/
def Ledger.record (g : Ledger) (id : Nat) : Option TxnRecord :=
  (g.txn.find? (fun p => p.1 == id)).map (fun p => p.2)

/-- Write a record (the port's `set_txn`, `native_state.rs:867-869`). -/
def Ledger.setRecord (g : Ledger) (id : Nat) (r : TxnRecord) : Ledger :=
  { g with txn := (id, r) :: g.txn.filter (fun p => p.1 ≠ id) }

/-- Write a vault balance (the port's `set_vault_balance`, `native_state.rs:838-844`). -/
def Ledger.setVault (g : Ledger) (a : String) (v : Nat) : Ledger :=
  { g with vault := fun x => if x = a then v else g.vault x }

/-- A write is readable back — the bridge the idempotence proofs need. -/
theorem Ledger.record_setRecord (g : Ledger) (id : Nat) (r : TxnRecord) :
    (g.setRecord id r).record id = some r := by
  simp [Ledger.setRecord, Ledger.record]

/-- `txn_prepare` (`native_state.rs:873-901`): escrow `amount` from the payer's vault. An existing record is
    returned **before** the balance check — that early return *is* the idempotence, and its position is
    why a retry cannot fail on a state the first call already accepted. -/
def txnPrepare (g : Ledger) (id : Nat) (payer payee : String) (amount : Nat) : Except String Ledger :=
  match g.record id with
  | some _ => .ok g
  | none =>
    if g.vault payer < amount then .error "txn prepare: insufficient balance"
    else
      .ok ((g.setVault payer (g.vault payer - amount)).setRecord id
        { state := .prepared, amount := amount, src := payer, dst := payee })

/-- `txn_commit` (`native_state.rs:906-935`): pay the escrow to the payee. A committed record is
    returned unchanged; an aborted one is an error. -/
def txnCommit (g : Ledger) (id : Nat) : Except String Ledger :=
  match g.record id with
  | none => .error "txn commit: unknown transaction"
  | some r =>
    match r.state with
    | .committed => .ok g
    | .aborted => .error "txn commit: already aborted"
    | .prepared =>
      .ok ((g.setVault r.dst (g.vault r.dst + r.amount)).setRecord id { r with state := .committed })

/-- `txn_abort` (`native_state.rs:938-966`): return the escrow to the payer. An aborted record is
    returned unchanged; a committed one is an error. -/
def txnAbort (g : Ledger) (id : Nat) : Except String Ledger :=
  match g.record id with
  | none => .error "txn abort: unknown transaction"
  | some r =>
    match r.state with
    | .aborted => .ok g
    | .committed => .error "txn abort: already committed"
    | .prepared =>
      .ok ((g.setVault r.src (g.vault r.src + r.amount)).setRecord id { r with state := .aborted })

/-- A ledger that already holds a record for `id` is a **fixed point** of `prepare` — which is what the
    port's early return *is* (`native_state.rs:881-883`). The idempotence below is its consequence. -/
theorem txnPrepare_fixes (g : Ledger) (id : Nat) (payer payee : String) (amount : Nat)
    (r : TxnRecord) (h : g.record id = some r) : txnPrepare g id payer payee amount = .ok g := by
  simp [txnPrepare, h]

/-- **Law 28 (prepare)** — a retried `prepare` leaves the ledger the first call produced: the second
    call finds the record and returns it unchanged, so the escrow is never taken twice. -/
theorem txnPrepare_idempotent (g : Ledger) (id : Nat) (payer payee : String) (amount : Nat)
    (g' : Ledger) (h : txnPrepare g id payer payee amount = .ok g') :
    txnPrepare g' id payer payee amount = .ok g' := by
  cases hr : g.record id with
  | some r =>
    have hok : (Except.ok g : Except String Ledger) = Except.ok g' := by
      simpa [txnPrepare, hr] using h
    have hg : g = g' := by injection hok
    subst hg
    exact txnPrepare_fixes g id payer payee amount r hr
  | none =>
    by_cases hb : g.vault payer < amount
    · simp [txnPrepare, hr, hb] at h
    · simp only [txnPrepare, hr, if_neg hb, Except.ok.injEq] at h
      subst h
      exact txnPrepare_fixes _ id payer payee amount _ (Ledger.record_setRecord _ id _)

/-- A ledger holding a **committed** record is a fixed point of `commit` (`native_state.rs:910-912`). -/
theorem txnCommit_fixes (g : Ledger) (id : Nat) (r : TxnRecord) (h : g.record id = some r)
    (hr : r.state = .committed) : txnCommit g id = .ok g := by
  simp [txnCommit, h, hr]

/-- **Law 28 (commit)** — a retried `commit` pays the escrow once: the second call returns the
    committed record unchanged. -/
theorem txnCommit_idempotent (g : Ledger) (id : Nat) (g' : Ledger)
    (h : txnCommit g id = .ok g') : txnCommit g' id = .ok g' := by
  cases hr : g.record id with
  | none => simp [txnCommit, hr] at h
  | some r =>
    cases hs : r.state with
    | committed =>
      have hok : (Except.ok g : Except String Ledger) = Except.ok g' := by
        simpa [txnCommit, hr, hs] using h
      have hg : g = g' := by injection hok
      subst hg
      exact txnCommit_fixes g id r hr hs
    | aborted => simp [txnCommit, hr, hs] at h
    | prepared =>
      simp only [txnCommit, hr, hs, Except.ok.injEq] at h
      subst h
      exact txnCommit_fixes _ id _ (Ledger.record_setRecord _ id _) rfl

/-- A ledger holding an **aborted** record is a fixed point of `abort` (`native_state.rs:942-944`). -/
theorem txnAbort_fixes (g : Ledger) (id : Nat) (r : TxnRecord) (h : g.record id = some r)
    (hr : r.state = .aborted) : txnAbort g id = .ok g := by
  simp [txnAbort, h, hr]

/-- **Law 28 (abort)** — a retried `abort` returns the escrow once. -/
theorem txnAbort_idempotent (g : Ledger) (id : Nat) (g' : Ledger)
    (h : txnAbort g id = .ok g') : txnAbort g' id = .ok g' := by
  cases hr : g.record id with
  | none => simp [txnAbort, hr] at h
  | some r =>
    cases hs : r.state with
    | aborted =>
      have hok : (Except.ok g : Except String Ledger) = Except.ok g' := by
        simpa [txnAbort, hr, hs] using h
      have hg : g = g' := by injection hok
      subst hg
      exact txnAbort_fixes g id r hr hs
    | committed => simp [txnAbort, hr, hs] at h
    | prepared =>
      simp only [txnAbort, hr, hs, Except.ok.injEq] at h
      subst h
      exact txnAbort_fixes _ id _ (Ledger.record_setRecord _ id _) rfl

/-- **And the two fences**, which idempotence alone would not give: the terminal verbs refuse each
    other, with the port's own message (`native_state.rs:912`). -/
theorem commit_after_abort_is_an_error (g : Ledger) (id : Nat) (r : TxnRecord)
    (h : g.record id = some r) (hr : r.state = .aborted) :
    txnCommit g id = .error "txn commit: already aborted" := by
  simp [txnCommit, h, hr]

/-- Aborting a committed transaction is an error (`native_state.rs:944`). -/
theorem abort_after_commit_is_an_error (g : Ledger) (id : Nat) (r : TxnRecord)
    (h : g.record id = some r) (hr : r.state = .committed) :
    txnAbort g id = .error "txn abort: already committed" := by
  simp [txnAbort, h, hr]

/-- An overdraft is refused with the port's message, **and refused before any write**: the ledger is
    returned untouched only as an `Except.error`, so nothing was escrowed
    (`native_state.rs:886`). -/
theorem prepare_refuses_overdraft (g : Ledger) (id : Nat) (payer payee : String) (amount : Nat)
    (h : g.record id = none) (hb : g.vault payer < amount) :
    txnPrepare g id payer payee amount = .error "txn prepare: insufficient balance" := by
  simp [txnPrepare, h, hb]

/-! ## Law 29 — decision durability & record determinism -/

/-- **Law 29.** The coordinator's recorded state is the deterministic decision of its votes —
    `committed` iff every participant voted `ready` — so a `prepared` participant can always recover
    the decision from the durable record, and the merge/close record is re-derivable on replay.
    **Stated** (the recovery window is the 2PC blocking caveat: a prepared participant holds its
    lock until it recovers the decision). -/
axiom commit_record_deterministic (r : CoordRecord) :
  r.state = TxnState.committed ↔ (∀ v ∈ r.votes, v.2 = Vote.ready)

end Rchain
