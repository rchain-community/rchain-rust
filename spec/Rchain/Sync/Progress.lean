import Mathlib.Data.List.Basic
import Init.Omega

/-!
# The windowed catch-up — a joined node's frontier is monotone, and its progress does not depend on the
  network being quiescent

**Why this file exists.** Every block request in this protocol is keyed by *hash*, so a node restored at
an anchor can only learn the gap above itself by pulling *downward* from a peer's tip through
`justifications`; each block justifies blocks inside the gap, so the walk pends the whole gap at once and
the receiver's fixed bound (`MAX_PENDING_BLOCKS`, `casper/src/engine/node_running.rs:756`) turns progress
into drops. Measured 2026-10-10 on a four-validator devnet
(`spec/audit/evidence/n-anchor-drill/run-2-catchup-stall.txt`): `block receiver state full (1024 blocks);
dropping` 173 times, **one** block validated, the node frozen at height 121 while the master ran to 577.

The landed fix (`casper/src/engine/catchup.rs`) walks the gap **upward in windows**: ask the peer for the
heights `[f+1, f+W]` (`BlockRangeRequest`, `catchup.rs:148-153`), request those blocks in the order the
responder listed them (topological, `node_running.rs:389-394`), and ask for the next window only once this
node's own frontier has reached the last (`catchup.rs:208-215`). The same 1200-block gap then reached the
tip in 98 seconds with zero drops (`run-3-windowed-catchup.txt`).

**The claim this file makes precise and checks.** A joined node's **frontier is monotone** (it never
decreases along any run), and **its progress does not depend on the network being quiescent** — the peer
may keep producing; every step is bounded by the window width `W`. The two halves are the theorems
`run_monotone` and `answered_run_reaches`, with the quiescence-independence carried by
`windowPending_independent_of_tip` and `answered_tip_independent`.

**What the model deliberately does *not* claim.** That a window always *arrives*. The peer may stall — a
silent or slow neighbour is modelled honestly as a step `f' = f` (`stall_is_a_step`), so the window bound
is stated of *answered* windows (`AnsweredRun`), not of every run. Reaching a *moving* tip, and the
join-termination question "does the walk finish at all", are the business of a separate module
(`lfs_block_requester.rs:234-276` is the downward walk's own give-up rule, `MAX_IDLE_ROUNDS`); this file
says only what one window does and what a run of answered windows buys. **The row this belongs to is
`C268`** (`spec/AUDIT.md`): a joiner against a producing master — the window is what keeps the gate from
holding the pending set open on the whole gap.

The Rust contact points, by definition:

* `WindowStep` — the step of `run_catchup`'s loop body (`catchup.rs:139-216`).
* `Answered` — `wait_for_frontier`'s success (`catchup.rs:235-243`) with the target `min (answer.to)
  (answer.tip)` (`catchup.rs:208-212`).
* `WindowStep`'s `f' ≤ f + W` — `CatchupWindow::hold`'s `frontier + width` (`catchup.rs:99-102`) and the
  ingest gate `admits` (`catchup.rs:88-90`, read at `node_running.rs:929`).
* `admits` / `hold` / `release` — `CatchupWindow` itself (`catchup.rs:70-108`).
* `windowPending` vs `gapPending` — the window's pace against the downward walk's whole-gap pending set
  (`lfs_block_requester.rs:207-222`, seeded from the fringe's hashes).
-/

namespace Rchain

namespace Sync

/-! ## The window — a width, a tip, and one step -/

/-- One window's effect on a node's frontier. `W` is the window width (`WINDOW_HEIGHTS` /
    `MAX_BLOCK_RANGE`, both `8`), `T` the peer's tip as the responder reported it (`catchup.rs:193`,
    `node_running.rs:388`), `f` the frontier before the window and `f'` after.

    The three conjuncts are the three things `run_catchup`'s loop body guarantees:

    * `f ≤ f'` — the frontier never goes backwards: `CatchupWindow::hold` only ever *widens* the ingest
      (`catchup.rs:99-102`) and the DAG only grows (`catchup.rs:213-215`);
    * `f' ≤ T` — the walk stops at the peer's tip; the driver clamps the window's target to `answer.tip`
      (`catchup.rs:208-212`) and breaks when `tip ≤ frontier` (`catchup.rs:193-202`);
    * `f' ≤ f + W` — one window is at most one width: the loop asks for `[f+1, f+W]`
      (`catchup.rs:148-153`) and holds the ingest open only to `frontier + width` (`catchup.rs:215`).

    A **stalled** window — a silent peer, `catchup.rs:154-165` — is `f' = f` (`stall_is_a_step`); an
    **answered** window is `f' = min (f + W) T` (`Answered`). Everything in between is a *partial* window:
    some of the window's blocks validated, the rest did not arrive in `WINDOW_TIMEOUT`.

    `f` is a height as a `Nat`; the port carries the frontier as an `i64` whose empty DAG reads `-1`
    (`frontier_of`, `catchup.rs:227-230`, `latest_block_number() - 1`). The model takes the frontier to be
    a `Nat` and does not carry the sentinel — the arithmetic the window does (`+ 1`, `+ W`) is the same. -/
def WindowStep (W T f f' : Nat) : Prop := f ≤ f' ∧ f' ≤ T ∧ f' ≤ f + W

/-- An **answered** window: the frontier reaches the window's top, or the tip if that is nearer. This is
    what `wait_for_frontier` yields when the window's blocks validate and the peer is keeping up or
    running ahead (`catchup.rs:208-213`, `:235-243`). A peer that keeps *producing* still closes every
    window it is asked for (`catchup.rs:187-202`, whose comment calls this "the case that matters"), which
    is why the bound below is stated of answered windows. -/
def Answered (W T f f' : Nat) : Prop := f' = min (f + W) T

/-- **A run**: a frontier per window index, each the result of a `WindowStep` from the last. The sequence
    is total — a walk that has reached the tip sits there for ever, which `WindowStep` admits as `f' = f`
    with `f = T`. -/
def Run (W T : Nat) (σ : Nat → Nat) : Prop := ∀ n, WindowStep W T (σ n) (σ (n + 1))

/-- **An answered run**: every window arrives and its blocks validate, so the frontier reaches each
    window's top. This is the hypothesis the window bound needs — the network may still be *producing*
    (the tip is free above), it just may not *stall*. -/
def AnsweredRun (W T : Nat) (σ : Nat → Nat) : Prop := ∀ n, Answered W T (σ n) (σ (n + 1))

/-- **The number of windows a gap of `T - f` takes**: `⌈(T - f) / W⌉`, written with `Nat`'s floor
    division (`(T - f + W - 1) / W`). For `W ≥ 1` this is exactly the ceiling
    (`reachSteps_spec`, and `answered_run_reaches` which spends it). -/
def reachSteps (W T f : Nat) : Nat := (T - f + W - 1) / W

/-! ## Monotonicity — PROVED -/

/-- **The frontier never decreases along any run** — stalled windows and all. `WindowStep`'s first
    conjunct, composed along the sequence. This is the property that makes a catch-up safe to run
    automatically: whatever else happens, the node is never *behind* where it was.

    Proved on the gap `m = n + c` (`Nat.le.dest`) so the step's own index is available for `h`. -/
theorem run_monotone {W T : Nat} {σ : Nat → Nat} (h : Run W T σ) (n m : Nat) (hnm : n ≤ m) :
    σ n ≤ σ m := by
  obtain ⟨c, hm⟩ := Nat.le.dest hnm
  rw [← hm]
  clear hnm hm
  induction c with
  | zero => exact le_rfl
  | succ d ih =>
    rw [Nat.add_succ]
    exact le_trans ih (h (n + d)).1

/-- A run's frontier is bounded above by the tip: the starting frontier is at or below `T`
    (`catchup.rs:193-202`) and every successor is (`WindowStep`'s second conjunct). -/
theorem run_le_tip {W T : Nat} {σ : Nat → Nat} (h : Run W T σ) (h0 : σ 0 ≤ T) :
    ∀ n, σ n ≤ T := by
  intro n
  cases n with
  | zero => exact h0
  | succ k => exact (h k).2.1

/-! ## A stall is a step — so the bound is about answered windows, not every run -/

/-- **A stalled window is a legitimate step.** A peer that never answers (`catchup.rs:154-165`, the
    `WINDOW_TIMEOUT` arm) leaves the frontier exactly where it was. This is why the window *bound* below
    is stated of `AnsweredRun` rather than `Run`: the machine as it is can stand still, and the model says
    so instead of idealising it away. (That a window always arrives against a *live* peer is the
    join-termination theorem's business — see the file header.) -/
theorem stall_is_a_step {W T f : Nat} (hf : f ≤ T) : WindowStep W T f f :=
  ⟨le_rfl, hf, Nat.le_add_right f W⟩

/-! ## The window bound — PROVED -/

/-- The clamp identity the induction below turns on: pushing `+ W` through `min · T` commutes with the
    clamp. Proved by cases on whether `a` is already at or above the tip. -/
theorem min_min_add (a T W : Nat) : min (min a T + W) T = min (a + W) T := by
  by_cases ha : a ≤ T
  · rw [min_eq_left ha]
  · have hTa : T ≤ a := by omega
    have h1 : min a T = T := min_eq_right hTa
    have h2 : min (T + W) T = T := min_eq_right (Nat.le_add_right T W)
    have h3 : min (a + W) T = T := min_eq_right (by omega)
    rw [h1, h2, h3]

/-- **The frontier after `n` answered windows is exactly `min (f + n·W) T`.** Each answered window closes
    a full width of the gap, so the frontier climbs by `W` a window — clamped at the tip. The proof's one
    step (`min_min_add`) is what says the clamp does not interfere with the width: the tip only ever
    *caps* the walk, it never widens a step. -/
theorem answered_run_eq {W T : Nat} {σ : Nat → Nat} (h : AnsweredRun W T σ) (h0 : σ 0 ≤ T) :
    ∀ n, σ n = min (σ 0 + n * W) T := by
  intro n
  induction n with
  | zero => rw [Nat.zero_mul, Nat.add_zero, min_eq_left h0]
  | succ k ih =>
    have hassoc : σ 0 + (k + 1) * W = (σ 0 + k * W) + W := by
      rw [Nat.add_mul, Nat.one_mul, Nat.add_assoc]
    rw [h k, ih, hassoc]
    exact min_min_add (σ 0 + k * W) T W

/-- An answered window is a `WindowStep` — it respects the tip and the width. (`f ≤ min (f + W) T` needs
    the frontier to be at or below the tip, which is `wait_for_frontier`'s own precondition,
    `catchup.rs:235-238`.) -/
theorem answered_step {W T f f' : Nat} (h : Answered W T f f') (hf : f ≤ T) :
    WindowStep W T f f' := by
  refine ⟨?_, ?_, ?_⟩
  · rw [h]; exact le_min (Nat.le_add_right f W) hf
  · rw [h]; exact min_le_right _ _
  · rw [h]; exact min_le_left _ _

/-- An answered run is a run. -/
theorem answeredRun_is_run {W T : Nat} {σ : Nat → Nat} (h : AnsweredRun W T σ) (h0 : σ 0 ≤ T) :
    Run W T σ := by
  intro n
  refine answered_step (h n) ?_
  rw [answered_run_eq h h0 n]
  exact min_le_right _ _

/-- **`⌈(T - f) / W⌉` windows are enough to cover the gap**: `f + reachSteps W T f · W ≥ T`. The `+ W - 1`
    in the numerator is the ceiling, and the proof is `Nat`'s division identity `mod_add_div` with the
    remainder below `W`. -/
theorem reachSteps_spec {W T f : Nat} (hW : 1 ≤ W) (hf : f ≤ T) :
    f + reachSteps W T f * W ≥ T := by
  unfold reachSteps
  have hmod : (T - f + W - 1) % W < W := Nat.mod_lt _ (by omega)
  have hdiv : (T - f + W - 1) % W + (T - f + W - 1) / W * W = T - f + W - 1 := by
    have h := Nat.mod_add_div (T - f + W - 1) W
    rw [Nat.mul_comm] at h
    exact h
  omega

/-- **A window bound — PROVED.** With a tip `T` at or above the frontier and every window answered, the
    walk reaches `T` in at most `⌈(T - f) / W⌉` windows: at window `reachSteps W T f` the frontier is
    exactly `T`. Against the run's 1200-block gap (`T - f = 300`, `W = 8`) that is 38 windows; the run
    measured 38 windows in 98 seconds with zero drops (`run-3-windowed-catchup.txt`).

    The `T` here is a *fixed* tip; that the step does not depend on the tip standing still is
    `answered_tip_independent` and `windowPending_independent_of_tip` below. -/
theorem answered_run_reaches {W T : Nat} (hW : 1 ≤ W) {σ : Nat → Nat} (h : AnsweredRun W T σ)
    (h0 : σ 0 ≤ T) : σ (reachSteps W T (σ 0)) = T := by
  rw [answered_run_eq h h0 (reachSteps W T (σ 0))]
  exact min_eq_right (reachSteps_spec hW h0)

/-- Past the bound, the frontier stays at the tip: once reached, more windows are no-ops (a run at `T`). -/
theorem answered_run_stays {W T : Nat} (hW : 1 ≤ W) {σ : Nat → Nat} (h : AnsweredRun W T σ)
    (h0 : σ 0 ≤ T) {n : Nat} (hn : reachSteps W T (σ 0) ≤ n) : σ n = T := by
  have hreach : σ (reachSteps W T (σ 0)) = T := answered_run_reaches hW h h0
  have hrun : Run W T σ := answeredRun_is_run h h0
  have hle : T ≤ σ n := by rw [← hreach]; exact run_monotone hrun _ _ hn
  exact le_antisymm (run_le_tip hrun h0 n) hle

/-! ## No step depends on the peer standing still

The downward walk (`lfs_block_requester.rs:207-222`) pends the **whole gap** at once — that is what fills
the receiver. The window is what bounds it, and the bound is the width `W`, *not* the gap. Read the two
theorems below together: the pending set is `≤ W` and is the *same integer* however far above the window
the tip sits; and an answered window lands the frontier at the same height whether the peer is still or
producing. That is the "no step's success depends on the peer standing still" claim, made precise. -/

/-- The heights one window asks for, when the tip is `T`: the `W` heights above the frontier, capped at
    the tip. This is the window's **pending set** — what the receiver must hold for one step. Contrast
    `gapPending`. -/
def windowPending (W T f : Nat) : Nat := min (f + W) T - f

/-- The downward walk's pending set — every height above the frontier, pended at once (the hash-keyed
    pull, `lfs_block_requester.rs:207-222`). -/
def gapPending (T f : Nat) : Nat := T - f

/-- **The window's pending set is at most one width, whatever the gap.** `min (f + W) T ≤ f + W`, so the
    step holds at most `W` heights — never the gap. This is the bound `MAX_PENDING_BLOCKS` needs:
    `WINDOW_HEIGHTS = 8` on a four-validator shard is at most 32 blocks (`catchup.rs:48-51`). -/
theorem windowPending_le_width (W T f : Nat) : windowPending W T f ≤ W := by
  have h : min (f + W) T ≤ f + W := min_le_left _ _
  unfold windowPending
  omega

/-- **Adding more blocks above the frontier never widens the pending set.** When the tip is a full window
    above the frontier *or further*, the window is the same `W` heights whatever the tip is: `T` and `T'`
    both at least `f + W` give the same pending set. A producing peer is one whose tip is far above; this
    theorem says the step does not see the difference. -/
theorem windowPending_independent_of_tip {W T T' f : Nat} (hT : f + W ≤ T) (hT' : f + W ≤ T') :
    windowPending W T f = windowPending W T' f := by
  unfold windowPending
  rw [min_eq_left hT, min_eq_left hT']

/-- **The contrast with the downward walk.** When the gap exceeds the window (`f + W < T`), the downward
    walk's pending set is *strictly larger* than the window — it would pend more than a window's worth at
    once. This is the shape that produced 173 drops and one validated block. -/
theorem gapPending_exceeds_the_window {W T f : Nat} (h : f + W < T) :
    W < gapPending T f := by
  unfold gapPending
  omega

/-- **A step's success does not depend on the peer standing still.** Two answered windows from the same
    frontier, against a tip at `T` and a tip at `T'`, both at least one window above the frontier, land
    the frontier at the *same* height `f + W`. The tip enters only as the clamp `min (f + W) T`; blocks
    above the window are not asked for, so whether the peer is standing still or producing is invisible to
    the step. This is what makes the catch-up work against a *live* master — the case `run_catchup`'s "the
    case that matters" comment is about (`catchup.rs:187-202`). -/
theorem answered_tip_independent {W T T' f f₁ f₂ : Nat} (hT : f + W ≤ T) (hT' : f + W ≤ T')
    (h₁ : Answered W T f f₁) (h₂ : Answered W T' f f₂) : f₁ = f₂ := by
  rw [h₁, h₂, min_eq_left hT, min_eq_left hT']

/-- The first height a window names, mirroring `run_catchup`'s `from` (`catchup.rs:148`:
    `frontier.saturating_add(1)`). -/
def windowFrom (f : Nat) : Nat := f + 1

/-- The last height a window names (`catchup.rs:149-151`, `to = from + width - 1 = f + width`). -/
def windowTo (W f : Nat) : Nat := f + W

/-- **One window names exactly `W` heights**, whatever the gap above it. The window's *width*, not the
    *gap*, is what the requester asks for — the fix's whole content against the downward walk. -/
theorem window_height_count {W f : Nat} (hW : 1 ≤ W) :
    windowTo W f - windowFrom f + 1 = W := by
  unfold windowTo windowFrom
  omega

/-! ## The ingest gate — `CatchupWindow::admits`, mirrored

The window is also a *gate* on the ordinary ingest: while a walk is catching up, a block above the window
is left **un-acked**, not refused (`catchup.rs:83-90`, read at `node_running.rs:923-939`), so the retriever
re-offers it once the frontier has moved past it — a pace, not a loss. `none` is the released window
(`i64::MAX`, `catchup.rs:78`): an unattended node admits everything exactly as before. -/

/-- The ingest window's top, as `Option`: `none` is the released window (`catchup.rs:78`, `i64::MAX`),
    `some t` a held one. One question, the one `node_running.rs:929` asks: may a block at this height be
    pended *now*? -/
def admits (top : Option Nat) (height : Nat) : Bool :=
  match top with
  | none => true
  | some t => decide (height ≤ t)

/-- Hold the window open to `frontier + width` (`CatchupWindow::hold`, `catchup.rs:99-102`, called at
    `catchup.rs:129,215`). -/
def hold (W f : Nat) : Option Nat := some (f + W)

/-- Release the window — every height is admitted again (`CatchupWindow::release`, `catchup.rs:105-107`,
    called on every exit path at `catchup.rs:217`). -/
def release : Option Nat := none

/-- **Released, the window admits every height** — the Rust test `a_released_window_admits_every_height`
    (`catchup.rs:252-262`). An ordinary node's ingest must be exactly what it was before the driver
    existed, or this would be a bound on every node rather than a pace for one that is catching up. -/
theorem released_admits_everything (height : Nat) : admits release height = true := rfl

/-- **A held window admits the window and nothing above it** — the Rust test
    `a_held_window_admits_the_window_and_not_the_gap_above_it` (`catchup.rs:267-280`). The height `f + W`
    is admitted; one past it is the gap. -/
theorem held_admits_the_window_not_the_gap (W f : Nat) :
    admits (hold W f) (f + W) = true ∧ admits (hold W f) (f + W + 1) = false := by
  refine ⟨?_, ?_⟩
  · unfold admits hold; exact decide_eq_true (le_refl _)
  · unfold admits hold; exact decide_eq_false (by omega)

/-- A held window admits every height at or below its top — an ancestor or a late arrival, which is never
    what fills the pending set (`catchup.rs:279`, "an ancestor is always admissible"). -/
theorem held_admits_below (W f h : Nat) (hh : h ≤ f + W) : admits (hold W f) h = true := by
  unfold admits hold
  exact decide_eq_true hh

end Sync

end Rchain
