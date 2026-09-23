(* Laws 2–6 statements over the flat `Par` — the Coq substitution/α-equivalence metatheory.

   Coq owns the *definitions* (capture-avoiding de Bruijn substitution, α-equivalence) per
   `AGENTS.md`; this file states each law's signature precisely (axiomatized) over `Syntax.v` so the
   catalog is complete. The definitions themselves remain Phase-1 obligations (Autosubst-style).

   **The trust surface here is gated, and it is a ceiling rather than zero.** Nothing used to read
   this file except `make`: the gate checked that it compiled and not what it *assumed*. It now
   counts the `Axiom`/`Parameter`/`Conjecture`/`Admitted` declarations under `spec/coq/` against
   `COQ_AXIOM_CEILING` in `tools/check-lean-conformance.sh`, and prints the count every run, so the
   number can only move down deliberately.

   `binds_at_most_once` was **deleted** (2026-09-23): it stated a proposition Lean had found *false*
   as written (AUDIT C26). An `Axiom` for a false proposition is worse than no statement at all — it
   makes the file unsound for the row it claims — and the honest Coq equivalent is Lean's `linear`
   predicate, which is a *definition*. *)

From Rchain Require Import Syntax Sort.

(* ------------------------------------------------------------------ *)
(* Law 2 — α / name equivalence.                                       *)
(* ------------------------------------------------------------------ *)

(* `alpha_equiv p q` — p and q are equal up to bound-variable renaming (and the quote/eval
   structural equalities of `name-equivalence.k`). *)
Axiom alpha_equiv : Par -> Par -> Prop.
Axiom alpha_equiv_refl : forall p : Par, alpha_equiv p p.
Axiom alpha_equiv_symm : forall p q : Par, alpha_equiv p q -> alpha_equiv q p.
Axiom alpha_equiv_trans : forall p q r : Par, alpha_equiv p q -> alpha_equiv q r -> alpha_equiv p r.

(* ------------------------------------------------------------------ *)
(* Law 3 — capture-avoiding de Bruijn substitution.                    *)
(* ------------------------------------------------------------------ *)

(* A simultaneous substitution: one `Par` per de Bruijn level. *)
Definition Subst : Type := Var -> Par.

(* `substPar σ p` — substitute every free occurrence of a level by its image under σ, shifting
   bound levels through binders (capture-avoiding). *)
Axiom substPar : Subst -> Par -> Par.

(* Law 3: canonicalization commutes with substitution (`sort(subst t) = subst(sort t)`). *)
Axiom subst_commutes_sort :
  forall (σ : Subst) (p : Par), sortPar (substPar σ p) = substPar σ (sortPar p).

(* ------------------------------------------------------------------ *)
(* Law 4 — reduction (COMM).                                           *)
(* ------------------------------------------------------------------ *)

(* `reduce p q` — p reduces to q by a COMM contraction (first-match-wins). *)
Axiom reduce : Par -> Par -> Prop.

(* ------------------------------------------------------------------ *)
(* Law 5 — spatial matching / a free var is bound at most once.        *)
(* ------------------------------------------------------------------ *)

(* `spatial_matches target pattern` — the spatial matcher accepts `target` against `pattern`. *)
Axiom spatial_matches : Par -> Par -> Prop.

(* The linearity half of Law 5 (`addedVars.distinct`) is **not** stated here. Its old form,
   `binds_at_most_once`, was false as written — AUDIT C26 — and Lean replaced it with `linear`, a
   *predicate that is decidable by construction* and whose enforcement lives in the port's
   normalizer rather than in the matcher. A Coq copy of that predicate is owed with the rest of the
   matcher; stating it as a `Prop` axiom here would re-introduce exactly the defect Lean removed. *)

(* ------------------------------------------------------------------ *)
(* Law 6 — no globally free variables.                                 *)
(* ------------------------------------------------------------------ *)

(* `closed p` — p has no free (unbound) de Bruijn level. *)
Axiom closed : Par -> Prop.

(* Law 6: closedness is decidable (the interpreter's `Closed` refinement newtype). *)
Axiom closed_decidable : forall p : Par, {closed p} + {~ closed p}.
