import Rchain.Effect
import Rchain.Par

/-!
# The path-ordered scheduler — Laws 20–22

`Rchain.Effect` established that the *static* channel-sharded effect scheduler is unsound
(`effect_reorder_diverges`): disjoint **footprints** do not commute, only disjoint **closures** do
(`effect_commute_of_disjoint_closure`), and closures are not statically decidable. This module
formalizes the scheduler that *is* sound, and the one that is *usefully parallel*, at the effect
level (`docs/src/formal/scheduling.md`):

* **Law 20 — channel-task linearization** ("1 channel = 1 logical task"). Every channel owns a
  *claim queue*; an effect claims its channels at its DFS path, and commits only when it is the
  head (the path-smallest pending claim) of **every** claimed channel. `queue_commit_path_ordered`
  proves that queue steps preserve path-sortedness and that a channel's commit sequence follows
  its initial (path-sorted) queue order; `pathSorted_head_minimal` is the bakery argument's core —
  a path-sorted queue's head is its path-smallest element, so the queue's head can always commit.

* **Law 21 — DFS-gate linearization**. The *gate* scheduler runs effect `i` only after every
  earlier effect's subtree has completed. The Rust builds that with a **chain of awaits**
  (`reduce.rs:2334-2351`: task `i` holds task `i−1`'s handle and awaits it first), and the claim
  worth proving is that the immediate-predecessor chain is *transitively complete* —
  `gate_await_closure_orders` — which is what "not the quadratic all-predecessors join" means. The
  tempting repair — a **one-hop** scheduler that lets a sibling run once an effect's *next-step*
  footprint is disjoint — is unsound: `one_hop_depth2_diverges` exhibits a pair whose next-step
  footprints are disjoint at every decision point yet whose closures overlap, so the one-hop
  interleaving reaches a state the sequential reducer never reaches.

* **Law 22 — next-step closure is computable at dispatch.** Once a trigger matches, the matched
  datum is concrete, so the continuation chosen by that datum has a *computable* first-step
  footprint — the reason the Rust reducer can resolve a `Par`'s terms concurrently
  (`resolve_children`, `reduce.rs:2228-2257`, whose own comment is the claim: "Pure w.r.t. the
  tuple space … so it can run concurrently across a `Par`'s terms and still produce the same result
  as the sequential interpreter"). That positive half is a fact about a **signature** — the resolver
  takes no store — and the section below says why it is not dressed as a theorem here, and why the
  tempting distributivity law is false. What is proved is the negative half,
  `depth2_next_step_disjoint`: this computability does **not** make cross-channel pruning sound.

## What the consolidation pass changed here (and why)

Three statements in this module were true by construction and are gone, replaced by statements
about the code:

* `next_step_closure_computable` was `nextFootprint f b = EffectWith.footprint (f b)` — both sides
  the same expression, `by rfl`. It said nothing about `resolve_children`, which is what it cited.
* `gate_exec_refines_apply` defined `gateApply` as the sequential fold and then proved the fold is
  the fold. The Rust comment above the gate says as much ("Not a speedup — the sound,
  sequential-equivalent carrier"), so the content is in the *dependency structure* (Law 21 above),
  not in the identification.
* `law20_deadlock_freedom` was an **axiom**, and unprovable as stated: it quantified over every
  `Chan → Queue`, so a set of pending claims could have paths `[0] > [0,0] > [0,0,0] > …` with no
  minimum (`PathLt` is not well-founded on all paths). The real system's paths are bounded by the
  depth of the term being reduced and its pending set is finite, so the corrected statement needs
  that finiteness as a hypothesis — the finding is recorded in the register's Law 20 row, and what
  is proved here is the queue-level core that needs no such hypothesis.
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

/-- **Law 20 (core)** — channel-task linearization: every channel's queue is always a suffix of its
    initial queue, so that channel's commits happen in the order they were enqueued. This is the
    claim-queue discipline the relaxed scheduler enforces per channel: at most one op executes per
    channel at any instant, and same-channel ops commit in DFS path order.

    **No sortedness hypothesis** — and that is a finding, not a slip: the mechanism that makes the
    suffix property true is the commit rule (`removeHead` on a claim that is head-on-all), which
    preserves order whatever the queues start as. Path-*sortedness* is what makes a head
    *path-smallest*, which is `pathSorted_head_minimal`'s hypothesis, not this theorem's. The
    previous version of this statement carried `hinit : ∀ ch, PathSorted (q0 ch)` and never used it
    — the same unused-hypothesis shape as Law 24's `DFSSerializable`, found by the same pass. -/
theorem queue_commit_path_ordered {q0 q : Queues}
    (hrun : Relation.ReflTransGen QueueStep q0 q) :
    ∀ ch, ∃ pre, q0 ch = pre ++ q ch := by
  induction hrun with
  | refl => intro ch; exact ⟨[], by simp⟩
  | @tail b _ _ hstep ih =>
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

/-! ### The bakery argument's core, and the finiteness it needs

A path-sorted queue's head is its path-smallest element, which is why the head can always commit:
anything ahead of it in a queue it claims would have to have a strictly smaller path, and a strictly
smaller path is exactly what "head" excludes. That argument needs no finiteness hypothesis, and it is
what is proved here.

What *does* need one is the global statement the axiom this replaces made — that some claim is
head-on-all. `PathLt` is **not** well-founded on paths: `[0] > [0,0] > [0,0,0] > …` is an infinite
descending chain, so an unbounded set of pending claims can have no minimum. The real system's paths
are bounded by the depth of the term being reduced and its pending set is finite, so the corrected
statement carries that finiteness — recorded in the register's Law 20 row rather than assumed here.
-/

/-- `PathLt` is irreflexive: no path precedes itself. -/
theorem PathLt_irrefl : ∀ (a : DfsPath), ¬ PathLt a a
  | [], h => by cases h
  | _ :: _, h => by
      cases h with
      | cons hlt => exact absurd rfl (Nat.ne_of_lt hlt)
      | consEq h' => exact PathLt_irrefl _ h'

/-- `PathLt` is transitive. -/
theorem PathLt_trans : ∀ {a b c : DfsPath}, PathLt a b → PathLt b c → PathLt a c
  | _, _, _, .nilCons b bs, hbc => by cases hbc <;> exact .nilCons _ _
  | _, _, _, .cons hlt, hbc => by
      cases hbc with
      | cons hlt2 => exact .cons (Nat.lt_trans hlt hlt2)
      | consEq _ => exact .cons hlt
  | _, _, _, .consEq hab, hbc => by
      cases hbc with
      | cons hlt => exact .cons hlt
      | consEq hbc' => exact .consEq (PathLt_trans hab hbc')

/-- In a path-sorted queue, everything after the head has a strictly larger path. -/
theorem pathSorted_lt_of_mem {c0 : Claim} {rest : Queue} (hs : PathSorted (c0 :: rest)) :
    ∀ c ∈ rest, PathLt c0.path c.path := by
  induction rest generalizing c0 with
  | nil => intro c hc; simp at hc
  | cons c1 rest' ih =>
      intro c hc
      have hpair : PathLt c0.path c1.path ∧ PathSorted (c1 :: rest') := by
        cases rest' <;> simpa [PathSorted] using hs
      rcases List.mem_cons.mp hc with h | h
      · subst h; exact hpair.1
      · exact PathLt_trans hpair.1 (ih hpair.2 c h)

/-- **Law 20 (the bakery argument's core)** — the head of a path-sorted queue is that queue's
    path-smallest element: nothing in the queue has a strictly smaller path than the head, so the
    head is exactly the claim that may commit first on that channel. -/
theorem pathSorted_head_minimal {q : Queue} {hd : Claim} (hs : PathSorted q) (hh : IsHead q hd) :
    ∀ c ∈ q, ¬ PathLt c.path hd.path := by
  intro c hc hlt
  cases q with
  | nil => simp [IsHead] at hh
  | cons c0 rest =>
      have hhd : c0 = hd := by simpa [IsHead] using hh
      subst hhd
      rcases List.mem_cons.mp hc with h | h
      · subst h; exact PathLt_irrefl _ hlt
      · exact PathLt_irrefl _ (PathLt_trans (pathSorted_lt_of_mem hs c h) hlt)

/-! ## Law 21 — the gate scheduler, as the dependency structure it is -/

/-- The gate scheduler over a DFS-ordered effect list: effect `i` runs only after effects `0..i-1`
    have completed. This is the *sequential reference* the gate must be equivalent to; in this
    state-passing model "completed" means the state carries their result, so the reference is the
    fold. What the gate's *code* adds is the dependency chain below, and that is where the claim
    with content lives. -/
def gateApply : List Effect → State → State
  | [], s => s
  | e :: rest, s => gateApply rest (e.apply s)

/-- The gate's dependency, as the Rust builds it (`reduce.rs:2334-2351`): the task for effect `i`
    holds the handle of the task for `i−1` and awaits it first, so the relation is
    `j + 1 = i` among `n` tasks. -/
def GateAwaits (n : Nat) : Nat → Nat → Prop := fun j i => i < n ∧ j + 1 = i

/-- **Law 21** — the immediate-predecessor chain is **transitively complete**: every task awaits
    every earlier task through it. This is exactly what the Rust's comment claims — "Awaiting the
    immediate predecessor alone suffices because that task itself awaits its own — a linear chain
    of awaits, not the quadratic all-predecessors join" (`reduce.rs:2341-2345`) — and it is the
    content the identification "the gate is the fold" lacks: the *transitive closure* of a linear
    dependency orders all earlier tasks, so no task can observe a missing dependency. A chain with
    a missing link (say `j + 2 = i` for even `i`) would leave odd-indexed tasks unordered, and this
    theorem would be false. -/
theorem gate_await_closure_orders {n j i : Nat}
    (h : Relation.TransGen (GateAwaits n) j i) : j < i := by
  induction h with
  | single hab => obtain ⟨_, hji⟩ := hab; omega
  | tail _ hbc ih => obtain ⟨_, hcb⟩ := hbc; omega

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

/-! ## Law 22 — closure computability is a signature fact, and the *negative* half is the law

`EffectWith` and its `footprint`/`apply` are the datum-dependent effect tree Law 23's
`read_state_determines_outcome` is stated over, so they live here and are used there.

What this module does **not** claim about Law 22, and why:

* The positive half — "the next-step closure is computable at dispatch" — is a fact about a
  *signature*, not a theorem about a definition. `resolve_children` takes a `Par`, an `Env` and a
  `Blake2b512Random` and **no store** (`reduce.rs:2228-2234`), and its own comment says what that
  buys: "Pure w.r.t. the tuple space … so it can run concurrently across a `Par`'s terms and still
  produce the same result as the sequential interpreter". Stating it in Lean as "two stores give the
  same resolution" would prove something only because the model's resolver ignores a parameter
  nobody would pass — exactly the definition-shuffling this pass exists to remove. The signature
  *is* the claim, and a reader checks it by reading the signature.
* The tempting positive *law* — resolving `p | q` term-wise gives the resolution of `p` followed by
  that of `q` — is **false** for the Rust's shape, and a proof attempt is what showed it.
  `resolve_children` flattens *all* sends before *all* receives (`reduce.rs:2234-2257`), so a merge
  interleaves differently than a concatenation: with one send and one receive in each half, the
  merged resolution is `[send_p, send_q, recv_p, recv_q]` while the concatenation is
  `[send_p, recv_p, send_q, recv_q]`. The concurrency licence therefore rests on *per-term
  independence* — each term resolved on its own, with no store — and not on distributivity. (The
  statement was written, would not close, and is recorded rather than deleted quietly.)
* The half with content is the **negative** one, and it is proved above:
  `depth2_next_step_disjoint` — computability does not license cross-channel pruning. The register's
  Law 22 row states it that way rather than claiming a computability theorem this model cannot give
  content to.
-/

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

end Rchain
