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

/-- One slot's part of the invariant: a prefix inside the encoder's 7-bit length field, and a payload of
    exactly 32 bytes. -/
def ItemWF (it : Item) : Prop :=
  match it with
  | .empty => True
  | .leaf pref value => pref.length < 128 ∧ value.length = 32
  | .node pref ptr => pref.length < 128 ∧ ptr.length = 32

/-- Every slot of a node satisfies `ItemWF`. -/
def ItemsWF (n : Node) : Prop := ∀ it ∈ n, ItemWF it

/-- **The invariant the code carries structurally and this model carries as a predicate.** The code's
    `Node` is `[Item; 256]` (256 slots, and the Rust type makes any other length unrepresentable), its
    values and pointers are `Hash32` (exactly 32 bytes), and a prefix is a path suffix of a 32-byte key,
    so it is shorter than 128 bytes — which is what the encoder's 7-bit length field needs. Without all
    three, the encoding is not canonical; with them it is, and the operations that build nodes maintain
    them. -/
def WellFormed (n : Node) : Prop := n.length = 256 ∧ ItemsWF n

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

/-! ## The canonicity proof

`encodeNode_injective` used to be an axiom here. It is a theorem now, and the shape of the proof is the
one the code's own decoder would have: a record is self-describing — its header carries the slot index
and a `length | kind` byte, so a reader knows where its payload ends — and `WellFormed` supplies the two
facts that makes true (32-byte payloads, prefixes under 128 so the 7-bit field is not truncated). The
width, 256 slots, is what rules out the last ambiguity: a byte stream that ends after a record could
belong to a node with any number of trailing empty slots, and `WellFormed` says the number is 256.

Two hypotheses do the work, and both are false of the model's *types* — which is why
`the_encoder_is_not_canonical_over_the_models_types` above is still the reason the statement carries
them. -/

/-- **A slot index is written as one byte, and on the range the trie uses that is injective.** -/
theorem byteOf_injective {i j : Nat} (hi : i < 256) (hj : j < 256) (h : byteOf i = byteOf j) :
    i = j := by
  have := congrArg Fin.val h
  simp only [byteOf, Fin.val_mk] at this
  omega

/-- A length that fits the 7-bit field is its own remainder. -/
theorem mod128_of_lt {n : Nat} (h : n < 128) : n % 128 = n := Nat.mod_eq_of_lt h

/-- An empty slot encodes to nothing, and nothing else does. -/
theorem encodeItem_eq_nil_iff (i : Nat) (it : Item) : encodeItem i it = [] ↔ it = Item.empty := by
  cases it <;> simp [encodeItem]

/-- A nonempty slot's record begins with its own index byte. -/
theorem encodeItem_head (i : Nat) {it : Item} (h : it ≠ Item.empty) (r : Msg) :
    (encodeItem i it ++ r).head? = some (byteOf i) := by
  cases it <;> simp_all [encodeItem]

/-- And such a record is never the empty message. -/
theorem encodeItem_append_ne_nil {i : Nat} {it : Item} (h : it ≠ Item.empty) (r : Msg) :
    encodeItem i it ++ r ≠ [] := by
  cases it <;> simp_all [encodeItem]

/-- **The first byte of a nonempty encoding is the index of its first nonempty slot**, and that index
    lies inside the slots the list covers. -/
theorem encodeNodeAux_head {s : Node} {i : Nat} (h : encodeNodeAux i s ≠ []) :
    ∃ k, i ≤ k ∧ k < i + s.length ∧ (encodeNodeAux i s).head? = some (byteOf k) := by
  induction s generalizing i with
  | nil => simp [encodeNodeAux] at h
  | cons it rest ih =>
      cases it with
      | empty =>
          simp only [encodeNodeAux, encodeItem, List.nil_append] at h ⊢
          obtain ⟨k, hk₁, hk₂, hhead⟩ := ih h
          exact ⟨k, by omega, by simp only [List.length_cons]; omega, hhead⟩
      | leaf pref value =>
          exact ⟨i, Nat.le_refl i, by simp only [List.length_cons]; omega,
            by simp [encodeNodeAux, encodeItem]⟩
      | node pref ptr =>
          exact ⟨i, Nat.le_refl i, by simp only [List.length_cons]; omega,
            by simp [encodeNodeAux, encodeItem]⟩

/-- A suffix's encoding never begins with the byte of an *earlier* slot: its first record is at a slot at
    or after the suffix's own start, and those two bytes differ on the range the trie uses. This is the
    step that rules out the empty-slot-versus-record ambiguity. -/
theorem encodeNodeAux_head_ne_byteOf {s : Node} {i j : Nat} (h : encodeNodeAux i s ≠ [])
    (hij : j < i) (hb : i + s.length ≤ 256) :
    (encodeNodeAux i s).head? ≠ some (byteOf j) := by
  obtain ⟨k, hk₁, hk₂, hhead⟩ := encodeNodeAux_head h
  intro hc
  have hkj : byteOf k = byteOf j := (Option.some.injEq _ _).mp (by rw [← hhead, hc])
  have := byteOf_injective (by omega) (by omega) hkj
  omega

/-- **A record determines itself.** Two nonempty slots encoded at the same index whose bytes agree are
    the same slot, and so are the bytes after them: the header's kind bit and length field say where the
    payload ends, so no byte stream has two readings. -/
theorem encodeItem_injective_at {it jt : Item} {i : Nat} {r r' : Msg}
    (hi : it ≠ Item.empty) (hj : jt ≠ Item.empty) (hwi : ItemWF it) (hwj : ItemWF jt)
    (h : encodeItem i it ++ r = encodeItem i jt ++ r') : it = jt ∧ r = r' := by
  match it, jt, hwi, hwj with
  | .leaf pref val, .leaf pref' val', ⟨hpi, hvi⟩, ⟨hpi', hvi'⟩ =>
      simp only [encodeItem] at h
      injection h with _ htail
      injection htail with htag hrest
      have hlen : pref.length % 128 = pref'.length % 128 :=
        byteOf_injective (by omega) (by omega) htag
      rw [mod128_of_lt hpi, mod128_of_lt hpi'] at hlen
      obtain ⟨h1, h2⟩ :=
        List.append_inj hrest (by simp only [List.length_append, hvi, hvi', hlen])
      obtain ⟨hpref, hval⟩ := List.append_inj h1 (by simp only [hlen])
      exact ⟨by rw [hpref, hval], h2⟩
  | .node pref ptr, .node pref' ptr', ⟨hpi, hvi⟩, ⟨hpi', hvi'⟩ =>
      simp only [encodeItem] at h
      injection h with _ htail
      injection htail with htag hrest
      have htag' : 128 + pref.length % 128 = 128 + pref'.length % 128 :=
        byteOf_injective (by omega) (by omega) htag
      have hlen : pref.length % 128 = pref'.length % 128 := by omega
      rw [mod128_of_lt hpi, mod128_of_lt hpi'] at hlen
      obtain ⟨h1, h2⟩ :=
        List.append_inj hrest (by simp only [List.length_append, hvi, hvi', hlen])
      obtain ⟨hpref, hptr⟩ := List.append_inj h1 (by simp only [hlen])
      exact ⟨by rw [hpref, hptr], h2⟩
  | .leaf pref _, .node pref' _, ⟨hpi, _⟩, _ =>
      simp only [encodeItem] at h
      injection h with _ htail
      injection htail with htag _
      have hlen : pref.length % 128 = 128 + pref'.length % 128 :=
        byteOf_injective (by omega) (by omega) htag
      rw [mod128_of_lt hpi] at hlen
      omega
  | .node pref _, .leaf pref' _, ⟨hpi, _⟩, ⟨hpi', _⟩ =>
      simp only [encodeItem] at h
      injection h with _ htail
      injection htail with htag _
      have hlen : 128 + pref.length % 128 = pref'.length % 128 :=
        byteOf_injective (by omega) (by omega) htag
      rw [mod128_of_lt hpi'] at hlen
      omega

/-- The `cons`/`cons` case of `encodeNodeAux_injective`: two heads of the trie, either of which may be
    empty, a leaf or a node. Batched out of the induction so that the induction reads as an induction —
    the sixteen cases are its own concern, and the induction hypothesis arrives as a function of
    `encodeNodeAux (i + 1) rest = encodeNodeAux (i + 1) rest'` rather than as a rewrite. -/
private theorem encodeNodeAux_injective_cons (it jt : Item) (rest rest' : Node) (i : Nat)
    (hs_head : ItemWF it) (ht_head : ItemWF jt) (hi' : i + 1 + rest.length ≤ 256)
    (hlen' : rest.length = rest'.length)
    (ih : encodeNodeAux (i + 1) rest = encodeNodeAux (i + 1) rest' → rest = rest')
    (h : encodeNodeAux i (it :: rest) = encodeNodeAux i (jt :: rest')) :
    it :: rest = jt :: rest' := by
            rw [encodeNodeAux, encodeNodeAux] at h
            cases hjt : jt with
            | empty =>
                cases hit : it with
                | empty =>
                    simp only [hit, hjt, encodeItem, List.nil_append] at h
                    exact congrArg (List.cons Item.empty)
                      (ih h)
                | leaf pref value =>
                    simp only [hit, hjt, encodeItem, List.nil_append] at h
                    have hright : (encodeNodeAux (i + 1) rest').head? = some (byteOf i) := by
                      rw [← h]
                      exact encodeItem_head (it := Item.leaf pref value) i (by simp) _
                    exact absurd hright
                      (encodeNodeAux_head_ne_byteOf (s := rest') (i := i + 1) (j := i)
                        (by rw [← h]; exact encodeItem_append_ne_nil (it := Item.leaf pref value) (by simp) _) (by omega) (by rw [← hlen']; exact hi'))
                | node pref ptr =>
                    simp only [hit, hjt, encodeItem, List.nil_append] at h
                    have hright : (encodeNodeAux (i + 1) rest').head? = some (byteOf i) := by
                      rw [← h]
                      exact encodeItem_head (it := Item.node pref ptr) i (by simp) _
                    exact absurd hright
                      (encodeNodeAux_head_ne_byteOf (s := rest') (i := i + 1) (j := i)
                        (by rw [← h]; exact encodeItem_append_ne_nil (it := Item.node pref ptr) (by simp) _) (by omega) (by rw [← hlen']; exact hi'))
            | leaf pref' value' =>
                cases hit : it with
                | empty =>
                    simp only [hit, hjt, encodeItem, List.nil_append] at h
                    have hleft : (encodeNodeAux (i + 1) rest).head? = some (byteOf i) := by
                      rw [h]
                      exact encodeItem_head (it := Item.leaf pref' value') i (by simp) _
                    exact absurd hleft
                      (encodeNodeAux_head_ne_byteOf (s := rest) (i := i + 1) (j := i)
                        (by rw [h]; exact encodeItem_append_ne_nil (it := Item.leaf pref' value') (by simp) _) (by omega) hi')
                | leaf pref value =>
                    rw [hit, hjt] at h
                    obtain ⟨heq, hrest⟩ :=
                      encodeItem_injective_at (it := Item.leaf pref value)
                        (jt := Item.leaf pref' value') (by simp) (by simp) (hit ▸ hs_head)
                        (hjt ▸ ht_head) h
                    rw [heq]
                    exact congrArg (List.cons (Item.leaf pref' value'))
                      (ih hrest)
                | node pref ptr =>
                    rw [hit, hjt] at h
                    obtain ⟨heq, _⟩ :=
                      encodeItem_injective_at (it := Item.node pref ptr)
                        (jt := Item.leaf pref' value') (by simp) (by simp) (hit ▸ hs_head)
                        (hjt ▸ ht_head) h
                    exact absurd heq (by simp)
            | node pref' ptr' =>
                cases hit : it with
                | empty =>
                    simp only [hit, hjt, encodeItem, List.nil_append] at h
                    have hleft : (encodeNodeAux (i + 1) rest).head? = some (byteOf i) := by
                      rw [h]
                      exact encodeItem_head (it := Item.node pref' ptr') i (by simp) _
                    exact absurd hleft
                      (encodeNodeAux_head_ne_byteOf (s := rest) (i := i + 1) (j := i)
                        (by rw [h]; exact encodeItem_append_ne_nil (it := Item.node pref' ptr') (by simp) _) (by omega) hi')
                | leaf pref value =>
                    rw [hit, hjt] at h
                    obtain ⟨heq, _⟩ :=
                      encodeItem_injective_at (it := Item.leaf pref value)
                        (jt := Item.node pref' ptr') (by simp) (by simp) (hit ▸ hs_head)
                        (hjt ▸ ht_head) h
                    exact absurd heq (by simp)
                | node pref ptr =>
                    rw [hit, hjt] at h
                    obtain ⟨heq, hrest⟩ :=
                      encodeItem_injective_at (it := Item.node pref ptr)
                        (jt := Item.node pref' ptr') (by simp) (by simp) (hit ▸ hs_head)
                        (hjt ▸ ht_head) h
                    rw [heq]
                    exact congrArg (List.cons (Item.node pref' ptr'))
                      (ih hrest)

/-- **The encoding is canonical on well-formed nodes.** Two nodes of the same width whose encodings agree
    are equal: each record carries its slot and its own extent, so a byte stream has one reading, and
    `WellFormed` supplies the two facts that makes true plus the width that rules out a stream ending in
    empty slots. -/
theorem encodeNodeAux_injective (s t : Node) (i : Nat)
    (hs : ItemsWF s) (ht : ItemsWF t) (hlen : s.length = t.length)
    (hi : i + s.length ≤ 256) (h : encodeNodeAux i s = encodeNodeAux i t) : s = t := by
  induction s generalizing t i with
  | nil =>
      cases t with
      | nil => rfl
      | cons jt rest' => simp only [List.length_nil, List.length_cons] at hlen; omega
  | cons it rest ih =>
      have hs_head : ItemWF it := hs it (List.mem_cons_self ..)
      have hs_rest : ItemsWF rest := fun x hx => hs x (List.mem_cons_of_mem _ hx)
      have hi' : i + 1 + rest.length ≤ 256 := by
        simp only [List.length_cons] at hi; omega
      cases t with
      | nil => simp only [List.length_nil, List.length_cons] at hlen; omega
      | cons jt rest' =>
          have ht_head : ItemWF jt := ht jt (List.mem_cons_self ..)
          have ht_rest : ItemsWF rest' := fun x hx => ht x (List.mem_cons_of_mem _ hx)
          have hlen' : rest.length = rest'.length := by
            simp only [List.length_cons] at hlen; omega
          have hstep : encodeNodeAux (i + 1) rest = encodeNodeAux (i + 1) rest' → rest = rest' :=
            fun hh => ih rest' (i + 1) hs_rest ht_rest hlen' hi' hh
          exact encodeNodeAux_injective_cons it jt rest rest' i hs_head ht_head hi' hlen' hstep h

/-- The encoding is **canonical on the nodes the trie builds**: distinct well-formed nodes have distinct
    encodings. This was the file's `axiom encodeNode_injective`; it is a theorem now, and the two
    hypothesis it carries are exactly what the code's own types hold — the proof above reads the encoder
    the way a decoder would. -/
theorem encodeNode_injective {a b : Node} (ha : WellFormed a) (hb : WellFormed b) :
    encodeNode a = encodeNode b → a = b := by
  obtain ⟨halen, ha'⟩ := ha
  obtain ⟨hblen, hb'⟩ := hb
  exact encodeNodeAux_injective a b 0 ha' hb' (by rw [halen, hblen]) (by rw [halen]; omega)

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
