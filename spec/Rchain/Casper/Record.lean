import Mathlib.Data.Finset.Card

/-!
# The fringe record, the key that does not determine it, and the shape that retires the store

The node **stored** one record per **fringe** (`models/src/fringe_data.rs:20-42`, the value written at
`casper/src/dag.rs:790-850`). The record's *identity* was its `fringe_hash` — the `Hash` impl hashed
`fringe_hash` and nothing else (`fringe_data.rs:31-35`), the Scala it mirrors said the same
("uniquely identified by the hash of its fringe hashes"), and the store was written at exactly that
key: `FringeData::fringe_hash_of(&block_metadata.fringe)` (`dag.rs:757`), a function of the **fringe
alone**. The code stated the assumption this file is about, in its own words: *"the value at a fringe
key must be a function of that key"* (`dag.rs:820-822`), and `join_fringe_records`' doc said it again
("A fringe record is a function of its fringe key", `dag.rs:70`).

**The assumption was false, and it was measured.** On a four-validator devnet with autopropose on and
**no injection and no fault**, four fringe keys were recorded with **two different states each** —
`[fringe-divergence] block <h> finalises a fringe already recorded with a different state <a> vs <b>`,
four times on **every** node, the same four blocks and the same two hashes. Three of the four nodes
then rejected **their own** block and halted their own production
(`spec/audit/evidence/n-anchor-drill/self-reject-stall.txt`, register row C270).

Why it was false is visible in the reader, not the writer: `state_hash` for a fringe was not computed
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
- `the_composite_key_determines_the_value` — the positive statement for the composite-key fix the
  project **chose and then refuted** (2026-10-10): keying by `(fringe, base)` is not implementable at
  the reader that needs it (`casper/src/multi_parent_casper.rs:156-165` reads by the fringe-only key
  while the base is computed *later*, `:230-238`). Vacuous — tuple injectivity — and kept only because
  `RecordJoin.lean` cites it; the fix chosen instead is the per-block shape at the end of this file.
- `the_join_rewrites_a_writers_value` — the mechanism of the stall. `join_fringe_records`
  (`dag.rs:82-108`) settles a disagreement by `min`, which makes the *map* deterministic (C215) while
  the *value* read back is no longer what either writer computed. A node that built its block under
  the value it computed then reads a third thing, and its own validation refuses its own block (C270).

**Retired 2026-10-10, and this header is where the reason is recorded.** The `fringe-data` store is
retired: every claim it held is a per-block fact the node computed itself and wrote once —
`BlockMetadata.fringe_state_hash` (`models/src/block_metadata.rs:200`, derived at
`casper/src/interpreter_util.rs:766-767`), the block's own `rejected_deploys`
(`models/src/casper/protocol/casper_message.rs:780`), and the index `member_of_fringe`
(`models/src/block_metadata.rs:201`, built at `casper/src/dag.rs:887-895`). The reader now **names a block**
and reads that block's claim, so a fringe's state is a function of the blocks that finalised it, not of a
key that hides them. The `writeKey` and `joinRecord` sections below are the retired mechanism, kept rather than deleted; the new shape is stated at the end of this file.
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

/-! ## The fix that was chosen, and then refuted (retired 2026-10-10)

The key names the base the merge started from. It was reasoned as a **write**-site change
(`casper/src/dag.rs:757-800`), because the base is not recoverable from what the store holds:
`prev_fringe` is computed there for `fringe_diff` (`dag.rs:775`) beside `fringe` (`dag.rs:757`). It is
**retired** — the review refuted it as inexpressible at the reader that needs it (`casper/src/multi_parent_casper.rs:156-165`); the design chosen instead is the per-block shape at the end of this file, and the declarations below are kept only because `RecordJoin.lean` cites both. -/

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

/-! ## The join, and the stall it produces (retired with the map, 2026-10-10)

`join_fringe_records` (`casper/src/dag.rs:82-108`) is what `insert` did with a record already stored
at the key: the rejection sets and the diff are **unioned** (monotone — merges can only reject more,
the fail-closed direction) and `state_hash` is settled by `min`, because a hash cannot be joined. The
union is monotone and the `min` is a function, so the *map* was deterministic under every arrival order:
that is C215's contract, and it held. What did not hold is that the value was harmless — the value read
back was not the value a writer computed, which is C270. It **retires with the map**: there is nothing left to join, and the stall's mechanism goes with it. -/

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

/-! ## The store retires: a claim travels with its block (2026-10-10)

The design the node implements drops the keyed store entirely. Every claim `fringe-data` held is already
a per-block fact each block computed for itself and wrote once, under its own block hash
(`BlockMetadata`'s identity is `block_hash` alone, `models/src/block_metadata.rs:205-207`):

- `fringe_state_hash` — the block's **own** derivation for its fringe (`block_metadata.rs:200`, computed
  at `casper/src/interpreter_util.rs:766-767`);
- `rejected_deploys` — the block's own merge's rejections
  (`models/src/casper/protocol/casper_message.rs:780`);
- `member_of_fringe` — the index of blocks by fringe (`models/src/block_metadata.rs:201`, built at
  `casper/src/dag.rs:887-895`).

So the reader **names a block** — a version — and reads *that block's* claim; a fringe's state is a
function of the blocks that finalised it, not of a key that hides them. This is the shape Law 66's
register `note` records as the one chosen after the composite key was refuted, and it is the shape Law
68's positive half needs: a block's claim is written **once**, so the claim store is append-only (proved
in `spec/Rchain/Casper/ReadVersion.lean`), and a read names a version that names a value.

The two facts below are `the_fringe_only_key_is_incomplete` read backwards. There the key was the fringe,
which many blocks share, so one key had to hold one value for many bases; here the version is the block,
which carries one claim. The first fact is the reader's guarantee (a block determines the state it
derived); the second is the ambiguity's impossibility (two claims at one fringe that disagree are told
apart by their blocks — no key merges them). -/

/-- **A block's claim** — a block and the claim it carries: the fringe it finalised and the state it
    derived for that fringe. Where `FringeRecord` above was written at `fringe_hash_of(fringe)`, a key
    many blocks share, this is written at the block's own hash (`BlockMetadata`'s identity is
    `block_hash`, `models/src/block_metadata.rs:205-207`): the claim travels with the block, and the key
    that hid the block retires with the store.

    The id is a `Nat` like this file's neighbours (`BlockId`); the Rust is `BlockHash`. -/
structure BlockClaim where
  /-- The block — the **version** a reader names (`BlockMetadata.block_hash`,
      `models/src/block_metadata.rs:142`). -/
  block : BlockId
  /-- The fringe the block finalised (`BlockMetadata.fringe`, `models/src/block_metadata.rs:199`). -/
  fringe : FringeSet
  /-- The state the block derived for its fringe (`BlockMetadata.fringe_state_hash`,
      `models/src/block_metadata.rs:200`, computed at `casper/src/interpreter_util.rs:766-767`). -/
  stateHash : StateHash
  deriving DecidableEq

/-- The claims of the blocks that finalised a given fringe: the **per-block view** that replaces the
    keyed record. The store held one record per fringe; the DAG holds one claim per block and *finds*
    the blocks of a fringe by `member_of_fringe` (`casper/src/dag.rs:887-895`), which is this filter
    read the other way round. -/
def claimsOf (claims : Finset BlockClaim) (f : FringeSet) : Finset BlockClaim :=
  claims.filter (fun c => c.fringe = f)

/-- Membership in `claimsOf` is exactly "a claim for this fringe". -/
theorem mem_claimsOf {claims : Finset BlockClaim} {f : FringeSet} {c : BlockClaim} :
    c ∈ claimsOf claims f ↔ c ∈ claims ∧ c.fringe = f := by
  simp [claimsOf]

/-- **The node's per-block write pattern: a block determines the claim it carries.** `BlockMetadata` is
    uniquely identified by its block hash (`models/src/block_metadata.rs:205-207`: the `Hash` impl
    hashes `block_hash` and nothing else) and is written once at that key (`casper/src/dag.rs:887-895`),
    so no two claims of one block can differ. This is the invariant the keyed store never had: there the
    identity was the fringe (`models/src/fringe_data.rs:31-33`), which many blocks share. -/
def BlockDetermined (claims : Finset BlockClaim) : Prop :=
  ∀ c₁ ∈ claims, ∀ c₂ ∈ claims, c₁.block = c₂.block → c₁ = c₂

/-- **The claim a block carries, built from the node's two per-block facts** — the functions that give a
    block its fringe and its derived state. `BlockMetadata` supplies both under the block's hash, so a
    claim set built this way is the node's own. -/
def claimOf (fringeOf : BlockId → FringeSet) (stateHashOf : BlockId → StateHash)
    (block : BlockId) : BlockClaim :=
  { block, fringe := fringeOf block, stateHash := stateHashOf block }

/-- **The invariant is not vacuous** — the non-vacuity check on `BlockDetermined`. A claim set built by
    the node's write pattern (`claimOf` over any set of blocks) satisfies it: two of its claims with
    equal blocks are the same claim, because a claim's block *is* the id it was built at. -/
theorem claims_from_blocks_are_block_determined (bs : Finset BlockId)
    (fringeOf : BlockId → FringeSet) (stateHashOf : BlockId → StateHash) :
    BlockDetermined (bs.image (claimOf fringeOf stateHashOf)) := by
  intro c₁ h₁ c₂ h₂ hb
  obtain ⟨b₁, _, rfl⟩ := Finset.mem_image.mp h₁
  obtain ⟨b₂, _, rfl⟩ := Finset.mem_image.mp h₂
  have hb' : b₁ = b₂ := hb
  rw [hb']

/-- **Law 68's version rule on the new shape: the state is named by the block that carries it.** A read
    names a block — a version — and two claims from that block agree on the state it derived, so the
    version the reader names determines the value it reads. This is exactly what the keyed store could
    not offer: there the version was the fringe, and the value also depended on the merge base the key
    never mentioned (`the_fringe_only_key_is_incomplete`). The `BlockDetermined` hypothesis is the node's
    own write pattern (`casper/src/dag.rs:887-895`), not a convenience — `claimOf` and
    `claims_from_blocks_are_block_determined` above show it holds of the claims the node actually
    writes. -/
theorem the_state_is_named_by_a_block {claims : Finset BlockClaim} (h : BlockDetermined claims)
    {c₁ c₂ : BlockClaim} (h₁ : c₁ ∈ claims) (h₂ : c₂ ∈ claims) (hb : c₁.block = c₂.block) :
    c₁.stateHash = c₂.stateHash := by
  rw [h c₁ h₁ c₂ h₂ hb]

/-- **A disagreement is visible: the blocks distinguish what the key merged.** Two claims of one fringe
    whose states differ are carried by **different blocks** — there is no key that merges them, because
    the claim travels with its block and a block's claim is written once.

    This is the positive counterpart of `the_fringe_only_key_is_incomplete`, and what it states is what
    that refutation means: **the ambiguity Law 66 describes was created by the store** — one key, many
    bases — and cannot arise when the claim travels with its block. The two claims below sit at one
    fringe (they share the old key `fringeHashOf f`, so a store keyed by it would fold them together)
    but name distinct blocks, so a reader that names a block reads exactly one of them — the claim that
    block derived, not a `min` of two. The measured divergence — four fringes on every node of an
    uninjected rig, two states each
    (`spec/audit/evidence/n-anchor-drill/self-reject-stall.txt`, C270) — is this pair, and under the new
    shape its two states belong to two blocks. -/
theorem a_disagreement_is_visible {claims : Finset BlockClaim} (h : BlockDetermined claims)
    {f : FringeSet} {c₁ c₂ : BlockClaim}
    (h₁ : c₁ ∈ claimsOf claims f) (h₂ : c₂ ∈ claimsOf claims f)
    (hs : c₁.stateHash ≠ c₂.stateHash) :
    c₁.block ≠ c₂.block := by
  intro hb
  exact hs (the_state_is_named_by_a_block h (mem_claimsOf.mp h₁).1 (mem_claimsOf.mp h₂).1 hb)

end Rchain
