import Rchain.Ty

set_option maxHeartbeats 1000000

/-!
# Law 6 — no globally free variables

A program is closed: no free (unbound) de Bruijn level occurs in it. The Scala oracle is
`free.k`/`program-restrictions.k` + `HasLocallyFree.scala`; the Rust realization is
`models::types::Closed` (a refinement newtype). `Closed` is already defined, made decidable, and
proven preserved by composition/`≡`/`⟶` in `Rchain.Ty`; this module adds the *semantic* free-variable
predicate and ties it to `Closed`.

**The predicate is a definition here, and it used to be an `axiom`.** The register's row said in as
many words that the law rested on "a defined-but-undefined-elsewhere predicate": `freeVarOf` was an
opaque `Par → Nat → Prop` with a second `axiom` (`closed_iff_no_freeVars`) tying it to the structural
checker. Two axioms, and nothing could look inside either — the same shape as law 5's three axioms
(AUDIT C26), where one of them was false. So `freeVarOf` is now a `mutual` block mirroring `Ty.lean`'s
`closed*` block type for type, and the tie is a proof.

**What makes that mirroring exact is the model's variable representation.** Variables are de Bruijn
*levels*: `Var.free k` is the free occurrence of level `k`, `Var.bound _` references an enclosing
binder, and `Var.wildcard` binds nothing. `Ty.lean`'s `closedVar` reads every `.bound` as closed and
every `.free` as open, so `closed` refuses **exactly** the `.free` occurrences `freeVarOf` accepts —
which is what makes the tie provable rather than assumed, and also what makes it non-trivial: the
agreement is a fact about the two definitions, not a restatement of one.

The block below is `closed*`'s shape with `∨` in place of `&&` (the conjunction of "every occurrence
is closed" is the *negation* of the disjunction "some occurrence is free"), and the per-remainder
predicate is `Option Var`, so a pattern's `...rest` is read exactly as `closedRemainder` reads it.
-/

namespace Rchain

/-- A variable occurrence that is free at level `k`: the model's `.free k`. A `.bound` occurrence
    references an enclosing binder and a `.wildcard` binds nothing — neither escapes — so neither is
    free at any level. This is the leaf the whole block below is built from, and it is the negation of
    `closedVar`'s leaf. -/
def freeVarAt (k : Nat) (v : Var) : Prop := v = .free k

/-- A collection remainder (`...rest`) is free at level `k` when its variable is: a receive-bound or
    discarded remainder is not, which is `closedRemainder`'s judgment on the same two cases. -/
def freeVarInRemainder (k : Nat) : Option Var → Prop
  | none => False
  | some v => v = .free k

mutual
  /-- `freeVarOf p k` — the de Bruijn level `k` occurs free in `p`. Field-wise on the flat `Par`,
  exactly as `closed` is, so the two disagree on a term precisely when the term has a free variable. -/
  def freeVarOf : Par → Nat → Prop
    | Par.mk s r nw e m u b c, k =>
        freeVarOfListSend s k ∨ freeVarOfListReceive r k ∨ freeVarOfListNew nw k ∨
        freeVarOfListExpr e k ∨ freeVarOfListMatch m k ∨ freeVarOfListGUnforgeable u k ∨
        freeVarOfListBundle b k ∨ freeVarOfListConnective c k
  termination_by p _ => sizeOf p

  def freeVarOfSend : Send → Nat → Prop
    | Send.mk c d _, k => freeVarOf c k ∨ freeVarOfListPar d k
  termination_by s _ => sizeOf s

  def freeVarOfReceiveBind : ReceiveBind → Nat → Prop
    | ReceiveBind.mk ps s _, k => freeVarOfListPar ps k ∨ freeVarOf s k
  termination_by b _ => sizeOf b

  def freeVarOfReceive : Receive → Nat → Prop
    | Receive.mk bs b _ _, k => freeVarOfListReceiveBind bs k ∨ freeVarOf b k
  termination_by r _ => sizeOf r

  def freeVarOfNew : New → Nat → Prop
    | New.mk _ b, k => freeVarOf b k
  termination_by n _ => sizeOf n

  def freeVarOfMatchCase : MatchCase → Nat → Prop
    | MatchCase.mk p s _, k => freeVarOf p k ∨ freeVarOf s k
  termination_by m _ => sizeOf m

  def freeVarOfMatch : Match → Nat → Prop
    | Match.mk t cs, k => freeVarOf t k ∨ freeVarOfListMatchCase cs k
  termination_by m _ => sizeOf m

  def freeVarOfExpr : Expr → Nat → Prop
    | Expr.ground _, _ => False
    | Expr.evar v, k => freeVarAt k v
    | Expr.eneg p, k => freeVarOf p k
    | Expr.enot p, k => freeVarOf p k
    | Expr.eplus p q, k => freeVarOf p k ∨ freeVarOf q k
    | Expr.eminus p q, k => freeVarOf p k ∨ freeVarOf q k
    | Expr.emult p q, k => freeVarOf p k ∨ freeVarOf q k
    | Expr.ediv p q, k => freeVarOf p k ∨ freeVarOf q k
    | Expr.emod p q, k => freeVarOf p k ∨ freeVarOf q k
    | Expr.elt p q, k => freeVarOf p k ∨ freeVarOf q k
    | Expr.ele p q, k => freeVarOf p k ∨ freeVarOf q k
    | Expr.egt p q, k => freeVarOf p k ∨ freeVarOf q k
    | Expr.ege p q, k => freeVarOf p k ∨ freeVarOf q k
    | Expr.eeq p q, k => freeVarOf p k ∨ freeVarOf q k
    | Expr.eneq p q, k => freeVarOf p k ∨ freeVarOf q k
    | Expr.eand p q, k => freeVarOf p k ∨ freeVarOf q k
    | Expr.eor p q, k => freeVarOf p k ∨ freeVarOf q k
    | Expr.ematches p q, k => freeVarOf p k ∨ freeVarOf q k
    | Expr.eshortand p q, k => freeVarOf p k ∨ freeVarOf q k
    | Expr.eshortor p q, k => freeVarOf p k ∨ freeVarOf q k
    | Expr.elist ps r, k => freeVarOfListPar ps k ∨ freeVarInRemainder k r
    | Expr.etuple ps, k => freeVarOfListPar ps k
    | Expr.eset ps r, k => freeVarOfListPar ps k ∨ freeVarInRemainder k r
    | Expr.emap kvs r, k => freeVarOfListParPair kvs k ∨ freeVarInRemainder k r
    -- The five added 2026-09-25. `emethod`'s name is a code-point list and holds no variable.
    | Expr.ebigint _, _ => False
    | Expr.emethod _ t args, k => freeVarOf t k ∨ freeVarOfListPar args k
    | Expr.epercentPercent p q, k => freeVarOf p k ∨ freeVarOf q k
    | Expr.eplusPlus p q, k => freeVarOf p k ∨ freeVarOf q k
    | Expr.eminusMinus p q, k => freeVarOf p k ∨ freeVarOf q k
  termination_by e _ => sizeOf e

  def freeVarOfBundle : Bundle → Nat → Prop
    | Bundle.mk b _ _, k => freeVarOf b k
  termination_by b _ => sizeOf b

  /-- A `GUnforgeable` carries no variable — the same reading as `closedGUnforgeable`'s `true`. -/
  def freeVarOfGUnforgeable : GUnforgeable → Nat → Prop
    | _, _ => False

  def freeVarOfConnective : Connective → Nat → Prop
    | Connective.connAnd ps, k => freeVarOfListPar ps k
    | Connective.connOr ps, k => freeVarOfListPar ps k
    | Connective.connNot p, k => freeVarOf p k
    | Connective.connVarRef _ _, _ => False
  termination_by c _ => sizeOf c

  def freeVarOfListSend : List Send → Nat → Prop
    | [], _ => False
    | a :: as, k => freeVarOfSend a k ∨ freeVarOfListSend as k
  termination_by l _ => sizeOf l

  def freeVarOfListReceive : List Receive → Nat → Prop
    | [], _ => False
    | a :: as, k => freeVarOfReceive a k ∨ freeVarOfListReceive as k
  termination_by l _ => sizeOf l

  def freeVarOfListNew : List New → Nat → Prop
    | [], _ => False
    | a :: as, k => freeVarOfNew a k ∨ freeVarOfListNew as k
  termination_by l _ => sizeOf l

  def freeVarOfListExpr : List Expr → Nat → Prop
    | [], _ => False
    | a :: as, k => freeVarOfExpr a k ∨ freeVarOfListExpr as k
  termination_by l _ => sizeOf l

  def freeVarOfListMatch : List Match → Nat → Prop
    | [], _ => False
    | a :: as, k => freeVarOfMatch a k ∨ freeVarOfListMatch as k
  termination_by l _ => sizeOf l

  def freeVarOfListGUnforgeable : List GUnforgeable → Nat → Prop
    | [], _ => False
    | a :: as, k => freeVarOfGUnforgeable a k ∨ freeVarOfListGUnforgeable as k
  termination_by l _ => sizeOf l

  def freeVarOfListBundle : List Bundle → Nat → Prop
    | [], _ => False
    | a :: as, k => freeVarOfBundle a k ∨ freeVarOfListBundle as k
  termination_by l _ => sizeOf l

  def freeVarOfListConnective : List Connective → Nat → Prop
    | [], _ => False
    | a :: as, k => freeVarOfConnective a k ∨ freeVarOfListConnective as k
  termination_by l _ => sizeOf l

  def freeVarOfListPar : List Par → Nat → Prop
    | [], _ => False
    | a :: as, k => freeVarOf a k ∨ freeVarOfListPar as k
  termination_by l _ => sizeOf l

  def freeVarOfListReceiveBind : List ReceiveBind → Nat → Prop
    | [], _ => False
    | a :: as, k => freeVarOfReceiveBind a k ∨ freeVarOfListReceiveBind as k
  termination_by l _ => sizeOf l

  def freeVarOfListMatchCase : List MatchCase → Nat → Prop
    | [], _ => False
    | a :: as, k => freeVarOfMatchCase a k ∨ freeVarOfListMatchCase as k
  termination_by l _ => sizeOf l

  def freeVarOfListParPair : List (Par × Par) → Nat → Prop
    | [], _ => False
    | (a, b) :: as, k => freeVarOf a k ∨ freeVarOf b k ∨ freeVarOfListParPair as k
  termination_by l _ => sizeOf l
end

/-! ## `freeVarOf` distributes over `|` and over list append

Both are needed by law 4b (`spec/Rchain/Reduce.lean`), whose step is a congruence under `|`: the
free-variable reading of `parMerge p q` is the disjunction of the two readings, because `parMerge`
appends each of the eight fields and the block above is field-wise. -/

@[simp] theorem freeVarOfListSend_append (l l' : List Send) (k : Nat) :
    freeVarOfListSend (l ++ l') k ↔ freeVarOfListSend l k ∨ freeVarOfListSend l' k := by
  induction l with
  | nil => simp [freeVarOfListSend]
  | cons a as ih => simp only [List.cons_append, freeVarOfListSend, ih]; tauto

@[simp] theorem freeVarOfListReceive_append (l l' : List Receive) (k : Nat) :
    freeVarOfListReceive (l ++ l') k ↔ freeVarOfListReceive l k ∨ freeVarOfListReceive l' k := by
  induction l with
  | nil => simp [freeVarOfListReceive]
  | cons a as ih => simp only [List.cons_append, freeVarOfListReceive, ih]; tauto

@[simp] theorem freeVarOfListNew_append (l l' : List New) (k : Nat) :
    freeVarOfListNew (l ++ l') k ↔ freeVarOfListNew l k ∨ freeVarOfListNew l' k := by
  induction l with
  | nil => simp [freeVarOfListNew]
  | cons a as ih => simp only [List.cons_append, freeVarOfListNew, ih]; tauto

@[simp] theorem freeVarOfListExpr_append (l l' : List Expr) (k : Nat) :
    freeVarOfListExpr (l ++ l') k ↔ freeVarOfListExpr l k ∨ freeVarOfListExpr l' k := by
  induction l with
  | nil => simp [freeVarOfListExpr]
  | cons a as ih => simp only [List.cons_append, freeVarOfListExpr, ih]; tauto

@[simp] theorem freeVarOfListMatch_append (l l' : List Match) (k : Nat) :
    freeVarOfListMatch (l ++ l') k ↔ freeVarOfListMatch l k ∨ freeVarOfListMatch l' k := by
  induction l with
  | nil => simp [freeVarOfListMatch]
  | cons a as ih => simp only [List.cons_append, freeVarOfListMatch, ih]; tauto

@[simp] theorem freeVarOfListGUnforgeable_append (l l' : List GUnforgeable) (k : Nat) :
    freeVarOfListGUnforgeable (l ++ l') k ↔
      freeVarOfListGUnforgeable l k ∨ freeVarOfListGUnforgeable l' k := by
  induction l with
  | nil => simp [freeVarOfListGUnforgeable]
  | cons a as ih => simp only [List.cons_append, freeVarOfListGUnforgeable, ih]; tauto

@[simp] theorem freeVarOfListBundle_append (l l' : List Bundle) (k : Nat) :
    freeVarOfListBundle (l ++ l') k ↔ freeVarOfListBundle l k ∨ freeVarOfListBundle l' k := by
  induction l with
  | nil => simp [freeVarOfListBundle]
  | cons a as ih => simp only [List.cons_append, freeVarOfListBundle, ih]; tauto

@[simp] theorem freeVarOfListConnective_append (l l' : List Connective) (k : Nat) :
    freeVarOfListConnective (l ++ l') k ↔
      freeVarOfListConnective l k ∨ freeVarOfListConnective l' k := by
  induction l with
  | nil => simp [freeVarOfListConnective]
  | cons a as ih => simp only [List.cons_append, freeVarOfListConnective, ih]; tauto

@[simp] theorem freeVarOfListPar_append (l l' : List Par) (k : Nat) :
    freeVarOfListPar (l ++ l') k ↔ freeVarOfListPar l k ∨ freeVarOfListPar l' k := by
  induction l with
  | nil => simp [freeVarOfListPar]
  | cons a as ih => simp only [List.cons_append, freeVarOfListPar, ih]; tauto

@[simp] theorem freeVarOfListReceiveBind_append (l l' : List ReceiveBind) (k : Nat) :
    freeVarOfListReceiveBind (l ++ l') k ↔
      freeVarOfListReceiveBind l k ∨ freeVarOfListReceiveBind l' k := by
  induction l with
  | nil => simp [freeVarOfListReceiveBind]
  | cons a as ih => simp only [List.cons_append, freeVarOfListReceiveBind, ih]; tauto

@[simp] theorem freeVarOfListMatchCase_append (l l' : List MatchCase) (k : Nat) :
    freeVarOfListMatchCase (l ++ l') k ↔
      freeVarOfListMatchCase l k ∨ freeVarOfListMatchCase l' k := by
  induction l with
  | nil => simp [freeVarOfListMatchCase]
  | cons a as ih => simp only [List.cons_append, freeVarOfListMatchCase, ih]; tauto

@[simp] theorem freeVarOfListParPair_append (l l' : List (Par × Par)) (k : Nat) :
    freeVarOfListParPair (l ++ l') k ↔
      freeVarOfListParPair l k ∨ freeVarOfListParPair l' k := by
  induction l with
  | nil => simp [freeVarOfListParPair]
  | cons a as ih =>
    obtain ⟨a₁, a₂⟩ := a
    simp only [List.cons_append, freeVarOfListParPair, ih]; tauto

/-- `freeVarOf` over `|`: `parMerge` appends all eight fields, so a free level is free in the merge
    exactly when it is free in one of the two sides. The shape law 4b's two congruence arms need. -/
@[simp] theorem freeVarOf_parMerge (p q : Par) (k : Nat) :
    freeVarOf (parMerge p q) k ↔ freeVarOf p k ∨ freeVarOf q k := by
  cases p with | mk s₁ r₁ n₁ e₁ m₁ u₁ b₁ c₁ =>
  cases q with | mk s₂ r₂ n₂ e₂ m₂ u₂ b₂ c₂ =>
  simp [parMerge, freeVarOf]
  tauto

/-- The remainder case of the block below: an `Option Var` carries no `Par`, so it is a leaf — but it
    is the same judgment as `closedRemainder`'s, which is why the collection forms' correspondence
    goes through it. -/
theorem freeVarInRemainder_iff_closed (r : Option Var) :
    (∀ k, ¬ freeVarInRemainder k r) ↔ closedRemainder r = true := by
  cases r with
  | none => simp [freeVarInRemainder, closedRemainder]
  | some v =>
    cases v with
    | bound l => simp [freeVarInRemainder, closedRemainder, closedVar]
    | free m =>
        simp only [freeVarInRemainder, closedRemainder, closedVar, Bool.false_eq_true]
        exact ⟨fun h => absurd rfl (h m), fun h => h.elim⟩
    | wildcard => simp [freeVarInRemainder, closedRemainder, closedVar]

/-! ## The tie to the `closed` checker (Law 6, and law 4b's premise)

One `mutual` block, mirroring the `closed*` block in `Rchain.Ty` member for member: for every one of
the 23 types, "no level is free here" is "the checker accepts". Each member is proved by the same
structural recursion the definition runs, and the composition members are pure propositional
arithmetic (`tauto`) because both sides are the same field-wise shape up to `∧`/`∨` associativity. -/

mutual
  theorem freeVarOf_iff_closed : (p : Par) → ((∀ k, ¬ freeVarOf p k) ↔ closed p = true)
    | Par.mk s r nw e m u b c => by
      simp only [freeVarOf, closed, not_or, forall_and, Bool.and_eq_true, and_assoc,
        freeVarOfListSend_iff_closed, freeVarOfListReceive_iff_closed, freeVarOfListNew_iff_closed,
        freeVarOfListExpr_iff_closed, freeVarOfListMatch_iff_closed,
        freeVarOfListGUnforgeable_iff_closed, freeVarOfListBundle_iff_closed,
        freeVarOfListConnective_iff_closed]

  theorem freeVarOfSend_iff_closed : (s : Send) → ((∀ k, ¬ freeVarOfSend s k) ↔ closedSend s = true)
    | Send.mk c d _ => by
      simp only [freeVarOfSend, closedSend, not_or, forall_and, Bool.and_eq_true,
        freeVarOf_iff_closed, freeVarOfListPar_iff_closed]

  theorem freeVarOfReceiveBind_iff_closed :
      (b : ReceiveBind) → ((∀ k, ¬ freeVarOfReceiveBind b k) ↔ closedReceiveBind b = true)
    | ReceiveBind.mk ps s _ => by
      simp only [freeVarOfReceiveBind, closedReceiveBind, not_or, forall_and, Bool.and_eq_true,
        freeVarOfListPar_iff_closed, freeVarOf_iff_closed]

  theorem freeVarOfReceive_iff_closed :
      (r : Receive) → ((∀ k, ¬ freeVarOfReceive r k) ↔ closedReceive r = true)
    | Receive.mk bs b _ _ => by
      simp only [freeVarOfReceive, closedReceive, not_or, forall_and, Bool.and_eq_true,
        freeVarOfListReceiveBind_iff_closed, freeVarOf_iff_closed]

  theorem freeVarOfNew_iff_closed : (nw : New) → ((∀ k, ¬ freeVarOfNew nw k) ↔ closedNew nw = true)
    | New.mk _ b => by
      simp only [freeVarOfNew, closedNew, freeVarOf_iff_closed]

  theorem freeVarOfMatchCase_iff_closed :
      (m : MatchCase) → ((∀ k, ¬ freeVarOfMatchCase m k) ↔ closedMatchCase m = true)
    | MatchCase.mk p s _ => by
      simp only [freeVarOfMatchCase, closedMatchCase, not_or, forall_and, Bool.and_eq_true,
        freeVarOf_iff_closed]

  theorem freeVarOfMatch_iff_closed :
      (m : Match) → ((∀ k, ¬ freeVarOfMatch m k) ↔ closedMatch m = true)
    | Match.mk t cs => by
      simp only [freeVarOfMatch, closedMatch, not_or, forall_and, Bool.and_eq_true,
        freeVarOf_iff_closed, freeVarOfListMatchCase_iff_closed]

  theorem freeVarOfExpr_iff_closed :
      (e : Expr) → ((∀ k, ¬ freeVarOfExpr e k) ↔ closedExpr e = true)
    | Expr.ground _ => by simp [freeVarOfExpr, closedExpr]
    | Expr.evar v => by
      cases v with
      | bound l => simp [freeVarOfExpr, freeVarAt, closedExpr, closedVar]
      | free m =>
          simp only [freeVarOfExpr, freeVarAt, closedExpr, closedVar, Bool.false_eq_true]
          exact ⟨fun h => absurd rfl (h m), fun h => h.elim⟩
      | wildcard => simp [freeVarOfExpr, freeVarAt, closedExpr, closedVar]
    | Expr.eneg p | Expr.enot p => by
      simp only [freeVarOfExpr, closedExpr, freeVarOf_iff_closed]
    | Expr.eplus p q | Expr.eminus p q | Expr.emult p q | Expr.ediv p q | Expr.emod p q
    | Expr.elt p q | Expr.ele p q | Expr.egt p q | Expr.ege p q | Expr.eeq p q
    | Expr.eneq p q | Expr.eand p q | Expr.eor p q
    | Expr.ematches p q | Expr.eshortand p q | Expr.eshortor p q => by
      simp only [freeVarOfExpr, closedExpr, not_or, forall_and, Bool.and_eq_true,
        freeVarOf_iff_closed]
    | Expr.elist ps r | Expr.eset ps r => by
      simp only [freeVarOfExpr, closedExpr, not_or, forall_and, Bool.and_eq_true,
        freeVarOfListPar_iff_closed, freeVarInRemainder_iff_closed]
    | Expr.etuple ps => by
      simp only [freeVarOfExpr, closedExpr, freeVarOfListPar_iff_closed]
    | Expr.emap kvs r => by
      simp only [freeVarOfExpr, closedExpr, not_or, forall_and, Bool.and_eq_true,
        freeVarOfListParPair_iff_closed, freeVarInRemainder_iff_closed]
    | Expr.ebigint _ => by simp [freeVarOfExpr, closedExpr]
    | Expr.emethod _ t args => by
      simp only [freeVarOfExpr, closedExpr, not_or, forall_and, Bool.and_eq_true,
        freeVarOf_iff_closed, freeVarOfListPar_iff_closed]
    | Expr.epercentPercent p q | Expr.eplusPlus p q | Expr.eminusMinus p q => by
      simp only [freeVarOfExpr, closedExpr, not_or, forall_and, Bool.and_eq_true,
        freeVarOf_iff_closed]

  theorem freeVarOfBundle_iff_closed :
      (b : Bundle) → ((∀ k, ¬ freeVarOfBundle b k) ↔ closedBundle b = true)
    | Bundle.mk b _ _ => by
      simp only [freeVarOfBundle, closedBundle, freeVarOf_iff_closed]

  theorem freeVarOfGUnforgeable_iff_closed :
      (g : GUnforgeable) → ((∀ k, ¬ freeVarOfGUnforgeable g k) ↔ closedGUnforgeable g = true)
    | _ => by simp [freeVarOfGUnforgeable, closedGUnforgeable]

  theorem freeVarOfConnective_iff_closed :
      (c : Connective) → ((∀ k, ¬ freeVarOfConnective c k) ↔ closedConnective c = true)
    | Connective.connAnd ps | Connective.connOr ps => by
      simp only [freeVarOfConnective, closedConnective, freeVarOfListPar_iff_closed]
    | Connective.connNot p => by
      simp only [freeVarOfConnective, closedConnective, freeVarOf_iff_closed]
    | Connective.connVarRef _ _ => by
      simp [freeVarOfConnective, closedConnective]

  theorem freeVarOfListSend_iff_closed :
      (l : List Send) → ((∀ k, ¬ freeVarOfListSend l k) ↔ closedListSend l = true)
    | [] => by simp [freeVarOfListSend, closedListSend]
    | a :: as => by
      simp only [freeVarOfListSend, closedListSend, not_or, forall_and, Bool.and_eq_true,
        freeVarOfSend_iff_closed, freeVarOfListSend_iff_closed]

  theorem freeVarOfListReceive_iff_closed :
      (l : List Receive) → ((∀ k, ¬ freeVarOfListReceive l k) ↔ closedListReceive l = true)
    | [] => by simp [freeVarOfListReceive, closedListReceive]
    | a :: as => by
      simp only [freeVarOfListReceive, closedListReceive, not_or, forall_and, Bool.and_eq_true,
        freeVarOfReceive_iff_closed, freeVarOfListReceive_iff_closed]

  theorem freeVarOfListNew_iff_closed :
      (l : List New) → ((∀ k, ¬ freeVarOfListNew l k) ↔ closedListNew l = true)
    | [] => by simp [freeVarOfListNew, closedListNew]
    | a :: as => by
      simp only [freeVarOfListNew, closedListNew, not_or, forall_and, Bool.and_eq_true,
        freeVarOfNew_iff_closed, freeVarOfListNew_iff_closed]

  theorem freeVarOfListExpr_iff_closed :
      (l : List Expr) → ((∀ k, ¬ freeVarOfListExpr l k) ↔ closedListExpr l = true)
    | [] => by simp [freeVarOfListExpr, closedListExpr]
    | a :: as => by
      simp only [freeVarOfListExpr, closedListExpr, not_or, forall_and, Bool.and_eq_true,
        freeVarOfExpr_iff_closed, freeVarOfListExpr_iff_closed]

  theorem freeVarOfListMatch_iff_closed :
      (l : List Match) → ((∀ k, ¬ freeVarOfListMatch l k) ↔ closedListMatch l = true)
    | [] => by simp [freeVarOfListMatch, closedListMatch]
    | a :: as => by
      simp only [freeVarOfListMatch, closedListMatch, not_or, forall_and, Bool.and_eq_true,
        freeVarOfMatch_iff_closed, freeVarOfListMatch_iff_closed]

  theorem freeVarOfListGUnforgeable_iff_closed :
      (l : List GUnforgeable) →
        ((∀ k, ¬ freeVarOfListGUnforgeable l k) ↔ closedListGUnforgeable l = true)
    | [] => by simp [freeVarOfListGUnforgeable, closedListGUnforgeable]
    | a :: as => by
      simp only [freeVarOfListGUnforgeable, closedListGUnforgeable, not_or, forall_and,
        Bool.and_eq_true, freeVarOfGUnforgeable_iff_closed, freeVarOfListGUnforgeable_iff_closed]

  theorem freeVarOfListBundle_iff_closed :
      (l : List Bundle) → ((∀ k, ¬ freeVarOfListBundle l k) ↔ closedListBundle l = true)
    | [] => by simp [freeVarOfListBundle, closedListBundle]
    | a :: as => by
      simp only [freeVarOfListBundle, closedListBundle, not_or, forall_and, Bool.and_eq_true,
        freeVarOfBundle_iff_closed, freeVarOfListBundle_iff_closed]

  theorem freeVarOfListConnective_iff_closed :
      (l : List Connective) → ((∀ k, ¬ freeVarOfListConnective l k) ↔ closedListConnective l = true)
    | [] => by simp [freeVarOfListConnective, closedListConnective]
    | a :: as => by
      simp only [freeVarOfListConnective, closedListConnective, not_or, forall_and, Bool.and_eq_true,
        freeVarOfConnective_iff_closed, freeVarOfListConnective_iff_closed]

  theorem freeVarOfListPar_iff_closed :
      (l : List Par) → ((∀ k, ¬ freeVarOfListPar l k) ↔ closedListPar l = true)
    | [] => by simp [freeVarOfListPar, closedListPar]
    | a :: as => by
      simp only [freeVarOfListPar, closedListPar, not_or, forall_and, Bool.and_eq_true,
        freeVarOf_iff_closed, freeVarOfListPar_iff_closed]

  theorem freeVarOfListReceiveBind_iff_closed :
      (l : List ReceiveBind) →
        ((∀ k, ¬ freeVarOfListReceiveBind l k) ↔ closedListReceiveBind l = true)
    | [] => by simp [freeVarOfListReceiveBind, closedListReceiveBind]
    | a :: as => by
      simp only [freeVarOfListReceiveBind, closedListReceiveBind, not_or, forall_and,
        Bool.and_eq_true, freeVarOfReceiveBind_iff_closed, freeVarOfListReceiveBind_iff_closed]

  theorem freeVarOfListMatchCase_iff_closed :
      (l : List MatchCase) →
        ((∀ k, ¬ freeVarOfListMatchCase l k) ↔ closedListMatchCase l = true)
    | [] => by simp [freeVarOfListMatchCase, closedListMatchCase]
    | a :: as => by
      simp only [freeVarOfListMatchCase, closedListMatchCase, not_or, forall_and, Bool.and_eq_true,
        freeVarOfMatchCase_iff_closed, freeVarOfListMatchCase_iff_closed]

  theorem freeVarOfListParPair_iff_closed :
      (l : List (Par × Par)) → ((∀ k, ¬ freeVarOfListParPair l k) ↔ closedListParPair l = true)
    | [] => by simp [freeVarOfListParPair, closedListParPair]
    | (a, b) :: as => by
      simp only [freeVarOfListParPair, closedListParPair, not_or, forall_and, Bool.and_eq_true,
        and_assoc, freeVarOf_iff_closed, freeVarOfListParPair_iff_closed]
end

/-- Law 6: a process is closed exactly when it has no free variables. `Closed` is `Ty.lean`'s
    structural, `decide`d reading; `freeVarOf` is the semantic predicate above; the tie is
    `freeVarOf_iff_closed` composed with the checker's own agreement lemma (`closed_eq_Closed`). -/
theorem closed_iff_no_freeVars (p : Par) : Closed p ↔ ∀ n, ¬ freeVarOf p n :=
  ((freeVarOf_iff_closed p).trans (closed_eq_Closed p)).symm

/-! ## Non-vacuity: the two predicates separate on concrete terms

A tie is worth stating only if both sides can be *wrong*, so these are the terms that make it
falsifiable, each discharged by computation: a closed term, a term free at exactly one level (the
level is not decoration — `freeVarOf p 4` is false of a `.free 3` occurrence), and a free occurrence
**nested** under `|`, which is what law 4b's congruence arms are about and the reason the predicate
is a structural recursion rather than a scan of the top-level fields.

The two mutations that would falsify the block above were checked while writing it: making
`freeVarAt` accept `.bound k` as free breaks the `Expr.evar` arm, and dropping one disjunct from
`freeVarOf`'s `Par` arm breaks the `Par` arm. Both are recorded here because "the proof compiled" is
not evidence that the statement has content. -/

/-- A term holding one expression and nothing else — `Json.lean`'s `one`, which this module cannot
    import (the JSON layer is downstream of the calculus). -/
def exprPar (e : Expr) : Par := Par.mk [] [] [] [e] [] [] [] []

-- The block as a `local simp` set. None of the 23 members is a *global* `@[simp]` lemma — the block
-- is a definition, and unfolding it on every `freeVarOf` in the tree is a rewrite the other files do
-- not want — but the fixtures below are exactly the place where the unfolding *is* the proof, and
-- naming 23 members in each `simp only […]` would bury what is being checked.
attribute [local simp] freeVarOf freeVarOfSend freeVarOfReceiveBind freeVarOfReceive freeVarOfNew
  freeVarOfMatchCase freeVarOfMatch freeVarOfExpr freeVarOfBundle freeVarOfGUnforgeable
  freeVarOfConnective freeVarOfListSend freeVarOfListReceive freeVarOfListNew freeVarOfListExpr
  freeVarOfListMatch freeVarOfListGUnforgeable freeVarOfListBundle freeVarOfListConnective
  freeVarOfListPar freeVarOfListReceiveBind freeVarOfListMatchCase freeVarOfListParPair
  freeVarAt freeVarInRemainder

/-- `.bound 0` with no enclosing binder is still *closed* in the model's judgment — `closedVar` reads
    every bound occurrence as a back-reference the normalizer resolves. -/
theorem bound_var_is_closed : Closed (exprPar (.evar (.bound 0))) := by
  simp [Closed, exprPar, closedListExpr, closedExpr, closedVar]

/-- A free occurrence is not closed, and the level is what the predicate is about. -/
theorem free_var_is_not_closed :
    ¬ Closed (exprPar (.evar (.free 3))) ∧ freeVarOf (exprPar (.evar (.free 3))) 3
      ∧ ¬ freeVarOf (exprPar (.evar (.free 3))) 4 := by
  refine ⟨?_, ?_, ?_⟩ <;>
    simp [Closed, exprPar, freeVarOf, closedListExpr, closedExpr, closedVar, freeVarOfListExpr,
      freeVarOfExpr, freeVarAt]

/-- A free level inside a `|` is free in the whole — the merge lemma law 4b's congruence arms use. -/
theorem free_var_is_free_under_par :
    freeVarOf (parMerge (exprPar (.evar (.free 7))) nilPar) 7 := by
  simp [exprPar]

end Rchain
