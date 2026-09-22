/-!
# Law 19 — `Blake2b512Random` (a splittable merge that is **not** commutative)

A split random state can be split into children and merged back into one state. The Scala oracle is
`crypto/.../Blake2b512Random`; the Rust realization is `crypto::hash::blake2b512_random`. The primitive
is **axiomatized by design** (a cryptographic hash), not proven.

## What the code is, and what this file therefore does *not* claim

The Rust merge is `fn merge(children: &[Self]) -> Self`
(`crypto/src/hash/blake2b512_random.rs:191`) — **n-ary**, and **order-sensitive**: the position of each
child is part of the hashed input. The Rust pins that with its own test `merge_is_order_sensitive`
(`:548`):

    let merged12 = Blake2b512Random::merge(&[rnd1.clone(), rnd2.clone()]);
    let merged21 = Blake2b512Random::merge(&[rnd2, rnd1]);
    assert_ne!(merged12, merged21);

This module used to declare a *binary* `mergeRandom : Random → Random → Random` with an `assoc` and a
`comm` axiom. **The commutativity axiom was false of the code.** An axiom that is false is worse than
one that is owed, because anything follows from it — the lesson of AUDIT C26 (a law-5 axiom the
implementations denied) and C40 (a law-38 axiom refuted by `chan = nilPar`). Both axioms were used by
nothing, so nothing had to be re-examined once they went.

Associativity was not false of the model so much as *not a property the code has*: `merge [merge [a,b],
c]` hashes two children, one of them a hash, while `merge [a,b,c]` hashes three, and the two differ. So
there is no algebraic law of this merge to state, and the honest model is what remains below: a function
of the **ordered** list of children.

The Rust also refuses a merge of fewer than two children (`merge_with_empty_children_throws` at `:496`,
`merge_with_single_child_throws` at `:502`). Lean functions are total, so that precondition is carried
by the caller wherever the merge is used — a hypothesis, not a fact this file can state.
-/

namespace Rchain

/-- A split random generator state. -/
structure Random where
  state : Nat

/-- Merge the children of a split into one state — `Blake2b512Random::merge(children: &[Self])`
(`crypto/src/hash/blake2b512_random.rs:191`). N-ary and order-sensitive; see the module doc for why
neither commutativity nor associativity is claimed here. -/
axiom mergeRandom : List Random → Random

end Rchain
