import Rchain.Progress

/-!
# Law 73 — a raised request is not spent until the act commits

**Why this module exists.** The self-reset's request is a one-shot cell: the detector records it and the
driver takes it. The port takes it **first** and acts **second** — `act_on_reset_request` calls
`take_reset_request()` as its opening statement (`casper/src/engine/reset.rs:215`), and only then runs
`perform_reset`, which can fail before it has touched a store (the anchor's body may be absent from the
block store, `:246-258`). A failure there spends the request with nothing done: the cell is empty, the
divergence is untouched, and no rule re-raises — the node's only way back is a restart, and it is
re-derivable then only because the *detector's* count (`restore_attempts`) is persisted, not because the
request survived.

**The law.** The act is one step: the request is taken **and** the effects applied together. That is the
whole content — a take that is not also the act is a step that loses a request — and it is exactly what
the port's two statements do not do. Stated as a safety property over the reachable set: no reachable
state has the request spent with nothing applied, so a raised request is *always* still actionable.

**The falsifier is the port's split.** `takeThenActStep` is the same machine with the take and the act
separated — the shape the port has today — and `a_split_take_reaches_a_state_that_spent_the_request`
exhibits the state it admits: the request gone, nothing applied. The correct machine cannot reach it.

This is the first half of HAZOP node U11 (docs/src/spec/failure-hazop.md); the shard-lane half
(F-U11-02) is a Rust partition and is not modelled here.
-/

namespace Rchain

/-- The request cell and the effects it asks for: `pending` — the request is in the cell; `applied` —
    the reset's durable effects have been applied. -/
structure RequestState where
  pending : Bool
  applied : Bool
deriving DecidableEq, Repr

/-- The state a node is in once the detector has raised a request and nothing has been done about it. -/
def requestRaised : RequestState := ⟨true, false⟩

/-- **The act as one step**: the request is taken and the effects applied together. There is no other
    step, which is what makes the request unspendable without the act. -/
def spendRequestStep (s s' : RequestState) : Prop :=
  s.pending = true ∧ s.applied = false ∧ s' = ⟨false, true⟩

/-- A step is available exactly while a request is pending and nothing has been applied. -/
def spendEnabled (s : RequestState) : Bool := s.pending && !s.applied

/-- **The request's lifecycle, as a `System`** — one step, the act. -/
def spendSystem : System where
  State := RequestState
  Step := spendRequestStep
  enabled := spendEnabled
  enabled_iff := by
    intro σ
    obtain ⟨p, a⟩ := σ
    constructor
    · intro h
      cases p <;> cases a
      · simp [spendEnabled] at h
      · simp [spendEnabled] at h
      · exact ⟨⟨false, true⟩, ⟨rfl, rfl, rfl⟩⟩
      · simp [spendEnabled] at h
    · rintro ⟨σ', hs⟩
      unfold spendRequestStep at hs
      rcases hs with ⟨h1, h2, _⟩
      have hp : p = true := h1
      have ha : a = false := h2
      simp [spendEnabled, hp, ha]

/-- The step lemma: the act's target has the effects applied. -/
theorem spend_target_has_applied (s s' : RequestState) (hs : spendRequestStep s s') :
    s'.pending = true ∨ s'.applied = true := by
  unfold spendRequestStep at hs
  rcases hs with ⟨_, _, h3⟩
  rw [h3]
  exact Or.inr rfl

/-- **The law, in one line.** From a raised request, no reachable state has the request spent with
    nothing applied: the request is never cleared except by the act that applies the effects. -/
theorem a_request_is_not_spent_without_the_act (σ : RequestState)
    (hr : spendSystem.Reach requestRaised σ) : σ.pending = true ∨ σ.applied = true := by
  induction hr with
  | refl => exact Or.inl rfl
  | tail _ hstep _ => exact spend_target_has_applied _ _ hstep

/-- **Non-vacuity: a raised request can be acted on.** -/
theorem a_raised_request_can_be_acted_on :
    ∃ σ, spendSystem.Reach requestRaised σ ∧ σ.applied = true :=
  ⟨⟨false, true⟩, Relation.ReflTransGen.single ⟨rfl, rfl, rfl⟩, rfl⟩

/-- **The mutant the port has today**: the take and the act are two steps, so a failure between them
    leaves the cell empty and the effects unapplied. -/
def takeThenActStep (s s' : RequestState) : Prop :=
  (s.pending = true ∧ s.applied = false ∧ s' = ⟨false, false⟩)
  ∨ (s.pending = false ∧ s.applied = false ∧ s' = ⟨false, true⟩)

/-- **The falsifier, fired.** Under the port's split a state is reachable whose cell is empty and whose
    effects are unapplied — the request spent, nothing done, and nothing to re-raise it in-process. -/
theorem a_split_take_reaches_a_state_that_spent_the_request :
    ∃ σ, Relation.ReflTransGen takeThenActStep requestRaised σ
      ∧ σ.pending = false ∧ σ.applied = false :=
  ⟨⟨false, false⟩, Relation.ReflTransGen.single (Or.inl ⟨rfl, rfl, rfl⟩), rfl, rfl⟩

end Rchain
