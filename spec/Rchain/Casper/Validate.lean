import Rchain.Casper.Fringe
import Rchain.Cmp
import Rchain.Crypto.Spec
import Rchain.Proto

/-!
# Laws 16–18 — block/merge/storage validation

Block number is `max(parent) + 1` and `seqNum` strictly increments (Law 16); the block hash is
`Blake2b256` over the body with `block_hash` and `sig` cleared — *computed*, not stored — and the bonds
cache equals the PoS state (Law 16); the merge's arithmetic is the checked 64-bit one, and the numeric
channels it works on are **signed** (Law 17); the height map is contiguous and the fringe identity is
order-independent (Law 18). The Scala oracle is `casper/Validate.scala` +
`sdk/dag/merging/ConflictResolutionLogic.scala` + `block-storage/BlockMetadataStore.scala`; the Rust
realization is `BlockHeight`/`SeqNum`/`BlockHash`/`StateHash` (`[u8; 32]`).
-/

namespace Rchain

/-- A justification as the two number checks read it: the sender, block number and sequence number of a
    block this one justifies, and whether that block failed validation — the four fields
    `block_number`/`sequence_number` fold over (`casper/src/validate.rs:121-162`). The rest of the port's
    `BlockMetadata` is not modelled, because no rule here reads it. -/
structure Parent where
  sender : Nat
  number : Nat
  seqNum : Nat
  validationFailed : Bool
deriving DecidableEq

/-- A block, as the model has it. It carries **no** hash field: the port's block hash is *computed* from
    the body (`hash_block`, `casper/src/proto_util.rs:58-64`), so the model computes it too rather than
    storing a number that could disagree with the body.

    The field list is the *hashed* one, which is the point: `hash_block` clears `block_hash` and `sig`
    and hashes every other field, so a model that carried fewer fields would be claiming the hash ignores
    the ones it left out. -/
structure Block where
  number : Nat
  seqNum : Nat
  /-- The proposer — a field of its own because Law 16b's check folds over the justifications **whose
      sender is this block's sender** (`casper/src/validate.rs:149-156`). -/
  sender : Nat
  /-- The blocks this one justifies (the port's `justifications`, whose metadata the two number checks
      look up — `validate.rs:124-133`, `:150-156`). -/
  justifications : List Parent
  /-- The proposer's wall clock (ms). No consensus rule reads it, but `hash_block` **hashes** it — which
      is what the port's own `hash_block_changes_with_timestamp` pins (`proto_util.rs:155-162`). -/
  timestamp : Nat
  /-- Every **other** field `hash_block` covers — `version`, `shard_id`, the two state hashes, `bonds`,
      the three rejected sets, `state`, `sig_algorithm` (`BlockMessage`,
      `models/src/casper/protocol/casper_message.rs:563-585`) — as one canonical serialization. Its type
      is abstract, unlike `encodeBody` below (which is a definition and writes this field as one
      length-delimited blob), because the field *list* is not the law; its **presence** is load-bearing,
      because the hash covers them and a model without it would claim the hash ignores the header. -/
  header : Msg
deriving DecidableEq

/-- The maximum block number among a block's **non-failed** justifications — the port's
    `max_block_number`, which starts at `-1` so that a block with no live justification must be numbered
    `0` (`casper/src/validate.rs:123-133`). `none` is that `-1`, a value no block number can take. -/
def maxParentNumber (js : List Parent) : Option Nat :=
  (js.filter (fun p => !p.validationFailed)).foldl
    (fun acc p => some (match acc with | none => p.number | some n => max n p.number)) none

/-- **Law 16a's predicate** — the port's own check (`validate.rs:135-140`): the block is valid exactly
    when its number is one more than the maximum non-failed justification's, or `0` when it justifies
    nothing live. The code enforces this by *refusing* a block, so the predicate — not a universal axiom
    — is what the law is stated over. -/
def BlockNumberValid (b : Block) : Prop :=
  match maxParentNumber b.justifications with
  | none => b.number = 0
  | some n => b.number = n + 1

/-- **Law 16a** — block number = max(parent) + 1, as a consequence of the validation predicate. -/
theorem block_number_max_parent_plus_one (b : Block) (h : BlockNumberValid b) (n : Nat)
    (hn : maxParentNumber b.justifications = some n) : b.number = n + 1 := by
  simpa [BlockNumberValid, hn] using h

/-- The predicate is not vacuous: a block whose number is not the maximum-plus-one is rejected — the
    port's `InvalidBlockNumber` (`validate.rs:135-140`). -/
theorem block_number_rejects (b : Block) (n : Nat)
    (hn : maxParentNumber b.justifications = some n) (hbad : b.number ≠ n + 1) :
    ¬ BlockNumberValid b := by
  simpa [BlockNumberValid, hn] using hbad

/-- **The axiom that stood here was false**, and it is published rather than quietly replaced: it
    quantified over every `Block` with no hypothesis, and a `Block` is freely constructed — a block
    numbered 5 with nothing live to justify it is valid only if `5 = 0`. A false axiom is worse than an
    owed proof: anything follows from it. -/
theorem block_number_universal_is_false : ¬ ∀ b : Block, BlockNumberValid b := by
  intro h
  have hb := h ⟨5, 0, 0, [], 0, []⟩
  simp [BlockNumberValid, maxParentNumber] at hb

/-- The sender's latest sequence number among a block's justifications — the port's `creator_latest_seq`,
    folded over the justifications **whose sender is this block's sender** and seeded `-1`
    (`casper/src/validate.rs:146-157`). `none` is that `-1`. -/
def senderLatestSeq (sender : Nat) (js : List Parent) : Option Nat :=
  (js.filter (fun p => p.sender == sender)).foldl
    (fun acc p => some (match acc with | none => p.seqNum | some n => max n p.seqNum)) none

/-- **Law 16b's predicate** — the port's `sequence_number` check (`validate.rs:158-162`). -/
def SeqNumValid (b : Block) : Prop :=
  match senderLatestSeq b.sender b.justifications with
  | none => b.seqNum = 0
  | some n => b.seqNum = n + 1

/-- **Law 16b** — `seqNum` strictly increases, **re-scoped to what the check reads**: the sender's own
    justifications, and their maximum. Two blocks from one sender with no justification between them are
    not related by this law at all — which is why the previous row's diagnosis ("the sender relation is
    missing") would not have been enough. -/
theorem seq_num_strictly_increases (b : Block) (h : SeqNumValid b) (n : Nat)
    (hn : senderLatestSeq b.sender b.justifications = some n) : b.seqNum = n + 1 := by
  simpa [SeqNumValid, hn] using h

/-- **The universal form that stood here is false** — and it is false even of *one* sender, which is the
    part the old note got wrong: the missing hypothesis is the justification relation and its maximum,
    not the sender relation. Published, because a false axiom proves anything. -/
theorem seq_num_universal_is_false : ¬ ∀ b : Block, SeqNumValid b := by
  intro h
  have hb := h ⟨0, 5, 0, [], 0, []⟩
  simp [SeqNumValid, senderLatestSeq] at hb

/-- The block body a hash commits to: the block itself. `Block.hash` was deleted in the consolidation
    pass, so there is no stored hash that could disagree with the body — the hash is *computed*
    (`blockHash`), and the fields the port clears before hashing (`block_hash` to a zero-filled `Hash32`,
    `sig` to empty, `casper/src/proto_util.rs:58-64`) are fields the model does not carry. -/
abbrev BlockBody := Block

/-- A block's body — the identity here, named because the law is stated about the body. -/
def Block.body (b : Block) : BlockBody := b

/-- A justification's canonical key: the order `to_proto` sorts them into before hashing. -/
def Parent.key (p : Parent) : Nat × Nat × Nat := (p.sender, p.number, p.seqNum)

/-- **What the serializer can represent**: the numbers fit the proto's `int64` fields
    (`CasperMessage.proto:72,74` — `blockNumber` and `seqNum`, and the Rust's `timestamp` likewise), and
    the justifications are already in the port's canonical order (`misc`/`casper_message.rs:636` sorts
    them, so their order is not part of what is hashed).

    Both halves are properties of the *model* rather than of the code, which is why they are a
    hypothesis here: the code's `blockNumber`/`seqNum` **are** `i64` and its justifications are sorted
    before the bytes are written. The model's are a `Nat` and whatever order the block arrived in,
    because its validation rules fold over them in any order (Law 16a/16b). -/
def Canonical (b : BlockBody) : Prop :=
  b.number < 2 ^ 63 ∧ b.seqNum < 2 ^ 63 ∧ b.timestamp < 2 ^ 63 ∧
  b.justifications.Pairwise (fun p q => p.key < q.key)

/-! ## The block-body encoder — what `hash_block` hashes, as a definition

This block used to be two axioms (`axiom encodeBody`, `axiom encodeBody_injective`) with the note that
they were "assumed of an undefined function, so they constrain nothing". They are now a definition over
protobuf's wire format (`Rchain/Proto.lean`) and a theorem about it — and the hypothesis the theorem
needs is **load-bearing**, which `the_body_encoder_is_not_injective_without_canonical` proves: the
encoder is *not* injective without `Canonical`.

The field list and their order are `BlockMessageProto`'s (`models/proto/casper.proto:45-66`) and the
justification order is the Rust's (`casper_message.rs:633-675`: `to_proto` sorts `justifications`,
`bonds` and the three rejected sets "for Law 16 determinism"). Four things cannot be mirrored byte for
byte, and saying so is part of the point:

* `number`, `seqNum` and `timestamp` are the proto's `int64` fields (tags 32, 48, 136, wire type 0), so
  they are written as **64-bit** varints. That is what makes `Canonical`'s number bounds load-bearing:
  the bytes determine the residue, and the bound is what makes the residue the value
  (`int64_eq_of_lt`).
* `sender` is `bytes` in the proto and an abstract id (`Nat`) in the model, so it is written as an
  ordinary varint under its own tag. The port writes 32 bytes; the model has no 32-byte form to write.
* `justifications` are `repeated bytes` (32-byte block hashes) in the port and structured `Parent`s
  here — the metadata the number checks read (`validate.rs:121-162`). The sort order is `Parent.key`,
  which is the port's `justifications.sort()` (on hashes) carried over to the objects carrying those
  hashes' metadata.
* `header` is the model's bundle of *every other* hashed field (`version`, `shardId`, the two state
  hashes, `bonds`, the three rejected sets, `state`, `sigAlgorithm`), written as one length-delimited
  blob.

**The residue, stated rather than implied.** `prost`'s `encode_to_vec` is an external crate (0.13.5,
`Cargo.lock`), not vendored, so "these bytes are the node's bytes" remains a *prose* tie. What is
machine-checked here is the *structure*: which fields are hashed, in what order, with which collection
sorted — the part a missing length prefix or an unordered map would break, and the part that is a
merkle-root collision between distinct states if it is wrong.

**Why this does not need a permutation lemma.** The port's sort makes its encoder insensitive to the
order of `justifications`, so the port's hash identifies blocks differing only in that order — which is
the content of the *existing* `a_body_encoder_that_canonicalises_is_not_injective` below, stated for
*any* order-insensitive encoder and therefore not instantiable at this one. That is not a gap: it is the
reason `Canonical` carries `justifications.Pairwise (key <)`, and with that hypothesis the sort *is* the
identity (`sortParents_of_pairwise`), so the two facts compose into injectivity on the port's domain.
Proving `Perm l l' → sortParents l = sortParents l'` would be needed only to state injectivity *without*
`Canonical`, and without `Canonical` injectivity is false. -/

/-- The port's justification order — `to_proto`'s `justifications.sort()`, on the model's objects rather
    than on their hashes. Keyed on `Parent.key`, which `Canonical` requires to be strictly increasing. -/
def sortParents (l : List Parent) : List Parent := l.insertionSort (fun p q => p.key ≤ q.key)

/-- **The sort is the identity on the port's domain** — how `Canonical`'s ordering clause turns the sort
    into nothing at all. -/
theorem sortParents_of_pairwise {l : List Parent} (h : l.Pairwise (fun p q => p.key < q.key)) :
    sortParents l = l := by
  unfold sortParents
  exact List.Sorted.insertionSort_eq (h.imp (fun hab => le_of_lt hab))

/-- The wire form of one justification: three varints and its flag byte. -/
def encodeParent (p : Parent) : Msg :=
  varint p.sender ++ varint p.number ++ varint p.seqNum
    ++ [if p.validationFailed then (1 : Byte) else 0]

/-- The inverse of `encodeParent`. -/
def decodeParent (m : Msg) : Option (Parent × Msg) := do
  let (sender, m) ← decodeVarint m
  let (number, m) ← decodeVarint m
  let (seqNum, m) ← decodeVarint m
  match m with
  | [] => failure
  | b :: rest => pure (⟨sender, number, seqNum, (b : Nat) == 1⟩, rest)

/-- **A justification is self-delimiting**, with no length prefix — sound only because each field in it
    is. -/
theorem decodeParent_encodeParent (p : Parent) (r : Msg) :
    decodeParent (encodeParent p ++ r) = some (p, r) := by
  obtain ⟨sender, number, seqNum, failed⟩ := p
  simp [encodeParent, decodeParent, decodeVarint_varint, List.append_assoc]
  cases failed <;> rfl

/-- Decode `k` justifications in sequence (the encoder writes their count, not their byte length). -/
def decodeParents : Nat → Msg → Option (List Parent × Msg)
  | 0, m => some ([], m)
  | k + 1, m => do
      let (p, m) ← decodeParent m
      let (ps, m) ← decodeParents k m
      pure (p :: ps, m)

/-- The count-prefixed justification list round-trips. -/
theorem decodeParents_length_encodings (l : List Parent) (r : Msg) :
    decodeParents l.length ((l.map encodeParent).join ++ r) = some (l, r) := by
  induction l with
  | nil => simp [decodeParents]
  | cons p ps ih =>
    simp only [List.map_cons, List.join_cons, List.length_cons, List.append_assoc]
    simp [decodeParents, decodeParent_encodeParent, ih]

/-- **The encoder**: every field `hash_block` covers, in the proto's field-number order, with the
    justification collection in the port's canonical order. -/
def encodeBody (b : BlockBody) : Msg :=
  varintField 32 (int64 b.number)
    ++ varintField 48 (int64 b.seqNum)
    ++ varintField 40 b.sender
    ++ varintField 136 (int64 b.timestamp)
    ++ varint 74 ++ varint b.justifications.length
    ++ ((sortParents b.justifications).map encodeParent).join
    ++ bytesField 114 b.header

/-- The canonical form the encoder is **blind** to: the three fields it narrows to 64 bits, and the
    justification order it sorts. The round trip below returns this, which is exactly why `Canonical` is
    the hypothesis the injectivity theorem needs.

    Written as a constructor rather than an update: Lean 4.12's parser rejects a `{ b with … }` (or a
    bare `{ … }` literal) whose field list has **a line break after a comma** — `unexpected identifier;
    expected '}'` — while the same text inside `[ … ]` parses, which is why the register rows
    (`Laws.lean`) get away with it and this definition did not. -/
def canonicalise (b : BlockBody) : BlockBody :=
  ⟨int64 b.number, int64 b.seqNum, b.sender, sortParents b.justifications, int64 b.timestamp,
    b.header⟩

/-- One tagged `varint` field: read the field's tag, check it is the expected one, then read the value. -/
def taggedVarint (tag : Nat) (m : Msg) : Option (Nat × Msg) :=
  match decodeVarint m with
  | some (t, m) => if t = tag then decodeVarint m else none
  | none => none

/-- **A tagged `varint` field is self-delimiting too**, and reading it consumes exactly the field: what
    follows the value is what followed the field. -/
theorem taggedVarint_varintField (tag n : Nat) (r : Msg) :
    taggedVarint tag (varintField tag n ++ r) = some (n, r) := by
  simp [taggedVarint, varintField, List.append_assoc, decodeVarint_varint]

/-- A tagged `varint` field written by hand — a tag, then a value that is not a `varintField` (the
    justification count, tag 74) — reads back the same way.

    The parentheses are load-bearing: `++` is **left**-associative, so `varint tag ++ varint n ++ r` is
    `(varint tag ++ varint n) ++ r`, while the goal `decodeBody_encodeBody` reaches this step with is the
    reassociated `varint tag ++ (varint n ++ r)` — `List.append_assoc` is what puts it in that shape.
    Stated without the parentheses the lemma compiles, reads correctly, and never fires. -/
theorem taggedVarint_varint (tag n : Nat) (r : Msg) :
    taggedVarint tag (varint tag ++ (varint n ++ r)) = some (n, r) := by
  simp [taggedVarint, decodeVarint_varint]

/-- A length-delimited field reads back as its payload and the bytes after it — the length prefix is
    consumed and the payload is returned whole. -/
theorem taggedVarint_bytesField (tag : Nat) (p r : Msg) :
    taggedVarint tag (bytesField tag p ++ r) = some (p.length, p ++ r) := by
  simp [taggedVarint, bytesField, List.append_assoc, decodeVarint_varint]

/-- The inverse of `encodeBody`, one field at a time (each field is self-delimiting, which is what makes
    that possible).

    **Why this is not the `guard`-per-tag do-block it first was.** That spelling — a `let (t, m) ←
    decodeVarint m` and a `guard (t = …)` for each of the six tags — elaborated in 128 ms and then made
    the **compiler** take **98.6 s** and 5 GB on this one definition (`lean --profile`: `compilation of
    Rchain.decodeBody took 98.6s`), which is what left the build unusable. The eighteen nested closures
    are what the code generator choked on. This form, one `taggedVarint` per field, compiles in 224 ms. -/
def decodeBody (m : Msg) : Option (BlockBody × Msg) := do
  let (number, m) ← taggedVarint 32 m
  let (seqNum, m) ← taggedVarint 48 m
  let (sender, m) ← taggedVarint 40 m
  let (timestamp, m) ← taggedVarint 136 m
  let (k, m) ← taggedVarint 74 m
  let (js, m) ← decodeParents k m
  let (hlen, m) ← taggedVarint 114 m
  pure (⟨number, seqNum, sender, js, timestamp, m.take hlen⟩, m.drop hlen)

/-- **The round trip**: the encoder's own inverse recovers the body up to exactly what the encoding
    cannot see — the 64-bit narrowing and the justification order. -/
theorem decodeBody_encodeBody (b : BlockBody) (r : Msg) :
    decodeBody (encodeBody b ++ r) = some (canonicalise b, r) := by
  obtain ⟨number, seqNum, sender, js, timestamp, header⟩ := b
  -- The encoder writes the justification count, so the decoder is handed `js.length`; the lemma about
  -- the round trip is stated for the list it actually encoded, `sortParents js`.
  have hlen : js.length = (sortParents js).length :=
    (List.length_insertionSort (fun p q : Parent => p.key ≤ q.key) js).symm
  simp [encodeBody, decodeBody, canonicalise, hlen, taggedVarint_varintField, taggedVarint_varint,
    taggedVarint_bytesField, decodeParents_length_encodings, List.append_assoc,
    List.take_left, List.drop_left]

/-- **Law 16c's injectivity, as a theorem about a definition**: on the bodies the port can hash — the
    numbers fit their `int64` fields and the justifications are already in the port's order — equal
    encodings are equal bodies. `content_addressing` below composes this with Law 19's collision-freedom
    to get the hash-level law. -/
theorem encodeBody_injective {a b : BlockBody} (ha : Canonical a) (hb : Canonical b)
    (h : encodeBody a = encodeBody b) : a = b := by
  have hda := decodeBody_encodeBody a []
  have hdb := decodeBody_encodeBody b []
  rw [h] at hda
  have hcanon : canonicalise a = canonicalise b :=
    congrArg Prod.fst (Option.some.inj (hda.symm.trans hdb))
  have hn : a.number = b.number :=
    int64_eq_of_lt ha.1 hb.1 (congrArg Block.number hcanon)
  have hs : a.seqNum = b.seqNum :=
    int64_eq_of_lt ha.2.1 hb.2.1 (congrArg Block.seqNum hcanon)
  have ht : a.timestamp = b.timestamp :=
    int64_eq_of_lt ha.2.2.1 hb.2.2.1 (congrArg Block.timestamp hcanon)
  have hj : a.justifications = b.justifications := by
    have h' := congrArg Block.justifications hcanon
    simp only [canonicalise] at h'
    rwa [sortParents_of_pairwise ha.2.2.2, sortParents_of_pairwise hb.2.2.2] at h'
  obtain ⟨an, asq, asd, aj, ats, ah⟩ := a
  obtain ⟨bn, bsq, bsd, bj, bts, bh⟩ := b
  -- `canonicalise` narrows the three numbers and sorts the justifications; the other two fields it
  -- carries over untouched, so they are equal for the same reason and have to be said.
  have hsd : asd = bsd := congrArg Block.sender hcanon
  have hh : ah = bh := congrArg Block.header hcanon
  simp only at hn hs ht hj
  subst hn; subst hs; subst hsd; subst hj; subst ht; subst hh; rfl

/-- **`Canonical` is load-bearing, not decoration**: the *un-narrowed* statement — "equal encodings are
    equal bodies" — is false of **this** encoder, in the first of the two ways the witnesses below name.
    The encoder writes the three `int64` fields as 64-bit values, so a body numbered `2^63` and one
    numbered `2^63 + 2^64` are different bodies with the same bytes — and the first is exactly the
    largest number `Canonical` admits. -/
theorem the_body_encoder_is_not_injective_without_canonical : ¬ Function.Injective encodeBody := by
  intro hinj
  have hres : int64 (2 ^ 63 + 2 ^ 64) = int64 (2 ^ 63) := by
    have h1 : ((2 : Nat) ^ 63 + 2 ^ 64) % 2 ^ 64 = 2 ^ 63 := by
      rw [show (2 : Nat) ^ 63 + 2 ^ 64 = 2 ^ 63 + 1 * 2 ^ 64 by rw [Nat.one_mul],
        Nat.add_mul_mod_self_right, Nat.mod_eq_of_lt (by omega)]
    simp only [int64]
    rw [h1, Nat.mod_eq_of_lt (by omega)]
  have hne : (⟨2 ^ 63, 0, 0, [], 0, []⟩ : BlockBody) ≠ ⟨2 ^ 63 + 2 ^ 64, 0, 0, [], 0, []⟩ := by
    intro heq
    have := congrArg Block.number heq
    simp only at this
    omega
  refine hne (hinj ?_)
  simp only [encodeBody, sortParents, List.insertionSort, hres]

/-- **The un-narrowed axiom is false, first way: the model's numbers are wider than the proto's.** An
    encoder that mirrors the port writes `blockNumber` as an `int64` varint
    (`CasperMessage.proto:72`), so it identifies every pair of numbers with the same residue mod 2^64 —
    and `2^63` and `2^63 + 2^64` are two different `Nat`s with one image. -/
theorem a_body_encoder_that_truncates_is_not_injective
    (enc : BlockBody → Msg)
    (h : ∀ a b : BlockBody, a.number % 2 ^ 64 = b.number % 2 ^ 64 → enc a = enc b) :
    ¬ Function.Injective enc := by
  intro hinj
  have hbad : (2 : Nat) ^ 63 ≠ 2 ^ 63 + 2 ^ 64 := by decide
  have heq : (⟨2 ^ 63, 0, 0, [], 0, []⟩ : BlockBody) = ⟨2 ^ 63 + 2 ^ 64, 0, 0, [], 0, []⟩ :=
    hinj (h _ _ (by decide))
  exact hbad (congrArg Block.number heq)

/-- **The un-narrowed axiom is false, second way: the port hashes a *canonical* order.** `to_proto`
    sorts the justifications before writing them (`casper_message.rs:636`), so an encoder that mirrors
    it cannot distinguish a body from the same body with its justifications permuted — and those are two
    different `BlockBody`s. -/
theorem a_body_encoder_that_canonicalises_is_not_injective
    (enc : BlockBody → Msg)
    (h : ∀ a b : BlockBody, a.justifications.Perm b.justifications → enc a = enc b) :
    ¬ Function.Injective enc := by
  -- Both facts are decided on *closed* terms: `decide` refuses an expected type with `let`-bound
  -- locals in it ("must not contain free or meta variables"), so the parents are written out.
  have hne : (⟨0, 0, 0, [⟨0, 0, 0, false⟩, ⟨1, 1, 1, false⟩], 0, []⟩ : BlockBody)
      ≠ ⟨0, 0, 0, [⟨1, 1, 1, false⟩, ⟨0, 0, 0, false⟩], 0, []⟩ := by decide
  have hperm : ([⟨0, 0, 0, false⟩, ⟨1, 1, 1, false⟩] : List Parent).Perm
      [⟨1, 1, 1, false⟩, ⟨0, 0, 0, false⟩] := by decide
  intro hinj
  exact hne (hinj (h _ _ hperm))

/-- The block's content hash, as the port computes it: Blake2b256 over the canonical encoding of the
    body (`hash_block`, `casper/src/proto_util.rs:58-64`). -/
noncomputable def blockHash (b : Block) : Hash := blake2b256 (encodeBody b.body)

/-- **Law 16c — content addressing: the hash determines the body.** Proven, not postulated: it composes
    Law 19's `blake2b256_collision_free` with `encodeBody_injective`, which is exactly what the port
    relies on and pins from both sides — `hash_block_is_deterministic_and_ignores_sig` (`proto_util.rs:138-144`,
    two blocks differing only in `sig` hash identically) and `hash_block_changes_with_body` (`:147-152`).
    The old axiom said it about a bare `hash : Nat` field, with no relation to any hashing function. -/
theorem content_addressing {a b : Block} (ha : Canonical a.body) (hb : Canonical b.body)
    (h : blockHash a = blockHash b) : a.body = b.body :=
  encodeBody_injective ha hb (blake2b256_collision_free _ _ h)

/-- **The port's `hash_block_changes_with_timestamp` (`proto_util.rs:155-162`), derived.** Because
    `hash_block` covers every field except `block_hash` and `sig`, two blocks differing in **any** hashed
    field hash differently. This is the statement a model that carried only `(number, seqNum, parents)`
    could not make — and the reason the body carries `timestamp` and `header`. -/
theorem blockHash_changes_with_header {a b : Block} (ha : Canonical a.body) (hb : Canonical b.body)
    (h : a.header ≠ b.header) : blockHash a ≠ blockHash b := by
  intro hh
  exact h (by simpa using congrArg Block.header (content_addressing ha hb hh))

-- Law 17's arithmetic is not in this file. `numeric_channels_nonneg` lived here until the
-- consolidation pass and claimed numeric channels are non-negative — which is **false of the code**:
-- they are signed `i64` and negative diffs are ordinary (`rholang/src/merging.rs:161-166`, tests at
-- `:349,370` with `diff: -5`). The non-negativity that *is* true belongs to Law 14's bonds and to
-- `NonNegI64` (`shared/src/refined.rs:64`), which types bonds and heights, never numeric channels.
-- The law's real content — the checked `i64` arithmetic of the merge, the unchecked accumulator beside
-- it, and the RNG merge's call-site canonicalization — is modelled in `Rchain/Merging.lean`.

/-! ## Law 18 — the store's own invariants

Two axioms stood here and neither was a law of the code. `height_map_contiguous` claimed a property of
*any* `List Block` — false, because the DAG structure it needs was not a hypothesis.
`fringe_identity_order_independent` claimed `Perm → f = g` over a hand-built `Fringe` — also false, since
two lists being permutations does not make them equal. What the port has is an *invariant the store
checks* and a *representation that gives order-independence for free*. -/

/-- The store's height map, as the set of heights it holds (`block-storage/src/dag/metadata_store.rs:20`
    keeps a `BTreeMap<BlockHeight, BTreeSet<BlockHash>>`, and `validate_dag_state` checks its keys for
    holes, `:77-87`). -/
abbrev HeightSet := Nat → Prop

/-- **The invariant `validate_dag_state` checks**, as what it means: every height between the smallest
    the store holds and the largest is present — no holes. The port's arithmetic on a duplicate-free key
    list (`(max + 1) - min = len`, `metadata_store.rs:80-85`) is this. -/
def Contiguous (hs : HeightSet) : Prop :=
  ∀ h lo hi, hs lo → hs hi → lo ≤ h → h ≤ hi → hs h

/-- **Law 18a, the half the store cannot derive for itself.** Inserting a block whose number is the
    successor of the current maximum preserves contiguity — and the derivation runs through **Law 16a's
    check**, not through the store: `validate_dag_state` (`metadata_store.rs:77-87`) only checks the keys
    it is handed and never inspects parent structure, so the store cannot establish this on its own. -/
theorem contiguous_insert_succ (hs : HeightSet) (m : Nat) (hc : Contiguous hs) (hmax : hs m)
    (hbound : ∀ h, hs h → h ≤ m) : Contiguous (fun h => h = m + 1 ∨ hs h) := by
  intro h lo hi hlo hhi hle hle'
  rcases hlo with rfl | hlo
  · left
    rcases hhi with hhi | hhi
    · omega
    · have := hbound hi hhi; omega
  · rcases hhi with hhi | hhi
    · by_cases hh : h = m + 1
      · exact Or.inl hh
      · exact Or.inr (hc h lo m hlo hmax hle (by omega))
    · exact Or.inr (hc h lo hi hlo hhi hle hle')

/-- **Law 18a's negative case** — a block that skips a number leaves a hole at its predecessor, which is
    what the store's check reports (`metadata_store.rs:83-86`). The `hs lo` hypothesis is the store being
    non-empty, which the port's check also requires: on a single key there is no hole to have. -/
theorem contiguous_skip_leaves_hole (hs : HeightSet) (m lo : Nat) (hlo : hs lo)
    (hbound : ∀ h, hs h → h ≤ m) : ¬ Contiguous (fun h => h = m + 2 ∨ hs h) := by
  intro hc
  have hmem : (fun h => h = m + 2 ∨ hs h) (m + 1) :=
    hc (m + 1) lo (m + 2) (Or.inr hlo) (Or.inl rfl)
      (by have := hbound lo hlo; omega) (by omega)
  rcases hmem with h | h
  · omega
  · exact absurd (hbound (m + 1) h) (by omega)

/-- **The axiom that stood here was false**: a list of one block, numbered 1, has an element above `0`
    and no element numbered `0` — so the "no holes" claim fails of a list that is not a DAG's height
    map. The invariant is real, but it is the *store's*, not a property of any `List Block`. -/
theorem height_map_universal_is_false :
    ¬ ∀ bs : List Block, ∀ b ∈ bs, b.number > 0 → ∃ c ∈ bs, c.number = b.number - 1 := by
  intro h
  let b : Block := ⟨1, 0, 0, [], 0, []⟩
  have hb := h [b] b (by simp) (by simp [b])
  rcases hb with ⟨c, hc, hnum⟩
  simp at hc
  subst hc
  simp [b] at hnum

/-- The fringe's identity as the store keys it: its message ids in **sorted** order — the code hashes a
    `BTreeSet<BlockHash>` (`models/src/fringe_data.rs:22,38-43`), so its input is sorted by
    construction. -/
noncomputable def fringeId (f : Fringe) : List Nat :=
  Comparator.sortList (Comparator.linearOrderComparator Nat) (f.messages.map (·.id))

/-- **Law 18b** — the fringe identity is order-independent. In the port this is **structural**, not a
    law: `fringe` is a `BTreeSet<BlockHash>` wherever it appears (`models/src/fringe_data.rs:22`,
    `block-storage/src/dag/finalizer.rs:30`), so there is no list order for the identity to be invariant
    under. The model keeps a list — so the claim can be *stated* at all — and proves the invariance the
    type supplies, the same way Law 7's join key does. -/
theorem fringeId_perm (f g : Fringe) (h : List.Perm f.messages g.messages) :
    fringeId f = fringeId g := by
  simp only [fringeId]
  exact Comparator.sortList_perm (Comparator.linearOrderComparator Nat)
    (List.Perm.map (fun m => m.id) h)

/-- **The axiom that stood here was false**: two `Fringe`s that are permutations of one another are
    different values, and no hypothesis makes them equal. The port's order-independence comes from the
    *type* — a `BTreeSet` — which the model replaces with a sort (`fringeId`). -/
theorem fringe_identity_order_independent_is_false :
    ¬ ∀ f g : Fringe, List.Perm f.messages g.messages → f = g := by
  intro h
  let m0 : Message := ⟨0, 0, 0, 0, [], []⟩
  let m1 : Message := ⟨1, 0, 0, 0, [], []⟩
  exact absurd (h ⟨[m0, m1]⟩ ⟨[m1, m0]⟩ (List.Perm.swap m1 m0 [])) (by decide)

end Rchain
