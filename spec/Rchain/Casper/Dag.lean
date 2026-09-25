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

/-- `mv`'s direct parents, of the same sender, not already finalized. -/
def sameSenderParents (d : Dag) (sender : Nat) (mv : Message) (finalized : List Nat) : List Message :=
  (mv.parents.filterMap (Dag.msg d)).filter
    (fun x => x.sender = sender && decide (x.id ∉ finalized))

/-- The walk, on its fuel: pop a message, fold in its same-sender unfinalized parents. Top-level rather
    than a `where` clause, because a `where`-bound helper is invisible to every theorem outside its
    declaration and the boundary property below is about this recursion. -/
def walkSameSender (d : Dag) (finalized : List Nat) :
    Nat → List Message → List Message → List Message
  | 0, _, acc => acc
  | _, [], acc => acc
  | fuel + 1, m :: rest, acc =>
    walkSameSender d finalized fuel (sameSenderParents d m.sender m finalized ++ rest) (m :: acc)

/-- **Step 1 — `self_parents`** (`finalizer.rs:74-95`): the same-sender, not-yet-finalized ancestors
    reachable from `mv`, in walk order.

    The fuel is the DAG's own size: every step pops one message and folds in its parents, so `d.length`
    steps cannot be exceeded — a bound justified by the walk rather than chosen, which is what keeps
    this a definition and not a partiality. -/
def selfParents (d : Dag) (mv : Message) (finalized : List Nat) : List Message :=
  walkSameSender d finalized d.length (sameSenderParents d mv.sender mv finalized) []

/-- **Step 2 — the min messages** (`next_fringe`, `:186-211`): per justification, the *oldest*
    non-finalized same-sender ancestor, with the justification itself when there is none (the port's
    `chain.into_iter().last()` over `[p] ++ self_parents p`).

    **This took the wrong end until 2026-09-25, and the fix is `.head?`.** `selfParents` returns its list
    **oldest-first** — the model prepends into its accumulator, so the deepest message lands last — while
    the port's `self_parents` builds its chain by `push`ing the visit order, which is **newest-first**
    (`block-storage/src/dag/finalizer.rs:78-95`), and `next_fringe` seeds it with `chain = vec![p]` before
    extending (`:194-199`). So the port's `.last()` is the oldest, and the model's `getLast?` on an
    oldest-first list was the *newest*. Measured on a chain `10 (h 0) ← 11 (h 1) ← 12 (h 2)`: the model's
    `selfParents` is `[10, 11]` and the port's chain is `[11, 10]`, so the model answered `11` where the
    port answers `10`.

    **Why nothing failed**: every other `decide`d instance in this file gives a sender at most **one**
    same-sender ancestor, and with one element both ends agree. `chain3` and the two theorems beside it
    are the instance that can see the difference — a test that could not fail is what let a step of the
    derivation read the opposite end of the chain. -/
def minMsgs (d : Dag) (js : List Message) (finalized : List Nat) : List Message :=
  js.map fun p =>
    match (selfParents d p finalized).head? with
    | some m => m
    | none => p

/-- A three-message **same-sender chain**: `10 (h 0) ← 11 (h 1) ← 12 (h 2)`, with one other sender so
    the DAG is not degenerate. Deep enough for the walk to have two ancestors and so for the two ends of
    its output — oldest-first in the model, newest-first in the port — to be *different* messages. -/
def chain3 : Dag :=
  [ ⟨10, 0, 0, 0, [], []⟩,
    ⟨11, 1, 0, 1, [10], []⟩,
    ⟨12, 2, 0, 2, [11], []⟩,
    ⟨20, 2, 1, 0, [12], []⟩ ]

/-- **The walk's order, pinned**: oldest-first, which is what makes `head?` the oldest. -/
theorem the_walk_is_oldest_first :
    (selfParents chain3 ⟨12, 2, 0, 2, [11], []⟩ []).map (·.id) = [10, 11] := by decide

/-- **And the min message is the oldest of them** — `10`, not `11`: the instance the previous form of
    `minMsgs` failed, kept so the end cannot be reversed again by a reader who assumes the port's order. -/
theorem a_chain_of_three_picks_the_oldest :
    (minMsgs chain3 [⟨12, 2, 0, 2, [11], []⟩] []).map (·.id) = [10] := by decide

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

/-! ### The walk stops at the finalized boundary, and law 15's two readings -/

/-- Everything the filter keeps is unfinalized — the predicate's second conjunct, unpacked once here
    rather than at each use. -/
theorem sameSenderParents_unfinalized (d : Dag) (sender : Nat) (mv : Message)
    (finalized : List Nat) : ∀ m ∈ sameSenderParents d sender mv finalized, m.id ∉ finalized := by
  intro m hm
  simp only [sameSenderParents, List.mem_filter, Bool.and_eq_true] at hm
  exact of_decide_eq_true hm.2.2

/-- **The walk never returns a finalized message.** Each step filters by "not finalized" *and* never
    traverses through an excluded message (the worklist is rebuilt from the survivors), so everything
    pushed — and therefore everything returned — is unfinalized. This is the fact that makes the
    **per-sender** reading of law 15 provable: a min message is the previous sentinel's direct
    successor rather than a descendant of something older. -/
theorem walkSameSender_skips_finalized (d : Dag) (finalized : List Nat) (fuel : Nat) :
    ∀ work acc, (∀ m ∈ work, m.id ∉ finalized) → (∀ m ∈ acc, m.id ∉ finalized) →
      ∀ m ∈ walkSameSender d finalized fuel work acc, m.id ∉ finalized := by
  induction fuel with
  | zero =>
    intro work acc _ hacc m hm
    simp only [walkSameSender] at hm
    exact hacc m hm
  | succ f ih =>
    intro work acc hwork hacc m hm
    cases work with
    | nil =>
      simp only [walkSameSender] at hm
      exact hacc m hm
    | cons w rest =>
      simp only [walkSameSender] at hm
      refine ih (sameSenderParents d w.sender w finalized ++ rest) (w :: acc) ?_ ?_ m hm
      · intro x hx
        rw [List.mem_append] at hx
        rcases hx with hx | hx
        · exact sameSenderParents_unfinalized d w.sender w finalized x hx
        · exact hwork x (List.mem_cons.mpr (Or.inr hx))
      · intro x hx
        rcases List.mem_cons.mp hx with hwx | hx
        · rw [hwx]
          exact hwork w (List.mem_cons.mpr (Or.inl rfl))
        · exact hacc x hx

/-- The walk's boundary, at `selfParents`. -/
theorem selfParents_skips_finalized (d : Dag) (mv : Message) (finalized : List Nat) :
    ∀ m ∈ selfParents d mv finalized, m.id ∉ finalized := by
  unfold selfParents
  refine walkSameSender_skips_finalized d finalized d.length
    (sameSenderParents d mv.sender mv finalized) [] ?_ ?_
  · exact sameSenderParents_unfinalized d mv.sender mv finalized
  · intro x hx; simp at hx

/-- **The cross-sender reading of law 15 is false**, and this is the sharp witness: one previous
    fringe with a **high** message and a published fringe with a **low** one on a different sender —
    `prev = {m3 (sender 0, height 3)}`, `f = {q2 (sender 1, height 2)}`. The port's gate cannot see
    it, because what `calculate_fringe` reads is the **support map**, not the heights, and the
    oracle's loop has no fringe comparison either (AUDIT C69, §6). Kept as a refutation rather than
    dropped, so the next reader sees which reading was false and on what. -/
theorem cross_sender_height_monotone_is_false :
    ¬ ∀ prev f : Fringe, ∀ p ∈ prev.messages, ∀ m ∈ f.messages, p.height ≤ m.height := by
  intro h
  let prev : Fringe := ⟨[⟨3, 3, 0, 0, [], []⟩]⟩
  let f : Fringe := ⟨[⟨2, 2, 1, 0, [], []⟩]⟩
  exact absurd (h prev f ⟨3, 3, 0, 0, [], []⟩ (by simp [prev]) ⟨2, 2, 1, 0, [], []⟩ (by simp [f]))
    (by decide)

/-! ### Law 15's literal form, over the id→message lookup

The closure is proved along `Reaches` (`Rchain/Casper/Fringe.lean`) — a relation between *values*. What
law 15 states literally is the id-based form, `a ∈ b.seen → a.seen ⊆ b.seen`, and row 15's note names
what it was waiting for: "an id→message map (so the *id* can be turned back into the *value*) **that the
DAG model would bring**". `Dag.msg` is that lookup.

The hypotheses are the port's own construction and the two facts a DAG carries, **stated rather than
assumed** — the shape the tie's domain and law 14b's `checkMinMessages` boundary already use. They are
hypotheses and not axioms: resolving an id back to a value needs uniqueness, and the descent needs a
finite measure. -/

/-- **The port's construction** (`block-storage/src/dag/message_state.rs:54-59`): a message's seen set is
    `seenOf` of the messages its parents name — `new_seen`, stated rather than assumed. -/
def Constructed (d : Dag) (m : Message) : Prop :=
  m.seen = seenOf (m.parents.filterMap (Dag.msg d)) m.id

/-- **The DAG's closure and its order**, in one hypothesis: whatever a message's parent id resolves to
    is *in* the DAG and *lower* than the message that names it.

    **And it is enforced, not merely assumed** (H1b, `46c35b545`): `block_number` refuses a block that
    names a resolved parent at or above its own number, the **failed** ones included
    (`casper/src/validate.rs:152-154`), and refuses one whose justification does not resolve at all
    (`:150`) — so every state the port *admits* satisfies this as stated. The reference validator does
    not: it filters failed parents out of `blockNumber` (`legacy/casper/.../Validate.scala`) and so
    admits the violating block. That is the §6 deviation AUDIT C83 carries, kept because this is the
    premise law 15's proof consumes.

    The plan to **weaken** this to *unfailed* parents is therefore superseded, and the measurement that
    prompted the enforcement is why: a weakened `Descends` forces `unfailed` into the proof's descent
    step, while the strong form is what the ingress check guarantees — a premise that is enforced needs
    no narrower statement. -/
def Descends (d : Dag) : Prop :=
  ∀ m ∈ d, ∀ p ∈ m.parents, ∀ pm, Dag.msg d p = some pm → pm ∈ d ∧ pm.height < m.height

/-- A message found in a `filterMap` of `Dag.msg` comes from a parent id that resolves to it — proved
    here rather than named, since `List.mem_filterMap` is used nowhere in this tree. -/
theorem mem_filterMap_msg (d : Dag) (ps : List Nat) (x : Message)
    (h : x ∈ ps.filterMap (Dag.msg d)) : ∃ p ∈ ps, Dag.msg d p = some x := by
  induction ps with
  | nil => simp at h
  | cons p rest ih =>
    cases hmsg : Dag.msg d p with
    | none =>
      simp only [List.filterMap_cons, hmsg] at h
      rcases ih h with ⟨q, hq, hq'⟩
      exact ⟨q, List.mem_cons.mpr (Or.inr hq), hq'⟩
    | some m =>
      simp only [List.filterMap_cons, hmsg, List.mem_cons] at h
      rcases h with hx | hx
      · exact ⟨p, List.mem_cons.mpr (Or.inl rfl), by rw [hx, hmsg]⟩
      · rcases ih hx with ⟨q, hq, hq'⟩
        exact ⟨q, List.mem_cons.mpr (Or.inr hq), hq'⟩

/-- **Law 15's literal form.** Over a DAG whose messages are constructed, which descends, and whose ids
    are unique, every id in `b`'s seen set is the id of a message whose entire seen set is `b`'s:
    `a.seen ⊆ b.seen`. The induction is on `b`'s height, because each step of the walk resolves a parent
    and `Descends` makes that strictly decreasing — a bound justified by the walk rather than chosen. -/
theorem seen_subset_of_mem_seen (d : Dag) (a b : Message)
    (ha : a ∈ d) (hb : b ∈ d)
    (hcon : ∀ m ∈ d, Constructed d m)
    (hdes : Descends d)
    (hid : ∀ m ∈ d, ∀ m' ∈ d, m.id = m'.id → m = m')
    (h : a.id ∈ b.seen) : a.seen ⊆ b.seen := by
  have main : ∀ n, ∀ b ∈ d, b.height = n → ∀ a ∈ d, a.id ∈ b.seen → a.seen ⊆ b.seen := by
    intro n
    induction n using Nat.strong_induction_on with
    | _ n ih =>
      intro b hb hbn
      intro a ha
      intro hmem
      show ∀ x, x ∈ a.seen → x ∈ b.seen
      intro x hx
      have hseen : a.id ∈ seenOf (b.parents.filterMap (Dag.msg d)) b.id := by
        rw [hcon b hb] at hmem
        exact hmem
      rcases List.mem_append.mp hseen with hjust | hself
      · -- seen by one of the justification *messages*: resolve the join through the seen sets, then
        -- the map back to the message, and induct on it
        rcases List.mem_join.mp hjust with ⟨s, hs, has⟩
        rcases List.mem_map.mp hs with ⟨j, hj, rfl⟩
        rcases mem_filterMap_msg d b.parents j hj with ⟨p, hp, hpj⟩
        have hjd : j ∈ d := (hdes b hb p hp j hpj).1
        have hlt : j.height < n := by
          have : j.height < b.height := (hdes b hb p hp j hpj).2
          rwa [hbn] at this
        have hsub : a.seen ⊆ j.seen := ih j.height hlt j hjd rfl a ha has
        have hxjoin : x ∈ List.join (List.map (fun m => m.seen)
            (b.parents.filterMap (Dag.msg d))) :=
          List.mem_join.mpr ⟨j.seen, List.mem_map.mpr ⟨j, hj, rfl⟩, hsub hx⟩
        have hxseen : x ∈ seenOf (b.parents.filterMap (Dag.msg d)) b.id :=
          List.mem_append_left _ hxjoin
        rw [hcon b hb]
        exact hxseen
      · have hid_eq : a.id = b.id := by simpa using hself
        have hab : a = b := hid a ha b hb hid_eq
        simpa [hab] using hx
  exact main b.height b hb rfl a ha h

end Rchain
