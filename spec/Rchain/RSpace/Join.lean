import Rchain.Cmp
import Rchain.Crypto.Spec

/-!
# Law 7 — join commutativity

A join (multi-channel consume) is keyed by the hash of its channels taken **in sorted order**, so the key
is invariant under channel permutation. The Scala oracle is `StableHashProvider.scala:18-22`; the Rust
realization is `rspace::hashing::StableHashProvider` — `hash_seq` hashes each channel and **sorts** the
hashes (`rspace/src/hashing/stable_hash_provider.rs:22-34`), `hash_hashes` sorts again and hashes the
concatenation (`:36-46`), and `hash_channels` composes them (`:105-116` pins both, and the join key used
throughout the RSpace is `hash_hashes` over the sorted channel hashes).

## Why this file no longer carries two axioms

It declared `joinKey : List Channel → Nat` and `joinKey_perm` as **axioms** — an opaque function and a
claim about it. But the Rust's join key is not opaque: it *is* hash-of-sorted-hashes, so permutation
invariance is a corollary of Law 1's canonicalization (`Cmp.sortList_perm`), not an independent
postulate. `hashHashes` below is the one primitive that stays axiomatized, and it belongs to Law 19's
class: a cryptographic hash, not a claim about channels.
-/

namespace Rchain

/-- A channel, identified by its Blake2b256 key (the Rust's `hash_channel`). -/
structure Channel where
  key : Hash

/-- One hash over a *sorted* sequence of channel hashes — the Rust's `hash_hashes`
    (`rspace/src/hashing/stable_hash_provider.rs:36-46`), a Blake2b256 over the concatenation. -/
axiom hashHashes : List Hash → Hash

/-- The join key, as the Rust computes it: hash each channel, **sort the hashes**, then hash the
    sorted list (`hash_seq` then `hash_hashes`; `stable_hash_provider.rs:22-46`). Sorting is what makes
    the key canonical — the same sort-then-hash shape as Law 1.

`noncomputable` because it calls `hashHashes`, an axiom — a modelled primitive has no compiled code. -/
noncomputable def joinKey (cs : List Channel) : Hash :=
  hashHashes (Comparator.sortList (Comparator.listComparator (Comparator.linearOrderComparator Byte))
    (cs.map (·.key)))

/-- **Law 7** — the join key is invariant under permutation of the channels, *because the code sorts
    them first*. Law 1's canonicalization applied to a join key: it follows from `sortList_perm`
    (`Rchain/Cmp.lean:228`) and nothing else, which is why it is a theorem here rather than the axiom it
    used to be. -/
theorem joinKey_perm (cs ds : List Channel) (h : List.Perm cs ds) : joinKey cs = joinKey ds := by
  simp only [joinKey]
  congr 1
  exact Comparator.sortList_perm (Comparator.listComparator (Comparator.linearOrderComparator Byte))
    (List.Perm.map (fun c => c.key) h)

end Rchain
