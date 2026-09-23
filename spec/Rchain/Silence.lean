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

- `receiveParP` / `receiveParPs` — a receive with a pattern (one pattern, or as many as a call has
  arguments), each with its persistence flag. Its bind is `(patterns := […], source := chan)` — the
  channel in the slot that means *channel* (C27 fixed the base-sort `Rho.lean`'s receive, which had the
  body there instead).
- `ReduceP` — pattern-aware reduction: `comm` and `commPs` carry the match as a hypothesis; everything
  else is congruence under `|`. Both constructors take the send's and the receive's persistence flags,
  because a `!!` send and a replicated receive contract in the node exactly as their non-persistent
  forms do.
- `takesStep` — the same question, computed: does this flat `Par` hold a send and a receive on one
  channel whose pattern matches the send's datum? It is a plain structural search (the model's `Par` is
  flat, so a redex is a pair of entries, not a subtree), which is what lets the corpus `decide` against
  it.

`takesStep_iff_reduces` is the tie between the two, and it is **owed** — named here rather than
assumed, and checked behaviourally meanwhile by `spec/conformance/silence.tsv`'s consumer, which runs
each case through the node. Its first statement was **false** and is now scoped to the computation's
domain (`allStringChans`), with the unrestricted direction stated separately as `takesStep_sound`;
AUDIT C40 records both that and the missing arity clause.
**Law 40 lives one clause of this rule.** `stepsInBinds` reads the *arity*: a receive accepts a send
only when it has as many patterns as the send has data (`receiveParPs` is that receive, and law 40's
cases in the corpus vary the arity). That is where C22 item 2 lived — `MCAwrite!("Chat", *C_Chat)`
against a three-argument `write` — and the clause is why "a call at the wrong arity does nothing" is a
consequence of the rule rather than a remark about the implementation.
-/

namespace Rchain

/-- A send on `chan` carrying `data`, with the **persistence flag** (`!` vs `!!`). `Rho.lean`'s
`sendPar` fixes it to non-persistent; a `!!` send contracts in the node exactly as a `!` one does, so
the rule below has to be able to say so. -/
def sendParP (chan : Par) (data : List Par) (persistent : Bool) : Par :=
  Par.mk [Send.mk chan data persistent] [] [] [] [] [] [] []

/-- A receive with a *pattern* on `chan`: the datum must match `pattern` for it to fire. The flag is
whether the receive is replicated (`<=`) — a replicated receive re-arms, which is what makes a
`contract` callable twice. -/
def receiveParP (chan pattern body : Par) (persistent : Bool) : Par :=
  Par.mk [] [Receive.mk [ReceiveBind.mk [pattern] chan 1] body persistent 1] [] [] [] [] [] []

/-- A receive with **several** patterns on `chan` — the shape a contract call is matched against, so
that law 40's question (*at which arities does a call have an accepting receive?*) is about this
constructor. Its persistence flag is a parameter: a `contract`'s receive is replicated, and the flag is
what says so. -/
def receiveParPs (chan : Par) (patterns : List Par) (body : Par) (persistent : Bool) : Par :=
  Par.mk [] [Receive.mk [ReceiveBind.mk patterns chan patterns.length] body persistent 1]
    [] [] [] [] [] []

/-- A `Par` as a *string* channel, when it is one. The corpus's channels are string channels
(`@"store"`), and comparing them by name keeps the search below structural. -/
def stringChan (p : Par) : Option String :=
  match p with
  | .mk [] [] [] [.ground (.str l)] [] [] [] [] => some (l.map Char.ofNat).asString
  | _ => none

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
AUDIT C40.

**The rule's receive shape was too narrow as well** (AUDIT C45). Both constructors built their receive
through `receiveParP`/`receiveParPs`, which *fix* `freeCount := patterns.length` and `bindCount := 1` —
and the search reads neither, so the rule could not derive what the search accepted. It is not a
theoretical gap: the port's `free_count` is `count_no_wildcards` (`normalizer.rs:1295-1300`), so
`for (@a, @b <- c)` — a legitimate, ordinary receive — has `freeCount = 0` against `patterns.length = 2`,
and the node contracts it. The constructors now take both fields as parameters, which is what makes
`takesStep_sound` true; the builders stay as the canonical-shape shorthand the corpus uses. -/
inductive ReduceP : Par → Par → Prop where
  | comm {sendChan srcChan pattern data body : Par} {name : String}
      {sendPersistent recvPersistent : Bool} {freeCount bindCount : Nat}
      (hsend : stringChan sendChan = some name) (hsrc : stringChan srcChan = some name)
      (h : spatialMatches data pattern) :
      ReduceP (parMerge (sendParP sendChan [data] sendPersistent)
        (Par.mk [] [Receive.mk [ReceiveBind.mk [pattern] srcChan freeCount] body recvPersistent
          bindCount] [] [] [] [] [] [])) body
  | commPs {sendChan srcChan : Par} {name : String} {patterns data : List Par} {body : Par}
      {sendPersistent recvPersistent : Bool} {freeCount bindCount : Nat}
      (hsend : stringChan sendChan = some name) (hsrc : stringChan srcChan = some name)
      (harity : patterns.length = data.length)
      (hmatch : (patterns.zip data).all (fun pd => spatialMatch pd.2 pd.1) = true) :
      ReduceP (parMerge (sendParP sendChan data sendPersistent)
        (Par.mk [] [Receive.mk [ReceiveBind.mk patterns srcChan freeCount] body recvPersistent
          bindCount] [] [] [] [] [] [])) body
  | parLeft {p p' q : Par} : ReduceP p p' → ReduceP (parMerge p q) (parMerge p' q)
  | parRight {p q q' : Par} : ReduceP q q' → ReduceP (parMerge p q) (parMerge p q')

mutual
  /-- Does any send/receive pair in this `Par` form a contract step? -/
  def takesStep (p : Par) : Bool := stepsInSends p.sends p.receives

  /-- Each send against every receive. -/
  def stepsInSends : List Send → List Receive → Bool
    | [], _ => false
    | s :: ss, receives => stepsInReceives s receives || stepsInSends ss receives

  /-- One send against every receive — **and only against a single-bind receive**. A receive with two
  or more binds is a **join** (`for (x <- @"c"; y <- @"d")`), and the node fires a join only when *every*
  bound channel holds a matching datum; the rule below has no join clause at all, so a join is not a
  step in this model, and the search must not claim one either. It did: iterating the binds one at a
  time made a join with one channel filled a "step", which is the shape AUDIT C45 records and law 38's
  corpus case 13 is (the node is silent there, and silence is what the case declares). -/
  def stepsInReceives (s : Send) : List Receive → Bool
    | [] => false
    | r :: rs => (r.binds.length == 1 && stepsInBinds s r.binds) || stepsInReceives s rs

  /-- One send against the binds of a single-bind receive: the same channel, **as many patterns as
  data** (a call's arity is part of what a receive accepts — a call at the wrong arity matches no
  receive, and that is silence rather than an error, which is AUDIT C22 item 2: `MCAwrite!("Chat",
  *C_Chat)` against a three-argument `write`), and each pattern matching its own datum. A channel that
  is not a string channel fails closed — the boundary note in `Rchain/Match.lean` applies. The bind's
  `freeCount` and the receive's `bindCount` are **not** read: the node reads neither when it decides
  whether to contract (they are the linearity and join counts, checked by the normalizer and by the
  join rule), and the rule below no longer requires them either. -/
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

/-! ## The extraction lemmas, and the embedding

Three of them, one per level of the search, each turning a `true` into the send, the receive and the
*bind* that produced it, with the four facts `stepsInBinds` checks. Note what they do **not** carry:
persistence (the search never reads it, and the rule takes it as a parameter, so any flag at all
derives), and `freeCount`/`bindCount` (same — the rule no longer requires the canonical values). -/

theorem stepsInSends_sound : ∀ (ss : List Send) (rs : List Receive),
    stepsInSends ss rs = true → ∃ s ∈ ss, stepsInReceives s rs = true
  | [], _, h => by simp [stepsInSends] at h
  | s :: ss, rs, h => by
      rw [stepsInSends, Bool.or_eq_true] at h
      rcases h with h | h
      · exact ⟨s, by simp, h⟩
      · obtain ⟨s', hmem, h'⟩ := stepsInSends_sound ss rs h
        exact ⟨s', List.mem_cons_of_mem s hmem, h'⟩

theorem stepsInReceives_sound : ∀ (s : Send) (rs : List Receive),
    stepsInReceives s rs = true → ∃ r ∈ rs, r.binds.length = 1 ∧ stepsInBinds s r.binds = true
  | s, [], h => by simp [stepsInReceives] at h
  | s, r :: rs, h => by
      rw [stepsInReceives, Bool.or_eq_true] at h
      rcases h with h | h
      · rw [Bool.and_eq_true] at h
        exact ⟨r, by simp, by simpa using h.1, h.2⟩
      · obtain ⟨r', hmem, hl, h'⟩ := stepsInReceives_sound s rs h
        exact ⟨r', List.mem_cons_of_mem r hmem, hl, h'⟩

theorem stepsInBinds_sound : ∀ (s : Send) (bs : List ReceiveBind),
    stepsInBinds s bs = true →
      ∃ b ∈ bs, ∃ a, stringChan s.chan = some a ∧ stringChan b.source = some a
        ∧ b.patterns.length = s.data.length
        ∧ (b.patterns.zip s.data).all (fun pd => spatialMatch pd.2 pd.1) = true
  | s, [], h => by simp [stepsInBinds] at h
  | s, b :: bs, h => by
      rw [stepsInBinds, Bool.or_eq_true] at h
      rcases h with h | h
      · cases hc : stringChan s.chan with
        | none => simp [hc] at h
        | some a =>
          cases hc' : stringChan b.source with
          | none => simp [hc, hc'] at h
          | some c =>
            rw [hc, hc', Bool.and_eq_true, Bool.and_eq_true] at h
            obtain ⟨⟨hac, harity⟩, hmatch⟩ := h
            have hac' : a = c := by simpa using hac
            have harity' : b.patterns.length = s.data.length := by simpa using harity
            refine ⟨b, by simp, a, rfl, ?_, harity', hmatch⟩
            rw [hc', hac']
      · obtain ⟨b', hmem, a, h1, h2, h3, h4⟩ := stepsInBinds_sound s bs h
        exact ⟨b', List.mem_cons_of_mem b hmem, a, h1, h2, h3, h4⟩

/-- A single-element list is its own element: the step from `r.binds.length = 1` and `b ∈ r.binds` to
`r.binds = [b]`, which is the shape the rule's receive has. -/
theorem eq_singleton_of_length_one {α : Type} {l : List α} {a : α} (h : l.length = 1)
    (hmem : a ∈ l) : l = [a] := by
  obtain ⟨s, t, rfl⟩ := List.append_of_mem hmem
  simp only [List.length_append, List.length_cons, List.length_nil] at h
  have hs : s = [] := List.eq_nil_of_length_eq_zero (by omega)
  have ht : t = [] := List.eq_nil_of_length_eq_zero (by omega)
  simp [hs, ht]

/-- The **embedding**: a send at one position of `p.sends` and a receive at one position of
`p.receives` present `p` as three pars — the fields before the pair, the pair, and the fields after —
with `parMerge` doing the concatenation. This is the fact the note above called "a right-nested chain
of `parRight`": the redex is the middle par, the two contexts are the others, and the two congruence
constructors attach them. -/
theorem exists_redex_split (p : Par) {s : Send} {r : Receive} (hs : s ∈ p.sends)
    (hr : r ∈ p.receives) :
    ∃ (sL sR : List Send) (rL rR : List Receive),
      p.sends = sL ++ s :: sR ∧ p.receives = rL ++ r :: rR ∧
      p = parMerge (parMerge (Par.mk sL rL p.news p.exprs p.matches p.unforgeables p.bundles
            p.connectives)
          (Par.mk [s] [r] [] [] [] [] [] []))
        (Par.mk sR rR [] [] [] [] [] []) := by
  obtain ⟨sL, sR, hs'⟩ := List.append_of_mem hs
  obtain ⟨rL, rR, hr'⟩ := List.append_of_mem hr
  refine ⟨sL, sR, rL, rR, hs', hr', ?_⟩
  cases p
  simp only [Par.sends, Par.receives, Par.news, Par.exprs, Par.matches, Par.unforgeables,
    Par.bundles, Par.connectives] at hs' hr' ⊢
  subst hs'
  subst hr'
  simp [parMerge, List.singleton_append, List.append_assoc]

/-- **Law 38's sound direction**: when the search reports a step, a step exists. The search is the
computation the corpus decides against; this says it never claims one the rule cannot derive, which is
what makes the corpus's `true` verdicts statements about the *rule* — and, together with the domain
restriction in `takesStep_iff_reduces`, what makes silence a law rather than a remark. -/
theorem takesStep_sound (p : Par) : takesStep p = true → ∃ q', ReduceP p q' := by
  intro h
  obtain ⟨s, hs, h₁⟩ := stepsInSends_sound p.sends p.receives h
  obtain ⟨r, hr, hlen, h₂⟩ := stepsInReceives_sound s p.receives h₁
  obtain ⟨b, hbmem, a, hchan, hsrc, harity, hmatch⟩ := stepsInBinds_sound s r.binds h₂
  have hbinds : r.binds = [b] := eq_singleton_of_length_one hlen hbmem
  obtain ⟨sL, sR, rL, rR, hsp, hrp, hsplit⟩ := exists_redex_split p hs hr
  rw [hsplit]
  refine ⟨parMerge (parMerge (Par.mk sL rL p.news p.exprs p.matches p.unforgeables p.bundles
      p.connectives) r.body) (Par.mk sR rR [] [] [] [] [] []), ?_⟩
  have hredex : ReduceP (Par.mk [s] [r] [] [] [] [] [] []) r.body := by
    cases s with | mk chan data persistent =>
    cases r with | mk bs body pr bc =>
    cases b with | mk patterns source freeCount =>
    simp only at hbinds
    subst hbinds
    simpa only [sendParP, parMerge, Par.sends, Par.receives, Par.news, Par.exprs, Par.matches,
      Par.unforgeables, Par.bundles, Par.connectives, List.nil_append, List.append_nil]
      using ReduceP.commPs (sendChan := Send.mk chan data persistent |>.chan) (srcChan := source)
        hchan hsrc harity hmatch
  exact ReduceP.parLeft (ReduceP.parRight hredex)

end Rchain
