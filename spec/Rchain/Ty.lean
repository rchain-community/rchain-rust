import Rchain.Par
import Rchain.Cmp
import Rchain.Rho
import Rchain.Sort

set_option maxHeartbeats 1000000

/-!
# The type system over the ρ-calculus (the CoC layer)

Lean 4 *is* a Calculus of Constructions (CIC); this module uses it to type the node, taking the
ρ-calculus (`Rchain.Rho`, `Rchain.Par`) as its base sort. The goal is the **totality invariant**: the
node's own code admits no silent partiality (no `unwrap`/panic); every partial operation is either
proven total on a refinement, or returns `Option`/`Except` at a declared boundary.

Two layers, both here:

1. **The language sorts** — the one real rholang type distinction: a term is used in *process*
   position or *name* position (see `rholang/reference_doc/pattern_matching/rholangmatchingtut.md`).
   `PSort`/`Ctx` give the variable-level judgment; `isPureName`/`classify` give the structural
   name-vs-process classification of a term.
2. **Well-formedness refinements** — `Closed` (no free variables, Law 6), the refinement that makes
   the interpreter's partiality impossible, together with the proof that it is preserved by
   composition (`parMerge`), structural congruence (`≡`), and canonicalization (`sortList`).

The theorems at the bottom are the **fundamentals** the specification document cites.

## Canonical order (Law 1) — the one residual assumption

`Rchain.Sort` *defines* the canonical `sortPar` and *proves* Law 1 (`sortPar_idempotent`,
`sortPar_comm`). The single remaining assumption is the **lawfulness of the structural
comparators** — that `cmpPar`/`cmpSend`/…/`cmpListParPair` form a total order, stated as the 69
`cmpX_eq_iff` / `cmpX_swap` / `cmpX_lt_trans` axioms in `Rchain.Sort`. This is the standard "the
canonical order is total" postulate (every canonicalization needs a lawful order); discharging it is
a separate, later proof obligation (the 23-function mutual induction over the flat `Par` family).
-/

namespace Rchain

/-! ## Language sorts: process vs name -/

/-- The two syntactic sorts: a term in *process* position or *name* position. -/
inductive PSort where
  | proc
  | name
deriving DecidableEq, BEq, Repr

/-- A de Bruijn context: the sort of each level in scope. -/
def Ctx := List PSort

/-- The sort of a variable occurrence: a bound level is looked up in the context; a free variable or
    wildcard has no local sort (it is assigned by the top-level environment / pattern). -/
def varSort (Γ : Ctx) : Var → Option PSort
  | .bound l => Γ.get? l
  | .free _ => none
  | .wildcard => none

/-- `HasVarSort Γ v s` — the context classifies the variable occurrence `v` as sort `s`. -/
def HasVarSort (Γ : Ctx) (v : Var) (s : PSort) : Prop := varSort Γ v = some s

/-- The context judgment is decidable. -/
instance hasVarSort_decidable (Γ : Ctx) (v : Var) (s : PSort) : Decidable (HasVarSort Γ v s) := by
  unfold HasVarSort
  infer_instance

/-- The context judgment is functional: a variable has at most one sort. -/
theorem hasVarSort_functional {Γ : Ctx} {v : Var} {s s' : PSort}
    (h : HasVarSort Γ v s) (h' : HasVarSort Γ v s') : s = s' := by
  unfold HasVarSort at h h'
  rw [h] at h'
  exact Option.some.inj h'

/-! ## Structural name-vs-process classification -/

/-- A *pure name*: a `Par` with no process constructors at the top (empty sends/receives/news/
    matches). These are the terms that occur in name position: `Nil`, ground/expressions, bundles,
    unforgeables, connectives. Anything with a top-level send/receive/new/match is process-like. -/
def isPureName (p : Par) : Bool :=
  p.sends.isEmpty && p.receives.isEmpty && p.news.isEmpty && p.matches.isEmpty

/-- The `Prop` reading of `isPureName`: a pure name is a term the classifier accepts. -/
def IsPureName (p : Par) : Prop := isPureName p = true

/-- The sort classification: a pure name is a `name`, otherwise a `proc`. -/
def classify (p : Par) : PSort :=
  if isPureName p then PSort.name else PSort.proc

instance isPureName_decidable (p : Par) : Decidable (isPureName p = true) := inferInstance

instance IsPureName_decidable (p : Par) : Decidable (IsPureName p) := by
  unfold IsPureName
  infer_instance

theorem IsPureName_nil : IsPureName nilPar := by simp [IsPureName, isPureName, nilPar]

theorem classify_nil : classify nilPar = PSort.name := by simp [classify, isPureName, nilPar]

/-! ## Closedness (Law 6): no free variables -/

/-- A closed variable occurrence is one that is not a free de Bruijn level. -/
def closedVar (v : Var) : Bool :=
  match v with
  | .free _ => false
  | .bound _ => true
  | .wildcard => true

/-- A collection remainder is closed when its variable is: a pattern's `...rest` is bound by the
enclosing receive (a `bound` var after normalization) or discarded (`wildcard`). A *free* remainder
would escape the term, so it is not closed — the same judgment as any other pattern variable. -/
def closedRemainder : Option Var → Bool
  | none => true
  | some v => closedVar v

mutual
  def closed : Par → Bool
    | Par.mk s r n e m u b c =>
        closedListSend s && closedListReceive r && closedListNew n &&
        closedListExpr e && closedListMatch m && closedListGUnforgeable u &&
        closedListBundle b && closedListConnective c
  def closedSend : Send → Bool
    | Send.mk c d _ => closed c && closedListPar d
  def closedReceiveBind : ReceiveBind → Bool
    | ReceiveBind.mk ps s _ => closedListPar ps && closed s
  def closedReceive : Receive → Bool
    | Receive.mk bs b _ _ => closedListReceiveBind bs && closed b
  def closedNew : New → Bool
    | New.mk _ b => closed b
  def closedMatchCase : MatchCase → Bool
    | MatchCase.mk p s _ => closed p && closed s
  def closedMatch : Match → Bool
    | Match.mk t cs => closed t && closedListMatchCase cs
  def closedExpr : Expr → Bool
    | Expr.ground _ => true
    | Expr.evar v => closedVar v
    | Expr.eneg p => closed p
    | Expr.enot p => closed p
    | Expr.eplus p q => closed p && closed q
    | Expr.eminus p q => closed p && closed q
    | Expr.emult p q => closed p && closed q
    | Expr.ediv p q => closed p && closed q
    | Expr.emod p q => closed p && closed q
    | Expr.elt p q => closed p && closed q
    | Expr.ele p q => closed p && closed q
    | Expr.egt p q => closed p && closed q
    | Expr.ege p q => closed p && closed q
    | Expr.eeq p q => closed p && closed q
    | Expr.eneq p q => closed p && closed q
    | Expr.eand p q => closed p && closed q
    | Expr.eor p q => closed p && closed q
    | Expr.ematches p q => closed p && closed q
    | Expr.eshortand p q => closed p && closed q
    | Expr.eshortor p q => closed p && closed q
    | Expr.elist ps r => closedListPar ps && closedRemainder r
    | Expr.etuple ps => closedListPar ps
    | Expr.eset ps r => closedListPar ps && closedRemainder r
    | Expr.emap kvs r => closedListParPair kvs && closedRemainder r
  def closedBundle : Bundle → Bool
    | Bundle.mk b _ _ => closed b
  def closedGUnforgeable : GUnforgeable → Bool
    | _ => true
  def closedConnective : Connective → Bool
    | Connective.connAnd ps => closedListPar ps
    | Connective.connOr ps => closedListPar ps
    | Connective.connNot p => closed p
    | Connective.connVarRef _ _ => true
  def closedListSend : List Send → Bool
    | [] => true
    | a :: as => closedSend a && closedListSend as
  def closedListReceive : List Receive → Bool
    | [] => true
    | a :: as => closedReceive a && closedListReceive as
  def closedListNew : List New → Bool
    | [] => true
    | a :: as => closedNew a && closedListNew as
  def closedListExpr : List Expr → Bool
    | [] => true
    | a :: as => closedExpr a && closedListExpr as
  def closedListMatch : List Match → Bool
    | [] => true
    | a :: as => closedMatch a && closedListMatch as
  def closedListGUnforgeable : List GUnforgeable → Bool
    | [] => true
    | a :: as => closedGUnforgeable a && closedListGUnforgeable as
  def closedListBundle : List Bundle → Bool
    | [] => true
    | a :: as => closedBundle a && closedListBundle as
  def closedListConnective : List Connective → Bool
    | [] => true
    | a :: as => closedConnective a && closedListConnective as
  def closedListPar : List Par → Bool
    | [] => true
    | a :: as => closed a && closedListPar as
  def closedListReceiveBind : List ReceiveBind → Bool
    | [] => true
    | a :: as => closedReceiveBind a && closedListReceiveBind as
  def closedListMatchCase : List MatchCase → Bool
    | [] => true
    | a :: as => closedMatchCase a && closedListMatchCase as
  def closedListParPair : List (Par × Par) → Bool
    | [] => true
    | (a, b) :: as => closed a && closed b && closedListParPair as
end

/-- `Closed p` — the process has no free variables (Law 6). Decidable (via the `closed*` `Bool`
    functions) and preserved by composition, `≡`, and canonicalization (below). -/
def Closed (p : Par) : Prop :=
  closedListSend p.sends = true ∧ closedListReceive p.receives = true ∧
  closedListNew p.news = true ∧ closedListExpr p.exprs = true ∧
  closedListMatch p.matches = true ∧ closedListGUnforgeable p.unforgeables = true ∧
  closedListBundle p.bundles = true ∧ closedListConnective p.connectives = true

/-- Closedness is decidable (fundamental: the checker can actually run). -/
instance closed_decidable (p : Par) : Decidable (Closed p) := by
  unfold Closed
  infer_instance

/-! ## The fundamentals -/

-- Closedness distributes over list append, for each of the 8 top-level fields.
@[simp] theorem closedListSend_append (l l' : List Send) :
    closedListSend (l ++ l') = (closedListSend l && closedListSend l') := by
  induction l with
  | nil => simp [closedListSend]
  | cons a as ih => simp [closedListSend, ih, Bool.and_assoc]
@[simp] theorem closedListReceive_append (l l' : List Receive) :
    closedListReceive (l ++ l') = (closedListReceive l && closedListReceive l') := by
  induction l with
  | nil => simp [closedListReceive]
  | cons a as ih => simp [closedListReceive, ih, Bool.and_assoc]
@[simp] theorem closedListNew_append (l l' : List New) :
    closedListNew (l ++ l') = (closedListNew l && closedListNew l') := by
  induction l with
  | nil => simp [closedListNew]
  | cons a as ih => simp [closedListNew, ih, Bool.and_assoc]
@[simp] theorem closedListExpr_append (l l' : List Expr) :
    closedListExpr (l ++ l') = (closedListExpr l && closedListExpr l') := by
  induction l with
  | nil => simp [closedListExpr]
  | cons a as ih => simp [closedListExpr, ih, Bool.and_assoc]
@[simp] theorem closedListMatch_append (l l' : List Match) :
    closedListMatch (l ++ l') = (closedListMatch l && closedListMatch l') := by
  induction l with
  | nil => simp [closedListMatch]
  | cons a as ih => simp [closedListMatch, ih, Bool.and_assoc]
@[simp] theorem closedListGUnforgeable_append (l l' : List GUnforgeable) :
    closedListGUnforgeable (l ++ l') = (closedListGUnforgeable l && closedListGUnforgeable l') := by
  induction l with
  | nil => simp [closedListGUnforgeable]
  | cons a as ih => simp [closedListGUnforgeable, ih, Bool.and_assoc]
@[simp] theorem closedListBundle_append (l l' : List Bundle) :
    closedListBundle (l ++ l') = (closedListBundle l && closedListBundle l') := by
  induction l with
  | nil => simp [closedListBundle]
  | cons a as ih => simp [closedListBundle, ih, Bool.and_assoc]
@[simp] theorem closedListConnective_append (l l' : List Connective) :
    closedListConnective (l ++ l') = (closedListConnective l && closedListConnective l') := by
  induction l with
  | nil => simp [closedListConnective]
  | cons a as ih => simp [closedListConnective, ih, Bool.and_assoc]

-- Closedness of the empty field lists.
@[simp] theorem closedListSend_nil : closedListSend ([] : List Send) = true := by simp [closedListSend]
@[simp] theorem closedListReceive_nil : closedListReceive ([] : List Receive) = true := by simp [closedListReceive]
@[simp] theorem closedListNew_nil : closedListNew ([] : List New) = true := by simp [closedListNew]
@[simp] theorem closedListExpr_nil : closedListExpr ([] : List Expr) = true := by simp [closedListExpr]
@[simp] theorem closedListMatch_nil : closedListMatch ([] : List Match) = true := by simp [closedListMatch]
@[simp] theorem closedListGUnforgeable_nil : closedListGUnforgeable ([] : List GUnforgeable) = true := by simp [closedListGUnforgeable]
@[simp] theorem closedListBundle_nil : closedListBundle ([] : List Bundle) = true := by simp [closedListBundle]
@[simp] theorem closedListConnective_nil : closedListConnective ([] : List Connective) = true := by simp [closedListConnective]

/-- Fundamental: `nilPar` is closed. -/
theorem Closed_nil : Closed nilPar := by
  simp [Closed, nilPar]

/-- Fundamental: closedness is a **monoid invariant** — `|` (parMerge) of two closed processes is
    closed, and conversely. -/
theorem Closed_parMerge_iff (p q : Par) : Closed (parMerge p q) ↔ Closed p ∧ Closed q := by
  cases p <;> cases q
  simp [Closed, parMerge, Bool.and_eq_true]
  tauto

theorem Closed_parMerge {p q : Par} (hp : Closed p) (hq : Closed q) : Closed (parMerge p q) :=
  (Closed_parMerge_iff p q).mpr ⟨hp, hq⟩

/-- Fundamental: structural congruence preserves closedness (well-formedness is invariant under
    the `≡` fragment of reduction). -/
theorem strCong_closed_iff {p q : Par} : StrCong p q → (Closed p ↔ Closed q) := by
  intro h
  exact StrCong.rec (motive := fun {p q} _ => Closed p ↔ Closed q)
    (fun _p => Iff.rfl)
    (fun _h ih => ih.symm)
    (fun _h₁ _h₂ ih₁ ih₂ => ih₁.trans ih₂)
    (fun _p _q => by simp [Closed_parMerge_iff, and_comm])
    (fun _p _q _r => by simp [Closed_parMerge_iff, and_assoc])
    (fun _p => by simp [Closed_parMerge_iff, Closed_nil])
    (fun _h₁ _h₂ ih₁ ih₂ => by simpa [Closed_parMerge_iff] using and_congr ih₁ ih₂)
    h

theorem strCong_closed {p q : Par} (h : StrCong p q) (hp : Closed p) : Closed q :=
  (strCong_closed_iff h).mp hp

/-- Fundamental: canonicalization (`sortList`) is a permutation, so it preserves every
    element-wise predicate — in particular `Closed`. Instantiate with `P := Closed` (and, for `Par`,
    `C := parComparator` from Law 1) to get "canonicalization preserves well-formedness". -/
theorem sortList_mem_pred {α : Type} (C : Comparator α) (P : α → Prop) (l : List α) :
    (∀ x ∈ l, P x) → (∀ x ∈ Comparator.sortList C l, P x) := by
  intro h x hx
  have hp : List.Perm (Comparator.sortList C l) l := by
    unfold Comparator.sortList
    exact List.perm_insertionSort C.le l
  exact h x ((hp.mem_iff).mp hx)


/-! ## Canonicalization preserves closedness

`sortList_mem_pred` above is the *membership* form: sorting permutes, so every element-wise predicate
survives. What the closure laws need is the checker-level form — an **equation** `closed (sortPar p) =
closed p`, and the same for every member of the sort family, because `substExprToPar` sorts a set's and a
map's children *inside* its recursion and the proof of closure has to walk through the sorted form.

The two halves of the argument are the same as in `sortList_mem_pred` (a sort is a permutation; a closure
checker is a fold over the members), but the conclusion is an equation between Bool checkers rather than
a predicate, so the intermediate facts are `List.all`'s: `all_perm_eq` (a permutation is invisible to a
fold) and `all_congr` (pointwise-equal predicates fold the same). One such lemma per field type, each
taking the element fact as a hypothesis, and then one mutual block for the AST — the same `sizeOf`
template as `sortPar_idempotent` in `Rchain.Sort`.
-/

/-- `List.all` sees only the members, so a permutation is invisible to it. -/
theorem all_perm_eq {α : Type} (p : α → Bool) {l l' : List α} (h : l.Perm l') :
    l.all p = l'.all p := by
  induction h with
  | nil => rfl
  | cons x _ ih => simp only [List.all_cons, ih]
  | swap x y l => simp only [List.all_cons, Bool.and_left_comm]
  | trans _ _ ih1 ih2 => rw [ih1, ih2]

/-- Pointwise-equal predicates give the same `all`. -/
theorem all_congr {α : Type} (l : List α) {f g : α → Bool} (h : ∀ x ∈ l, f x = g x) :
    l.all f = l.all g := by
  induction l with
  | nil => rfl
  | cons a as ih =>
      rw [List.all_cons, List.all_cons, h a (List.mem_cons_self ..),
        ih (fun x hx => h x (List.mem_cons_of_mem _ hx))]

/-- `Comparator.sortList` is a sort, so it permutes its input. -/
theorem sortList_perm_self {α : Type} (C : Comparator α) (l : List α) : (C.sortList l).Perm l := by
  unfold Comparator.sortList
  exact List.perm_insertionSort C.le l

/-- `List.all` sees only the members, so a permutation is invisible to it. -/
theorem closedListSend_all (l : List Send) : closedListSend l = l.all closedSend := by
  induction l with
  | nil => rfl
  | cons a as ih => simp [closedListSend, ih]
theorem closedListReceive_all (l : List Receive) : closedListReceive l = l.all closedReceive := by
  induction l with
  | nil => rfl
  | cons a as ih => simp [closedListReceive, ih]
theorem closedListNew_all (l : List New) : closedListNew l = l.all closedNew := by
  induction l with
  | nil => rfl
  | cons a as ih => simp [closedListNew, ih]
theorem closedListExpr_all (l : List Expr) : closedListExpr l = l.all closedExpr := by
  induction l with
  | nil => rfl
  | cons a as ih => simp [closedListExpr, ih]
theorem closedListMatch_all (l : List Match) : closedListMatch l = l.all closedMatch := by
  induction l with
  | nil => rfl
  | cons a as ih => simp [closedListMatch, ih]
theorem closedListBundle_all (l : List Bundle) : closedListBundle l = l.all closedBundle := by
  induction l with
  | nil => rfl
  | cons a as ih => simp [closedListBundle, ih]
theorem closedListConnective_all (l : List Connective) :
    closedListConnective l = l.all closedConnective := by
  induction l with
  | nil => rfl
  | cons a as ih => simp [closedListConnective, ih]
theorem closedListPar_all (l : List Par) : closedListPar l = l.all closed := by
  induction l with
  | nil => rfl
  | cons a as ih => simp [closedListPar, ih]
theorem closedListParPair_all (l : List (Par × Par)) :
    closedListParPair l = l.all (fun x => closed x.1 && closed x.2) := by
  induction l with
  | nil => rfl
  | cons a as ih => obtain ⟨a1, a2⟩ := a; simp [closedListParPair, ih]
theorem closedListGUnforgeable_eq_true (l : List GUnforgeable) : closedListGUnforgeable l = true := by
  induction l with
  | nil => rfl
  | cons _ as ih => simp [closedListGUnforgeable, closedGUnforgeable, ih]

theorem closedListSend_sorted (l : List Send)
    (hf : ∀ x ∈ l, closedSend (sortSend x) = closedSend x) :
    closedListSend (Comparator.sortList sendComparator (l.map sortSend)) = closedListSend l := by
  rw [closedListSend_all, closedListSend_all]
  rw [all_perm_eq closedSend (sortList_perm_self sendComparator (l.map sortSend))]
  rw [List.all_map]
  exact all_congr l hf

theorem closedListReceive_sorted (l : List Receive)
    (hf : ∀ x ∈ l, closedReceive (sortReceive x) = closedReceive x) :
    closedListReceive (Comparator.sortList receiveComparator (l.map sortReceive)) =
      closedListReceive l := by
  rw [closedListReceive_all, closedListReceive_all]
  rw [all_perm_eq closedReceive (sortList_perm_self receiveComparator (l.map sortReceive))]
  rw [List.all_map]
  exact all_congr l hf

theorem closedListNew_sorted (l : List New)
    (hf : ∀ x ∈ l, closedNew (sortNew x) = closedNew x) :
    closedListNew (Comparator.sortList newComparator (l.map sortNew)) = closedListNew l := by
  rw [closedListNew_all, closedListNew_all]
  rw [all_perm_eq closedNew (sortList_perm_self newComparator (l.map sortNew))]
  rw [List.all_map]
  exact all_congr l hf

theorem closedListExpr_sorted (l : List Expr)
    (hf : ∀ x ∈ l, closedExpr (sortExpr x) = closedExpr x) :
    closedListExpr (Comparator.sortList exprComparator (l.map sortExpr)) = closedListExpr l := by
  rw [closedListExpr_all, closedListExpr_all]
  rw [all_perm_eq closedExpr (sortList_perm_self exprComparator (l.map sortExpr))]
  rw [List.all_map]
  exact all_congr l hf

theorem closedListMatch_sorted (l : List Match)
    (hf : ∀ x ∈ l, closedMatch (sortMatch x) = closedMatch x) :
    closedListMatch (Comparator.sortList matchComparator (l.map sortMatch)) = closedListMatch l := by
  rw [closedListMatch_all, closedListMatch_all]
  rw [all_perm_eq closedMatch (sortList_perm_self matchComparator (l.map sortMatch))]
  rw [List.all_map]
  exact all_congr l hf

theorem closedListBundle_sorted (l : List Bundle)
    (hf : ∀ x ∈ l, closedBundle (sortBundle x) = closedBundle x) :
    closedListBundle (Comparator.sortList bundleComparator (l.map sortBundle)) = closedListBundle l := by
  rw [closedListBundle_all, closedListBundle_all]
  rw [all_perm_eq closedBundle (sortList_perm_self bundleComparator (l.map sortBundle))]
  rw [List.all_map]
  exact all_congr l hf

theorem closedListConnective_sorted (l : List Connective)
    (hf : ∀ x ∈ l, closedConnective (sortConnective x) = closedConnective x) :
    closedListConnective (Comparator.sortList connectiveComparator (l.map sortConnective)) =
      closedListConnective l := by
  rw [closedListConnective_all, closedListConnective_all]
  rw [all_perm_eq closedConnective (sortList_perm_self connectiveComparator (l.map sortConnective))]
  rw [List.all_map]
  exact all_congr l hf

theorem closedListPar_sorted (l : List Par)
    (hf : ∀ x ∈ l, closed (sortPar x) = closed x) :
    closedListPar (Comparator.sortList parComparator (l.map sortPar)) = closedListPar l := by
  rw [closedListPar_all, closedListPar_all]
  rw [all_perm_eq closed (sortList_perm_self parComparator (l.map sortPar))]
  rw [List.all_map]
  exact all_congr l hf

theorem closedListParPair_sorted (l : List (Par × Par))
    (hf : ∀ x ∈ l, (closed (sortParPair x).1 && closed (sortParPair x).2) =
      (closed x.1 && closed x.2)) :
    closedListParPair
        (Comparator.sortList (Comparator.cmpPair parComparator parComparator) (l.map sortParPair)) =
      closedListParPair l := by
  rw [closedListParPair_all, closedListParPair_all]
  rw [all_perm_eq (fun x : Par × Par => closed x.1 && closed x.2)
        (sortList_perm_self (Comparator.cmpPair parComparator parComparator) (l.map sortParPair))]
  rw [List.all_map]
  exact all_congr l hf

theorem closedListReceiveBind_all (l : List ReceiveBind) :
    closedListReceiveBind l = l.all closedReceiveBind := by
  induction l with
  | nil => rfl
  | cons a as ih => simp [closedListReceiveBind, ih]
theorem closedListMatchCase_all (l : List MatchCase) :
    closedListMatchCase l = l.all closedMatchCase := by
  induction l with
  | nil => rfl
  | cons a as ih => simp [closedListMatchCase, ih]

theorem closedListReceiveBind_sorted (l : List ReceiveBind)
    (hf : ∀ x ∈ l, closedReceiveBind (sortReceiveBind x) = closedReceiveBind x) :
    closedListReceiveBind (Comparator.sortList receiveBindComparator (l.map sortReceiveBind)) =
      closedListReceiveBind l := by
  rw [closedListReceiveBind_all, closedListReceiveBind_all]
  rw [all_perm_eq closedReceiveBind (sortList_perm_self receiveBindComparator (l.map sortReceiveBind))]
  rw [List.all_map]
  exact all_congr l hf
theorem closedListMatchCase_sorted (l : List MatchCase)
    (hf : ∀ x ∈ l, closedMatchCase (sortMatchCase x) = closedMatchCase x) :
    closedListMatchCase (Comparator.sortList matchCaseComparator (l.map sortMatchCase)) =
      closedListMatchCase l := by
  rw [closedListMatchCase_all, closedListMatchCase_all]
  rw [all_perm_eq closedMatchCase (sortList_perm_self matchCaseComparator (l.map sortMatchCase))]
  rw [List.all_map]
  exact all_congr l hf

mutual
  theorem closed_sortPar : ∀ (p : Par), closed (sortPar p) = closed p
    | Par.mk s r n e m u b c => by
        have hs : ∀ x ∈ s, closedSend (sortSend x) = closedSend x := fun x _ => closed_sortSend x
        have hr : ∀ x ∈ r, closedReceive (sortReceive x) = closedReceive x :=
          fun x _ => closed_sortReceive x
        have hn : ∀ x ∈ n, closedNew (sortNew x) = closedNew x := fun x _ => closed_sortNew x
        have he : ∀ x ∈ e, closedExpr (sortExpr x) = closedExpr x := fun x _ => closed_sortExpr x
        have hm : ∀ x ∈ m, closedMatch (sortMatch x) = closedMatch x := fun x _ => closed_sortMatch x
        have hb : ∀ x ∈ b, closedBundle (sortBundle x) = closedBundle x :=
          fun x _ => closed_sortBundle x
        have hc : ∀ x ∈ c, closedConnective (sortConnective x) = closedConnective x :=
          fun x _ => closed_sortConnective x
        simp only [sortPar, sortListSend_eq_map, sortListReceive_eq_map, sortListNew_eq_map,
          sortListExpr_eq_map, sortListMatch_eq_map, sortListGUnforgeable_eq_map,
          sortListBundle_eq_map, sortListConnective_eq_map, closed]
        rw [closedListSend_sorted s hs, closedListReceive_sorted r hr, closedListNew_sorted n hn,
          closedListExpr_sorted e he, closedListMatch_sorted m hm,
          closedListGUnforgeable_eq_true
            (Comparator.sortList gUnforgeableComparator (List.map sortGUnforgeable u)),
          closedListGUnforgeable_eq_true u, closedListBundle_sorted b hb,
          closedListConnective_sorted c hc]
  termination_by p => sizeOf p

  theorem closed_sortSend : ∀ (x : Send), closedSend (sortSend x) = closedSend x
    | Send.mk ch d _ => by
        have hd : ∀ x ∈ d, closed (sortPar x) = closed x := fun x _ => closed_sortPar x
        simp only [sortSend, sortListPar_eq_map, closedSend, closed_sortPar ch]
        rw [closedListPar_sorted d hd]
  termination_by x => sizeOf x

  theorem closed_sortReceiveBind : ∀ (x : ReceiveBind),
      closedReceiveBind (sortReceiveBind x) = closedReceiveBind x
    | ReceiveBind.mk ps src _ => by
        have hps : ∀ x ∈ ps, closed (sortPar x) = closed x := fun x _ => closed_sortPar x
        simp only [sortReceiveBind, sortListPar_eq_map, closedReceiveBind, closed_sortPar src]
        rw [closedListPar_sorted ps hps]
  termination_by x => sizeOf x

  theorem closed_sortReceive : ∀ (x : Receive), closedReceive (sortReceive x) = closedReceive x
    | Receive.mk bs b _ _ => by
        have hbs : ∀ x ∈ bs, closedReceiveBind (sortReceiveBind x) = closedReceiveBind x :=
          fun x _ => closed_sortReceiveBind x
        simp only [sortReceive, sortListReceiveBind_eq_map, closedReceive, closed_sortPar b]
        rw [closedListReceiveBind_sorted bs hbs]
  termination_by x => sizeOf x

  theorem closed_sortNew : ∀ (x : New), closedNew (sortNew x) = closedNew x
    | New.mk _ b => by
        simp only [sortNew, closedNew]
        exact closed_sortPar b
  termination_by x => sizeOf x

  theorem closed_sortMatchCase : ∀ (x : MatchCase),
      closedMatchCase (sortMatchCase x) = closedMatchCase x
    | MatchCase.mk p s _ => by
        simp only [sortMatchCase, closedMatchCase, closed_sortPar p, closed_sortPar s]
  termination_by x => sizeOf x

  theorem closed_sortMatch : ∀ (x : Match), closedMatch (sortMatch x) = closedMatch x
    | Match.mk t cs => by
        have hcs : ∀ x ∈ cs, closedMatchCase (sortMatchCase x) = closedMatchCase x :=
          fun x _ => closed_sortMatchCase x
        simp only [sortMatch, sortListMatchCase_eq_map, closedMatch, closed_sortPar t]
        rw [closedListMatchCase_sorted cs hcs]
  termination_by x => sizeOf x

  theorem closed_sortBundle : ∀ (x : Bundle), closedBundle (sortBundle x) = closedBundle x
    | Bundle.mk b _ _ => by
        simp only [sortBundle, closedBundle]
        exact closed_sortPar b
  termination_by x => sizeOf x

  theorem closed_sortConnective : ∀ (x : Connective),
      closedConnective (sortConnective x) = closedConnective x
    | Connective.connAnd ps => by
        have hps : ∀ x ∈ ps, closed (sortPar x) = closed x := fun x _ => closed_sortPar x
        simp only [sortConnective, sortListPar_eq_map, closedConnective]
        rw [closedListPar_sorted ps hps]
    | Connective.connOr ps => by
        have hps : ∀ x ∈ ps, closed (sortPar x) = closed x := fun x _ => closed_sortPar x
        simp only [sortConnective, sortListPar_eq_map, closedConnective]
        rw [closedListPar_sorted ps hps]
    | Connective.connNot p => by
        simp only [sortConnective, closedConnective]
        exact closed_sortPar p
    | Connective.connVarRef _ _ => by
        simp [sortConnective, closedConnective]
  termination_by x => sizeOf x

  theorem closed_sortParPair : ∀ (x : Par × Par),
      (closed (sortParPair x).1 && closed (sortParPair x).2) = (closed x.1 && closed x.2)
    | (a, b) => by
        simp only [sortParPair, closed_sortPar a, closed_sortPar b]
  termination_by x => sizeOf x

  theorem closed_sortExpr : ∀ (x : Expr), closedExpr (sortExpr x) = closedExpr x
    | Expr.ground _ => by simp [sortExpr, closedExpr]
    | Expr.evar _ => by simp [sortExpr, closedExpr]
    | Expr.eneg p => by simp only [sortExpr, closedExpr]; exact closed_sortPar p
    | Expr.enot p => by simp only [sortExpr, closedExpr]; exact closed_sortPar p
    | Expr.eplus p q => by simp only [sortExpr, closedExpr, closed_sortPar p, closed_sortPar q]
    | Expr.eminus p q => by simp only [sortExpr, closedExpr, closed_sortPar p, closed_sortPar q]
    | Expr.emult p q => by simp only [sortExpr, closedExpr, closed_sortPar p, closed_sortPar q]
    | Expr.ediv p q => by simp only [sortExpr, closedExpr, closed_sortPar p, closed_sortPar q]
    | Expr.emod p q => by simp only [sortExpr, closedExpr, closed_sortPar p, closed_sortPar q]
    | Expr.elt p q => by simp only [sortExpr, closedExpr, closed_sortPar p, closed_sortPar q]
    | Expr.ele p q => by simp only [sortExpr, closedExpr, closed_sortPar p, closed_sortPar q]
    | Expr.egt p q => by simp only [sortExpr, closedExpr, closed_sortPar p, closed_sortPar q]
    | Expr.ege p q => by simp only [sortExpr, closedExpr, closed_sortPar p, closed_sortPar q]
    | Expr.eeq p q => by simp only [sortExpr, closedExpr, closed_sortPar p, closed_sortPar q]
    | Expr.eneq p q => by simp only [sortExpr, closedExpr, closed_sortPar p, closed_sortPar q]
    | Expr.eand p q => by simp only [sortExpr, closedExpr, closed_sortPar p, closed_sortPar q]
    | Expr.eor p q => by simp only [sortExpr, closedExpr, closed_sortPar p, closed_sortPar q]
    | Expr.ematches p q => by simp only [sortExpr, closedExpr, closed_sortPar p, closed_sortPar q]
    | Expr.eshortand p q => by simp only [sortExpr, closedExpr, closed_sortPar p, closed_sortPar q]
    | Expr.eshortor p q => by simp only [sortExpr, closedExpr, closed_sortPar p, closed_sortPar q]
    | Expr.elist ps _ => by
        have hps : ∀ x ∈ ps, closed (sortPar x) = closed x := fun x _ => closed_sortPar x
        simp only [sortExpr, sortListPar_eq_map, closedExpr]
        rw [closedListPar_sorted ps hps]
    | Expr.etuple ps => by
        have hps : ∀ x ∈ ps, closed (sortPar x) = closed x := fun x _ => closed_sortPar x
        simp only [sortExpr, sortListPar_eq_map, closedExpr]
        rw [closedListPar_sorted ps hps]
    | Expr.eset ps _ => by
        have hps : ∀ x ∈ ps, closed (sortPar x) = closed x := fun x _ => closed_sortPar x
        simp only [sortExpr, sortListPar_eq_map, closedExpr]
        rw [closedListPar_sorted ps hps]
    | Expr.emap kvs _ => by
        have hkv : ∀ x ∈ kvs, (closed (sortParPair x).1 && closed (sortParPair x).2) =
            (closed x.1 && closed x.2) := fun x _ => closed_sortParPair x
        simp only [sortExpr, sortListParPair_eq_map, closedExpr]
        rw [closedListParPair_sorted kvs hkv]
  termination_by x => sizeOf x
end

/-! ## Totality (the effect model bridging to Rust) -/

/-- `TotalOn f` — the operation `f` on processes is *total*: it maps closed processes to closed
    processes. This is the Lean spelling of "no `unwrap`/panic on the happy path": a Rust function
    `f : Par → Par` (not `→ Option`) whose spec is `TotalOn f` cannot panic on a free variable. -/
def TotalOn (f : Par → Par) : Prop := ∀ p, Closed p → Closed (f p)

/-- The identity is total. -/
theorem TotalOn_id : TotalOn (fun p => p) := by intro p hp; exact hp

/-- Totality is closed under composition. -/
theorem TotalOn_comp {f g : Par → Par} (hf : TotalOn f) (hg : TotalOn g) :
    TotalOn (fun p => g (f p)) := by
  intro p hp
  exact hg (f p) (hf p hp)

/-! ## Refinement types (the "no type escape" reading of totality) -/

/-- A refinement type: a value paired with a proof that it satisfies a predicate `P`. This is the
    sigma type that a Rust refinement newtype (`Port`, `Cost`, `BlockHeight`, …) corresponds to. The
    invariant `P` is *structural* — it cannot be projected away without the proof, which is the Lean
    reading of "no `Deref`/`.get()` type escape" in the Rust port. -/
def Refined (α : Type) (P : α → Prop) := { a : α // P a }

/-- `TotalOn f` is exactly "`f` lifts to a total map on the refinement `Refined Par Closed`": a
    total operation composes with the proof that its input is closed to produce the proof that its
    output is closed. Projecting the raw `Par` (dropping the proof) is the escape the Rust port
    forbids; keeping the refinement through `f` is the totality the port demands. -/
theorem totalOn_lifts_to_refined (f : Par → Par) :
    TotalOn f ↔ ∀ p : Refined Par Closed, Closed (f p.1) := by
  simp [TotalOn, Refined]

/-! ## Substitution (minimal) and the full sort judgment -/

/-- A simultaneous substitution: a term for each variable (de Bruijn level). -/
abbrev Subst := Var → Par

/-- Substitute a single expression occurrence: a free variable `evar (free _)` is rewritten to the
    substituted term's expressions; every other expression is left in place. This is the *minimal*
    substitution the type-system fundamentals need — it is visibly sort-preserving because it never
    touches `sends`/`receives`/`news`/`matches`. Deep capture-avoiding de Bruijn substitution is
    Coq's Autosubst obligation (`AGENTS.md`). -/
def substExpr (σ : Subst) : Expr → List Expr
  | Expr.evar v =>
      match v with
      | .free _ => (σ v).exprs
      | .bound _ => [Expr.evar v]
      | .wildcard => [Expr.evar v]
  | e => [e]

def substListExpr (σ : Subst) : List Expr → List Expr
  | [] => []
  | e :: es => substExpr σ e ++ substListExpr σ es

/-- Minimal substitution `subst σ p`: only the top-level `exprs` field is rewritten. -/
def subst (σ : Subst) (p : Par) : Par :=
  Par.mk p.sends p.receives p.news (substListExpr σ p.exprs) p.matches p.unforgeables p.bundles p.connectives

/-- `HasSort t s` — term `t` is classified as sort `s` (the unique structural process/name sort).
    The context `Γ` classifies variable occurrences via `HasVarSort`; `HasSort` is the structural
    half of the `Γ ⊢ t : s` judgment, and `Closed` is the well-scopedness refinement. -/
def HasSort (t : Par) (s : PSort) : Prop := classify t = s

instance HasSort_decidable (t : Par) (s : PSort) : Decidable (HasSort t s) := by
  unfold HasSort; infer_instance

/-- Fundamental 1: sort classification is functional (a term has at most one sort). -/
theorem HasSort_functional {t : Par} {s s' : PSort} (h : HasSort t s) (h' : HasSort t s') : s = s' := by
  unfold HasSort at h h'
  rw [h] at h'
  exact h'

/-- Fundamental 3 (structural half): substitution preserves the structural sort. -/
theorem subst_classify (σ : Subst) (p : Par) : classify (subst σ p) = classify p := by
  simp [classify, isPureName, subst]

/-- Fundamental 3: substitution preserves sort (`HasSort t s → HasSort (subst σ t) s`). -/
theorem subst_preserves_sort (σ : Subst) {t : Par} {s : PSort} (h : HasSort t s) : HasSort (subst σ t) s := by
  unfold HasSort at h ⊢
  rw [subst_classify σ t, h]

/-! ## Reduction preserves closedness (Fundamental 4) -/

/-- The boolean `closed` checker agrees with the `Closed` predicate. -/
@[simp] theorem closed_eq_Closed (p : Par) : closed p = true ↔ Closed p := by
  cases p
  simp [closed, Closed, Bool.and_eq_true]
  tauto

/-- The pattern a receive binds (`anyPat`) is closed: the model checks a bind's pattern with the same
judgement as its channel (`closedReceiveBind`), and the pattern is a *bound* variable, so it is closed
on its own. Stated here and marked `simp` so the `Closed` proofs below never unfold the pattern's
`Par` — without it `Closed_receivePar_iff` spends its whole heartbeat budget inside that one term. -/
@[simp] theorem closed_anyPat : closed anyPat = true := by
  simp [anyPat, closed, closedListExpr, closedExpr, closedVar]

/-- `receivePar chan body` is closed iff `chan` and `body` are closed. -/
theorem Closed_receivePar_iff (chan body : Par) :
    Closed (receivePar chan body) ↔ Closed chan ∧ Closed body := by
  cases chan <;> cases body
  simp [Closed, receivePar, closedListReceive, closedReceive, closedListReceiveBind,
    closedReceiveBind, closedListPar, Bool.and_eq_true, and_assoc, and_left_comm]

/-- Fundamental 4: reduction preserves closedness (COMM never introduces free variables). -/
theorem reduce_closed {p p' : Par} (h : Reduce p p') (hp : Closed p) : Closed p' := by
  exact Reduce.rec (motive := fun {p p'} _ => Closed p → Closed p')
    (fun chan data body hp =>
      have hboth := (Closed_parMerge_iff (sendPar chan [data]) (receivePar chan body)).mp hp
      (Closed_receivePar_iff chan body).mp hboth.2 |>.2)
    (fun {p p' q} _h ih hp =>
      have hboth := (Closed_parMerge_iff p q).mp hp
      (Closed_parMerge_iff p' q).mpr ⟨ih hboth.1, hboth.2⟩)
    (fun {p q q'} _h ih hp =>
      have hboth := (Closed_parMerge_iff p q).mp hp
      (Closed_parMerge_iff p q').mpr ⟨hboth.1, ih hboth.2⟩)
    h hp

end Rchain
