import Rchain.Casper.Fringe
import Rchain.Crypto.Spec

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
      is abstract, like `encodeBody` below, because the field *list* is not the law; its **presence** is
      load-bearing, because the hash covers them and a model without it would claim the hash ignores the
      header. -/
  header : Msg

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

/-- The canonical encoding of a body — the serialization `hash_block` hashes. Abstract, like
    `Rchain.RSpace.encodeNode`, because the serializer is not the law. -/
axiom encodeBody : BlockBody → Msg

/-- The encoding is **canonical**: equal encodings are equal bodies. `to_bytes` over a canonical proto
    has this property, and it must be stated because the model's encoding is abstract — a hash can make
    an encoding collision-resistant but never canonical. -/
axiom encodeBody_injective {a b : BlockBody} : encodeBody a = encodeBody b → a = b

/-- The block's content hash, as the port computes it: Blake2b256 over the canonical encoding of the
    body (`hash_block`, `casper/src/proto_util.rs:58-64`). -/
noncomputable def blockHash (b : Block) : Hash := blake2b256 (encodeBody b.body)

/-- **Law 16c — content addressing: the hash determines the body.** Proven, not postulated: it composes
    Law 19's `blake2b256_collision_free` with `encodeBody_injective`, which is exactly what the port
    relies on and pins from both sides — `hash_block_is_deterministic_and_ignores_sig` (`proto_util.rs:138-144`,
    two blocks differing only in `sig` hash identically) and `hash_block_changes_with_body` (`:147-152`).
    The old axiom said it about a bare `hash : Nat` field, with no relation to any hashing function. -/
theorem content_addressing {a b : Block} (h : blockHash a = blockHash b) : a.body = b.body :=
  encodeBody_injective (blake2b256_collision_free _ _ h)

/-- **The port's `hash_block_changes_with_timestamp` (`proto_util.rs:155-162`), derived.** Because
    `hash_block` covers every field except `block_hash` and `sig`, two blocks differing in **any** hashed
    field hash differently. This is the statement a model that carried only `(number, seqNum, parents)`
    could not make — and the reason the body carries `timestamp` and `header`. -/
theorem blockHash_changes_with_header {a b : Block} (h : a.header ≠ b.header) :
    blockHash a ≠ blockHash b := by
  intro hh
  exact h (by simpa using congrArg Block.header (content_addressing hh))

-- Law 17's arithmetic is not in this file. `numeric_channels_nonneg` lived here until the
-- consolidation pass and claimed numeric channels are non-negative — which is **false of the code**:
-- they are signed `i64` and negative diffs are ordinary (`rholang/src/merging.rs:161-166`, tests at
-- `:349,370` with `diff: -5`). The non-negativity that *is* true belongs to Law 14's bonds and to
-- `NonNegI64` (`shared/src/refined.rs:64`), which types bonds and heights, never numeric channels.
-- The law's real content — the checked `i64` arithmetic of the merge, the unchecked accumulator beside
-- it, and the RNG merge's call-site canonicalization — is modelled in `Rchain/Merging.lean`.

/-- Law 18: the height map is contiguous — no holes in block heights. -/
axiom height_map_contiguous (bs : List Block) :
  ∀ b ∈ bs, b.number > 0 → ∃ c ∈ bs, c.number = b.number - 1

/-- Law 18: the fringe identity is order-independent (a set, not a list). -/
axiom fringe_identity_order_independent (f g : Fringe) :
  List.Perm f.messages g.messages → f = g

end Rchain
