import Rchain.RSpace.Merkle
import Rchain.Casper.Record

/-!
# A read is a function of the version it names

The atomicity a snapshot must satisfy: **a read returns a version, and the version determines what is
read.** A *version* here is a key — for the state half a content-addressed hash, for the record store
the fringe hash alone. The property is deliberately stated about the *key* and not about a moment in
time: two reads of one version agree, whatever happened to the store in between.

## Why this is the shape the reconciliation tool needs

`tools/reconcile-network.sh` copies a node's data directory and is labelled a **fiat** wherever it
appears, in its own words: *"a filesystem-level copy of a live LMDB is a torn snapshot"* (`:456-457`).
A copy taken while the store is being written can carry one page from before a commit and another from
after it, so the copy is not a version of anything a writer ever wrote. This file is about exactly that
question, and its answer is a dichotomy: a read is safe to take from a copy **iff** what it reads is
determined by the key it names rather than by the moment the copy was taken.

## The state half is anchored; the record half is not

Law 10 (`spec/Rchain/RSpace/Merkle.lean`) proves the history is a content-addressed radix trie:
`root_collision_free` says the root determines the node on well-formed nodes, so a read keyed by a hash
reads a fixed version **by construction** — the key *is* the hash of the value, and the trie refuses a
colliding write (`rspace/src/history/radix_tree.rs:208-223,226-258`). That is the positive half below
(`two_content_addressed_reads_of_one_version_agree`), and it is nearly immediate precisely because the
content-addressed store's key already is the value's identity.

The half that is **not** anchored is every store keyed by something else, and the node's record store
is exactly that today (`spec/Rchain/Casper/Record.lean`): the key is the fringe hash alone
(`models/src/fringe_data.rs:31-35` — the `Hash` impl hashes only `fringe_hash`), and the value it holds
also depends on the merge base the merge started at, which the key never mentions (`casper/src/dag.rs:820-850`,
the read-then-put of `fringe_data_store`; `casper/src/storage.rs:52`, the `fringe-data` DB).
`the_fringe_only_key_is_incomplete` already proves the key cannot tell two such records apart; this file
says what that costs a **reader**, which is the half the node feels: the version no longer determines
the value.

## The consequence, plainly

With the composite key the fix specifies (`writeKey`, the fringe **and** the base — `Record.lean`), the
record store's key names everything its value is built from, so the store joins the content-addressed
class and the positive theorem holds for it too: a read becomes a function of the version it names.
**Until that keying is in place, a copy of the record store is a copy of a moment and not of a
version** — which is why the tool stops the network before taking one (`--restore-from-master`, the
fiat printed at `tools/reconcile-network.sh:456-470`). Read as a reader's problem, that is register row
**C270**: a node that reads a joined value at a fringe key is reading a "version" that names no value,
and its own validation refuses its own block. And it is the cost **C250**'s residue predicted: "the
merge for a fringe is a function of the DAG" is true, and it is still not a function of the *key* the
store reads by.

Nothing here edits another file: `Merkle.lean` supplies the content-addressed half, `Record.lean` the
refuting witness. The model reuses `Node`, `Item`, `WellFormed`, `nodeHash` and `root_collision_free`
from the former, and `FringeRecord`, `recordOf`, `key` and the witness of
`the_fringe_only_key_is_incomplete` from the latter, rather than re-deriving either.
-/

namespace Rchain

/-! ## The model — a store, a version, a read

A **store** is modelled as the code's is: a finite map from keys to values, read by a `get` that is a
partial function — an absent key reads `none`. (`K → Option V` is the finite map as a partial function;
the finite support is carried by how the store is built, not by the type, which is enough for every
statement below — the same way `radix_tree.rs`'s store is read.) A **version** is a key. For the state
half the key type is `Version` (a hash); for the record store it is `Nat` (the fringe hash). -/

/-- A store: a finite map from keys to values, as a partial function (`none` = absent), the shape of an
    LMDB `get`. -/
abbrev Store (K V : Type) := K → Option V

/-- **A version** — the key a read names. The state half's versions are hashes (`Hash`, 32 bytes); the
    record store's are fringe hashes. A version is a *key*, and that is the whole point of this file. -/
abbrev Version := Hash

/-- The read: `get s k` — the value the store holds at version `k`, or `none` if it holds none. -/
def read {K V : Type} (s : Store K V) (k : K) : Option V := s k

/-- The write: set the value at `k`, leaving every other key as it was. This is the code's `put`
    (`fringe_data_store.put`, `casper/src/dag.rs:845-847`). -/
def put {K V : Type} [DecidableEq K] (s : Store K V) (k : K) (v : V) : Store K V :=
  fun k' => if k' = k then some v else s k'

/-- **`s` is contained in `s'`** — `s'` is `s` with only additions, never a rewrite of a key `s` already
    held. This is the *append-only* write pattern, and it is what a content-addressed store has and a
    record store does not: the trie refuses a colliding write (`radix_tree.rs:208-223`) while
    `join_fringe_records` merges into an existing key (`casper/src/dag.rs:826-843`). -/
def SubStore {K V : Type} (s s' : Store K V) : Prop := ∀ k v, s k = some v → s' k = some v

/-- **What "a function of the version" means, as a property of a read procedure.** A read `R` reads by
    version when it cannot distinguish two stores that agree at the key being read — however they differ
    everywhere else. The property is about a *key*, never about a moment: the moment is the argument the
    read is forbidden to consult. -/
def ReadsByVersion (K V : Type) (R : Store K V → K → Option V) : Prop :=
  ∀ (s s' : Store K V) (k : K), s k = s' k → R s k = R s' k

/-- `read` reads by version: it consults the key's entry and nothing else, so two stores that agree there
    give the same value. This is the headline property in its general form; what the two halves below add
    is whether a *store* can be built so that agreeing at the key is enough to agree on the *value's
    version*. -/
theorem read_reads_by_version (K V : Type) : ReadsByVersion K V (@read K V) :=
  fun _ _ _ h => h

/-! ## The positive half — a content-addressed read is a function of its version -/

/-- **A store is content-addressed** when every value it holds at a key is named by that key: the key is
    the value's hash. This is the property the trie has by construction (`root_collision_free` and the
    collision refusal at `radix_tree.rs:208-223`), and the property the record store lacks. -/
def ContentAddressed (s : Store Version Node) : Prop :=
  ∀ {k : Version} {v : Node}, s k = some v → nodeHash v = k

/-- The general headline, this file's name: **a read is a function of the version it names.** Stated
    against the code's own danger — "whatever happens to the store in between" is any change to any
    other key, because the hypothesis fixes only the one being read. -/
theorem read_is_a_function_of_its_version {K V : Type} (s s' : Store K V) (k : K)
    (h : s k = s' k) : read s k = read s' k :=
  h

/-- A write at another key does not move this read — the model's statement that the read depends on the
    version and, of the store, only that entry. -/
theorem another_versions_write_does_not_move_this_read {K V : Type} [DecidableEq K]
    (s : Store K V) (k k' : K) (v : V) (h : k ≠ k') :
    read (put s k' v) k = read s k := by
  simp only [read, put, if_neg h]

/-- A write at a version makes that version read what was written. -/
theorem read_after_write {K V : Type} [DecidableEq K] (s : Store K V) (k : K) (v : V) :
    read (put s k v) k = some v := by
  simp [read, put]

/-- **The content-addressed writer's store**: write a node at the version its own hash names. This is
    `radix_tree.rs`'s `save_node` in the model — the store is keyed by `nodeHash n`, so the key and the
    value's identity are one. (`noncomputable` for the same reason `nodeHash` is: it calls the modelled
    hash primitive, which has no compiled code, and the specification only ever reasons about it.) -/
noncomputable def storeNode (s : Store Version Node) (n : Node) : Store Version Node :=
  put s (nodeHash n) n

/-- **Reading a version a content-addressed writer wrote returns the node that version names.** Nearly
    immediate: the key is the hash of the value, so the value is at the key by construction. The content
    is not in the proof; it is in what the statement excludes — no read anywhere else, and no intervening
    write, enters it. -/
theorem a_content_addressed_read_returns_what_the_version_names
    (s : Store Version Node) (n : Node) :
    read (storeNode s n) (nodeHash n) = some n := by
  simp [storeNode, read, put]

/-- **Two reads of one version agree — across two different stores.** This is the positive half's real
    content and it is the one theorem here that leans on `Merkle.lean`: a reader of two stores that each
    hold *something* at version `k` and are each content-addressed reads nodes that hash to `k`, so by
    law 10's `root_collision_free` they read the *same* node. The stores may be two moments of one
    history, or two copies taken at different times: the version pins the value regardless, which is
    precisely what a torn copy of a content-addressed store cannot violate for a key it holds whole. -/
theorem two_content_addressed_reads_of_one_version_agree
    {s s' : Store Version Node} {k : Version} {v v' : Node}
    (hs : s k = some v) (hs' : s' k = some v')
    (hw : WellFormed v) (hw' : WellFormed v')
    (hca : ContentAddressed s) (hca' : ContentAddressed s') : v = v' :=
  root_collision_free hw hw' (by rw [hca hs, hca' hs'])

/-- **A read from an append-only store is stable.** If `s'` only adds to `s`, then any key `s` already
    held reads the same value in `s'` — the read is a function of the version across the whole
    extension, not just at one instant. This is the property the trie has and the record store's `put`
    (an overwrite; C215's fix turned it into a *join*) deliberately does not. -/
theorem a_read_of_an_append_only_store_is_stable {K V : Type} {s s' : Store K V}
    (h : SubStore s s') {k : K} {v : V} (hv : s k = some v) : read s' k = some v :=
  h k v hv

/-- **Content-addressing is maintained by writes.** Writing a node at its own hash keeps a store
    content-addressed, so the positive half's property is an invariant of the content-addressed write
    pattern and not an accident of one store. -/
theorem content_addressing_is_preserved_by_a_store_node_write
    {s : Store Version Node} (hca : ContentAddressed s) (n : Node) :
    ContentAddressed (storeNode s n) := by
  intro k v hv
  simp only [storeNode, put] at hv
  by_cases hk : k = nodeHash n
  · rw [if_pos hk] at hv
    rw [Option.some.injEq] at hv
    rw [← hv]
    exact hk.symm
  · rw [if_neg hk] at hv
    exact hca hv

/-! ## The refutation — a store keyed by something else

The record store keys on the fringe hash alone, and — as `Record.lean` shows — that key does not
determine the value: two records the key cannot tell apart differ in the state their merges produced
from two different bases. Here the same fact is read as a statement about a **store**, which is the half
a reader feels.

The model below is deliberately minimal and assumes **no** nondeterminism: `recordOf` is a *function* of
`(fringe, base)` (`Record.lean`), so each store is a fixed, deterministic writing of one record at one
key. What fails is not the write but the key: the "version" the store is read by is the fringe hash, and
two records with one fringe hash hold two states. -/

/-- The store left by writing one record at the code's key for it — the fringe hash alone
    (`key r`, `models/src/fringe_data.rs:31-35`; the write at `casper/src/dag.rs:845-847`). -/
def storeRecord (r : FringeRecord) : Store Nat FringeRecord :=
  fun k => if k = key r then some r else none

/-- Reading a one-record store at the key it was written under returns that record. -/
theorem read_storeRecord_at_its_key (r : FringeRecord) :
    read (storeRecord r) (key r) = some r := by
  simp [read, storeRecord]

/-- **Any key that does not determine its value reads two ways.** The refutation in general form: two
    records the code's key cannot tell apart (`key r₁ = key r₂`) whose states differ are read apart by a
    reader of that key. Parameterised in the pair so that `Record.lean`'s colliding pair feeds straight
    in — `recordOf ∅ 0` and `recordOf ∅ 1`, one fringe, two merge bases, two states — instead of being
    re-derived here. -/
theorem a_key_that_does_not_determine_its_value_reads_two_ways
    (r₁ r₂ : FringeRecord) (hk : key r₁ = key r₂) (hstate : r₁.stateHash ≠ r₂.stateHash) :
    ∃ (s s' : Store Nat FringeRecord) (k : Nat), read s k ≠ read s' k := by
  refine ⟨storeRecord r₁, storeRecord r₂, key r₁, ?_⟩
  rw [read_storeRecord_at_its_key, hk, read_storeRecord_at_its_key]
  intro hc
  exact hstate (congrArg FringeRecord.stateHash (Option.some.inj hc))

/-- **A read of a stale version is not a function of the version** — the headline refutation, and it is
    `Record.lean`'s colliding pair read out of two stores rather than a re-derivation of it: the records
    come from `the_fringe_only_key_is_incomplete`, so the witness is concrete (a fringe, two bases, two
    states) even though it is named existentially here.

    Two stores, each holding one record at the fringe-hash key — the "same version" `k = key r₁ = key r₂`
    — differ **only** in the record stored there, and that record differs only in the merge base its
    state came from; yet the version reads two different values out of them. That is the atom of the
    "torn snapshot" fiat in its non-content-addressed form: a version that no value is a function of is a
    version a copy cannot preserve, because there is nothing there to preserve but the moment the copy
    was taken. -/
theorem a_non_content_addressed_read_is_not_a_function_of_its_version :
    ∃ (s s' : Store Nat FringeRecord) (k : Nat), read s k ≠ read s' k := by
  obtain ⟨r₁, r₂, hk, hstate⟩ := the_fringe_only_key_is_incomplete
  exact a_key_that_does_not_determine_its_value_reads_two_ways r₁ r₂ hk hstate

/-- The same refutation written on one store rather than across two: **one version, two writers, two
    values.** A `put` at the code's key is an overwrite, so which value the version reads depends on
    which writer got there — the hazard C215's `join` was built to remove from the *map*, and which it
    removed only from the map (the value that comes back is still neither writer's; C270). The colliding
    pair is again `Record.lean`'s. -/
theorem one_version_two_writers_two_values (s : Store Nat FringeRecord) (k : Nat) :
    ∃ r₁ r₂ : FringeRecord,
      key r₁ = key r₂ ∧ read (put s k r₁) k ≠ read (put s k r₂) k := by
  obtain ⟨r₁, r₂, hk, hstate⟩ := the_fringe_only_key_is_incomplete
  refine ⟨r₁, r₂, hk, ?_⟩
  rw [read_after_write, read_after_write]
  intro hc
  exact hstate (congrArg FringeRecord.stateHash (Option.some.inj hc))

end Rchain
