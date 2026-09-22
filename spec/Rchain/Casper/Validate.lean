import Rchain.Casper.Fringe

/-!
# Laws 16–18 — block/merge/storage validation

Block number is `max(parent) + 1` and `seqNum` strictly increments (Law 16); content addressing
(`hash = Blake2b256(block − {hash,sig})`) and the bonds cache equals the PoS state (Law 16); merge is
deterministic and numeric channels are non-negative (Law 17); the height map is contiguous and the
fringe identity is order-independent (Law 18). The Scala oracle is `casper/Validate.scala` +
`sdk/dag/merging/ConflictResolutionLogic.scala` + `block-storage/BlockMetadataStore.scala`; the Rust
realization is `BlockHeight`/`SeqNum`/`BlockHash`/`StateHash` (`[u8; 32]`).
-/

namespace Rchain

/-- A block: number, sequence number, parent numbers, and its content hash. -/
structure Block where
  number : Nat
  seqNum : Nat
  parents : List Nat
  hash : Nat

/-- Law 16: block number = max(parent) + 1. -/
axiom block_number_max_parent_plus_one (b : Block) :
  b.number = (b.parents.foldl (fun acc p => max acc p) 0) + 1

/-- Law 16: `seqNum` is strictly one more than the sender's previous (monotone, no reuse). -/
axiom seq_num_strictly_increases (prev next : Block) :
  prev.seqNum + 1 = next.seqNum

/-- Law 16: content addressing — the hash determines the block body (collision-free). -/
axiom content_addressing (a b : Block) : a.hash = b.hash → a = b

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
