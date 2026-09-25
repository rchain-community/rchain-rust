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

/-! ### The walk's descent — the half of law 15's comparison that is about the DAG

Law 15's last conjunct is the per-sender height comparison between successive fringes, and its first
ingredient is that the walk **strictly descends**: everything `selfParents` returns is reached from its
seed along parent edges, so `Descends` makes it strictly lower. The relation below follows **parents**,
where `Rchain.Reaches` (`Fringe.lean:143`) follows *justifications* through the seen set — different
edges, and the walk takes these.

This is stated and proved here rather than left inside the comparison because it is what any form of the
comparison consumes, and because the sentence that asserted it until now was a doc comment on
`walkSameSender_skips_finalized` ("a min message is the previous sentinel's direct successor rather than
a descendant of something older") — a claim nothing checked. What is *still* owed after it is the
chain half: with a single same-sender parent per message the ancestors of a message are linearly
ordered, and that is what puts a finalized ancestor strictly below every message the walk keeps. -/

/-- A message a lookup answers with is in the DAG — the closure that `Descends` asserts of *parents*,
    here for the lookup itself. -/
theorem mem_of_msg (d : Dag) {p : Nat} {q : Message} (h : Dag.msg d p = some q) : q ∈ d := by
  simpa only [Dag.msg] using List.mem_of_find?_eq_some h

/-- **Parent reachability** — the walk's own relation: `q` resolves one of `m`'s parents, transitively.
    It is deliberately *not* `Rchain.Reaches`: that one is about the seen set (justifications), and the
    walk follows `parents`. -/
inductive ReachesF (d : Dag) : Message → Message → Prop where
  /-- One edge: `q` is the message one of `m`'s **same-sender** parent ids resolves to. The sender
      condition is the walk's own (`sameSenderParents`), and it is what makes a sender's ancestry a chain
      rather than a tree — the property the sentinel comparison turns on. -/
  | step (m : Message) (q : Message) (p : Nat) (hp : p ∈ m.parents) (hq : Dag.msg d p = some q)
      (hs : q.sender = m.sender) : ReachesF d q m
  /-- …and its transitive closure. -/
  | trans (m : Message) (n : Message) (q : Message) (h1 : ReachesF d n m)
      (h2 : ReachesF d q n) : ReachesF d q m

/-- **The descent is strict** — `Descends` gives one strict step, and height is a `Nat` — so the walk's
    relation is well-founded, which is why `selfParents`' fuel can be the DAG's own length. -/
theorem reachesF_height_lt (d : Dag) (hdes : Descends d) :
    ∀ {q m : Message}, ReachesF d q m → m ∈ d → q ∈ d ∧ q.height < m.height := by
  intro q m h
  induction h with
  | step m q p hp hq _hs =>
    intro hm
    exact hdes m hm p hp q hq
  | trans m n q _h1 _h2 ih1 ih2 =>
    intro hm
    obtain ⟨hnd, hnm⟩ := ih1 hm
    obtain ⟨hqd, hqn⟩ := ih2 hnd
    exact ⟨hqd, Nat.lt_trans hqn hnm⟩

/-- Membership travels along the walk's relation: whatever a DAG message is reached from is in the DAG
    too. -/
theorem reachesF_mem (d : Dag) :
    ∀ {q m : Message}, ReachesF d q m → m ∈ d → q ∈ d := by
  intro q m h
  induction h with
  | step _m q _p _hp hq _hs =>
    intro _hm
    exact mem_of_msg d hq
  | trans _m _n _q _h1 _h2 ih1 ih2 =>
    intro hm
    exact ih2 (ih1 hm)

/-- A seed element is a **direct** parent-edge predecessor: the filter's two halves, unpacked. -/
theorem sameSenderParents_reaches (d : Dag) (sender : Nat) (mv : Message) (fin : List Nat)
    {x : Message} (h : x ∈ sameSenderParents d sender mv fin) :
    x ∈ d ∧ x.sender = sender ∧ ∃ p ∈ mv.parents, Dag.msg d p = some x := by
  simp only [sameSenderParents, List.mem_filter, List.mem_filterMap, Bool.and_eq_true] at h
  obtain ⟨⟨p, hp, hpx⟩, hs, _⟩ := h
  exact ⟨mem_of_msg d hpx, of_decide_eq_true hs, p, hp, hpx⟩

/-- **The invariant the walk carries**: every message in the worklist and in the accumulator is reached
    from the seed `mv` along parent edges. A step replaces the popped message's same-sender parents — each
    of which reaches the popped one — so the property composes by transitivity rather than by induction on
    the DAG. -/
theorem walkSameSender_reaches (d : Dag) (fin : List Nat) (mv : Message) (fuel : Nat) :
    ∀ work acc,
      (∀ x ∈ work, x ∈ d ∧ ReachesF d x mv) →
      (∀ x ∈ acc, x ∈ d ∧ ReachesF d x mv) →
      ∀ m ∈ walkSameSender d fin fuel work acc, m ∈ d ∧ ReachesF d m mv := by
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
      refine ih (sameSenderParents d w.sender w fin ++ rest) (w :: acc) ?_ ?_ m hm
      · intro x hx
        rw [List.mem_append] at hx
        rcases hx with hx | hx
        · obtain ⟨hxd, hsx, p, hp, hpx⟩ := sameSenderParents_reaches d w.sender w fin hx
          have hw := hwork w (List.mem_cons.mpr (Or.inl rfl))
          exact ⟨hxd, ReachesF.trans mv w x hw.2 (ReachesF.step w x p hp hpx hsx)⟩
        · exact hwork x (List.mem_cons.mpr (Or.inr hx))
      · intro x hx
        rcases List.mem_cons.mp hx with hwx | hx
        · rw [hwx]
          exact hwork w (List.mem_cons.mpr (Or.inl rfl))
        · exact hacc x hx

/-- **Everything `selfParents` returns is reached from the seed** — the walk's descent, at the definition
    the port's `.last()` reads. -/
theorem selfParents_reaches (d : Dag) (mv : Message) (fin : List Nat) :
    ∀ m ∈ selfParents d mv fin, m ∈ d ∧ ReachesF d m mv := by
  unfold selfParents
  refine walkSameSender_reaches d fin mv d.length (sameSenderParents d mv.sender mv fin) [] ?_ ?_
  · intro x hx
    obtain ⟨hxd, hsx, p, hp, hpx⟩ := sameSenderParents_reaches d mv.sender mv fin hx
    exact ⟨hxd, ReachesF.step mv x p hp hpx hsx⟩
  · intro x hx
    simp at hx

/-- **…so every one of them is strictly lower than the seed**: the walk cannot return a message at or
    above the message it walked from. This is the fact the per-sender comparison rests on, and the one
    that makes the min message a *descent* rather than a search. -/
theorem selfParents_height_lt (d : Dag) (hdes : Descends d) (mv : Message) (fin : List Nat)
    (hmv : mv ∈ d) : ∀ m ∈ selfParents d mv fin, m.height < mv.height := by
  intro m hm
  obtain ⟨_hmd, hre⟩ := selfParents_reaches d mv fin m hm
  -- the relation's *second* index is the seed, so the two are named rather than left to inference
  exact (reachesF_height_lt d hdes (q := m) (m := mv) hre hmv).2

/-! ### The chain half — one same-sender parent, and the sentinel above a finalized ancestor

`selfParents_height_lt` says the walk descends. What it does not yet say is that the walk's *output* is
above a **finalized** message that the walk must not return. That needs the second ingredient, and it is
the hypothesis the port *enforces* rather than observes: at most one same-sender parent per message, so a
sender's ancestry is a chain rather than a tree, so there is exactly one way down and a finalized ancestor
is on it. -/

/-- **The fork-freedom the ingress refusal gives.** A message has **at most one** same-sender parent in
    the DAG. The port does not merely happen to be fork-free: H-1's equivocation detection refuses a
    second distinct block by one sender reusing a `seq_num`, `sequence_number` requires a block to justify
    a same-sender block one lower, and `check_justification_regression` admits at most one justification
    per sender and demands it be the latest (`casper/src/dag.rs:244-252`, `validate.rs:169-188`,
    `:205-241`; AUDIT C82 and C84). Stated over an arbitrary `fin` because that is the shape the walk
    uses, and `sameSenderParents_subset` below is what makes it the *same* fact as the unfiltered one. -/
def NoFork (d : Dag) : Prop :=
  ∀ m ∈ d, ∀ fin, ∀ a ∈ sameSenderParents d m.sender m fin,
    ∀ b ∈ sameSenderParents d m.sender m fin, a = b

/-- The `fin`-filtered parent set is a subset of the unfiltered one: the predicate's second conjunct is
    weaker. This is what makes `NoFork`'s statement at any `fin` the fact ingress gives at `[]`. -/
theorem sameSenderParents_subset (d : Dag) (s : Nat) (mv : Message) (fin : List Nat) :
    ∀ x ∈ sameSenderParents d s mv fin, x ∈ sameSenderParents d s mv [] := by
  intro x hx
  simp only [sameSenderParents, List.mem_filter, List.mem_filterMap, Bool.and_eq_true] at hx ⊢
  obtain ⟨⟨p, hp, hpx⟩, hs, _⟩ := hx
  exact ⟨⟨p, hp, hpx⟩, hs, by simp⟩

/-- …so the ingress fact — uniqueness among the **unfinalized** parents — is `NoFork` as the walk uses
    it. -/
theorem nofork_of_unfiltered (d : Dag)
    (h : ∀ m ∈ d, ∀ a ∈ sameSenderParents d m.sender m [],
      ∀ b ∈ sameSenderParents d m.sender m [], a = b) : NoFork d := by
  intro m hm fin a ha b hb
  exact h m hm a (sameSenderParents_subset d m.sender m fin a ha)
    b (sameSenderParents_subset d m.sender m fin b hb)

/-- **The first step is the only step.** With one same-sender parent per message, the descent from `m` has
    a single edge out of it: every proper same-sender ancestor of `m` is reached from *that* parent — or
    *is* that parent. This is the lemma that turns fork-freedom into "the walk cannot step around a
    finalized ancestor", and it is where the chain argument lives. -/
theorem nofork_ancestors_go_through_the_parent (d : Dag) (hnofork : NoFork d) :
    ∀ {q m x : Message}, ReachesF d q m → m ∈ d →
      x ∈ sameSenderParents d m.sender m [] → q = x ∨ ReachesF d q x := by
  -- `q`/`m` are *indices* of the relation, so each arm re-binds them; the intro'd names are therefore
  -- deliberately distinct (`q₀`/`m₀`), or the arm's own binders become inaccessible and the goals stop
  -- naming the messages they are about. `x` is fixed and keeps its name.
  intro q₀ m₀ x h
  induction h with
  | step m q p hp hq hs =>
    intro hm hx
    have hqmem : q ∈ sameSenderParents d m.sender m [] := by
      simp only [sameSenderParents, List.mem_filter, List.mem_filterMap, Bool.and_eq_true]
      exact ⟨⟨p, hp, hq⟩, decide_eq_true hs, by simp⟩
    exact Or.inl (hnofork m hm [] q hqmem x hx)
  | trans m n q _h1 h2 ih1 _ih2 =>
    intro hm hx
    rcases ih1 hm hx with hqx | hqx
    · exact Or.inr (hqx ▸ h2)
    · exact Or.inr (ReachesF.trans x n q hqx h2)

/-- **Law 15's last owed conjunct, the sentinel half.** For a justification `p`, every message the walk
    keeps for it is **strictly above** any finalized same-sender ancestor of `p` — so the min message is
    the previous sentinel's successor and not a descendant of something older, which is what the doc
    comment on `walkSameSender_skips_finalized` asserted and nothing checked.

    The invariant the proof carries is not about heights but about **reachability**: `q` reaches every
    message in the worklist and the accumulator. A step adds a same-sender parent of the popped message,
    and `nofork_ancestors_go_through_the_parent` puts every proper ancestor of the popped message below
    it — so the property is maintained by the same lemma at each step, with no comparison of heights
    anywhere in the induction. Heights enter once, at the end. -/
theorem walkSameSender_above_finalized (d : Dag) (hnofork : NoFork d)
    (fin : List Nat) (q : Message) (hqfin : q.id ∈ fin) (fuel : Nat) :
    ∀ work acc,
      (∀ x ∈ work, x ∈ d) → (∀ x ∈ acc, x ∈ d) →
      (∀ x ∈ work, ReachesF d q x) → (∀ x ∈ acc, ReachesF d q x) →
      ∀ m ∈ walkSameSender d fin fuel work acc, ReachesF d q m := by
  induction fuel with
  | zero =>
    intro work acc _ _ _ hacc m hm
    simp only [walkSameSender] at hm
    exact hacc m hm
  | succ f ih =>
    intro work acc hworkd haccd hworkq haccq m hm
    cases work with
    | nil =>
      simp only [walkSameSender] at hm
      exact haccq m hm
    | cons w rest =>
      simp only [walkSameSender] at hm
      have hwd : w ∈ d := hworkd w (List.mem_cons.mpr (Or.inl rfl))
      have hwq : ReachesF d q w := hworkq w (List.mem_cons.mpr (Or.inl rfl))
      refine ih (sameSenderParents d w.sender w fin ++ rest) (w :: acc) ?_ ?_ ?_ ?_ m hm
      · intro y hy
        rw [List.mem_append] at hy
        rcases hy with hy | hy
        · exact (sameSenderParents_reaches d w.sender w fin hy).1
        · exact hworkd y (List.mem_cons.mpr (Or.inr hy))
      · intro y hy
        rcases List.mem_cons.mp hy with hyw | hy
        · rw [hyw]; exact hwd
        · exact haccd y hy
      · intro y hy
        rw [List.mem_append] at hy
        rcases hy with hy | hy
        · have hyunf : y.id ∉ fin := sameSenderParents_unfinalized d w.sender w fin y hy
          have hysub := sameSenderParents_subset d w.sender w fin y hy
          rcases nofork_ancestors_go_through_the_parent d hnofork hwq hwd hysub with hey | hqy
          · exact absurd (hey ▸ hqfin) hyunf
          · exact hqy
        · exact hworkq y (List.mem_cons.mpr (Or.inr hy))
      · intro y hy
        rcases List.mem_cons.mp hy with hyw | hy
        · rw [hyw]; exact hwq
        · exact haccq y hy

/-- **The theorem law 15 was owed**: for a justification `p`, the message the derivation keeps for it lies
    **strictly above every finalized same-sender ancestor of `p`**. The previous fringe *is* the finalized
    set (`derivedFringe` passes `prev.messages.map (·.id)`), so this is the per-sender comparison at the
    sentinel: the walk cannot return a message at or below the one it has already finalized. -/
theorem selfParents_above_a_finalized_ancestor (d : Dag) (hdes : Descends d) (hnofork : NoFork d)
    (p : Message) (hp : p ∈ d) (fin : List Nat) (q : Message) (hqfin : q.id ∈ fin)
    (hre : ReachesF d q p) : ∀ m ∈ selfParents d p fin, q.height < m.height := by
  intro m hm
  have hseed : ∀ x ∈ sameSenderParents d p.sender p fin, ReachesF d q x := by
    intro x hx
    have hxsub := sameSenderParents_subset d p.sender p fin x hx
    have hxunf : x.id ∉ fin := sameSenderParents_unfinalized d p.sender p fin x hx
    rcases nofork_ancestors_go_through_the_parent d hnofork hre hp hxsub with hqx | hqx
    · exact absurd (hqx ▸ hqfin) hxunf
    · exact hqx
  have hqm : ReachesF d q m := by
    unfold selfParents at hm
    exact walkSameSender_above_finalized d hnofork fin q hqfin d.length
      (sameSenderParents d p.sender p fin) []
      (fun x hx => (sameSenderParents_reaches d p.sender p fin hx).1)
      (fun x hx => by simp at hx) hseed (fun x hx => by simp at hx) m hm
  obtain ⟨hmd, _⟩ := selfParents_reaches d p fin m hm
  exact (reachesF_height_lt d hdes hqm hmd).2

/-! ### The hypothesis is load-bearing: the fork that refutes the comparison without it -/

/-- **The DAG that shows `NoFork` is not decoration.** Four messages: a common ancestor `w` (20, height
    1), a low branch `y` (21, height 2) and a high branch `q` (22, height 5), and a top `p` (23, height
    6) whose parents are **both** `y` and `q` — a same-sender fork, which is exactly what H-1's
    equivocation detection refuses at ingress (`casper/src/dag.rs:244-252`). -/
def fork4 : Dag :=
  [ ⟨20, 1, 0, 1, [], []⟩,
    ⟨21, 2, 0, 2, [20], []⟩,
    ⟨22, 5, 0, 3, [20], []⟩,
    ⟨23, 6, 0, 4, [21, 22], []⟩ ]

/-- The walk from `p` with `q` finalized, `decide`d: it takes the unfinalized branch and steps *around*
    the finalized ancestor — `y` then `w`, so the output is newest-last as always. -/
theorem selfParents_fork4 :
    (selfParents fork4 ⟨23, 6, 0, 4, [21, 22], []⟩ [22]).map (·.id) = [20, 21] := by decide

/-- **…and there the comparison is false**: the walk's oldest message is `w` at height 1 while the
    finalized ancestor `q` is at height 5, so `q.height < m.height` fails for every message the walk
    kept. The theorem above is therefore a claim about *fork-free* DAGs and not about any DAG, and this
    refutation is what says so — the same shape as `fringe_monotone_is_false` one layer up. -/
theorem the_comparison_is_false_without_fork_freedom :
    ¬ (∀ m ∈ selfParents fork4 ⟨23, 6, 0, 4, [21, 22], []⟩ [22],
        (⟨22, 5, 0, 3, [20], []⟩ : Message).height < m.height) := by decide

/-! ### And fork-freedom is *not enough*: the fold can still publish below the previous fringe

The sentinel theorem is about `selfParents`, and the derivation does not stop there: `nextLayer` folds the
min messages and then adds their **candidate parents** (`:129-135`). The DAG below shows that step can
publish a message *below* the previous fringe's message for the same sender — and it is fork-free, so the
hypothesis the sentinel theorem needs does not exclude it.

Three messages: two sender-0 blocks, `99` (height 2) and `100` (height 5), **neither with a parent** — a
forest rather than a fork, which `NoFork` allows — and `101` (height 6) justifying `99`. With `100`
finalized (it is the previous fringe's sender-0 message), the walk from `101` stops at `99`, `nextLayer`
publishes `99`, and the previous fringe held `100`. **What excludes this is the sequence rule, not the
fork rule**: `101`'s `seqNum` is 6 while its same-sender justification `99` is at `seqNum` 2, and the
port's `sequence_number`/`check_justification_regression` require a block to justify its sender's
**latest** block — so this DAG is refused at ingress for a different reason than a fork is. -/
def fold4 : Dag :=
  [ ⟨99, 2, 0, 2, [], []⟩,
    ⟨100, 5, 0, 5, [], []⟩,
    ⟨101, 6, 0, 6, [99], []⟩ ]

/-- The walk from `101` with `100` finalized, `decide`d: it takes the unfinalized `99` and stops (no
    parents). -/
theorem minMsgs_fold4 :
    (minMsgs fold4 [⟨101, 6, 0, 6, [99], []⟩] [100]).map (·.id) = [99] := by decide

/-- **`fold4` is fork-free** — every message has at most one same-sender parent in the DAG (each has none,
    or one) — so `NoFork` cannot be the hypothesis that excludes the case below. -/
theorem fold4_is_fork_free :
    ∀ m ∈ fold4, ∀ a ∈ sameSenderParents fold4 m.sender m [],
      ∀ b ∈ sameSenderParents fold4 m.sender m [], a = b := by decide

/-- **…and the published layer lies below the previous fringe**: the fold publishes `99` (height 2) where
    the previous fringe held `100` (height 5), both sender 0. So a *fringe-level* monotonicity claim is
    false of this model even with fork-freedom, and the hypothesis it needs is the sequence rule — the
    finding that narrows law 15's remainder from "prove the lift" to "the lift needs the ingress rule that
    makes a justification the sender's latest". -/
theorem the_fold_can_publish_below_the_previous_fringe :
    ¬ (∀ m ∈ nextLayer fold4 (minMsgs fold4 [⟨101, 6, 0, 6, [99], []⟩] [100]),
        (⟨100, 5, 0, 5, [], []⟩ : Message).height ≤ m.height) := by decide

/-- **A second DAG, for a sharper question: is the *sequence* rule the lift's hypothesis?** Four messages —
    `q` (100, sender 0, seq 5) as the previous fringe's message, `c` (101, sender 0, seq 2) a *lower*
    sender-0 block in another root, `p` (102, sender 1) justifying `c`, and `z` (103, sender 0, seq 6)
    justifying `q`. Every same-sender parent edge has consecutive sequence numbers. -/
def fold5 : Dag :=
  [ ⟨100, 5, 0, 5, [], []⟩,
    ⟨101, 2, 0, 2, [], []⟩,
    ⟨102, 3, 1, 0, [101], []⟩,
    ⟨103, 6, 0, 6, [100], []⟩ ]

/-- `fold5` satisfies **the sequence rule** — each same-sender parent is exactly one `seqNum` below its
    child, which is the port's `sequence_number`. -/
theorem fold5_satisfies_the_sequence_rule :
    ∀ m ∈ fold5, ∀ x ∈ sameSenderParents fold5 m.sender m [], x.seqNum + 1 = m.seqNum := by decide

/-- …and it is fork-free, like `fold4`. -/
theorem fold5_is_fork_free :
    ∀ m ∈ fold5, ∀ a ∈ sameSenderParents fold5 m.sender m [],
      ∀ b ∈ sameSenderParents fold5 m.sender m [], a = b := by decide

/-- **…and it still publishes below the previous fringe**: the fold's *last* insertion for sender 0 is the
    candidate `c` (height 2) rather than the min message `z` (height 6), because `cands` is folded
    right-to-left and `c` comes from the earlier justification. So the sequence rule alone is not the
    lift's hypothesis either. -/
theorem the_sequence_rule_is_not_enough :
    ¬ (∀ m ∈ nextLayer fold5
          (minMsgs fold5 [⟨102, 3, 1, 0, [101], []⟩, ⟨103, 6, 0, 6, [100], []⟩] [100]),
        (⟨100, 5, 0, 5, [], []⟩ : Message).height ≤ m.height) := by decide

end Rchain
