(* Laws 2–6 statements over the flat `Par` — the Coq substitution/structural-congruence metatheory.

   Coq owns the *definitions* (capture-avoiding de Bruijn substitution, α-equivalence) per
   `AGENTS.md`; this file states each law's signature precisely over `Syntax.v` so the catalog is
   complete. What is a *definition* here is stated as one; what is only a signature stays an axiom,
   and the gate counts the `Axiom`/`Parameter`/`Conjecture`/`Admitted` declarations under
   `spec/coq/` against `COQ_AXIOM_CEILING` in `tools/check-lean-conformance.sh`, printing the count
   every run, so the number can only move down deliberately.

   **Definitions that are real as of 2026-09-23.** `alpha_equiv`, its three laws, `closed` and
   `closed_decidable` were axioms; they are definitions and theorems now, and `linear` — the honest
   replacement for the deleted `binds_at_most_once` (AUDIT C26) — is a definition with the witness
   that makes it non-vacuous. The mutual recursion over the flat family uses the higher-order idiom
   the guard checker accepts, with the recursive function *passed as a value* to `forallb`/`flat_map`:
   the naive nested `Fixpoint` is rejected ("recursive call to `closedSend` has principal argument
   equal to `a`"), and this spelling is the one Coq 8.18 takes — the idiom `Sort.v`'s header records.

   **What is deferred, named rather than implied.** `cmpPar`/`sortPar` and their two laws are the
   comparator spine the Lean track is still finishing (register row 1b). `substPar`, `reduce` and
   `spatial_matches` are the Coq analogue of laws 3/4/5, and mirroring them here would be a second
   formalization of units E8/E9 — outside an "honest and gated" tier.

   **The deep-α finding (2026-09-23).** With de Bruijn *levels* the representation is canonical, so
   deep α-equivalence *is* equality and no track owns it as a separate obligation; what remains, and
   what `alpha_equiv` is, is the structural congruence (`name-equivalence.k`'s `P | Q = Q | P`,
   `P = P | Nil`, associativity, congruence). The register's law-2 note said the deep-α half was
   Coq's obligation; it no longer says that. *)

From Rchain Require Import Syntax Sort.
Require Import Bool List Lia.
Import ListNotations.

(* ------------------------------------------------------------------ *)
(* Law 2 — structural congruence / name equivalence.                   *)
(* ------------------------------------------------------------------ *)

(* The mirror of Lean's `StrCong` (`Rchain/Rho.lean:24-31`). `refl`/`symm`/`trans` are constructors,
   so the three names the catalog used are the relation's laws rather than assumptions about it. *)
Inductive alpha_equiv : Par -> Par -> Prop :=
  | alpha_equiv_refl (p : Par) : alpha_equiv p p
  | alpha_equiv_symm (p q : Par) (H : alpha_equiv p q) : alpha_equiv q p
  | alpha_equiv_trans (p q r : Par) (H1 : alpha_equiv p q) (H2 : alpha_equiv q r) :
      alpha_equiv p r
  | alpha_equiv_comm (p q : Par) : alpha_equiv (parMerge p q) (parMerge q p)
  | alpha_equiv_assoc (p q r : Par) :
      alpha_equiv (parMerge (parMerge p q) r) (parMerge p (parMerge q r))
  | alpha_equiv_ident (p : Par) : alpha_equiv (parMerge p nilPar) p
  | alpha_equiv_par (p p' q q' : Par) (H1 : alpha_equiv p p') (H2 : alpha_equiv q q') :
      alpha_equiv (parMerge p q) (parMerge p' q').

(* `nilPar` is a *left* identity as well — Lean's `strCong_nil_left`, a theorem rather than a
   constructor. *)
Theorem alpha_equiv_nil_left : forall p : Par, alpha_equiv (parMerge nilPar p) p.
Proof.
  intro p. apply alpha_equiv_trans with (q := parMerge p nilPar).
  - apply alpha_equiv_comm.
  - apply alpha_equiv_ident.
Qed.

(* **Non-vacuity**: the relation does not relate every pair. The invariant is the number of elements
   in the eight fields; `parMerge` adds it and every constructor of the derivation preserves it, so
   `nilPar` (weight 0) is not congruent to a one-send process (weight 1). *)
Definition parWeight (p : Par) : nat :=
  match p with
  | ParMk ss rs ns es ms us bs cs =>
      length ss + length rs + length ns + length es + length ms + length us + length bs + length cs
  end.

Lemma parWeight_nilPar : parWeight nilPar = 0.
Proof. reflexivity. Qed.

Lemma parMerge_weight : forall p q : Par, parWeight (parMerge p q) = parWeight p + parWeight q.
Proof.
  intros [ss rs ns es ms us bs cs] [ss' rs' ns' es' ms' us' bs' cs'].
  unfold parWeight, parMerge. simpl. repeat rewrite app_length. lia.
Qed.

Theorem alpha_equiv_weight : forall p q : Par, alpha_equiv p q -> parWeight p = parWeight q.
Proof.
  intros p q H. induction H as [ p | p q H IH | p q r H1 IH1 H2 IH2 | p q | p q r | p
                              | p p' q q' H1 IH1 H2 IH2 ].
  - reflexivity.
  - symmetry. exact IH.
  - rewrite IH1, IH2. reflexivity.
  - rewrite !parMerge_weight. lia.
  - rewrite !parMerge_weight. lia.
  - rewrite parMerge_weight, parWeight_nilPar. lia.
  - rewrite !parMerge_weight. rewrite IH1, IH2. reflexivity.
Qed.

Definition a_one_send_par : Par :=
  ParMk (SendMk nilPar nil false :: nil) nil nil nil nil nil nil nil.

Theorem a_send_is_not_alpha_equiv_to_nil : ~ alpha_equiv nilPar a_one_send_par.
Proof.
  intro H. pose proof (alpha_equiv_weight _ _ H) as W.
  vm_compute in W. discriminate.
Qed.

(* ------------------------------------------------------------------ *)
(* Law 3 — capture-avoiding de Bruijn substitution.                    *)
(* ------------------------------------------------------------------ *)

(* A simultaneous substitution: one `Par` per de Bruijn level. *)
Definition Subst : Type := Var -> Par.

(* `substPar σ p` — substitute every free occurrence of a level by its image under σ, shifting bound
   levels through binders (capture-avoiding). **Stated, not defined**: mirroring it is the Coq
   analogue of Lean's law 3 (unit E8), which is itself still owed. *)
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

(* Law 5's linearity half as a **definition** — the predicate Lean uses (`Match.lean`'s `linear`,
   `freeLevelsOfPar`'s `Nodup`): no free de Bruijn level occurs twice, so a pattern's match binds
   each level at most once. It replaces the deleted `binds_at_most_once`, which stated as an axiom a
   proposition Lean found *false* as written (AUDIT C26) — an axiom for a false proposition makes
   the file unsound for the row it claims. *)

Definition var_free_level (v : Var) : list nat :=
  match v with Free n => n :: nil | _ => nil end.

(* Only the shapes the matcher's clauses can reach are traversed: a level inside a send or a `match`
   cannot be accepted as a pattern, so linearity never has to look inside one (Lean's
   `freeLevelsOfExpr` is the same shape). *)
Definition expr_free_levels (f : Par -> list nat) (e : Expr) : list nat :=
  match e with
  | EVar v => var_free_level v
  | EList ps | ETuple ps | ESet ps => flat_map f ps
  | EMap kvs => flat_map (fun kv => f (fst kv) ++ f (snd kv)) kvs
  | _ => nil
  end.

Fixpoint par_free_levels (p : Par) : list nat :=
  match p with ParMk _ _ _ es _ _ _ _ => flat_map (expr_free_levels par_free_levels) es end.

Fixpoint nodupb (l : list nat) : bool :=
  match l with
  | nil => true
  | x :: rest => negb (existsb (Nat.eqb x) rest) && nodupb rest
  end.

Definition linear (p : Par) : Prop := nodupb (par_free_levels p) = true.

Theorem linear_decidable : forall p : Par, {linear p} + {~ linear p}.
Proof.
  intro p. unfold linear. destruct (nodupb (par_free_levels p)) eqn:E.
  - left. reflexivity.
  - right. intros H. rewrite H in E. discriminate.
Qed.

Definition a_free_0 : Par := ParMk nil nil nil (EVar (Free 0) :: nil) nil nil nil nil.

(* Non-vacuity: a pattern that mentions the same free level twice is **not** linear. The two-element
   list is `[x, x]`, and `x` is a free level — the shape the normalizer's distinctness check rejects. *)
Definition a_double_binding : Par :=
  ParMk nil nil nil (EList (a_free_0 :: a_free_0 :: nil) :: nil) nil nil nil nil.

Theorem a_double_binding_is_not_linear : ~ linear a_double_binding.
Proof. intro H. vm_compute in H. discriminate. Qed.

(* ------------------------------------------------------------------ *)
(* Law 6 — no globally free variables.                                 *)
(* ------------------------------------------------------------------ *)

(* A closed variable occurrence is one that is not a free de Bruijn level (Lean's `closedVar`:
   `bound` and `wildcard` are closed in the model's judgement). *)
Definition var_closed (v : Var) : bool :=
  match v with Free _ => false | _ => true end.

(* The closure check, with the recursive function passed as a *value* — the idiom the guard checker
   accepts (see the header). *)
Definition closed_expr (f : Par -> bool) (e : Expr) : bool :=
  match e with
  | EGround _ => true
  | EVar v => var_closed v
  | ENeg p | ENot p => f p
  | EPlus p q | EMinus p q | EMult p q | EDiv p q | EMod p q
  | ELt p q | ELe p q | EGt p q | EGe p q | EEq p q | ENeq p q
  | EAnd p q | EOr p q => f p && f q
  | EList ps | ETuple ps | ESet ps => forallb f ps
  | EMap kvs => forallb (fun kv => f (fst kv) && f (snd kv)) kvs
  end.

Definition closed_send (f : Par -> bool) (s : Send) : bool :=
  f (send_chan s) && forallb f (send_data s).
Definition closed_rb (f : Par -> bool) (rb : ReceiveBind) : bool :=
  forallb f (rb_patterns rb) && f (rb_source rb).
Definition closed_receive (f : Par -> bool) (r : Receive) : bool :=
  forallb (closed_rb f) (receive_binds r) && f (receive_body r).
Definition closed_new (f : Par -> bool) (n : New) : bool := f (new_body n).
Definition closed_mc (f : Par -> bool) (m : MatchCase) : bool :=
  f (mc_pattern m) && f (mc_source m).
Definition closed_match (f : Par -> bool) (m : Match) : bool :=
  f (match_target m) && forallb (closed_mc f) (match_cases m).
Definition closed_bundle (f : Par -> bool) (b : Bundle) : bool := f (bundle_body b).
Definition closed_conn (f : Par -> bool) (c : Connective) : bool :=
  match c with
  | ConnAnd ps | ConnOr ps => forallb f ps
  | ConnNot p => f p
  | ConnVarRef _ _ => true
  end.

Fixpoint closedB (p : Par) : bool :=
  match p with
  | ParMk ss rs ns es ms us bs cs =>
      forallb (closed_send closedB) ss && forallb (closed_receive closedB) rs &&
      forallb (closed_new closedB) ns && forallb (closed_expr closedB) es &&
      forallb (closed_match closedB) ms && forallb (closed_bundle closedB) bs &&
      forallb (closed_conn closedB) cs
  end.

(* `closed p` — p has no free (unbound) de Bruijn level (Law 6). The *name* and the statement of
   `closed_decidable` are unchanged from the axiom they replace: a changed statement is a changed
   law. *)
Definition closed (p : Par) : Prop := closedB p = true.

(* Law 6: closedness is decidable (the interpreter's `Closed` refinement newtype). *)
Theorem closed_decidable : forall p : Par, {closed p} + {~ closed p}.
Proof.
  intro p. unfold closed. destruct (closedB p) eqn:E.
  - left. reflexivity.
  - right. intros H. rewrite H in E. discriminate.
Qed.

(* Non-vacuity: an expression that mentions a free level is not closed. *)
Definition a_free_level_is_not_closed : ~ closed (ParMk nil nil nil (EVar (Free 0) :: nil) nil nil nil nil).
Proof. intro H. vm_compute in H. discriminate. Qed.
