import Rchain.Sort
import Rchain.Ty

/-!
# Law 3 — capture-avoiding de Bruijn substitution

`sort(subst t) = subst(sort t)`: canonicalization commutes with substitution. The Scala oracle is
`Substitute.scala` + `Env.scala`; the Rust realization is `rholang::substitute`
(`rholang/src/substitute.rs:164`'s `substitute_par`, total on `Closed`). The *minimal* substitution the
type-system fundamentals need is defined in `Rchain.Ty` (`subst`/`substExpr`); the **deep**
capture-avoiding substitution is Coq's Autosubst obligation (`AGENTS.md`).

## What is wrong with this file as it stood (2026-09-23)

`substPar` is an `axiom`, and the two laws are stated *about* it. That is not a definition with a
theorem attached; it is three postulates, and the pair of facts below is why the register now calls
the row `owed` rather than treating it as evidence:

1. **The trio does not constrain substitution at all.** `noSubst` — the function that substitutes
   *nothing* — satisfies both laws, `the_identity_satisfies_sort_subst` and
   `the_identity_satisfies_subst_closed`. So every `proved*` claim resting on these axioms rests on a
   statement the identity also satisfies: the register's `vacuous` shape, one level up.
2. **`subst_closed` was missing a hypothesis, without which it is false of the operation it
   names.** `Closed` is "no `Var.free`" (`Ty.lean`'s `closedVar`: `bound` counts as closed), while
   `σ : Var → Par` is arbitrary — so a substitution whose image at a *bound* occurrence is open takes
   a closed term to an open one (`bound_is_closed_free_is_not` states the two halves of that fact).
   The Rust's own test carries exactly the missing hypothesis
   (`law3_substituting_a_closed_value_keeps_the_term_closed`, whose value is `arb_closed`), so the
   statement below is narrowed to match the code it claims to describe.

The fix for both is the same, and it is the register's Programme D unit 5: **define** `substPar` by
mirroring `substitute_par` (an `Env<Par>` keyed by de Bruijn level with a shift, and a `depth` that
is incremented inside receive and match-case *patterns*), at which point these become theorems about
a definition rather than postulates. Until then the axioms are kept — a row citing an axiom that is
gone fails the register's accounting — with the narrowing and the two witnesses the row cites.
-/

namespace Rchain
open Comparator

/-! ## The operation, defined (mirroring `rholang/src/substitute.rs`)

The three axioms that stood here — `substPar`, `sort_subst`, `subst_closed` — are replaced by the
definition the code has, because an axiom whose function is undefined is satisfied by the identity (see
`the_identity_satisfies_sort_subst` below, kept for the record). The mirror is of
`substitute_par_no_sort` (`substitute.rs:116-162`) and the arms it calls, with three deliberate
differences, each a *representational* one rather than a semantic one, recorded here because a reader
comparing the two files will meet them:

1. **No environment shift.** The port's `Env<Par>` is a map keyed by absolute level with a `level`
   counter and a `shift`, and a `BoundVar(index)` occurrence is resolved as
   `env_map[(level + shift) - index - 1]` (`env.rs:36-41`) — an *index*-based representation of what
   this model writes as `Var.bound (level : Nat)`. Levels are absolute, so under a binder the model
   needs no shift: `substPar` looks up `σ v` directly. The port's `env.shift(bind_count)` under a `New`
   body and a receive body (`substitute.rs:239,252`) and `shift(free_count)` under a match case's body
   (`:275`) are what an index-based representation must do to keep referring to the same variables —
   here that is free.
2. **The depth is carried, and it is the *term position* gate**: `d = 0` substitutes, `d ≥ 1` keeps
   every variable (`maybe_substitute_var`, `substitute.rs:21-40`). The port's callers use both — `0`
   for a term and `1` for the pattern side of an `EMatches` (`reduce.rs:606-607`) — and patterns inside
   a receive or a match case go to `d + 1` (`:229`, `:277`), which is why the recursion carries it.
3. **`σ` is total where the port's environment is partial.** The port keeps an absent index unchanged
   and errors on a `FreeVar`/`Wildcard` at depth 0 (`SubstituteError`); a total `σ : Var → Par` has no
   absent case, and the error case is a guard against a term that cannot occur in a closed program. The
   model therefore substitutes every `bound` occurrence it finds and leaves `free`/`wildcard`
   occurrences alone, which is the port's "keep" branch.

The one *semantic* step kept verbatim is the splice: a substituted occurrence contributes the whole
`Par` (`par_concat (sub_par.quote ())`, `:58`, `:73` — `quote` is a sort-relabel in this port, so this
is a field-wise merge), not an expression wrapping it; the substituted value's fields land in the
target's fields. And the set/map children are sorted *inside* the recursion (`:437`, `:454`) — the one
place the core is not a plain homomorphism, which is why the public entry point's final `sort_par_term`
is not enough on its own. -/


/-- A `Par` with a single expression — the model's `one`. -/
def singleExpr (e : Expr) : Par := Par.mk [] [] [] [e] [] [] [] []

/-- A `Par` with a single connective — a connective is a *field* of its own, not an expression. -/
def singleConnective (c : Connective) : Par := Par.mk [] [] [] [] [] [] [] [c]

mutual
  /-- **Substitution**, mirroring `substitute_par_no_sort` (`:116-162`). `d` is the depth: `0` is a term
      position (variables substitute), `d ≥ 1` a pattern position (they do not).

      Every list is walked by a function of its own rather than by `.map`, which is both what the port
      does (`for … in …iter()` accumulating a `Par`) and what lets the termination checker see the
      descent: the ASTs are mutually recursive, so the measure is `sizeOf` — the shape `Sort.lean`'s
      `sortPar` family uses — and a `.map` would hide the subterm relation. -/
  def substPar (σ : Var → Par) (d : Nat) : Par → Par
    | Par.mk s r n e m u b c =>
        parMerge (parMerge (substExprsToPar σ d e) (substListConnective σ d c))
          (Par.mk (substListSend σ d s) (substListReceive σ d r) (substListNew σ d n) []
                  (substListMatch σ d m) u (substListBundle σ d b) [])

  /-- One `Send`: channel and data at the same depth (`:196-214`). -/
  def substSend (σ : Var → Par) (d : Nat) : Send → Send
    | Send.mk ch data p => Send.mk (substPar σ d ch) (substListPar σ d data) p

  /-- One `Receive`: the binds and the body at the same depth — the port shifts its environment by
      `bind_count` here, which a level-based representation does not need. -/
  def substReceive (σ : Var → Par) (d : Nat) : Receive → Receive
    | Receive.mk binds body p n =>
        Receive.mk (substListReceiveBind σ d binds) (substPar σ d body) p n

  /-- One bind: the source at the same depth, the patterns one deeper (`:229`). -/
  def substReceiveBind (σ : Var → Par) (d : Nat) : ReceiveBind → ReceiveBind
    | ReceiveBind.mk ps src fc =>
        ReceiveBind.mk (substListPar σ (d + 1) ps) (substPar σ d src) fc

  /-- `new`: the body at the same depth (`:251-260`; the port shifts its environment by `bind_count`). -/
  def substNew (σ : Var → Par) (d : Nat) : New → New
    | New.mk n body => New.mk n (substPar σ d body)

  /-- A `match`: the target at the same depth, the cases one by one. -/
  def substMatch (σ : Var → Par) (d : Nat) : Match → Match
    | Match.mk t cs => Match.mk (substPar σ d t) (substListMatchCase σ d cs)

  /-- One case: the pattern one deeper, the source at the same depth (`:277`; the port's environment
      shift by `free_count` is again index bookkeeping). -/
  def substMatchCase (σ : Var → Par) (d : Nat) : MatchCase → MatchCase
    | MatchCase.mk p src fc => MatchCase.mk (substPar σ (d + 1) p) (substPar σ d src) fc

  /-- A bundle's body at the same depth (`:293-306`; the port additionally merges a bundle-of-bundle,
      a normalisation the model's `Bundle` has no constructor for). -/
  def substBundle (σ : Var → Par) (d : Nat) : Bundle → Bundle
    | Bundle.mk body w r => Bundle.mk (substPar σ d body) w r

  /-- One connective: the connective bodies at the same depth, and a **term-position** `VarRef`
      (`depth = 0`) spliced like a variable occurrence — the port substitutes a var-ref only when its
      recorded depth equals the ambient one (`maybe_substitute_varref`, `:42-49`), and the value it
      splices is the environment's, not re-substituted. -/
  def substConnective (σ : Var → Par) (d : Nat) : Connective → Par
    | Connective.connAnd ps => singleConnective (.connAnd (substListPar σ d ps))
    | Connective.connOr ps => singleConnective (.connOr (substListPar σ d ps))
    | Connective.connNot p => singleConnective (.connNot (substPar σ d p))
    | Connective.connVarRef dep k =>
        if dep = 0 then σ (.bound k) else singleConnective (.connVarRef dep k)

  /-- The connective list as a `Par`, because a substituted `VarRef` contributes a whole one
      (`sub_conns`, `:68-114`). -/
  def substListConnective (σ : Var → Par) (d : Nat) : List Connective → Par
    | [] => nilPar
    | c :: cs => parMerge (substConnective σ d c) (substListConnective σ d cs)

  /-- The `exprs` field as a `Par`: each expression contributes either a spliced `Par` (a substituted
      occurrence) or itself (`sub_exprs`, `:51-66`). -/
  def substExprsToPar (σ : Var → Par) (d : Nat) : List Expr → Par
    | [] => nilPar
    | e :: es => parMerge (substExprToPar σ d e) (substExprsToPar σ d es)

  /-- One expression: a ground is left alone; an `evar` substitutes at depth 0 (its image's **fields**
      spliced in, as the port does) and is kept otherwise; the operators recurse; the collections are
      walked, with `eset`/`emap` children sorted — the one non-homomorphic step (`:437`, `:454`). -/
  def substExprToPar (σ : Var → Par) (d : Nat) : Expr → Par
    | .ground g => singleExpr (.ground g)
    | .evar (.bound k) => if d = 0 then σ (.bound k) else singleExpr (.evar (.bound k))
    | .evar (.free k) => singleExpr (.evar (.free k))
    | .evar .wildcard => singleExpr (.evar .wildcard)
    | .eneg p => singleExpr (.eneg (substPar σ d p))
    | .enot p => singleExpr (.enot (substPar σ d p))
    | .eplus a b => singleExpr (.eplus (substPar σ d a) (substPar σ d b))
    | .eminus a b => singleExpr (.eminus (substPar σ d a) (substPar σ d b))
    | .emult a b => singleExpr (.emult (substPar σ d a) (substPar σ d b))
    | .ediv a b => singleExpr (.ediv (substPar σ d a) (substPar σ d b))
    | .emod a b => singleExpr (.emod (substPar σ d a) (substPar σ d b))
    | .elt a b => singleExpr (.elt (substPar σ d a) (substPar σ d b))
    | .ele a b => singleExpr (.ele (substPar σ d a) (substPar σ d b))
    | .egt a b => singleExpr (.egt (substPar σ d a) (substPar σ d b))
    | .ege a b => singleExpr (.ege (substPar σ d a) (substPar σ d b))
    | .eeq a b => singleExpr (.eeq (substPar σ d a) (substPar σ d b))
    | .eneq a b => singleExpr (.eneq (substPar σ d a) (substPar σ d b))
    | .eand a b => singleExpr (.eand (substPar σ d a) (substPar σ d b))
    | .eor a b => singleExpr (.eor (substPar σ d a) (substPar σ d b))
    | .elist ps r => singleExpr (.elist (substListPar σ d ps) r)
    | .etuple ps => singleExpr (.etuple (substListPar σ d ps))
    | .eset ps r => singleExpr (.eset (sortListPar (substListPar σ d ps)) r)
    | .emap kvs r => singleExpr (.emap (sortListParPair (substListParPair σ d kvs)) r)

  def substListSend (σ : Var → Par) (d : Nat) : List Send → List Send
    | [] => []
    | x :: xs => substSend σ d x :: substListSend σ d xs

  def substListReceive (σ : Var → Par) (d : Nat) : List Receive → List Receive
    | [] => []
    | x :: xs => substReceive σ d x :: substListReceive σ d xs

  def substListReceiveBind (σ : Var → Par) (d : Nat) : List ReceiveBind → List ReceiveBind
    | [] => []
    | x :: xs => substReceiveBind σ d x :: substListReceiveBind σ d xs

  def substListNew (σ : Var → Par) (d : Nat) : List New → List New
    | [] => []
    | x :: xs => substNew σ d x :: substListNew σ d xs

  def substListMatch (σ : Var → Par) (d : Nat) : List Match → List Match
    | [] => []
    | x :: xs => substMatch σ d x :: substListMatch σ d xs

  def substListMatchCase (σ : Var → Par) (d : Nat) : List MatchCase → List MatchCase
    | [] => []
    | x :: xs => substMatchCase σ d x :: substListMatchCase σ d xs

  def substListBundle (σ : Var → Par) (d : Nat) : List Bundle → List Bundle
    | [] => []
    | x :: xs => substBundle σ d x :: substListBundle σ d xs

  def substListPar (σ : Var → Par) (d : Nat) : List Par → List Par
    | [] => []
    | x :: xs => substPar σ d x :: substListPar σ d xs

  def substListParPair (σ : Var → Par) (d : Nat) : List (Par × Par) → List (Par × Par)
    | [] => []
    | (a, b) :: xs => (substPar σ d a, substPar σ d b) :: substListParPair σ d xs
end

/-! ## The two `single` lemmas

`substExprToPar` and `substConnective` do not return a `Par` with the expression or connective in the
corresponding field at an arbitrary place: they return `singleExpr`/`singleConnective`, whose only
non-empty field is the last. `sortPar` of one is therefore the sorted form `sortExpr`/`sortConnective`
produce, which is what the members' induction steps consume. These are `Par`-level, but they are about
the two `single` constructors this file defines, so they live here. -/

/-- A `Par` with one expression sorts to the `Par` with the sorted expression. -/
private theorem sortPar_singleExpr (e : Expr) :
    sortPar (singleExpr e) = singleExpr (sortExpr e) := by
  cases e <;> simp [singleExpr, sortExpr, sortPar, sortListExpr, sortListPar, sortListParPair,
    sortList, Comparator.sortList, List.insertionSort, sortListPar_eq_map, List.map_nil]

/-- The same for the connective constructor. -/
private theorem sortPar_singleConnective (c : Connective) :
    sortPar (singleConnective c) = singleConnective (sortConnective c) := by
  cases c <;> simp [singleConnective, sortConnective, sortPar, sortListConnective, sortListPar,
    sortList, Comparator.sortList, List.insertionSort, sortListPar_eq_map, List.map_nil]

/-! ## The substitution walks as maps

`substListX` is a structural walk, so these are inductions — but they are inductions about the
*definition*, not about the block: they let a member write `substListX σ d l` as a `map`, which is the
form in which the list members' two ingredients — the head's element law and the induction
hypothesis — are both usable. -/

private theorem substListSend_eq_map (σ : Var → Par) (d : Nat) (l : List Send) :
    substListSend σ d l = l.map (substSend σ d) := by
  induction l <;> simp [substListSend, *]

private theorem substListReceive_eq_map (σ : Var → Par) (d : Nat) (l : List Receive) :
    substListReceive σ d l = l.map (substReceive σ d) := by
  induction l <;> simp [substListReceive, *]

private theorem substListReceiveBind_eq_map (σ : Var → Par) (d : Nat) (l : List ReceiveBind) :
    substListReceiveBind σ d l = l.map (substReceiveBind σ d) := by
  induction l <;> simp [substListReceiveBind, *]

private theorem substListNew_eq_map (σ : Var → Par) (d : Nat) (l : List New) :
    substListNew σ d l = l.map (substNew σ d) := by
  induction l <;> simp [substListNew, *]

private theorem substListMatch_eq_map (σ : Var → Par) (d : Nat) (l : List Match) :
    substListMatch σ d l = l.map (substMatch σ d) := by
  induction l <;> simp [substListMatch, *]

private theorem substListMatchCase_eq_map (σ : Var → Par) (d : Nat) (l : List MatchCase) :
    substListMatchCase σ d l = l.map (substMatchCase σ d) := by
  induction l <;> simp [substListMatchCase, *]

private theorem substListBundle_eq_map (σ : Var → Par) (d : Nat) (l : List Bundle) :
    substListBundle σ d l = l.map (substBundle σ d) := by
  induction l <;> simp [substListBundle, *]

private theorem substListPar_eq_map (σ : Var → Par) (d : Nat) (l : List Par) :
    substListPar σ d l = l.map (substPar σ d) := by
  induction l <;> simp [substListPar, *]

private theorem substListParPair_eq_map (σ : Var → Par) (d : Nat) (l : List (Par × Par)) :
    substListParPair σ d l = l.map (fun x => (substPar σ d x.1, substPar σ d x.2)) := by
  induction l with
  | nil => rfl
  | cons x xs ih => cases x with | mk a b => simp [substListParPair, ih]

/-! ## Permutation invariance of the two folds

`substExprsToPar`/`substListConnective` fold by `parMerge`, and `sortPar` sees a `parMerge` tree only
as its sorted fields — so the fold is invariant under permutation, which is what lets the expression
and connective members change their target's `orderedInsert`-shaped argument back into a cons. -/

private theorem sortPar_substExprsToPar_perm (σ : Var → Par) (d : Nat) {l l' : List Expr}
    (h : List.Perm l l') :
    sortPar (substExprsToPar σ d l) = sortPar (substExprsToPar σ d l') := by
  induction h with
  | nil => rfl
  | cons x _ ih => simp only [substExprsToPar]; exact sortPar_parMerge rfl ih
  | swap x y l =>
      show sortPar (parMerge (substExprToPar σ d y)
              (parMerge (substExprToPar σ d x) (substExprsToPar σ d l)))
          = sortPar (parMerge (substExprToPar σ d x)
              (parMerge (substExprToPar σ d y) (substExprsToPar σ d l)))
      rw [← parMerge_assoc (substExprToPar σ d y) (substExprToPar σ d x) (substExprsToPar σ d l),
          ← parMerge_assoc (substExprToPar σ d x) (substExprToPar σ d y) (substExprsToPar σ d l)]
      exact sortPar_parMerge (sortPar_comm _ _) rfl
  | trans _ _ ih1 ih2 => exact ih1.trans ih2

private theorem sortPar_substListConnective_perm (σ : Var → Par) (d : Nat) {l l' : List Connective}
    (h : List.Perm l l') :
    sortPar (substListConnective σ d l) = sortPar (substListConnective σ d l') := by
  induction h with
  | nil => rfl
  | cons x _ ih => simp only [substListConnective]; exact sortPar_parMerge rfl ih
  | swap x y l =>
      show sortPar (parMerge (substConnective σ d y)
              (parMerge (substConnective σ d x) (substListConnective σ d l)))
          = sortPar (parMerge (substConnective σ d x)
              (parMerge (substConnective σ d y) (substListConnective σ d l)))
      rw [← parMerge_assoc (substConnective σ d y) (substConnective σ d x) (substListConnective σ d l),
          ← parMerge_assoc (substConnective σ d x) (substConnective σ d y) (substListConnective σ d l)]
      exact sortPar_parMerge (sortPar_comm _ _) rfl
  | trans _ _ ih1 ih2 => exact ih1.trans ih2

set_option maxHeartbeats 1000000 in
set_option maxRecDepth 10000 in
mutual
  /-- **The commuting law's induction.** Every member says the same thing of its own type: substituting
      then canonicalizing is canonicalizing after substituting into the canonical form. -/
  theorem sortPar_subst (σ : Var → Par) (d : Nat) :
      ∀ t : Par, sortPar (substPar σ d t) = sortPar (substPar σ d (sortPar t))
    | Par.mk s r n e m u b c => by
      simp only [substPar, sortPar]
      have hpiece : sortPar (Par.mk (substListSend σ d s) (substListReceive σ d r)
            (substListNew σ d n) [] (substListMatch σ d m) u (substListBundle σ d b) [])
          = sortPar (Par.mk (substListSend σ d (sortList sendComparator (sortListSend s)))
              (substListReceive σ d (sortList receiveComparator (sortListReceive r)))
              (substListNew σ d (sortList newComparator (sortListNew n))) []
              (substListMatch σ d (sortList matchComparator (sortListMatch m)))
              (sortList gUnforgeableComparator (sortListGUnforgeable u))
              (substListBundle σ d (sortList bundleComparator (sortListBundle b))) []) := by
        simp only [sortPar, Par.mk.injEq]
        exact ⟨sortListSend_subst σ d s, sortListReceive_subst σ d r, sortListNew_subst σ d n,
          trivial, sortListMatch_subst σ d m, sortListGUnforgeable_subst σ d u,
          sortListBundle_subst σ d b, trivial⟩
      exact (sortPar_parMerge (sortPar_parMerge (sortExprsToPar_subst σ d e)
        (sortListConnective_subst σ d c)) hpiece).trans rfl
  termination_by t => sizeOf t

  theorem sortSend_subst (σ : Var → Par) (d : Nat) :
      ∀ x : Send, sortSend (substSend σ d x) = sortSend (substSend σ d (sortSend x))
    | Send.mk c dta p => by
      simp only [substSend, sortSend]
      rw [sortPar_subst σ d c, sortListPar_subst σ d dta]
  termination_by x => sizeOf x

  theorem sortReceiveBind_subst (σ : Var → Par) (d : Nat) :
      ∀ x : ReceiveBind,
        sortReceiveBind (substReceiveBind σ d x) = sortReceiveBind (substReceiveBind σ d (sortReceiveBind x))
    | ReceiveBind.mk ps src fc => by
      simp only [substReceiveBind, sortReceiveBind]
      rw [sortListPar_subst σ (d + 1) ps, sortPar_subst σ d src]
  termination_by x => sizeOf x

  theorem sortReceive_subst (σ : Var → Par) (d : Nat) :
      ∀ x : Receive,
        sortReceive (substReceive σ d x) = sortReceive (substReceive σ d (sortReceive x))
    | Receive.mk bs body p n => by
      simp only [substReceive, sortReceive]
      rw [show receiveBindComparator.sortList (sortListReceiveBind (substListReceiveBind σ d bs))
            = receiveBindComparator.sortList
                (sortListReceiveBind (substListReceiveBind σ d
                  (receiveBindComparator.sortList (sortListReceiveBind bs))))
          from sortListReceiveBind_subst σ d bs,
          show sortPar (substPar σ d body) = sortPar (substPar σ d (sortPar body))
          from sortPar_subst σ d body]
  termination_by x => sizeOf x

  theorem sortNew_subst (σ : Var → Par) (d : Nat) :
      ∀ x : New, sortNew (substNew σ d x) = sortNew (substNew σ d (sortNew x))
    | New.mk n b => by simp only [substNew, sortNew]; rw [sortPar_subst σ d b]
  termination_by x => sizeOf x

  theorem sortMatchCase_subst (σ : Var → Par) (d : Nat) :
      ∀ x : MatchCase,
        sortMatchCase (substMatchCase σ d x) = sortMatchCase (substMatchCase σ d (sortMatchCase x))
    | MatchCase.mk p src fc => by
      simp only [substMatchCase, sortMatchCase]
      rw [sortPar_subst σ (d + 1) p, sortPar_subst σ d src]
  termination_by x => sizeOf x

  theorem sortMatch_subst (σ : Var → Par) (d : Nat) :
      ∀ x : Match, sortMatch (substMatch σ d x) = sortMatch (substMatch σ d (sortMatch x))
    | Match.mk t cs => by
      simp only [substMatch, sortMatch]
      rw [sortPar_subst σ d t, sortListMatchCase_subst σ d cs]
  termination_by x => sizeOf x

  theorem sortBundle_subst (σ : Var → Par) (d : Nat) :
      ∀ x : Bundle, sortBundle (substBundle σ d x) = sortBundle (substBundle σ d (sortBundle x))
    | Bundle.mk body w r => by
      simp only [substBundle, sortBundle]; rw [sortPar_subst σ d body]
  termination_by x => sizeOf x

  theorem sortExprToPar_subst (σ : Var → Par) (d : Nat) :
      ∀ e : Expr, sortPar (substExprToPar σ d e) = sortPar (substExprToPar σ d (sortExpr e))
    | .ground g => by simp only [substExprToPar.eq_def, sortExpr.eq_def, sortPar_singleExpr]
    | .evar (.bound k) => by simp only [substExprToPar.eq_def, sortExpr.eq_def]
    | .evar (.free k) => by simp only [substExprToPar.eq_def, sortExpr.eq_def, sortPar_singleExpr]
    | .evar .wildcard => by simp only [substExprToPar.eq_def, sortExpr.eq_def, sortPar_singleExpr]
    | .eneg p => by
      simp only [substExprToPar.eq_def, sortExpr.eq_def, sortPar_singleExpr]
      rw [sortPar_subst σ d p]
    | .enot p => by
      simp only [substExprToPar.eq_def, sortExpr.eq_def, sortPar_singleExpr]
      rw [sortPar_subst σ d p]
    | .eplus a b => by
      simp only [substExprToPar.eq_def, sortExpr.eq_def, sortPar_singleExpr]
      rw [sortPar_subst σ d a, sortPar_subst σ d b]
    | .eminus a b => by
      simp only [substExprToPar.eq_def, sortExpr.eq_def, sortPar_singleExpr]
      rw [sortPar_subst σ d a, sortPar_subst σ d b]
    | .emult a b => by
      simp only [substExprToPar.eq_def, sortExpr.eq_def, sortPar_singleExpr]
      rw [sortPar_subst σ d a, sortPar_subst σ d b]
    | .ediv a b => by
      simp only [substExprToPar.eq_def, sortExpr.eq_def, sortPar_singleExpr]
      rw [sortPar_subst σ d a, sortPar_subst σ d b]
    | .emod a b => by
      simp only [substExprToPar.eq_def, sortExpr.eq_def, sortPar_singleExpr]
      rw [sortPar_subst σ d a, sortPar_subst σ d b]
    | .elt a b => by
      simp only [substExprToPar.eq_def, sortExpr.eq_def, sortPar_singleExpr]
      rw [sortPar_subst σ d a, sortPar_subst σ d b]
    | .ele a b => by
      simp only [substExprToPar.eq_def, sortExpr.eq_def, sortPar_singleExpr]
      rw [sortPar_subst σ d a, sortPar_subst σ d b]
    | .egt a b => by
      simp only [substExprToPar.eq_def, sortExpr.eq_def, sortPar_singleExpr]
      rw [sortPar_subst σ d a, sortPar_subst σ d b]
    | .ege a b => by
      simp only [substExprToPar.eq_def, sortExpr.eq_def, sortPar_singleExpr]
      rw [sortPar_subst σ d a, sortPar_subst σ d b]
    | .eeq a b => by
      simp only [substExprToPar.eq_def, sortExpr.eq_def, sortPar_singleExpr]
      rw [sortPar_subst σ d a, sortPar_subst σ d b]
    | .eneq a b => by
      simp only [substExprToPar.eq_def, sortExpr.eq_def, sortPar_singleExpr]
      rw [sortPar_subst σ d a, sortPar_subst σ d b]
    | .eand a b => by
      simp only [substExprToPar.eq_def, sortExpr.eq_def, sortPar_singleExpr]
      rw [sortPar_subst σ d a, sortPar_subst σ d b]
    | .eor a b => by
      simp only [substExprToPar.eq_def, sortExpr.eq_def, sortPar_singleExpr]
      rw [sortPar_subst σ d a, sortPar_subst σ d b]
    | .elist ps r => by
      simp only [substExprToPar.eq_def, sortExpr.eq_def, sortPar_singleExpr]
      exact congrArg (fun t => singleExpr (.elist t r)) (sortListPar_subst σ d ps)
    | .etuple ps => by
      simp only [substExprToPar.eq_def, sortExpr.eq_def, sortPar_singleExpr]
      exact congrArg (fun t => singleExpr (.etuple t)) (sortListPar_subst σ d ps)
    | .eset ps r => by
      -- `substExprToPar` sorts a set's children and `sortExpr` sorts them again, so both sides
      -- carry a double `sortListPar` that the outer `sortList` cannot see through.
      simp only [substExprToPar.eq_def, sortExpr.eq_def, sortPar_singleExpr]
      have h1 := congrArg (fun t => singleExpr (.eset t r))
        (sortList_sortListPar_idem (substListPar σ d ps))
      have h2 := congrArg (fun t => singleExpr (.eset t r)) (sortListPar_subst σ d ps)
      have h3 := congrArg (fun t => singleExpr (.eset t r))
        (sortList_sortListPar_idem (substListPar σ d (sortList parComparator (sortListPar ps))))
      exact h1.trans (h2.trans h3.symm)
    | .emap kvs r => by
      simp only [substExprToPar.eq_def, sortExpr.eq_def, sortPar_singleExpr]
      have h1 := congrArg (fun t => singleExpr (.emap t r))
        (sortList_sortListParPair_idem (substListParPair σ d kvs))
      have h2 := congrArg (fun t => singleExpr (.emap t r)) (sortListParPair_subst σ d kvs)
      have h3 := congrArg (fun t => singleExpr (.emap t r)) (sortList_sortListParPair_idem
        (substListParPair σ d
          (sortList (Comparator.cmpPair parComparator parComparator) (sortListParPair kvs))))
      exact h1.trans (h2.trans h3.symm)
  termination_by e => sizeOf e

  theorem sortConnective_subst (σ : Var → Par) (d : Nat) :
      ∀ c : Connective,
        sortPar (substConnective σ d c) = sortPar (substConnective σ d (sortConnective c))
    | Connective.connAnd ps => by
      simp only [substConnective, sortConnective, sortPar_singleConnective]
      exact congrArg (fun t => singleConnective (.connAnd t)) (sortListPar_subst σ d ps)
    | Connective.connOr ps => by
      simp only [substConnective, sortConnective, sortPar_singleConnective]
      exact congrArg (fun t => singleConnective (.connOr t)) (sortListPar_subst σ d ps)
    | Connective.connNot p => by
      simp only [substConnective, sortConnective, sortPar_singleConnective]
      exact congrArg (fun t => singleConnective (.connNot t)) (sortPar_subst σ d p)
    | Connective.connVarRef dep k => by
      simp only [substConnective, sortConnective]
  termination_by c => sizeOf c

  theorem sortExprsToPar_subst (σ : Var → Par) (d : Nat) :
      ∀ l : List Expr,
        sortPar (substExprsToPar σ d l)
          = sortPar (substExprsToPar σ d (sortList exprComparator (sortListExpr l)))
    | [] => by simp [substExprsToPar, sortList, sortListExpr]
    | e :: es => by
      simp only [substExprsToPar, sortListExpr, List.map_cons]
      have hperm : List.Perm (sortList exprComparator (sortExpr e :: sortListExpr es))
          (sortExpr e :: sortList exprComparator (sortListExpr es)) :=
        List.perm_orderedInsert exprComparator.le (sortExpr e)
          (sortList exprComparator (sortListExpr es))
      rw [sortPar_substExprsToPar_perm σ d hperm]
      simp only [substExprsToPar]
      exact sortPar_parMerge (sortExprToPar_subst σ d e) (sortExprsToPar_subst σ d es)
  termination_by l => sizeOf l

  theorem sortListConnective_subst (σ : Var → Par) (d : Nat) :
      ∀ l : List Connective,
        sortPar (substListConnective σ d l)
          = sortPar (substListConnective σ d (sortList connectiveComparator (sortListConnective l)))
    | [] => by simp [substListConnective, sortList, sortListConnective]
    | c :: cs => by
      simp only [substListConnective, sortListConnective, List.map_cons]
      have hperm : List.Perm (sortList connectiveComparator (sortConnective c :: sortListConnective cs))
          (sortConnective c :: sortList connectiveComparator (sortListConnective cs)) :=
        List.perm_orderedInsert connectiveComparator.le (sortConnective c)
          (sortList connectiveComparator (sortListConnective cs))
      rw [sortPar_substListConnective_perm σ d hperm]
      simp only [substListConnective]
      exact sortPar_parMerge (sortConnective_subst σ d c) (sortListConnective_subst σ d cs)
  termination_by l => sizeOf l

  theorem sortListSend_subst (σ : Var → Par) (d : Nat) :
      ∀ l : List Send,
        sortList sendComparator (sortListSend (substListSend σ d l))
          = sortList sendComparator
              (sortListSend (substListSend σ d (sortList sendComparator (sortListSend l))))
    | [] => by simp [substListSend, sortListSend]
    | x :: xs => by
      simp only [substListSend_eq_map, sortListSend_eq_map, List.map_map, List.map_cons]
      rw [sortList_cons, sortList_cons]
      have hperm : List.Perm
          ((List.orderedInsert sendComparator.le (sortSend x)
            (sortList sendComparator (List.map sortSend xs))).map (sortSend ∘ substSend σ d))
          ((sortSend x :: sortList sendComparator (List.map sortSend xs)).map
            (sortSend ∘ substSend σ d)) :=
        (List.perm_orderedInsert sendComparator.le (sortSend x)
          (sortList sendComparator (List.map sortSend xs))).map _
      rw [sortList_perm _ hperm]
      simp only [List.map_cons]
      have IH : sortList sendComparator (List.map (sortSend ∘ substSend σ d) xs)
          = sortList sendComparator
              (List.map (sortSend ∘ substSend σ d) (sortList sendComparator (List.map sortSend xs))) := by
        simpa only [substListSend_eq_map, sortListSend_eq_map, List.map_map] using
          sortListSend_subst σ d xs
      exact sortList_cons_congr sendComparator (sortSend_subst σ d x) IH
  termination_by l => sizeOf l

  theorem sortListReceiveBind_subst (σ : Var → Par) (d : Nat) :
      ∀ l : List ReceiveBind,
        sortList receiveBindComparator (sortListReceiveBind (substListReceiveBind σ d l))
          = sortList receiveBindComparator
              (sortListReceiveBind (substListReceiveBind σ d
                (sortList receiveBindComparator (sortListReceiveBind l))))
    | [] => by simp [substListReceiveBind, sortListReceiveBind]
    | x :: xs => by
      simp only [substListReceiveBind_eq_map, sortListReceiveBind_eq_map, List.map_map, List.map_cons]
      rw [sortList_cons, sortList_cons]
      have hperm : List.Perm
          ((List.orderedInsert receiveBindComparator.le (sortReceiveBind x)
            (sortList receiveBindComparator (List.map sortReceiveBind xs))).map (sortReceiveBind ∘ substReceiveBind σ d))
          ((sortReceiveBind x :: sortList receiveBindComparator (List.map sortReceiveBind xs)).map (sortReceiveBind ∘ substReceiveBind σ d)) :=
        (List.perm_orderedInsert receiveBindComparator.le (sortReceiveBind x)
          (sortList receiveBindComparator (List.map sortReceiveBind xs))).map _
      rw [sortList_perm _ hperm]
      simp only [List.map_cons]
      have IH : sortList receiveBindComparator (List.map (sortReceiveBind ∘ substReceiveBind σ d) xs)
          = sortList receiveBindComparator (List.map (sortReceiveBind ∘ substReceiveBind σ d) (sortList receiveBindComparator (List.map sortReceiveBind xs))) := by
        simpa only [substListReceiveBind_eq_map, sortListReceiveBind_eq_map, List.map_map] using sortListReceiveBind_subst σ d xs
      exact sortList_cons_congr receiveBindComparator (sortReceiveBind_subst σ d x) IH
  termination_by l => sizeOf l

  theorem sortListReceive_subst (σ : Var → Par) (d : Nat) :
      ∀ l : List Receive,
        sortList receiveComparator (sortListReceive (substListReceive σ d l))
          = sortList receiveComparator
              (sortListReceive (substListReceive σ d (sortList receiveComparator (sortListReceive l))))
    | [] => by simp [substListReceive, sortListReceive]
    | x :: xs => by
      simp only [substListReceive_eq_map, sortListReceive_eq_map, List.map_map, List.map_cons]
      rw [sortList_cons, sortList_cons]
      have hperm : List.Perm
          ((List.orderedInsert receiveComparator.le (sortReceive x)
            (sortList receiveComparator (List.map sortReceive xs))).map (sortReceive ∘ substReceive σ d))
          ((sortReceive x :: sortList receiveComparator (List.map sortReceive xs)).map (sortReceive ∘ substReceive σ d)) :=
        (List.perm_orderedInsert receiveComparator.le (sortReceive x)
          (sortList receiveComparator (List.map sortReceive xs))).map _
      rw [sortList_perm _ hperm]
      simp only [List.map_cons]
      have IH : sortList receiveComparator (List.map (sortReceive ∘ substReceive σ d) xs)
          = sortList receiveComparator (List.map (sortReceive ∘ substReceive σ d) (sortList receiveComparator (List.map sortReceive xs))) := by
        simpa only [substListReceive_eq_map, sortListReceive_eq_map, List.map_map] using sortListReceive_subst σ d xs
      exact sortList_cons_congr receiveComparator (sortReceive_subst σ d x) IH
  termination_by l => sizeOf l

  theorem sortListNew_subst (σ : Var → Par) (d : Nat) :
      ∀ l : List New,
        sortList newComparator (sortListNew (substListNew σ d l))
          = sortList newComparator
              (sortListNew (substListNew σ d (sortList newComparator (sortListNew l))))
    | [] => by simp [substListNew, sortListNew]
    | x :: xs => by
      simp only [substListNew_eq_map, sortListNew_eq_map, List.map_map, List.map_cons]
      rw [sortList_cons, sortList_cons]
      have hperm : List.Perm
          ((List.orderedInsert newComparator.le (sortNew x)
            (sortList newComparator (List.map sortNew xs))).map (sortNew ∘ substNew σ d))
          ((sortNew x :: sortList newComparator (List.map sortNew xs)).map (sortNew ∘ substNew σ d)) :=
        (List.perm_orderedInsert newComparator.le (sortNew x)
          (sortList newComparator (List.map sortNew xs))).map _
      rw [sortList_perm _ hperm]
      simp only [List.map_cons]
      have IH : sortList newComparator (List.map (sortNew ∘ substNew σ d) xs)
          = sortList newComparator (List.map (sortNew ∘ substNew σ d) (sortList newComparator (List.map sortNew xs))) := by
        simpa only [substListNew_eq_map, sortListNew_eq_map, List.map_map] using sortListNew_subst σ d xs
      exact sortList_cons_congr newComparator (sortNew_subst σ d x) IH
  termination_by l => sizeOf l

  theorem sortListMatchCase_subst (σ : Var → Par) (d : Nat) :
      ∀ l : List MatchCase,
        sortList matchCaseComparator (sortListMatchCase (substListMatchCase σ d l))
          = sortList matchCaseComparator
              (sortListMatchCase (substListMatchCase σ d (sortList matchCaseComparator (sortListMatchCase l))))
    | [] => by simp [substListMatchCase, sortListMatchCase]
    | x :: xs => by
      simp only [substListMatchCase_eq_map, sortListMatchCase_eq_map, List.map_map, List.map_cons]
      rw [sortList_cons, sortList_cons]
      have hperm : List.Perm
          ((List.orderedInsert matchCaseComparator.le (sortMatchCase x)
            (sortList matchCaseComparator (List.map sortMatchCase xs))).map (sortMatchCase ∘ substMatchCase σ d))
          ((sortMatchCase x :: sortList matchCaseComparator (List.map sortMatchCase xs)).map (sortMatchCase ∘ substMatchCase σ d)) :=
        (List.perm_orderedInsert matchCaseComparator.le (sortMatchCase x)
          (sortList matchCaseComparator (List.map sortMatchCase xs))).map _
      rw [sortList_perm _ hperm]
      simp only [List.map_cons]
      have IH : sortList matchCaseComparator (List.map (sortMatchCase ∘ substMatchCase σ d) xs)
          = sortList matchCaseComparator (List.map (sortMatchCase ∘ substMatchCase σ d) (sortList matchCaseComparator (List.map sortMatchCase xs))) := by
        simpa only [substListMatchCase_eq_map, sortListMatchCase_eq_map, List.map_map] using sortListMatchCase_subst σ d xs
      exact sortList_cons_congr matchCaseComparator (sortMatchCase_subst σ d x) IH
  termination_by l => sizeOf l

  theorem sortListMatch_subst (σ : Var → Par) (d : Nat) :
      ∀ l : List Match,
        sortList matchComparator (sortListMatch (substListMatch σ d l))
          = sortList matchComparator
              (sortListMatch (substListMatch σ d (sortList matchComparator (sortListMatch l))))
    | [] => by simp [substListMatch, sortListMatch]
    | x :: xs => by
      simp only [substListMatch_eq_map, sortListMatch_eq_map, List.map_map, List.map_cons]
      rw [sortList_cons, sortList_cons]
      have hperm : List.Perm
          ((List.orderedInsert matchComparator.le (sortMatch x)
            (sortList matchComparator (List.map sortMatch xs))).map (sortMatch ∘ substMatch σ d))
          ((sortMatch x :: sortList matchComparator (List.map sortMatch xs)).map (sortMatch ∘ substMatch σ d)) :=
        (List.perm_orderedInsert matchComparator.le (sortMatch x)
          (sortList matchComparator (List.map sortMatch xs))).map _
      rw [sortList_perm _ hperm]
      simp only [List.map_cons]
      have IH : sortList matchComparator (List.map (sortMatch ∘ substMatch σ d) xs)
          = sortList matchComparator (List.map (sortMatch ∘ substMatch σ d) (sortList matchComparator (List.map sortMatch xs))) := by
        simpa only [substListMatch_eq_map, sortListMatch_eq_map, List.map_map] using sortListMatch_subst σ d xs
      exact sortList_cons_congr matchComparator (sortMatch_subst σ d x) IH
  termination_by l => sizeOf l

  theorem sortListBundle_subst (σ : Var → Par) (d : Nat) :
      ∀ l : List Bundle,
        sortList bundleComparator (sortListBundle (substListBundle σ d l))
          = sortList bundleComparator
              (sortListBundle (substListBundle σ d (sortList bundleComparator (sortListBundle l))))
    | [] => by simp [substListBundle, sortListBundle]
    | x :: xs => by
      simp only [substListBundle_eq_map, sortListBundle_eq_map, List.map_map, List.map_cons]
      rw [sortList_cons, sortList_cons]
      have hperm : List.Perm
          ((List.orderedInsert bundleComparator.le (sortBundle x)
            (sortList bundleComparator (List.map sortBundle xs))).map (sortBundle ∘ substBundle σ d))
          ((sortBundle x :: sortList bundleComparator (List.map sortBundle xs)).map (sortBundle ∘ substBundle σ d)) :=
        (List.perm_orderedInsert bundleComparator.le (sortBundle x)
          (sortList bundleComparator (List.map sortBundle xs))).map _
      rw [sortList_perm _ hperm]
      simp only [List.map_cons]
      have IH : sortList bundleComparator (List.map (sortBundle ∘ substBundle σ d) xs)
          = sortList bundleComparator (List.map (sortBundle ∘ substBundle σ d) (sortList bundleComparator (List.map sortBundle xs))) := by
        simpa only [substListBundle_eq_map, sortListBundle_eq_map, List.map_map] using sortListBundle_subst σ d xs
      exact sortList_cons_congr bundleComparator (sortBundle_subst σ d x) IH
  termination_by l => sizeOf l

  theorem sortListGUnforgeable_subst (σ : Var → Par) (d : Nat) :
      ∀ l : List GUnforgeable,
        sortList gUnforgeableComparator (sortListGUnforgeable l)
          = sortList gUnforgeableComparator
              (sortListGUnforgeable (sortList gUnforgeableComparator (sortListGUnforgeable l)))
    | l => by
      -- `sortGUnforgeable` is the identity (it has no `Par` to sort), so both sides are `sortList`
      -- of `l` — and `sortList C (l.map id)` is `sortList C l`.
      simp only [sortListGUnforgeable_eq_map]
      exact sortList_map_congr gUnforgeableComparator sortGUnforgeable sortGUnforgeable l
        (fun x => rfl)
  termination_by l => sizeOf l

  theorem sortListParPair_subst (σ : Var → Par) (d : Nat) :
      ∀ l : List (Par × Par),
        sortList (Comparator.cmpPair parComparator parComparator) (sortListParPair (substListParPair σ d l))
          = sortList (Comparator.cmpPair parComparator parComparator)
              (sortListParPair (substListParPair σ d
                (sortList (Comparator.cmpPair parComparator parComparator) (sortListParPair l))))
    | [] => by simp [substListParPair, sortListParPair]
    | x :: xs => by
      cases x with
      | mk a b =>
        simp only [substListParPair_eq_map, sortListParPair_eq_map, List.map_map, List.map_cons,
          Function.comp_apply]
        rw [sortList_cons, sortList_cons]
        have hperm : List.Perm
            ((List.orderedInsert (Comparator.cmpPair parComparator parComparator).le
              (sortParPair (a, b))
              (sortList (Comparator.cmpPair parComparator parComparator) (List.map sortParPair xs))).map
              (sortParPair ∘ fun y => (substPar σ d y.1, substPar σ d y.2)))
            ((sortParPair (a, b) ::
              sortList (Comparator.cmpPair parComparator parComparator) (List.map sortParPair xs)).map
              (sortParPair ∘ fun y => (substPar σ d y.1, substPar σ d y.2))) :=
          (List.perm_orderedInsert (Comparator.cmpPair parComparator parComparator).le (sortParPair (a, b))
            (sortList (Comparator.cmpPair parComparator parComparator)
              (List.map sortParPair xs))).map _
        rw [sortList_perm _ hperm]
        simp only [List.map_cons, Function.comp_apply, sortParPair, Prod.mk.injEq]
        have IH : sortList (Comparator.cmpPair parComparator parComparator)
              (List.map (sortParPair ∘ fun y => (substPar σ d y.1, substPar σ d y.2)) xs)
            = sortList (Comparator.cmpPair parComparator parComparator)
              (List.map (sortParPair ∘ fun y => (substPar σ d y.1, substPar σ d y.2))
                (sortList (Comparator.cmpPair parComparator parComparator)
                  (List.map sortParPair xs))) := by
          simpa only [substListParPair_eq_map, sortListParPair_eq_map, List.map_map] using
            sortListParPair_subst σ d xs
        exact sortList_cons_congr (Comparator.cmpPair parComparator parComparator)
          (by
            simp only [Prod.mk.injEq]
            exact ⟨sortPar_subst σ d a, sortPar_subst σ d b⟩) IH
  termination_by l => sizeOf l

  theorem sortListPar_subst (σ : Var → Par) (d : Nat) :
      ∀ l : List Par,
        sortList parComparator (sortListPar (substListPar σ d l))
          = sortList parComparator (sortListPar (substListPar σ d (sortList parComparator (sortListPar l))))
    | [] => by simp [substListPar, sortListPar]
    | x :: xs => by
      simp only [substListPar_eq_map, sortListPar_eq_map, List.map_map, List.map_cons]
      rw [sortList_cons, sortList_cons]
      have hperm : List.Perm
          ((List.orderedInsert parComparator.le (sortPar x)
            (sortList parComparator (List.map sortPar xs))).map (sortPar ∘ substPar σ d))
          ((sortPar x :: sortList parComparator (List.map sortPar xs)).map (sortPar ∘ substPar σ d)) :=
        (List.perm_orderedInsert parComparator.le (sortPar x)
          (sortList parComparator (List.map sortPar xs))).map _
      rw [sortList_perm _ hperm]
      simp only [List.map_cons]
      have IH : sortList parComparator (List.map (sortPar ∘ substPar σ d) xs)
          = sortList parComparator (List.map (sortPar ∘ substPar σ d) (sortList parComparator (List.map sortPar xs))) := by
        simpa only [substListPar_eq_map, sortListPar_eq_map, List.map_map] using sortListPar_subst σ d xs
      exact sortList_cons_congr parComparator (sortPar_subst σ d x) IH
  termination_by l => sizeOf l
end

/-! ## The record, kept: the axioms constrained nothing, and the law needed its hypothesis

These three are the finding the previous version of this file published, and they stay because the
register's row cites them. With a *definition* in place they no longer mean "the axioms are vacuous" —
they mean something narrower and still worth keeping: `noSubst` shows that the *laws alone* do not
identify substitution, which is why the definition above had to be written. -/

/-- The substitution that does nothing — the other way to satisfy the two laws. -/
def noSubst : (Var → Par) → Par → Par := fun _ t => t

/-- `noSubst` satisfies the commuting law, so that law alone does not identify `substPar`. -/
theorem the_identity_satisfies_sort_subst (σ : Var → Par) (t : Par) :
    sortPar (noSubst σ t) = noSubst σ (sortPar t) := rfl

/-- `noSubst` satisfies the closedness law too, for the same reason. -/
theorem the_identity_satisfies_subst_closed (σ : Var → Par) (t : Par)
    (hσ : ∀ v, Closed (σ v)) (h : Closed t) : Closed (noSubst σ t) := h

/-- **Why the closed-image hypothesis is not optional.** `Closed` counts a *bound* occurrence as closed
    and a *free* one as not (`Ty.lean`'s `closedVar`), so a term whose single expression is a bound
    variable is closed while the same term with a free variable is not — which is the step where an
    open `σ` image escapes into a closed term. -/
theorem bound_is_closed_free_is_not :
    Closed (Par.mk [] [] [] [Expr.evar (.bound 0)] [] [] [] []) ∧
    ¬ Closed (Par.mk [] [] [] [Expr.evar (.free 0)] [] [] [] []) := by
  refine ⟨?_, ?_⟩ <;> native_decide


/-! ### The two laws, both proved

`substPar` is a definition, so the laws are statements about a function rather than postulates about
nothing, and as of 2026-09-24 both are *theorems*: the closedness one from the checker-level `mutual`
block below, and the commuting one from the `mutual` block above, whose 22 members are one per helper
of the substitution family. -/

/-- **The commuting law**: canonicalization before or after substitution gives the same term, both sides
    sorted. This is the port's `law3_substitution_and_sorting_commute`
    (`rholang/src/property_tests.rs`) — with `sortPar` on both sides because `substPar` here is the
    *no-sort* core the port's `substitute_par_no_sort` is (`substitute.rs:116`), while its public entry
    point sorts once at the end (`:169`).

    **Proved** on 2026-09-24 by the `mutual` block above, which states the same law one type at a time
    (`sortPar_subst`, `sortSend_subst`, …); this is its depth-`0` instance. A substituted occurrence
    contributes a whole `Par` (`parMerge`), which is why both sides are sorted — the splice appends a
    value's fields after whatever the target already had, and the final sort is what makes the order
    irrelevant. -/
theorem sort_subst (σ : Var → Par) (t : Par) :
    sortPar (substPar σ 0 t) = sortPar (substPar σ 0 (sortPar t)) :=
  sortPar_subst σ 0 t

/-! ### `subst_closed`, attempted again — where it now stands (2026-09-23, Programme D unit 12)

The closure law was attempted a second time, on top of what this file now has (the definition) and what
`Rchain.Ty` gained in the same session (`closed_sortPar`: canonicalization preserves closedness, in the
checker-level *equation* form the collection arms need — `substExprToPar` sorts a set's and a map's
children *inside* its recursion).

**What closed.** The whole mutual block over the substitution family (21 members) is *stated* and
type-checks as a `mutual ... termination_by sizeOf` block in the `sortPar_idempotent` template, with σ
and the closed-image hypothesis threaded explicitly (they must be parameters, not auto-bound per member,
or the mutual recursion does not typecheck). The structural members — the ten list walks, `substNew`,
`substBundle`, the `parMerge` composition in `substPar` — close, as do the vacuous case (`.evar (.free k)`
contradicts the hypothesis) and the two `sortListPar`/`sortListParPair` absorptions.

**The obstruction, confirmed and removed.** The closure checkers in `Ty.lean` (`closed`, `closedListPar`,
…) carried `termination_by` clauses, which made them **well-founded** and therefore non-reducing: a
rewrite had to go through `closedListX.eq_def`, `rw [closedListX]` failed outright, and the `&&`-chain a
`closedListX` unfolding leaves on a *goal* is left-nested where `Bool.and_eq_true` wants it
right-nested. The clauses were **not needed** — the family is structural (every arm takes a field or an
element) and Lean infers that on its own — so they are gone, the checkers reduce by `rfl`/`rw` again, and
the register builds unchanged. That is the same trade `Match.lean` records for the matcher, and the same
conclusion: an annotation that costs kernel reduction is worth having only when something needs it.

**What remains, and it is not the unfolding — it is the measure.** Twenty-one members are stated and
type-check, the structural walks and the `parMerge` composition close, and the collection arms are
settled. What blocks the block is that its recursion spans *types*: `substPar_closed` calls
`substExprsToPar_closed` (a `Par` calling a `List Expr`), and Lean's structural checker skips those
parameters — "Skipping arguments of type `List Bundle`, as `substPar_closed` has no compatible
argument" — so no structural descent is inferred; and with `termination_by d x => sizeOf x` the
obligations are `sizeOf x < sizeOf l` for an element or a subterm, which the default tactic cannot
discharge without the membership facts (`List.sizeOf_lt_of_mem`) and which a `rw`-extracted hypothesis
hides behind the `Eq.mp` that `Bool.and_eq_true` leaves.

The way through is already recorded in this repository, for the comparator family: **one theorem over
the sum type** (`Par ⊕ Send ⊕ … ⊕ List (Par × Par)`) proved by strong induction on `sizeOf`, with the
family's members as its cases — `Sort.lean`'s note names exactly that ("a single well-founded recursion
over a sum type"), and it removes both obstacles at once: the measure is a single `Nat` inequality that
`omega` can finish, and no structural inference is needed at all. **And it is done, by a route this note did not predict (2026-09-24).** The
*sum type* is not needed: the mutual-**theorem** block `Sort.lean` uses for its comparator laws carries
this family too. What the earlier attempts were missing is a shape, not a measure — and the two rules
are recorded because each cost a cycle:

* the `termination_by` clause must name a **prefix of the equation patterns**, so the members are written
  with *variable* patterns and the destructuring inside (`| t, h => by cases t with | mk s r n e m u b c
  => …`) with `termination_by t _ => sizeOf t`. Written the other way round — patterns that destructure,
  a clause naming the `∀`-binder — Lean reports `5 parameters bound in termination_by, but the body …
  only binds 2`, and it is easy to read that as the measure being wrong;
* every recursive call must then be on a *destructured* subterm, which the `cases` supplies.

The budget: the proof needs `set_option maxHeartbeats 1000000` (the default 200000 is not enough for the
`closed`/`Closed` iff over 24 predicates on the substituted terms — measured, not guessed; `Sort.lean`
carries the same kind of measurement).
-/

/-- A `Par` with one expression is closed exactly when that expression is. -/
private theorem closed_singleExpr_iff (e : Expr) : Closed (singleExpr e) ↔ closedExpr e = true := by
  simp [Closed, singleExpr, closedListSend, closedListReceive, closedListNew, closedListExpr,
        closedListMatch, closedListGUnforgeable, closedListBundle, closedListConnective]

/-- A `Par` with one connective is closed exactly when that connective is. -/
private theorem closed_singleConnective_iff (c : Connective) :
    Closed (singleConnective c) ↔ closedConnective c = true := by
  simp [Closed, singleConnective, closedListSend, closedListReceive, closedListNew, closedListExpr,
        closedListMatch, closedListGUnforgeable, closedListBundle, closedListConnective]


private theorem closedListPar_map_sortPar (l : List Par) :
    closedListPar (l.map sortPar) = closedListPar l := by
  rw [closedListPar_all, closedListPar_all, List.all_map]
  exact all_congr l (fun x _ => closed_sortPar x)

/-- The same for a list of pairs. -/
private theorem closedListParPair_map_sortParPair (l : List (Par × Par)) :
    closedListParPair (l.map sortParPair) = closedListParPair l := by
  rw [closedListParPair_all, closedListParPair_all, List.all_map]
  exact all_congr l (fun x _ => closed_sortParPair x)

-- Measured, not guessed: the `closed`/`Closed` iff over 24 predicates on the substituted terms needs
-- more than the default 200000 heartbeats. `Sort.lean` carries the same kind of note for its own budget.
set_option maxHeartbeats 1000000 in

mutual
  theorem substPar_closed (σ : Var → Par) (hσ : ∀ v, Closed (σ v)) (d : Nat) :
      ∀ t : Par, Closed t → Closed (substPar σ d t)
    | t, h => by
      cases t with
      | mk s r n e m u b c =>
        simp only [Closed] at h
        obtain ⟨hs, hr, hn, he, hm, hu, hb, hc⟩ := h
        simp only [substPar]
        rw [Closed_parMerge_iff, Closed_parMerge_iff]
        refine ⟨⟨substExprsToPar_closed σ hσ d e he, substListConnective_closed σ hσ d c hc⟩, ?_⟩
        simp only [Closed]
        exact ⟨substListSend_closed σ hσ d s hs, substListReceive_closed σ hσ d r hr,
               substListNew_closed σ hσ d n hn, by simp [closedListExpr],
               substListMatch_closed σ hσ d m hm, hu,
               substListBundle_closed σ hσ d b hb, by simp [closedListConnective]⟩
  termination_by t _ => sizeOf t

  theorem substSend_closed (σ : Var → Par) (hσ : ∀ v, Closed (σ v)) (d : Nat) :
      ∀ x : Send, closedSend x = true → closedSend (substSend σ d x) = true
    | x, h => by
      cases x with
      | mk ch data p =>
        simp only [closedSend, Bool.and_eq_true] at h
        simp only [substSend, closedSend, Bool.and_eq_true]
        exact ⟨(by simpa [closed_eq_Closed] using substPar_closed σ hσ d ch (by simpa [closed_eq_Closed] using h.1)), substListPar_closed σ hσ d data h.2⟩
  termination_by x _ => sizeOf x

  theorem substReceiveBind_closed (σ : Var → Par) (hσ : ∀ v, Closed (σ v)) (d : Nat) :
      ∀ x : ReceiveBind, closedReceiveBind x = true → closedReceiveBind (substReceiveBind σ d x) = true
    | x, h => by
      cases x with
      | mk ps src fc =>
        simp only [closedReceiveBind, Bool.and_eq_true] at h
        simp only [substReceiveBind, closedReceiveBind, Bool.and_eq_true]
        exact ⟨substListPar_closed σ hσ (d + 1) ps h.1, (by simpa [closed_eq_Closed] using substPar_closed σ hσ d src (by simpa [closed_eq_Closed] using h.2))⟩
  termination_by x _ => sizeOf x

  theorem substReceive_closed (σ : Var → Par) (hσ : ∀ v, Closed (σ v)) (d : Nat) :
      ∀ x : Receive, closedReceive x = true → closedReceive (substReceive σ d x) = true
    | x, h => by
      cases x with
      | mk binds body p n =>
        simp only [closedReceive, Bool.and_eq_true] at h
        simp only [substReceive, closedReceive, Bool.and_eq_true]
        exact ⟨substListReceiveBind_closed σ hσ d binds h.1, (by simpa [closed_eq_Closed] using substPar_closed σ hσ d body (by simpa [closed_eq_Closed] using h.2))⟩
  termination_by x _ => sizeOf x

  theorem substNew_closed (σ : Var → Par) (hσ : ∀ v, Closed (σ v)) (d : Nat) :
      ∀ x : New, closedNew x = true → closedNew (substNew σ d x) = true
    | x, h => by
      cases x with
      | mk n body =>
        simp only [closedNew] at h ⊢
        simp only [substNew]
        exact (by simpa [closed_eq_Closed] using substPar_closed σ hσ d body (by simpa [closed_eq_Closed] using h))
  termination_by x _ => sizeOf x

  theorem substMatchCase_closed (σ : Var → Par) (hσ : ∀ v, Closed (σ v)) (d : Nat) :
      ∀ x : MatchCase, closedMatchCase x = true → closedMatchCase (substMatchCase σ d x) = true
    | x, h => by
      cases x with
      | mk p src fc =>
        simp only [closedMatchCase, Bool.and_eq_true] at h
        simp only [substMatchCase, closedMatchCase, Bool.and_eq_true]
        exact ⟨(by simpa [closed_eq_Closed] using substPar_closed σ hσ (d + 1) p (by simpa [closed_eq_Closed] using h.1)), (by simpa [closed_eq_Closed] using substPar_closed σ hσ d src (by simpa [closed_eq_Closed] using h.2))⟩
  termination_by x _ => sizeOf x

  theorem substMatch_closed (σ : Var → Par) (hσ : ∀ v, Closed (σ v)) (d : Nat) :
      ∀ x : Match, closedMatch x = true → closedMatch (substMatch σ d x) = true
    | x, h => by
      cases x with
      | mk t cs =>
        simp only [closedMatch, Bool.and_eq_true] at h
        simp only [substMatch, closedMatch, Bool.and_eq_true]
        exact ⟨(by simpa [closed_eq_Closed] using substPar_closed σ hσ d t (by simpa [closed_eq_Closed] using h.1)), substListMatchCase_closed σ hσ d cs h.2⟩
  termination_by x _ => sizeOf x

  theorem substBundle_closed (σ : Var → Par) (hσ : ∀ v, Closed (σ v)) (d : Nat) :
      ∀ x : Bundle, closedBundle x = true → closedBundle (substBundle σ d x) = true
    | x, h => by
      cases x with
      | mk body w r =>
        simp only [closedBundle] at h ⊢
        simp only [substBundle]
        exact (by simpa [closed_eq_Closed] using substPar_closed σ hσ d body (by simpa [closed_eq_Closed] using h))
  termination_by x _ => sizeOf x

  theorem substConnective_closed (σ : Var → Par) (hσ : ∀ v, Closed (σ v)) (d : Nat) :
      ∀ c : Connective, closedConnective c = true → Closed (substConnective σ d c)
    | c, h => by
      cases c with
      | connAnd ps =>
        simp only [substConnective, closedConnective] at h ⊢
        rw [closed_singleConnective_iff]
        simp only [closedConnective]
        exact substListPar_closed σ hσ d ps h
      | connOr ps =>
        simp only [substConnective, closedConnective] at h ⊢
        rw [closed_singleConnective_iff]
        simp only [closedConnective]
        exact substListPar_closed σ hσ d ps h
      | connNot p =>
        simp only [substConnective, closedConnective] at h ⊢
        rw [closed_singleConnective_iff]
        simp only [closedConnective]
        exact (by simpa [closed_eq_Closed] using substPar_closed σ hσ d p (by simpa [closed_eq_Closed] using h))
      | connVarRef dep k =>
        simp only [substConnective]
        by_cases hd : dep = 0
        · simp only [hd, if_true]
          exact hσ (.bound k)
        · simp only [hd, if_false]
          rw [closed_singleConnective_iff]
          simp [closedConnective]
  termination_by c _ => sizeOf c

  theorem substListConnective_closed (σ : Var → Par) (hσ : ∀ v, Closed (σ v)) (d : Nat) :
      ∀ l : List Connective, closedListConnective l = true → Closed (substListConnective σ d l)
    | [], _ => by simp [substListConnective, Closed_nil]
    | c :: cs, h => by
      simp only [closedListConnective, Bool.and_eq_true] at h
      simp only [substListConnective]
      exact Closed_parMerge (substConnective_closed σ hσ d c h.1)
        (substListConnective_closed σ hσ d cs h.2)
  termination_by l _ => sizeOf l

  theorem substExprsToPar_closed (σ : Var → Par) (hσ : ∀ v, Closed (σ v)) (d : Nat) :
      ∀ l : List Expr, closedListExpr l = true → Closed (substExprsToPar σ d l)
    | [], _ => by simp [substExprsToPar, Closed_nil]
    | e :: es, h => by
      simp only [closedListExpr, Bool.and_eq_true] at h
      simp only [substExprsToPar]
      exact Closed_parMerge (substExprToPar_closed σ hσ d e h.1)
        (substExprsToPar_closed σ hσ d es h.2)
  termination_by l _ => sizeOf l

  theorem substListSend_closed (σ : Var → Par) (hσ : ∀ v, Closed (σ v)) (d : Nat) :
      ∀ l : List Send, closedListSend l = true → closedListSend (substListSend σ d l) = true
    | [], _ => by simp [substListSend, closedListSend]
    | x :: xs, h => by
      simp only [closedListSend, Bool.and_eq_true] at h
      simp only [substListSend, closedListSend, Bool.and_eq_true]
      exact ⟨substSend_closed σ hσ d x h.1, substListSend_closed σ hσ d xs h.2⟩
  termination_by l _ => sizeOf l

  theorem substListReceive_closed (σ : Var → Par) (hσ : ∀ v, Closed (σ v)) (d : Nat) :
      ∀ l : List Receive, closedListReceive l = true → closedListReceive (substListReceive σ d l) = true
    | [], _ => by simp [substListReceive, closedListReceive]
    | x :: xs, h => by
      simp only [closedListReceive, Bool.and_eq_true] at h
      simp only [substListReceive, closedListReceive, Bool.and_eq_true]
      exact ⟨substReceive_closed σ hσ d x h.1, substListReceive_closed σ hσ d xs h.2⟩
  termination_by l _ => sizeOf l

  theorem substListReceiveBind_closed (σ : Var → Par) (hσ : ∀ v, Closed (σ v)) (d : Nat) :
      ∀ l : List ReceiveBind, closedListReceiveBind l = true →
        closedListReceiveBind (substListReceiveBind σ d l) = true
    | [], _ => by simp [substListReceiveBind, closedListReceiveBind]
    | x :: xs, h => by
      simp only [closedListReceiveBind, Bool.and_eq_true] at h
      simp only [substListReceiveBind, closedListReceiveBind, Bool.and_eq_true]
      exact ⟨substReceiveBind_closed σ hσ d x h.1, substListReceiveBind_closed σ hσ d xs h.2⟩
  termination_by l _ => sizeOf l

  theorem substListNew_closed (σ : Var → Par) (hσ : ∀ v, Closed (σ v)) (d : Nat) :
      ∀ l : List New, closedListNew l = true → closedListNew (substListNew σ d l) = true
    | [], _ => by simp [substListNew, closedListNew]
    | x :: xs, h => by
      simp only [closedListNew, Bool.and_eq_true] at h
      simp only [substListNew, closedListNew, Bool.and_eq_true]
      exact ⟨substNew_closed σ hσ d x h.1, substListNew_closed σ hσ d xs h.2⟩
  termination_by l _ => sizeOf l

  theorem substListMatch_closed (σ : Var → Par) (hσ : ∀ v, Closed (σ v)) (d : Nat) :
      ∀ l : List Match, closedListMatch l = true → closedListMatch (substListMatch σ d l) = true
    | [], _ => by simp [substListMatch, closedListMatch]
    | x :: xs, h => by
      simp only [closedListMatch, Bool.and_eq_true] at h
      simp only [substListMatch, closedListMatch, Bool.and_eq_true]
      exact ⟨substMatch_closed σ hσ d x h.1, substListMatch_closed σ hσ d xs h.2⟩
  termination_by l _ => sizeOf l

  theorem substListMatchCase_closed (σ : Var → Par) (hσ : ∀ v, Closed (σ v)) (d : Nat) :
      ∀ l : List MatchCase, closedListMatchCase l = true →
        closedListMatchCase (substListMatchCase σ d l) = true
    | [], _ => by simp [substListMatchCase, closedListMatchCase]
    | x :: xs, h => by
      simp only [closedListMatchCase, Bool.and_eq_true] at h
      simp only [substListMatchCase, closedListMatchCase, Bool.and_eq_true]
      exact ⟨substMatchCase_closed σ hσ d x h.1, substListMatchCase_closed σ hσ d xs h.2⟩
  termination_by l _ => sizeOf l

  theorem substListBundle_closed (σ : Var → Par) (hσ : ∀ v, Closed (σ v)) (d : Nat) :
      ∀ l : List Bundle, closedListBundle l = true → closedListBundle (substListBundle σ d l) = true
    | [], _ => by simp [substListBundle, closedListBundle]
    | x :: xs, h => by
      simp only [closedListBundle, Bool.and_eq_true] at h
      simp only [substListBundle, closedListBundle, Bool.and_eq_true]
      exact ⟨substBundle_closed σ hσ d x h.1, substListBundle_closed σ hσ d xs h.2⟩
  termination_by l _ => sizeOf l
  theorem substListPar_closed (σ : Var → Par) (hσ : ∀ v, Closed (σ v)) (d : Nat) :
      ∀ l : List Par, closedListPar l = true → closedListPar (substListPar σ d l) = true
    | [], _ => by simp [substListPar, closedListPar]
    | x :: xs, h => by
      simp only [closedListPar, Bool.and_eq_true] at h
      simp only [substListPar, closedListPar, Bool.and_eq_true]
      exact ⟨by simpa [closed_eq_Closed] using
               (by simpa [closed_eq_Closed] using substPar_closed σ hσ d x (by simpa [closed_eq_Closed] using h.1)),
             substListPar_closed σ hσ d xs h.2⟩
  termination_by l _ => sizeOf l

  theorem substListParPair_closed (σ : Var → Par) (hσ : ∀ v, Closed (σ v)) (d : Nat) :
      ∀ l : List (Par × Par), closedListParPair l = true →
        closedListParPair (substListParPair σ d l) = true
    | [], _ => by simp [substListParPair, closedListParPair]
    | (a, b) :: xs, h => by
      simp only [closedListParPair, Bool.and_eq_true] at h
      simp only [substListParPair, closedListParPair, Bool.and_eq_true]
      exact ⟨⟨by simpa [closed_eq_Closed] using
                (by simpa [closed_eq_Closed] using substPar_closed σ hσ d a (by simpa [closed_eq_Closed] using h.1.1)),
              by simpa [closed_eq_Closed] using
                (by simpa [closed_eq_Closed] using substPar_closed σ hσ d b (by simpa [closed_eq_Closed] using h.1.2))⟩,
             substListParPair_closed σ hσ d xs h.2⟩
  termination_by l _ => sizeOf l

  theorem substExprToPar_closed (σ : Var → Par) (hσ : ∀ v, Closed (σ v)) (d : Nat) :
      ∀ e : Expr, closedExpr e = true → Closed (substExprToPar σ d e)
    | .ground g, _ => by
      simp only [substExprToPar]; rw [closed_singleExpr_iff]; simp [closedExpr]
    | .evar v, h => by
      cases v with
      | bound k =>
        simp only [substExprToPar]
        by_cases hd : d = 0
        · simp only [hd, if_true]; exact hσ (.bound k)
        · simp only [hd, if_false]; rw [closed_singleExpr_iff]; simp [closedExpr, closedVar]
      | free k =>
        simp only [substExprToPar]; rw [closed_singleExpr_iff]; simp [closedExpr, closedVar] at h ⊢
      | wildcard =>
        simp only [substExprToPar]; rw [closed_singleExpr_iff]; simp [closedExpr, closedVar]
    | .eneg p, h => by
      simp only [substExprToPar]; rw [closed_singleExpr_iff]
      simp only [closedExpr] at h ⊢
      exact (by simpa [closed_eq_Closed] using substPar_closed σ hσ d p (by simpa [closed_eq_Closed] using h))
    | .enot p, h => by
      simp only [substExprToPar]; rw [closed_singleExpr_iff]
      simp only [closedExpr] at h ⊢
      exact (by simpa [closed_eq_Closed] using substPar_closed σ hσ d p (by simpa [closed_eq_Closed] using h))
    | .eplus a b, h => by
      simp only [substExprToPar]; rw [closed_singleExpr_iff]
      simp only [closedExpr, Bool.and_eq_true] at h ⊢
      exact ⟨(by simpa [closed_eq_Closed] using substPar_closed σ hσ d a (by simpa [closed_eq_Closed] using h.1)), (by simpa [closed_eq_Closed] using substPar_closed σ hσ d b (by simpa [closed_eq_Closed] using h.2))⟩
    | .eminus a b, h => by
      simp only [substExprToPar]; rw [closed_singleExpr_iff]
      simp only [closedExpr, Bool.and_eq_true] at h ⊢
      exact ⟨(by simpa [closed_eq_Closed] using substPar_closed σ hσ d a (by simpa [closed_eq_Closed] using h.1)), (by simpa [closed_eq_Closed] using substPar_closed σ hσ d b (by simpa [closed_eq_Closed] using h.2))⟩
    | .emult a b, h => by
      simp only [substExprToPar]; rw [closed_singleExpr_iff]
      simp only [closedExpr, Bool.and_eq_true] at h ⊢
      exact ⟨(by simpa [closed_eq_Closed] using substPar_closed σ hσ d a (by simpa [closed_eq_Closed] using h.1)), (by simpa [closed_eq_Closed] using substPar_closed σ hσ d b (by simpa [closed_eq_Closed] using h.2))⟩
    | .ediv a b, h => by
      simp only [substExprToPar]; rw [closed_singleExpr_iff]
      simp only [closedExpr, Bool.and_eq_true] at h ⊢
      exact ⟨(by simpa [closed_eq_Closed] using substPar_closed σ hσ d a (by simpa [closed_eq_Closed] using h.1)), (by simpa [closed_eq_Closed] using substPar_closed σ hσ d b (by simpa [closed_eq_Closed] using h.2))⟩
    | .emod a b, h => by
      simp only [substExprToPar]; rw [closed_singleExpr_iff]
      simp only [closedExpr, Bool.and_eq_true] at h ⊢
      exact ⟨(by simpa [closed_eq_Closed] using substPar_closed σ hσ d a (by simpa [closed_eq_Closed] using h.1)), (by simpa [closed_eq_Closed] using substPar_closed σ hσ d b (by simpa [closed_eq_Closed] using h.2))⟩
    | .elt a b, h => by
      simp only [substExprToPar]; rw [closed_singleExpr_iff]
      simp only [closedExpr, Bool.and_eq_true] at h ⊢
      exact ⟨(by simpa [closed_eq_Closed] using substPar_closed σ hσ d a (by simpa [closed_eq_Closed] using h.1)), (by simpa [closed_eq_Closed] using substPar_closed σ hσ d b (by simpa [closed_eq_Closed] using h.2))⟩
    | .ele a b, h => by
      simp only [substExprToPar]; rw [closed_singleExpr_iff]
      simp only [closedExpr, Bool.and_eq_true] at h ⊢
      exact ⟨(by simpa [closed_eq_Closed] using substPar_closed σ hσ d a (by simpa [closed_eq_Closed] using h.1)), (by simpa [closed_eq_Closed] using substPar_closed σ hσ d b (by simpa [closed_eq_Closed] using h.2))⟩
    | .egt a b, h => by
      simp only [substExprToPar]; rw [closed_singleExpr_iff]
      simp only [closedExpr, Bool.and_eq_true] at h ⊢
      exact ⟨(by simpa [closed_eq_Closed] using substPar_closed σ hσ d a (by simpa [closed_eq_Closed] using h.1)), (by simpa [closed_eq_Closed] using substPar_closed σ hσ d b (by simpa [closed_eq_Closed] using h.2))⟩
    | .ege a b, h => by
      simp only [substExprToPar]; rw [closed_singleExpr_iff]
      simp only [closedExpr, Bool.and_eq_true] at h ⊢
      exact ⟨(by simpa [closed_eq_Closed] using substPar_closed σ hσ d a (by simpa [closed_eq_Closed] using h.1)), (by simpa [closed_eq_Closed] using substPar_closed σ hσ d b (by simpa [closed_eq_Closed] using h.2))⟩
    | .eeq a b, h => by
      simp only [substExprToPar]; rw [closed_singleExpr_iff]
      simp only [closedExpr, Bool.and_eq_true] at h ⊢
      exact ⟨(by simpa [closed_eq_Closed] using substPar_closed σ hσ d a (by simpa [closed_eq_Closed] using h.1)), (by simpa [closed_eq_Closed] using substPar_closed σ hσ d b (by simpa [closed_eq_Closed] using h.2))⟩
    | .eneq a b, h => by
      simp only [substExprToPar]; rw [closed_singleExpr_iff]
      simp only [closedExpr, Bool.and_eq_true] at h ⊢
      exact ⟨(by simpa [closed_eq_Closed] using substPar_closed σ hσ d a (by simpa [closed_eq_Closed] using h.1)), (by simpa [closed_eq_Closed] using substPar_closed σ hσ d b (by simpa [closed_eq_Closed] using h.2))⟩
    | .eand a b, h => by
      simp only [substExprToPar]; rw [closed_singleExpr_iff]
      simp only [closedExpr, Bool.and_eq_true] at h ⊢
      exact ⟨(by simpa [closed_eq_Closed] using substPar_closed σ hσ d a (by simpa [closed_eq_Closed] using h.1)), (by simpa [closed_eq_Closed] using substPar_closed σ hσ d b (by simpa [closed_eq_Closed] using h.2))⟩
    | .eor a b, h => by
      simp only [substExprToPar]; rw [closed_singleExpr_iff]
      simp only [closedExpr, Bool.and_eq_true] at h ⊢
      exact ⟨(by simpa [closed_eq_Closed] using substPar_closed σ hσ d a (by simpa [closed_eq_Closed] using h.1)), (by simpa [closed_eq_Closed] using substPar_closed σ hσ d b (by simpa [closed_eq_Closed] using h.2))⟩
    | .elist ps r, h => by
      simp only [substExprToPar]; rw [closed_singleExpr_iff]
      simp only [closedExpr, Bool.and_eq_true] at h ⊢
      exact ⟨substListPar_closed σ hσ d ps h.1, h.2⟩
    | .etuple ps, h => by
      simp only [substExprToPar]; rw [closed_singleExpr_iff]
      simp only [closedExpr] at h ⊢
      exact substListPar_closed σ hσ d ps h
    | .eset ps r, h => by
      simp only [substExprToPar]; rw [closed_singleExpr_iff]
      simp only [closedExpr, Bool.and_eq_true] at h ⊢
      refine ⟨?_, h.2⟩
      rw [sortListPar_eq_map, closedListPar_map_sortPar]
      exact substListPar_closed σ hσ d ps h.1
    | .emap kvs r, h => by
      simp only [substExprToPar]; rw [closed_singleExpr_iff]
      simp only [closedExpr, Bool.and_eq_true] at h ⊢
      refine ⟨?_, h.2⟩
      rw [sortListParPair_eq_map, closedListParPair_map_sortParPair]
      exact substListParPair_closed σ hσ d kvs h.1
  termination_by e _ => sizeOf e
end

/-- **The closedness law, with the closed-image hypothesis it needs** — a **theorem about the
    definition** since 2026-09-24, with the statement the axiom carried unchanged (a changed statement
    is a changed law). `σ`'s values must be closed: `σ := fun _ => free 0` makes the conclusion false of
    any operation that substitutes at a bound occurrence, and the Rust's own test carries exactly this
    hypothesis (`law3_substituting_a_closed_value_keeps_the_term_closed`, whose value is `arb_closed`).

    The block above proves it at the checker level (`closed … = true`, the same shape `Ty.lean`'s
    `closed*` block uses); this wrapper states it in the `Closed` form the law is written in. -/
theorem subst_closed (σ : Var → Par) (hσ : ∀ v, Closed (σ v)) (d : Nat) (t : Par) (h : Closed t) :
    Closed (substPar σ d t) :=
  substPar_closed σ hσ d t h

end Rchain
