import Rchain.Casper.Stake

/-!
# Laws 44–47 — Proof-of-Stake: the epoch gate, the reward split, and the dust

The port's native PoS state (`rholang/src/native_state.rs`) implements the validator lifecycle — bond,
withdraw, slash, trust — and **takes epoch transitions immediately**: `close_block` refunds quarantined
withdrawers and recomputes `pos:active` on every block, never reading `PosParams.epoch_length`. The Scala
contract it replaces (`legacy/casper/src/main/resources/Pos.rhox`) gates the whole of that on
`blockNumber % epochLength == 0` (:517), computes an epoch's rewards at the boundary
(`getCurrentEpochRewards`, :241-256), commits them (`commitCurrentEpochRewards`, :568-576), and pays out
only the withdrawers whose quarantine has expired (:556-567, :592-621).

This module is the **specification first** half of closing that gap (the plan's rule for Programme III):
the rule stated, modelled, and proved where it can be, before the Rust implements it.

## The reward split does not conserve, and the law has to say so

`getCurrentEpochRewards` divides **twice** with integer division:

    reward_i = pot * (bond_i / minimumBond) / (activeBonds / minimumBond)

so `Σ reward_i` is the pot *minus dust* — the sum of the floored shares, re-floored. A conservation law
written as an equality would be **false**, and the Scala's own comment does not say which it means. The
model settles it: `list_sum_div_le` and `sum_rewards_le_pot` below are the inequality, and the instance
at the end is a case where it is *strict* — which is what makes the statement falsifiable rather than a
restatement of an accounting identity.
-/

/-- The sum of a list of naturals. Defined here rather than imported: this file is about the arithmetic
of one fold, and Mathlib's big-operators import would pull a tree the spec does not otherwise need. -/
def nsum : List Nat → Nat
  | [] => 0
  | a :: rest => a + nsum rest

namespace Rchain

/-- The distributable pot of an epoch: `posBalance - totalBond - totalWithdraw - totalCommittedRewards`
(`Pos.rhox:241-256`). The Scala computes it in `Long` and the contract's guards keep it non-negative, so
the model is a `Nat` subtraction — the floor is the *stated* part, not an artefact of the type. -/
def rewardPot (posBalance totalBond totalWithdraw committed : Nat) : Nat :=
  posBalance - totalBond - totalWithdraw - committed

/-- One active validator's epoch reward, exactly as `getCurrentEpochRewards` divides it: the pot scaled
by the validator's bond over `minimumBond`, normalised by the active set's bond over `minimumBond`. -/
def reward (pot minimumBond activeBonds bond : Nat) : Nat :=
  pot * (bond / minimumBond) / (activeBonds / minimumBond)

/-- **The integer division that makes the split conservative**: `⌊a/m⌋ + ⌊b/m⌋ ≤ ⌊(a+b)/m⌋`. Two
floors never add up to more than the floor of the sum, which is why the reward formula leaves a
remainder rather than distributing the pot exactly. -/
theorem div_add_div_le (m a b : Nat) : a / m + b / m ≤ (a + b) / m := by
  rcases Nat.eq_zero_or_pos m with rfl | hm
  · simp
  -- Two floors add up to at most the floor of the sum: each division drops a remainder, and dropping
  -- two of them can only lose. Stated through `div_mul_le_self` rather than left to `omega`, because
  -- `(a / m) * m` is a product of two terms and `omega` is linear in its variables.
  · rw [Nat.le_div_iff_mul_le hm, Nat.add_mul]
    exact Nat.add_le_add (Nat.div_mul_le_self a m) (Nat.div_mul_le_self b m)

/-- The same fact over a list: the sum of the floored shares never exceeds the floor of the sum. The
induction is `div_add_div_le` once per element. -/
theorem list_sum_div_le (m : Nat) : ∀ (l : List Nat), nsum (l.map (· / m)) ≤ nsum l / m
  | [] => by simp [nsum]
  | a :: rest => by
    have ih := list_sum_div_le m rest
    have step := div_add_div_le m a (nsum rest)
    simp only [List.map_cons, nsum] at ih ⊢
    omega

/-- `Σ (pot * x_i) = pot * Σ x_i` — the factor the split multiplies each share by. -/
theorem nsum_map_mul_left (pot : Nat) : ∀ (l : List Nat), nsum (l.map (fun x => pot * x)) = pot * nsum l
  | [] => by simp [nsum]
  | a :: rest => by
    simp only [List.map_cons, nsum, nsum_map_mul_left pot rest]
    rw [Nat.mul_add]

/-- **Law 46 — the epoch's split never pays out more than the pot**, and the remainder is the dust of two
integer divisions. Stated over the list of active bonds rather than a map, because the formula only ever
reads each bond through `bond / minimumBond`.

The hypotheses are the contract's own: the active set's bond total is the sum of its members'
(`hactive`), and the normaliser is positive (`hD`) — which is the only hypothesis the proof needs, and
it is the one that carries `minimumBond > 0` with it: `activeBonds / minimumBond` is positive only if the
divisor is, so a `minimumBond` of zero is already excluded. `activeBonds / minimumBond = 0` would be a
contract that pays every validator nothing, which is the shape the hypothesis refuses. -/
theorem sum_rewards_le_pot (pot minimumBond activeBonds : Nat) (bonds : List Nat)
    (hactive : activeBonds = nsum bonds) (hD : 0 < activeBonds / minimumBond) :
    nsum (bonds.map (fun b => reward pot minimumBond activeBonds b)) ≤ pot := by
  have hstep :
      nsum (bonds.map (fun b => reward pot minimumBond activeBonds b))
        ≤ nsum (bonds.map (fun b => pot * (b / minimumBond))) / (activeBonds / minimumBond) := by
    simpa only [reward, List.map_map, Function.comp_def] using
      list_sum_div_le (activeBonds / minimumBond) (bonds.map (fun b => pot * (b / minimumBond)))
  have hsum : nsum (bonds.map (fun b => pot * (b / minimumBond)))
      = pot * nsum (bonds.map (fun b => b / minimumBond)) := by
    simpa only [List.map_map, Function.comp_def] using
      nsum_map_mul_left pot (bonds.map (fun b => b / minimumBond))
  have hfits : nsum (bonds.map (fun b => b / minimumBond)) ≤ activeBonds / minimumBond := by
    rw [hactive]
    exact list_sum_div_le minimumBond bonds
  have hmono : pot * nsum (bonds.map (fun b => b / minimumBond)) / (activeBonds / minimumBond)
      ≤ pot * (activeBonds / minimumBond) / (activeBonds / minimumBond) :=
    Nat.div_le_div_right (Nat.mul_le_mul_left pot hfits)
  calc nsum (bonds.map (fun b => reward pot minimumBond activeBonds b))
      ≤ nsum (bonds.map (fun b => pot * (b / minimumBond))) / (activeBonds / minimumBond) := hstep
    _ = pot * nsum (bonds.map (fun b => b / minimumBond)) / (activeBonds / minimumBond) := by rw [hsum]
    _ ≤ pot * (activeBonds / minimumBond) / (activeBonds / minimumBond) := hmono
    _ = pot := Nat.mul_div_cancel _ hD

/-- **The inequality is strict, so it is not an accounting identity in disguise.** `minimumBond = 3`,
bonds `[4, 5]`: the normaliser is `9 / 3 = 3`, each validator's scaled share is `4/3 = 1` and `5/3 = 1`,
so with a pot of ten each is paid `10 * 1 / 3 = 3` and the epoch distributes **6 of 10** — four units of
dust, which is the whole content of law 46. A statement that said `Σ = pot` would be refuted by this
line. -/
theorem the_dust_is_real :
    reward 10 3 9 4 + reward 10 3 9 5 = 6 ∧ (6 : Nat) < 10 := by decide

end Rchain
