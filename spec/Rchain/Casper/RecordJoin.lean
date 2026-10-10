import Rchain.Casper.Record

/-!
# The record join is a commutative idempotent semigroup at one key — and what that buys

`join_fringe_records` (`casper/src/dag.rs:82-108`) is the whole of the write side of the fringe store:
`insert` reads the record already at the key, joins it with the one the block carries, and writes the
result back (`dag.rs:820-850`). Its doc states three laws for that join — commutative, associative,
idempotent — and the consequence it draws from them, *"so every arrival order reaches the same
record"* (`dag.rs:67-68`). This file is the check on that paragraph.
`Rchain/Casper/Record.lean` proves the idempotence (`joinRecord_idempotent`) and refutes the assumption
the join exists to preserve; **here are the other two laws and the consequence**, and — the part that
is worth more than the laws — what the consequence is and is not worth.

## The statement, and where it is not `joinRecord : FringeRecord → FringeRecord → FringeRecord`

The join is a binary operation on records, but it is **not** one the code ever applies to a pair of
arbitrary records: it takes `existing`, read out of the store *by the key* `incoming` was built for.
The code says so itself (`dag.rs:79-80`): "Both arguments are records for one key: the caller reads
`existing` out of the store *by* the key `incoming` was built for, so `fringe` and `fringe_hash` agree
by construction and are taken from the existing record without a check." That precondition is
`AtOneKey` below, and it is exactly what commutativity needs — **associativity is unconditional, and
commutativity is not**, for the reason the code's own shape gives: the join keeps its *left* operand's
`fringe_hash` and `fringe` (`dag.rs:84-85`), so `joinRecord a b` and `joinRecord b a` are different
records whenever `a` and `b` are not for one key. This is not a defect to be repaired by changing the
definition — the left-biased key is what makes `joinRecord a b` a record *for `a`'s* key, which is the
invariant `joinRecord_keeps_the_stored_key` states — and the asymmetry is real: it is exhibited
(`joinRecord_not_commutative`) rather than hidden behind the hypothesis. See the note on
`joinRecord_has_no_identity` for the law a monoid would need and this operation does not have.

**This is the same shape as Law 9** (`Rchain/RSpace/Merge.lean`, the RSpace state-change merge): there
too associativity is structural and commutativity holds *under a side condition* — channel
disjointness there, one store key here — and there too the honest statement names the condition rather
than assuming the operation is commutative on the nose. `mergeChanges`' right-biased join map and
`joinRecord`'s left-biased key are the same phenomenon: one field the merge cannot combine, so it is
settled by *operand order*. Both files therefore keep commutativity under an explicit condition and
both state what the un-combinable field costs when the condition fails — `join_last_wins` (`Merge.lean`
`:103`) there, `joinRecord_not_commutative` here.

## What the laws buy, and what they do not

1. **The value at a key is a function of the arriving set, not of the arriving sequence**
   (`the_joined_value_is_a_function_of_the_set`). This is the contract C215 asks for and the property
   the store's determinism rests on: `merging.rs::rejections_for` (`casper/src/merging.rs:1512-1525`)
   reads these records back, so two nodes holding the same blocks must fold them to the same record
   whatever order the blocks arrived in.
2. **The settlement is by `min`, so the value that comes back need not be one any writer computed**
   (`Rchain/Casper/Record.lean`'s `the_join_rewrites_a_writers_value`). Order-independence is bought
   by the tie-break, and the tie-break is precisely what makes a divergence look like a settled
   value — the mechanism of C270's self-reject stall.
3. **Under the fix, the tie-break is unreachable and the join is the identity**
   (`the_tie_break_is_unreachable`, `no_writer_is_rewritten_at_a_writeKey`,
   `the_join_at_a_writeKey_changes_nothing`): at one `writeKey` the two records a writer can produce
   are the *same record*, so `min` is `min s s` and the read-join-write at `dag.rs:820-850` writes
   back what was already there. That is the whole content of "the key names the base": not a new
   invariant to maintain, but the removal of the only case in which the join could rewrite a value.

## What is *not* claimed

- The laws say the fold is a function of the *set*; they do **not** say the fold agrees with what any
  writer computed. The first bullet above and the stall are consistent: `min` is a function, and the
  function it computes is not the writer's. The theorems that close that gap are the `writeKey` ones,
  and they are about the *key*, not about the join.
- Nothing here is about `Blake2b`: `fringeHashOf` is any function of the fringe
  (`Record.lean:89-90`), so the composite-key result below is independent of the hash's strength, and
  the only thing `AtOneKey`'s second conjunct adds over the code's key is what that model choice
  removes.
-/

namespace Rchain

/-! ## The precondition, and the two records it takes for granted

`joinRecord` copies its left operand's key data and unions the rest, so the record it returns is a
record *for the left operand's* key: `joinRecord_keeps_the_stored_key` (`Record.lean:234`) is `rfl`,
which is why the code can call the result "the record at this key" without checking. The cost is that
the operation is asymmetric unless both operands are, as the code puts it, "records for one key". -/

/-- **Two records the store holds at one key** — the precondition `join_fringe_records` states for
    itself ("Both arguments are records for one key … `fringe` and `fringe_hash` agree by
    construction", `casper/src/dag.rs:79-80`) and the exact hypothesis under which the join is
    symmetric.

    The two conjuncts are the two fields `joinRecord` takes from its left operand (`dag.rs:84-85`),
    so away from this relation the operation is order-sensitive *by construction* — see
    `joinRecord_not_commutative`, which exhibits it rather than leaving the hypothesis looking like a
    convenience.

    It is deliberately **not** `key a = key b` (`Record.lean:156`). In the node equal keys do give
    equal fringes, because the key is `fringe_hash_of(fringe)` (`models/src/fringe_data.rs:39-42`) —
    but that is a fact about Blake2b, and `fringeHashOf` is modelled as *any* function of the fringe
    (`Record.lean:89-90`) precisely so that `the_fringe_only_key_is_incomplete` does not rest on the
    hash's strength. `AtOneKey` is therefore stated at the granularity of the fields the code copies,
    and `atOneKey_implies_key_eq` recovers the weaker statement. -/
def AtOneKey (a b : FringeRecord) : Prop := a.fringeHash = b.fringeHash ∧ a.fringe = b.fringe

/-- The relation is symmetric, so "the arguments are for one key" does not privilege an order — which
    is what lets it be used as a hypothesis both ways round in `joinRecord_comm`. -/
theorem atOneKey_symm {a b : FringeRecord} (h : AtOneKey a b) : AtOneKey b a :=
  ⟨h.1.symm, h.2.symm⟩

/-- And transitive, so "agreeing with one record" is a property of a whole key class — all the
    `perm`-invariance below needs is that every element of the arriving list agrees with the seed. -/
theorem atOneKey_trans {a b c : FringeRecord} (hab : AtOneKey a b) (hbc : AtOneKey b c) :
    AtOneKey a c :=
  ⟨hab.1.trans hbc.1, hab.2.trans hbc.2⟩

/-- Two records for one key are for one key with each other — the form of the hypothesis the join's
    commutativity actually consumes, since the arriving records are known to agree with the *stored*
    one (`dag.rs:79-80`) and not with each other. -/
theorem atOneKey_of_common_base {a b s : FringeRecord} (ha : AtOneKey a s) (hb : AtOneKey b s) :
    AtOneKey a b :=
  atOneKey_trans ha (atOneKey_symm hb)

/-- `AtOneKey` implies what the code's key can see. The converse is false, and that is
    `the_fringe_only_key_is_incomplete` (`Record.lean:173`) — the two records it exhibits share a
    `fringe_hash` (and a fringe) and differ in `stateHash`, so the store's key is blind to a
    difference this relation keeps. -/
theorem atOneKey_implies_key_eq {a b : FringeRecord} (h : AtOneKey a b) : key a = key b := h.1

/-- The structure's extensionality, so that the laws below can be proved field by field. Only the
    seven stored fields: the record carries nothing else, which is the point of `Record.lean`'s
    header (the base the value depends on is deliberately not one of them). -/
@[ext] theorem FringeRecord.ext {a b : FringeRecord} (hfringeHash : a.fringeHash = b.fringeHash)
    (hfringe : a.fringe = b.fringe) (hfringeDiff : a.fringeDiff = b.fringeDiff)
    (hstateHash : a.stateHash = b.stateHash) (hrejectedDeploys : a.rejectedDeploys = b.rejectedDeploys)
    (hrejectedBlocks : a.rejectedBlocks = b.rejectedBlocks)
    (hrejectedSenders : a.rejectedSenders = b.rejectedSenders) : a = b := by
  cases a
  cases b
  simp_all

/-! ## The laws

The doc's three: commutativity and associativity below, idempotence already in `Record.lean`
(`joinRecord_idempotent`, `:258`) and not re-proved. The fourth thing an operation needs to be called
a monoid is an identity, and there is none — `joinRecord_has_no_identity` — because the difference
between "the three laws" and "a monoid" is the difference between what the code relies on and what its
doc's word *monoid* would claim. -/

/-- **Law 1 — associativity, unconditional.** Folding three records in either grouping reaches the
    same record, because every component is a `⊔` of one kind or another: the sets are `Finset.union`
    (`Finset.union_assoc`), `state_hash` is `Nat.min` (`Nat.min_assoc`), and the key data is the
    *leftmost* operand's in both groupings (`joinRecord` copies `stored.fringeHash`/`stored.fringe`
    from its first argument, `dag.rs:84-85`). Nothing about the operands is assumed, and this is the
    law that a fold needs: the `foldr` below is well defined at every step for that reason alone.

    The Rust's claim of the same law is exercised by `casper/tests/merge_determinism.rs` and the
    conformance harness, but this is where it is a theorem: the code's doc says "commutative,
    associative and idempotent" and does not say *under what conditions*, and the answer for the
    middle one is "always" and for the first "only at one key". -/
theorem joinRecord_assoc (a b c : FringeRecord) :
    joinRecord (joinRecord a b) c = joinRecord a (joinRecord b c) := by
  apply FringeRecord.ext <;>
    simp [joinRecord, Finset.union_assoc, Nat.min_assoc]

/-- **Law 2 — commutativity, at one key.** For two records the store holds at one key, the join is
    symmetric: the fields it copies from its left operand are the very fields the hypothesis says the
    right operand shares, and every other field is a commutative `⊔`.

    **The hypothesis is the code's own precondition, not a weakening introduced to make a proof
    close** (`casper/src/dag.rs:79-80`; the caller reads `existing` by `incoming`'s key). Away from
    one key the law is simply false — `joinRecord_not_commutative` below — so a version of this
    theorem without the hypothesis would be a claim about a pair of records the join is never handed.

    The Rust's doc says the join "is commutative, associative and idempotent, so every arrival order
    reaches the same record" and the *caller* supplies the key agreement (`dag.rs:820-850` reads the
    record at `fringe_hash` before joining). This is that sentence with its precondition made
    explicit: the join is commutative **on the argument pairs the function is ever called with**. -/
theorem joinRecord_comm (a b : FringeRecord) (h : AtOneKey a b) :
    joinRecord a b = joinRecord b a := by
  obtain ⟨hfh, hf⟩ := h
  apply FringeRecord.ext <;>
    simp [joinRecord, hfh, hf, Finset.union_comm, Nat.min_comm]

/-- **The hypothesis is not a convenience** — this is the non-vacuity check on `joinRecord_comm`, in
    the same spirit as `Rchain/RSpace/Merge.lean`'s `nonConflicting_not_necessary`: two records that
    are *not* for one key are joined to two different records, since the join returns a record for its
    left operand's key (`joinRecord_keeps_the_stored_key`). So "the join is commutative" is false as a
    general statement and `joinRecord_comm`'s hypothesis is doing real work — which is exactly what the
    code's doc, read without the sentence about the caller, hides. -/
theorem joinRecord_not_commutative : ∃ a b : FringeRecord, joinRecord a b ≠ joinRecord b a := by
  refine ⟨⟨0, ∅, ∅, 0, ∅, ∅, ∅⟩, ⟨1, ∅, ∅, 0, ∅, ∅, ∅⟩, fun h => ?_⟩
  have hfh := congrArg FringeRecord.fringeHash h
  simp [joinRecord] at hfh

/-- **The fourth law is the one that fails, and it fails on `state_hash`.** A monoid needs an identity:
    an `e` with `joinRecord e r = r` for every `r`. There is none, and the reason is the same one that
    makes the join interesting — `state_hash` is settled by `min` (`dag.rs:91`), and `min` has no
    identity on `Nat` (`min e s = s` for *every* `s` would need `e` above every natural). The sets
    would be happy with `∅`; the state is not, and the state is the field C215 is about.

    So what the join is, precisely, is a **commutative idempotent semigroup** — a band — on the records
    at a key: `joinRecord_assoc`, `joinRecord_comm` and `joinRecord_idempotent`. The word *monoid* is
    loose, and the loss is worth naming rather than papering over with a fabricated identity: the
    `foldr` below is seeded with the **stored record** (which is what `insert` joins with,
    `dag.rs:820-850`), and the stored record is not an identity — it contributes its own value to the
    union and its own state to the `min`, which is precisely how a writer's value comes to be
    rewritten (`the_join_rewrites_a_writers_value`). -/
theorem joinRecord_has_no_identity : ¬ ∃ e : FringeRecord, ∀ r : FringeRecord, joinRecord e r = r := by
  rintro ⟨e, he⟩
  have h : e.fringeHash = e.fringeHash + 1 :=
    congrArg FringeRecord.fringeHash
      (he { fringeHash := e.fringeHash + 1, fringe := ∅, fringeDiff := ∅, stateHash := 0,
            rejectedDeploys := ∅, rejectedBlocks := ∅, rejectedSenders := ∅ })
  omega

/-! ## The consequence the laws are for: the value is a function of the set

`insert` applies the join once per arriving block, so the record at a key after `n` arrivals is a
`foldr` of the join over the arrival sequence seeded with the stored record. The laws above say which
part of that fold is observable: associativity removes the grouping, commutativity removes the order,
and idempotence (`joinRecord_idempotent`) removes a repeat. What follows is that the fold depends on
the *set* of arrivals and nothing else — which is the sentence in `dag.rs:56` ("A fringe record is a
function of its fringe key") read at the granularity the model supports, and the reason
`merging.rs::rejections_for` can read these records back and get the same answer on every node.

**Why `List.Perm` and not `Finset.fold`.** The arrival order is a list, and the honest statement is
that two orders of the same arrivals — a `List.Perm` — fold to the same record; that is what the code
does, one `insert` at a time. The `Finset` version (`the_joined_value_is_a_function_of_the_set`) is
derived from it, and `Finset.fold` is not used because its commutativity requirement is quantified
over *all* elements and an arbitrary accumulator, where the law at hand only holds when the elements
share a key: the fold below consumes `Perm.foldr_eq'`, whose condition is quantified over the list's
own elements (`Mathlib/Data/List/Perm.lean:54-55`), which is the shape the theorem needs. -/

/-- The step `Perm.foldr_eq'` needs, stated on its own because it is the entire interaction between
    commutativity and associativity here: two records for one key can be swapped past each other
    **whatever the accumulator is**. The accumulator's own key is irrelevant — `joinRecord x z` is a
    record for `x`'s key whatever `z` is — so the hypothesis is only that the two swapped records
    agree, which is the weakest form of `joinRecord_comm` that can be applied mid-fold. -/
theorem joinRecord_swap (a b z : FringeRecord) (h : AtOneKey a b) :
    joinRecord b (joinRecord a z) = joinRecord a (joinRecord b z) := by
  rw [← joinRecord_assoc b a z, ← joinRecord_assoc a b z, joinRecord_comm a b h]

/-- **Order-independence.** Every element of the arriving list is a record for the seed's key (the
    store's precondition, `dag.rs:79-80`), so every two of them are for one key with each other
    (`atOneKey_of_common_base`) and may be swapped — and the fold of two permutations of the list is
    one record. `foldr` with the join as the operation and the stored record as the seed is exactly
    `insert` repeated (`dag.rs:820-850`), so this is the sentence "every arrival order reaches the same
    record" turned into a theorem about the arrival sequences. -/
theorem joinRecord_fold_perm (s : FringeRecord) {l₁ l₂ : List FringeRecord}
    (hperm : l₁.Perm l₂) (h₁ : ∀ r ∈ l₁, AtOneKey r s) :
    l₁.foldr joinRecord s = l₂.foldr joinRecord s :=
  hperm.foldr_eq'
    (fun x hx y hy z => joinRecord_swap x y z (atOneKey_of_common_base (h₁ x hx) (h₁ y hy))) s

/-- **The value is a function of the arriving *set*.** Any listing of a given `Finset` of arrivals
    folds to the same record, so the record after the arrivals is determined by *which* records have
    arrived and not by the order they did — the property `dag.rs:56` claims of the key and this file
    supplies for the join.

    `Finset` is the right carrier and not a convenience: the store sees a set of blocks
    (`seen(fringe)`, `dag.rs:765-788`) and the writes are idempotent on repeats, so an arrival
    *sequence* is an enumeration of its own set — and two enumerations of one set are permutations of
    each other, which is what the fold lemma above consumes. The list `S.val.toList` is only here to
    be the canonical enumeration every other listing is compared against. -/
theorem the_joined_value_is_a_function_of_the_set (s : FringeRecord) (S : Finset FringeRecord)
    (hS : ∀ r ∈ S, AtOneKey r s) (l : List FringeRecord) (hl : l.Perm S.val.toList) :
    l.foldr joinRecord s = S.val.toList.foldr joinRecord s := by
  refine joinRecord_fold_perm s hl fun r hr => hS r ?_
  simpa [Multiset.mem_toList, Finset.mem_val] using (hl.mem_iff).1 hr

/-- **A repeat is a no-op.** `joinRecord r (joinRecord r z) = joinRecord r z` with **no hypothesis at
    all**: the second join's key data is overwritten by the first's, the sets are idempotent
    (`Finset.union_self`/`union_assoc`), and `min (min s t) t = min s t` for the same reason idempotence
    holds for `min` anywhere. So two adjacent copies of an arrival collapse to one, which is the
    multiplicity half of "the value is a function of the set, not of the sequence": with
    `joinRecord_fold_perm` (which supplies the ordering half) an arrival sequence and the set it
    enumerates are the same input.

    Stated for two adjacent copies rather than as a general `List.dedup` lemma: that would need the
    fold's value to dominate each of its elements, and nothing below uses it. -/
theorem joining_a_record_twice_is_joining_it_once (r z : FringeRecord) :
    joinRecord r (joinRecord r z) = joinRecord r z := by
  apply FringeRecord.ext <;>
    simp [joinRecord, Finset.union_self, Finset.union_assoc, Nat.min_self, Nat.min_assoc]

/-! ## What the laws are worth: they make the tie-break reachable, and the fix makes it unreachable

The two sections above are the good news — the map is deterministic under any arrival order. The bad
news is *how* it is deterministic: `state_hash` cannot be joined, so it is settled by `min`
(`dag.rs:91`), and `min` is a function that need not return either argument's value. `Record.lean`
proves that consequence (`the_join_rewrites_a_writers_value`, `:250`) and it is the mechanism of
C270's stall: a node builds a block under the state it computed, reads back the `min`, and its own
validation refuses its own block (`spec/findings.tsv` C270;
`spec/audit/evidence/n-anchor-drill/self-reject-stall.txt`).

**The tie-break is not inherent to the join; it is inherent to the key.** At one `writeKey` — the
fringe *and* the base the merge started from (`Record.lean:197`) — two writers cannot disagree, because
`recordOf` is a function of exactly those two arguments (`the_composite_key_determines_the_value`,
`Record.lean:205`, cited below rather than re-derived). So `min` is only ever `min s s` there, and the
whole `the_join_rewrites_a_writers_value` case becomes unreachable — which is what "the key names the
base" buys, stated as theorems rather than as the intention it was. -/

/-- At one write key the writer's record is determined: two writers who fixed the same
    `(fringe, base)` compute the *same record*, not merely the same state. Immediate from `recordOf`
    being a function of `(fringe, base)` — the content of the fix is not in this proof but in
    `writeKey` naming both arguments the value is built from (`Record.lean:188-197`), which the code's
    key does not. -/
theorem writers_at_one_writeKey_agree (f₁ f₂ : FringeSet) (b₁ b₂ : MergeBase)
    (h : writeKey f₁ b₁ = writeKey f₂ b₂) : recordOf f₁ b₁ = recordOf f₂ b₂ := by
  simp only [writeKey] at h
  obtain ⟨rfl, rfl⟩ := h
  rfl

/-- **The tie-break is the identity at a write key.** `dag.rs:91`'s `min` over two records written at
    one `writeKey` returns the first argument — because there is nothing to break a tie between. This
    cites `the_composite_key_determines_the_value` (`Record.lean:205`) for the disagreement's
    impossibility and adds only `min_self`. -/
theorem the_tie_break_is_the_identity_at_a_writeKey (f₁ f₂ : FringeSet) (b₁ b₂ : MergeBase)
    (h : writeKey f₁ b₁ = writeKey f₂ b₂) :
    min (recordOf f₁ b₁).stateHash (recordOf f₂ b₂).stateHash = (recordOf f₁ b₁).stateHash := by
  rw [the_composite_key_determines_the_value f₁ f₂ b₁ b₂ h, min_self]

/-- **`the_join_rewrites_a_writers_value` is unreachable at a write key.** That theorem
    (`Record.lean:250`) is conditional on `r₁.stateHash < r₂.stateHash` — the disagreement it needs in
    order to rewrite something. Under the composite key no such pair exists, so the stall's mechanism
    has no case: the `min` cannot pick the stored value *over* the writer's, because the two are one
    value. This is the theorem that retires the tie-break, stated as the negation of the hypothesis
    the stall needs. -/
theorem the_tie_break_is_unreachable (f₁ f₂ : FringeSet) (b₁ b₂ : MergeBase)
    (h : writeKey f₁ b₁ = writeKey f₂ b₂) :
    ¬ (recordOf f₁ b₁).stateHash < (recordOf f₂ b₂).stateHash := by
  rw [the_composite_key_determines_the_value f₁ f₂ b₁ b₂ h]
  exact lt_irrefl _

/-- **No writer is rewritten.** The join of two records written at one `writeKey` keeps the writer's
    state, because `joinRecord_settles_the_state_by_min` has nothing to settle. This is
    `the_join_rewrites_a_writers_value` with its hypothesis discharged by the key: `dag.rs:91`'s `min`
    can no longer return a value that neither writer computed, and `multi_parent_casper.rs`'s
    `get_pre_state_for_parents` therefore reads back for the block's pre-state what the block's own
    writer computed for it. -/
theorem no_writer_is_rewritten_at_a_writeKey (f₁ f₂ : FringeSet) (b₁ b₂ : MergeBase)
    (h : writeKey f₁ b₁ = writeKey f₂ b₂) :
    (joinRecord (recordOf f₁ b₁) (recordOf f₂ b₂)).stateHash = (recordOf f₁ b₁).stateHash := by
  rw [joinRecord_settles_the_state_by_min,
    the_composite_key_determines_the_value f₁ f₂ b₁ b₂ h, min_self]

/-- **And the join is the identity, not merely faithful on `state_hash`.** Two records written at one
    write key are one record (`writers_at_one_writeKey_agree`), so the read-join-write at
    `dag.rs:820-850` writes back what was already stored — the whole loop is a no-op. Nothing is
    unioned that was not already there and nothing is `min`ned that differs, so under the fix the
    clause *"so every arrival order reaches the same record"* is true for the trivial reason that
    every arrival reaches the *same record object*. -/
theorem the_join_at_a_writeKey_changes_nothing (f₁ f₂ : FringeSet) (b₁ b₂ : MergeBase)
    (h : writeKey f₁ b₁ = writeKey f₂ b₂) :
    joinRecord (recordOf f₁ b₁) (recordOf f₂ b₂) = recordOf f₁ b₁ := by
  rw [writers_at_one_writeKey_agree f₁ f₂ b₁ b₂ h, joinRecord_idempotent]

/-- **What C270 was, exactly.** The read-join-write at `dag.rs:820-850` writes back the stored record
    unchanged precisely when the incoming report adds nothing: no new diff or rejection, and no state
    below the stored one. So the stall is not "the join ran" and not "the two records were at one key"
    — the key data is the left operand's here with no hypothesis at all, which is the same left-bias
    `joinRecord_not_commutative` measures — it is a disagreement about the *state*.

    And the two sides of an asymmetry that is the stall's whole shape are visible in the conjuncts:
    when the incoming report adds nothing, the **store** sees a write it cannot distinguish from no
    write, while the **incoming writer** reads back a value it did not compute
    (`the_join_rewrites_a_writers_value`: the stored state was strictly smaller, so `min` kept the
    stored one and the arrival's own state is gone). That is a node whose own block's report is
    silently replaced by the store's — the self-reject. The join is a no-op for exactly one of the two
    parties, and it is not the one that computed the value.

    At a `writeKey` the incoming report *is* the stored record (`writers_at_one_writeKey_agree`), so
    every conjunct holds for both parties and there is no side to take: the no-op that
    `the_join_at_a_writeKey_changes_nothing` proves directly is the one this characterises.

    The Rust's own words for the case are `Some(existing) if existing != fringe_data` (`dag.rs:832`) —
    an equality test the port has to make because the join is a function that would otherwise silently
    settle a disagreement; under the composite key the test's false branch is the only branch. -/
theorem the_join_changes_nothing_iff_the_report_adds_nothing (r₁ r₂ : FringeRecord) :
    joinRecord r₁ r₂ = r₁ ↔
      r₂.fringeDiff ⊆ r₁.fringeDiff ∧ r₁.stateHash ≤ r₂.stateHash ∧
        r₂.rejectedDeploys ⊆ r₁.rejectedDeploys ∧ r₂.rejectedBlocks ⊆ r₁.rejectedBlocks ∧
        r₂.rejectedSenders ⊆ r₁.rejectedSenders := by
  constructor
  · intro heq
    refine ⟨?_, ?_, ?_, ?_, ?_⟩
    · exact Finset.union_eq_left.mp (by simpa [joinRecord] using congrArg FringeRecord.fringeDiff heq)
    · exact min_eq_left_iff.mp (by simpa [joinRecord] using congrArg FringeRecord.stateHash heq)
    · exact Finset.union_eq_left.mp
        (by simpa [joinRecord] using congrArg FringeRecord.rejectedDeploys heq)
    · exact Finset.union_eq_left.mp
        (by simpa [joinRecord] using congrArg FringeRecord.rejectedBlocks heq)
    · exact Finset.union_eq_left.mp
        (by simpa [joinRecord] using congrArg FringeRecord.rejectedSenders heq)
  · rintro ⟨hd, hs, hrde, hrb, hrs⟩
    apply FringeRecord.ext <;>
      simp only [joinRecord] <;>
      first
        | rfl
        | exact Finset.union_eq_left.mpr (by assumption)
        | exact min_eq_left_iff.mpr (by assumption)

end Rchain
