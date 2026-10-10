import Mathlib.Data.Finset.Card

/-!
# The fringe record, and the key that does not determine it

The node stores one record per **fringe** (`models/src/fringe_data.rs:20-42`, the value written at
`casper/src/dag.rs:790-850`). The record's *identity* is its `fringe_hash` — the `Hash` impl hashes
`fringe_hash` and nothing else (`fringe_data.rs:31-35`), the Scala it mirrors says the same
("uniquely identified by the hash of its fringe hashes"), and the store is written at exactly that
key: `FringeData::fringe_hash_of(&block_metadata.fringe)` (`dag.rs:757`), a function of the **fringe
alone**. The code states the assumption this file is about, in its own words: *"the value at a fringe
key must be a function of that key"* (`dag.rs:820-822`), and `join_fringe_records`' doc says it again
("A fringe record is a function of its fringe key", `dag.rs:70`).

**The assumption is false, and it was measured.** On a four-validator devnet with autopropose on and
**no injection and no fault**, four fringe keys were recorded with **two different states each** —
`[fringe-divergence] block <h> finalises a fringe already recorded with a different state <a> vs <b>`,
four times on **every** node, the same four blocks and the same two hashes. Three of the four nodes
then rejected **their own** block and halted their own production
(`spec/audit/evidence/n-anchor-drill/self-reject-stall.txt`, register row C270).

Why it is false is visible in the reader, not the writer: `state_hash` for a fringe is not computed
from the fringe but from the **merge base** the merge started at — `base_state = base_opt's post-state,
else prev_fringe_state` (`casper/src/multi_parent_casper.rs:226-238`), where `base_opt` comes from
`MergeScope::from_dag(fringe, &prev_fringe_hashes, …)` (`:228-229`) and `prev_fringe` is the *parents'*
finalized fringe (`:146`). Two blocks can reach **one fringe from two different `prev_fringe`s**: the
key mentions neither. The base is right there at the read (`:156-166, :230-238`) and nowhere at the
write, so the value is a function of `(fringe, base)` stored under a key that names only `fringe`.

## What is proved, and what that is worth

- `the_fringe_only_key_is_incomplete` — the deliverable: two records the code's key cannot tell apart
  with two different states, with a concrete witness (`mergeFringe f b := b`, two bases), so it is a
  theorem and not a hypothesis dressed as one. **No nondeterminism is assumed to get there** — the
  merge here is a deterministic function of `(fringe, base)`; what fails is the key, not the merge.
- `no_fringe_only_key_can_be_complete` — the same refutation **quantified over every candidate key
  that reads only the fringe**. No improvement to `fringe_hash_of` — a better hash, a stronger mix, an
  injective encoding — can repair the key, because the collision is between records with the *same
  fringe*, where every function of the fringe agrees. This is why the refutation is not about the hash.
- `the_composite_key_determines_the_value` — the **positive** statement for the fix the project has
  chosen (the key names the base). It is trivial once the model is right, and honestly so: `recordOf`
  is a *function* of `(fringe, base)`, so its state is determined by exactly those two arguments. The
  content of the fix is not in the proof; it is in noticing that the code's key drops one of the two
  arguments the value is built from.
- `the_join_rewrites_a_writers_value` — the mechanism of the stall. `join_fringe_records`
  (`dag.rs:82-108`) settles a disagreement by `min`, which makes the *map* deterministic (C215) while
  the *value* read back is no longer what either writer computed. A node that built its block under
  the value it computed then reads a third thing, and its own validation refuses its own block (C270).

**One place the model is deliberately coarser than the Rust, and it is a finding.** The prompt's form
`key r := (r.fringe, r.base)` requires the record to carry its base. The Rust record does **not**
(`fringe_data.rs:20-28` has no base field), and it cannot be recovered from what is stored: `fringe_diff`
is `seen(fringe) − seen(prev_fringe)` (`dag.rs:765-788`), a difference of seen-sets, and the rejection
sets are merge *outputs*. So the composite key cannot be a key on the stored record — it has to be
applied at the **write** (`dag.rs:757-800`), where the base is still in hand as `prev_fringe`
(`dag.rs:775`) beside `fringe` (`:757`). That is modelled as `writeKey`, and it is the reason the fix
is a write-site change rather than a store-schema change.
-/

namespace Rchain

/-- A block, by its hash. The tree represents ids as `Nat` throughout (`Rchain.Casper.Dag`); the Rust
    type is `BlockHash` (`models/src/fringe_data.rs:21`). -/
abbrev BlockId := Nat

/-- A deploy, by its id. The Rust type is `Vec<u8>` (a deploy's id bytes,
    `models/src/fringe_data.rs:25`); only equality matters here. -/
abbrev DeployId := Nat

/-- A fringe: the **set** of block hashes the finalizer published (`BTreeSet<BlockHash>`,
    `models/src/fringe_data.rs:22`). A `Finset`, because that is what the Rust's ordered set is — and
    because Law 18 (fringe identity is order-independent, pinned by the port's own
    `law18_fringe_hash_is_order_independent`) is a property of the set, not of any list that spells it. -/
abbrev FringeSet := Finset BlockId

/-- A stored post-state hash (`state_hash`, `models/src/fringe_data.rs:24`). -/
abbrev StateHash := Nat

/-- **The merge base**: the state a merge for a fringe starts from. In the Rust this is `base_opt`'s
    post-state when the final scope has one, else `prev_fringe_state`
    (`casper/src/multi_parent_casper.rs:230-238`) — both are state hashes, which is why the base *is* a
    `StateHash`. The key never mentions it. -/
abbrev MergeBase := StateHash

/-- The hash of the (sorted) fringe — the store's primary key. `fringe_hash_of`,
    `models/src/fringe_data.rs:39-42` (§ `Blake2b256Hash::create_many` over the sorted hashes).
    Modelled as any function of the fringe: what matters below is only that it *is* one, so that the
    refutation is independent of the hash's strength (see `no_fringe_only_key_can_be_complete`). -/
def fringeHashOf (f : FringeSet) : Nat := f.card

/-- The merge a fringe's blocks carry, as a function of the fringe and the **base** it starts from.

    Abstract on purpose — the real one is `MergeScope::merge(&m_scope, base_state, …)`
    (`casper/src/multi_parent_casper.rs:239-247`), which folds the scope's blocks over the base — but
    the model only needs the one fact every merge has: it **reads the base**. A merge that reads the
    base at all is enough to refute "the fringe determines the value", so no more is modelled than
    that, and the witness below uses the definite function `fun _ b => b`.

    **The model is deterministic, and deliberately so**: `mergeFringe` is a *function* of `(fringe,
    base)` — the favourable case for the code, granting it the determinism C250's residue argues for
    ("the merge for a fringe is a function of the DAG"). The refutation below assumes no
    nondeterminism anywhere; it needs only that the key drops one of the two arguments the value is
    built from. A model that let the merge be nondeterministic would prove less, not more. -/
def mergeFringe (_f : FringeSet) (base : MergeBase) : StateHash := base

/-- **The stored record** — the fields the Rust store carries
    (`models/src/fringe_data.rs:20-28`), in the same order.

    Which of them are functions of the fringe **alone** is the whole subject:

    - `fringeHash` and `fringe` **are**: the first is `fringe_hash_of` of the second
      (`dag.rs:757`), and the store's `Hash` impl hashes only the first (`fringe_data.rs:31-35`). These
      are the key's own data, and they are the *only* fields the key can speak about;
    - `stateHash` **is not**: it is the merge's post-state, which reads the base
      (`multi_parent_casper.rs:230-238`);
    - `fringeDiff` **is not**: it is `seen(fringe) − seen(prev_fringe)` (`dag.rs:783-787`), and
      `prev_fringe` is not a function of `fringe`;
    - the three `rejected*` sets **are not**: they are the merge's outputs for the scope the base
      bounds (`merging.rs::rejections_for` reads them back out of these records, `:1512-1525`).

    The record carries **no base**, which is the defect stated at the top of this file: the store keys
    on the one input the value does not depend on alone, and drops the one it also depends on. -/
structure FringeRecord where
  /-- `fringe_hash` — the key (`models/src/fringe_data.rs:21`, `dag.rs:757`). -/
  fringeHash : Nat
  /-- `fringe` — the set of finalized block hashes (`fringe_data.rs:22`). -/
  fringe : FringeSet
  /-- `fringe_diff` — the blocks seen since the previous fringe (`fringe_data.rs:23`, `dag.rs:765-788`). -/
  fringeDiff : FringeSet
  /-- `state_hash` — the merge's post-state (`fringe_data.rs:24`, `dag.rs:794-796`). -/
  stateHash : StateHash
  /-- `rejected_deploys` (`fringe_data.rs:25`). -/
  rejectedDeploys : Finset DeployId
  /-- `rejected_blocks` (`fringe_data.rs:26`). -/
  rejectedBlocks : FringeSet
  /-- `rejected_senders` (`fringe_data.rs:27`). -/
  rejectedSenders : Finset DeployId
  deriving DecidableEq

/-- **What the writer computes** for a fringe and the base its merge starts from — the record the node
    builds at `casper/src/dag.rs:790-800`. The rejection sets and the diff are held empty here: they
    are not what the key claims to determine, and the refutation does not need them. Their emptiness is
    a modelling choice, not a claim that they are empty in the node. -/
def recordOf (f : FringeSet) (base : MergeBase) : FringeRecord where
  fringeHash := fringeHashOf f
  fringe := f
  fringeDiff := ∅
  stateHash := mergeFringe f base
  rejectedDeploys := ∅
  rejectedBlocks := ∅
  rejectedSenders := ∅

/-- **The key the code uses today** — the fringe hash alone (`dag.rs:757`; `FringeData`'s identity,
    `fringe_data.rs:31-35`). Everything below is stated over this, so the statements are the code's and
    not a paraphrase of them. -/
def key (r : FringeRecord) : Nat := r.fringeHash

/-! ## The refutation -/

/-- **The claim the node assumes, and its falsifier.** "The value at a fringe key is a function of that
    key" (`dag.rs:820-822`) — stated as the code uses it (`key` is the fringe hash alone) and refuted
    by two records the key cannot tell apart: one fringe (`∅`), two bases, two states.

    This is the measured devnet divergence, not a hypothetical: `[fringe-divergence] block <h> finalises
    a fringe already recorded with a different state <a> vs <b>`, four keys on every node of an
    uninjected four-validator rig (`spec/audit/evidence/n-anchor-drill/self-reject-stall.txt`, C270).
    The code's `min` tie-break (`join_fringe_records`, `dag.rs:91`) is what hides it: it makes the map
    *deterministic* — every arrival order agrees — while the value that comes back is neither writer's,
    which is precisely how a node comes to reject its own block.

    The witness is concrete (`mergeFringe f b := b`, base `0` vs base `1`), so this is a theorem about
    the model's own value and there is no hypothesis to discharge. -/
theorem the_fringe_only_key_is_incomplete :
    ∃ r₁ r₂ : FringeRecord, key r₁ = key r₂ ∧ r₁.stateHash ≠ r₂.stateHash := by
  refine ⟨recordOf ∅ 0, recordOf ∅ 1, rfl, ?_⟩
  decide

/-- **The refutation does not depend on the hash.** For **every** candidate key `g` that reads only the
    fringe — any hash at all, however strong, injective included — two records exist with equal keys and
    different states. So `fringe_hash_of` cannot be repaired by improving it: the colliding pair has the
    *same fringe*, where every function of the fringe agrees. The only thing that can make the key
    complete is naming the input that is not the fringe (`the_composite_key_determines_the_value`). -/
theorem no_fringe_only_key_can_be_complete (g : FringeSet → Nat) :
    ∃ r₁ r₂ : FringeRecord, g r₁.fringe = g r₂.fringe ∧ r₁.stateHash ≠ r₂.stateHash := by
  refine ⟨recordOf ∅ 0, recordOf ∅ 1, rfl, ?_⟩
  decide

/-! ## The fix: the key names the base

The project's chosen fix is that the key names the base the merge started from. It has to be applied at
the **write** (`casper/src/dag.rs:757-800`) rather than by adding a field to the stored record, because
the base is not recoverable from what the store holds (see the header). At the write it is in hand:
`prev_fringe` is computed there for `fringe_diff` (`dag.rs:775`) beside `fringe` (`dag.rs:757`). -/

/-- **The composite key, on the writer's two inputs**: the fringe *and* the base. `(fringe, base)` is
    what the value is a function of, so this key is the weakest one that can determine it. -/
def writeKey (f : FringeSet) (base : MergeBase) : FringeSet × MergeBase := (f, base)

/-- **The positive statement, and it is trivial — which is the honest way to say it.** Once `recordOf`
    is a function of `(fringe, base)`, the state is determined by exactly those two arguments and the
    composite key distinguishes nothing else, so the proof is `rfl` after the key agrees. The content
    is not here: it is in the refutation above, whose whole force is that the code's key drops one of
    these two arguments. Stated anyway, because a fix that is never written down as a theorem is a fix
    nobody checked is a fix. -/
theorem the_composite_key_determines_the_value (f₁ f₂ : FringeSet) (b₁ b₂ : MergeBase)
    (h : writeKey f₁ b₁ = writeKey f₂ b₂) :
    (recordOf f₁ b₁).stateHash = (recordOf f₂ b₂).stateHash := by
  obtain ⟨rfl, rfl⟩ := h
  rfl

/-! ## The join, and the stall it produces

`join_fringe_records` (`casper/src/dag.rs:82-108`) is what `insert` does with a record already stored
at the key: the rejection sets and the diff are **unioned** (monotone — merges can only reject more,
the fail-closed direction) and `state_hash` is settled by `min`, because a hash cannot be joined. The
union is monotone and the `min` is a function, so the *map* is deterministic under every arrival order:
that is C215's contract, and it holds. What does not hold is that the value is harmless — the value read
back is not the value a writer computed, which is C270. -/

/-- The join of a stored record and an incoming one for the same key — `join_fringe_records`,
    `casper/src/dag.rs:82-108`. Ids come from the stored side (`existing.fringe_hash`/`fringe`,
    `:84-85`), the sets are unioned, and `state_hash` is `min` (`:91`). -/
def joinRecord (stored incoming : FringeRecord) : FringeRecord where
  fringeHash := stored.fringeHash
  fringe := stored.fringe
  fringeDiff := stored.fringeDiff ∪ incoming.fringeDiff
  stateHash := min stored.stateHash incoming.stateHash
  rejectedDeploys := stored.rejectedDeploys ∪ incoming.rejectedDeploys
  rejectedBlocks := stored.rejectedBlocks ∪ incoming.rejectedBlocks
  rejectedSenders := stored.rejectedSenders ∪ incoming.rejectedSenders

/-- The join keeps the stored side's key data — so the value at a key is a record *for* that key, which
    is the weak invariant the code can actually state (`dag.rs:84-85`). -/
theorem joinRecord_keeps_the_stored_key (r₁ r₂ : FringeRecord) :
    (joinRecord r₁ r₂).fringeHash = r₁.fringeHash := rfl

/-- **The tie-break.** A disagreement about a fringe's state is settled by taking the smaller
    (`dag.rs:91`, the code's "the smaller is kept so that every node holding these blocks holds the same
    record"). This is the *whole* of the map's determinism: with the sets unioned (monotone) and the
    state `min`ed (a function), every arrival order reaches one record. -/
theorem joinRecord_settles_the_state_by_min (r₁ r₂ : FringeRecord) :
    (joinRecord r₁ r₂).stateHash = min r₁.stateHash r₂.stateHash := rfl

/-- **The stall (C270).** When the stored record's state is the smaller, the join *rewrites* the value
    the incoming writer computed: the node that built its block under its own state now reads the
    stored one, and its own validation — which recomputes the merge over the record it now holds —
    disagrees with the block it already made. Measured: three of four nodes rejected their own block at
    height 49 and stopped producing (`self-reject-stall.txt`). The tie-break is what makes this look
    like a settled value rather than the divergence it is. -/
theorem the_join_rewrites_a_writers_value (r₁ r₂ : FringeRecord) (h : r₁.stateHash < r₂.stateHash) :
    (joinRecord r₁ r₂).stateHash ≠ r₂.stateHash := by
  rw [joinRecord_settles_the_state_by_min, Nat.min_eq_left (Nat.le_of_lt h)]
  exact ne_of_lt h

/-- The join is idempotent — re-inserting the same record changes nothing — which is why the write is
    safe to repeat: `dag.rs`'s `insert` short-circuits on a known block, and a re-insert that is not
    short-circuited lands on the same value. -/
theorem joinRecord_idempotent (r : FringeRecord) : joinRecord r r = r := by
  cases r
  simp [joinRecord, Finset.union_self, min_self]

end Rchain
