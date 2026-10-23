import Rchain.Pos

/-!
# Law 79 — the developer allocation unlocks on a schedule

**Why this module exists.** Law 78 fixes *how much* the genesis owes the developer. This one fixes
*when it becomes spendable*, because an allocation credited to a vault at genesis is spendable at
genesis — an immediate claim on the whole amount, which is the shape the review called a sell overhang.
So the allocation is credited to a **vesting account** instead, and REV moves from it to the recipient's
own vault as it vests.

**What the law says.** The vested fraction is the schedule position: linear in the height, truncated
toward the network so the released amount never runs ahead of the schedule, zero at genesis and total at
the end. A duration of `0` is *no schedule at all* rather than a zero-length one — the whole amount is
vested from the start — which is what a params record written before the field existed decodes to.

**What it does not claim.** Nothing here is about the term being right. Thirty-six months linear is a
decision, recorded in `defaults.conf` with the cadence it assumes; the arithmetic is what this module
states, and it is the only part a machine can check.
-/

namespace Rchain

/-- **The schedule**: how much of `allocation` has vested at `height`, over `duration` blocks from
genesis. Linear, no cliff; `duration = 0` is no schedule at all. -/
def devTaxVested (allocation duration height : Nat) : Nat :=
  if duration = 0 then allocation
  else if duration ≤ height then allocation
  else allocation * height / duration

/-- **No schedule means the whole amount is vested from the start** — the rule for a chain that wants
the allocation spendable at its first boundary, and the value a params record without the field
decodes to. -/
theorem devTaxVested_zero_duration (allocation height : Nat) :
    devTaxVested allocation 0 height = allocation := by
  simp [devTaxVested]

/-- **Nothing vests at genesis.** The account holds the whole allocation at height 0, which is what
makes the balance in the genesis post-state the allocation itself rather than a released fraction. -/
theorem devTaxVested_at_genesis (allocation duration : Nat) (hd : 0 < duration) :
    devTaxVested allocation duration 0 = 0 := by
  unfold devTaxVested
  have h0 : ¬duration = 0 := by omega
  rw [if_neg h0]
  have hle : ¬duration ≤ 0 := by omega
  rw [if_neg hle]
  simp

/-- **At the end of the schedule the whole allocation is vested**, and it stays there: the two arms
below agree at `height = duration`, which is the boundary the release depends on. -/
theorem devTaxVested_at_end (allocation duration : Nat) :
    devTaxVested allocation duration duration = allocation := by
  unfold devTaxVested
  by_cases hd : duration = 0
  · simp [hd]
  · rw [if_neg hd]
    rw [if_pos (Nat.le_refl duration)]

/-- **The schedule never vests more than the allocation.** The load-bearing half of the pair: with the
release moving `vested − released`, this is what bounds the recipient's vault by the allocation itself,
however long the chain runs. -/
theorem devTaxVested_le_allocation (allocation duration height : Nat) :
    devTaxVested allocation duration height ≤ allocation := by
  unfold devTaxVested
  by_cases hd : duration = 0
  · simp [hd]
  · rw [if_neg hd]
    by_cases hle : duration ≤ height
    · simp [hle]
    · rw [if_neg hle]
      have hlt : height < duration := Nat.lt_of_not_le hle
      have hmul : allocation * height ≤ duration * allocation := by
        rw [Nat.mul_comm duration allocation]
        exact Nat.mul_le_mul_left allocation (Nat.le_of_lt hlt)
      exact Nat.div_le_of_le_mul hmul

/-- **The schedule is monotone in the height** — a later boundary never vests less — which is what
makes the released amount non-decreasing and the release (a difference of two monotone quantities)
never negative. -/
theorem devTaxVested_monotone (allocation duration a b : Nat) (hab : a ≤ b) :
    devTaxVested allocation duration a ≤ devTaxVested allocation duration b := by
  unfold devTaxVested
  by_cases hd : duration = 0
  · simp [hd]
  · rw [if_neg hd, if_neg hd]
    by_cases hb : duration ≤ b
    · -- `b` is at or past the end of the schedule, so its side is the whole allocation and it suffices
      -- that `a`'s side is bounded by it — which is `devTaxVested_le_allocation` itself.
      rw [if_pos hb]
      by_cases ha : duration ≤ a
      · simp [ha]
      · rw [if_neg ha]
        have hlt : a < duration := Nat.lt_of_not_le ha
        have hmul : allocation * a ≤ duration * allocation := by
          rw [Nat.mul_comm duration allocation]
          exact Nat.mul_le_mul_left allocation (Nat.le_of_lt hlt)
        exact Nat.div_le_of_le_mul hmul
    · rw [if_neg hb]
      have ha : ¬duration ≤ a := by omega
      rw [if_neg ha]
      exact Nat.div_le_div_right (Nat.mul_le_mul_left allocation hab)

/-- **Non-vacuity**: the schedule is not a constant. At half its duration an allocation of two units has
vested one — so the theorems above are about a rule rather than about `fun _ => allocation`. -/
theorem the_schedule_is_not_a_constant :
    devTaxVested 2 2 1 = 1 ∧ devTaxVested 2 2 2 = 2 ∧ devTaxVested 2 2 1 ≠ devTaxVested 2 2 2 := by
  decide

/-- **Truncation is toward the network.** Seven units over a three-block schedule vest two at the third
of the way — `7/3` floored, not rounded up — so the released amount is never ahead of the schedule. -/
theorem the_schedule_truncates :
    devTaxVested 7 3 1 = 2 ∧ devTaxVested 7 3 2 = 4 ∧ devTaxVested 7 3 3 = 7 := by
  decide

end Rchain
