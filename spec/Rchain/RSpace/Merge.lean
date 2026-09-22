import Rchain.Crypto.Spec
import Rchain.Effect

/-!
# Law 9 — merge is a monoid; non-conflicting logs commute

State-channel changes merge associatively, and merging two non-conflicting change logs is commutative.
The Scala oracle is `merger/{StateChange,ChannelChange,EventLogMergingLogic}.scala`; the Rust realization
is `rspace/src/merger/state_change.rs` (`StateChange::combine`) + `rspace/src/merger/channel_change.rs`
(`ChannelChange::combine`) + `rspace/src/merger/event_log_merging_logic.rs` (`are_conflicting`).

## What changed, and why

The file declared `mergeChanges : StateChange → StateChange → StateChange` as an axiom over
`StateChange = { id : Nat }`, `NonConflicting` as an axiom (an undefined relation), and then
associativity and commutativity as axioms *about those*. Nothing could be checked, and a claim about an
undefined relation is a claim about nothing: the commutativity axiom was true of any `NonConflicting`
one cared to imagine.

The Rust gives the definition, so the model takes it: `combine` concatenates each channel's added and
removed lists, and **overwrites** the join map (`state_change.rs:186-189` — `joins.insert(k, v)` in a
loop over the right operand, which is the Scala's `x.map ++ y.map`, `StateChange.scala:152`, since
`immutable.Map.++` is right-biased). That makes associativity **structural and proven**. Commutativity is
*not* a property of `combine` on the nose — concatenation is order-sensitive, and the Rust's own test
compares sorted multisets rather than lists (`state_change.rs:224-238`) — so the theorem below proves the
version the engine relies on: **disjoint channels do not interact**, which makes the merge
order-independent in practice.

`NonConflicting` is that sufficient condition, and **not** the Rust's `are_conflicting` read negatively:
that predicate is over two `EventLogIndex`es and its three checks are not channel-disjointness (see its
doc below). The relation is named for exactly what it gives — the condition under which the merge
commutes.
-/

namespace Rchain

/-- One channel's change of one kind: the data added (or removed) on a channel, in log order. The Rust
    concatenates these lists (`ChannelChange::combine`,
    `rspace/src/merger/channel_change.rs:20-27`) and the engine reads the result as a multiset. -/
abbrev ChannelChange := List Hash

/-- A state change: for each channel, what was added and removed, plus the join map the merge writes
    (`StateChange`, `rspace/src/merger/state_change.rs:40-46`). -/
structure StateChange where
  added : Chan → ChannelChange
  removed : Chan → ChannelChange
  joins : Chan → Option (List Hash)

/-- Merge two state changes as the Rust does: concatenate each channel's added and removed lists
    (`combine_channel_change_map`, `state_change.rs:18-38`), and let the **right** operand win a
    contested join (`StateChange::combine`, `state_change.rs:185-195` — pinned by the code's own
    `combine_has_an_identity_and_a_right_biased_join_map`, `:502-544`). -/
def mergeChanges (x y : StateChange) : StateChange where
  added := fun c => x.added c ++ y.added c
  removed := fun c => x.removed c ++ y.removed c
  joins := fun c =>
    match x.joins c, y.joins c with
    | some _, some b => some b
    | some a, none => some a
    | none, some b => some b
    | none, none => none

/-- The channels a change touches. -/
def StateChange.touches (x : StateChange) (c : Chan) : Prop :=
  x.added c ≠ [] ∨ x.removed c ≠ [] ∨ x.joins c ≠ none

/-- Two changes are **non-conflicting** when they touch disjoint sets of channels — a *sufficient
    condition for the merge to commute*, which is what the engine needs of a merged state change, and the
    only thing this relation claims.

    It is **not** the Rust's `are_conflicting` read negatively, and saying so would be wrong:
    `are_conflicting` (`rspace/src/merger/event_log_merging_logic.rs:100-103`) is a predicate over two
    `EventLogIndex`es — event logs, not state changes — and its three checks are not channel-disjointness:
    a non-persistent produce/consume destroyed in both branches (`:107-125`), a **potential COMM** between
    one branch's creates and the other's (`:127-147`, a *shared-channel* interaction), and a produce
    touching a base join (`:149-156`), which is a property of the pre-state that no `StateChange`
    records. The predicate the merge actually branches on is broader still: it also tests shared deploy
    ids (`casper/src/merging.rs:177-181`).

    This was an `axiom` before, an undefined relation; it is a definition now, which is what lets the
    commutativity below be proved rather than postulated. -/
def NonConflicting (x y : StateChange) : Prop := ∀ c, ¬ (x.touches c ∧ y.touches c)

/-- **Sufficient, and not necessary** — so the register cannot quietly upgrade this relation to "the
    port's conflict predicate". Two *different* changes that touch the same channel commute anyway: the
    law needs only that *if* they are disjoint *then* the merge commutes, never the converse. -/
theorem nonConflicting_not_necessary :
    ∃ x y : StateChange, ¬ NonConflicting x y ∧ mergeChanges x y = mergeChanges y x := by
  let k : ChannelChange := [[0]]
  let x : StateChange := ⟨fun _ => k, fun _ => [], fun _ => none⟩
  let y : StateChange := ⟨fun _ => [], fun _ => [], fun _ => some []⟩
  refine ⟨x, y, fun h => h 0 ⟨Or.inl ?_, Or.inr ?_⟩, ?_⟩
  · simp [StateChange.touches, x, k]
  · simp [StateChange.touches, y]
  · simp only [mergeChanges]
    congr 1 <;> funext c <;> simp [x, y]

/-- **The join map is right-biased**, so the fold order decides a contested join
    (`state_change.rs:186-189`, pinned by `combine_has_an_identity_and_a_right_biased_join_map`
    `:502-544` — "the later change's join body wins"; the fold that consumes it is
    `casper/src/merging.rs:752-755`). This is why commutativity needs the disjointness hypothesis and is
    not a property of `mergeChanges` on the nose. -/
theorem join_last_wins (x y : StateChange) (c : Chan) (a b : ChannelChange)
    (hx : x.joins c = some a) (hy : y.joins c = some b) : (mergeChanges x y).joins c = some b := by
  simp [mergeChanges, hx, hy]

/-- A change that does not touch a channel is empty on it — the bridge between the conflict predicate and
    the merge's arithmetic. -/
theorem not_touches_iff (x : StateChange) (c : Chan) :
    ¬ x.touches c ↔ x.added c = [] ∧ x.removed c = [] ∧ x.joins c = none := by
  simp [StateChange.touches, not_or]

/-- **Law 9 (associativity)** — merge is associative. Structural, and proven: concatenation of lists and
    a right-biased overwrite of the join map are associative, which is the fact the fold
    (`casper/src/merging.rs:753-755`) relies on.

    **It is not tested on the Rust side**, and the test that looks like it is does the opposite:
    `state_change.rs:203-238` is named `combine_is_associative`, but its own comment says "the monoid law
    tested here is empty-is-identity" and its assertions are the identity and a **sorted-multiset**
    agreement between the two orders. The inner monoid's associativity *is* tested
    (`channel_change.rs:35-50`), as is the identity and the right-biased join
    (`state_change.rs:502-544`); this one is owed — recorded in Law 9's row and AUDIT §17. -/
theorem mergeChanges_assoc (a b c : StateChange) :
    mergeChanges (mergeChanges a b) c = mergeChanges a (mergeChanges b c) := by
  unfold mergeChanges
  congr 1
  · funext ch; simp [List.append_assoc]
  · funext ch; simp [List.append_assoc]
  · funext ch
    cases ha : a.joins ch <;> cases hb : b.joins ch <;> cases hc : c.joins ch <;>
      simp [ha, hb, hc, List.append_assoc]

/-- **Law 9 (commutativity under non-conflict)** — merging non-conflicting changes commutes. Per channel
    at most one side touches anything, and an untouched side contributes `[]` to the concatenation and
    `none` to the join map, so the two orders agree *on every channel* — which is the order-independence
    the engine's merges need, and the reason the law is stated over disjoint channels rather than over
    list order or a contested join (neither of which `combine` preserves: the lists concatenate in
    operand order and the join map is right-biased, `join_last_wins`). -/
theorem mergeChanges_comm (x y : StateChange) (h : NonConflicting x y) :
    mergeChanges x y = mergeChanges y x := by
  classical
  have hdisj : ∀ c, x.touches c → ¬ y.touches c := fun c hx hy => h c ⟨hx, hy⟩
  unfold mergeChanges
  congr 1
  · funext ch
    by_cases hx : x.touches ch
    · obtain ⟨ha, _, _⟩ := (not_touches_iff y ch).mp (hdisj ch hx)
      simp [ha]
    · obtain ⟨ha, _, _⟩ := (not_touches_iff x ch).mp hx
      simp [ha]
  · funext ch
    by_cases hx : x.touches ch
    · obtain ⟨_, hr, _⟩ := (not_touches_iff y ch).mp (hdisj ch hx)
      simp [hr]
    · obtain ⟨_, hr, _⟩ := (not_touches_iff x ch).mp hx
      simp [hr]
  · funext ch
    by_cases hx : x.touches ch
    · obtain ⟨_, _, hj⟩ := (not_touches_iff y ch).mp (hdisj ch hx)
      cases hxj : x.joins ch <;> simp [hj, hxj]
    · obtain ⟨_, _, hj⟩ := (not_touches_iff x ch).mp hx
      cases hyj : y.joins ch <;> simp [hj, hyj]

end Rchain
