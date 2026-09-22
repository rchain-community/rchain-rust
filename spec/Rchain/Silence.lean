import Rchain.Rho
import Rchain.Match

/-!
# Law 38 — silence is specified

A receive whose pattern does not match the datum on its channel performs **no step and reports no
error**. That is the whole reason the defects of AUDIT C9-C26 were expensive: an unmatched `for` is not
an error, so a broken pattern reads as a client bug, and the deploy still reports success.

The model could not state it. `Rho.lean`'s `Reduce` contracts *any* send with *any* receive on the
same channel — its `ReceiveBind` carries no pattern at all. This module adds the pattern and makes the
contract rule require a match, so silence is a consequence of the rule:

- `receiveParP` — a receive with a pattern. Note its bind is `(patterns := [pattern], source := chan)`;
  `Rho.lean`'s `receivePar` has the *body* in the `source` slot, so the base-sort receive never says
  which channel it listens on. Left alone here (its theorems depend on it) and recorded instead.
- `ReduceP` — pattern-aware reduction: `comm` carries the match as a hypothesis; everything else is
  congruence under `|`.
- `takesStep` — the same question, computed: does this flat `Par` hold a send and a receive on one
  channel whose pattern matches the send's datum? It is a plain structural search (the model's `Par` is
  flat, so a redex is a pair of entries, not a subtree), which is what lets the corpus `decide` against
  it.

`takesStep_iff_reduces` is the tie between the two, and it is **owed** — named here rather than
assumed, and checked behaviourally meanwhile by `spec/conformance/silence.tsv`'s consumer, which runs
each case through the node.
-/

namespace Rchain

/-- A receive with a *pattern* on `chan`: the datum must match `pattern` for it to fire. -/
def receiveParP (chan pattern body : Par) : Par :=
  Par.mk [] [Receive.mk [ReceiveBind.mk [pattern] chan 1] body false 1] [] [] [] [] [] []

/-- Pattern-aware reduction. The contract rule requires `spatialMatches data pattern`, so "no match,
no step" is the rule rather than a remark about the implementation. -/
inductive ReduceP : Par → Par → Prop where
  | comm {chan pattern data body : Par} (h : spatialMatches data pattern) :
      ReduceP (parMerge (sendPar chan [data]) (receiveParP chan pattern body)) body
  | parLeft {p p' q : Par} : ReduceP p p' → ReduceP (parMerge p q) (parMerge p' q)
  | parRight {p q q' : Par} : ReduceP q q' → ReduceP (parMerge p q) (parMerge p q')

/-- A `Par` as a *string* channel, when it is one. The corpus's channels are string channels
(`@"store"`), and comparing them by name keeps the search below structural. -/
def stringChan (p : Par) : Option String :=
  match p with
  | .mk [] [] [] [.ground (.str l)] [] [] [] [] => some (l.map Char.ofNat).asString
  | _ => none

mutual
  /-- Does any send/receive pair in this `Par` form a contract step? -/
  def takesStep (p : Par) : Bool := stepsInSends p.sends p.receives

  /-- Each send against every receive. -/
  def stepsInSends : List Send → List Receive → Bool
    | [], _ => false
    | s :: ss, receives => stepsInReceives s receives || stepsInSends ss receives

  /-- One send against every receive. -/
  def stepsInReceives (s : Send) : List Receive → Bool
    | [] => false
    | r :: rs => stepsInBinds s r.binds || stepsInReceives s rs

  /-- One send against every bind of a receive: the same channel, and a pattern that matches the
  datum. A bind whose pattern is not a single pattern, or a channel that is not a string channel,
  fails closed — the boundary note in `Rchain/Match.lean` applies. -/
  def stepsInBinds (s : Send) : List ReceiveBind → Bool
    | [] => false
    | b :: bs =>
      (match b.patterns, s.data, stringChan s.chan, stringChan b.source with
       | [pat], [d], some a, some c => a == c && spatialMatch d pat
       | _, _, _, _ => false)
      || stepsInBinds s bs
end

/-- The tie between the relation and the computation: they answer the same question. **Owed** — the
search above is what the corpus decides against, and the node's behaviour is what the corpus checks,
so a disagreement of either kind is reported rather than passing. -/
axiom takesStep_iff_reduces (p : Par) : takesStep p = true ↔ ∃ q', ReduceP p q'

end Rchain
