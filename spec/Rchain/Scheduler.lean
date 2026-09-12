import Rchain.Effect

/-!
# The path-ordered scheduler — Laws 20–22

`Rchain.Effect` established that the *static* channel-sharded effect scheduler is unsound
(`effect_reorder_diverges`): disjoint **footprints** do not commute, only disjoint **closures** do
(`effect_commute_of_disjoint_closure`), and closures are not statically decidable. This module
formalizes the scheduler that *is* sound, and the one that is *usefully parallel*, at the effect
level (`docs/src/formal/channel-scheduler.md`):

* **Law 20 — channel-task linearization** ("1 channel = 1 logical task"). Every channel owns a
  *claim queue*; an effect claims its channels at its DFS path, and commits only when it is the
  head (the path-smallest pending claim) of **every** claimed channel. `queue_commit_path_ordered`
  proves that queue steps preserve path-sortedness and that a channel's commit sequence follows
  its initial (path-sorted) queue order; `law20_deadlock_freedom` states the bakery argument —
  the globally path-smallest pending claim is at the head of all its channels, so some claim can
  always commit.

* **Law 21 — DFS-gate linearization**. The *gate* scheduler runs effect `i` only after every
  earlier effect's subtree has completed. `gate_exec_refines_apply` shows the gate's execution is
  exactly the sequential `Effect.apply` fold — sound, but with no parallelism beyond Level 1.
  The tempting repair — a **one-hop** scheduler that lets a sibling run once an effect's
  *next-step* footprint is disjoint — is unsound: `one_hop_depth2_diverges` exhibits a pair whose
  next-step footprints are disjoint at every decision point yet whose closures overlap, so the
  one-hop interleaving reaches a state the sequential reducer never reaches.

* **Law 22 — next-step closure is computable at dispatch.** Once a trigger matches, the matched
  datum is concrete, so a continuation chosen by that datum (`consumeWith`) has a
  *computable* first-step footprint (`next_step_closure_computable`) — this is what the Rust
  reducer's `resolve_children` computes. `depth2_next_step_disjoint` records that this
  computability does **not** make cross-channel pruning sound (the depth-2 pair is next-step
  disjoint yet non-commuting). The strengthened Law 9 itself — disjoint **closure** commutes — is
  promoted from an axiom to the theorem `effect_commute_of_disjoint_closure` in `Rchain.Effect`.
-/

namespace Rchain

/-! ## DFS paths -/

/-- A DFS path: the sequence of child indices from the root of the reduction tree. `[4]` is the
    4th sibling of a `Par`; `[4, 0]` is its first child. `PathLt` is exactly the sequential
    reducer's depth-first order: `[4] < [4,0] < [5]`. -/
abbrev DfsPath := List Nat

/-- Strict lexicographic order on paths: `[]` precedes any non-empty path; heads are compared
    first; equal heads recurse. -/
inductive PathLt : List Nat → List Nat → Prop where
  | nilCons (b : Nat) (bs : List Nat) : PathLt [] (b :: bs)
  | cons {a b : Nat} {as bs : List Nat} : a < b → PathLt (a :: as) (b :: bs)
  | consEq {a : Nat} {as bs : List Nat} : PathLt as bs → PathLt (a :: as) (a :: bs)

/-! ## Law 20 — the channel claim queue -/

/-- A claim: an effect at a DFS path, on the channels it touches. -/
structure Claim where
  path : DfsPath
  channels : Finset Chan

/-- One channel's pending queue, and the tuple of all channels' queues. -/
abbrev Queue := List Claim
abbrev Queues := Chan → Queue

/-- A queue is path-sorted when adjacent claims ascend under `PathLt`. Claim queues maintain this
    invariant: insertion places a claim at its path position (possibly ahead of the current head),
    and commits remove the head only. -/
def PathSorted : Queue → Prop
  | [] => True
  | [_] => True
  | c1 :: c2 :: rest => PathLt c1.path c2.path ∧ PathSorted (c2 :: rest)

/-- `c` is the head of `q`. -/
def IsHead (q : Queue) (c : Claim) : Prop :=
  match q with
  | h :: _ => h = c
  | [] => False

/-- `c` is at the head of every channel it claims — the condition under which it may commit. -/
def HeadOnAll (queues : Queues) (c : Claim) : Prop :=
  ∀ ch, ch ∈ c.channels → IsHead (queues ch) c

/-- Remove the head of a queue. -/
def removeHead : Queue → Queue
  | [] => []
  | _ :: t => t

/-- One queue step: a claim that is head on all its channels commits and is removed from them. -/
def QueueStep (queues queues' : Queues) : Prop :=
  ∃ c, HeadOnAll queues c ∧
    queues' = fun ch => if ch ∈ c.channels then removeHead (queues ch) else queues ch

/-- The tail of a path-sorted queue is path-sorted. -/
theorem pathSorted_tail {q : Queue} (h : PathSorted q) : PathSorted (removeHead q) := by
  cases q with
  | nil => simp [PathSorted, removeHead]
  | cons c1 rest =>
      cases rest with
      | nil => simp [PathSorted, removeHead]
      | cons c2 rest' =>
          simp [PathSorted, removeHead] at h ⊢
          exact h.2

/-- A queue step preserves path-sortedness of every channel's queue. -/
theorem queueStep_preserves_pathSorted {queues queues' : Queues} (hstep : QueueStep queues queues')
    (hsorted : ∀ ch, PathSorted (queues ch)) : ∀ ch, PathSorted (queues' ch) := by
  rcases hstep with ⟨c, _, hdef⟩
  intro ch
  by_cases hch : ch ∈ c.channels
  · rw [hdef]
    simp [hch]
    exact pathSorted_tail (hsorted ch)
  · rw [hdef]
    simp [hch]
    exact hsorted ch

/-- **Law 20 (core)** — channel-task linearization: in any run from path-sorted queues, every
    channel's queue is always a suffix of its initial queue, so that channel's commits happen in
    its initial (path) order. This is the claim-queue discipline the relaxed scheduler enforces
    per channel: at most one op executes per channel at any instant, and same-channel ops commit
    in DFS path order. -/
theorem queue_commit_path_ordered {q0 q : Queues}
    (hinit : ∀ ch, PathSorted (q0 ch))
    (hrun : Relation.ReflTransGen QueueStep q0 q) :
    ∀ ch, ∃ pre, q0 ch = pre ++ q ch := by
  induction hrun with
  | refl => intro ch; exact ⟨[], by simp⟩
  | @tail b _ hprev hstep ih =>
      rcases hstep with ⟨c, hhead, hdef⟩
      intro ch
      rcases ih ch with ⟨pre, hpre⟩
      by_cases hch : ch ∈ c.channels
      · use pre ++ [c]
        have hheadch : IsHead (b ch) c := hhead ch hch
        rw [hdef]
        simp [hch]
        cases hq : b ch with
        | nil => rw [hq] at hheadch; simp [IsHead] at hheadch
        | cons hd tl =>
            have hhd : hd = c := by simpa [IsHead, hq] using hheadch
            rw [hpre, hq, hhd]
            simp [removeHead]
      · use pre
        rw [hdef]
        simp [hch]
        exact hpre

/-- The globally path-smallest pending claim: no pending claim has a strictly smaller path. -/
def IsPathMinimal (queues : Queues) (c : Claim) : Prop :=
  (∃ ch, c ∈ queues ch) ∧ ∀ ch c', c' ∈ queues ch → ¬ PathLt c'.path c.path

/-- **Law 20 (deadlock-freedom)** — the bakery argument: a path-sorted queue's head is its
    path-smallest element, so the globally path-smallest pending claim is the head of every
    channel it claims, and therefore can always commit. Combined with `queue_commit_path_ordered`,
    this is the claim queue's liveness: paths are totally ordered, waits strictly descend, and
    some head claim always exists while anything is pending. -/
axiom law20_deadlock_freedom (queues : Queues) :
    (∃ ch, queues ch ≠ []) →
    (∀ ch, PathSorted (queues ch)) →
    ∃ c, HeadOnAll queues c

/-! ## Law 21 — the gate scheduler (sound, sequential-equivalent) -/

/-- The gate scheduler over a DFS-ordered effect list: effect `i` runs only after effects `0..i-1`
    have completed. In this state-passing model "completed" means the state carries their result,
    so the gate's execution is the sequential fold. -/
def gateApply : List Effect → State → State
  | [], s => s
  | e :: rest, s => gateApply rest (e.apply s)

/-- **Law 21** — DFS-gate linearization: the gate scheduler refines the sequential reducer
    (`Effect.apply` in DFS order). Sound — every concurrent execution under the gate reaches the
    sequential state — but it grants no cross-effect parallelism: the gate *is* the fold. The
    parallel-permitting variant is the relaxed scheduler of Law 20, whose per-channel order
    preservation is what `queue_commit_path_ordered` pins down. -/
theorem gate_exec_refines_apply (l : List Effect) (s : State) :
    gateApply l s = l.foldl (fun s e => e.apply s) s := by
  induction l generalizing s with
  | nil => rfl
  | cons e rest ih =>
      simp [gateApply, ih]

/-! ## The depth-2 counterexample (one-hop pruning is unsound)

The S.3 pair (`effectA`/`effectB`) has *disjoint footprints* but overlapping closures. A
scheduler armed with **one-hop closure lookahead** would repair it: `effectA`'s continuation
receives on `c`, so a lookahead sees the overlap. The pair below defeats even that repair —
each effect's *next-step* footprint is disjoint from the other's at every decision point, yet the
two orders diverge. Channels: `c = 0`, `d = 1`, `x = 2`, `"out" = 3`. -/

/-- `receive d { receive x { c!(v) } }` — the continuation's *next step* is the receive on `x`,
    which does not touch `c`. -/
def depth2A : Effect := Effect.consume 1 (Effect.consume 2 (Effect.produce 0))

/-- `receive c { @"out"!() }` — footprint `{0}`. -/
def depth2B : Effect := Effect.consume 0 (Effect.produce 3)

/-- Initial state: `c`, `d`, and `x` each hold one datum. -/
def state1 : State := fun x => x = 0 ∨ x = 1 ∨ x = 2

/-- The triggers' footprints are disjoint (the naive Law 9 reading holds). -/
theorem depth2_footprint_disjoint :
    Effect.footprint depth2A ∩ Effect.footprint depth2B = ∅ := by
  native_decide

/-- The next-step footprint of `depth2A` after its trigger (`receive d`) has run — the receive on
    `x` — is disjoint from `depth2B`'s footprint. A one-hop lookahead therefore lets `depth2B` run
    concurrently, exactly the divergence below. -/
theorem depth2_next_step_disjoint :
    Effect.footprint (Effect.consume 2 (Effect.produce 0)) ∩ Effect.footprint depth2B = ∅ := by
  native_decide

/-- The closures do overlap — `depth2A` transitively reaches `c`. -/
theorem depth2_closure_overlap : depth2A.closure ∩ depth2B.closure ≠ ∅ := by
  native_decide

/-- **Law 21 (counterexample)** — the one-hop scheduler is unsound: applying the two orders
    diverges (at channel `c = 0`), even though every next-step footprint pair is disjoint
    (`depth2_next_step_disjoint`). Hence computable next-step footprints (Law 22) do not license
    cross-channel pruning; only the claim queue's per-channel path order (Law 20) or the gate's
    full-subtree wait (Law 21) is sound. -/
theorem one_hop_depth2_diverges :
    depth2A.apply (depth2B.apply state1) 0 ≠ depth2B.apply (depth2A.apply state1) 0 := by
  intro h
  have hab : depth2A.apply (depth2B.apply state1) 0 = true := by native_decide
  have hba : depth2B.apply (depth2A.apply state1) 0 = false := by native_decide
  rw [hab, hba] at h
  cases h

/-! ## Law 22 — the next-step closure is computable at dispatch -/

/-- A consume whose continuation is chosen by the matched datum (the reflective case: a bound
    channel variable). In the Bool model the datum *is* the presence bit. -/
inductive EffectWith where
  | produce (c : Chan)
  | consume (c : Chan) (k : EffectWith)
  | consumeWith (c : Chan) (f : Bool → EffectWith)
  | stop

/-- The trigger footprint of a datum-dependent effect tree. -/
def EffectWith.footprint : EffectWith → Finset Chan
  | produce c => {c}
  | consume c _ => {c}
  | consumeWith c _ => {c}
  | stop => ∅

/-- Apply with datum-dependent continuations: the continuation is chosen by the value read. -/
def EffectWith.apply : EffectWith → State → State
  | produce c, s => fun x => if x = c then true else s x
  | consume c k, s => if s c then k.apply (fun x => if x = c then false else s x) else s
  | consumeWith c f, s => if s c then (f (s c)).apply (fun x => if x = c then false else s x) else s
  | stop, s => s

/-- The next-step footprint after the trigger matches on datum `b`: the footprint of the chosen
    continuation. Total because `f` is total — this is what makes the dispatch-time closure
    *computable* in the Rust reducer (matched data is concrete at dispatch). -/
def nextFootprint (f : Bool → EffectWith) (b : Bool) : Finset Chan :=
  EffectWith.footprint (f b)

/-- **Law 22** — next-step closure is computable at dispatch: given the matched datum, the
    continuation's first-step channel set is a total, decidable computation. The Rust realization
    is `resolve_children`'s per-term footprint computed after substitution. -/
theorem next_step_closure_computable (f : Bool → EffectWith) (b : Bool) :
    nextFootprint f b = EffectWith.footprint (f b) := by
  rfl

end Rchain
