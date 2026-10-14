import Rchain.Progress

/-!
# Law 75 — a dependent action is taken only on a committed record

**Why this module exists.** The refusal budget's increment and the reset request it can raise are not
ordered against each other. `revalidated_record` builds a `BlockMetadata` that carries the incremented
`restore_attempts`, and `note_unrestorable_if_exhausted` — which raises the reset request — runs on that
**uncommitted** value: the call sites at `casper/src/multi_parent_casper.rs:796` and `:1202` precede the
`update_metadata` that persists the record (`:797`, `:1214`). If the write then fails, the node has
already asked for a reset on a record whose stored count is one lower, and the request's premise is not
durable.

**The law.** An action that *depends on* a committed record is taken only after that record is committed:
no reachable state has the dependent action taken while the record it rests on is uncommitted. The model
below is minimal — a record's commit and the action it gates — because that is the whole content.

**The falsifier is the port's order.** `raiseFirstStep` is the same machine with the action taken before
the commit — the shape the port has today — and
`a_raised_request_on_an_uncommitted_record_is_reachable` exhibits the state it admits.

The read-modify-write half of the same node (F-U14-01, the attempt count's read and write under two
separate locks) is **the shape law 73 already states** — a two-phase operation on a cell must be one
step — and it is not modelled twice.
-/

namespace Rchain

/-- A record's commit and the action that depends on it: `committed` — the incremented record is in the
    store; `raised` — the reset request it gates has been raised. -/
structure AttemptState where
  committed : Bool
  raised : Bool
deriving DecidableEq, Repr

/-- The state before the record is committed and before the action. -/
def attemptFresh : AttemptState := ⟨false, false⟩

/-- **The steps, in the order that keeps the request's premise durable.** Commit the record first; then,
    and only then, take the action it gates. -/
def attemptStep (s s' : AttemptState) : Prop :=
  (s.committed = false ∧ s' = { s with committed := true })
  ∨ (s.committed = true ∧ s.raised = false ∧ s' = { s with raised := true })

/-- A step is available until both the commit and the action have happened. -/
def attemptEnabled (s : AttemptState) : Bool := !s.committed || !s.raised

/-- **The record and its dependent action, as a `System`** — commit, then act. -/
def attemptSystem : System where
  State := AttemptState
  Step := attemptStep
  enabled := attemptEnabled
  enabled_iff := by
    intro σ
    obtain ⟨c, r⟩ := σ
    constructor
    · intro h
      cases c <;> cases r
      · exact ⟨⟨true, false⟩, Or.inl ⟨rfl, rfl⟩⟩
      · exact ⟨⟨true, true⟩, Or.inl ⟨rfl, rfl⟩⟩
      · exact ⟨⟨true, true⟩, Or.inr ⟨rfl, rfl, rfl⟩⟩
      · simp [attemptEnabled] at h
    · rintro ⟨σ', hs⟩
      unfold attemptStep at hs
      rcases hs with ⟨h1, _⟩ | ⟨h1, h2, _⟩
      · have hc : c = false := h1
        simp [attemptEnabled, hc]
      · have hc : c = true := h1
        have hr : r = false := h2
        simp [attemptEnabled, hc, hr]

/-- The step lemma: the target of a step that has the action taken also has the record committed. -/
theorem attempt_target_commits_before_acting (s s' : AttemptState) (hs : attemptStep s s')
    (ha : s'.raised = true) : s'.committed = true := by
  unfold attemptStep at hs
  rcases hs with ⟨_, h2⟩ | ⟨h1, _, h2⟩
  · rw [h2]
  · rw [h2]; exact h1

/-- **The law, in one line.** No reachable state has the dependent action taken while the record it rests
    on is uncommitted: the request's premise is always durable. -/
theorem a_dependent_action_is_taken_only_on_a_committed_record (σ : AttemptState)
    (hr : attemptSystem.Reach attemptFresh σ) (ha : σ.raised = true) : σ.committed = true := by
  induction hr with
  | refl => exact absurd ha (by simp [attemptFresh])
  | tail _ hstep _ => exact attempt_target_commits_before_acting _ _ hstep ha

/-- **Non-vacuity: the record can be committed and the action taken on it.** -/
theorem a_committed_record_can_gate_an_action :
    ∃ σ, attemptSystem.Reach attemptFresh σ ∧ σ.committed = true ∧ σ.raised = true :=
  ⟨⟨true, true⟩,
    Relation.ReflTransGen.tail
      (Relation.ReflTransGen.single (Or.inl ⟨rfl, rfl⟩)) (Or.inr ⟨rfl, rfl, rfl⟩),
    rfl, rfl⟩

/-- **The mutant the port has today**: the action is taken without requiring the commit. -/
def raiseFirstStep (s s' : AttemptState) : Prop :=
  (s.committed = false ∧ s' = { s with committed := true })
  ∨ (s.raised = false ∧ s' = { s with raised := true })

/-- **The falsifier, fired.** Under the port's order a state is reachable with the request raised and the
    record it rests on uncommitted — the request's premise not durable, exactly as a failed
    `update_metadata` leaves it. -/
theorem a_raised_request_on_an_uncommitted_record_is_reachable :
    ∃ σ, Relation.ReflTransGen raiseFirstStep attemptFresh σ
      ∧ σ.raised = true ∧ σ.committed = false :=
  ⟨⟨false, true⟩, Relation.ReflTransGen.single (Or.inr ⟨rfl, rfl⟩), rfl, rfl⟩

end Rchain
