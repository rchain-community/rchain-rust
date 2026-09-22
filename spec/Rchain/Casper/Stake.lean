import Mathlib.Data.List.Basic

/-!
# Law 14 — finality requires > 2/3 bonded stake

A fringe finalizes iff the stake supporting it is a strict supermajority (`> 2/3`). The Scala oracle
is `sdk/consensus/Stake.scala:8`; the Rust realization is `sdk::consensus::is_super_majority` with the
exact integer comparison `3·stake > 2·total` (no floating-point precision loss).
-/

namespace Rchain

/-- A bonded validator. -/
structure Validator where
  id : Nat

/-- A bond: a validator and its (non-negative) stake. -/
structure Bond where
  validator : Validator
  stake : Nat

/-- The supermajority test: `stake` is strictly more than two thirds of `total` — the exact-integer
    spelling `3·stake > 2·total` (Law 14), not the lossy `stake/total > 2/3`. -/
@[reducible] def isSuperMajority (stake total : Nat) : Prop := 3 * stake > 2 * total

/-! ## The gate the finalizer actually runs

The axiom that stood here — `isSuperMajority s t ↔ s * 3 > t * 2` — was `Nat.mul_comm` twice: it restated
the definition's own body, which is why the register called the row `vacuous`. What the port has is a
*gate*, and the law is what that gate computes: the supporting stake is the bonded stake whose senders saw
the full partition, and the fringe advances exactly when that is a supermajority
(`block-storage/src/dag/finalizer.rs:153-171`, `:174-197`, `:202-215`). -/

/-- A validator identifier (the port's `S`). -/
abbrev Sender := Nat

/-- The bonds map: each bonded sender's stake (`BTreeMap<S, NonNegI64>`, `finalizer.rs:156`). -/
abbrev Bonds := List (Sender × Nat)

/-- The support map `calculate_fringe` is called with: for each candidate sender, the senders that saw it,
    each mapped to the set of senders *that* one saw — the port's
    `BTreeMap<S, BTreeMap<S, BTreeSet<S>>>` (`finalizer.rs:155`). -/
abbrev SupportMap := List (Sender × List (Sender × List Sender))

/-- The bonded senders — the port's `bonds_map.keys()` (`finalizer.rs:157`). -/
def bondedSenders (bonds : Bonds) : List Sender := bonds.map (·.1)

/-- A sender's stake, if it is bonded — the port's `bonds_map.get(sender)`. -/
def stakeOf (bonds : Bonds) (s : Sender) : Option Nat :=
  (bonds.find? (fun p => p.1 == s)).map (·.2)

theorem find?_eq_none_of_not_mem (bonds : Bonds) (s : Sender) (h : s ∉ bondedSenders bonds) :
    bonds.find? (fun p => p.1 == s) = none := by
  rw [List.find?_eq_none]
  intro x hx hbeq
  exact h (List.mem_map.mpr ⟨x, hx, (beq_iff_eq x.1 s).mp hbeq⟩)

/-- **A non-bonded sender has no stake to contribute.** This is the port's `if let Some(stake) =
    bonds_map.get(sender)` branch — the one that keeps a non-bonded justification sender from indexing
    the bonds map at all, which the code's own `calculate_fringe_ignores_non_bonded_sender`
    (`block-storage/src/dag/finalizer.rs:282`) pins. -/
theorem stakeOf_eq_none (bonds : Bonds) (s : Sender) (h : s ∉ bondedSenders bonds) :
    stakeOf bonds s = none := by
  simp [stakeOf, find?_eq_none_of_not_mem bonds s h]

/-- Every seer saw the whole bonded set — the port's
    `!seen_by.is_empty() && seen_by.values().all(|v| v == &bonded_senders)` (`finalizer.rs:161`). -/
def allBonded (bonded : List Sender) (seenBy : List (Sender × List Sender)) : Bool :=
  !seenBy.isEmpty && seenBy.all (fun p => p.2 == bonded)

/-- The senders whose full-partition support is recorded — the accumulation the port runs before the
    stake lookup (`finalizer.rs:158-168`). -/
def bondedSupport (supp : SupportMap) (bonded : List Sender) : List Sender :=
  (supp.filter (fun p => allBonded bonded p.2)).map (·.1)

/-- **The stake supporting the candidate fringe**: the bonded stake whose senders saw the full partition.
    A sender with no bond contributes nothing (`stakeOf_eq_none`). -/
def fullPartitionStake (supp : SupportMap) (bonds : Bonds) : Nat :=
  ((bondedSupport supp (bondedSenders bonds)).filterMap (fun s => stakeOf bonds s)).foldr
    (fun n acc => n + acc) 0

/-- The total bonded stake — summed exactly in the model. The port sums in `i128` for the same reason
    (`finalizer.rs:170`): a validator set can exceed `i64` before this comparison is reached
    (`sdk/src/consensus.rs:50`). -/
def totalStake (bonds : Bonds) : Nat := (bonds.map (·.2)).foldr (fun n acc => n + acc) 0

/-- **`calculate_fringe`** (`block-storage/src/dag/finalizer.rs:153-171`), as the model computes it. -/
def calculateFringe (supp : SupportMap) (bonds : Bonds) : Bool :=
  decide (isSuperMajority (fullPartitionStake supp bonds) (totalStake bonds))

/-! ### The boundary, which is where this law's content is

An `↔` whose shape is the gate's own `if` says little; what the gate *computes* is what these pin, and
each names the port's own test. -/

/-- Exactly two thirds is **not** a supermajority (`sdk/src/consensus.rs:24`). -/
theorem two_thirds_is_not_supermajority : ¬ isSuperMajority 2 3 := by decide

/-- Three quarters is (`sdk/src/consensus.rs:30`). -/
theorem above_two_thirds_is_supermajority : isSuperMajority 3 4 := by decide

/-- One third is not (`sdk/src/consensus.rs:35`). -/
theorem below_two_thirds_is_not_supermajority : ¬ isSuperMajority 1 3 := by decide

/-- **The boundary the exact form buys.** `stake = 2·2⁵³ + 1`, `total = 3·2⁵³` is just above two thirds —
    and the Scala oracle's `stake.toDouble / totalStake > 2d / 3` cannot represent `2·2⁵³+1` (it rounds to
    `2·2⁵³`) and so misclassifies it (`sdk/src/consensus.rs:40`). -/
theorem large_stake_just_above_two_thirds_is_exact :
    isSuperMajority (2 * 2 ^ 53 + 1) (3 * 2 ^ 53) := by decide

/-- A validator set whose bonded stake exceeds `i64::MAX` still compares exactly
    (`sdk/src/consensus.rs:50`). The model's `Nat` cannot wrap at all, so here it is *stronger* than
    the code — stated so the difference is visible rather than assumed; the port's `i128` is what buys
    the same property there. -/
theorem i64_overflowing_stakes_do_not_wrap :
    isSuperMajority (2 * (2 ^ 63 - 1) + 1) (3 * (2 ^ 63 - 1)) := by decide

end Rchain
