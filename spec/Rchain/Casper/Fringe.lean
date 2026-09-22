import Rchain.Casper.Stake
import Rchain.Cmp

/-!
# Laws 14b & 15 — the fringe, its antichain and its monotonicity

The finalized fringe is an antichain of one message per bonded validator (Law 14b); it is monotone by
height, and each message's seen set is monotone — no regression (Law 15). The Scala oracle is
`block-storage/dag/Finalizer.scala:76,133` + `MessageMapSyntax.scala:33` + `casper/Validate.scala:285`;
the Rust realization is `block-storage::dag::{finalizer,message_state}` (`Message`, `BlockHeight`/`SeqNum`).

## Three axioms that were false of the value they quantified over

`fringe_antichain`, `fringe_monotone` and `seen_monotone` each held of a **bare** `Fringe` or `Message`,
and those are freely constructed — so each was refuted by values one can write down, and the refutations
are proved in this file (`fringe_antichain_is_false`, `fringe_monotone_is_false`,
`seen_monotone_is_false`). A false axiom is worse than an owed proof, because anything follows from it.

What each actually needs is the **derivation**, not a hypothesis on the value: the fringe the port
finalizes comes from `calculate_finalization`, which advances only when the support gate holds and only
to a strictly new layer (`block-storage/src/dag/finalizer.rs:196-215`), and a message's `seen` set is
*constructed* — the union of its justifications' seen sets plus its own id
(`block-storage/src/dag/message_state.rs:54-59`). That construction is modelled below (`seenOf`), so the
half of Law 15 that follows from it is a theorem; the transitive closure it induces over the DAG — which
is what the finalizer leans on — is owed to a model of the DAG's derivation, which this layer does not
have.
-/

namespace Rchain

/-- A DAG message: id, block height, sender, sequence number, justification (parent) ids, and the seen
    set (the port's `Message`; `seen` is its cache of seen message ids,
    `block-storage/src/dag/finalizer.rs:23-33`). -/
structure Message where
  id : Nat
  height : Nat
  sender : Nat
  seqNum : Nat
  parents : List Nat
  seen : List Nat
  deriving DecidableEq, Repr

/-- A fringe: a set of messages (one per bonded validator). -/
structure Fringe where
  messages : List Message
  deriving DecidableEq, Repr

/-! ### The three refutations -/

/-- **The axiom that stood here was false.** Two messages from one sender with different ids: the claim
    "same sender ⇒ same id" is refuted by the value itself, with no hypothesis to appeal to. What the
    law needs is the derivation — a fringe advanced by `calculate_finalization` holds one message per
    bonded sender (`finalizer.rs:196-215`) — which is owed. -/
theorem fringe_antichain_is_false :
    ¬ ∀ f : Fringe, ∀ m ∈ f.messages, ∀ n ∈ f.messages, m.sender = n.sender → m.id = n.id := by
  intro h
  let f : Fringe := ⟨[⟨0, 0, 7, 0, [], []⟩, ⟨1, 0, 7, 0, [], []⟩]⟩
  exact absurd (h f ⟨0, 0, 7, 0, [], []⟩ (by simp [f]) ⟨1, 0, 7, 0, [], []⟩ (by simp [f]) rfl)
    (by decide)

/-- **The axiom that stood here was false.** Two fringes that *overlap* in height — one at 5 and 1, the
    other at 3: the first arm fails (`5 ≤ 3`) and the second too (`3 ≤ 1`). Height monotonicity relates
    *successive* fringes of one validator, not any two fringes. -/
theorem fringe_monotone_is_false :
    ¬ ∀ f g : Fringe, (∀ m ∈ f.messages, ∀ n ∈ g.messages, m.height ≤ n.height) ∨
      (∀ n ∈ g.messages, ∀ m ∈ f.messages, n.height ≤ m.height) := by
  intro h
  let f : Fringe := ⟨[⟨0, 5, 0, 0, [], []⟩, ⟨2, 1, 0, 0, [], []⟩]⟩
  let g : Fringe := ⟨[⟨1, 3, 1, 0, [], []⟩]⟩
  rcases h f g with hc | hc
  · exact absurd (hc ⟨0, 5, 0, 0, [], []⟩ (by simp [f]) ⟨1, 3, 1, 0, [], []⟩ (by simp [g]))
      (by decide)
  · exact absurd (hc ⟨1, 3, 1, 0, [], []⟩ (by simp [g]) ⟨2, 1, 0, 0, [], []⟩ (by simp [f]))
      (by decide)

/-- **The axiom that stood here was false.** Two unrelated messages: `b` sees `a` (`1 ∈ [1]`) and `a`
    sees `2`, but `b` does not see `2`. The claim is about *derived* messages, and the port's derivation
    is `seenOf` below. -/
theorem seen_monotone_is_false :
    ¬ ∀ a b : Message, a.id ∈ b.seen → ∀ x, x ∈ a.seen → x ∈ b.seen := by
  intro h
  exact absurd (h ⟨1, 0, 0, 0, [], [2]⟩ ⟨0, 0, 0, 0, [], [1]⟩ (by decide) 2 (by decide)) (by decide)

/-! ### What the port actually constructs -/

/-- The seen set the port **builds** for a new message: the union of its justifications' seen sets, plus
    the message's own id (`block-storage/src/dag/message_state.rs:54-59`) — so "no regression" is a
    property of the construction, and this is its half that follows directly: a message sees whatever its
    justifications saw. -/
def seenOf (js : List Message) (id : Nat) : List Nat := (js.map (·.seen)).join ++ [id]

/-- **The derivation half of Law 15** — a message built from `js` sees everything each justification
    saw, which is what the port's `new_seen` computes. -/
theorem seenOf_contains_justifications (js : List Message) (a : Message) (ha : a ∈ js) (x : Nat)
    (hx : x ∈ a.seen) : x ∈ seenOf js a.id :=
  List.mem_append_left _
    (List.mem_join.mpr ⟨a.seen, List.mem_map.mpr ⟨a, ha, rfl⟩, hx⟩)

/-- A message sees itself — the other half of the port's `new_seen.insert(id)`. -/
theorem mem_seenOf_self (js : List Message) (id : Nat) : id ∈ seenOf js id :=
  List.mem_append_right _ (by simp)

end Rchain
