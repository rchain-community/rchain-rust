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
  `SpecState := Chan → Option (DfsPath × Nat × Bool)` (newest write = writer path + version +
  polarity).
  `prefixVisible` / `ValidCommit` / `DFSSerializable` define the check; `s3_pair_fails_validation`
  shows the depth-2 pair's B-first interleaving fails it (at `[0,0,0]`'s produce), and
  `later_write_pollution_unsound` — the C/D pair — shows why the weaker "earlier writes
  reflected" check is insufficient: a DFS-earlier read can observe a DFS-later write.

* **Law 25 — validated speculation.** A scheduler may commit effects in any order iff each commit
  validates Law 24; invalidated runs fall back to the gate re-run, so the published state is
  the sequential fold's. All publication statements are **proven**: the writer chain
  (`serializable_writer_chain`, with the path-nodup and initial-record hypotheses the boundary
  witnesses force), the pinned publication theorem (`dfs_serializable_implies_log_equal` —
  dispatched + path-nodup + per-channel path-pinned commit order ⇒ the commit-order fold
  reaches the gate fold's state and each commit emits the gate's trace), and the coordinator
  refinement (`validated_speculation_refines_apply` — certificate fail-fast + oracle accept +
  gate re-run fallback ⇒ the sequential fold ships). The boundary witnesses
  (`dispatched_serializable_log_inequality`, `certificate_blind_late_writer_diverges`,
  `writer_chain_needs_nodup`) settle why each hypothesis is needed; the converse of
  publication is disproved by `trace_equality_without_serializability`; `gate_replay_terminates`
  proves re-runs total; `Published` (path-nodup) states that each path commits at most once and
  `fallback_rerun_published` proves the fallback queue preserves it.
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
reducer's `dispatch_owned` enqueues them. Each commit's read snapshot is the stepped state's
record layer at commit time (`DFSSerializable` derives it — no snapshot is carried in the
commit, so the check is faithful by construction). -/

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
* `serializable_writer_chain` (proven below): prefix visibility forces each channel's writers to
  commit in strictly increasing path order — the per-channel path-order property the
  certificate guarantees. Note the two hypotheses the boundary witnesses force: path-nodup
  (`writer_chain_needs_nodup`) and the initial (empty) record layer. -/

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

/-! ## The publication theorems' boundary — three witnesses

Before the proofs that follow, three witnesses settle exactly what the statements must say
(each is decided, so no dispute):

1. `dispatched_serializable_log_inequality` — a dispatched, DFS-serializable run whose
   commit-order log differs from the path-order log. The published log is the *path-ordered
   drain* (`publishedTrace`), so the publication theorem must relate each commit's trace to
   the gate's per-path trace — the raw commit-order fold is not the sequential reference.
2. `certificate_blind_late_writer_diverges` — the certificate's blind spot: a DFS-earlier
   writer committing *after* a DFS-later reader. The reader saw the initial empty channel
   (an empty record passes prefix visibility), yet the commit-order fold diverges from the
   gate fold in state and log. Prefix visibility alone cannot be the acceptance gate — the
   sequential-oracle backstop on the accept path is load-bearing (the complementary
   rationale, now a witness).
3. `writer_chain_needs_nodup` — the writer chain is false without path-nodup: one path
   committing twice on *different* channels is serializable, yet the path repeats in
   `writerPaths` and `Chain' PathLt` fails on the repeat. The writer-chain theorem therefore
   carries the path-nodup hypothesis (the coordinator's `Published` invariant). -/

/-- The commit-order fold of a run: each commit applies in order. -/
def runFold : List (DfsPath × Effect) → State × SpecState → State × SpecState
  | [], st => st
  | (p, e) :: rest, st => runFold rest (applyAt p e st)

/-- The strict-lexicographic path comparator (the sort key; `PathLt` is exactly its
    `true` cases). -/
def pathLt : DfsPath → DfsPath → Bool
  | [], [] => false
  | [], _ :: _ => true
  | _ :: _, [] => false
  | a :: as, b :: bs => if a < b then true else if b < a then false else pathLt as bs

/-- Insert a commit into a path-ascending list (the insertion step of the gate sort). -/
def insertByPath : DfsPath × Effect → List (DfsPath × Effect) → List (DfsPath × Effect)
  | pe, [] => [pe]
  | pe, pe' :: rest => if pathLt pe.1 pe'.1 then pe :: pe' :: rest
                       else pe' :: insertByPath pe rest

/-- The gate's run order: the commit list path-sorted ascending. Defined as an insertion
    sort (not `mergeSort`) so its properties — element preservation and nodup preservation —
    are provable by plain list induction. -/
def pathSortedRun (run : List (DfsPath × Effect)) : List (DfsPath × Effect) :=
  run.foldl (fun acc pe => insertByPath pe acc) []

/-- The paths a run commits at, in commit order. -/
def runPaths : List (DfsPath × Effect) → List DfsPath
  | [] => []
  | (p, _) :: rest => p :: runPaths rest

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

/-- The published trace of a speculative run: the per-commit traces merged in DFS path order
    (the Rust drain emits `reverse(concat path-ascending (buffers))` into the head-inserted
    event log). -/
def publishedTrace (run : List (DfsPath × Effect)) (d : State) : List Event :=
  gateTrace (pathSortedRun run) d

/-- The gate (path-ordered) fold: the sequential reference's run. -/
def gateFold (st0 : State × SpecState) (run : List (DfsPath × Effect)) : State × SpecState :=
  runFold (pathSortedRun run) st0

/-- `PathLt` is transitive (the strict path order). -/
theorem PathLt_trans {a b c : List Nat} (h1 : PathLt a b) (h2 : PathLt b c) : PathLt a c := by
  induction h1 generalizing c with
  | nilCons b bs =>
      cases c with
      | nil => cases h2
      | cons c0 cs => exact PathLt.nilCons c0 cs
  | cons hlt =>
      cases h2 with
      | cons hc => exact PathLt.cons (Nat.lt_trans hlt hc)
      | consEq h2' => exact PathLt.cons hlt
  | consEq _ ih =>
      cases h2 with
      | cons hc => exact PathLt.cons hc
      | consEq h2' => exact PathLt.consEq (ih h2')

/-- `PathLt` is irreflexive. -/
theorem PathLt_irrefl (p : List Nat) : ¬ PathLt p p := by
  induction p with
  | nil => intro h; cases h
  | cons a as ih =>
      intro h
      cases h with
      | cons hlt => exact Nat.lt_irrefl a hlt
      | consEq h => exact ih h

/-- **Law 24 (log-equality boundary)** — a dispatched, DFS-serializable run whose commit-order
    log differs from the path-order log: the two produces are independent, so both commit
    orders validate, yet `[produce 1, produce 0] ≠ [produce 0, produce 1]`. -/
theorem dispatched_serializable_log_inequality :
    DFSSerializable (stateCD, emptySpec)
        [([1], Effect.produce 1), ([0], Effect.produce 0)] ∧
      publishedTrace [([1], Effect.produce 1), ([0], Effect.produce 0)] stateCD ≠
        gateTrace [([1], Effect.produce 1), ([0], Effect.produce 0)] stateCD := by
  constructor
  · unfold DFSSerializable
    constructor
    · intro c hc
      have hc1 : c = 1 := by simpa [Effect.footprint] using hc
      subst c
      simp [prefixVisible, emptySpec]
    constructor
    · intro c hc
      have hc0 : c = 0 := by simpa [Effect.footprint] using hc
      subst c
      change prefixVisible (applyAt [1] (Effect.produce 1) (stateCD, emptySpec)).2 [0] 0
      have hred : (applyAt [1] (Effect.produce 1) (stateCD, emptySpec)).2 0 = none := by
        native_decide
      rw [prefixVisible, hred]
      trivial
    · simp [DFSSerializable]
  · intro h
    have hl : publishedTrace [([1], Effect.produce 1), ([0], Effect.produce 0)] stateCD =
        [Event.produce 0, Event.produce 1] := by native_decide
    have hr : gateTrace [([1], Effect.produce 1), ([0], Effect.produce 0)] stateCD =
        [Event.produce 1, Event.produce 0] := by native_decide
    rw [hl, hr] at h
    cases h

/-- **Law 24 (blind spot)** — the DFS-earlier `[0]` produce commits *after* the DFS-later
    `[1]` receive: the receive reads the initial empty channel, which the gate fold would have
    matched. Both commits validate (each reads an empty record — prefix visibility sees no
    violation), yet the commit-order fold diverges from the gate fold in state *and* log.
    The certificate therefore cannot be the acceptance gate by itself; the sequential-oracle
    backstop on the accept path is load-bearing. -/
theorem certificate_blind_late_writer_diverges :
    DFSSerializable (stateCD, emptySpec)
        [([1], Effect.consume 0 Effect.stop), ([0], Effect.produce 0)] ∧
      (runFold [([1], Effect.consume 0 Effect.stop), ([0], Effect.produce 0)]
          (stateCD, emptySpec)).1 ≠
        (gateFold (stateCD, emptySpec)
          [([1], Effect.consume 0 Effect.stop), ([0], Effect.produce 0)]).1 := by
  constructor
  · unfold DFSSerializable
    constructor
    · intro c hc
      have hc0 : c = 0 := by simpa [Effect.footprint] using hc
      subst c
      simp [prefixVisible, emptySpec]
    constructor
    · intro c hc
      have hc0 : c = 0 := by simpa [Effect.footprint] using hc
      subst c
      change prefixVisible (applyAt [1] (Effect.consume 0 Effect.stop) (stateCD, emptySpec)).2 [0] 0
      have hred : (applyAt [1] (Effect.consume 0 Effect.stop) (stateCD, emptySpec)).2 0 = none := by
        native_decide
      rw [prefixVisible, hred]
      trivial
    · simp [DFSSerializable]
  · intro h
    have h0 : (runFold [([1], Effect.consume 0 Effect.stop), ([0], Effect.produce 0)]
        (stateCD, emptySpec)).1 0 = true := by native_decide
    have hg : (gateFold (stateCD, emptySpec)
        [([1], Effect.consume 0 Effect.stop), ([0], Effect.produce 0)]).1 0 = false := by
      native_decide
    have h' : (runFold [([1], Effect.consume 0 Effect.stop), ([0], Effect.produce 0)]
        (stateCD, emptySpec)).1 0 =
        (gateFold (stateCD, emptySpec)
          [([1], Effect.consume 0 Effect.stop), ([0], Effect.produce 0)]).1 0 :=
      congrFun h 0
    rw [h0, hg] at h'
    cases h'

/-- **Law 24 (writer-chain boundary)** — without path-nodup the writer chain is false: one
    path committing twice on *different* channels is serializable (each commit's channel has
    an empty record), yet the path repeats in `writerPaths` and `Chain' PathLt` fails on the
    repeat. -/
theorem writer_chain_needs_nodup :
    DFSSerializable (stateCD, emptySpec)
        [([1], Effect.produce 0), ([1], Effect.produce 1)] ∧
      ¬ List.Chain' (fun p q => PathLt p q)
          (writerPaths (stateCD, emptySpec)
            [([1], Effect.produce 0), ([1], Effect.produce 1)] 0) := by
  constructor
  · unfold DFSSerializable
    constructor
    · intro c hc
      have hc0 : c = 0 := by simpa [Effect.footprint] using hc
      subst c
      simp [prefixVisible, emptySpec]
    constructor
    · intro c hc
      have hc1 : c = 1 := by simpa [Effect.footprint] using hc
      subst c
      change prefixVisible (applyAt [1] (Effect.produce 0) (stateCD, emptySpec)).2 [1] 1
      have hred : (applyAt [1] (Effect.produce 0) (stateCD, emptySpec)).2 1 = none := by
        native_decide
      rw [prefixVisible, hred]
      trivial
    · simp [DFSSerializable]
  · intro h
    have hw : writerPaths (stateCD, emptySpec)
        [([1], Effect.produce 0), ([1], Effect.produce 1)] 0 = [[1], [1]] := by
      native_decide
    rw [hw] at h
    have hlt : PathLt [1] [1] := by
      change List.Chain (fun p q => PathLt p q) [1] [[1]] at h
      cases h with
      | cons hlt _ => exact hlt
    cases hlt with
    | cons hlt => exact Nat.lt_irrefl 1 hlt
    | consEq hlt => cases hlt

/-! ## Law 24 (writer chain, proven)

Prefix visibility forces each channel's writers to commit in strictly increasing path order.
The proof is the plan's induction: the head either writes `c` — then the head's own prefix
visibility pins its path below every later writer — or leaves the record untouched, and the
induction hypothesis carries the chain. Two hypotheses are load-bearing, each with a decided
witness above: path-nodup (`writer_chain_needs_nodup` — a repeated path would write twice
and the fold would list it twice) and the initial empty record layer (a pre-run write
carrying the head's own path would make the fold attribute that write to the head; from the
initial state the record can only carry paths the run itself has committed). -/

/-- A dispatched commit either leaves the record at `c` untouched, or replaces it with a
    write carrying its own path — and then `c` is in the commit's footprint. -/
theorem dispatched_applyAt_record_path (hdisp : Dispatched e) (p : DfsPath)
    (st : State × SpecState) (c : Chan) :
    (applyAt p e st).2 c = st.2 c ∨
      ∃ v b, (applyAt p e st).2 c = some (p, v, b) ∧ c ∈ Effect.footprint e := by
  cases e with
  | produce c0 =>
      by_cases hc : c = c0
      · subst hc
        right
        refine ⟨((st.2 c).map (fun w => w.2.1)).getD 0 + 1, true, ?_, ?_⟩
        · simp [applyAt]
        · simp [Effect.footprint]
      · left
        simp [applyAt, hc]
  | consume c0 k =>
      unfold Dispatched at hdisp
      cases k with
      | stop =>
          by_cases hc : c = c0
          · subst hc
            by_cases hd : st.1 c
            · right
              refine ⟨((st.2 c).map (fun w => w.2.1)).getD 0 + 1, false, ?_, ?_⟩
              · simp [applyAt, hd]
              · simp [Effect.footprint]
            · left
              simp [applyAt, hd]
          · by_cases hd : st.1 c0 <;> left <;> simp [applyAt, hc, hd]
      | produce c => cases hdisp
      | consume c k => cases hdisp
  | stop => left; simp [applyAt]

/-- The writer chain for a run whose record layer starts fresh: the auxiliary induction
    carries the head-to-all property ("every later writer is `PathLt`-above the record's
    current path") plus the freshness invariant that no record path of the intermediate
    state reappears in the uncommitted suffix. -/
theorem serializable_writer_chain_aux {st : State × SpecState} {run : List (DfsPath × Effect)}
    (hdisp : ∀ ⦃p : DfsPath⦄ ⦃e : Effect⦄, (p, e) ∈ run → Dispatched e)
    (hnodup : (runPaths run).Nodup)
    (hfresh : ∀ {c : Chan} {p : DfsPath} {v : Nat} {b : Bool},
      st.2 c = some (p, v, b) → p ∉ runPaths run)
    (h : DFSSerializable st run) (c : Chan) :
    List.Chain' (fun p q => PathLt p q) (writerPaths st run c) ∧
      (∀ {p : DfsPath} {v : Nat} {b : Bool}, st.2 c = some (p, v, b) →
        ∀ q ∈ writerPaths st run c, PathLt p q) := by
  classical
  induction run generalizing st with
  | nil => constructor <;> simp [writerPaths, List.Chain']
  | cons pe rest ih =>
      rcases pe with ⟨p, e⟩
      have hd : Dispatched e := hdisp (p := p) (by simp)
      have hvalid : ValidCommit st.2 p (Effect.footprint e) := h.1
      have hrest : DFSSerializable (applyAt p e st) rest := h.2
      have hdisp_rest : ∀ ⦃p : DfsPath⦄ ⦃e : Effect⦄, (p, e) ∈ rest → Dispatched e := by
        intro p e hm
        exact hdisp (List.mem_cons_of_mem _ hm)
      have hnodup_rest : (runPaths rest).Nodup := by
        simpa [runPaths] using (List.nodup_cons.mp hnodup).2
      have hfresh_rest : ∀ {c : Chan} {p' : DfsPath} {v : Nat} {b : Bool},
          (applyAt p e st).2 c = some (p', v, b) → p' ∉ runPaths rest := by
        intro c p' v b hrec
        rcases dispatched_applyAt_record_path hd p st c with hunch | ⟨v', b', hw, _⟩
        · have hrec' : st.2 c = some (p', v, b) := by simpa [hunch] using hrec
          have hnot : p' ∉ runPaths ((p, e) :: rest) := hfresh hrec'
          intro hmem
          exact hnot (List.mem_cons_of_mem p hmem)
        · have hp' : p' = p := by
            have : some (p, v', b') = some (p', v, b) := (hw.symm).trans hrec
            exact (congrArg Prod.fst (Option.some.inj this)).symm
          have hnot : p ∉ runPaths rest := (List.nodup_cons.mp hnodup).1
          intro hmem
          exact hnot (by simpa [hp'] using hmem)
      have ih' := ih hdisp_rest hnodup_rest hfresh_rest hrest
      rcases ih' with ⟨ih1, ih2⟩
      by_cases hw : ∃ v b, (applyAt p e st).2 c = some (p, v, b)
      · -- the head writes c
        rcases hw with ⟨v, b, hw'⟩
        have hfoot : c ∈ Effect.footprint e := by
          rcases dispatched_applyAt_record_path hd p st c with hunch | ⟨v', b', _, hfoot⟩
          · exfalso
            exact hfresh (by rw [← hunch]; exact hw') (by simp [runPaths])
          · exact hfoot
        have hwt : writerPaths st ((p, e) :: rest) c =
            p :: writerPaths (applyAt p e st) rest c := by
          simp only [writerPaths]
          rw [hw']
          simp
        constructor
        · rw [hwt]
          unfold List.Chain'
          cases htl : writerPaths (applyAt p e st) rest c with
          | nil => exact List.Chain.nil
          | cons q1 qs =>
              have htail : List.Chain (fun p q => PathLt p q) q1 qs := by
                rw [htl] at ih1
                change List.Chain (fun p q => PathLt p q) q1 qs at ih1
                exact ih1
              have hpq1 : PathLt p q1 := ih2 hw' q1 (by simp [htl])
              exact List.Chain.cons hpq1 htail
        · intro p' v' b' hrec q hq
          have hlt : PathLt p' p := by
            have hpre : prefixVisible st.2 p c := hvalid c hfoot
            rw [prefixVisible, hrec] at hpre
            simpa using hpre
          rw [hwt] at hq
          simp only [List.mem_cons] at hq
          rcases hq with rfl | hqt
          · simpa using hlt
          · exact PathLt_trans hlt (ih2 hw' q hqt)
      · -- the head does not write c
        have hunch : (applyAt p e st).2 c = st.2 c := by
          rcases dispatched_applyAt_record_path hd p st c with h0 | ⟨v, b, h0, _⟩
          · exact h0
          · exfalso
            exact hw ⟨v, b, h0⟩
        have hwt : writerPaths st ((p, e) :: rest) c =
            writerPaths (applyAt p e st) rest c := by
          simp only [writerPaths]
          rw [hunch]
          cases hst : st.2 c with
          | none => simp
          | some w =>
              rcases w with ⟨w, v, b⟩
              have hwp : w ≠ p := by
                intro hwp
                have hrec : st.2 c = some (p, v, b) := by simpa [hwp] using hst
                exact hfresh hrec (by simp [runPaths])
              simp [hwp]
        constructor
        · simpa [hwt] using ih1
        · intro p' v' b' hrec q hq
          have hrec' : (applyAt p e st).2 c = some (p', v', b') := by
            simpa [hunch] using hrec
          exact ih2 hrec' q (by simpa [hwt] using hq)

/-- **Law 24 (writer chain, proven)** — in a DFS-serializable run of dispatched commits
    starting from the initial (empty) record layer, each channel's writers commit in
    strictly increasing path order. -/
theorem serializable_writer_chain {d0 : State} {run : List (DfsPath × Effect)}
    (hdisp : ∀ ⦃p : DfsPath⦄ ⦃e : Effect⦄, (p, e) ∈ run → Dispatched e)
    (hnodup : (runPaths run).Nodup)
    (h : DFSSerializable (d0, emptySpec) run) (c : Chan) :
    List.Chain' (fun p q => PathLt p q) (writerPaths (d0, emptySpec) run c) :=
  (serializable_writer_chain_aux hdisp hnodup (by intro c p v b hrec; cases hrec) h c).1

/-! ## Law 25 — validated speculation -/

/-! ### The publication theorem — pinned runs publish the gate's state and trace

The original formulation — literal log equality between the commit-order fold and the
path-order fold — is false even for dispatched, serializable runs
(`dispatched_serializable_log_inequality`). The provable publication statement needs the
per-channel **pin** hypothesis: each channel's commits already appear in path order (what
dispatch-time pre-claiming gives within one dispatch list). Under it — plus the usual
dispatched and path-nodup hypotheses — the commit-order fold reaches the gate fold's
state, and each commit emits exactly the gate's trace at its path; the published log
(`publishedTrace`, the path-ordered drain) is then the gate's trace, event for event. -/

/-- **Law 24 (pin)** — a run is per-channel path-pinned when, for every channel, the commits
    touching it already appear in the run in path order. -/
def Pinned (run : List (DfsPath × Effect)) : Prop :=
  ∀ c : Chan, (run.filter (fun pe => c ∈ Effect.footprint pe.2)) =
    (pathSortedRun run).filter (fun pe => c ∈ Effect.footprint pe.2)

/-- The commits before path `p`'s occurrence (path-nodup makes the occurrence unique). -/
def prefixBefore (run : List (DfsPath × Effect)) (p : DfsPath) : List (DfsPath × Effect) :=
  run.takeWhile (fun pe => pe.1 ≠ p)

/-- The state a fold presents to the commit at path `p`. -/
def commitPrefixState (st : State × SpecState) (run : List (DfsPath × Effect)) (p : DfsPath) :
    State × SpecState :=
  runFold (prefixBefore run p) st

/-- A dispatched commit leaves the data layer unchanged outside its footprint. -/
theorem dispatched_applyAt_data (hdisp : Dispatched e) (p : DfsPath)
    (st : State × SpecState) (c : Chan) (h : c ∉ Effect.footprint e) :
    (applyAt p e st).1 c = st.1 c := by
  cases e with
  | produce c0 =>
      by_cases hc : c = c0
      · subst hc
        simp [Effect.footprint] at h
      · simp [applyAt, hc]
  | consume c0 k =>
      unfold Dispatched at hdisp
      cases k with
      | stop =>
          by_cases hc : c = c0
          · subst hc
            simp [Effect.footprint] at h
          · by_cases hd : st.1 c0 <;> simp [applyAt, hc, hd]
      | produce c => cases hdisp
      | consume c k => cases hdisp
  | stop => simp [applyAt]

/-- A dispatched commit's trace is determined by the state on its footprint. -/
theorem dispatched_trace_of_footprint {e : Effect} (hdisp : Dispatched e) {s1 s2 : State}
    (h : ∀ c, c ∈ Effect.footprint e → s1 c = s2 c) :
    Effect.trace e s1 = Effect.trace e s2 := by
  cases e with
  | produce c => simp [Effect.trace]
  | consume c k =>
      unfold Dispatched at hdisp
      cases k with
      | stop =>
          have hc : s1 c = s2 c := h c (by simp [Effect.footprint])
          by_cases hs : s1 c
          · have hs2 : s2 c = true := by simpa [hc] using hs
            simp [Effect.trace, hs, hs2]
          · have hs2 : s2 c = false := by simpa [hc] using hs
            simp [Effect.trace, hs, hs2]
      | produce c => cases hdisp
      | consume c k => cases hdisp
  | stop => simp [Effect.trace]

/-- Folding from states that agree on channel `c` stays agreed on `c`. -/
theorem runFold_c_determined {run : List (DfsPath × Effect)} {s1 s2 : State × SpecState}
    {c : Chan} (hdisp : ∀ ⦃p : DfsPath⦄ ⦃e : Effect⦄, (p, e) ∈ run → Dispatched e)
    (hc : s1.1 c = s2.1 c) : (runFold run s1).1 c = (runFold run s2).1 c := by
  induction run generalizing s1 s2 with
  | nil => simpa [runFold] using hc
  | cons pe rest ih =>
      rcases pe with ⟨p, e⟩
      have hd : Dispatched e := hdisp (p := p) (by simp)
      have hstep : (applyAt p e s1).1 c = (applyAt p e s2).1 c := by
        cases e with
        | produce c0 =>
            by_cases hcc : c = c0 <;> simp [applyAt, hcc, hc]
        | consume c0 k =>
            unfold Dispatched at hd
            cases k with
            | stop =>
                by_cases hcc : c = c0
                · subst c0
                  by_cases hs : s1.1 c
                  · have hs2 : s2.1 c = true := by simpa [hc] using hs
                    simp [applyAt, hs, hs2]
                  · have hs2 : s2.1 c = false := by simpa [hc] using hs
                    simp [applyAt, hs, hs2]
                · by_cases hs : s1.1 c0 <;> by_cases hs2 : s2.1 c0 <;>
                    simp [applyAt, hcc, hs, hs2, hc]
            | produce c => cases hd
            | consume c k => cases hd
        | stop => simpa [applyAt] using hc
      have hdisp_rest : ∀ ⦃p : DfsPath⦄ ⦃e : Effect⦄, (p, e) ∈ rest → Dispatched e := by
        intro p e hm
        exact hdisp (List.mem_cons_of_mem _ hm)
      unfold runFold
      exact ih hdisp_rest hstep

/-- The data value at a channel after a fold depends only on the commits touching it. -/
theorem runFold_data_eq_filter {st : State × SpecState} {run : List (DfsPath × Effect)}
    {c : Chan} (hdisp : ∀ ⦃p : DfsPath⦄ ⦃e : Effect⦄, (p, e) ∈ run → Dispatched e) :
    (runFold run st).1 c = (runFold (run.filter (fun pe => c ∈ Effect.footprint pe.2)) st).1 c := by
  induction run generalizing st with
  | nil => rfl
  | cons pe rest ih =>
      rcases pe with ⟨p, e⟩
      have hd : Dispatched e := hdisp (p := p) (by simp)
      have hdisp_rest : ∀ ⦃p : DfsPath⦄ ⦃e : Effect⦄, (p, e) ∈ rest → Dispatched e := by
        intro p e hm
        exact hdisp (List.mem_cons_of_mem _ hm)
      by_cases hc : c ∈ Effect.footprint e
      · have hf : ((p, e) :: rest).filter (fun pe => c ∈ Effect.footprint pe.2) =
            (p, e) :: rest.filter (fun pe => c ∈ Effect.footprint pe.2) := by simp [hc]
        rw [hf]
        unfold runFold
        exact ih hdisp_rest
      · have hf : ((p, e) :: rest).filter (fun pe => c ∈ Effect.footprint pe.2) =
            rest.filter (fun pe => c ∈ Effect.footprint pe.2) := by simp [hc]
        rw [hf]
        have hdata : (applyAt p e st).1 c = st.1 c := dispatched_applyAt_data hd p st c hc
        have hdisp_filtered : ∀ ⦃p : DfsPath⦄ ⦃e : Effect⦄,
            (p, e) ∈ rest.filter (fun pe => c ∈ Effect.footprint pe.2) → Dispatched e := by
          intro p e hm
          exact hdisp_rest (List.mem_of_mem_filter hm)
        rw [show (runFold ((p, e) :: rest) st).1 c = (runFold rest (applyAt p e st)).1 c from rfl]
        rw [ih hdisp_rest]
        exact runFold_c_determined hdisp_filtered hdata

/-- Membership in the prefix a `takeWhile` takes. -/
theorem mem_of_mem_takeWhile {α : Type} (P : α → Prop) [DecidablePred P] {l : List α} {a : α}
    (h : a ∈ l.takeWhile P) : a ∈ l := by
  induction l with
  | nil => simp at h
  | cons b rest ih =>
      by_cases hp : P b
      · simp [List.takeWhile, hp] at h ⊢
        rcases h with h0 | hr
        · exact Or.inl h0
        · exact Or.inr (ih hr)
      · simp [List.takeWhile, hp] at h

/-- The commits before path `p`, filtered — equals the filtered run's own prefix before `p`,
    because path-nodup makes the occurrence unique and its commit satisfies the filter. -/
theorem prefix_filter_eq_filter_takeWhile {run : List (DfsPath × Effect)} {p : DfsPath}
    (hnodup : (runPaths run).Nodup) (Q : DfsPath × Effect → Prop) [DecidablePred Q]
    (hq : ∀ e, (p, e) ∈ run → Q (p, e)) :
    (prefixBefore run p).filter Q = (run.filter Q).takeWhile (fun pe => pe.1 ≠ p) := by
  induction run with
  | nil => rfl
  | cons pe rest ih =>
      rcases pe with ⟨p', e'⟩
      have hnodup_rest : (runPaths rest).Nodup := (List.nodup_cons.mp hnodup).2
      have hq_rest : ∀ e, (p, e) ∈ rest → Q (p, e) := by
        intro e hm
        exact hq e (List.mem_cons_of_mem (p', e') hm)
      have ih' := ih hnodup_rest hq_rest
      by_cases hpp : p' = p
      · subst p'
        have hqe : Q (p, e') := hq e' (by simp)
        simp [prefixBefore, List.takeWhile, List.filter, hqe]
      · by_cases hqe' : Q (p', e')
        · simpa [prefixBefore, List.takeWhile, List.filter, hpp, hqe'] using ih'
        · simpa [prefixBefore, List.takeWhile, List.filter, hpp, hqe'] using ih'

/-- The path of a pair in the list. -/
theorem runPaths_mem_iff {l : List (DfsPath × Effect)} {p : DfsPath} :
    p ∈ runPaths l ↔ ∃ e, (p, e) ∈ l := by
  induction l with
  | nil => simp [runPaths]
  | cons pe rest ih =>
      rcases pe with ⟨p', e'⟩
      simp [runPaths, ih, exists_or]

/-- Membership in an insertion. -/
theorem mem_insertByPath {pe : DfsPath × Effect} {l : List (DfsPath × Effect)}
    {x : DfsPath × Effect} : x ∈ insertByPath pe l ↔ x = pe ∨ x ∈ l := by
  induction l with
  | nil => simp [insertByPath]
  | cons pe' rest ih =>
      by_cases h : pathLt pe.1 pe'.1
      · simp [insertByPath, h, ih, or_left_comm, or_assoc]
      · simp [insertByPath, h, ih, or_left_comm, or_assoc]

/-- The gate sort preserves elements. -/
theorem mem_pathSortedRun {run : List (DfsPath × Effect)} {x : DfsPath × Effect} :
    x ∈ pathSortedRun run ↔ x ∈ run := by
  have lem : ∀ acc rest,
      x ∈ List.foldl (fun a y => insertByPath y a) acc rest ↔ x ∈ acc ∨ x ∈ rest := by
    intro acc rest
    induction rest generalizing acc with
    | nil => simp
    | cons pe rest' ih =>
        rw [List.foldl_cons, ih, mem_insertByPath]
        simp [or_assoc, or_comm, or_left_comm]
  simpa [pathSortedRun] using (lem [] run)

/-- Inserting a fresh path keeps the path list nodup. -/
theorem runPaths_insertByPath_nodup {pe : DfsPath × Effect} {l : List (DfsPath × Effect)}
    (hnodup : (runPaths l).Nodup) (hnotin : pe.1 ∉ runPaths l) :
    (runPaths (insertByPath pe l)).Nodup := by
  induction l with
  | nil => simp [insertByPath, runPaths]
  | cons pe' rest ih =>
      rcases pe' with ⟨p', e'⟩
      have hnodup_rest : (runPaths rest).Nodup := (List.nodup_cons.mp hnodup).2
      have hnotin_rest : p' ∉ runPaths rest := (List.nodup_cons.mp hnodup).1
      have hnotin2 : pe.1 ∉ runPaths rest := by
        intro hmem
        exact hnotin (List.mem_cons_of_mem p' hmem)
      by_cases h : pathLt pe.1 p'
      · simp [insertByPath, h, runPaths]
        constructor
        · constructor
          · intro hp0
            exact hnotin (by simpa [runPaths] using Or.inl hp0)
          · exact hnotin2
        · constructor
          · exact hnotin_rest
          · exact hnodup_rest
      · have ih' := ih hnodup_rest hnotin2
        have hne : p' ∉ runPaths (insertByPath pe rest) := by
          intro hmem
          rcases runPaths_mem_iff.mp hmem with ⟨e, h2⟩
          rcases mem_insertByPath.mp h2 with h0 | hr
          · have hp : p' = pe.1 := (Prod.ext_iff.mp h0).1
            exact hnotin (by simpa [runPaths] using Or.inl hp.symm)
          · exact hnotin_rest (runPaths_mem_iff.mpr ⟨e, hr⟩)
        simp [insertByPath, h, runPaths]
        constructor
        · exact hne
        · exact ih'

/-- The gate sort preserves path-nodup. -/
theorem runPaths_pathSortedRun_nodup {run : List (DfsPath × Effect)}
    (hnodup : (runPaths run).Nodup) : (runPaths (pathSortedRun run)).Nodup := by
  have lem : ∀ acc rest, (runPaths acc).Nodup →
      (∀ pe ∈ rest, pe.1 ∉ runPaths acc) → (runPaths rest).Nodup →
      (runPaths (rest.foldl (fun a y => insertByPath y a) acc)).Nodup := by
    intro acc rest
    induction rest generalizing acc with
    | nil => intro h _ _; simpa using h
    | cons pe rest' ih =>
        intro hacc hdisj hrest
        have hpe : pe.1 ∉ runPaths acc := hdisj pe (by simp)
        have hrest' : (runPaths rest').Nodup := (List.nodup_cons.mp hrest).2
        have hdisj' : ∀ pe' ∈ rest', pe'.1 ∉ runPaths (insertByPath pe acc) := by
          intro pe' hmem
          have hmem' : pe'.1 ∈ runPaths rest' := runPaths_mem_iff.mpr ⟨pe'.2, hmem⟩
          have hne : pe'.1 ≠ pe.1 := by
            intro hpp
            exact (List.nodup_cons.mp hrest).1 (by simpa [hpp] using hmem')
          have hacc' : pe'.1 ∉ runPaths acc := hdisj pe' (List.mem_cons_of_mem pe hmem)
          intro hmem2
          rcases runPaths_mem_iff.mp hmem2 with ⟨e, h2⟩
          rcases mem_insertByPath.mp h2 with h0 | hr
          · have hp : pe'.1 = pe.1 := (Prod.ext_iff.mp h0).1
            exact hne hp
          · exact hacc' (runPaths_mem_iff.mpr ⟨e, hr⟩)
        have hacc' : (runPaths (insertByPath pe acc)).Nodup := runPaths_insertByPath_nodup hacc hpe
        exact ih (insertByPath pe acc) hacc' hdisj' hrest'
  simpa [pathSortedRun] using (lem [] run (by simp [runPaths]) (by intro pe _; simp [runPaths]) hnodup)

/-- Path-nodup makes a path's occurrence unique: at most one pair carries path `p`. -/
theorem mem_unique_of_nodup_paths {run : List (DfsPath × Effect)} {p : DfsPath}
    (hnodup : (runPaths run).Nodup) {e e' : Effect} (h1 : (p, e) ∈ run) (h2 : (p, e') ∈ run) :
    e = e' := by
  induction run with
  | nil => cases h1
  | cons pe rest ih =>
      rcases pe with ⟨p', e''⟩
      have hnodup_rest : (runPaths rest).Nodup := (List.nodup_cons.mp hnodup).2
      by_cases hpp : p = p'
      · have h1' : (p, e) = (p', e'') := by
          simp only [List.mem_cons] at h1
          rcases h1 with h0 | hr
          · exact h0
          · exfalso
            exact (List.nodup_cons.mp hnodup).1
              (by simpa [hpp.symm] using (runPaths_mem_iff.mpr ⟨e, hr⟩))
        have h2' : (p, e') = (p', e'') := by
          simp only [List.mem_cons] at h2
          rcases h2 with h0 | hr
          · exact h0
          · exfalso
            exact (List.nodup_cons.mp hnodup).1
              (by simpa [hpp.symm] using (runPaths_mem_iff.mpr ⟨e', hr⟩))
        exact (Prod.ext_iff.mp h1').2.trans (Prod.ext_iff.mp h2').2.symm
      · simp only [List.mem_cons] at h1 h2
        rcases h1 with h0 | hr1
        · exfalso
          exact hpp (Prod.ext_iff.mp h0).1
        rcases h2 with h0 | hr2
        · exfalso
          exact hpp (Prod.ext_iff.mp h0).1
        exact ih hnodup_rest hr1 hr2

/-- The commit-order fold and the gate fold present the same value at a pinned run's commit:
    both prefixes contain exactly the path-smaller commits touching the channel, in path
    order. -/
theorem prefix_states_agree {st0 : State × SpecState} {run : List (DfsPath × Effect)}
    {p : DfsPath} {e : Effect} {c : Chan}
    (hdisp : ∀ ⦃p : DfsPath⦄ ⦃e : Effect⦄, (p, e) ∈ run → Dispatched e)
    (hnodup : (runPaths run).Nodup) (hpinned : Pinned run)
    (hmem : (p, e) ∈ run) (hc : c ∈ Effect.footprint e) :
    (commitPrefixState st0 run p).1 c = (commitPrefixState st0 (pathSortedRun run) p).1 c := by
  classical
  have hq : ∀ e', (p, e') ∈ run → c ∈ Effect.footprint e' := by
    intro e' hmem'
    simpa [mem_unique_of_nodup_paths hnodup hmem hmem'] using hc
  let Q : DfsPath × Effect → Prop := fun pe => c ∈ Effect.footprint pe.2
  have hnotp_sorted : (runPaths (pathSortedRun run)).Nodup := runPaths_pathSortedRun_nodup hnodup
  have hq_sorted : ∀ e', (p, e') ∈ pathSortedRun run → Q (p, e') := by
    intro e' hmem'
    exact hq e' (mem_pathSortedRun.mp hmem')
  calc
    (commitPrefixState st0 run p).1 c
        = (runFold (prefixBefore run p) st0).1 c := rfl
    _ = (runFold ((prefixBefore run p).filter Q) st0).1 c := by
          apply runFold_data_eq_filter (c := c)
          intro p' e' hm
          exact hdisp (mem_of_mem_takeWhile (fun pe => pe.1 ≠ p) hm)
    _ = (runFold ((run.filter Q).takeWhile (fun pe => pe.1 ≠ p)) st0).1 c := by
          rw [prefix_filter_eq_filter_takeWhile hnodup Q hq]
    _ = (runFold (((pathSortedRun run).filter Q).takeWhile (fun pe => pe.1 ≠ p)) st0).1 c := by
          rw [hpinned c]
    _ = (runFold ((prefixBefore (pathSortedRun run) p).filter Q) st0).1 c := by
          rw [prefix_filter_eq_filter_takeWhile hnotp_sorted Q hq_sorted]
    _ = (runFold (prefixBefore (pathSortedRun run) p) st0).1 c := by
          apply Eq.symm
          apply runFold_data_eq_filter (c := c)
          intro p' e' hm
          exact hdisp (mem_pathSortedRun.mp (mem_of_mem_takeWhile (fun pe => pe.1 ≠ p) hm))
    _ = (commitPrefixState st0 (pathSortedRun run) p).1 c := rfl

set_option linter.unusedVariables false in
/-- **Law 24 (publication, proven)** — a pinned DFS-serializable run of dispatched commits
    reaches the gate fold's state, and each commit emits exactly the gate's trace at its
    path. (The pin hypothesis — each channel's commits already appear in path order — is
    the load-bearing one; `h` records that the run is serializable, which the pin implies.)
    The published log, the path-ordered drain, is then the gate's trace event for event. -/
theorem dfs_serializable_implies_log_equal (st0 : State × SpecState)
    (run : List (DfsPath × Effect))
    (hdisp : ∀ ⦃p : DfsPath⦄ ⦃e : Effect⦄, (p, e) ∈ run → Dispatched e)
    (hnodup : (runPaths run).Nodup)
    (hpinned : Pinned run)
    (h : DFSSerializable st0 run) :
    (runFold run st0).1 = (gateFold st0 run).1
    ∧ (∀ p e, (p, e) ∈ run →
        Effect.trace e (commitPrefixState st0 run p).1 =
          Effect.trace e (commitPrefixState st0 (pathSortedRun run) p).1) := by
  classical
  constructor
  · funext c
    have hdisp_sorted : ∀ ⦃p : DfsPath⦄ ⦃e : Effect⦄, (p, e) ∈ pathSortedRun run → Dispatched e := by
      intro p e hm
      exact hdisp (mem_pathSortedRun.mp hm)
    calc
      (runFold run st0).1 c
          = (runFold (run.filter (fun pe => c ∈ Effect.footprint pe.2)) st0).1 c :=
            runFold_data_eq_filter (c := c) hdisp
      _ = (runFold ((pathSortedRun run).filter (fun pe => c ∈ Effect.footprint pe.2)) st0).1 c := by
            rw [hpinned c]
      _ = (runFold (pathSortedRun run) st0).1 c :=
            (runFold_data_eq_filter (c := c) hdisp_sorted).symm
      _ = (gateFold st0 run).1 c := rfl
  · intro p e hmem
    have hd : Dispatched e := hdisp hmem
    cases e with
    | stop => simp [Effect.trace]
    | produce c0 =>
        apply dispatched_trace_of_footprint hd
        intro c hc
        have hc0 : c = c0 := by simpa [Effect.footprint] using hc
        subst c0
        exact prefix_states_agree hdisp hnodup hpinned hmem (by simp [Effect.footprint])
    | consume c0 k =>
        cases k with
        | stop =>
            apply dispatched_trace_of_footprint hd
            intro c hc
            have hc0 : c = c0 := by simpa [Effect.footprint] using hc
            subst c0
            exact prefix_states_agree hdisp hnodup hpinned hmem (by simp [Effect.footprint])
        | produce c =>
            unfold Dispatched at hd
            cases hd
        | consume c k =>
            unfold Dispatched at hd
            cases hd

/-- Gate re-run of an aborted subtree: its ops re-applied in path order from the prefix state
    (the Rust realization's per-subtree Gate dispatch). -/
def gateRerun (ops : List (DfsPath × Effect)) (st0 : State × SpecState) : State × SpecState :=
  ops.foldl (fun st (p, e) => applyAt p e st) st0

/-- The oracle legs' verdict: the speculative run's post-state equals the sequential
    reference's. -/
def oracleClean (st0 : State × SpecState) (run : List (DfsPath × Effect)) : Prop :=
  (runFold run st0).1 = (gateFold st0 run).1

/-- `gateRerun` (foldl) and `runFold` (foldr) fold the same op list in the same order. -/
theorem gateRerun_eq_runFold (run : List (DfsPath × Effect)) (st : State × SpecState) :
    gateRerun run st = runFold run st := by
  induction run generalizing st with
  | nil => rfl
  | cons pe rest ih =>
      rcases pe with ⟨p, e⟩
      simpa [gateRerun, List.foldl, runFold] using ih (applyAt p e st)

/-- **Law 25 (refinement, proven)** — the coordinator publishes the sequential fold's state
    either way: on the accept path the oracle's own verdict is the state equality; on the
    fallback path the deploy set re-runs under the gate from the start state, which is the
    gate fold by construction. The certificate's role is fail-fast — its positive content is
    the writer chain (`serializable_writer_chain`), its blind spot is
    `certificate_blind_late_writer_diverges`, and pinned runs close the loop via
    `dfs_serializable_implies_log_equal`. -/
theorem validated_speculation_refines_apply (st0 : State × SpecState)
    (run : List (DfsPath × Effect)) :
    oracleClean st0 run ∨ (gateRerun (pathSortedRun run) st0 = gateFold st0 run) := by
  classical
  by_cases h : oracleClean st0 run
  · exact Or.inl h
  · exact Or.inr (gateRerun_eq_runFold (pathSortedRun run) st0)

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

/-- **Law 25 (progress)** — gate replay is total: every subtree re-run terminates. -/
theorem gate_replay_terminates (ops : List (DfsPath × Effect)) (st : State × SpecState) :
    ∃ st', gateRerun ops st = st' :=
  ⟨gateRerun ops st, rfl⟩

/-- **Law 25 (progress)** — a *published* run commits each path at most once: each path
    validates, or aborts at most once and then commits via the gate re-run (the path-ordered
    re-run queue of the Rust coordinator). This is the invariant the coordinator must
    establish — stated as a predicate, not an axiom over arbitrary runs. -/
def Published (run : List (DfsPath × Effect)) : Prop :=
  (runPaths run).Nodup

/-- **Law 25 (progress, proven)** — the fallback's path-sorted re-run commits each path at
    most once: the gate sort preserves path-nodup. -/
theorem fallback_rerun_published (run : List (DfsPath × Effect)) (h : (runPaths run).Nodup) :
    Published (pathSortedRun run) := by
  unfold Published
  exact runPaths_pathSortedRun_nodup h

end Rchain
