import Rchain.Rho
import Rchain.Ty
import Rchain.FreeVars

/-!
# Law 4 — reduction (COMM), first-match-wins, `new` freshness

The COMM contraction and the `|`-congruence are the `Reduce` relation in `Rchain.Rho`; closedness
preservation under `⟶` is proven in `Rchain.Ty` (`reduce_closed`). This module states the remaining
two clauses of Law 4: reduction is **deterministic** (first-match-wins) and `new` yields **fresh**
unforgeable names. The Scala oracle is `Reduce.scala`; the Rust realization is
`rholang::reduce::DebruijnInterpreter`.
-/

namespace Rchain

/- Law 4 (corrected): reduction is **not** even single-step deterministic up to `StrCong`, and the
   flat `Par` is **not confluent** either — a term with one receive and two sends on one channel is a
   redex in two ways (see `Rchain.Concurrent.reduce_not_deterministic`). What *does* hold on the flat
   `Par` is that an *isolated* redex reduces uniquely up to `StrCong`
   (`Rchain.Concurrent.reduce_redex_unique`). Full confluence is a property of the tree model, not of
   the field-wise flat `Par`. -/

/-- A receive's body is one of the things a free variable can be free *in*: `receivePar chan body`
    holds its body in the `Receive` field, so the levels free in the body are free in the term. The
    `comm` arm of law 4b needs exactly this — the contract consumes a send and a receive and leaves
    the body, and the body is the only source of free variables on either side. -/
theorem freeVarOf_receivePar {chan body : Par} {k : Nat} (h : freeVarOf body k) :
    freeVarOf (receivePar chan body) k := by
  simp only [freeVarOf, receivePar, freeVarOfListSend, freeVarOfListReceive, freeVarOfReceive,
    freeVarOfListReceiveBind, freeVarOfReceiveBind, freeVarOfListPar, freeVarOfListExpr,
    freeVarOfExpr, freeVarAt, freeVarOfListNew, freeVarOfListMatch, freeVarOfListGUnforgeable,
    freeVarOfListBundle, freeVarOfListConnective]
  tauto

/-- Law 4b: `new` binds names that are fresh — reduction cannot mention a `new`-bound name outside
    its binder, so a fresh name never clashes with an existing one. Phrased as: the free variables
    of the reduct are a subset of the free variables of the redex.

    An induction on the derivation. `comm` is the contract step, and it is where the law has content:
    the contracted redex is `send | receive` and the reduct is the receive's *body*, which is a
    component of the redex — so every level free in the reduct is free in the redex. The two
    congruence arms are `freeVarOf_parMerge` in both directions: a step happens on one side of a `|`,
    and a free level in `p' | q` is free in `p'` or in `q`, the first of which the induction
    hypothesis lifts back to `p`. -/
theorem reduce_freeVars_subset {p q : Par} (h : Reduce p q) : ∀ n, freeVarOf q n → freeVarOf p n := by
  induction h with
  | comm chan data body =>
      intro n hq
      exact (freeVarOf_parMerge _ _ n).mpr (Or.inr (freeVarOf_receivePar hq))
  | parLeft _h ih =>
      intro n hq
      rcases (freeVarOf_parMerge _ _ n).mp hq with h1 | h2
      · exact (freeVarOf_parMerge _ _ n).mpr (Or.inl (ih n h1))
      · exact (freeVarOf_parMerge _ _ n).mpr (Or.inr h2)
  | parRight _h ih =>
      intro n hq
      rcases (freeVarOf_parMerge _ _ n).mp hq with h1 | h2
      · exact (freeVarOf_parMerge _ _ n).mpr (Or.inl h1)
      · exact (freeVarOf_parMerge _ _ n).mpr (Or.inr (ih n h2))

end Rchain
