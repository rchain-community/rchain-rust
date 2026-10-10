import Rchain.RSpace.Merkle
import Mathlib.Data.List.Basic
import Init.Omega

/-!
# The state transfer — importing the walk of a root yields the state rooted there

**Why this file exists.** A joining node pulls its state from a peer one page at a time. The peer walks
its content-addressed history (a radix trie of `[Item; 256]` nodes, Law 10 in `RSpace/Merkle.lean`) from
the root the join is anchored on, and ships the nodes it visits (`RadixTree.sequentialExport`,
`rspace/src/history/export.rs:352-424`, driven by `RSpaceExporter.traverseHistory`,
`rspace/src/state/mod.rs:181-230`). The joiner must end up with **the state the root names** — not
"whatever the peer sent". The Rust already has the safety half: `validate_state_items`
(`rspace/src/state/mod.rs:239-315`) re-hashes every received node (its key must be the Blake2b256 of its
bytes, `:257-267`), re-runs the walk locally against the store the chunk lands in, and requires the
received key lists to equal the recomputed walk exactly (`:276-300`) — so a wrong page is *refused*, not
imported. What the Rust does not have is a *statement* of what the transfer means, or a theorem that the
walk returns exactly the rooted trie. Law 10 (`RSpace/Merkle.lean`) anchors the trie's
content-addressing; **this module anchors the transfer** — the walk's *correctness given the pages*.

## The three claims, and the shape of the proof

1. **The walk visits exactly the reachable nodes** (`WalkListing` and its theorems): the pages a walk
   delivers are exactly the nodes reachable from the root by `NodePtr` edges, *in some order*, and that
   set is independent of how the stream is cut into pages. The delivered *set* is the claim; the *order*
   of a page, its `skip`/`take` bounds and the `last_path` resume cursor are `sequential_export`'s own
   concern (`export.rs:64-131`, `:206-218`) and are not modelled here.
2. **Reading is determined by the root** (`reads_determined`): any two stores that agree on every node
   reachable from a root read the same value from that root. Reading (`Reads`, mirroring `RadixTree.read`,
   `radix_tree.rs:301-339`) only ever follows `NodePtr` edges, so it never leaves the reachable set — the
   content-addressing collapse (Law 10's `root_collision_free`) is what makes two stores agree there.
3. **The checker is sound** (`import_reads_the_rooted_trie`, the capstone): if the joiner's store is built
   from validated pages — each received node under its own hash, every reachable key delivered — then the
   value the joiner reads at the root **cannot differ** from the value the peer's own store reads there.
   The joiner is not trusting the peer; it is checking it, and the check is *sufficient*.

## What is *not* claimed (the hypotheses are named, not assumed)

* That the walk **terminates** — that every reachable key is present and the traversal finishes — is the
  business of a different module, `Rchain/Sync/Walk.lean` (the register row `C268`). Here it is the
  named hypothesis `WalkDefined` (every reachable key is *present*), carried by the walk's own success;
  `WalkListing`'s *existence* is likewise not proved — finiteness of the reachable set is the hypothesis
  a listing encodes, not a fact assumed.
* The **data (leaf-value) store** is the same shape one layer down and is not modelled: `validate_state_items`
  re-hashes each data item too (`state/mod.rs:303-311`), so the value hashes `Reads` returns are pinned
  the same way; the bytes behind them are pinned by the same Law-10 argument on the data store.
* The pages' **ordering, `skip`/`take` windows and resume path** are `sequential_export`'s; the transfer's
  claim is about the delivered set, not the schedule that delivers it.

The Rust contact points, by declaration:

* `Store` / `StoreCorrect` — the history store `Blake2b256Hash => Vec<u8>`, decoded to nodes; its
  invariant is `save_node`'s collision assert (`radix_tree.rs:241-256`) and `commit`'s refusal
  (`:259-291`), plus `SerializedNode`/`decode`'s framing (`:82-143`).
* `Reaches` — the `NodePtr` edges the walk follows (`add_node_ptr`, `export.rs:206-261`).
* `Reads` — `RadixTree.read` (`radix_tree.rs:301-339`).
* `KeyHonest` — the key check
  `Blake2b256Hash::create(trie_bytes) == hash` (`state/mod.rs:257-267`).
* `Installs` — `set_history_items` (`casper/src/engine/lfs_tuple_space_requester.rs:288-293`).
* `DeliversAll` — the walk-keys-equal-received-keys check (`state/mod.rs:276-300`).
-/

namespace Rchain

namespace Sync

/-! ## 1. The store, and the two things a transfer needs of it -/

/-- A **history store**: the content-addressed node table. The Rust keys serialized bytes by
    `Blake2b256Hash`; here the node is already the `decode`d `Node` (`radix_tree.rs:115-143`), so the
    store's keys are the `nodeHash`es `Merkle.lean` defines. A history store and the `RadixTreeImpl`
    that reads it are the same object under different names (`radix_tree.rs:152-155`). -/
abbrev Store := Hash → Option Node

/-- A **key**: the byte string the trie indexes (`KeySegment`, in the Rust; the model keeps the bytes).
    A `Byte` is `Fin 256` (`Crypto/Spec.lean`), so a slot index is a key's first byte. -/
abbrev Key := List Byte

/-- A hash is **present** in a store when the store holds a node under it. A `NodePtr` may name a hash the
    store does not hold — the walk treats that as an error (`export.rs:216-217`, "Node with key not
    found"), which is why "reachable" and "present" are different predicates here. -/
def Present (S : Store) (h : Hash) : Prop := ∃ n, S h = some n

/-- **Content addressing, as the store's invariant.** Every node the store holds is filed under its own
    hash — the property `save_node`/`commit` maintain by refusing a colliding write
    (`radix_tree.rs:241-256`, `:259-291`) and `decode` establishes for the width and payload
    (`:115-143`). The `WellFormed` half is exactly `Merkle.lean`'s trie invariant (`Node` of 256 slots,
    32-byte values, sub-128 prefixes), which `decode` gives structurally. `root_collision_free` is stated
    for well-formed nodes, so the invariant must carry it. -/
def StoreCorrect (S : Store) : Prop := ∀ h n, S h = some n → nodeHash n = h ∧ WellFormed n

/-! ## 2. The reachable nodes — a walk of a root visits these and no others -/

/-- **Reachability by `NodePtr` edges.** `Reaches S a b` holds when, starting at node-hash `a`, following
    the `NodePtr` slots of the nodes `S` holds leads to node-hash `b`. This is the relation the export
    walk is a traversal *of*: `add_node_ptr` descends a `NodePtr { prefix, ptr }` into `S ptr`
    (`export.rs:206-261`), so the nodes a walk of `r` can emit are exactly `{b | Reaches S r b}`. -/
inductive Reaches (S : Store) : Hash → Hash → Prop where
  /-- The walk of a root visits the root. -/
  | refl (h : Hash) : Reaches S h h
  /-- …and descends every `NodePtr` it finds (`hmem` is a `NodePtr { _, ptr }` slot of the node at `h`). -/
  | step {h : Hash} {n : Node} {pref : List Byte} {ptr b : Hash}
      (hS : S h = some n) (hmem : Item.node pref ptr ∈ n) (hrec : Reaches S ptr b) :
      Reaches S h b

/-- **Reachability composes.** A walk that reaches `b` and then continues to `c` reaches `c`; the reachable
    set is closed under the edge relation, which is what makes `Closed` below hold of `{b | Reaches S r b}`.
    The endpoint is generalised so the induction's indices line up. -/
theorem Reaches.trans {S : Store} {a b c : Hash}
    (hab : Reaches S a b) (hbc : Reaches S b c) : Reaches S a c := by
  induction hab generalizing c with
  | refl _ => exact hbc
  | step hS hmem _ ih => exact Reaches.step hS hmem (ih hbc)

/-- **A store whose nodes hold no `NodePtr` at all reaches only its root.** This is the small, fully
    explicit witness that `Reaches` is not vacuous and that `WalkListing` below is inhabited: for the
    empty trie, and for any trie whose nodes are built entirely from leaves, the walk's listing is `[r]`.
    The hypothesis speaks about the whole store (every node, not just the root) so that the induction
    generalises the root freely. -/
theorem Reaches.eq_root_of_no_nodeptr {S : Store} {r k : Hash}
    (hnode : ∀ (h : Hash) (n : Node), S h = some n →
      ∀ (pref : List Byte) (ptr : Hash), Item.node pref ptr ∉ n)
    (hk : Reaches S r k) : k = r := by
  induction hk with
  | refl _ => rfl
  | step hS hmem _ _ => exact absurd hmem (hnode _ _ hS _ _)

/-- **The walk is defined**: every node reachable from the root is present. This is the walk's *success*
    — the thing that makes the traversal terminate and the export emit a listing — and it is a
    **hypothesis**, not a theorem, because termination is `Rchain/Sync/Walk.lean`'s business (row `C268`).
    A peer whose trie has a dangling `NodePtr` fails here, and the Rust says so (`export.rs:216-217`). -/
def WalkDefined (S : Store) (r : Hash) : Prop := ∀ k, Reaches S r k → Present S k

/-! ## 3. The walk's listing — exactly the reachable nodes, whatever the pages are, however cut -/

/-- **A walk listing**: a duplicate-free `List Hash` whose elements are exactly the reachable nodes. The
    walk's `node_keys` stream (`export.rs:237-245`) is such a listing — the traversal emits each node once,
    on arrival. Its *existence* is `WalkDefined` plus finiteness of the reachable set; this structure does
    not assert existence, it *characterises* a listing, so that "the pages deliver exactly the reachable
    nodes" is a statement about the set and not about a particular serialization order. -/
structure WalkListing (S : Store) (r : Hash) (keys : List Hash) : Prop where
  /-- The walk emits a node at most once. -/
  nodup : keys.Nodup
  /-- Every emitted key is reachable from the root. -/
  sound : ∀ k, k ∈ keys → Reaches S r k
  /-- Every reachable key is emitted. -/
  complete : ∀ k, Reaches S r k → k ∈ keys

/-- **Any two listings name the same nodes.** The set a walk delivers is the reachable set, and that does
    not depend on the order a particular walk found them in (`WalkListing`'s two halves, read one against
    the other). -/
theorem WalkListing.same_elements {S : Store} {r : Hash} {L₁ L₂ : List Hash}
    (w₁ : WalkListing S r L₁) (w₂ : WalkListing S r L₂) (k : Hash) :
    k ∈ L₁ ↔ k ∈ L₂ :=
  ⟨fun h => w₂.complete k (w₁.sound k h), fun h => w₁.complete k (w₂.sound k h)⟩

/-- **The pages of a walk deliver exactly the reachable nodes, however the stream is cut.** Given a listing
    `L` and *any* pagination `pages` of it (`pages.join = L` — the concatenation of the chunks equals the
    whole walk, which is what the `last_path` resume cursor guarantees, `export.rs:219-228`,
    `state/mod.rs:201-210`), an element is in some page iff it is reachable from the root. Nothing here
    constrains the *number* of pages, their *order*, or how many nodes each holds: the page cut is free. -/
theorem WalkListing.pages_are_the_nodes {S : Store} {r : Hash} {L : List Hash}
    (w : WalkListing S r L) {pages : List (List Hash)} (hpages : pages.join = L) (k : Hash) :
    k ∈ pages.join ↔ Reaches S r k := by
  rw [hpages]
  exact ⟨w.sound k, w.complete k⟩

/-- **A `WalkListing` is inhabited** — the definitions are not vacuous. When the root's node (and indeed
    every node) carries no `NodePtr`, the reachable set is `{r}` and `[r]` lists it exactly. -/
theorem WalkListing.exists_of_leafless {S : Store} {r : Hash}
    (hnode : ∀ (h : Hash) (n : Node), S h = some n →
      ∀ (pref : List Byte) (ptr : Hash), Item.node pref ptr ∉ n) :
    WalkListing S r [r] := by
  refine ⟨List.nodup_cons.mpr ⟨by simp, List.nodup_nil⟩, ?_, ?_⟩
  · intro k hk
    rw [List.mem_singleton] at hk
    rw [hk]
    exact Reaches.refl r
  · intro k hk
    exact List.mem_singleton.mpr (Reaches.eq_root_of_no_nodeptr hnode hk)

/-! ## 4. Reading — and why it is determined by the root

`Reads S r key v` mirrors `RadixTree.read` (`radix_tree.rs:301-339`): walk the key byte by byte. A key's
first byte names the slot; an `Item.leaf { prefix, value }` there is a hit when its `prefix` is the rest of
the key (`read`'s `leaf_prefix == prefix.tail()`, `:318-323`), and an `Item.node { prefix, ptr }` descends
into `S ptr` when its `prefix` matches the next bytes of the key (`:324-336`, `common_prefix` +
`ptr_prefix_rest.is_empty()`). A slot that is `Item.empty`, a leaf whose prefix misses, or a node whose
prefix misses all read *nothing*; those "miss" cases are absent from `Reads` (a read that yields nothing
has no derivation), which is why a `Reads` derivation is only ever a *success*. The read returns a **value
hash** — the leaf's `value` — not bytes; the bytes are the data store's business (see the header). -/

/-- **Reading a key from the trie rooted at node-hash `r` in store `S` yields the value hash `v`.** The key
    is an index and the decomposition into `slot byte` + `remainder` is a *hypothesis* (`hkey`), not a
    computation, so that `cases`/`induction` over a derivation never has to invert an append — the same
    trick `epsilon`-free syntax trees use everywhere else in this tree. -/
inductive Reads (S : Store) : Hash → Key → Hash → Prop where
  /-- The key selects slot `b`; its leaf's prefix is the key's remainder — a hit (`:318-323`). -/
  | leaf (r : Hash) (n : Node) (key : Key) (b : Byte) (pref : List Byte) (v : Hash)
      (hS : S r = some n) (hkey : key = b :: pref)
      (hslot : n.get? b.val = some (Item.leaf pref v)) : Reads S r key v
  /-- The key selects slot `b` holding a `NodePtr` whose prefix matches, and the remainder reads below it
      (`:324-336`). -/
  | node (r : Hash) (n : Node) (key rest : Key) (b : Byte) (pp : List Byte) (ptr : Hash) (v : Hash)
      (hS : S r = some n) (hkey : key = b :: (pp ++ rest))
      (hslot : n.get? b.val = some (Item.node pp ptr)) (hrec : Reads S ptr rest v) :
      Reads S r key v

/-- **`Reads` is inhabited** — the read relation is not vacuous: a store that files a leaf at slot `b`
    reads the key `b :: pref`. (`Reads.leaf`'s explicit arguments are spelled out, mirroring a caller that
    knows the node and the slot; real calls leave them to unification.) -/
theorem reads_single_leaf {S : Store} {r : Hash} {n : Node} {b : Byte} {pref : List Byte} {v : Hash}
    (hS : S r = some n) (hslot : n.get? b.val = some (Item.leaf pref v)) :
    Reads S r (b :: pref) v :=
  Reads.leaf r n _ b pref v hS rfl hslot

/-- **A set of hashes is closed under `NodePtr` edges.** `Closed S A` says: every node's `NodePtr` targets
    lie in `A`. The reachable set is closed (next theorem) and the empty one is not — closure is what lets
    a reading walk stay inside `A`, so agreement on `A` is enough to pin a read down. -/
def Closed (S : Store) (A : Set Hash) : Prop :=
  ∀ h, h ∈ A → ∀ n, S h = some n →
    ∀ (pref : List Byte) (ptr : Hash), Item.node pref ptr ∈ n → ptr ∈ A

/-- **The reachable set is closed.** If `h` is reachable from `r`, every `NodePtr` of `h`'s node targets a
    hash also reachable from `r` (`Reaches.step`), so `{b | Reaches S r b}` is closed. -/
theorem reach_set_closed {S : Store} {r : Hash} :
    Closed S {k | Reaches S r k} := by
  intro h hh n hS pref ptr hmem
  exact Reaches.trans hh (Reaches.step hS hmem (Reaches.refl ptr))

/-- **Reading is determined by the root — the checker's soundness, in the abstract.** Let `S` and `S'` be two
    stores that agree on every node reachable from `a` (`hag`), and let `S`'s reachable set be closed
    (`hclosed`). Then any two successful reads of the same key from the same root give the **same value**.
    This is the content of Law 10 applied along a read: at every `NodePtr` the two stores hold *a node under
    the same hash*, and `root_collision_free` forces those nodes equal, so the descent is identical. A read
    only visits nodes reachable from its root, so agreement *there* is all that is needed.

    (Correctness of `S` — `StoreCorrect` — is *not* a hypothesis here: agreement `hag` already supplies the
    node equality at each step. Correctness is what *establishes* `hag` for an import; that is
    `import_agrees` below.) -/
theorem reads_determined {S S' : Store} {A : Set Hash}
    (hclosed : Closed S A) (hag : ∀ k, k ∈ A → S k = S' k) :
    ∀ (a : Hash), a ∈ A → ∀ {key : Key} {v : Hash}, Reads S a key v →
      ∀ {v' : Hash}, Reads S' a key v' → v = v' := by
  intro a ha key v h
  induction h with
  | leaf r n k b pref v hS hkey hslot =>
      intro v' h'
      cases h' with
      | leaf r' n' k' b' pref' v' hS' hkey' hslot' =>
          -- The two stores hold the same node under `r` (agreement at the root).
          have hn : n = n' := Option.some.inj (by rw [← hag r ha] at hS'; rw [hS] at hS'; exact hS')
          obtain ⟨hb, _⟩ := List.cons.inj (show b :: pref = b' :: pref' by rw [← hkey, hkey'])
          rw [← hn] at hslot'
          rw [← hb] at hslot'
          rw [hslot] at hslot'
          exact (Item.leaf.inj (Option.some.inj hslot')).2
      | node r' n' k' _ b' _ _ v' hS' hkey' hslot' _ =>
          -- A leaf and a node cannot both occupy the slot: `some (Item.leaf …) = some (Item.node …)`.
          have hn : n = n' := Option.some.inj (by rw [← hag r ha] at hS'; rw [hS] at hS'; exact hS')
          obtain ⟨hb, _⟩ := List.cons.inj (show b :: pref = b' :: (_ ++ _) by rw [← hkey, hkey'])
          rw [← hn] at hslot'
          rw [← hb] at hslot'
          rw [hslot] at hslot'
          simp at hslot'
  | node r n k rest b pp ptr v hS hkey hslot _ ih =>
      intro v' h'
      cases h' with
      | leaf r' n' k' b' pref' v' hS' hkey' hslot' =>
          -- Dually, a node cannot be a leaf.
          have hn : n = n' := Option.some.inj (by rw [← hag r ha] at hS'; rw [hS] at hS'; exact hS')
          obtain ⟨hb, _⟩ := List.cons.inj (show b :: (pp ++ rest) = b' :: pref' by rw [← hkey, hkey'])
          rw [← hn] at hslot'
          rw [← hb] at hslot'
          rw [hslot] at hslot'
          simp at hslot'
      | node r' n' k' rest' b' pp' ptr' v' hS' hkey' hslot' hrec' =>
          -- Same node at the root, same slot, same `NodePtr`: same `pp`, same `ptr`, same remainder —
          -- so the reads below agree by the induction hypothesis, taken at `ptr` (reachable, hence in `A`).
          have hn : n = n' := Option.some.inj (by rw [← hag r ha] at hS'; rw [hS] at hS'; exact hS')
          obtain ⟨hb, hkeyeq⟩ :=
            List.cons.inj (show b :: (pp ++ rest) = b' :: (pp' ++ rest') by rw [← hkey, hkey'])
          rw [← hn] at hslot'
          rw [← hb] at hslot'
          have hsome : some (Item.node pp ptr) = some (Item.node pp' ptr') := by rw [← hslot, hslot']
          obtain ⟨hpp, hptr⟩ := Item.node.inj (Option.some.inj hsome)
          rw [← hpp] at hkeyeq
          have hrest : rest = rest' := List.append_cancel_left hkeyeq
          rw [← hrest] at hrec'
          rw [← hptr] at hrec'
          exact ih (hclosed r ha n hS pp ptr (List.get?_mem hslot)) hrec'

/-- **The converse direction — a read transports across agreement.** If `S'` holds the same node as `S`
    everywhere in a closed set `A` containing `a`, then *every* read `S` performs from `a` is one `S'`
    performs too, with the same value. `reads_determined` says agreement *cannot differ*; this says the
    joiner's store also *has* the state — it is not a store that merely refuses to contradict. Proved by
    walking the derivation, replacing the root node at each level (`hag r ha`) and re-entering the child
    through `A`'s closure. -/
theorem Reads.congr {S S' : Store} {A : Set Hash}
    (hclosed : Closed S A) (hag : ∀ k, k ∈ A → S k = S' k) :
    ∀ (a : Hash), a ∈ A → ∀ {key : Key} {v : Hash}, Reads S a key v → Reads S' a key v := by
  intro a ha key v h
  induction h with
  | leaf r n k b pref v hS hkey hslot =>
      exact Reads.leaf r n k b pref v (by rw [← hag r ha, hS]) hkey hslot
  | node r n k rest b pp ptr v hS hkey hslot _ ih =>
      exact Reads.node r n k rest b pp ptr v (by rw [← hag r ha, hS]) hkey hslot
        (ih (hclosed r ha n hS pp ptr (List.get?_mem hslot)))

/-! ## 5. The import — the checks, and what they buy -/

/-- **The key check** — `validate_state_items`' re-hash (`state/mod.rs:257-267`). Each received history item
    is the claim "the node under this key is these bytes"; the check is that the bytes hash to the key. The
    Rust *implements* this, it is not an assumption of the model; here it is the import's obligation,
    stated over the received items. -/
def KeyHonest (items : List (Hash × Node)) : Prop := ∀ k n, (k, n) ∈ items → nodeHash n = k

/-- **What the import installs** — `set_history_items` (`lfs_tuple_space_requester.rs:288-293`): every
    received item lands in the target store under its key. -/
def Installs (S' : Store) (items : List (Hash × Node)) : Prop := ∀ k n, (k, n) ∈ items → S' k = some n

/-- **The walk-vs-received check** — `validate_state_items`' key-list equality (`state/mod.rs:276-300`):
    the walk recomputed from the received items delivers exactly the keys the walk of the root names, so
    every node reachable from `r` arrives. (The Rust compares the *sequences* the recomputed walk emits;
    this states the consequent — that the reachable set is covered — which is what the joiner needs.) -/
def DeliversAll (items : List (Hash × Node)) (S : Store) (r : Hash) : Prop :=
  ∀ k, Reaches S r k → ∃ n, (k, n) ∈ items

/-- **The key check makes the imported store agree with the peer's on the reachable nodes.** Suppose the
    peer's store is content-addressed (`hS`), the walk is defined (`hdef`), each received node is honest
    (`hh`) and well-formed (`hwf`, from `decode`), the items are installed (`hinst`), and every reachable
    key arrives (`hdel`). Then for every `k` reachable from `r`, the joiner's store holds what the peer's
    does: `S k = S' k`.

    The proof is Law 10 once per node. Reachability gives `Present S k`, so the peer has a node `n₀` under
    `k`; the check gives the received node `n` with `nodeHash n = k`; the peer's `StoreCorrect` gives
    `nodeHash n₀ = k`; `root_collision_free` (both well-formed) forces `n₀ = n`; and `hinst` files it. This
    is exactly `validate_state_items`: the key check plus the walk-agreement turn "the bytes the peer sent"
    into "the node the root names". -/
theorem import_agrees {S S' : Store} {items : List (Hash × Node)} {r : Hash}
    (hS : StoreCorrect S) (hdef : WalkDefined S r)
    (hh : KeyHonest items) (hwf : ∀ k n, (k, n) ∈ items → WellFormed n)
    (hinst : Installs S' items) (hdel : DeliversAll items S r) :
    ∀ k, Reaches S r k → S k = S' k := by
  intro k hk
  obtain ⟨n₀, hn₀⟩ := hdef k hk
  obtain ⟨n, hn⟩ := hdel k hk
  have h1 : nodeHash n₀ = k := (hS k n₀ hn₀).1
  have h2 : nodeHash n = k := hh k n hn
  have hn_eq : n₀ = n := root_collision_free (hS k n₀ hn₀).2 (hwf k n hn) (by rw [h1, h2])
  rw [hn₀, hn_eq]
  exact (hinst k n hn).symm

/-- **The capstone — importing the walk of a root yields the state rooted there.** If the joiner's store is
    built by an import that passes the checks (`hh`, `hwf`, `hdel`, `hinst`) from a content-addressed peer
    (`hS`) whose walk of `r` is defined (`hdef`), then for any key `key`, the value `v'` the joiner reads at
    the root `r` **equals** the value `v` the peer's own store reads there. The joiner cannot be handed a
    different state: `import_agrees` gives agreement on the reachable nodes, and `reads_determined` (with
    `reach_set_closed`) collapses the read. -/
theorem import_reads_the_rooted_trie {S S' : Store} {items : List (Hash × Node)} {r : Hash}
    (hS : StoreCorrect S) (hdef : WalkDefined S r)
    (hh : KeyHonest items) (hwf : ∀ k n, (k, n) ∈ items → WellFormed n)
    (hinst : Installs S' items) (hdel : DeliversAll items S r)
    {key : Key} {v v' : Hash} (hTrue : Reads S r key v) (hJoin : Reads S' r key v') :
    v = v' :=
  reads_determined reach_set_closed (import_agrees hS hdef hh hwf hinst hdel)
    r (Reaches.refl r) hTrue hJoin

/-- **…and it is not merely consistent — the joiner's store *answers*.** Under the same checks, every read
    the peer's store performs at the root is a read the joiner's store performs, to the same value
    (`Reads.congr` over `import_agrees`). Together with `import_reads_the_rooted_trie` this is the full
    statement: the imported store reads exactly what the rooted trie reads. -/
theorem import_preserves_reads {S S' : Store} {items : List (Hash × Node)} {r : Hash}
    (hS : StoreCorrect S) (hdef : WalkDefined S r)
    (hh : KeyHonest items) (hwf : ∀ k n, (k, n) ∈ items → WellFormed n)
    (hinst : Installs S' items) (hdel : DeliversAll items S r)
    {key : Key} {v : Hash} (h : Reads S r key v) : Reads S' r key v :=
  Reads.congr reach_set_closed (import_agrees hS hdef hh hwf hinst hdel)
    r (Reaches.refl r) h

/-! ## 6. What is not claimed

* **Termination.** `WalkDefined` (every reachable key present) and `WalkListing`'s existence are named
  hypotheses, not theorems: that the walk of a *finite* history finishes is `Rchain/Sync/Walk.lean`, the
  register row `C268`. Nothing here assumes finiteness silently — a store with a cycle simply has no
  `WalkListing` and no `WalkDefined`, and every theorem above is vacuous there, which is the honest
  reading.
* **The data store.** `Reads` returns leaf *value hashes*. That the bytes behind them are what the peer
  holds is the same Law-10 argument one layer down, on the data items the checker re-hashes
  (`state/mod.rs:303-311`); it is not restated here.
* **The pages' schedule.** `WalkListing.pages_are_the_nodes` holds for *any* cut with `pages.join = L`;
  the `skip`/`take` windowing and the `last_path` resume that produce the cut are `sequential_export`'s
  (`export.rs:64-131`, `:206-218`) and `LfsTupleSpaceState`'s (`lfs_tuple_space_requester.rs:196-241`).
-/

end Sync

end Rchain
