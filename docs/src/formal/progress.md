# Progress: the shapes of non-progress

**Read this before proposing a fix to a liveness-shaped defect.** The vocabulary here is shared by the
reducer and the protocol, its definitions live in `spec/Rchain/Progress.lean`, and its register row is
`51a`–`51c` in [`spec/LAWS.md`](../../../spec/LAWS.md).

## Why this page exists

Between 2026-09-28 and 2026-09-29 the same *kind* of defect arrived four times under four names:

- **C174** — the fringe gate's partition required a message from every bonded validator, so one silent
  validator made it unsatisfiable and 80 % of the stake could not finalise;
- **C173** — one attributable failure in a bonded validator's block estranged a node from the chain
  permanently;
- **C170** — the attestation guard counted a validator that had *ever* spoken;
- **C171** — an all-live net attests on every remote block: 126 blocks in three minutes, finality
  unmoved.

Each was argued in prose, fixed alone, and filed as a one-off — and the arguments did not accumulate. Two
readings of one paragraph disagreed about a fix (a plan proposed shrinking finality's quorum denominator,
which would have silently destroyed safety) and nothing in the tree could settle it. This page is the
settlement: a shared language with a **diagnosis**, a **law**, and a **named falsifier** for each shape.

## The two axes

**The shape** says why the obligation cannot be met, and therefore what kind of repair applies. **The
cause** says where the fix goes. Reading a symptom on both axes is what turns "the chain is stuck" into
"change this requirement" or "bound this rate".

| symptom an operator sees | shape | cause | law | the falsifier to run | the fix class |
|---|---|---|---|---|---|
| finality stops although more than two thirds of the stake is up | `Void` — nothing satisfies the requirement | `Historic` | 51a/51b | the partition filter's `Void` proof on the silent-validator fixture | **drop the requirement nothing can satisfy** (shrink the *partition*, never the denominator — see below) |
| a node attests although a third of the stake is silent | — | `Historic` | 51b | `a_silent_validators_stale_message_does_not_carry_the_quorum` | read the predicate off the **current view**, not the history |
| the chain grows and the fringe does not | `Drift` | `Historic` | 51a/51b | `the_two_cycle_drifts` + the devnet measurement | **bound the rate** — and fix the predicate that let it run |
| one bad block, and the node refuses every block above it, forever | `Terminal` | — | 51a | the estrangement fixture (AUDIT C173) | give the refused state an **inverse** (a restoring rule) |
| a block is in the index and not in the store | `Terminal` | `Split` | 51a/51b | the index/store ordering tests | make both readers ask **one** side, or order the writes |
| **two groups finalise different histories** | safety, not liveness | — | 52a | `the_live_denominator_lets_two_sides_finalise` | **never** shrink the denominator |
| an enabled step that no schedule takes | starvation | — | 51a (`open`) | — (the evidence is `reduce_not_deterministic`) | the schedule is the scheduler's |

The row that matters most is the second-to-last, because it is the one this vocabulary exists to refuse:
**the partition may shrink; the denominator may not.** A quorum measured against the speaking subset is
reached by *any* self-consistent group — `3·F > 2·L` with `F ≤ L` over the live set is unconditional — so
under a network partition each side finalises its own view and safety is gone. Law 52a carries the
arithmetic (`supermajorities_overlap`) and the two-sided counterexample.

## The hypotheses, and what each one is worth

Every liveness statement in this layer carries a hypothesis, because a liveness claim without one is the
Law 20 trap: `law20_deadlock_freedom` was an *axiom* until it was deleted as unprovable as stated, its
corrected form carrying the finiteness the real system has. Naming the hypothesis is what makes "which
assumption would this fix break?" a question with an answer.

| hypothesis | what provides it | what violates it | which finding was that |
|---|---|---|---|
| `Participation` | every bonded validator speaks within the window | one that stops, or cannot keep up | C174 (and #105's 91 % survivor) |
| `Delivery` | the network carries a message to every honest node | a partition, or a peer that never answers | #105's state-hash divergence (nodes disagreed; the transfer was never the problem) |
| `StalenessBound` | the window (`LIVENESS_WINDOW`) makes "current" a bounded claim | reading a map that keeps history | C170 |
| `Finiteness` | the term's depth bounds its pending set (Law 20's own note) | an unbounded path set, where the order is not well-founded | the deleted `law20_deadlock_freedom` |
| `Fair` | — **named and not modelled** | — | — (see below) |

**`Fair` is named and not modelled, and that is a theorem of this tree rather than an oversight.** "An
enabled step that no schedule takes" is unstatable over a transition *relation* — a relation does not say
when a step is taken — and `reduce_not_deterministic` (`spec/Rchain/Concurrent.lean`) is the repo's own
proof that the flat calculus fixes no schedule at all. So starvation is registered `open` with that
reason, and the day a scheduler model exists, that row is where the shape is stated.

## The vocabulary

All of it is in `spec/Rchain/Progress.lean`, generic over a `System` — a state type, a step relation, and
a **decidable** "is a step available now" predicate tied to the relation by the structure's own field.

| name | one line |
|---|---|
| `System` | a step relation plus `enabled`, with `enabled σ = true ↔ ∃ σ', Step σ σ'` **as a field** |
| `Reach` | the reflexive-transitive closure of `Step` (the primitive `Concurrent.lean` already uses) |
| `Stuck` / `Silent` | no step leaves this state / no continuation satisfies the goal |
| `Void` | no state satisfies the requirement |
| `Terminal` | a satisfying state was left **and** no continuation restores it |
| `Absorbing` | once lost, never regained — the second obligation of `Terminal`, and the word 2PC already uses |
| `Run` / `Drift` / `Paced` | a finite trace / a run of `n` steps whose measure never moves / the repair shape that forbids it |
| `Historic` | a predicate that disagrees with itself on two histories agreeing on the current view |
| `ReadsTheView` | what a liveness predicate must have — `Historic`'s complement |
| `Split` | two readers of one state that a reachable step makes disagree |

### The calculus correspondence, and how each row is kept honest

The vocabulary is **shared with the ρ-calculus rather than parallel to it**, and no row of this table may
claim agreement without saying which kind it is — the register's `status` discipline applied to prose:

| calculus | here | kind |
|---|---|---|
| `Reduce` / `ReduceP` as a step relation | a `System`'s `Step` | **instance** (`silenceSystem`) |
| `Relation.ReflTransGen Reduce` | `Reach` | **definitional** — `Reach` *is* that closure |
| `takesStep p = true ↔ ∃ q', ReduceP p q'` | the `enabled_iff` field | **theorem**, supplied per instance |
| a `for` that matches nothing: no step, **no error** (Law 38/40) | `Void`: no rule fires and nothing errors | **theorem**, one layer up — C174's block was never refused, the gate simply never fired |
| a **join** part-filled | — | **not modelled, because** `spec/Rchain/Silence.lean` records that boundary |
| an inactivity leak | — | **not modelled, because** it needs a state change (burning silent stake): #24, #39 |

The layers differ in exactly one way, and it is the distinction this vocabulary exists to keep sharp: in
the **closed** calculus nothing outside the term can add a send, so a lone receive is silent *forever* —
while at the **protocol** level delivery is itself a step, so the same shape is a *wait*, satisfiable by
an event, and only becomes `Terminal` when a rule takes the event away. A blocked wait and a permanent
stall must not share a name.

## The worked instances

- **C174, a `Void` whose cause is `Historic`** — the partition was the whole bonded set, so a validator
  that never speaks has no message to be seen by: the requirement is *empty*, not late. The fix dropped
  the requirement (the partition is the live weight set) and left the denominator alone. Law 52b.
- **C173, a `Terminal`** — the refusal rules have no inverse: the height maximum skips failed
  justifications, and a block justifying a failed bonded sender is refused before any other rule runs, so
  the refusal is `Absorbing` and the goal (a block above it) is never reachable again. The fix class is a
  restoring step; the register's row is a *guard on that fix*, because a rule that restores reddens the
  theorem on purpose.
- **C171, a `Drift` whose cause is `Historic`** — the guard read "has ever spoken", so its suppression
  never fired while the quorum *was* reachable, and each attestation was itself a remote block for the
  peers. Two obligations: the predicate (C170) and a pace condition, which is owed as `Paced`.
- **C172, a `Terminal` whose cause is a `Split`** — the in-memory index and the persisted store answered
  different questions about one block, and the disagreement turned an unresolvable justification into a
  dropped block with nothing to re-queue it.
- **C170, the `Historic` predicate itself** — the stake counted was the stake of validators whose *last
  message anywhere in the map* matched, not of validators speaking now: 300 of 300 where the truth was
  200 of 300.

## How to use this page

1. **Classify the symptom** in the table above — shape and cause.
2. **Read the law's row** (`51a`–`51c`, and the clause it is an instance of) in `spec/LAWS.md`, and the
   falsifier it names.
3. **Run the falsifier** before writing a fix. If it is red today, the defect is the one it pins; if it is
   green, the shape is not the one you think it is.
4. **Then pick the fix class** — and if the fix is to shrink a requirement, check the `Void` row's
   warning about the denominator first.

**What this page does not claim.** That the classification is complete (starvation is named and *not*
modelled, and the shape is registered `open`); that the protocol's rules are modelled in full (the
inactivity leak is not); or that a `System` is the only way to state these properties — it is the way this
tree states them, with a decidable predicate so the conformance corpus can reach the verdicts.
