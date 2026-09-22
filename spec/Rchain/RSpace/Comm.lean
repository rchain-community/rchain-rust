import Rchain.Cmp
import Rchain.Crypto.Spec

/-!
# Laws 8 & 11 — deterministic COMM, replay determinism

A COMM event is content-addressed by the sorted produce refs (Law 8); replay recomputes COMM events and
must agree with the recorded trace (Law 11). The Scala oracle is `trace/Event.scala:35-39` +
`ReplayRSpace.scala:68-71`; the Rust realization is `rspace/src/trace/event.rs` (the event types and
their hashes) + `rspace/src/space_matcher.rs` (sorted-first selection) + `rspace/src/replay_rspace.rs`.

## What the consolidation pass changed here

This file carried four axioms: `produceRefs : Comm → List Nat`, `comm_content_addressed`,
`replayEvents : Trace → List Comm`, and `replay_comm_subset`. Two of them were claims about *undefined*
functions, so they said nothing —

* `produceRefs` is a **definition** now, mirroring `Comm::apply`'s sort (`event.rs:150-173`), and
  `comm_content_addressed` is a **theorem**: a COMM's identity is canonical because the refs are sorted,
  which is Law 1's canonicalization applied to the event log.
* `replayEvents`/`replay_comm_subset` are **deleted**, and Law 11's row is `vacuous` with the reason
  written down. The law is real in the code — replay recomputes the log with the same matcher and checks
  membership, and it checks the *reverse* too ("Unused COMM event", `replay_rspace.rs:580-590`) — but in
  this model "recompute" and "record" would be the *same function*, so the subset claim would be `rfl`.
  What would give it content is a model of the recorded store, so that the two are different functions
  that must agree; the proof is owed to that modelling step, not to a tactic.
-/

namespace Rchain

/-- A produce as the Rust stores it: its channel's hash, its own content hash, and its persistence flag.
    Equality and order in the code are by content hash (`Produce::apply` gives the hash at
    `rspace/src/trace/event.rs:24-49`, `Ord` is by it at `:67-71`), which is why the ref list below can
    be sorted at all. -/
structure Produce where
  chanHash : Hash
  hash : Hash
  persist : Bool

/-- A consume's identity: the hashes of the channels it joins and its continuation's hash
    (`Consume::apply`, `rspace/src/trace/event.rs:82-104`). -/
structure Consume where
  channels : List Hash
  continuation : Hash

/-- A COMM: a consume matched against the produces it consumed, **in arrival order** — that order is
    exactly what the canonical identity below removes (`Comm::apply`, `event.rs:150-173`). -/
structure Comm where
  consume : Consume
  produces : List Produce

/-- The Rust's produce order: by `(channels_hash, hash, persistent)` (`event.rs:165`). -/
def produceComparator : Comparator (Hash × Hash × Bool) :=
  Comparator.cmpPair (Comparator.listComparator (Comparator.linearOrderComparator Byte))
    (Comparator.cmpPair (Comparator.listComparator (Comparator.linearOrderComparator Byte))
      (Comparator.linearOrderComparator Bool))

/-- The produce refs in canonical order: sorted by the same key the Rust sorts by
    (`produce_refs.sort_by_key`, `event.rs:165`). -/
def produceRefs (c : Comm) : List (Hash × Hash × Bool) :=
  Comparator.sortList produceComparator
    (c.produces.map (fun p => (p.chanHash, p.hash, p.persist)))

/-- A COMM's canonical identity: its consume, and the **sorted** refs. -/
def commId (c : Comm) : Consume × List (Hash × Hash × Bool) := (c.consume, produceRefs c)

/-- **Law 8** — a COMM's identity is canonical: two comms whose produces differ only in *order* are the
    same event, because the identity sorts them. This is Law 1's canonicalization applied to the event
    log, and it is what makes the recorded trace reproducible — a replay that matched the same produces
    in a different order produces the same event, which is the fact Law 11's row then leans on. -/
theorem comm_content_addressed (a b : Comm) (hc : a.consume = b.consume)
    (h : List.Perm a.produces b.produces) : commId a = commId b := by
  simp only [commId]
  rw [hc]
  congr 1
  simp only [produceRefs]
  exact Comparator.sortList_perm produceComparator
    (List.Perm.map (fun p => (p.chanHash, p.hash, p.persist)) h)

/-- A recorded trace of COMM events: the log `ReplayRSpace` replays against. -/
structure Trace where
  events : List Comm

end Rchain
