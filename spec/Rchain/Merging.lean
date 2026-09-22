import Rchain.Cmp
import Rchain.Crypto.Random

/-!
# Law 17 — the merge arithmetic, and the RNG merge at its call site

The port's numeric-channel merge lives in `rholang/src/merging.rs`: `calculate_number_channel_merge`
(`:80-132`) and `calculate_num_channel_diff` (`:299-321`). Both do their arithmetic with the **checked**
`i64` operations, so a value that would leave the signed 64-bit range is *refused* rather than wrapped —
`checked_add` with the message "number channel merge overflow", `checked_sub` with "number channel diff
overflow". This module models exactly those two paths, and nothing else, because the law the catalogue
was carrying here — "numeric channels are non-negative" — was **false of the code**: numeric channels are
signed `i64` and negative diffs are ordinary (`merging.rs:161-166`; the tests at `:349,370` use
`diff: -5` and `-10`). `NonNegI64` is a real newtype in `shared/src/refined.rs:64`, but it types *bonds*
and *heights*, not numeric channels, and the old law conflated them.

## What one of the two paths does *not* do, which the law must therefore not claim

The merge refuses to wrap; the diff *accumulator* does not. Combining two branch index maps adds with a
plain `i64 +=`:

    rspace/src/merger/event_log_index.rs:151      *number_channels.entry(*k).or_insert(0) += *v;
    casper/src/merging.rs:758                     *mergeable_diffs.entry(*k).or_insert(0) += v;

so at the accumulator a debug build panics and a release build wraps. Nothing refuses that addition.
That is a finding about the code rather than a theorem about a model, so it is recorded in
`spec/AUDIT.md` and named in the register's Law 17 row — this module does not dress it up as a law.

## The RNG merge, and why there is no commutativity axiom here

The catalogue used to carry `mergeRandom_comm`. It was false: the primitive's Rust signature is
`Blake2b512Random::merge(children: &[Self])` — n-ary, and order-*sensitive*, pinned by the primitive's
own test (`crypto/src/hash/blake2b512_random.rs:548`, `merge_is_order_sensitive`).

But the *law* the axiom was reaching for is true where it matters: the merge's only caller canonicalizes
first. `merging.rs:121-124` dedups the branch generators and **sorts them by their bytes** before
merging:

    randoms.retain(|r| seen.insert(r.to_bytes()));
    randoms.sort_by_key(|r| r.to_bytes());
    Blake2b512Random::merge(&randoms)

So the merged state is a function of the *set* of branch generators, and `mergeRandoms_perm` below is
that fact proved from `Rchain/Cmp.lean`'s `sortList_perm` — the same sort-then-hash shape as Law 1.
-/

namespace Rchain

/-! ## The checked `i64` arithmetic -/

/-- The signed 64-bit lower bound, as the kernel's `i64::MIN`. -/
def i64Min : Int := -9223372036854775808

/-- The signed 64-bit upper bound, as the kernel's `i64::MAX`. -/
def i64Max : Int := 9223372036854775807

/-- The port's `checked_add`: `none` when the sum leaves the signed 64-bit range, which is how
`calculate_number_channel_merge` refuses to wrap (`rholang/src/merging.rs:102-104`). -/
def checkedAdd (a b : Int) : Option Int :=
  if i64Min ≤ a + b ∧ a + b ≤ i64Max then some (a + b) else none

/-- The port's `checked_sub`, from `calculate_num_channel_diff` (`rholang/src/merging.rs:309-311`). -/
def checkedSub (a b : Int) : Option Int :=
  if i64Min ≤ a - b ∧ a - b ≤ i64Max then some (a - b) else none

/-- The merge **refuses** a sum that leaves `i64` instead of wrapping — the code's
`checked_add(...).ok_or("number channel merge overflow")`. A witness rather than a remark: a
`checkedAdd` that wrapped would satisfy nothing here. -/
theorem checkedAdd_refuses_overflow :
    checkedAdd i64Max 1 = none ∧ checkedAdd i64Min (-1) = none := by
  constructor <;> decide

/-- The same, in the other direction, for the diff computation (`checked_sub`). -/
theorem checkedSub_refuses_overflow :
    checkedSub i64Min 1 = none ∧ checkedSub i64Max (-1) = none := by
  constructor <;> decide

/-- The arithmetic the two functions rely on, in range: the diff and the merge are inverse. Stated over
the kernel's bounds rather than in the abstract, so it is a claim about `checkedAdd`/`checkedSub` and not
about `Int` arithmetic in general. -/
theorem merge_diff_round_trip (init e : Int)
    (hsub : i64Min ≤ e - init ∧ e - init ≤ i64Max)
    (hadd : i64Min ≤ init + (e - init) ∧ init + (e - init) ≤ i64Max) :
    (checkedSub e init).bind (checkedAdd init) = some e := by
  unfold checkedSub
  rw [if_pos hsub]
  simp only [Option.some_bind]
  unfold checkedAdd
  rw [if_pos hadd]
  congr 1
  omega

/-! ## The RNG merge at its call site -/

/-- Merge the branch random generators the way `calculate_number_channel_merge` does: canonicalize
(dedup and sort by bytes) and *then* merge. `Random.state` models the generator's `to_bytes`, so sorting
by `state` is the code's `sort_by_key(|r| r.to_bytes())`.

`noncomputable` because it calls `mergeRandom`, which is an axiom — a modelled primitive has no compiled
code, and this is a definition in the specification rather than something the node executes. -/
noncomputable def mergeRandoms (rs : List Random) : Random :=
  mergeRandom
    ((Comparator.sortList (Comparator.linearOrderComparator Nat) (rs.map (·.state))).eraseDups
      |>.map Random.mk)

/-- **The order-independence the old `mergeRandom_comm` axiom claimed, proved where it is true.** The
primitive is order-sensitive; its caller sorts, so the merged state depends on the *set* of branch
generators and not on the order they arrived in — `sortList_perm` (`Rchain/Cmp.lean:228`) plus
`eraseDups`. -/
theorem mergeRandoms_perm (rs ss : List Random) (h : List.Perm rs ss) :
    mergeRandoms rs = mergeRandoms ss := by
  simp only [mergeRandoms]
  exact congrArg (fun l => mergeRandom (l.eraseDups.map Random.mk))
    (Comparator.sortList_perm (Comparator.linearOrderComparator Nat)
      (List.Perm.map Random.state h))

end Rchain
