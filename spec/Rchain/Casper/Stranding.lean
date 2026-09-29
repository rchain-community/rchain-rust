import Rchain.Casper.Validate
import Rchain.Progress

/-!
# Law 53 — one attribution is terminal

C173, measured on a live testnet on 2026-09-29 (#105): a node that attributes **one** validation failure
to a bonded validator's block never follows that validator's chain again. Two rules make it so, and both
are the oracle's (`block_number` skips failed justifications when it computes the maximum, and
`neglected_invalid_block` refuses a block that justifies a failed bonded sender's block), so the law is
about the *combination* rather than about either rule:

- **the record persists** — a block marked failed is never unmarked. That is a modelling claim read off
  the port (`mark_failed` records the metadata of every `ValidateError::ValidationFailed`, and no rule
  clears the record) and it is the *only* thing this module takes from the code rather than proves from
  a rule. It is the same shape as 2PC's terminal records — `an_abort_is_absorbing`,
  `a_commit_is_absorbing` (`Rchain/CrossShard.lean`) — which is why Law 51's vocabulary names the two
  polarities apart: `Persistent` for this, `Unrestorable` for its dual;
- **the rule refuses the child** — `neglects` below, which is `neglected_invalid_block`'s body.

Together they are `Rchain.System.persistent_blocks_the_goal`: from a state that holds the refusal, **no**
reachable state admits a block above it. The node is `Terminal`, not slow — which is the difference
between a repair that waits and a repair that needs an inverse.

**What this module does not model, stated because it is what the fix needs.** The restoring rule — a
revalidation of failed metadata, or a bounded re-fetch — does not exist in the port, and when it lands
`the_refusal_is_persistent` becomes false *by construction*: this theorem is a **guard on that fix**, and
the register's row says so, so that the next reader does not read the red as a regression.
-/

namespace Rchain

/-- A node's record of the blocks it has refused, by the sender whose block it refused. The port keeps
    this in the DAG's metadata (`BlockMetadata::validation_failed`, set by `mark_failed` for every
    `ValidateError::ValidationFailed` and by `mark_failed_attributable` when the failure is the block's
    own fault). -/
structure Strand where
  failed : List Nat
deriving DecidableEq

/-- **The refusal step**: the node marks one more sender's block failed. Nothing in the modelled rule set
    unmarks one — read off the port rather than assumed, and the subject of this law: `mark_failed`
    records, and no rule clears. -/
def strandStep (s s' : Strand) : Prop := ∃ v, s'.failed = v :: s.failed

/-- The transition system this law is stated over. `enabled` is `true` everywhere — a node can always
    refuse another block — and the interface field is discharged by construction. -/
def strandSystem : System where
  State := Strand
  Step := strandStep
  enabled := fun _ => true
  enabled_iff := by
    intro s
    exact ⟨fun _ => ⟨⟨0 :: s.failed⟩, 0, rfl⟩, fun _ => rfl⟩

/-- **The refusal persists.** At least one attribution frees nobody: once a sender's block has been marked
    failed it stays failed, because no step of the modelled rule set unmarks one. This is the amplifier
    C173 found, and it is the port's own sense of *absorbing* (`Rchain.an_abort_is_absorbing`): the node is
    not slow to forgive, it cannot. -/
theorem the_refusal_is_persistent (v : Nat) :
    strandSystem.Persistent (fun s : Strand => v ∈ s.failed) := by
  rintro s s' ⟨w, hw⟩ hv
  rw [hw]
  exact List.mem_cons_of_mem w hv

/-- **The rule**, on `Rchain.Casper.Validate`'s model of a block: a block that justifies a failed
    **bonded** sender's block is refused. That is `neglected_invalid_block`
    (`casper/src/validate.rs:321-340`), and it fires before any other rule runs — which is what makes it
    the structural half of the estrangement rather than one rule among several. -/
def neglects (bonded : List Nat) (b : Block) : Prop :=
  ∃ p ∈ b.justifications, p.validationFailed ∧ p.sender ∈ bonded

/-- **The rule is not vacuous**, on the model's own fixture: a block justifying a failed bonded sender's
    block is neglected, and one justifying a failed *unbonded* sender's is not — the port's own contrast,
    and the reason the reachable route to a failed parent is the unbonded one. -/
theorem a_neglected_block_is_detected :
    neglects [1] ⟨10, 0, 0, [⟨1, 9, 0, true⟩], 0, []⟩ ∧
      ¬ neglects [1] ⟨10, 0, 0, [⟨2, 9, 0, true⟩], 0, []⟩ := by
  constructor
  · exact ⟨⟨1, 9, 0, true⟩, by simp, by simp⟩
  · rintro ⟨p, hp, _hf, hb⟩
    simp only [List.mem_singleton] at hp
    subst hp
    simp at hb

/-- **…so a refused validator is never followed again.** The persistence keeps the refusal on every
    reachable state, the rule refuses any block above it, and the combination is `Terminal`: there is no
    reachable state in which a descendant of the refused block is admitted. The `hrule` hypothesis is the
    rule itself, named rather than assumed — which is the honest boundary of this clause, and the place a
    restoring rule would have to intervene. -/
theorem a_refused_validator_is_never_followed (v : Nat) (s : Strand) (hs : v ∈ s.failed)
    (above : Strand → Prop) (hrule : ∀ s, v ∈ s.failed → ¬ above s) :
    ¬ ∃ s', strandSystem.Reach s s' ∧ above s' :=
  strandSystem.persistent_blocks_the_goal (the_refusal_is_persistent v) hrule hs

end Rchain
