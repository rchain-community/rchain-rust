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
  id is a validated, ordered value, and the RNG seed + unforgeable names are shard-scoped. **The
  statement this module used to carry as an axiom (`∀ l, ValidShardId l.shard`) was false and is
  refuted** (`shard_scope_deterministic_is_false`); the law's real content is about the gateway's
  ingress, and stating it needs that function modelled.

* **Law 27 — cross-shard atomicity (2PC).** The transaction reaches one decision, and every leg that
  prepared follows it. **The axiom this module used to carry (`∀ r : Run, uniform r`) was false over
  `Run := List Outcome` and is refuted** (`txn_atomic_is_false`); the narrowed statement is `run_2pc`'s
  (`txn_coordinator.rs:152-192`).

* **Law 28 — leg idempotency.** `prepare`/`commit`/`abort` are idempotent under the transaction id,
  so re-delivery and client retry are safe. `leg_idempotent` — a **theorem** about the model's own
  `applyEffect`, not an axiom.

* **Law 29 — decision durability & record determinism.** The coordinator's decision is a function of
  its votes (proved: `coordinator_decision_committed_iff` over `allReady`/`coordinatorDecision`, which
  are the port's own two lines) and a durable record a prepared participant recovers. **The axiom this
  module used to carry was false** — it asserted a property of *every* `CoordRecord`, a free type —
  and is refuted (`commit_record_deterministic_is_false`); the durability half stays owed.
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

/-- **Law 26 was stated as `∀ l : Leg, ValidShardId l.shard`, and that statement is FALSE** — `Leg` is
    a freely constructible record (`:58`), so an empty shard id is an inhabitant the statement cannot
    hold of. The axiom is deleted and the refutation published in its place (2026-09-23, the
    consolidation pass's own remedy: "a false axiom is worse than an owed one, because anything follows
    from it", and each of the nine it found is refuted rather than dropped quietly).

    **What the law is actually about**: the *gateway's ingress*. The port validates a leg's shard id at
    the boundary — `ShardId::try_from` on `TxnLegDto.shard_id` (`node/src/web/http.rs:203-218`, with the
    400 the boundary test pins) — so the claim to state is about the function that admits a leg, not
    about every record the model can build. Stating it needs that function modelled (Step 4 of the
    plan); until then the row is `owed` with this sentence as its narrowed statement. -/
theorem shard_scope_deterministic_is_false :
    ¬ (∀ l : Leg, ValidShardId l.shard) :=
  fun h => (h ⟨"", 0, 0⟩) rfl

/-! ## Law 27 — cross-shard atomicity (2PC) -/

/-- A run is **uniform** when every participant reached the same terminal outcome — all
    `committed` or all `aborted`. -/
def uniform (r : Run) : Prop :=
  (∀ o ∈ r, o = Outcome.committed) ∨ (∀ o ∈ r, o = Outcome.aborted)

/-- **Law 27 was stated as `∀ r : Run, uniform r`, and that statement is FALSE** — `Run` is
    `List Outcome` (`:84`), so `[committed, aborted]` refutes it. Deleted, and the refutation published
    in its place, as law 26's was.

    **What the law is actually about**, read off the port (`casper/src/txn_coordinator.rs:152-192`): the
    phase-two outcomes of the legs that *prepared*. `all_ready` decides commit-or-abort once, and phase
    two then applies that one decision to every leg that voted ready — a leg that did not prepare is
    never locked and gets an error rather than an outcome. So uniformity holds over the prepared set,
    and the claim to state is `run_2pc`'s, not `List Outcome`'s. The Rust's own comment on
    `vote_from_reply` (`:196-215`) is the differential reference for *why* a re-run answering
    `committed` must count as ready: reading it as "not ready" would abort the other legs and leave one
    shard committed and another aborted — Law 27 broken on exactly the retry path recovery makes
    reachable. -/
theorem txn_atomic_is_false : ¬ (∀ r : Run, uniform r) := by
  intro h
  have hv := h [Outcome.committed, Outcome.aborted]
  rcases hv with h1 | h2
  · exact absurd (h1 Outcome.aborted (by simp)) (by simp)
  · exact absurd (h2 Outcome.committed (by simp)) (by simp)

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

/-- **Law 29 was stated as a property of *every* `CoordRecord`, and that statement is FALSE** — the
    record is freely constructible (`:77`), so `{state := committed, votes := [("", abort)]}` refutes
    it. Deleted, and the refutation published in its place.

    **What is provable now, and what is not.** The *decision* half is a function in the port, so it is
    modelled below and proved (`coordinator_decision_committed_iff`): `committed` iff every vote is
    ready, which is the Rust's `all_ready` line for line. The *durability* half — that a `prepared`
    participant can recover that decision from the durable record — needs the coordinator's record
    writes modelled (Step 4 of the plan); the row stays `owed` with this sentence as its narrowed
    statement. -/
theorem commit_record_deterministic_is_false :
    ¬ (∀ r : CoordRecord,
        r.state = TxnState.committed ↔ (∀ v ∈ r.votes, v.2 = Vote.ready)) := by
  intro h
  have hv := (h { txn := ⟨0, []⟩, state := TxnState.committed, votes := [("", Vote.abort)] }).mp rfl
  exact absurd (hv ("", Vote.abort) (by simp)) (by simp)

/-! ## The coordinator's decision, modelled from the port

`txn_coordinator.rs:177-179` is two lines — `let all_ready = ready.iter().all(|&b| b)` and
`let decision = if all_ready { "commit" } else { "abort" }` — over the *booleans* `vote_from_reply`
produces. The model below is those two lines, so the decision half of law 29 is a theorem about the
port's own function rather than a claim about an arbitrary record. -/

/-- Every vote is a ready vote — the Rust's `ready.iter().all(|&b| b)`. -/
def allReady : List Vote → Bool
  | [] => true
  | v :: vs => decide (v = Vote.ready) && allReady vs

/-- The coordinator's decision: `committed` iff every vote was ready. -/
def coordinatorDecision (votes : List Vote) : TxnState :=
  if allReady votes then TxnState.committed else TxnState.aborted

/-- `all`-fold is `true` exactly when every element satisfies the predicate. -/
theorem allReady_eq_true (votes : List Vote) :
    allReady votes = true ↔ ∀ v ∈ votes, v = Vote.ready := by
  induction votes with
  | nil => simp [allReady]
  | cons v vs ih => simp [allReady, ih, decide_eq_true_eq]

/-- **The decision half of law 29, proved**: the coordinator commits exactly when every participant
    voted ready (`txn_coordinator.rs:177-179`) — so the recorded decision is a function of the votes,
    which is what makes it re-derivable on replay. -/
theorem coordinator_decision_committed_iff (votes : List Vote) :
    coordinatorDecision votes = TxnState.committed ↔ ∀ v ∈ votes, v = Vote.ready := by
  rw [coordinatorDecision]
  by_cases h : allReady votes = true
  · rw [if_pos h]
    exact ⟨fun _ => (allReady_eq_true votes).mp h, fun _ => rfl⟩
  · rw [if_neg h]
    constructor
    · intro hc
      exact absurd hc (by simp)
    · intro hall
      exact absurd (h ((allReady_eq_true votes).mpr hall)) (by simp)

/-! ### The run the two laws are about (`run_2pc`)

`coordinatorDecision` decides from a list of votes; the port's `run_2pc`
(`casper/src/txn_coordinator.rs:151-195`) *produces* that list from the phase-one replies and then
applies the one decision to every leg that prepared. This section adds that half, so law 27's narrowed
statement is a theorem about the run rather than a claim about an arbitrary `Run` — the shape the
falsification of `txn_atomic` (`txn_atomic_is_false`) left owed. -/

/-- A participant's phase-one reply as the coordinator reads it (`ShardOutcome`). -/
inductive ShardOutcome where
  | value (p : String)
  | error (msg : String)
deriving DecidableEq

/-- **The reply-to-vote map** (`vote_from_reply`, `txn_coordinator.rs:196-215`): a value is a ready vote
    only for `"ready" | "prepared" | "committed"`; anything else — an abort, an unexpected string, a
    timeout — is an abort vote. The middle two are the *retry* cases its doc comment is about: the
    participant is idempotent under `txn_id`, so a re-run can be answered with an already-terminal
    state, and reading `committed` as "not ready" would abort the other legs and leave one shard
    committed and another aborted *on exactly the path recovery makes reachable*. -/
def voteFromReply : ShardOutcome → Bool
  | .value p => p == "ready" || p == "prepared" || p == "committed"
  | .error _ => false

/-- The decision from the replies: `all_ready` then `"commit"` or `"abort"`
    (`txn_coordinator.rs:177-179`, over the booleans `vote_from_reply` produces). -/
def decisionOf (replies : List ShardOutcome) : String :=
  if replies.all voteFromReply then "commit" else "abort"

/-- **Phase two** (`txn_coordinator.rs:181-194`): the one decision is applied to every leg that voted
    ready; a leg that voted abort gets `"not prepared"` — it never locked resources, so it has nothing
    to commit or compensate. -/
def phaseTwoWith (dec : String) (replies : List ShardOutcome) : List ShardOutcome :=
  replies.map (fun r => if voteFromReply r then .value dec else .error "not prepared")

/-- The cons case as an equation, so the induction can rewrite the folded form rather than unfold the
    `map` (which would put the tail out of reach of the induction hypothesis). -/
theorem phaseTwoWith_cons (dec : String) (r : ShardOutcome) (rs : List ShardOutcome) :
    phaseTwoWith dec (r :: rs)
      = (if voteFromReply r then .value dec else .error "not prepared") :: phaseTwoWith dec rs := rfl

/-- Phase two with the decision `decisionOf` computed. The helper above exists so the "they all get the
    same one" theorem can be stated and proved for *any* decision string, which is what makes the
    induction go through. -/
def runPhaseTwo (replies : List ShardOutcome) : List ShardOutcome :=
  phaseTwoWith (decisionOf replies) replies

/-- **Law 27, narrowed and proved: every leg reaches the one decision or nothing.**
    `txn_atomic_is_false` refuted the old universal form — not every `Run` is uniform, because a leg
    that never prepared has no outcome to be uniform about. What the coordinator actually guarantees is
    this: every leg's phase-two outcome is either the *single* decision computed at `:177-179` or
    `"not prepared"`, so no two prepared legs can disagree. -/
theorem every_leg_reaches_the_one_decision (replies : List ShardOutcome) :
    ∀ o ∈ runPhaseTwo replies, o = .error "not prepared" ∨ o = .value (decisionOf replies) := by
  intro o ho
  simp only [runPhaseTwo, phaseTwoWith, List.mem_map] at ho
  obtain ⟨r, _, rfl⟩ := ho
  by_cases h : voteFromReply r = true
  · exact Or.inr (by simp [h])
  · exact Or.inl (by simp [h])

/-- The helper step: when every reply is a ready vote, the phase-two outcome *is* the decision, for
    every leg — the induction the next theorem needs, general in the decision string. -/
theorem phaseTwoWith_all_prepared (dec : String) (replies : List ShardOutcome)
    (h : replies.all voteFromReply = true) :
    phaseTwoWith dec replies = replies.map (fun _ => .value dec) := by
  induction replies with
  | nil => simp [phaseTwoWith]
  | cons r rs ih =>
      simp only [List.all_cons, Bool.and_eq_true] at h
      obtain ⟨hr, hrs⟩ := h
      rw [phaseTwoWith_cons, List.map_cons]
      simp only [hr, if_true]
      rw [ih hrs]

/-- **…and when every leg prepared, they all commit**: the run's outcomes are `commit` values, which is
    law 27's "commit on all prepared legs" — the other branch is the same statement with the other
    literal, and the legs that did *not* prepare keep their `"not prepared"` outcome, which is what the
    falsification of the old universal form (`txn_atomic_is_false`) was about. -/
theorem all_prepared_legs_commit (replies : List ShardOutcome)
    (h : replies.all voteFromReply = true) :
    runPhaseTwo replies = replies.map (fun _ => .value "commit") := by
  have hdec : decisionOf replies = "commit" := by simp [decisionOf, h]
  rw [runPhaseTwo, hdec]
  exact phaseTwoWith_all_prepared "commit" replies h

/-- **The retry case the port's comment is about, as a theorem**: a leg answering `committed` and a leg
    answering `ready` still commit together, so a re-run cannot flip a transaction whose compensation
    has already run into an abort. -/
theorem an_already_committed_leg_still_commits :
    decisionOf [.value "committed", .value "ready"] = "commit" ∧
    voteFromReply (.value "committed") = true ∧
    voteFromReply (.value "prepared") = true := by
  refine ⟨?_, ?_, ?_⟩ <;> decide

/-- **…and the contrast that makes it a choice rather than a tautology**: an aborted or unexpected
    reply is an abort vote, so it does move the decision to `abort`. -/
theorem an_aborted_reply_moves_the_decision :
    decisionOf [.value "committed", .value "aborted"] = "abort" ∧
    voteFromReply (.value "aborted") = false ∧ voteFromReply (.error "timeout") = false := by
  refine ⟨?_, ?_, ?_⟩ <;> decide


/-! ### Law 29's durability half: the coordinator's vote record

The decision half of law 29 is `coordinator_decision_committed_iff`. The **durability** half is the
property that makes a recorded decision stick: a participant's `committed`/`aborted` is terminal, so a
late vote cannot resurrect it. That is not a hypothetical — AUDIT §15 C1 records the port writing a
terminal record's state from a later vote, *resurrecting* a transaction whose compensation had already
run, and the guard below is the fix (`casper/src/gateway/ledger.rs:158-178`). This section mirrors
`record_vote`, so the row's second half is a theorem rather than a sentence. -/

/-- A terminal state: `committed` or `aborted` (`CoordState::is_terminal`). -/
def TxnState.IsTerminal : TxnState → Prop
  | .prepared => False
  | .committed => True
  | .aborted => True

instance (s : TxnState) : Decidable s.IsTerminal := by
  cases s <;> unfold TxnState.IsTerminal <;> infer_instance

/-- **`record_vote`** (`ledger.rs:158-178`): a terminal record ignores every later vote — that guard is
    the C1 fix — and otherwise the vote is recorded (replacing a repeated shard's) and the state
    recomputed: `aborted` on an abort vote, `committed` when the votes are one per leg and all ready,
    `prepared` in between. -/
def CoordRecord.recordVote (r : CoordRecord) (shard : ShardId) (v : Vote) : CoordRecord :=
  if r.state.IsTerminal then r
  else
    let votes :=
      if r.votes.any (fun p => p.1 = shard) then r.votes.map (fun p => if p.1 = shard then (shard, v) else p)
      else r.votes ++ [(shard, v)]
    if v = Vote.abort then { r with votes := votes, state := TxnState.aborted }
    else if votes.length = r.txn.legs.length && votes.all (fun p => p.2 = Vote.ready) then
      { r with votes := votes, state := TxnState.committed }
    else { r with votes := votes, state := TxnState.prepared }

/-- **A recorded abort is absorbing** — the C1 property, and the Rust test of the same name
    (`an_abort_is_absorbing`). A compensation that has run cannot be undone by a vote that arrives
    later, which is exactly what the unguarded version of this function got wrong. -/
theorem an_abort_is_absorbing (r : CoordRecord) (h : r.state = TxnState.aborted) (shard : ShardId)
    (v : Vote) : r.recordVote shard v = r := by
  unfold CoordRecord.recordVote
  rw [h]
  rfl

/-- **A recorded commit is absorbing**, by the same guard (`a_commit_is_absorbing` in the port's tests). -/
theorem a_commit_is_absorbing (r : CoordRecord) (h : r.state = TxnState.committed) (shard : ShardId)
    (v : Vote) : r.recordVote shard v = r := by
  unfold CoordRecord.recordVote
  rw [h]
  rfl

/-- **…and that guard is the whole fix**: on a non-terminal record an abort vote does abort it, so the
    absorption above is a choice of the function's first line rather than a fact about votes in
    general — the contrast that makes the previous two theorems worth having. -/
theorem an_abort_vote_aborts_a_prepared_record (r : CoordRecord) (shard : ShardId)
    (h : r.state = TxnState.prepared) : (r.recordVote shard Vote.abort).state = TxnState.aborted := by
  unfold CoordRecord.recordVote
  rw [h]
  simp [TxnState.IsTerminal]

/-- **The decision is durable in the sense the law claims**: a record that reaches `committed` keeps it,
    so a prepared participant re-reading the record after a retry recovers the same decision — which is
    what `txnCommit_fixes` (`Ledger`, above) supplies on the participant's side and this on the
    coordinator's. -/
theorem a_committed_record_stays_committed (r : CoordRecord) (h : r.state = TxnState.committed)
    (shard : ShardId) (v : Vote) : (r.recordVote shard v).state = TxnState.committed := by
  rw [a_commit_is_absorbing r h shard v, h]


end Rchain
