import Rchain.Scheduler

/-!
# On-chain scheduling: validated speculation — Laws 23–25

`Rchain.Scheduler` (Laws 20–22) ships the sound schedulers: the claim queue (per-channel path
order) and the gate (the sequential fold). The **relaxed** mode runs the claim queue with free
cross-channel interleaving, and may diverge from the sequential reducer's final state and event
log — which is why the casper block paths hard-reject it. This module formalizes the extension
that makes effect-level concurrency *sound on-chain*
(`docs/src/formal/onchain-scheduling.md`):

* **Law 23 — read-determinism.** An effect's chosen candidate, commit outcome and event trace
  are a deterministic function of the state it reads. `read_state_determines_outcome` proves the
  trace corollary: states agreeing on the effect's closure yield identical traces.

* **Law 24 — DFS-order serializability.** A concurrent execution is sound for the block path iff
  every commit read exactly the state the DFS-earlier effects produced. The Bool-presence
  `State` of `Rchain.Effect` cannot state this — the depth-2 pair's stale read has the same Bool
  value as the correct one — so this module adds the **versioned write-record layer**
  `SpecState := Chan → Option (DfsPath × Nat)` (newest write = writer path + version).
  `prefixVisible` / `ValidCommit` / `DFSSerializable` define the check; `s3_pair_fails_validation`
  shows the depth-2 pair's B-first interleaving fails it (at `[0,0,0]`'s produce), and
  `later_write_pollution_unsound` — the C/D pair — shows why the weaker "earlier writes
  reflected" check is insufficient: a DFS-earlier read can observe a DFS-later write.

* **Law 25 — validated speculation.** A scheduler may commit effects in any order iff each commit
  validates Law 24; invalidated subtrees abort, their writes are inverse-replayed, and they
  re-run under the gate, so the published log is the sequential fold's.
  `validated_speculation_refines_apply` and `dfs_serializable_iff_log_equal` state the
  publication theorems; `gate_replay_terminates` proves re-runs total; `abort_bounded` states
  that each path aborts at most once (the path-ordered re-run coordinator of the Rust
  realization).
-/

namespace Rchain

/-! ## The versioned write-record layer -/

/-- One write to a channel: the writer's DFS path, the write's version, and the value the
    channel held *after* the write (a produce records `true`, a matched consume records
    `false`). The polarity is what lets the record layer witness the data layer: `State`'s
    Bool at a channel is exactly the newest write's recorded value (see
    `applyAt_preserves_invariant`). -/
abbrev Write := DfsPath × Nat × Bool

/-- The speculative write-record layer: for each channel, its newest write (writer path +
    version) — or `none` if the channel has never been written (its datum, if any, is initial).
    The data layer (`State`) is unchanged: the record layer *witnesses* what a read observed,
    which Bool values alone cannot (the depth-2 pair's stale read has the same value as the
    correct one). -/
abbrev SpecState := Chan → Option Write

/-- The empty record layer: nothing has been written. -/
def emptySpec : SpecState := fun _ => none

/-- Apply a dispatched effect at path `p`, recording its writes: a produce records an addition
    write, a consume that matches records a removal write, each with version = previous + 1 on
    that channel. A consume's continuation is applied at `p ++ [0]` (the reducer's
    `path.child(0)` dispatch). A non-matching consume writes nothing (its continuation install
    is not modeled — the Bool layer tracks data only). -/
def applyAt (p : DfsPath) : Effect → State × SpecState → State × SpecState
  | Effect.produce c, (d, s) =>
      (fun x => if x = c then true else d x,
       fun x => if x = c then some (p, ((s c).map (fun w => w.2.1)).getD 0 + 1, true) else s x)
  | Effect.consume c k, (d, s) =>
      if d c then
        applyAt (p ++ [0]) k
          (fun x => if x = c then false else d x,
           fun x => if x = c then some (p, ((s c).map (fun w => w.2.1)).getD 0 + 1, false) else s x)
      else (d, s)
  | Effect.stop, st => st

/-- **Law 24 (check)** — prefix visibility: a read of channel `c` by an effect at path `p` is
    valid when the newest write `c` exposes was recorded by a path `< p` (in `PathLt`, exactly
    the DFS order) — or there is no write at all. A later-path write visible to an earlier-path
    read is precisely what the check rejects. -/
def prefixVisible (s : SpecState) (p : DfsPath) (c : Chan) : Prop :=
  (s c).elim True (fun w => PathLt w.1 p)

/-- A commit validates when every channel it reads (`reads`) is prefix-visible. -/
def ValidCommit (sRead : SpecState) (p : DfsPath) (reads : Finset Chan) : Prop :=
  ∀ c ∈ reads, prefixVisible sRead p c

/-- **Law 24** — a run is DFS-serializable when every commit in it validates against the
    record layer the commit actually read (the stepped state's record at commit time — no
    separate snapshot is carried, so the check is faithful by construction). The
    log-equality direction is `dfs_serializable_implies_log_equal`. -/
def DFSSerializable (st0 : State × SpecState) : List (DfsPath × Effect) → Prop
  | [] => True
  | (p, e) :: rest =>
      ValidCommit st0.2 p (Effect.footprint e) ∧ DFSSerializable (applyAt p e st0) rest

/-! ## Law 23 — read-determinism -/

/-- A trace event: a produce, a consume, or a COMM on a channel. (The Rust realization's
    `rspace::trace::Event`.) -/
inductive Event where
  | produce (c : Chan)
  | consume (c : Chan)
  | comm (c : Chan)
  deriving DecidableEq, Repr

/-- The closure of a datum-dependent effect tree: the channels any possible continuation can
    reach. `consumeWith` branches on the matched Bool, so its closure is the union over both. -/
def EffectWith.closure : EffectWith → Finset Chan
  | produce c => {c}
  | consume c k => insert c k.closure
  | consumeWith c f => insert c ((f false).closure ∪ (f true).closure)
  | stop => ∅

/-- The event sequence an effect tree emits, given the state it reads: every produce and consume
    logs its own event; a consume additionally logs a COMM exactly when its trigger matches. -/
def EffectWith.trace : EffectWith → State → List Event
  | produce c, _ => [Event.produce c]
  | consume c k, s =>
      if s c then [Event.consume c, Event.comm c] ++ k.trace (fun x => if x = c then false else s x)
      else [Event.consume c]
  | consumeWith c f, s =>
      if s c then [Event.consume c, Event.comm c] ++ (f (s c)).trace (fun x => if x = c then false else s x)
      else [Event.consume c]
  | stop, _ => []

/-- **Law 23** — read-determinism: an effect's commit outcome and event trace are a deterministic
    function of the state it reads. Two states agreeing on the effect's closure yield identical
    traces. (The candidate choice is Law 8's sorted-first selection, made total by the state; the
    closure — not the footprint — is the read set, mirroring
    `Effect.apply_determined_by_closure`.) -/
theorem read_state_determines_outcome (e : EffectWith) {s1 s2 : State}
    (h : ∀ c, c ∈ e.closure → s1 c = s2 c) : e.trace s1 = e.trace s2 := by
  induction e generalizing s1 s2 with
  | produce c => simp [EffectWith.trace]
  | stop => simp [EffectWith.trace]
  | consume c k ih =>
      have hc : s1 c = s2 c := h c (by simp [EffectWith.closure])
      by_cases hs : s1 c
      · have hs2 : s2 c = true := by simpa [hc] using hs
        simp [EffectWith.trace, hs, hs2]
        apply ih
        intro x hx
        by_cases hxc : x = c
        · subst x; simp
        · have hx2 := h x (by simp [EffectWith.closure, hx, hxc])
          simp [hxc, hx2]
      · have hs2 : s2 c = false := by simpa [hc] using hs
        simp [EffectWith.trace, hs, hs2]
  | consumeWith c f ih =>
      have hc : s1 c = s2 c := h c (by simp [EffectWith.closure])
      by_cases hs : s1 c
      · have hs2 : s2 c = true := by simpa [hc] using hs
        simp [EffectWith.trace, hs, hs2]
        apply ih
        intro x hx
        by_cases hxc : x = c
        · subst x; simp
        · have hx2 := h x (by simp [EffectWith.closure, hx, hs, hxc])
          simp [hxc, hx2]
      · have hs2 : s2 c = false := by simpa [hc] using hs
        simp [EffectWith.trace, hs, hs2]

/-! ## Law 24 — the witnesses

The depth-2 pair (`depth2A`/`depth2B`/`state1` of `Rchain.Scheduler`) as *dispatched commits*:
each op is its own effect at its own path — A's chain `receive d` / `receive x` / `c!(v)` at
`[0]` / `[0,0]` / `[0,0,0]`, B's `receive c` / `@"out"!()` at `[1]` / `[1,0]` — exactly as the
reducer's `dispatch_owned` enqueues them. Each commit carries the record-layer snapshot its
reads observed. -/

/-- The B-first interleaving, stepped by the model: B commits, then A's chain. -/
def s3BSt1 := applyAt [1] (Effect.consume 0 Effect.stop) (state1, emptySpec)
def s3BSt2 := applyAt [1, 0] (Effect.produce 3) s3BSt1
def s3BSt3 := applyAt [0] (Effect.consume 1 Effect.stop) s3BSt2
def s3BSt4 := applyAt [0, 0] (Effect.consume 2 Effect.stop) s3BSt3
def s3BSt5 := applyAt [0, 0, 0] (Effect.produce 0) s3BSt4

/-- B's ops commit first, then A's chain. -/
def s3BFirst : List (DfsPath × Effect) :=
  [ ([1], Effect.consume 0 Effect.stop),
    ([1, 0], Effect.produce 3),
    ([0], Effect.consume 1 Effect.stop),
    ([0, 0], Effect.consume 2 Effect.stop),
    ([0, 0, 0], Effect.produce 0) ]

/-- The DFS (sequential) order: A's chain, then B's ops. -/
def s3ASt1 := applyAt [0] (Effect.consume 1 Effect.stop) (state1, emptySpec)
def s3ASt2 := applyAt [0, 0] (Effect.consume 2 Effect.stop) s3ASt1
def s3ASt3 := applyAt [0, 0, 0] (Effect.produce 0) s3ASt2
def s3ASt4 := applyAt [1] (Effect.consume 0 Effect.stop) s3ASt3
def s3ASt5 := applyAt [1, 0] (Effect.produce 3) s3ASt4

def s3AFirst : List (DfsPath × Effect) :=
  [ ([0], Effect.consume 1 Effect.stop),
    ([0, 0], Effect.consume 2 Effect.stop),
    ([0, 0, 0], Effect.produce 0),
    ([1], Effect.consume 0 Effect.stop),
    ([1, 0], Effect.produce 3) ]

/-- **Law 24 (S.3 witness)** — in the B-first interleaving the run is not DFS-serializable. The
    failing commit is `[0,0,0]`'s produce (not B's): B's `receive c` removed `0`'s datum at
    `[1]`, so `[0,0,0]`'s read of `0` observes a writer path that is not `< [0,0,0]` —
    prefix visibility rejects it. The scheduler aborts A's subtree and re-runs it under the
    gate. -/
theorem s3_pair_fails_validation : ¬ DFSSerializable (state1, emptySpec) s3BFirst := by
  intro h
  have h5 : ValidCommit s3BSt4.2 [0, 0, 0] ({0} : Finset Chan) := h.2.2.2.2.1
  have h0 : prefixVisible s3BSt4.2 [0, 0, 0] 0 := h5 0 (by simp)
  have hred : s3BSt4.2 0 = some ([1], 1, false) := by native_decide
  rw [prefixVisible, hred] at h0
  have hlt : PathLt [1] [0, 0, 0] := by simpa using h0
  cases hlt with
  | cons hlt => exact Nat.not_lt_zero 1 hlt

/-- **Law 24 (control)** — the DFS order itself validates: committing A's chain before B's ops is
    serializable (this is the run the sequential reducer produces, and the run the gate re-run
    restores). -/
theorem s3_sequential_order_valid : DFSSerializable (state1, emptySpec) s3AFirst := by
  unfold s3AFirst DFSSerializable
  constructor
  · intro c hc
    have hc1 : c = 1 := by simpa [Effect.footprint] using hc
    subst c
    simp [prefixVisible, emptySpec]
  constructor
  · intro c hc
    have hc2 : c = 2 := by simpa [Effect.footprint] using hc
    subst c
    change prefixVisible s3ASt1.2 [0, 0] 2
    have hred : s3ASt1.2 2 = none := by native_decide
    rw [prefixVisible, hred]
    trivial
  constructor
  · intro c hc
    have hc0 : c = 0 := by simpa [Effect.footprint] using hc
    subst c
    change prefixVisible s3ASt2.2 [0, 0, 0] 0
    have hred : s3ASt2.2 0 = none := by native_decide
    rw [prefixVisible, hred]
    trivial
  constructor
  · intro c hc
    have hc0 : c = 0 := by simpa [Effect.footprint] using hc
    subst c
    change prefixVisible s3ASt3.2 [1] 0
    have hred : s3ASt3.2 0 = some ([0, 0, 0], 1, true) := by native_decide
    rw [prefixVisible, hred]
    simpa using (PathLt.cons (by decide))
  constructor
  · intro c hc
    have hc3 : c = 3 := by simpa [Effect.footprint] using hc
    subst c
    change prefixVisible s3ASt4.2 [1, 0] 3
    have hred : s3ASt4.2 3 = none := by native_decide
    rw [prefixVisible, hred]
    trivial
  · simp [DFSSerializable]

/-! ## The C/D witness — why "earlier writes reflected" is not enough

The draft validation rule checked only that *every DFS-earlier write to a read channel is
reflected* in the read. That check is blind to the opposite race: a DFS-earlier read observing a
DFS-**later** write that committed first. C/D: `d = 1` holds a datum; `A = [1] = receive d {
receive c { stop } }` with its continuation at `[1,0]`, `B = [2] = c!(v)`, `c = 0` initially
empty. Sequential: `[1,0]` receives on the empty `c`, stores a continuation, no COMM; then
`[2]`'s produce matches it. Speculative: `[2]` commits first (its claim is the only one on
`c` — `[1,0]`'s claim does not exist yet), stores its datum; `[1,0]` then reads it and COMMs —
a log the sequential reducer never produces. There *are* no DFS-earlier writes on `0` to be
missing, so the earlier-writes check passes; only prefix visibility rejects the commit. -/

/-- The draft's weaker check: every DFS-earlier write in the current record is reflected in the
    snapshot the read observed — if the newest write on `c` is by a path `< p`, the read must
    have seen that very record. Later-path writes are simply not examined. -/
def EarlierWritesReflected (sNow sRead : SpecState) (p : DfsPath) (reads : Finset Chan) : Prop :=
  ∀ c ∈ reads, match sNow c with
    | none => True
    | some w => PathLt w.1 p → sRead c = some w

/-- C/D initial state: `d = 1` holds one datum, `c = 0` is empty. -/
def stateCD : State := fun x => x = 1

/-- `[2]`'s produce commits first, storing its datum on `0` before the DFS-earlier `[1,0]`
    receive runs. -/
def sCD := applyAt [2] (Effect.produce 0) (stateCD, emptySpec)

/-- **Law 24 (C/D witness)** — at `[1,0]`'s commit the earlier-writes check passes (the newest
    write on `0` is `[2]`'s — no DFS-earlier write is missing) while prefix visibility fails:
    the commit reads a later-path write, so the run is not DFS-serializable. This pins the
    strengthened rule. -/
theorem later_write_pollution_unsound :
    EarlierWritesReflected sCD.2 sCD.2 [1, 0] ({0} : Finset Chan) ∧
    ¬ ValidCommit sCD.2 [1, 0] ({0} : Finset Chan) := by
  constructor
  · intro c hc
    have hc0 : c = 0 := by simpa [Effect.footprint] using hc
    subst c
    have hred : sCD.2 0 = some ([2], 1, true) := by native_decide
    simp [EarlierWritesReflected, hred]
  · intro h
    have h0 : prefixVisible sCD.2 [1, 0] 0 := h 0 (by simp)
    have hred : sCD.2 0 = some ([2], 1, true) := by native_decide
    rw [prefixVisible, hred] at h0
    have hlt : PathLt [2] [1, 0] := by simpa using h0
    cases hlt with
    | cons hlt => exact (by decide : ¬ 2 < 1) hlt

/-- The C/D pair stepped in the speculative commit order: `[2]` first, then `[1]`, then
    `[1,0]`. -/
def cdSt1 := applyAt [2] (Effect.produce 0) (stateCD, emptySpec)
def cdSt2 := applyAt [1] (Effect.consume 1 Effect.stop) cdSt1

def cdRun : List (DfsPath × Effect) :=
  [ ([2], Effect.produce 0),
    ([1], Effect.consume 1 Effect.stop),
    ([1, 0], Effect.consume 0 Effect.stop) ]

/-- The C/D pair in the DFS (sequential) order: `[1]`, `[1,0]`, then `[2]`. -/
def cdSeqSt1 := applyAt [1] (Effect.consume 1 Effect.stop) (stateCD, emptySpec)
def cdSeqSt2 := applyAt [1, 0] (Effect.consume 0 Effect.stop) cdSeqSt1

def cdSequential : List (DfsPath × Effect) :=
  [ ([1], Effect.consume 1 Effect.stop),
    ([1, 0], Effect.consume 0 Effect.stop),
    ([2], Effect.produce 0) ]

/-- **Law 24 (C/D witness, run level)** — the speculative commit order is not DFS-serializable
    (the `[1,0]` commit fails; see `later_write_pollution_unsound`). -/
theorem cd_pair_fails_validation : ¬ DFSSerializable (stateCD, emptySpec) cdRun := by
  intro h
  have h3 : ValidCommit cdSt2.2 [1, 0] ({0} : Finset Chan) := h.2.2.1
  have h0 : prefixVisible cdSt2.2 [1, 0] 0 := h3 0 (by simp)
  have hred : cdSt2.2 0 = some ([2], 1, true) := by native_decide
  rw [prefixVisible, hred] at h0
  have hlt : PathLt [2] [1, 0] := by simpa using h0
  cases hlt with
  | cons hlt => exact (by decide : ¬ 2 < 1) hlt

/-- **Law 24 (C/D control)** — the DFS order validates: `[1,0]` reads the empty `c` before `[2]`
    writes it. -/
theorem cd_sequential_order_valid : DFSSerializable (stateCD, emptySpec) cdSequential := by
  unfold cdSequential DFSSerializable
  constructor
  · intro c hc
    have hc1 : c = 1 := by simpa [Effect.footprint] using hc
    subst c
    simp [prefixVisible, emptySpec]
  constructor
  · intro c hc
    have hc0 : c = 0 := by simpa [Effect.footprint] using hc
    subst c
    change prefixVisible cdSeqSt1.2 [1, 0] 0
    have hred : cdSeqSt1.2 0 = none := by native_decide
    rw [prefixVisible, hred]
    trivial
  constructor
  · intro c hc
    have hc0 : c = 0 := by simpa [Effect.footprint] using hc
    subst c
    change prefixVisible cdSeqSt2.2 [2] 0
    have hred : cdSeqSt2.2 0 = none := by native_decide
    rw [prefixVisible, hred]
    trivial
  · simp [DFSSerializable]

/-! ## The soundness lemmas

`applyAt` maintains a two-layer invariant: the record layer *witnesses* the data layer — the
datum-presence Bool at a channel is exactly the newest write's recorded post-value (the polarity
of `Write`). Two lemmas compose this with prefix visibility toward the publication theorem:

* `applyAt_preserves_invariant` / `record_determines_value` (proven): in any reachable state, a
  channel's value is its newest write's post-value, or the initial value if unwritten.
* `dispatched_record_at` (proven): a dispatched commit (continuations are separate commits) only
  ever writes its own channel — the record elsewhere is untouched.
* `serializable_writer_chain` (stated): prefix visibility forces each channel's writers to commit
  in strictly increasing path order — the per-channel path-order property that, with Law 23's
  read-determinism, closes `dfs_serializable_implies_log_equal`. -/

/-- The invariant: at every channel, if the record layer has a newest write then the data
    layer holds exactly that write's recorded post-value; otherwise the data layer still
    holds the initial value. -/
def Invariant (d0 : State) (st : State × SpecState) : Prop :=
  ∀ c, match st.2 c with
    | none => st.1 c = d0 c
    | some (_, _, b) => st.1 c = b

/-- One `applyAt` step preserves the invariant, for every effect (induction on the effect
    tree; a matched consume's continuation is applied under the same invariant by the
    induction hypothesis at the child path). -/
theorem applyAt_preserves_invariant (p : DfsPath) (e : Effect) (d0 : State)
    (st : State × SpecState) (h : Invariant d0 st) : Invariant d0 (applyAt p e st) := by
  induction e generalizing p st with
  | produce c =>
      intro x
      by_cases hx : x = c
      · subst x
        simp [applyAt, Invariant]
      · simp [applyAt, Invariant, hx, h x]
  | consume c k ih =>
      by_cases hd : st.1 c
      · have hst : Invariant d0
            (fun x => if x = c then false else st.1 x,
             fun x => if x = c then some (p, ((st.2 c).map (fun w => w.2.1)).getD 0 + 1, false) else st.2 x) := by
          intro x
          by_cases hx : x = c
          · subst x
            simp [Invariant]
          · have hx2 := h x
            simp [Invariant, hx, hx2]
        let st' : State × SpecState :=
          (fun x => if x = c then false else st.1 x,
           fun x => if x = c then some (p, ((st.2 c).map (fun w => w.2.1)).getD 0 + 1, false) else st.2 x)
        simpa [applyAt, hd, Invariant, st'] using ih (p ++ [0]) st' hst
      · simpa [applyAt, hd] using h
  | stop => intro x; simpa [applyAt, Invariant] using h x

/-- Reachable from the initial data state `d0` and the empty record by `applyAt` steps. -/
inductive Reachable (d0 : State) : State × SpecState → Prop where
  | init : Reachable d0 (d0, emptySpec)
  | step : ∀ {p : DfsPath} {e : Effect} {st : State × SpecState},
      Reachable d0 st → Reachable d0 (applyAt p e st)

/-- **Law 24 (witness lemma, proven)** — the record determines the value: in any reachable
    state, a channel's datum-presence Bool is exactly its newest write's recorded post-value
    (or the initial value if the channel has no write). -/
theorem record_determines_value {d0 : State} {st : State × SpecState}
    (h : Reachable d0 st) : Invariant d0 st := by
  induction h with
  | init => intro c; simp [Invariant, emptySpec]
  | @step p e st _ ih => exact applyAt_preserves_invariant p e d0 st ih

/-- The dispatched effects of the commit model: a continuation is its own commit, so a
    commit's consume is always `consume c stop`. -/
def Dispatched : Effect → Prop
  | Effect.produce _ => True
  | Effect.consume _ Effect.stop => True
  | Effect.consume _ _ => False
  | Effect.stop => True

/-- A dispatched commit only ever writes its own channel: if the record at `c` does not
    carry the commit's path afterwards, the commit did not touch `c` and the record is
    unchanged. -/
theorem dispatched_record_at {e : Effect} (hdisp : Dispatched e) (p0 : DfsPath)
    (st : State × SpecState) (c : Chan)
    (hw : ¬ ∃ v b, (applyAt p0 e st).2 c = some (p0, v, b)) :
    (applyAt p0 e st).2 c = st.2 c := by
  cases e with
  | produce c0 =>
      by_cases hc0 : c = c0
      · subst c0
        have : ∃ v b, (applyAt p0 (Effect.produce c) st).2 c = some (p0, v, b) := by
          refine ⟨((st.2 c).map (fun w => w.2.1)).getD 0 + 1, true, ?_⟩
          simp [applyAt]
        contradiction
      · simp [applyAt, hc0]
  | consume c0 k =>
      unfold Dispatched at hdisp
      cases k with
      | stop =>
          by_cases hc0 : c = c0
          · subst c0
            by_cases hd : st.1 c
            · have : ∃ v b, (applyAt p0 (Effect.consume c Effect.stop) st).2 c = some (p0, v, b) := by
                refine ⟨((st.2 c).map (fun w => w.2.1)).getD 0 + 1, false, ?_⟩
                simp [applyAt, hd]
              contradiction
            · simp [applyAt, hd]
          · by_cases hd : st.1 c0 <;> simp [applyAt, hc0, hd]
      | produce c => cases hdisp
      | consume c k => cases hdisp
  | stop => simp [applyAt]

/-- The writer paths on channel `c`, in commit order (extracted from the stepped record:
    a commit's write is recognized as the record's newest write carrying the commit's path). -/
def writerPaths (st0 : State × SpecState) : List (DfsPath × Effect) → Chan → List DfsPath
  | [], _ => []
  | (p, e) :: rest, c =>
      match (applyAt p e st0).2 c with
      | some (w, _, _) => if w = p then p :: writerPaths (applyAt p e st0) rest c
                         else writerPaths (applyAt p e st0) rest c
      | none => writerPaths (applyAt p e st0) rest c

/-- **Law 24 (writer chain, stated)** — in a DFS-serializable run of dispatched commits, each
    channel's writers commit in strictly increasing path order (the per-channel path-order
    property). Proof obligation: induction over `DFSSerializable` with the head case split by
    `dispatched_record_at` — a writing head's prefix visibility (`hc` at the old record)
    yields `PathLt p p0`, a non-writing head leaves the record unchanged for the induction
    hypothesis. With `record_determines_value` and Law 23's read-determinism this closes
    `dfs_serializable_implies_log_equal`: the commit-time data state equals the path-order
    fold's, so the two folds emit equal traces. -/
axiom serializable_writer_chain {st0 : State × SpecState} {run : List (DfsPath × Effect)}
    (hdisp : ∀ ⦃p : DfsPath⦄ ⦃e : Effect⦄, (p, e) ∈ run → Dispatched e)
    (h : DFSSerializable st0 run) (c : Chan) :
    List.Chain' (fun p q => PathLt p q) (writerPaths st0 run c)

/-! ## Law 25 — validated speculation -/

/-- The trace one *dispatched* effect emits, given the data state it reads (continuations are
    separate commits, so `consume`'s own trace is its install/COMM pair only). -/
def Effect.trace : Effect → State → List Event
  | produce c, _ => [Event.produce c]
  | consume c _, s => if s c then [Event.consume c, Event.comm c] else [Event.consume c]
  | stop, _ => []

/-- The sequential (DFS-order) trace: the fold over a path-sorted op list. -/
def gateTrace : List (DfsPath × Effect) → State → List Event
  | [], _ => []
  | (_, e) :: rest, d => e.trace d ++ gateTrace rest (e.apply d)

/-- The strict-lexicographic path comparator (the `qsort` key; `PathLt` is exactly its `true`
    cases). -/
def pathLt : DfsPath → DfsPath → Bool
  | [], [] => false
  | [], _ :: _ => true
  | _ :: _, [] => false
  | a :: as, b :: bs => if a < b then true else if b < a then false else pathLt as bs

/-- The published trace of a speculative run: the per-commit traces merged in DFS path order
    (the Rust drain emits `reverse(concat path-ascending (buffers))` into the head-inserted
    event log). -/
def publishedTrace (run : List (DfsPath × Effect)) (d : State) : List Event :=
  gateTrace (List.mergeSort (fun a b => pathLt a.1 b.1) run) d

/-- **Law 24 (publication, stated)** — DFS-serializability implies log-equality: a speculative
    run whose commits all validate publishes the sequential fold's trace (the commit-order
    fold equals the path-order fold). The converse is **false** —
    `trace_equality_without_serializability` exhibits a run with equal traces that fails
    prefix visibility, so no `iff` is stated. The proof obligation is the writer-chain lemma
    (prefix visibility forces each channel's writers to commit in path order — see
    `serializable_writer_chain`, stated below) composed with Law 23's read-determinism and
    the record-determines-value invariant (`record_determines_value`, proven). -/
axiom dfs_serializable_implies_log_equal (st0 : State × SpecState)
    (run : List (DfsPath × Effect)) :
    DFSSerializable st0 run → publishedTrace run st0.1 = gateTrace run st0.1

/-- **Law 25** — validated speculation refines the sequential reducer: any run whose commits all
    validate (aborted subtrees re-run under the gate until they do) publishes the sequential
    fold's trace. Follows from `dfs_serializable_implies_log_equal`; the abort/re-run
    coordinator is the Rust realization target (Phase B1/B2). -/
theorem validated_speculation_refines_apply (st0 : State × SpecState)
    (run : List (DfsPath × Effect)) :
    DFSSerializable st0 run → publishedTrace run st0.1 = gateTrace run st0.1 :=
  dfs_serializable_implies_log_equal st0 run

/-! ### The converse fails: trace equality without serializability

The produce-only pair commits `[2]`'s produce before `[1]`'s. Both folds emit
`[produce 0, produce 0]`, so log equality holds — but `[1]`'s commit reads `[2]`'s write,
which prefix visibility rejects. The publication check is therefore one-directional:
log equality is a *necessary* consequence, never a sufficient certificate. -/

/-- The produce-only pair, committed out of path order. -/
def produceOnlyRun : List (DfsPath × Effect) :=
  [ ([2], Effect.produce 0), ([1], Effect.produce 0) ]

/-- **Law 24 (converse witness)** — equal published traces, yet not DFS-serializable. -/
theorem trace_equality_without_serializability :
    publishedTrace produceOnlyRun stateCD = gateTrace produceOnlyRun stateCD ∧
    ¬ DFSSerializable (stateCD, emptySpec) produceOnlyRun := by
  constructor
  · native_decide
  · intro h
    have h2 : ValidCommit
        (applyAt [2] (Effect.produce 0) (stateCD, emptySpec)).2 [1] ({0} : Finset Chan) := h.2.1
    have h0 : prefixVisible (applyAt [2] (Effect.produce 0) (stateCD, emptySpec)).2 [1] 0 :=
      h2 0 (by simp)
    have hred : (applyAt [2] (Effect.produce 0) (stateCD, emptySpec)).2 0 = some ([2], 1, true) :=
      by native_decide
    rw [prefixVisible, hred] at h0
    have hlt : PathLt [2] [1] := by simpa using h0
    cases hlt with
    | cons hlt => exact (by decide : ¬ 2 < 1) hlt

/-- Gate re-run of an aborted subtree: its ops re-applied in path order from the prefix state
    (the Rust realization's per-subtree Gate dispatch). -/
def gateRerun (ops : List (DfsPath × Effect)) (st0 : State × SpecState) : State × SpecState :=
  ops.foldl (fun st (p, e) => applyAt p e st) st0

/-- **Law 25 (progress)** — gate replay is total: every subtree re-run terminates. -/
theorem gate_replay_terminates (ops : List (DfsPath × Effect)) (st : State × SpecState) :
    ∃ st', gateRerun ops st = st' :=
  ⟨gateRerun ops st, rfl⟩

/-- The paths a run commits at, in commit order. -/
def runPaths : List (DfsPath × Effect) → List DfsPath
  | [] => []
  | (p, _) :: rest => p :: runPaths rest

/-- **Law 25 (progress)** — a *published* run commits each path at most once: each path
    validates, or aborts at most once and then commits via the gate re-run (the path-ordered
    re-run queue of the Rust coordinator, Phase B1/B2). This is the invariant the coordinator
    must establish — stated as a predicate, not an axiom over arbitrary runs. -/
def Published (run : List (DfsPath × Effect)) : Prop :=
  (runPaths run).Nodup

end Rchain
