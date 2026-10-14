import Rchain.Progress

/-!
# Law 72 — a completion record brackets every durable effect

**Why this module exists.** The node's self-reset applies **three** durable effects — it rewinds the DAG's
metadata above the anchor, deletes the dropped block bodies, and moves the runtime's durable root to the
anchor's state — and it records only *two* of them under its completion marker. `rewind_and_purge` writes
`ResetMarker{Rewound}` when the first two have returned (`casper/src/engine/reset.rs:81`) and the caller
writes the runtime root *afterwards* (`:261-266`), through `HistoryRepository::reset` →
`roots_repository.validate_and_set_current_root` (`rspace/src/history/history_repository.rs:337`), itself a
durable write. A crash between the two leaves a `Rewound` record claiming "the stores agree" while a third
durable store does not — and the boot guard, reading `Rewound`, re-applies **nothing** (`reset.rs:140-151`).
The port already treats this class as real elsewhere: `reset_volatile`'s doc
(`history_repository.rs:346-358`) exists because a crash between a merge's two commit points "leaves
`CURRENT_ROOT` naming an ancestor … resumes from neither state".

**The law.** A completion record is a **bracket**, not a flag: it may be written only once *every* effect
it brackets has been applied. The model below is the port's own sequence — write the intent, apply the
effects in the port's forced order, record completion — with a boot repair that re-applies the whole set
from any partially-applied state (idempotent, which is why it is a single step). The headline theorem is
that **no reachable state has the record written while an effect is missing**, so the window the port has
today is not a race but a modelling error, and the fix is to move the record's write, not to add a guard.

**The falsifier is the port's current order.** `earlyCommitStep` is the same machine with the completion
recorded once the rewind and the purge have returned, without waiting for the root — the shape the port
has today — and `an_early_commit_reaches_a_record_that_lies` exhibits the reachable state it admits: a
`Rewound` record with the root unmoved. A law whose statement the current code satisfies would be the
register's `vacuous`; this one is red on the code as it stands, which is what makes it a finding.
-/

namespace Rchain

/-- The phase of a reset's completion record. `idle` — no record; `rewinding` — the intent is durable and
    the effects may be partially applied; `rewound` — the completion is recorded. -/
inductive ResetPhase where
  | idle
  | rewinding
  | rewound
deriving DecidableEq, Repr

/-- A reset's durable state: the completion record's phase and the three effects the port applies —
    `rewound` (the DAG's metadata above the anchor is gone), `purged` (the dropped block bodies are
    deleted), `rooted` (the runtime's durable root names the anchor's state). -/
structure ResetState where
  phase : ResetPhase
  rewound : Bool
  purged : Bool
  rooted : Bool
deriving DecidableEq, Repr

/-- The state a node is in before it has ever reset: no record, no effect. -/
def resetInitial : ResetState := ⟨.idle, false, false, false⟩

/-- **The steps of a reset, in the port's order.** The intent is written before any effect (`start`); the
    three effects follow in the order the port forces them — the rewind, then the purge whose input is the
    rewind's output, then the runtime root; the completion is recorded **only after every effect has
    returned** (`commit`); and the boot guard re-applies the whole set from any partially-applied state
    (`repair`), which is why it is one step that sets all three at once. -/
def resetStep (s s' : ResetState) : Prop :=
  (s.phase = .idle ∧ s' = { s with phase := .rewinding })
  ∨ (s.phase = .rewinding ∧ s.rewound = false ∧ s' = { s with rewound := true })
  ∨ (s.phase = .rewinding ∧ s.rewound = true ∧ s.purged = false ∧ s' = { s with purged := true })
  ∨ (s.phase = .rewinding ∧ s.purged = true ∧ s.rooted = false ∧ s' = { s with rooted := true })
  ∨ (s.phase = .rewinding ∧ s.rewound = true ∧ s.purged = true ∧ s.rooted = true
        ∧ s' = { s with phase := .rewound })
  ∨ (s.phase = .rewinding ∧ s' = { s with rewound := true, purged := true, rooted := true })

/-- A step is available while the reset is in progress: from `idle` the intent can be written, from
    `rewinding` either the next effect, the completion, or the repair; from `rewound` nothing — the reset
    is done. So `enabled` is exactly "the record does not yet say `rewound`". -/
def resetEnabled (s : ResetState) : Bool :=
  match s.phase with
  | .idle => true
  | .rewinding => true
  | .rewound => false

theorem resetEnabled_iff_phase (s : ResetState) :
    resetEnabled s = true ↔ s.phase ≠ .rewound := by
  cases hp : s.phase <;> simp [resetEnabled, hp]

/-- **The reset, as a `System`** — the three effects under one completion record. -/
def resetSystem : System where
  State := ResetState
  Step := resetStep
  enabled := resetEnabled
  enabled_iff := by
    intro σ
    constructor
    · intro h
      have hne : σ.phase ≠ .rewound := (resetEnabled_iff_phase σ).mp h
      cases hp : σ.phase
      · exact ⟨_, Or.inl ⟨hp, rfl⟩⟩
      · cases hr : σ.rewound <;> cases hp2 : σ.purged <;> cases hrt : σ.rooted
        · exact ⟨_, Or.inr (Or.inl ⟨hp, hr, rfl⟩)⟩
        · exact ⟨_, Or.inr (Or.inl ⟨hp, hr, rfl⟩)⟩
        · exact ⟨_, Or.inr (Or.inl ⟨hp, hr, rfl⟩)⟩
        · exact ⟨_, Or.inr (Or.inl ⟨hp, hr, rfl⟩)⟩
        · exact ⟨_, Or.inr (Or.inr (Or.inl ⟨hp, hr, hp2, rfl⟩))⟩
        · exact ⟨_, Or.inr (Or.inr (Or.inl ⟨hp, hr, hp2, rfl⟩))⟩
        · exact ⟨_, Or.inr (Or.inr (Or.inr (Or.inl ⟨hp, hp2, hrt, rfl⟩)))⟩
        · exact ⟨_, Or.inr (Or.inr (Or.inr (Or.inr (Or.inl ⟨hp, hr, hp2, hrt, rfl⟩))))⟩
      · exact absurd hp hne
    · rintro ⟨σ', hs⟩
      rcases hs with ⟨h1, _⟩ | ⟨h1, _, _⟩ | ⟨h1, _, _, _⟩ | ⟨h1, _, _, _⟩ | ⟨h1, _, _, _, _⟩ | ⟨h1, _⟩
      all_goals simp [resetEnabled, h1]

/-- The step lemma the law rests on: a step whose *target* has the record written has every effect
    applied. The completion step (`commit`) is the only one that can write `rewound`, and it demands all
    three effects; every other step leaves the phase as it found it. -/
theorem step_target_has_completion (s s' : ResetState) (hs : resetStep s s') (hph : s'.phase = .rewound) :
    s'.rewound = true ∧ s'.purged = true ∧ s'.rooted = true := by
  unfold resetStep at hs
  rcases hs with ⟨h1, h2⟩ | ⟨h1, h2, h3⟩ | ⟨h1, h2, h3, h4⟩ | ⟨h1, h2, h3, h4⟩
    | ⟨h1, h2, h3, h4, h5⟩ | ⟨h1, h2⟩
  · rw [h2] at hph; exact absurd hph (by simp [h1])
  · rw [h3] at hph; exact absurd hph (by simp [h1])
  · rw [h4] at hph; exact absurd hph (by simp [h1])
  · rw [h4] at hph; exact absurd hph (by simp [h1])
  · rw [h5, h2, h3, h4] at hph ⊢; exact ⟨rfl, rfl, rfl⟩
  · rw [h2] at hph; exact absurd hph (by simp [h1])

/-- **The law, in one line.** Every reachable state whose record says `rewound` has **every** effect
    applied — the rewind, the purge, and the runtime root. Proved per step rather than by induction on
    the trace: the only step that writes `rewound` requires all three effects first, and no step ever
    moves an effect back to `false`, so the target of any step satisfies the conclusion outright. -/
theorem a_rewound_record_implies_every_effect_applied
    (σ : ResetState) (hr : resetSystem.Reach resetInitial σ) (hph : σ.phase = .rewound) :
    σ.rewound = true ∧ σ.purged = true ∧ σ.rooted = true := by
  induction hr with
  | refl => exact absurd hph (by simp [resetInitial])
  | tail _ hstep _ => exact step_target_has_completion _ _ hstep hph

/-- **Non-vacuity: a reset can complete.** The correct system does reach a `Rewound` record with all
    three effects — so the law above is a statement about something, not about a state nothing reaches. -/
theorem a_reset_can_complete :
    ∃ σ, resetSystem.Reach resetInitial σ ∧ σ.phase = .rewound
      ∧ σ.rewound = true ∧ σ.purged = true ∧ σ.rooted = true := by
  refine ⟨⟨.rewound, true, true, true⟩, ?_, rfl, rfl, rfl, rfl⟩
  have h1 : resetStep resetInitial ⟨.rewinding, false, false, false⟩ := Or.inl ⟨rfl, rfl⟩
  have h2 : resetStep ⟨.rewinding, false, false, false⟩ ⟨.rewinding, true, false, false⟩ :=
    Or.inr (Or.inl ⟨rfl, rfl, rfl⟩)
  have h3 : resetStep ⟨.rewinding, true, false, false⟩ ⟨.rewinding, true, true, false⟩ :=
    Or.inr (Or.inr (Or.inl ⟨rfl, rfl, rfl, rfl⟩))
  have h4 : resetStep ⟨.rewinding, true, true, false⟩ ⟨.rewinding, true, true, true⟩ :=
    Or.inr (Or.inr (Or.inr (Or.inl ⟨rfl, rfl, rfl, rfl⟩)))
  have h5 : resetStep ⟨.rewinding, true, true, true⟩ ⟨.rewound, true, true, true⟩ :=
    Or.inr (Or.inr (Or.inr (Or.inr (Or.inl ⟨rfl, rfl, rfl, rfl, rfl⟩))))
  exact Relation.ReflTransGen.tail
    (Relation.ReflTransGen.tail
      (Relation.ReflTransGen.tail
        (Relation.ReflTransGen.tail (Relation.ReflTransGen.single h1) h2) h3) h4) h5

/-- **The mutant the port has today**: the completion is recorded once the rewind and the purge have
    returned, without waiting for the runtime's root. Same machine, one disjunct changed. -/
def earlyCommitStep (s s' : ResetState) : Prop :=
  (s.phase = .idle ∧ s' = { s with phase := .rewinding })
  ∨ (s.phase = .rewinding ∧ s.rewound = false ∧ s' = { s with rewound := true })
  ∨ (s.phase = .rewinding ∧ s.rewound = true ∧ s.purged = false ∧ s' = { s with purged := true })
  ∨ (s.phase = .rewinding ∧ s.rewound = true ∧ s.purged = true ∧ s' = { s with phase := .rewound })

/-- **The falsifier, fired.** Under the port's current order a `Rewound` record is reachable with the
    runtime root **unmoved** — the state `resume_after_reset` reads as "the stores agree" and repairs
    nothing. It is `decide`-able and it is red on the code as it stands, which is the register's
    `falsifiable` for this law made a theorem. -/
theorem an_early_commit_reaches_a_record_that_lies :
    ∃ σ, Relation.ReflTransGen earlyCommitStep resetInitial σ
      ∧ σ.phase = .rewound ∧ σ.rooted = false := by
  refine ⟨⟨.rewound, true, true, false⟩, ?_, rfl, rfl⟩
  have h1 : earlyCommitStep resetInitial ⟨.rewinding, false, false, false⟩ := Or.inl ⟨rfl, rfl⟩
  have h2 : earlyCommitStep ⟨.rewinding, false, false, false⟩ ⟨.rewinding, true, false, false⟩ :=
    Or.inr (Or.inl ⟨rfl, rfl, rfl⟩)
  have h3 : earlyCommitStep ⟨.rewinding, true, false, false⟩ ⟨.rewinding, true, true, false⟩ :=
    Or.inr (Or.inr (Or.inl ⟨rfl, rfl, rfl, rfl⟩))
  have h4 : earlyCommitStep ⟨.rewinding, true, true, false⟩ ⟨.rewound, true, true, false⟩ :=
    Or.inr (Or.inr (Or.inr ⟨rfl, rfl, rfl, rfl⟩))
  exact Relation.ReflTransGen.tail
    (Relation.ReflTransGen.tail
      (Relation.ReflTransGen.tail (Relation.ReflTransGen.single h1) h2) h3) h4

end Rchain
