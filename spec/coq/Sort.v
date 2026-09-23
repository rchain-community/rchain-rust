(* Canonicalization (Law 1) over the flat `Par` — M2 gate, axiomatized.

   The Phase-0 *binary* `Proc` proof of Law 1 was committed (see git history) but the flat `Par`
   ADT (a record of 8 `list` fields, mutually recursive with `Send`/`Receive`/…/`Connective`) has no
   Coq definition of `sortPar` yet. What is missing is *both* halves, and they are not the same size:
   the mutual recursion over the family is expressible with the higher-order idiom the guard checker
   accepts (`map`/`forallb`, which is what makes an analogous `closed` a plain `Fixpoint` rather than
   a `Program Fixpoint` — verified on this Coq 8.18), while the *laws* need `cmpPar`'s total-order
   lemmas, which are the same comparator spine the Lean track is still finishing (register row 1b).

   The earlier version of this header said the axioms "mirror the Lean track's residual axioms".
   They no longer do: Lean proves both of these (`sortPar_idempotent`, `sortPar_comm`, law 1a
   `proved-model`). Coq keeps the signatures, and the gate counts them.

   Law 1 is `sortPar (sortPar p) = sortPar p` (idempotence) and
   `sortPar (parMerge p q) = sortPar (parMerge q p)` (commutativity). *)

From Rchain Require Import Syntax.

(* The structural total order (constructor declaration order, lexicographic via `lex`). *)
Axiom cmpPar : Par -> Par -> comparison.

(* The canonical sort: sort each of the 8 fields' elements by the corresponding comparator. *)
Axiom sortPar : Par -> Par.

(* Law 1 (admitted): sort is idempotent and `|` (parMerge) is commutative after sorting. *)
Axiom sortPar_idempotent : forall p : Par, sortPar (sortPar p) = sortPar p.
Axiom sortPar_comm : forall p q : Par, sortPar (parMerge p q) = sortPar (parMerge q p).
