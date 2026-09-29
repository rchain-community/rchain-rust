import Rchain.Casper.Liveness
import Rchain.Progress

/-!
# Law 54 — which view a rule reads

Two findings of 2026-09-29 look unrelated and are the same defect: **a rule read a view that was not the
authoritative one**. C170's attestation guard counted liveness off a map that keeps history; C172's
reader asked the in-memory index while the decision it fed asked the persisted store. The fixes differ
(quote the map to the current view; make both readers ask one side, or write both atomically) and the
diagnosis does not, which is why Law 51 puts them on one axis.

## Clause a — a liveness predicate reads the current view

`everSpoke` searches the retention map, so its verdict depends on what has *ever* happened: it is
`Historic` (Law 51's cause), and the two histories below differ in nothing but the past. The port's
guard counted stake with exactly that reader, which is how three equal validators with one silent read
as 300 of 300 — the false supermajority C170 measured, and the Rust test
`a_silent_validators_stale_message_does_not_carry_the_quorum` pins the same numbers (`left: 200,
right: 100`). The predicate that replaces it takes the sender's **entry** — the datum a view supplies —
and tests it against the window.

## Clause b — one view, or both written together

`viewsStep` is C172's shape: the index and the store are updated by **separate** steps, so a reachable
state has one of them holding a key the other does not, and the two readers disagree exactly there. The
fix's shape is `atomicStep`: one step, both views. The theorem is that the atomic order **cannot** split
— which is what makes clause b's falsifier a *rule* rather than a schedule: any step that updates one
side alone reddens it by construction.
-/

namespace Rchain

/-! ### Clause a — the retention map against the view -/

/-- A node's retention map: the port's `latest_msgs`, keyed by sender and **never pruned** — a silent
    sender's last message stays indefinitely (C170). -/
abbrev Retention := List (Nat × Nat)

/-- The **historic** reader: has this sender ever spoken? Its verdict depends on the whole map. -/
def everSpoke (v : Nat) (m : Retention) : Bool := m.any (fun p => p.1 == v)

/-- The **view** predicate: does this sender's latest message fall inside the window? It takes the
    sender's *entry* — the datum a view supplies — rather than searching a map that keeps history, which
    is the port's `liveness::live_weight_set`'s test as a `Bool`. -/
def inWindow (tip w : Nat) (p : Nat × Nat) : Bool := decide (p.2 + w ≥ tip)

/-- **The retention map is historic.** Two maps that agree on the current view — the same last entry —
    and disagree on `everSpoke`; the second history differs from the first in nothing but its past, which
    is the whole of C170. -/
theorem everSpoke_is_historic :
    Historic (fun m : Retention => everSpoke 7 m) := by
  refine ⟨[(1, 5)], [(7, 5), (1, 5)], rfl, ?_⟩
  decide

/-- **On the port's own fixture the two readers disagree about one sender.** Sender 3's latest message is
    7 heights behind the tip: `everSpoke` counts it and the window refuses it. The Rust test that pins the
    same disagreement in stake is `a_silent_validators_stale_message_does_not_carry_the_quorum`, whose
    failure message reads `left: 200, right: 100`. -/
theorem the_retention_reader_counts_a_stale_sender :
    everSpoke 3 [(3, 3), (1, 10)] = true ∧ inWindow 10 5 (3, 3) = false := by
  decide

/-! ### Clause b — the index and the store -/

/-- A node's two views of the same set of keys: the in-memory index and the persisted store. The port
    keeps both (`BlockMetadataStore::dag_state` and the store it is built from), and C172 is that they
    could disagree. -/
structure TwoViews where
  index : List Nat
  store : List Nat

/-- **C172's shape**: the index and the store are updated by **separate** steps, so a state is reachable
    in which one holds a key the other does not. That is the write order the port had — the index first,
    with an `await` between — and the readers that disagreed were `has_all_deps` (the index) and
    `block_summary` (the store). -/
def viewsStep (s s' : TwoViews) : Prop :=
  (∃ k, s'.index = k :: s.index ∧ s'.store = s.store) ∨
    (∃ k, s'.store = k :: s.store ∧ s'.index = s.index)

/-- **The fix's shape**: one step, both views. -/
def atomicStep (s s' : TwoViews) : Prop :=
  ∃ k, s'.index = k :: s.index ∧ s'.store = k :: s.store

/-- **The separate steps can split** — a key the index holds and the store does not is reachable, which
    is the state in which the two readers answer differently (C172's divergence, AUDIT C172). -/
theorem the_separate_steps_can_split :
    ∃ s : TwoViews, Relation.ReflTransGen viewsStep ⟨[], []⟩ s ∧ 7 ∈ s.index ∧ 7 ∉ s.store := by
  refine ⟨⟨[7], []⟩, ?_, by simp, by simp⟩
  exact Relation.ReflTransGen.single (Or.inl ⟨7, rfl, rfl⟩)

/-- **…and the atomic order cannot**: if every step writes both views, no reachable state has them
    disagreeing about any key. This is what makes clause b a statement about the *rule* rather than about
    a schedule — a step that updates one side alone reddens it by construction, which is the falsifier the
    register's row names.

    The initial condition is a hypothesis rather than a fact: a node whose two views **start** out of
    step stays out of step, which is C172's own finding one state earlier — and the reason the port's fix
    both orders the writes and states which side a reader should ask. -/
theorem the_atomic_order_cannot_split (s₀ : TwoViews) (k : Nat)
    (h₀ : k ∈ s₀.index ↔ k ∈ s₀.store) :
    ∀ s, Relation.ReflTransGen atomicStep s₀ s → (k ∈ s.index ↔ k ∈ s.store) := by
  intro s hr
  induction hr with
  | refl => exact h₀
  | tail _ hstep ih =>
    obtain ⟨j, hj, hj'⟩ := hstep
    rw [hj, hj', List.mem_cons, List.mem_cons]
    exact or_congr Iff.rfl ih

end Rchain
