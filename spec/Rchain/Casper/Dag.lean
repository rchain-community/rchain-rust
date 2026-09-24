import Rchain.Casper.Fringe

/-!
# The DAG, and the fringe it derives (Law 14b)

`Rchain/Casper/Fringe.lean` models a fringe as a **value** — a set of messages — and proves the
refutation of the unrestricted antichain (`fringe_antichain_is_false`: two messages from one sender
refute "same sender ⇒ same id" with no hypothesis to appeal to). What law 14b is about is the fringe
the finalizer **derives**, and the derivation is four steps with a gate on each side of it
(`block-storage/src/dag/finalizer.rs`):

1. **`selfParents`** (`:74-95`) — the walk: a justification's ancestors of the **same sender** that are
   not already finalized (the previous fringe's messages are excluded);
2. **`minMsgs`** — per justification, the **oldest** such ancestor (`next_fringe`, `:186-211`, takes
   `chain.into_iter().last()` of `[p] ++ self_parents p`);
3. **`checkMinMessages`** (`:99`) — the count gate: `min_msgs.len() == bonds_map.len()`. Count-only, and
   the upstream epoch TODO in its body is recorded in law 14a's row as **fidelity rather than
   oversight**;
4. **`nextLayer`** (`:109-127`) — a **fold over the min messages**, keyed by sender, keeping the entry
   with the higher `sender_seq`; then the stake gate (`calculate_fringe`, `:165-184`) decides whether
   the layer is published (`:211`).

**The antichain comes from step 4's fold, not from the map it is stored in.** The port keeps the layer
in a `BTreeMap<sender, Message>`, which gives distinct senders for free — and modelling the map would
make law 14b true by construction, which is the shape G6 had to refuse one unit earlier: a rule that
holds by construction is a rule nothing can falsify. So the layer here is the **derivation's own data**,
a list written by that fold, and the antichain is stated *about the fold*. The mutation that falsifies
it is **one word** — `layerInsert`'s `if (l.any …)` guard dropped, so that the fold appends every
candidate instead of keeping one per sender — and it fails `derivedFringe_antichain` on a DAG with two
messages from one sender.

**What the theorem earns, and what it does not**: pairwise-distinct **senders** is what the derivation
gives. "One message per **bonded** validator" additionally rests on `checkMinMessages`' count
comparison — the epoch TODO above — so the statement is named for what it proves.

The stake gate's *content* is law 14a's (`Rchain.Casper.Stake`'s `calculateFringe`, with its boundary
theorems), so the support map enters `derivedFringe` as an argument, exactly as it enters
`Rchain.nextFringe`: this module is about the walk and the layer, not about the stake.
-/

namespace Rchain

/-- The DAG: messages by id, the port's `msg_map` (`block-storage/src/dag/finalizer.rs:23-33`). -/
abbrev Dag := List Message

/-- A message by id — the port's `Finalizer::msg`. -/
def Dag.msg (d : Dag) (id : Nat) : Option Message := d.find? (·.id = id)

/-- **Step 1 — `self_parents`** (`finalizer.rs:74-95`): the same-sender, not-yet-finalized ancestors
    reachable from `mv`, in walk order (newest first, since the port pops a worklist seeded with `mv`'s
    direct parents).

    The fuel is the DAG's own size: every step pops one message and folds in its parents, so `d.length`
    steps cannot be exceeded — a bound justified by the walk rather than chosen, which is what keeps
    this a definition and not a partiality. -/
def selfParents (d : Dag) (mv : Message) (finalized : List Nat) : List Message :=
  go d.length (sameSenderParents d mv.sender mv finalized) []
where
  /-- `mv`'s direct parents, of the same sender, not already finalized. -/
  sameSenderParents (d : Dag) (sender : Nat) (mv : Message) (finalized : List Nat) : List Message :=
    (mv.parents.filterMap (Dag.msg d)).filter
      (fun x => x.sender = sender && decide (x.id ∉ finalized))
  go : Nat → List Message → List Message → List Message
    | 0, _, acc => acc
    | _, [], acc => acc
    | fuel + 1, m :: rest, acc =>
      go fuel (sameSenderParents d m.sender m finalized ++ rest) (m :: acc)

/-- **Step 2 — the min messages** (`next_fringe`, `:186-211`): per justification, the *oldest*
    non-finalized same-sender ancestor, with the justification itself when there is none (the port's
    `chain.into_iter().last()` over `[p] ++ self_parents p`). -/
def minMsgs (d : Dag) (js : List Message) (finalized : List Nat) : List Message :=
  js.map fun p =>
    match (selfParents d p finalized).getLast? with
    | some m => m
    | none => p

/-- **Step 3 — `check_min_messages`** (`:99`): the count gate. Its body is a count comparison, which is
    the upstream epoch TODO that law 14a's row records as fidelity rather than oversight. -/
def checkMinMessages (ms : List Message) (bonds : Bonds) : Bool := ms.length == bonds.length

/-- **Step 4 — the layer fold** (`calculate_next_layer`, `:109-127`), as the derivation's own data:
    keyed by sender. An incoming message **replaces** whatever the list already holds for its sender —
    the port's `if m.sender_seq > curr.sender_seq` insertion, as a mutation. Which of two same-sender
    messages wins is the fold's own order here where the port compares `sender_seq`; **the antichain
    does not depend on that choice**, which is why the model may take the simpler one. What it must not
    do is keep both — and that is the one word the falsifier drops: `m :: l.filter …` → `m :: l`. The
    port's `BTreeMap` is a *representation* of this fold's result; the fold is what makes the keys
    distinct. -/
def layerInsert (m : Message) (l : List Message) : List Message :=
  m :: l.filter (fun m' => !(m'.sender = m.sender))

/-- The layer: the min messages folded in, then the candidate parents whose sender is already a key
    (`calculate_next_layer`'s second half). -/
def nextLayer (d : Dag) (ms : List Message) : List Message :=
  let seeded := ms.foldr (fun m acc => layerInsert m acc) []
  let cands := ((ms.map (fun x => x.parents.filterMap (Dag.msg d))).join).filter
    (fun c => decide (c.sender ∈ seeded.map (·.sender)))
  cands.foldr (fun m acc => layerInsert m acc) seeded

/-- **The derivation** — the port's `next_fringe` (`:186-211`) in its own decision order: the walk, the
    count gate, the layer, the stake gate. The support map is an argument (its content is law 14a's,
    `Rchain.Casper.Stake`), exactly as it is for `Rchain.nextFringe`. -/
def derivedFringe (d : Dag) (js : List Message) (prev : Fringe) (supp : SupportMap) (bonds : Bonds) :
    Option Fringe :=
  if checkMinMessages (minMsgs d js (prev.messages.map (·.id))) bonds then
    if calculateFringe supp bonds then
      some ⟨nextLayer d (minMsgs d js (prev.messages.map (·.id)))⟩
    else none
  else none

/-! ### The antichain, about the fold -/

/-- Filtering messages by sender and then reading the senders is filtering the senders — the bridge
    between the fold's data and the key list the antichain is stated over. -/
theorem filter_senders (l : List Message) (s : Nat) :
    ((l.filter (fun m' => !(m'.sender = s))).map (·.sender)) =
      (l.map (·.sender)).filter (fun x => !(x = s)) := by
  induction l with
  | nil => rfl
  | cons m' rest ih =>
    by_cases hs : m'.sender = s <;> simp [hs, ih]

/-- **The fold is what keeps the senders distinct**, one insertion at a time: the message goes to the
    front and every earlier entry with its sender is **dropped** (`l.filter`), so the keys are the old
    keys minus that sender plus this one. Dropping the filter is the one-word mutation. -/
theorem layerInsert_nodup (m : Message) (l : List Message) (h : (l.map (·.sender)).Nodup) :
    ((layerInsert m l).map (·.sender)).Nodup := by
  unfold layerInsert
  rw [List.map_cons, filter_senders]
  refine List.nodup_cons.mpr ⟨?_, h.filter _⟩
  simp

/-- A `foldr` of the insertion over any list preserves the keys' distinctness. -/
theorem foldr_layerInsert_nodup : ∀ (l init : List Message), (init.map (·.sender)).Nodup →
    ((l.foldr (fun m acc => layerInsert m acc) init).map (·.sender)).Nodup
  | [], init, h => by simpa using h
  | m :: rest, init, h => by
    rw [List.foldr_cons]
    exact layerInsert_nodup m _ (foldr_layerInsert_nodup rest init h)

/-- **The layer's senders are pairwise distinct** — the antichain, from the fold that builds it. -/
theorem nextLayer_nodup (d : Dag) (ms : List Message) :
    ((nextLayer d ms).map (·.sender)).Nodup := by
  unfold nextLayer
  exact foldr_layerInsert_nodup _ _ (foldr_layerInsert_nodup _ _ (by simp))

/-- **Law 14b — the derived fringe is an antichain.** A fringe the derivation *publishes* holds at most
    one message per sender: `(f.messages.map (·.sender)).Nodup`. That is what the walk and the layer
    earn; the step to "one per **bonded** validator" is `checkMinMessages`' count comparison, which is
    the epoch TODO this module does not model (law 14a's row carries it as fidelity). -/
theorem derivedFringe_antichain (d : Dag) (js : List Message) (prev : Fringe) (supp : SupportMap)
    (bonds : Bonds) (f : Fringe) (h : derivedFringe d js prev supp bonds = some f) :
    (f.messages.map (·.sender)).Nodup := by
  unfold derivedFringe at h
  split at h
  · split at h
    · simp only [Option.some.injEq] at h
      rw [← h]
      exact nextLayer_nodup d _
    · exact absurd h (by simp)
  · exact absurd h (by simp)

/-! ### Non-vacuity: a derivation that publishes -/

/-- A three-message DAG: two from sender 0 (the newer justified on the older, so the walk has something
    to walk), one from sender 1; the second sender's message justifies the first's newer one. The layer
    therefore has **two** entries, so the antichain is a claim about a fold that ran rather than about
    `[]`. -/
def dag3 : Dag :=
  [ ⟨10, 0, 0, 0, [], []⟩,
    ⟨11, 1, 0, 1, [10], [10]⟩,
    ⟨12, 1, 1, 0, [11], [10, 11]⟩ ]

/-- The walk's output on `dag3`, `decide`d: justification `11` walks back to its own sender's oldest
    non-finalized message (`10`), and `12` has no same-sender ancestor, so it stands for itself. -/
theorem minMsgs_dag3 :
    (minMsgs dag3 [⟨11, 1, 0, 1, [10], [10]⟩, ⟨12, 1, 1, 0, [11], [10, 11]⟩] []).map (·.id) =
      [10, 12] := by decide

/-- **The layer is one message per sender, and the fold did real work on a concrete DAG** — the
    non-vacuity half: the theorem above is about a fold that ran, and this is the run. It exercises both
    cases of `layerInsert`: `12`'s sender is new, so it is **prepended**, and `11` (a candidate parent
    of `12`, with a higher `sender_seq` than `10`) **replaces** `10` rather than joining it. -/
theorem a_derivation_publishes_a_layer :
    ((nextLayer dag3 (minMsgs dag3 [⟨11, 1, 0, 1, [10], [10]⟩, ⟨12, 1, 1, 0, [11], [10, 11]⟩] []))
      |>.map (·.sender)) = [0, 1] := by decide

/-- And that layer is an antichain, `decide`d on the same instance. -/
theorem a_derivation_is_an_antichain :
    ((nextLayer dag3 (minMsgs dag3 [⟨11, 1, 0, 1, [10], [10]⟩, ⟨12, 1, 1, 0, [11], [10, 11]⟩] []))
      |>.map (·.sender)).Nodup = true := by decide

end Rchain
