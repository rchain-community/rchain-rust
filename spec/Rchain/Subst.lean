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

/-- Deep capture-avoiding de Bruijn substitution: replace every bound occurrence resolved by `σ`,
    shifting levels through binders. **Stated, not defined** — see the note above; the definition is
    Programme D's unit 5, mirroring `rholang/src/substitute.rs:164`. -/
axiom substPar (σ : Var → Par) : Par → Par

/-- Law 3: substitution commutes with canonicalization (`sort(subst t) = subst(sort t)`). -/
axiom sort_subst (σ : Var → Par) (t : Par) : sortPar (substPar σ t) = substPar σ (sortPar t)

/-- Substitution preserves closedness, **given a closed image**: `σ`'s values must themselves be
    closed, which is the hypothesis the earlier statement of this axiom lacked (`σ := fun _ => free 0`
    makes the conclusion false of any operation that substitutes at a bound occurrence — see
    `bound_is_closed_free_is_not`) and which the Rust's own test carries. -/
axiom subst_closed (σ : Var → Par) (t : Par) (hσ : ∀ v, Closed (σ v)) : Closed t → Closed (substPar σ t)

/-! ## The trio is satisfied by a function that substitutes nothing -/

/-- The substitution that does nothing — the other way to satisfy every axiom in this file. Its
    existence is the finding, not a curiosity: it means the three postulates above do not say that
    substitution substitutes. -/
def noSubst : (Var → Par) → Par → Par := fun _ t => t

/-- `noSubst` satisfies `sort_subst`, so that axiom does not identify `substPar`. -/
theorem the_identity_satisfies_sort_subst (σ : Var → Par) (t : Par) :
    sortPar (noSubst σ t) = noSubst σ (sortPar t) := rfl

/-- `noSubst` satisfies the narrowed `subst_closed` too, for the same reason. -/
theorem the_identity_satisfies_subst_closed (σ : Var → Par) (t : Par)
    (hσ : ∀ v, Closed (σ v)) (h : Closed t) : Closed (noSubst σ t) := h

/-- **Why the closed-image hypothesis is not optional.** `Closed` counts a *bound* occurrence as
    closed and a *free* one as not (`Ty.lean`'s `closedVar`), so a term whose single expression is a
    bound variable is closed while the same term with a free variable is not — which is the step
    where an open `σ` image escapes into a closed term. -/
theorem bound_is_closed_free_is_not :
    Closed (Par.mk [] [] [] [Expr.evar (.bound 0)] [] [] [] []) ∧
    ¬ Closed (Par.mk [] [] [] [Expr.evar (.free 0)] [] [] [] []) := by
  refine ⟨?_, ?_⟩ <;> native_decide

end Rchain
