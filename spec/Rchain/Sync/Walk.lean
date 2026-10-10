import Rchain.Progress
import Mathlib.Data.Finset.Card

/-!
# C268 — the state-sync page walk: the loop that has no give-up rule

**The defect this file models.** `casper/src/engine/lfs_tuple_space_requester.rs`'s
`request_tuple_space_roots` has exactly two exits and neither is a bound: its request loop
(`:366-391`) leaves only when `error` is set or `st.is_finished()` holds, and its response loop
(`:394-409`) leaves only on a closed channel or an error. A joiner whose pages never complete
therefore spins — on every `request_timeout` the loop resends the same request, for ever.

That alone would be survivable, and it is not alone, which is the whole of C268.
`node_syncing.rs:461` runs the two legs under `tokio::join!`:

    let (block_st, tuple_res) = tokio::join!(block_fut, tuple_fut);

so an attempt returns only when **both** legs return. The block leg has a give-up rule —
`MAX_IDLE_ROUNDS` (`lfs_block_requester.rs:53`), spent at `:249-266` — and the page walk has
none. A walk that is answered but never finishes therefore does not fail the attempt either:
`attempts` never increments, `MAX_SYNC_ATTEMPTS` never bites (`node_syncing.rs:372-380`), the
`terminal` signal never fires (`:392`), and the node sits in `NodeSyncing` serving an API over a
DAG it never populated. The comment at `node_syncing.rs:70-75` bounds the *retries* and says "the
total wait is bounded by the product of the two"; the product of a bounded leg with an unbounded
one is not a bound, and the falsifier below is that arithmetic.

**What is here.**

* the walk's state (`WalkState`) and its two loops as two step relations: `written`, which is
  `request_tuple_space_roots` as written, and `fixed`, which is the same loop carrying the block
  leg's give-up rule;
* the falsifier as a **concrete infinite run** — `the_page_walk_as_written_spins_for_ever`: the
  responder answers a path the walk never asked for, so every turn changes nothing and the goal is
  never reached. `the_page_walk_as_written_drifts` says the same thing in `Rchain.Progress`'s
  vocabulary and `the_page_walk_as_written_is_unpaced` is its negation;
* the positive direction, proved: `the_page_walk_with_the_give_up_rule_is_paced` (the `Paced` shape),
  `run_length_le_fuel` (a run cannot outlast its fuel) and `the_fixed_walk_cannot_run_for_ever` (no
  infinite run at all) — the loop that carries the block leg's rule exits;
* the goal direction in two deliberately unequal halves: what is **proved** is the bridge
  `all_keys_done_of_full_measure` (a maximal measure *is* the goal), and what is **owed** is
  `ReachesTheGoal`, a `Prop` with the hypothesis it needs named, because that hypothesis is not true
  of the Rust and no model may quietly assume it.

The walk's keys are `StatePartPath`s (`lfs_tuple_space_requester.rs:26`) — nested `(hash, index)`
rungs of the state trie. Nothing below depends on what a path is, only on the four statuses and on
which of them the loop reads, so the model keeps them abstract.
-/

namespace Rchain

namespace Sync
namespace Walk

/-- One key's request status — the Rust's `ReqStatus`
    (`casper/src/engine/lfs_tuple_space_requester.rs:32-38`): the same four cases in the same order,
    with the transitions the Rust gives each of them (`:64-97`). -/
inductive Status where
  /-- `Init` — named, not asked for. `getNext` selects exactly these on the first round
      (`:64-76`). -/
  | init
  /-- `Requested` — asked for. An answer for a key in this status is the **only** kind the walk
      accepts: `received` reports `false` for every other status, `none` (never named) included
      (`:80-87`). -/
  | requested
  /-- `Received` — an answer arrived, validation passed, `done` has not run yet. -/
  | received
  /-- `Done` — imported. **Absorbing**: `add` refuses to touch a key it already holds (`:55-61`),
      `getNext` moves only `init`, `received` only `requested`, `done` only `received` (`:64-97`) —
      which is what makes the measure below monotone. -/
  | done
  deriving DecidableEq

/-- The walk's state: the requester's map, plus the two counters the **block leg** keeps
    (`lfs_block_requester.rs:236-238`) and the page walk does not.

    `d` **is** `LfsTupleSpaceState.d` (`lfs_tuple_space_requester.rs:42-44`), with `none` read as
    "never named" — how the Rust's `BTreeMap` separates a key it holds from one it has never heard
    of, and what makes `received` answer `false` for it.

    `idle` and `lastFinished` are `idle_rounds` and `last_finished`: **locals of `request_blocks`'
    loop there, fields of the state here**, carried so that `fixed` below can be that loop and
    `written` exactly the one that has neither. -/
structure WalkState (Key : Type) where
  d : Key → Option Status
  idle : Nat
  lastFinished : Nat

/-- How many consecutive idle rounds may complete nothing before the walk gives up — the block leg's
    constant (`lfs_block_requester.rs:53`), carried over unchanged. -/
def MAX_IDLE_ROUNDS : Nat := 3

section

variable {Key : Type} [DecidableEq Key]

omit [DecidableEq Key] in
/-- Two states are equal when their three fields are — the structure's own extensionality, spelled
    out so that every proof below can use it without depending on `ext` for a structure whose first
    field is a function. -/
theorem walkState_ext {σ τ : WalkState Key} (hd : σ.d = τ.d) (hi : σ.idle = τ.idle)
    (hl : σ.lastFinished = τ.lastFinished) : σ = τ := by
  cases σ
  cases τ
  simp_all

/-- `getNext`'s **state effect** (`lfs_tuple_space_requester.rs:64-76`): every `Init` key, and only
    an `Init` key, becomes `Requested`. The `resend` flag decides which keys are *returned*, not
    what the state becomes — which is why the loop's request turn moves the state this way whether
    it fires on a trigger or on the idle timeout, and why a state with nothing `Init` is a fixed
    point of it. -/
def requestTurn (σ : WalkState Key) : WalkState Key :=
  { σ with d := fun k => if σ.d k = some Status.init then some Status.requested else σ.d k }

/-- The keys of `K` the walk has **finished** — the block leg's `finished.len()`
    (`lfs_block_requester.rs:250-252`), which is the measure its give-up rule counts, and the subject
    of `LfsTupleSpaceState::is_finished` (`:100-102`).

    `K` is the walk's key universe, and it is a parameter rather than "all keys" because the model
    has no reason to be more specific: `doneKeys_accept` below needs only that the key an answer
    completes lies in `K`. -/
def doneKeys (K : Finset Key) (σ : WalkState Key) : Finset Key :=
  K.filter (fun k => σ.d k = some Status.done)

/-- The measure: the number of keys of `K` the walk has finished. -/
def finished (K : Finset Key) (σ : WalkState Key) : Nat := (doneKeys K σ).card

/-- An **accepted** answer for `k`, the responder's next page starting at `last` — the Rust's
    `process_store_items` (`:246-303`) in one step: `received k`, `add last_path`, `done k`, in that
    order. `last = k` is harmless because `add` never overwrites (`:55-61`), and under that clause
    this is the Rust's result on the nose. -/
def accept (k last : Key) (σ : WalkState Key) : WalkState Key :=
  { σ with d := fun j =>
      if j = k then some Status.done
      else if j = last then (if σ.d j = none then some Status.init else σ.d j)
      else σ.d j }

/-- The **response turn's effect** on the requester — `process_store_items` (`:246-303`), whose
    entire body sits under `if is_received` (`:262`). An answer for a key the walk never asked for —
    `none`, or `init`, or anything but `requested` — is **nothing at all**: `received` reports
    `false` (`:80-87`), no key is added, nothing is imported, and the request queue is not even
    triggered. `an_unrequested_answer_leaves_the_walk_where_it_was` below is that clause, and it is
    what the falsifier's run is built from. -/
def answerTurn (k last : Key) (σ : WalkState Key) : WalkState Key :=
  if σ.d k = some Status.requested then accept k last σ else σ

/-- The goal: `LfsTupleSpaceState::is_finished` (`:100-102`) — every key the map holds is `Done`.
    Vacuously true of a map that holds none, which is the Rust's own reading and the wart
    `the_goal_does_not_imply_a_full_measure` records. -/
def isFinished (σ : WalkState Key) : Prop := ∀ k s, σ.d k = some s → s = Status.done

/-- One **idle round**'s counter update — the block leg's give-up rule
    (`lfs_block_requester.rs:245-276`) as a state transition: read the measure, and either spend a
    round of the budget (nothing finished since the last idle round) or refill it (something did).
    The Rust *fails the walk* at `MAX_IDLE_ROUNDS`; here that is the state `idle = MAX_IDLE_ROUNDS`,
    from which `fixed` has no step — the same move `Rchain.Progress` makes of a silent term. -/
def idleTurn (K : Finset Key) (σ : WalkState Key) : WalkState Key :=
  if σ.lastFinished = finished K σ then { σ with idle := σ.idle + 1 }
  else { σ with idle := 0, lastFinished := finished K σ }

/-- One turn of the loop on the **timeout** path: the counter update, then `request_next` (whose
    state effect is `requestTurn`, `:196-242`). The order is the Rust's — the counter update lives in
    the `sleep` arm of the `select!` (`:245-275`) and runs before `request_next` (`:383`) — and the
    two commute anyway, since `requestTurn` moves no `Done` key and `finished` is all that `idleTurn`
    reads. -/
def idleRound (K : Finset Key) (σ : WalkState Key) : WalkState Key :=
  requestTurn (idleTurn K σ)

/-- One turn of the loop on the **response** path: `request_next` after an answer has been consumed
    (the Rust's response loop triggers the request queue, `:269`/`:300`). -/
def progressRound (k last : Key) (σ : WalkState Key) : WalkState Key :=
  requestTurn (answerTurn k last σ)

/-! ### The two loops

`written` is `request_tuple_space_roots`'s loop; `fixed` is that loop with the block leg's give-up
rule added. The difference between them is one constructor, and it is the whole of C268. -/

/-- The walk's turns **exactly as `request_tuple_space_roots` is written** (`:366-409`).

    One constructor per loop turn: the request turn (`request_next`) and the response turn (one
    `StoreItemsMessage` consumed).

    **Neither touches the counters, because the loop has none** — and neither can end a run, so every
    state here has a step. That is not an omission in the model, it is the loop: its only exit is
    `is_finished()`, so a state that is not finished and can accept no answer keeps taking turns for
    ever. `the_page_walk_as_written_spins_for_ever` builds such a run. -/
inductive WrittenStep : WalkState Key → WalkState Key → Prop
  /-- The request turn — the loop's `request_next` (`:383`), including the idle resend. -/
  | resend (σ : WalkState Key) : WrittenStep σ (requestTurn σ)
  /-- The response turn: one incoming `StoreItemsMessage`. `answerTurn` is the identity unless the
      message answers a `Requested` key, so this constructor is also the *unsolicited* answer — a
      peer pushing a path the walk never asked for, which changes nothing. -/
  | answer (k last : Key) (σ : WalkState Key) : WrittenStep σ (progressRound k last σ)

/-- The walk's turns **with the give-up rule** — `request_blocks`' loop
    (`lfs_block_requester.rs:239-281`) transferred to the page walk: the same two turns, plus the idle
    round, and a state whose budget is spent has **no step**, which is the Rust's `Err` return
    (`:256-261`) unwinding the loop.

    `k ∈ K` on `progress` is the responder's honesty, and it is the one hypothesis this machine
    needs: the key an answer *completes* must be one of the walk's keys for the measure to record it.
    A response naming a path outside the trie fails `validate_state_items`
    (`lfs_tuple_space_requester.rs:272-283`) and errors the walk out, so no completed round answers a
    key outside `K`. -/
inductive FixedStep (K : Finset Key) : WalkState Key → WalkState Key → Prop
  /-- The idle round: a whole `request_timeout` with nothing finished, which is what the give-up rule
      counts. -/
  | idle (σ : WalkState Key) (h : σ.idle < MAX_IDLE_ROUNDS) : FixedStep K σ (idleRound K σ)
  /-- A turn in which a **`Requested`** key was answered and the walk moved. -/
  | progress (k last : Key) (σ : WalkState Key) (hk : σ.d k = some Status.requested)
      (hkK : k ∈ K) (h : σ.idle < MAX_IDLE_ROUNDS) : FixedStep K σ (progressRound k last σ)

/-- The loop as written, as a transition system. `enabled` is `true` everywhere: that **every** state
    has a step is the defect, and the instance proves it of its own relation. -/
def written : System where
  State := WalkState Key
  Step := WrittenStep
  enabled := fun _ => true
  enabled_iff := fun σ => ⟨fun _ => ⟨requestTurn σ, WrittenStep.resend σ⟩, fun _ => rfl⟩

/-- The loop with the give-up rule. `enabled` is "the budget is not spent", and the instance proves it
    agrees with the relation: a state with budget has the idle round, and a state without has no step
    at all. -/
def fixed (K : Finset Key) : System where
  State := WalkState Key
  Step := FixedStep K
  enabled := fun σ => decide (σ.idle < MAX_IDLE_ROUNDS)
  enabled_iff := fun σ =>
    ⟨fun h => ⟨idleRound K σ, FixedStep.idle σ (of_decide_eq_true h)⟩,
     fun ⟨_, hs⟩ => by
       cases hs with
       | idle σ hid => exact decide_eq_true hid
       | progress k last σ hk hkK hid => exact decide_eq_true hid⟩

/-! ### The state after the first request turn — the falsifier's starting point -/

/-- The state `request_tuple_space_roots` is in one turn after it seeds itself from a single root
    (`:348-360`): the root is `Requested` and **nothing is `Init`**, so every later request turn is
    the identity (`requestTurn_asked`). `Bool` is the smallest key type with room for both a key the
    walk asked for (`true`) and one it never named (`false`). -/
def asked : WalkState Bool where
  d := fun b => if b then some Status.requested else none
  idle := 0
  lastFinished := 0

/-! ### The requester's operations, as the facts this file needs -/

omit [DecidableEq Key] in
/-- The **frame lemmas**: the request turn moves `d` and nothing else. -/
theorem requestTurn_idle (σ : WalkState Key) : (requestTurn σ).idle = σ.idle := rfl

omit [DecidableEq Key] in
theorem requestTurn_lastFinished (σ : WalkState Key) :
    (requestTurn σ).lastFinished = σ.lastFinished := rfl

theorem requestTurn_asked : requestTurn asked = asked := by
  refine walkState_ext ?_ rfl rfl
  funext k
  cases k <;> simp [requestTurn, asked]

/-- `received`'s clause, as an identity: **an answer for a key the walk did not ask for changes
    nothing.** The Rust reports `false` for anything but `Requested` (`:80-87`) and
    `process_store_items` then does nothing at all (`:262`), so the response turn's effect *is* the
    identity — which is why the falsifier's trace is a fixed point. -/
theorem an_unrequested_answer_leaves_the_walk_where_it_was (k last : Key) (σ : WalkState Key)
    (h : σ.d k ≠ some Status.requested) : answerTurn k last σ = σ := by
  unfold answerTurn
  rw [if_neg h]

omit [DecidableEq Key] in
/-- The idle resend re-asks what is already `Requested` and marks nothing new, so on a state with
    nothing `Init` it is a no-op. -/
theorem a_resend_moves_nothing (σ : WalkState Key) (h : ∀ k, σ.d k ≠ some Status.init) :
    requestTurn σ = σ := by
  refine walkState_ext ?_ rfl rfl
  funext k
  simp only [requestTurn]
  rw [if_neg (h k)]

/-- **`received` moves the measure.** An accepted answer completes its own key; `add last_path` may
    add one `Init` key, which changes no `Done` key (the Rust's `or_insert`, `:55-61`), so the
    finished set grows by exactly `k` — which is what makes the measure a measure. -/
theorem doneKeys_accept (K : Finset Key) (σ : WalkState Key) (k last : Key)
    (hk : σ.d k = some Status.requested) (hkK : k ∈ K) :
    doneKeys K (accept k last σ) = insert k (doneKeys K σ) := by
  unfold doneKeys
  ext j
  simp only [Finset.mem_filter, Finset.mem_insert]
  by_cases hjk : j = k
  · subst hjk
    simp [accept, hkK]
  · rw [accept]
    simp only [if_neg hjk]
    by_cases hjl : j = last
    · subst hjl
      rw [if_pos rfl]
      by_cases hd : σ.d j = none
      · rw [if_pos hd]
        simp [hd, hjk]
      · rw [if_neg hd]
        simp [hjk]
    · rw [if_neg hjl]
      simp [hjk]

/-- The measure's arithmetic, in the direction the give-up rule reads it: an accepted answer raises
    `finished` by exactly one, whatever the responder's next page is. -/
theorem finished_accept (K : Finset Key) (σ : WalkState Key) (k last : Key)
    (hk : σ.d k = some Status.requested) (hkK : k ∈ K) :
    finished K (accept k last σ) = finished K σ + 1 := by
  have hknot : k ∉ doneKeys K σ := by
    intro hmem
    simp only [doneKeys, Finset.mem_filter] at hmem
    obtain ⟨_, hd⟩ := hmem
    rw [hk] at hd
    exact absurd hd (by decide)
  unfold finished
  rw [doneKeys_accept K σ k last hk hkK, Finset.card_insert_of_not_mem hknot]

omit [DecidableEq Key] in
/-- The request turn moves no `Done` key, so it moves no measure. -/
theorem doneKeys_requestTurn (K : Finset Key) (σ : WalkState Key) :
    doneKeys K (requestTurn σ) = doneKeys K σ := by
  unfold doneKeys
  ext j
  simp only [Finset.mem_filter, requestTurn]
  by_cases hc : σ.d j = some Status.init
  · rw [if_pos hc]
    simp [hc]
  · rw [if_neg hc]

omit [DecidableEq Key] in
theorem finished_requestTurn (K : Finset Key) (σ : WalkState Key) :
    finished K (requestTurn σ) = finished K σ := by
  unfold finished
  rw [doneKeys_requestTurn]

/-- The response turn moves the measure by one exactly when the answer is accepted, and not at all
    otherwise — `finished_requestTurn`'s twin for the response path. -/
theorem finished_answerTurn (K : Finset Key) (σ : WalkState Key) (k last : Key)
    (hk : σ.d k = some Status.requested) (hkK : k ∈ K) :
    finished K (answerTurn k last σ) = finished K σ + 1 := by
  unfold answerTurn
  rw [if_pos hk]
  exact finished_accept K σ k last hk hkK

omit [DecidableEq Key] in
/-- The idle round moves no `Done` key either — it reads the measure and writes only the budget. -/
theorem idleTurn_d (K : Finset Key) (σ : WalkState Key) : (idleTurn K σ).d = σ.d := by
  by_cases h : σ.lastFinished = finished K σ
  · simp only [idleTurn, if_pos h]
  · simp only [idleTurn, if_neg h]

omit [DecidableEq Key] in
theorem finished_idleTurn (K : Finset Key) (σ : WalkState Key) :
    finished K (idleTurn K σ) = finished K σ := by
  unfold finished doneKeys
  rw [idleTurn_d]

/-! ### The falsifier: a run of the walk as written that never reaches the goal -/

/-- **C268, as a trace.** A responder that answers `false` — a path the walk never asked for — for
    ever. Every turn is a response turn for an unrequested key, so by
    `an_unrequested_answer_leaves_the_walk_where_it_was` every turn leaves `asked` where it is; the
    state is a fixed point, the run is infinite, and `isFinished` is false at every point of it: the
    one key the map holds is `Requested`, and `:100-102` reads that as unfinished.

    **This is the machine as `request_tuple_space_roots` is written** (`:366-409`): the two
    constructors of `WrittenStep` are its two `select!` arms, and nothing in either bounds how many
    times it runs. On the real node the turn that matters is the resend — `requestTurn_asked` shows
    that constructor is a self-loop here too — and it is the pairing with
    `node_syncing.rs:461`'s `join!` that makes it fatal. -/
theorem the_page_walk_as_written_spins_for_ever :
    ∃ trace : Nat → WalkState Bool,
      (∀ n, WrittenStep (trace n) (trace (n + 1))) ∧ ∀ n, ¬ isFinished (trace n) := by
  have hfix : progressRound (Key := Bool) false false asked = asked := by
    unfold progressRound
    rw [an_unrequested_answer_leaves_the_walk_where_it_was false false asked (by simp [asked])]
    exact requestTurn_asked
  refine ⟨fun _ => asked, fun n => ?_, fun n h => ?_⟩
  · have hstep : WrittenStep asked (progressRound false false asked) :=
      WrittenStep.answer false false asked
    rwa [hfix] at hstep
  · exact absurd (h true Status.requested rfl) (by decide)

/-- A run of `n` steps of a self-looping relation — what turns the trace above into the list
    `Rchain.System.Run` is stated over. -/
theorem chain_replicate {α : Type} (R : α → α → Prop) (a : α) (h : R a a) :
    ∀ n : Nat, List.Chain' R (List.replicate n a)
  | 0 => trivial
  | 1 => List.Chain.nil
  | n + 2 => by
      have hrep : List.replicate (n + 2) a = a :: (a :: List.replicate n a) := by
        rw [show n + 2 = n + 1 + 1 from rfl, List.replicate_succ, List.replicate_succ]
      rw [hrep]
      exact List.Chain.cons h (chain_replicate R a h (n + 1))

/-- Every element of `a :: List.replicate n a` is `a`. -/
theorem eq_of_mem_self_cons_replicate {α : Type} {a t : α} {n : Nat}
    (h : t ∈ a :: List.replicate n a) : t = a := by
  rcases List.mem_cons.mp h with h' | h'
  · exact h'
  · exact List.eq_of_mem_replicate h'

/-- The trace above in `Rchain.Progress`'s vocabulary: a `Drift` of any length. The measure never
    moves because the state never moves. -/
theorem the_page_walk_as_written_drifts (K : Finset Bool) (n : Nat) :
    System.Drift (written (Key := Bool)) (fun σ => finished K σ) n := by
  have hloop : WrittenStep asked asked := by
    have h1 : WrittenStep asked (requestTurn asked) := WrittenStep.resend asked
    rwa [requestTurn_asked] at h1
  exact ⟨asked, List.replicate n asked,
    chain_replicate WrittenStep asked hloop (n + 1), by simp, fun t ht => by
      rw [eq_of_mem_self_cons_replicate ht]⟩

/-- `Drift` refutes `Paced` at every bound below the drift's length — the step the register takes
    whenever a pace bound is proposed, in the form `Rchain/Progress.lean` gives the shapes. -/
theorem drift_refutes_pace {S : System} {m : S.State → Nat} {k n : Nat} (hd : System.Drift S m n)
    (hkn : k < n) : ¬ System.Paced S m k := by
  rintro hp
  obtain ⟨σ, rest, hrun, hlen, hconst⟩ := hd
  obtain ⟨t, ht, hne⟩ := hp σ rest hrun (by rw [hlen]; exact hkn)
  exact hne (hconst t ht)

/-- **No pace bound holds of the loop as written**: whatever `k` a repair might state, there is a run
    longer than `k` on which the measure never moves. This is the negative half of the register row —
    `Progress.lean`'s `Paced` refused, which is C171's shape in the state walk. -/
theorem the_page_walk_as_written_is_unpaced (K : Finset Bool) (k : Nat) :
    ¬ System.Paced (written (Key := Bool)) (fun σ => finished K σ) k :=
  drift_refutes_pace (the_page_walk_as_written_drifts K (k + 1)) (Nat.lt_succ_self k)

/-! ### The positive direction: the give-up rule bounds the walk

The measure is `finished` and the rule is the block leg's, because the defect is the same defect.
Two statements are proved, the second stronger than the first:

* `the_page_walk_with_the_give_up_rule_is_paced` — `Paced`, the shape `lfs_block_requester.rs:36-52`
  argues for its own constant and `Laws.lean:2853` already reads `MAX_IDLE_ROUNDS` as;
* `the_fixed_walk_cannot_run_for_ever` — no infinite run at all, which is what the `join!` at
  `node_syncing.rs:461` needs: a leg that returns is a leg the attempt can bound.

Both rest on one **decreasing measure on the state**, `fuel`, which is why the bound composes across
runs of any shape rather than being a wall the argument stops at. -/

omit [DecidableEq Key] in
/-- Whether the walk owes an idle round an explanation: `1` when the measure has moved since the last
    idle round (so the round refills the budget rather than spending it), `0` when it has not. It is
    the block leg's `finished == last_finished` test (`lfs_block_requester.rs:253`) as a `Nat`. -/
def pending (K : Finset Key) (σ : WalkState Key) : Nat :=
  if σ.lastFinished = finished K σ then 0 else 1

omit [DecidableEq Key] in
theorem pending_eq_zero (K : Finset Key) (σ : WalkState Key)
    (h : σ.lastFinished = finished K σ) : pending K σ = 0 := by
  unfold pending
  rw [if_pos h]

omit [DecidableEq Key] in
theorem pending_eq_one (K : Finset Key) (σ : WalkState Key)
    (h : σ.lastFinished ≠ finished K σ) : pending K σ = 1 := by
  unfold pending
  rw [if_neg h]

omit [DecidableEq Key] in
theorem pending_le_one (K : Finset Key) (σ : WalkState Key) : pending K σ ≤ 1 := by
  unfold pending
  split <;> simp

omit [DecidableEq Key] in
/-- The budget term of `fuel`: what is left of the idle allowance, plus one round of credit for the
    refill a progress turn will earn. -/
def budget (K : Finset Key) (σ : WalkState Key) : Nat :=
  (MAX_IDLE_ROUNDS - σ.idle) + (MAX_IDLE_ROUNDS + 1) * pending K σ

omit [DecidableEq Key] in
theorem deltaWeight_le (K : Finset Key) (σ : WalkState Key) :
    (MAX_IDLE_ROUNDS + 1) * pending K σ ≤ MAX_IDLE_ROUNDS + 1 := by
  have h := Nat.mul_le_mul_left (MAX_IDLE_ROUNDS + 1) (pending_le_one K σ)
  rwa [Nat.mul_one] at h

omit [DecidableEq Key] in
theorem budget_le (K : Finset Key) (σ : WalkState Key) :
    budget K σ ≤ (MAX_IDLE_ROUNDS - σ.idle) + (MAX_IDLE_ROUNDS + 1) := by
  unfold budget
  exact Nat.add_le_add_left (deltaWeight_le K σ) _

omit [DecidableEq Key] in
/-- **How many more turns a run can take** — a bound on the *state*, so that it composes across steps.
    Two terms: the parts of the trie still to walk (each progress turn paying one off at the weight
    below), and the idle budget in hand with its credit. -/
def fuel (K : Finset Key) (σ : WalkState Key) : Nat :=
  (K.card - finished K σ) * (2 * MAX_IDLE_ROUNDS + 2) + budget K σ

omit [DecidableEq Key] in
theorem max_idle_lt_weight : MAX_IDLE_ROUNDS + 1 < 2 * MAX_IDLE_ROUNDS + 2 := by
  rw [show 2 * MAX_IDLE_ROUNDS + 2 = 2 * (MAX_IDLE_ROUNDS + 1) from (Nat.mul_succ 2 _).symm]
  have h : 1 * (MAX_IDLE_ROUNDS + 1) < 2 * (MAX_IDLE_ROUNDS + 1) :=
    Nat.mul_lt_mul_of_pos_right (by decide : 1 < 2) (Nat.succ_pos _)
  rwa [Nat.one_mul] at h

omit [DecidableEq Key] in
/-- `finished` is bounded by the walk's universe — `doneKeys` is a filter of `K`. This is the
    finiteness the bound needs: without it the walk would be paced and still unbounded, because every
    answer may name a page with a key of its own. -/
theorem finished_le_card (K : Finset Key) (σ : WalkState Key) : finished K σ ≤ K.card := by
  unfold finished doneKeys
  exact Finset.card_le_card (Finset.filter_subset _ _)

omit [DecidableEq Key] in
theorem budget_lt_of_idle_eq (K : Finset Key) {σ σ' : WalkState Key}
    (hidle : σ'.idle = σ.idle) :
    budget K σ' < (2 * MAX_IDLE_ROUNDS + 2) + budget K σ := by
  have h1 : budget K σ' ≤ (MAX_IDLE_ROUNDS - σ.idle) + (MAX_IDLE_ROUNDS + 1) := by
    have h := budget_le K σ'
    rwa [hidle] at h
  have h2 : (MAX_IDLE_ROUNDS - σ.idle) + (MAX_IDLE_ROUNDS + 1) <
      (MAX_IDLE_ROUNDS - σ.idle) + (2 * MAX_IDLE_ROUNDS + 2) :=
    Nat.add_lt_add_left max_idle_lt_weight _
  have h3 : MAX_IDLE_ROUNDS - σ.idle ≤ budget K σ := by
    unfold budget
    exact Nat.le_add_right _ _
  have h4 : (2 * MAX_IDLE_ROUNDS + 2) + (MAX_IDLE_ROUNDS - σ.idle) ≤
      (2 * MAX_IDLE_ROUNDS + 2) + budget K σ := Nat.add_le_add_left h3 _
  calc budget K σ'
      ≤ (MAX_IDLE_ROUNDS - σ.idle) + (MAX_IDLE_ROUNDS + 1) := h1
    _ < (MAX_IDLE_ROUNDS - σ.idle) + (2 * MAX_IDLE_ROUNDS + 2) := h2
    _ = (2 * MAX_IDLE_ROUNDS + 2) + (MAX_IDLE_ROUNDS - σ.idle) := Nat.add_comm _ _
    _ ≤ (2 * MAX_IDLE_ROUNDS + 2) + budget K σ := h4

/-- The response turn leaves the budget alone — it moves `d` and nothing else. -/
theorem answerTurn_idle (k last : Key) (σ : WalkState Key) :
    (answerTurn k last σ).idle = σ.idle := by
  by_cases hc : σ.d k = some Status.requested
  · simp [answerTurn, accept, if_pos hc]
  · simp [answerTurn, if_neg hc]

theorem progressRound_idle (k last : Key) (σ : WalkState Key) :
    (progressRound k last σ).idle = σ.idle := by
  unfold progressRound
  rw [requestTurn_idle, answerTurn_idle]

omit [DecidableEq Key] in
theorem idleRound_idle (K : Finset Key) (σ : WalkState Key) :
    (idleRound K σ).idle = (idleTurn K σ).idle := rfl

omit [DecidableEq Key] in
theorem idleRound_lastFinished (K : Finset Key) (σ : WalkState Key) :
    (idleRound K σ).lastFinished = (idleTurn K σ).lastFinished := rfl

omit [DecidableEq Key] in
theorem idleTurn_idle_of_eq (K : Finset Key) (σ : WalkState Key)
    (h : σ.lastFinished = finished K σ) : (idleTurn K σ).idle = σ.idle + 1 := by
  simp only [idleTurn, if_pos h]

omit [DecidableEq Key] in
theorem idleTurn_lastFinished_of_eq (K : Finset Key) (σ : WalkState Key)
    (h : σ.lastFinished = finished K σ) : (idleTurn K σ).lastFinished = σ.lastFinished := by
  simp only [idleTurn, if_pos h]

omit [DecidableEq Key] in
theorem idleTurn_idle_of_ne (K : Finset Key) (σ : WalkState Key)
    (h : σ.lastFinished ≠ finished K σ) : (idleTurn K σ).idle = 0 := by
  simp only [idleTurn, if_neg h]

omit [DecidableEq Key] in
theorem idleTurn_lastFinished_of_ne (K : Finset Key) (σ : WalkState Key)
    (h : σ.lastFinished ≠ finished K σ) : (idleTurn K σ).lastFinished = finished K σ := by
  simp only [idleTurn, if_neg h]

omit [DecidableEq Key] in
/-- **An idle round spends the budget.** Both branches of the block leg's rule
    (`lfs_block_requester.rs:253-266`) lower the budget term: the spending branch by one round, and
    the refilling branch by one round of credit — which is the round the raise it rewards has just
    paid for. -/
theorem a_single_idle_round_spends_budget (K : Finset Key) (σ : WalkState Key)
    (hidle : σ.idle < MAX_IDLE_ROUNDS) : budget K (idleRound K σ) < budget K σ := by
  have hfin : finished K (idleRound K σ) = finished K σ := by
    unfold idleRound
    rw [finished_requestTurn, finished_idleTurn]
  by_cases hA : σ.lastFinished = finished K σ
  · have hlast : (idleRound K σ).lastFinished = finished K (idleRound K σ) := by
      rw [idleRound_lastFinished, idleTurn_lastFinished_of_eq K σ hA, hA, hfin]
    have hidle' : (idleRound K σ).idle = σ.idle + 1 := by
      rw [idleRound_idle, idleTurn_idle_of_eq K σ hA]
    have hbσ : budget K (idleRound K σ) = MAX_IDLE_ROUNDS - (σ.idle + 1) := by
      unfold budget
      rw [pending_eq_zero K _ hlast, hidle', Nat.mul_zero, Nat.add_zero]
    have hb : budget K σ = MAX_IDLE_ROUNDS - σ.idle := by
      unfold budget
      rw [pending_eq_zero K σ hA, Nat.mul_zero, Nat.add_zero]
    rw [hbσ, hb]
    exact Nat.sub_lt_sub_left hidle (Nat.lt_succ_self σ.idle)
  · have hlast : (idleRound K σ).lastFinished = finished K (idleRound K σ) := by
      rw [idleRound_lastFinished, idleTurn_lastFinished_of_ne K σ hA, hfin]
    have hidle' : (idleRound K σ).idle = 0 := by
      rw [idleRound_idle, idleTurn_idle_of_ne K σ hA]
    have hbσ : budget K (idleRound K σ) = MAX_IDLE_ROUNDS - 0 := by
      unfold budget
      rw [pending_eq_zero K _ hlast, hidle', Nat.mul_zero, Nat.add_zero]
    have hb : budget K σ = (MAX_IDLE_ROUNDS - σ.idle) + (MAX_IDLE_ROUNDS + 1) := by
      unfold budget
      rw [pending_eq_one K σ hA, Nat.mul_one]
    rw [hbσ, hb, Nat.sub_zero]
    exact Nat.lt_of_lt_of_le (Nat.lt_succ_self _) (Nat.le_add_left _ _)

omit [DecidableEq Key] in
theorem fuel_idleRound_lt (K : Finset Key) (σ : WalkState Key)
    (hidle : σ.idle < MAX_IDLE_ROUNDS) : fuel K (idleRound K σ) < fuel K σ := by
  have hfin : finished K (idleRound K σ) = finished K σ := by
    unfold idleRound
    rw [finished_requestTurn, finished_idleTurn]
  have hb := a_single_idle_round_spends_budget K σ hidle
  calc fuel K (idleRound K σ)
      = (K.card - finished K (idleRound K σ)) * (2 * MAX_IDLE_ROUNDS + 2) +
          budget K (idleRound K σ) := by simp only [fuel]
    _ = (K.card - finished K σ) * (2 * MAX_IDLE_ROUNDS + 2) +
          budget K (idleRound K σ) := by rw [hfin]
    _ < (K.card - finished K σ) * (2 * MAX_IDLE_ROUNDS + 2) + budget K σ :=
        Nat.add_lt_add_left hb _
    _ = fuel K σ := by simp only [fuel]

/-- **A progress turn pays its weight and buys back its credit.** One key of `K` is finished (the
    first term drops by the weight) and the refill the round earns costs at most one round — strictly
    less than the weight, which is what the constant is chosen for. -/
theorem fuel_progressRound_lt (K : Finset Key) (σ : WalkState Key) (k last : Key)
    (hk : σ.d k = some Status.requested) (hkK : k ∈ K) :
    fuel K (progressRound k last σ) < fuel K σ := by
  have hfin : finished K (progressRound k last σ) = finished K σ + 1 := by
    unfold progressRound
    rw [finished_requestTurn, finished_answerTurn K σ k last hk hkK]
  have hcard : finished K σ + 1 ≤ K.card := by
    rw [← hfin]
    exact finished_le_card K _
  have hsub : (K.card - (finished K σ + 1)) + 1 = K.card - finished K σ := by
    have hpos : 0 < K.card - finished K σ := Nat.sub_pos_of_lt (Nat.lt_of_succ_le hcard)
    rw [Nat.sub_succ, Nat.pred_eq_sub_one]
    exact Nat.sub_add_cancel (Nat.succ_le_of_lt hpos)
  have hb := budget_lt_of_idle_eq K (progressRound_idle k last σ)
  calc fuel K (progressRound k last σ)
      = (K.card - (finished K σ + 1)) * (2 * MAX_IDLE_ROUNDS + 2) +
          budget K (progressRound k last σ) := by simp only [fuel, hfin]
    _ < (K.card - (finished K σ + 1)) * (2 * MAX_IDLE_ROUNDS + 2) +
          ((2 * MAX_IDLE_ROUNDS + 2) + budget K σ) := Nat.add_lt_add_left hb _
    _ = ((K.card - (finished K σ + 1)) + 1) * (2 * MAX_IDLE_ROUNDS + 2) + budget K σ := by
        rw [Nat.add_mul, Nat.one_mul, ← Nat.add_assoc]
    _ = (K.card - finished K σ) * (2 * MAX_IDLE_ROUNDS + 2) + budget K σ := by rw [hsub]
    _ = fuel K σ := by simp only [fuel]

/-- **Every turn of the fixed loop spends fuel** — one case per constructor, and the two bounds above
    are the work. This is what makes the give-up rule a bound rather than a hope. -/
theorem a_fixed_turn_spends_fuel (K : Finset Key) {σ σ' : WalkState Key}
    (h : FixedStep K σ σ') : fuel K σ' < fuel K σ := by
  cases h with
  | idle σ hidle => exact fuel_idleRound_lt K σ hidle
  | progress k last σ hk hkK hidle => exact fuel_progressRound_lt K σ k last hk hkK

/-- **A run cannot be longer than the fuel it started with** — the bound in fuel form, from which the
    two statements below follow. The induction is over the run, with the head generalised so that the
    tail's bound is the same statement at the next state. -/
theorem run_length_chain_le_fuel (K : Finset Key) :
    ∀ (rest : List (WalkState Key)) (σ : WalkState Key),
      List.Chain (FixedStep K) σ rest → rest.length ≤ fuel K σ := by
  intro rest
  induction rest with
  | nil => intro σ _; simp
  | cons b l ih =>
      intro σ hchain
      cases hchain with
      | cons hstep htail =>
          have hlt : fuel K b < fuel K σ := a_fixed_turn_spends_fuel K hstep
          exact Nat.le_trans (Nat.succ_le_succ (ih b htail)) (Nat.succ_le_of_lt hlt)

theorem run_length_le_fuel (K : Finset Key) (σ : WalkState Key) (rest : List (WalkState Key))
    (hrun : (fixed K).Run (σ :: rest)) : rest.length ≤ fuel K σ :=
  run_length_chain_le_fuel K rest σ hrun

omit [DecidableEq Key] in
theorem fuel_le (K : Finset Key) (σ : WalkState Key) :
    fuel K σ ≤ K.card * (2 * MAX_IDLE_ROUNDS + 2) +
      (MAX_IDLE_ROUNDS + MAX_IDLE_ROUNDS + 1) := by
  have h1 : (K.card - finished K σ) * (2 * MAX_IDLE_ROUNDS + 2) ≤
      K.card * (2 * MAX_IDLE_ROUNDS + 2) :=
    Nat.mul_le_mul_right (2 * MAX_IDLE_ROUNDS + 2) (Nat.sub_le _ _)
  have h2 : budget K σ ≤ MAX_IDLE_ROUNDS + MAX_IDLE_ROUNDS + 1 := by
    calc budget K σ
        ≤ (MAX_IDLE_ROUNDS - σ.idle) + (MAX_IDLE_ROUNDS + 1) := budget_le K σ
      _ ≤ MAX_IDLE_ROUNDS + (MAX_IDLE_ROUNDS + 1) := Nat.add_le_add (Nat.sub_le _ _) le_rfl
      _ = MAX_IDLE_ROUNDS + MAX_IDLE_ROUNDS + 1 := (Nat.add_assoc _ _ _).symm
  unfold fuel
  exact Nat.add_le_add h1 h2

/-- **The walk with the give-up rule cannot run for ever.** Every run is finite, so the leg returns —
    which is what `node_syncing.rs:461`'s `join!` needs in order for the attempt itself to return, and
    therefore what lets `attempts` (`:372`) and `MAX_SYNC_ATTEMPTS` (`:380`) mean anything. Fuel is a
    `Nat`, so an infinite run is impossible: it would have to descend for ever from a finite start. -/
theorem the_fixed_walk_cannot_run_for_ever (K : Finset Key) :
    ¬ ∃ trace : Nat → WalkState Key, ∀ n, FixedStep K (trace n) (trace (n + 1)) := by
  rintro ⟨trace, htrace⟩
  have hdec : ∀ n, fuel K (trace n) + n ≤ fuel K (trace 0) := by
    intro n
    induction n with
    | zero => simp
    | succ m ih =>
        have hstep : fuel K (trace (m + 1)) < fuel K (trace m) :=
          a_fixed_turn_spends_fuel K (htrace m)
        have h1 : fuel K (trace (m + 1)) + 1 ≤ fuel K (trace m) := Nat.succ_le_of_lt hstep
        calc fuel K (trace (m + 1)) + (m + 1)
            = (fuel K (trace (m + 1)) + 1) + m := by rw [Nat.add_assoc, Nat.add_comm 1 m]
          _ ≤ fuel K (trace m) + m := Nat.add_le_add_right h1 m
          _ ≤ fuel K (trace 0) := ih
  have hv := hdec (fuel K (trace 0) + 1)
  have hle : fuel K (trace 0) + 1 ≤
      fuel K (trace (fuel K (trace 0) + 1)) + (fuel K (trace 0) + 1) :=
    Nat.le_add_left _ _
  exact absurd (Nat.le_trans hle hv) (Nat.not_succ_le_self (fuel K (trace 0)))

/-- **The give-up rule bounds the walk's pace** — the positive half of the register row, in the form
    `Rchain/Progress.lean` gives a repair (`Paced`) and `lfs_block_requester.rs:36-52` argues for the
    block leg's own constant. The bound is a fuel that no run can exceed, so a run longer than it
    cannot have held the measure still. -/
theorem the_page_walk_with_the_give_up_rule_is_paced (K : Finset Key) :
    System.Paced (fixed K) (fun σ => finished K σ)
      (K.card * (2 * MAX_IDLE_ROUNDS + 2) + (MAX_IDLE_ROUNDS + MAX_IDLE_ROUNDS + 1)) := by
  intro σ rest hrun hlen
  exact absurd
    (Nat.lt_of_lt_of_le hlen
      (Nat.le_trans (run_length_le_fuel K σ rest hrun) (fuel_le K σ)))
    (Nat.lt_irrefl _)

/-! ### The goal, in the two halves that are honestly unequal

`isFinished` is `LfsTupleSpaceState::is_finished` (`:100-102`) and `finished` is the measure the
give-up rule counts; the bridge between them is the first theorem below. It runs in one direction
only, and the second theorem says why: the Rust reads an empty map as finished, so a state can satisfy
the goal with the measure at zero. -/

omit [DecidableEq Key] in
/-- **A maximal measure is the goal.** If every key of `K` is `Done`, then every key the map holds is
    `Done` — the bridge a termination proof consumes, and the reason the measure is the right one to
    count. -/
theorem all_keys_done_of_full_measure (K : Finset Key) (σ : WalkState Key)
    (hK : ∀ k, σ.d k ≠ none → k ∈ K) (h : finished K σ = K.card) : isFinished σ := by
  have hdone : ∀ x ∈ K, σ.d x = some Status.done := by
    intro x hx
    by_contra hne
    have hssub : doneKeys K σ ⊂ K := by
      unfold doneKeys
      exact Finset.filter_ssubset.mpr ⟨x, hx, hne⟩
    have hlt : (doneKeys K σ).card < K.card := Finset.card_lt_card hssub
    unfold finished at h
    rw [h] at hlt
    exact absurd hlt (Nat.lt_irrefl _)
  intro k s hk
  have hkK : k ∈ K := hK k (by rw [hk]; exact fun h => Option.noConfusion h)
  have := hdone k hkK
  rw [hk] at this
  exact Option.some.inj this

omit [DecidableEq Key] in
/-- **The goal is not the measure, and the Rust says so**: `is_finished` is `all` over the values the
    map holds (`:100-102`), so the empty map satisfies it while the measure reads zero. Any proof of
    termination that read `finished` as `isFinished` would be claiming a restored state for a walk
    that restored nothing — the same mistake `node_syncing.rs:475-481` refuses for the block leg. -/
theorem the_goal_does_not_imply_a_full_measure (K : Finset Key) (hk : K.Nonempty) :
    ∃ σ : WalkState Key, isFinished σ ∧ finished K σ ≠ K.card := by
  refine ⟨⟨fun _ => none, 0, 0⟩, fun k s hk' => absurd hk' (by simp), ?_⟩
  have h0 : finished K (⟨fun _ => none, 0, 0⟩ : WalkState Key) = 0 := by
    simp [finished, doneKeys]
  intro hcon
  rw [h0] at hcon
  exact absurd hcon.symm (Nat.pos_iff_ne_zero.mp (Finset.card_pos.mpr hk))

/-- **What the register would like, and what is owed.** The claim is that under the give-up rule the
    walk from a set of requestable roots reaches the goal. It is stated here as a `Prop` — a claim
    with no proof, and deliberately not an axiom — because the step that would prove it is missing
    from the Rust, not from this file.

    **The hypothesis it needs, exactly.** The rule this file models bounds the *idle* rounds, so what
    it guarantees is that the walk exits — proved above as `the_page_walk_with_the_give_up_rule_is_paced`
    and `the_fixed_walk_cannot_run_for_ever`. Reaching `is_finished` rather than the exhausted-budget dead
    end needs one thing more, and it is a property of the *responder*: that every key the walk asks
    for is answered within `MAX_IDLE_ROUNDS` idle rounds of being asked. That is not true of
    `request_tuple_space_roots`: a responder may answer a page that leaves the walk waiting, and no
    node-side rule can make an answer arrive. The node already owns the right treatment of that case
    one level up — the retry at `node_syncing.rs:333-404`, whose `attempts` bound exists only because
    a failing leg *returns* — so the fix this row wants is not a proof here but the block leg's
    counter at `lfs_tuple_space_requester.rs:366-391`, and the failing return it makes possible at
    `:387-389`. -/
def ReachesTheGoal (K : Finset Key) (σ₀ : WalkState Key) : Prop :=
  ∃ σ' : WalkState Key, (fixed K).Reach σ₀ σ' ∧ isFinished σ'

end

end Walk
end Sync
end Rchain
