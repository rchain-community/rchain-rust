import Rchain.Crypto.Spec

/-!
# Law 10 — Merkle determinism

The history is a content-addressed radix trie. The Scala oracle is `history/RadixTree.scala:50-68`; the
Rust realization is `rspace/src/history/radix_tree.rs`: a node is `[Item; 256]` (`:15-35`), `encode`
writes its non-empty items in slot order as `[slot][length|kind][prefix][32 bytes]` (`:48-77`),
`hash_node` is Blake2b256 over that encoding (`:139-142`), `empty_root_hash` is the hash of an all-empty
node (`:38-45`), and the store **refuses** an actual collision at both layers (`save_node`'s assert at
`:208-223`, `commit`'s error at `:226-258`).

## What changed, and why, in two passes

The file first declared `trieRoot : NodeHash` — a *constant*, with no arguments, which is why nothing
could be proved about it — plus `trie_collision_free` and `trie_empty_root`. Those two are statements
about the *hash function* wearing a trie's name: "equal hashes give equal contents" is Law 19's
`blake2b256_collision_free` applied to the node encoding. The root became a **definition** over the node
type the code has, and the collision statements **theorems** composing the hash's injectivity with the
encoding's canonicity — where canonicity was `axiom encodeNode_injective`, stated over `Node := List
Item` and `Hash := List Byte`.

**That axiom was false as stated, and this file now says so** (2026-09-23). Both of its types are wider
than the code's: the code's values are `Blake2b256Hash` (exactly 32 bytes) and its nodes are
`[Item; 256]`, while the model's are a byte list of any length and a list of any length. The encoder
writes neither arity, so a byte stream has more than one reading, and
`the_encoder_is_not_canonical_over_the_models_types` exhibits two distinct nodes with the same encoding
— one item whose 35-byte "hash" re-reads as two items. The code is not vulnerable: `Hash32([u8; 32])`
and `[Item; 256]` make the second reading unrepresentable, and a prefix is a suffix of a 32-byte key so
it is under 128 bytes, well inside the 7-bit length field. What *was* wrong is the **statement**: it
claimed canonicity of the model's own types, which is false, instead of canonicity of the serializer
*given the invariant the code carries structurally*. That invariant is now `WellFormed`, the axiom
carries it, and `root_collision_free`/`nodeHash_eq_emptyRoot` inherit it — which is honest *and*
stronger where it matters, because the hypothesis is exactly what the trie's operations maintain.
-/

namespace Rchain

/-- A trie slot: empty, a leaf (prefix and value), or a pointer to a child by hash (`Item`,
    `rspace/src/history/radix_tree.rs:19-30`). The field is `pref` rather than `prefix`, which Lean
    reserves as a notation keyword. -/
inductive Item where
  | empty
  | leaf (pref : List Byte) (value : Hash)
  | node (pref : List Byte) (ptr : Hash)
deriving DecidableEq

/-- A trie node: 256 slots (`Node = [Item; NUM_ITEMS]`, `NUM_ITEMS = 256`, `radix_tree.rs:15-35`). The
    model keeps the list; the width is carried by `WellFormed` below rather than by the type. -/
abbrev Node := List Item

/-- **The invariant the code carries structurally and this model carries as a predicate.** The code's
    `Node` is `[Item; 256]` (256 slots, and the Rust type makes any other length unrepresentable), its
    values and pointers are `Hash32` (exactly 32 bytes), and a prefix is a path suffix of a 32-byte key,
    so it is shorter than 128 bytes — which is what the encoder's 7-bit length field needs. Without all
    three, the encoding is not canonical; with them it is, and the operations that build nodes maintain
    them. -/
def WellFormed (n : Node) : Prop :=
  n.length = 256 ∧
  ∀ it ∈ n,
    (match it with
      | .empty => True
      | .leaf pref value => pref.length < 128 ∧ value.length = 32
      | .node pref ptr => pref.length < 128 ∧ ptr.length = 32)

/-- One item's record: the slot index, a byte holding the prefix length with the top bit as the
    leaf/pointer tag, the prefix bytes, then the value's 32 bytes (`encode`, `radix_tree.rs:60-75`). The
    Rust truncates the index with `as u8` and the length with `& 0x7F`; both are mirrored here rather
    than idealized, because the truncation is what the invariant above has to be true of. -/
def byteOf (n : Nat) : Byte := ⟨n % 256, Nat.mod_lt _ (by decide)⟩

def encodeItem (idx : Nat) : Item → Msg
  | .empty => []
  | .leaf pref value => byteOf idx :: byteOf (pref.length % 128) :: (pref ++ value)
  | .node pref ptr => byteOf idx :: byteOf (128 + pref.length % 128) :: (pref ++ ptr)

/-- `encode` over the slots in order, empty slots contributing nothing (`radix_tree.rs:48-77`). -/
def encodeNodeAux : Nat → Node → Msg
  | _, [] => []
  | i, it :: rest => encodeItem i it ++ encodeNodeAux (i + 1) rest

/-- The canonical encoding of a node (`encode`, `radix_tree.rs:48-77`). A definition now, not an
    axiom: the serializer is part of the model, which is what lets its canonicity be *tested* — and the
    test is what found the false statement the previous version carried. -/
def encodeNode (n : Node) : Msg := encodeNodeAux 0 n

/-- The witness that the canonicity claim needed its invariant: a node whose single leaf has a 35-byte
    value, and a two-slot node whose records concatenate to the same bytes. `W.take 32` is the first
    leaf's value and `[1, 0, 9]` the second record's `[slot][length][value]`. -/
def witnessMsg : Hash := List.replicate 32 (0 : Byte) ++ [1, 0, 9]

/-- **The canonicity law is false over the model's own types.** Two distinct nodes, one encoding: the
    first node's 35-byte "hash" does not record its own length, so the same bytes are also the encoding
    of a node with two items. `WellFormed` excludes both nodes (the first value is not 32 bytes; the
    node is not 256 slots), which is exactly why the axiom below carries it. -/
theorem the_encoder_is_not_canonical_over_the_models_types :
    encodeNode [Item.leaf [] witnessMsg]
      = encodeNode [Item.leaf [] (witnessMsg.take 32), Item.leaf [] [9]] ∧
    ([Item.leaf [] witnessMsg] : Node)
      ≠ [Item.leaf [] (witnessMsg.take 32), Item.leaf [] [9]] := by
  decide

/-- The encoding is **canonical on the nodes the trie builds**: the slots are written in order with
    their index, so distinct well-formed nodes have distinct encodings. Stated as an axiom — the proof
    (a list induction unpacking the records, with the invariant supplying `len < 128` and the 32-byte
    widths) is owed, and the file's header records why it is not merely assumed: it is *false* without
    the hypothesis, as `the_encoder_is_not_canonical_over_the_models_types` shows. -/
axiom encodeNode_injective {a b : Node} (ha : WellFormed a) (hb : WellFormed b) :
    encodeNode a = encodeNode b → a = b

/-- The node's hash: Blake2b256 over the canonical encoding (`hash_node`, `radix_tree.rs:139-142`).

`noncomputable` because it calls `blake2b256`, an axiom — a modelled primitive has no compiled code, and
this is a definition in the specification rather than something the node executes. -/
noncomputable def nodeHash (n : Node) : Hash := blake2b256 (encodeNode n)

/-- The empty node: 256 empty slots (`empty_node`, `radix_tree.rs:38-40`). -/
def emptyNode : Node := List.replicate 256 Item.empty

/-- The empty node satisfies the invariant — the only one of its obligations that is not vacuous. -/
theorem emptyNode_wellFormed : WellFormed emptyNode := by
  constructor
  · show (List.replicate 256 Item.empty).length = 256
    simp only [List.length_replicate]
  · intro it hit
    rw [emptyNode, List.mem_replicate] at hit
    obtain ⟨-, rfl⟩ := hit
    exact trivial

/-- The empty history's root: the hash of the empty node (`empty_root_hash`, `radix_tree.rs:43-45`). -/
noncomputable def emptyRoot : Hash := nodeHash emptyNode

/-- **Law 10 (content addressing)** — the root determines the node. Proven, not postulated: it composes
    Law 19's `blake2b256_collision_free` (the hash is injective) with `encodeNode_injective` (the
    serializer is canonical *on well-formed nodes*), which is exactly what the code relies on when it
    refuses a colliding write instead of tolerating one (`radix_tree.rs:208-223,226-258`). -/
theorem root_collision_free {a b : Node} (ha : WellFormed a) (hb : WellFormed b)
    (h : nodeHash a = nodeHash b) : a = b :=
  encodeNode_injective ha hb (blake2b256_collision_free _ _ h)

/-- **Law 10 (the empty root)** — a well-formed node's root is the empty root **iff** it is the empty
    node: the empty root is a fixed point, and nothing else the trie can build hashes to it. -/
theorem nodeHash_eq_emptyRoot {n : Node} (hn : WellFormed n) :
    nodeHash n = emptyRoot ↔ n = emptyNode := by
  constructor
  · intro h
    exact root_collision_free hn emptyNode_wellFormed (by simpa [emptyRoot] using h)
  · intro h
    rw [h]
    rfl

end Rchain
