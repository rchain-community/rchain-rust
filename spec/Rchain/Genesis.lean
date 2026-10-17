import Rchain.Pos

/-!
# Law 78 — the developer allocation

**Why this module exists.** The genesis installs one vault that no wallets file contains: an allocation
to the developer, of `max(20 000 000 REV, 2% of the allocated total)`. It is a **deliberate deviation
from the Scala oracle**, and it is consensus state — the vault balances are written outside the genesis
block's deploys and content-addressed into its post-state hash, so a node that derives a different
amount computes a different hash and refuses the block (AUDIT C46). A number that decides state, and
that the oracle does not have, is exactly what the register exists to carry: this module states the
rule, and `casper/src/genesis/dev_tax.rs` is the Rust the law is tied to.

**What the law says.** The allocation is the **larger** of a floor and a rate. It is therefore a claim
about where the arms cross: `2%` reaches `20 000 000 REV` at exactly one billion REV, so below that
allocation the floor governs and above it the rate does. That boundary is the load-bearing statement —
`devTaxAmount_eq_floor_of_le_crossover` and `devTaxAmount_eq_percentage_of_crossover_le` are the two
directions, and on the network this is being written for (908 962 714 REV allocated, dated 13 May 2026)
the floor is the arm that applies.

**What it does not claim.** Nothing here is about the allocation being *just*, or about where the floor
and the rate came from: they are the decision this row records. The law is about the arithmetic being
one function of the allocated total, so that the creator of a genesis and a node replaying it cannot
disagree about an amount.
-/

namespace Rchain

/-- Drops per REV, the unit a vault balance is stored in (`casper/src/genesis/dev_tax.rs`). -/
def dropsPerRev : Nat := 100000000

/-- The floor of the allocation, in REV. -/
def devTaxFloorRev : Nat := 20000000

/-- The floor, in drops. -/
def devTaxFloor : Nat := devTaxFloorRev * dropsPerRev

/-- The rate, in basis points (200 = 2%), on the `executor_share` convention. -/
def devTaxBps : Nat := 200

/-- The basis-point denominator. -/
def bpsDenominator : Nat := 10000

/-- The allocated total at which the rate first reaches the floor — one billion REV, in drops. -/
def devTaxCrossover : Nat := devTaxFloor * bpsDenominator / devTaxBps

/-- **The rule**: the allocation owed on an allocated total of `total` drops, as the larger of the
floor and the rate. Truncating division, so the rounding dust stays with the network. -/
def devTaxAmount (total : Nat) : Nat :=
  max devTaxFloor (total * devTaxBps / bpsDenominator)

/-- The floor is never undershot, at any allocated total. -/
theorem devTaxAmount_ge_floor (total : Nat) : devTaxFloor ≤ devTaxAmount total :=
  le_max_left _ _

/-- The crossover is where the rate first reaches the floor: 10 000 divides by 200 exactly, so it is
`50 ×` the floor and there is no truncation to reason about at the boundary. -/
theorem devTaxCrossover_eq : devTaxCrossover = devTaxFloor * 50 := by
  decide

/-- **Below the crossover the floor governs.** -/
theorem devTaxAmount_eq_floor_of_le_crossover (total : Nat) (h : total ≤ devTaxCrossover) :
    devTaxAmount total = devTaxFloor := by
  unfold devTaxAmount
  rw [max_eq_left]
  have h' : total ≤ devTaxFloor * 50 := by rw [← devTaxCrossover_eq]; exact h
  have hmul : total * 200 ≤ 10000 * devTaxFloor := by
    rw [Nat.mul_comm 10000 devTaxFloor]
    exact Nat.le_trans (Nat.mul_le_mul_right 200 h') (by decide)
  simp only [devTaxBps, bpsDenominator]
  exact Nat.div_le_of_le_mul hmul

/-- **At and above the crossover the rate governs.** -/
theorem devTaxAmount_eq_percentage_of_crossover_le (total : Nat) (h : devTaxCrossover ≤ total) :
    devTaxAmount total = total * devTaxBps / bpsDenominator := by
  unfold devTaxAmount
  rw [max_eq_right]
  have h' : devTaxFloor * 50 ≤ total := by rw [← devTaxCrossover_eq]; exact h
  have hmul : devTaxFloor * 10000 ≤ total * 200 :=
    Nat.le_trans (by decide : devTaxFloor * 10000 ≤ (devTaxFloor * 50) * 200)
      (Nat.mul_le_mul_right 200 h')
  simp only [devTaxBps, bpsDenominator]
  exact (Nat.le_div_iff_mul_le (by decide : 0 < 10000)).mpr hmul

/-- **Non-vacuity**: above the crossover the rate is *strictly* larger than the floor, so the `max` is
doing work and the rule is not a constant dressed as one. Without this the two theorems above would
hold of any floor. -/
theorem the_rate_strictly_exceeds_the_floor_above_the_crossover :
    devTaxFloor < devTaxAmount (2 * devTaxCrossover) := by
  have h : devTaxCrossover ≤ 2 * devTaxCrossover := by omega
  rw [devTaxAmount_eq_percentage_of_crossover_le _ h]
  simp only [devTaxBps, bpsDenominator, devTaxFloor, devTaxFloorRev, dropsPerRev, devTaxCrossover]
  decide

/-- The allocation is monotone in the allocated total: a larger allocation never owes less. -/
theorem devTaxAmount_monotone {a b : Nat} (h : a ≤ b) : devTaxAmount a ≤ devTaxAmount b := by
  unfold devTaxAmount
  apply max_le
  · exact le_max_left _ _
  · exact le_trans (Nat.div_le_div_right (Nat.mul_le_mul_right _ h)) (le_max_right _ _)

/-- **The allocation on the allocation this network has** (908 962 714 REV, dated 13 May 2026): 2% of it
is 18 179 254 REV, under the floor, so the allocation is the floor — 20 000 000 REV. -/
theorem devTaxAmount_of_the_published_allocation :
    devTaxAmount 90896271400000000 = devTaxFloor := by
  rw [devTaxAmount_eq_floor_of_le_crossover _ (by decide)]

end Rchain
