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
**Law 40 lives one clause of this rule.** `stepsInBinds` reads the *arity*: a receive accepts a send
only when it has as many patterns as the send has data (`receiveParPs` is that receive, and law 40's
cases in the corpus vary the arity). That is where C22 item 2 lived — `MCAwrite!("Chat", *C_Chat)`
against a three-argument `write` — and the clause is why "a call at the wrong arity does nothing" is a
consequence of the rule rather than a remark about the implementation.
-/

namespace Rchain

/-- A receive with a *pattern* on `chan`: the datum must match `pattern` for it to fire. -/
def receiveParP (chan pattern body : Par) : Par :=
  Par.mk [] [Receive.mk [ReceiveBind.mk [pattern] chan 1] body false 1] [] [] [] [] [] []

/-- A receive with **several** patterns on `chan` — the shape a contract call is matched against, so
that law 40's question (*at which arities does a call have an accepting receive?*) is about this
constructor. Replicated, because a contract's receive is. -/
def receiveParPs (chan : Par) (patterns : List Par) (body : Par) : Par :=
  Par.mk [] [Receive.mk [ReceiveBind.mk patterns chan patterns.length] body true 1]
    [] [] [] [] [] []

/-- Pattern-aware reduction. The contract rule requires `spatialMatches data pattern`, so "no match,
no step" is the rule rather than a remark about the implementation.

**`commPs` is law 40's clause, and it was missing.** The docstring above says the arity lives in this
rule — but as first written the rule could contract only a send of *one* datum, against `receiveParP`
(one pattern), while `stepsInBinds` (the computation the corpus decides against) accepts a bind with as
many patterns as the send has data. So the relation could not express what the computation accepted: a
two-argument call against a two-argument contract is a step by the search and had no derivation.
`commPs` is that derivation — the arity hypothesis and the pairwise match, exactly the clauses
`stepsInBinds` checks — and it is what makes law 40's question ("at which arities does a call have an
accepting receive?") a question about the *rule* rather than about an implementation detail.
AUDIT C40. -/
inductive ReduceP : Par → Par → Prop where
  | comm {chan pattern data body : Par} (h : spatialMatches data pattern) :
      ReduceP (parMerge (sendPar chan [data]) (receiveParP chan pattern body)) body
  | commPs {chan : Par} {patterns data : List Par} {body : Par}
      (harity : patterns.length = data.length)
      (hmatch : (patterns.zip data).all (fun pd => spatialMatch pd.2 pd.1) = true) :
      ReduceP (parMerge (sendPar chan data) (receiveParPs chan patterns body)) body
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

  /-- One send against every bind of a receive: the same channel, **as many patterns as data** (a
  call's arity is part of what a receive accepts — a call at the wrong arity matches no receive, and
  that is silence rather than an error, which is AUDIT C22 item 2: `MCAwrite!("Chat", *C_Chat)`
  against a three-argument `write`), and each pattern matching its own datum. A channel that is not a
  string channel fails closed — the boundary note in `Rchain/Match.lean` applies. -/
  def stepsInBinds (s : Send) : List ReceiveBind → Bool
    | [] => false
    | b :: bs =>
      (match stringChan s.chan, stringChan b.source with
       | some a, some c =>
         a == c && b.patterns.length == s.data.length
           && (b.patterns.zip s.data).all (fun pd => spatialMatch pd.2 pd.1)
       | _, _ => false)
      || stepsInBinds s bs
end

/-- Every channel the search will compare is a **string** channel: each send's channel and each bind's
source. This is the computation's domain, and it has to be stated because the two sides of
`takesStep_iff_reduces` are not defined on the same terms: `ReduceP.comm` fires on *any* channel, while
`stepsInBinds` compares channels with `stringChan` — a decidable `String` identity, which is what makes
the corpus's verdicts `decide`-able at all, since the model's `Par` has no `DecidableEq` and its
canonical comparator is a well-founded recursion that `decide` cannot unfold. -/
def allStringChans (p : Par) : Bool :=
  p.sends.all (fun s => (stringChan s.chan).isSome)
    && p.receives.all (fun r => r.binds.all (fun b => (stringChan b.source).isSome))

/-- The tie between the relation and the computation, **on the computation's domain**: for a `Par`
whose channels are all string channels, the search reports a step exactly when the relation has one.

**This replaced an axiom that was false** (AUDIT C40): it read `takesStep p = true ↔ ∃ q', ReduceP p q'`
for *every* `p`, and `chan = nilPar` refutes it — `ReduceP.comm` fires with `data = pattern = nilPar`
(`spatialMatches` accepts them, and the rule never looks at the channel), while `stepsInBinds` answers
`false` because `stringChan nilPar = none`. A false axiom is not an owed proof; it is the state C26
found law 5 in, where anything follows from it, so the statement is corrected here and the proof is
still owed — with the *sound* direction separated out below, which is the one the corpus leans on. -/
axiom takesStep_iff_reduces (p : Par) (h : allStringChans p = true) :
    takesStep p = true ↔ ∃ q', ReduceP p q'

/-- The **sound** direction on its own, and the one that matters for reading the corpus: when the
search reports a step, a step exists. Stated without the domain restriction because it does not need
it — a `true` from `stepsInBinds` already implies both channels were string channels. **Owed**, and
the half to prove first.

What the proof needs, written down here so the next attempt starts at the modelling step rather than
at the induction (measured, not guessed):

1. **The constructors must be parameterised by persistence.** `sendPar` fixes `persistent := false`
   and `receiveParP`/`receiveParPs` fix their own flags, but `stepsInBinds` ignores persistence
   entirely — so a `!!` send is a step by the search and has no derivation, exactly the shape of the
   `commPs` gap above. The constructors need the flags as arguments before the induction can even be
   stated.
2. **Three witness-extraction lemmas**, one per level of the search (`stepsInSends`,
   `stepsInReceives`, `stepsInBinds`): each turns a `true` into the send, the receive and the *bind*
   that produced it, with the four facts `stepsInBinds` checks (`Bool.and_eq_true` at each level, and
   `String`/`Nat` `==` to `=`).
3. **The embedding**: given a send at index `i` of `p.sends` and a receive at index `j` of
   `p.receives`, a right-nested chain of `parRight` peels the fields in order down to the redex
   (`parMerge` concatenates the flat fields, so a redex is a pair of entries at known positions, and
   the target is `parMerge body leftover`). -/
axiom takesStep_sound (p : Par) : takesStep p = true → ∃ q', ReduceP p q'

end Rchain
