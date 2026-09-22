import Rchain.Crypto.Spec

/-!
# Law 10 — Merkle determinism

The history is a content-addressed radix trie. The Scala oracle is `history/RadixTree.scala:50-68`; the
Rust realization is `rspace/src/history/radix_tree.rs`: a node is `[Item; 256]` (`:15-35`), `encode`
writes its items in slot order with prefix lengths (`:48-77`), `hash_node` is Blake2b256 over that
encoding (`:139-142`), `empty_root_hash` is the hash of an all-empty node (`:38-45`), and the store
**refuses** an actual collision at both layers (`save_node`'s assert at `:208-223`, `commit`'s error at
`:226-258`).

## What changed, and why

The file declared `trieRoot : NodeHash` — a *constant*, with no arguments, which is why nothing could be
proved about it — plus `trie_collision_free` and `trie_empty_root`. Those two are statements about the
*hash function* wearing a trie's name: "equal hashes give equal contents" is Law 19's
`blake2b256_collision_free` applied to the node encoding. The root is now a **definition** over the node
type the code has, and the collision statements are **theorems** that compose the hash's injectivity with
the encoding's canonicity.
-/

namespace Rchain

/-- A trie slot: empty, a leaf (prefix and value), or a pointer to a child by hash (`Item`,
    `rspace/src/history/radix_tree.rs:19-30`). The field is `pref` rather than `prefix`, which Lean
    reserves as a notation keyword. -/
inductive Item where
  | empty
  | leaf (pref : List Byte) (value : Hash)
  | node (pref : List Byte) (ptr : Hash)

/-- A trie node: 256 slots (`Node = [Item; NUM_ITEMS]`, `NUM_ITEMS = 256`, `radix_tree.rs:15-35`). The
    model keeps the list; the width is an invariant of the operations that build nodes rather than a
    type-level one. -/
abbrev Node := List Item

/-- The canonical encoding of a node: its slots in order, each prefixed and length-tagged (`encode`,
    `radix_tree.rs:48-77`). The model's encoding is abstract — the serializer is not the law — so its
    canonicity is stated below rather than implemented. -/
axiom encodeNode : Node → Msg

/-- The encoding is **canonical**: two nodes with the same encoding are the same node. This is the
    serializer's job, not the hash's — `encode` writes each item's slot index, prefix length and bytes
    in order, which is decodable — and it is stated here because the model's `encodeNode` is abstract.
    A hash cannot give canonicity to a serializer that had none. -/
axiom encodeNode_injective {a b : Node} : encodeNode a = encodeNode b → a = b

/-- The node's hash: Blake2b256 over the canonical encoding (`hash_node`, `radix_tree.rs:139-142`).

`noncomputable` because it calls `blake2b256`, an axiom — a modelled primitive has no compiled code, and
this is a definition in the specification rather than something the node executes. -/
noncomputable def nodeHash (n : Node) : Hash := blake2b256 (encodeNode n)

/-- The empty node: 256 empty slots (`empty_node`, `radix_tree.rs:38-40`). -/
def emptyNode : Node := List.replicate 256 Item.empty

/-- The empty history's root: the hash of the empty node (`empty_root_hash`, `radix_tree.rs:43-45`). -/
noncomputable def emptyRoot : Hash := nodeHash emptyNode

/-- **Law 10 (content addressing)** — the root determines the node. Proven, not postulated: it composes
    Law 19's `blake2b256_collision_free` (the hash is injective) with `encodeNode_injective` (the
    serializer is canonical), which is exactly what the code relies on when it refuses a colliding
    write instead of tolerating one (`radix_tree.rs:208-223,226-258`). -/
theorem root_collision_free {a b : Node} (h : nodeHash a = nodeHash b) : a = b :=
  encodeNode_injective (blake2b256_collision_free _ _ h)

/-- **Law 10 (the empty root)** — a node's root is the empty root **iff** it is the empty node: the
    empty root is a fixed point, and nothing else hashes to it. -/
theorem nodeHash_eq_emptyRoot {n : Node} : nodeHash n = emptyRoot ↔ n = emptyNode := by
  constructor
  · intro h
    exact root_collision_free (by simpa [emptyRoot] using h)
  · intro h
    rw [h]
    rfl

end Rchain
