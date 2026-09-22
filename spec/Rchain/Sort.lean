import Rchain.Par
import Rchain.Cmp

set_option maxHeartbeats 100000000

/-!
# Canonicalization (Law 1) over the flat `Par`

Mirrors `models/src/main/scala/coop/rchain/models/rholang/sorter/ScoreTree.scala` and
`ordering.scala`: `Par` (and `ESet`/`EMap`) are canonicalized to a total order so that structural
equality of the sorted form *is* process equality up to α.

The total order is a hand-rolled structural `cmpPar` (constructor declaration order, lexicographic
via `lex`), order-isomorphic to the Scala `ScoreTree` order. It is defined by **direct mutual
recursion** (each list field has its own comparator, so the recursion is fully first-order), which
Lean accepts with a `sizeOf` measure.

Law 1 is `sortPar (sortPar p) = sortPar p` (idempotence) and
`sortPar (parMerge p q) = sortPar (parMerge q p)` (commutativity), proven from the total-order
bundle on each comparator via the generic `sortList` canonicality in `Rchain.Cmp`.
-/

namespace Rchain

open Comparator

/-! ## Leaf comparators (`Ground`, `Var`) -/

/-- Structural comparison of `Ground`, constructor-declaration order (`bool < int < str < uri <
bytes`). Every constructor but the last needs the `.lt`/`.gt` pair after its own arm: the `.lt` arm
answers "this constructor against anything later", the `.gt` arm "anything earlier against this one",
and both are reached only when no earlier group already decided the pair. -/
def cmpGround : Ground → Ground → Ordering
  | .bool b, .bool b' => _root_.cmp b b'
  | .bool _, _ => .lt | _, .bool _ => .gt
  | .int n, .int n' => _root_.cmp n n'
  | .int _, _ => .lt | _, .int _ => .gt
  | .str l, .str l' => cmpListF (fun n m => _root_.cmp n m) l l'
  | .str _, _ => .lt | _, .str _ => .gt
  | .uri l, .uri l' => cmpListF (fun n m => _root_.cmp n m) l l'
  | .uri _, _ => .lt | _, .uri _ => .gt
  | .bytes l, .bytes l' => cmpListF (fun n m => _root_.cmp n m) l l'

/-- Structural comparison of `Var`, constructor-declaration order (`bound < free < wildcard`). -/
def cmpVar : Var → Var → Ordering
  | .bound n, .bound m => _root_.cmp n m
  | .bound _, _ => .lt | _, .bound _ => .gt
  | .free n, .free m => _root_.cmp n m
  | .free _, _ => .lt | _, .free _ => .gt
  | .wildcard, .wildcard => .eq

/-- Compare two optional remainder variables, `none` first. A collection form's remainder is part of
its identity — `[…]` and `[…, ...rest]` are different patterns — so canonicalization must order by it
rather than ignore it: Law 1 over the surface the matcher actually reads (AUDIT C22). -/
def cmpOptionVar : Option Var → Option Var → Ordering
  | none, none => .eq
  | none, some _ => .lt
  | some _, none => .gt
  | some a, some b => cmpVar a b

/-- `cmpGround` is a lawful comparator. -/
def groundComparator : Comparator Ground where
  cmp := cmpGround
  eq_iff := by
    intro a b
    cases a <;> cases b <;> simp [cmpGround]
    all_goals first
      | exact cmp_eq_eq_iff
      | exact (linearOrderComparator Int).eq_iff
      | exact (listComparator (linearOrderComparator Nat)).eq_iff
  swap := by
    intro a b
    cases a <;> cases b <;> simp [cmpGround]
    all_goals first
      | rfl
      | exact (linearOrderComparator Bool).swap
      | exact (linearOrderComparator Int).swap
      | exact (listComparator (linearOrderComparator Nat)).swap
  lt_trans := by
    intro a b c h1 h2
    cases a <;> cases b <;> cases c <;> simp [cmpGround] at h1 h2 ⊢
    all_goals first
      | exact _root_.lt_trans h1 h2
      | exact (listComparator (linearOrderComparator Nat)).lt_trans h1 h2

/-- `cmpVar` is a lawful comparator. -/
def varComparator : Comparator Var where
  cmp := cmpVar
  eq_iff := by
    intro a b
    cases a <;> cases b <;> simp [cmpVar]
    all_goals exact cmp_eq_eq_iff
  swap := by
    intro a b
    cases a <;> cases b <;> simp [cmpVar]
    all_goals first
      | rfl
      | exact (linearOrderComparator Nat).swap
  lt_trans := by
    intro a b c h1 h2
    cases a <;> cases b <;> cases c <;> simp [cmpVar] at h1 h2 ⊢
    all_goals exact _root_.lt_trans h1 h2

@[simp] theorem cmpGround_eq_iff (a b : Ground) : cmpGround a b = Ordering.eq ↔ a = b := groundComparator.eq_iff
@[simp] theorem cmpGround_swap (a b : Ground) : cmpGround b a = Ordering.swap (cmpGround a b) := groundComparator.swap
@[simp] theorem cmpVar_eq_iff (a b : Var) : cmpVar a b = Ordering.eq ↔ a = b := varComparator.eq_iff
@[simp] theorem cmpVar_swap (a b : Var) : cmpVar b a = Ordering.swap (cmpVar a b) := varComparator.swap

/-! ## The comparator family (direct mutual recursion) -/

mutual
  def cmpPar : Par → Par → Ordering
    | Par.mk s r n e m u b c, Par.mk s' r' n' e' m' u' b' c' =>
        lex (cmpListSend s s') (lex (cmpListReceive r r') (lex (cmpListNew n n')
        (lex (cmpListExpr e e') (lex (cmpListMatch m m') (lex (cmpListGUnforgeable u u')
        (lex (cmpListBundle b b') (cmpListConnective c c')))))))
  termination_by p q => sizeOf p + sizeOf q
  def cmpSend : Send → Send → Ordering
    | Send.mk c d p, Send.mk c' d' p' => lex (cmpPar c c') (lex (cmpListPar d d') (_root_.cmp p p'))
  termination_by s t => sizeOf s + sizeOf t
  def cmpReceiveBind : ReceiveBind → ReceiveBind → Ordering
    | ReceiveBind.mk ps s n, ReceiveBind.mk ps' s' n' => lex (cmpListPar ps ps') (lex (cmpPar s s') (_root_.cmp n n'))
  termination_by s t => sizeOf s + sizeOf t
  def cmpReceive : Receive → Receive → Ordering
    | Receive.mk bs b p n, Receive.mk bs' b' p' n' => lex (cmpListReceiveBind bs bs') (lex (cmpPar b b') (lex (_root_.cmp p p') (_root_.cmp n n')))
  termination_by s t => sizeOf s + sizeOf t
  def cmpNew : New → New → Ordering
    | New.mk n b, New.mk n' b' => lex (_root_.cmp n n') (cmpPar b b')
  termination_by s t => sizeOf s + sizeOf t
  def cmpMatchCase : MatchCase → MatchCase → Ordering
    | MatchCase.mk p s n, MatchCase.mk p' s' n' => lex (cmpPar p p') (lex (cmpPar s s') (_root_.cmp n n'))
  termination_by s t => sizeOf s + sizeOf t
  def cmpMatch : Match → Match → Ordering
    | Match.mk t cs, Match.mk t' cs' => lex (cmpPar t t') (cmpListMatchCase cs cs')
  termination_by s t => sizeOf s + sizeOf t
  def cmpExpr : Expr → Expr → Ordering
    | Expr.ground g, Expr.ground g' => cmpGround g g'
    | Expr.ground _, _ => .lt | _, Expr.ground _ => .gt
    | Expr.evar v, Expr.evar v' => cmpVar v v'
    | Expr.evar _, _ => .lt | _, Expr.evar _ => .gt
    | Expr.eneg p, Expr.eneg p' => cmpPar p p'
    | Expr.eneg _, _ => .lt | _, Expr.eneg _ => .gt
    | Expr.enot p, Expr.enot p' => cmpPar p p'
    | Expr.enot _, _ => .lt | _, Expr.enot _ => .gt
    | Expr.eplus p q, Expr.eplus p' q' => lex (cmpPar p p') (cmpPar q q')
    | Expr.eplus _ _, _ => .lt | _, Expr.eplus _ _ => .gt
    | Expr.eminus p q, Expr.eminus p' q' => lex (cmpPar p p') (cmpPar q q')
    | Expr.eminus _ _, _ => .lt | _, Expr.eminus _ _ => .gt
    | Expr.emult p q, Expr.emult p' q' => lex (cmpPar p p') (cmpPar q q')
    | Expr.emult _ _, _ => .lt | _, Expr.emult _ _ => .gt
    | Expr.ediv p q, Expr.ediv p' q' => lex (cmpPar p p') (cmpPar q q')
    | Expr.ediv _ _, _ => .lt | _, Expr.ediv _ _ => .gt
    | Expr.emod p q, Expr.emod p' q' => lex (cmpPar p p') (cmpPar q q')
    | Expr.emod _ _, _ => .lt | _, Expr.emod _ _ => .gt
    | Expr.elt p q, Expr.elt p' q' => lex (cmpPar p p') (cmpPar q q')
    | Expr.elt _ _, _ => .lt | _, Expr.elt _ _ => .gt
    | Expr.ele p q, Expr.ele p' q' => lex (cmpPar p p') (cmpPar q q')
    | Expr.ele _ _, _ => .lt | _, Expr.ele _ _ => .gt
    | Expr.egt p q, Expr.egt p' q' => lex (cmpPar p p') (cmpPar q q')
    | Expr.egt _ _, _ => .lt | _, Expr.egt _ _ => .gt
    | Expr.ege p q, Expr.ege p' q' => lex (cmpPar p p') (cmpPar q q')
    | Expr.ege _ _, _ => .lt | _, Expr.ege _ _ => .gt
    | Expr.eeq p q, Expr.eeq p' q' => lex (cmpPar p p') (cmpPar q q')
    | Expr.eeq _ _, _ => .lt | _, Expr.eeq _ _ => .gt
    | Expr.eneq p q, Expr.eneq p' q' => lex (cmpPar p p') (cmpPar q q')
    | Expr.eneq _ _, _ => .lt | _, Expr.eneq _ _ => .gt
    | Expr.eand p q, Expr.eand p' q' => lex (cmpPar p p') (cmpPar q q')
    | Expr.eand _ _, _ => .lt | _, Expr.eand _ _ => .gt
    | Expr.eor p q, Expr.eor p' q' => lex (cmpPar p p') (cmpPar q q')
    | Expr.eor _ _, _ => .lt | _, Expr.eor _ _ => .gt
    | Expr.elist ps r, Expr.elist ps' r' => lex (cmpListPar ps ps') (cmpOptionVar r r')
    | Expr.elist _ _, _ => .lt | _, Expr.elist _ _ => .gt
    | Expr.etuple ps, Expr.etuple ps' => cmpListPar ps ps'
    | Expr.etuple _, _ => .lt | _, Expr.etuple _ => .gt
    | Expr.eset ps r, Expr.eset ps' r' => lex (cmpListPar ps ps') (cmpOptionVar r r')
    | Expr.eset _ _, _ => .lt | _, Expr.eset _ _ => .gt
    | Expr.emap kvs r, Expr.emap kvs' r' => lex (cmpListParPair kvs kvs') (cmpOptionVar r r')
  termination_by s t => sizeOf s + sizeOf t
  def cmpBundle : Bundle → Bundle → Ordering
    | Bundle.mk b w r, Bundle.mk b' w' r' => lex (cmpPar b b') (lex (_root_.cmp w w') (_root_.cmp r r'))
  termination_by s t => sizeOf s + sizeOf t
  def cmpGUnforgeable : GUnforgeable → GUnforgeable → Ordering
    | GUnforgeable.gPrivate n, GUnforgeable.gPrivate n' => _root_.cmp n n'
    | GUnforgeable.gPrivate _, _ => .lt | _, GUnforgeable.gPrivate _ => .gt
    | GUnforgeable.gDeployId n, GUnforgeable.gDeployId n' => _root_.cmp n n'
    | GUnforgeable.gDeployId _, _ => .lt | _, GUnforgeable.gDeployId _ => .gt
    | GUnforgeable.gDeployerId, GUnforgeable.gDeployerId => .eq
    | GUnforgeable.gDeployerId, _ => .lt | _, GUnforgeable.gDeployerId => .gt
    | GUnforgeable.gSysAuthToken, GUnforgeable.gSysAuthToken => .eq
  termination_by s t => sizeOf s + sizeOf t
  def cmpConnective : Connective → Connective → Ordering
    | Connective.connAnd ps, Connective.connAnd ps' => cmpListPar ps ps'
    | Connective.connAnd _, _ => .lt | _, Connective.connAnd _ => .gt
    | Connective.connOr ps, Connective.connOr ps' => cmpListPar ps ps'
    | Connective.connOr _, _ => .lt | _, Connective.connOr _ => .gt
    | Connective.connNot p, Connective.connNot p' => cmpPar p p'
    | Connective.connNot _, _ => .lt | _, Connective.connNot _ => .gt
    | Connective.connVarRef d n, Connective.connVarRef d' n' => lex (_root_.cmp d d') (_root_.cmp n n')
  termination_by s t => sizeOf s + sizeOf t
  def cmpListSend : List Send → List Send → Ordering
    | [], [] => .eq | [], _ => .lt | _, [] => .gt
    | a :: as, b :: bs => lex (cmpSend a b) (cmpListSend as bs)
  termination_by l l' => sizeOf l + sizeOf l'
  def cmpListReceive : List Receive → List Receive → Ordering
    | [], [] => .eq | [], _ => .lt | _, [] => .gt
    | a :: as, b :: bs => lex (cmpReceive a b) (cmpListReceive as bs)
  termination_by l l' => sizeOf l + sizeOf l'
  def cmpListNew : List New → List New → Ordering
    | [], [] => .eq | [], _ => .lt | _, [] => .gt
    | a :: as, b :: bs => lex (cmpNew a b) (cmpListNew as bs)
  termination_by l l' => sizeOf l + sizeOf l'
  def cmpListExpr : List Expr → List Expr → Ordering
    | [], [] => .eq | [], _ => .lt | _, [] => .gt
    | a :: as, b :: bs => lex (cmpExpr a b) (cmpListExpr as bs)
  termination_by l l' => sizeOf l + sizeOf l'
  def cmpListMatch : List Match → List Match → Ordering
    | [], [] => .eq | [], _ => .lt | _, [] => .gt
    | a :: as, b :: bs => lex (cmpMatch a b) (cmpListMatch as bs)
  termination_by l l' => sizeOf l + sizeOf l'
  def cmpListGUnforgeable : List GUnforgeable → List GUnforgeable → Ordering
    | [], [] => .eq | [], _ => .lt | _, [] => .gt
    | a :: as, b :: bs => lex (cmpGUnforgeable a b) (cmpListGUnforgeable as bs)
  termination_by l l' => sizeOf l + sizeOf l'
  def cmpListBundle : List Bundle → List Bundle → Ordering
    | [], [] => .eq | [], _ => .lt | _, [] => .gt
    | a :: as, b :: bs => lex (cmpBundle a b) (cmpListBundle as bs)
  termination_by l l' => sizeOf l + sizeOf l'
  def cmpListConnective : List Connective → List Connective → Ordering
    | [], [] => .eq | [], _ => .lt | _, [] => .gt
    | a :: as, b :: bs => lex (cmpConnective a b) (cmpListConnective as bs)
  termination_by l l' => sizeOf l + sizeOf l'
  def cmpListPar : List Par → List Par → Ordering
    | [], [] => .eq | [], _ => .lt | _, [] => .gt
    | a :: as, b :: bs => lex (cmpPar a b) (cmpListPar as bs)
  termination_by l l' => sizeOf l + sizeOf l'
  def cmpListReceiveBind : List ReceiveBind → List ReceiveBind → Ordering
    | [], [] => .eq | [], _ => .lt | _, [] => .gt
    | a :: as, b :: bs => lex (cmpReceiveBind a b) (cmpListReceiveBind as bs)
  termination_by l l' => sizeOf l + sizeOf l'
  def cmpListMatchCase : List MatchCase → List MatchCase → Ordering
    | [], [] => .eq | [], _ => .lt | _, [] => .gt
    | a :: as, b :: bs => lex (cmpMatchCase a b) (cmpListMatchCase as bs)
  termination_by l l' => sizeOf l + sizeOf l'
  def cmpListParPair : List (Par × Par) → List (Par × Par) → Ordering
    | [], [] => .eq | [], _ => .lt | _, [] => .gt
    | (a, b) :: as, (c, d) :: bs => lex (lex (cmpPar a c) (cmpPar b d)) (cmpListParPair as bs)
  termination_by l l' => sizeOf l + sizeOf l'
end
/-! ## Lawfulness: `eq_iff` -/

/-- `cmpGUnforgeable` reflects equality (leaf; no `Par` recursion). -/
theorem cmpGUnforgeable_eq_iff (s t : GUnforgeable) : cmpGUnforgeable s t = Ordering.eq ↔ s = t := by
  cases s <;> cases t <;> simp [cmpGUnforgeable]
  all_goals exact cmp_eq_eq_iff

theorem cmpListGUnforgeable_eq_iff (l l' : List GUnforgeable) : cmpListGUnforgeable l l' = Ordering.eq ↔ l = l' := by
  induction l generalizing l' with
  | nil => cases l' <;> simp [cmpListGUnforgeable]
  | cons a as ih =>
      cases l' with
      | nil => simp [cmpListGUnforgeable]
      | cons b bs => simp [cmpListGUnforgeable, lex_eq_iff, cmpGUnforgeable_eq_iff, ih, List.cons.injEq]

/-! `cmpExpr` is a 20-constructor well-founded function; `simp`/`rw` on it hit Lean's recursion
    depth (the equation lemmas are too large), so its laws are axiomatized here. -/
axiom cmpExpr_eq_iff (s t : Expr) : cmpExpr s t = Ordering.eq ↔ s = t

/-! The remaining 9 element + 11 list `eq_iff` laws, by one-argument mutual induction
    (the first argument always descends structurally; cf. `sortX_idempotent`). -/
mutual
  theorem cmpPar_eq_iff : ∀ p : Par, ∀ q : Par, cmpPar p q = Ordering.eq ↔ p = q
    | Par.mk s r n e m u b c, Par.mk s' r' n' e' m' u' b' c' => by
        have hs := cmpListSend_eq_iff s s'
        have hr := cmpListReceive_eq_iff r r'
        have hn := cmpListNew_eq_iff n n'
        have he := cmpListExpr_eq_iff e e'
        have hm := cmpListMatch_eq_iff m m'
        have hu := cmpListGUnforgeable_eq_iff u u'
        have hb := cmpListBundle_eq_iff b b'
        have hc := cmpListConnective_eq_iff c c'
        simp [cmpPar, lex_eq_iff, hs, hr, hn, he, hm, hu, hb, hc]
  termination_by p => sizeOf p

  theorem cmpSend_eq_iff : ∀ s : Send, ∀ t : Send, cmpSend s t = Ordering.eq ↔ s = t
    | Send.mk c d p, Send.mk c' d' p' => by
        have hc := cmpPar_eq_iff c c'
        have hd := cmpListPar_eq_iff d d'
        simp [cmpSend, lex_eq_iff, hc, hd, cmp_eq_eq_iff]
  termination_by s => sizeOf s

  theorem cmpReceiveBind_eq_iff : ∀ s : ReceiveBind, ∀ t : ReceiveBind, cmpReceiveBind s t = Ordering.eq ↔ s = t
    | ReceiveBind.mk ps s n, ReceiveBind.mk ps' s' n' => by
        have hps := cmpListPar_eq_iff ps ps'
        have hs := cmpPar_eq_iff s s'
        simp [cmpReceiveBind, lex_eq_iff, hps, hs, cmp_eq_eq_iff]
  termination_by s => sizeOf s

  theorem cmpReceive_eq_iff : ∀ s : Receive, ∀ t : Receive, cmpReceive s t = Ordering.eq ↔ s = t
    | Receive.mk bs b p n, Receive.mk bs' b' p' n' => by
        have hbs := cmpListReceiveBind_eq_iff bs bs'
        have hb := cmpPar_eq_iff b b'
        simp [cmpReceive, lex_eq_iff, hbs, hb, cmp_eq_eq_iff]
  termination_by s => sizeOf s

  theorem cmpNew_eq_iff : ∀ s : New, ∀ t : New, cmpNew s t = Ordering.eq ↔ s = t
    | New.mk n b, New.mk n' b' => by
        have hb := cmpPar_eq_iff b b'
        simp [cmpNew, lex_eq_iff, hb, cmp_eq_eq_iff]
  termination_by s => sizeOf s

  theorem cmpMatchCase_eq_iff : ∀ s : MatchCase, ∀ t : MatchCase, cmpMatchCase s t = Ordering.eq ↔ s = t
    | MatchCase.mk p s n, MatchCase.mk p' s' n' => by
        have hp := cmpPar_eq_iff p p'
        have hs := cmpPar_eq_iff s s'
        simp [cmpMatchCase, lex_eq_iff, hp, hs, cmp_eq_eq_iff]
  termination_by s => sizeOf s

  theorem cmpMatch_eq_iff : ∀ s : Match, ∀ t : Match, cmpMatch s t = Ordering.eq ↔ s = t
    | Match.mk t cs, Match.mk t' cs' => by
        have ht := cmpPar_eq_iff t t'
        have hcs := cmpListMatchCase_eq_iff cs cs'
        simp [cmpMatch, lex_eq_iff, ht, hcs]
  termination_by s => sizeOf s

  theorem cmpBundle_eq_iff : ∀ s : Bundle, ∀ t : Bundle, cmpBundle s t = Ordering.eq ↔ s = t
    | Bundle.mk b w r, Bundle.mk b' w' r' => by
        have hb := cmpPar_eq_iff b b'
        simp [cmpBundle, lex_eq_iff, hb, cmp_eq_eq_iff]
  termination_by s => sizeOf s

  theorem cmpConnective_eq_iff : ∀ s : Connective, ∀ t : Connective, cmpConnective s t = Ordering.eq ↔ s = t
    | Connective.connAnd ps, Connective.connAnd ps' => by
        have h := cmpListPar_eq_iff ps ps'; simp [cmpConnective, h]
    | Connective.connOr ps, Connective.connOr ps' => by
        have h := cmpListPar_eq_iff ps ps'; simp [cmpConnective, h]
    | Connective.connNot p, Connective.connNot p' => by
        have h := cmpPar_eq_iff p p'; simp [cmpConnective, h]
    | Connective.connVarRef d n, Connective.connVarRef d' n' => by
        simp [cmpConnective, lex_eq_iff, cmp_eq_eq_iff]
    | Connective.connAnd _, Connective.connOr _ | Connective.connAnd _, Connective.connNot _ |
      Connective.connAnd _, Connective.connVarRef _ _ |
      Connective.connOr _, Connective.connAnd _ | Connective.connOr _, Connective.connNot _ |
      Connective.connOr _, Connective.connVarRef _ _ |
      Connective.connNot _, Connective.connAnd _ | Connective.connNot _, Connective.connOr _ |
      Connective.connNot _, Connective.connVarRef _ _ |
      Connective.connVarRef _ _, Connective.connAnd _ | Connective.connVarRef _ _, Connective.connOr _ |
      Connective.connVarRef _ _, Connective.connNot _ => by
        simp [cmpConnective]
  termination_by s => sizeOf s

  theorem cmpListSend_eq_iff : ∀ l : List Send, ∀ l' : List Send, cmpListSend l l' = Ordering.eq ↔ l = l'
    | [], [] => by simp [cmpListSend]
    | [], _ :: _ => by simp [cmpListSend]
    | _ :: _, [] => by simp [cmpListSend]
    | a :: as, b :: bs => by
        have hab := cmpSend_eq_iff a b
        have htail := cmpListSend_eq_iff as bs
        simp [cmpListSend, lex_eq_iff, hab, htail]
  termination_by l => sizeOf l

  theorem cmpListReceive_eq_iff : ∀ l : List Receive, ∀ l' : List Receive, cmpListReceive l l' = Ordering.eq ↔ l = l'
    | [], [] => by simp [cmpListReceive]
    | [], _ :: _ => by simp [cmpListReceive]
    | _ :: _, [] => by simp [cmpListReceive]
    | a :: as, b :: bs => by
        have hab := cmpReceive_eq_iff a b
        have htail := cmpListReceive_eq_iff as bs
        simp [cmpListReceive, lex_eq_iff, hab, htail]
  termination_by l => sizeOf l

  theorem cmpListNew_eq_iff : ∀ l : List New, ∀ l' : List New, cmpListNew l l' = Ordering.eq ↔ l = l'
    | [], [] => by simp [cmpListNew]
    | [], _ :: _ => by simp [cmpListNew]
    | _ :: _, [] => by simp [cmpListNew]
    | a :: as, b :: bs => by
        have hab := cmpNew_eq_iff a b
        have htail := cmpListNew_eq_iff as bs
        simp [cmpListNew, lex_eq_iff, hab, htail]
  termination_by l => sizeOf l

  theorem cmpListExpr_eq_iff : ∀ l : List Expr, ∀ l' : List Expr, cmpListExpr l l' = Ordering.eq ↔ l = l'
    | [], [] => by simp [cmpListExpr]
    | [], _ :: _ => by simp [cmpListExpr]
    | _ :: _, [] => by simp [cmpListExpr]
    | a :: as, b :: bs => by
        have hab := cmpExpr_eq_iff a b
        have htail := cmpListExpr_eq_iff as bs
        simp [cmpListExpr, lex_eq_iff, hab, htail]
  termination_by l => sizeOf l

  theorem cmpListMatch_eq_iff : ∀ l : List Match, ∀ l' : List Match, cmpListMatch l l' = Ordering.eq ↔ l = l'
    | [], [] => by simp [cmpListMatch]
    | [], _ :: _ => by simp [cmpListMatch]
    | _ :: _, [] => by simp [cmpListMatch]
    | a :: as, b :: bs => by
        have hab := cmpMatch_eq_iff a b
        have htail := cmpListMatch_eq_iff as bs
        simp [cmpListMatch, lex_eq_iff, hab, htail]
  termination_by l => sizeOf l

  theorem cmpListBundle_eq_iff : ∀ l : List Bundle, ∀ l' : List Bundle, cmpListBundle l l' = Ordering.eq ↔ l = l'
    | [], [] => by simp [cmpListBundle]
    | [], _ :: _ => by simp [cmpListBundle]
    | _ :: _, [] => by simp [cmpListBundle]
    | a :: as, b :: bs => by
        have hab := cmpBundle_eq_iff a b
        have htail := cmpListBundle_eq_iff as bs
        simp [cmpListBundle, lex_eq_iff, hab, htail]
  termination_by l => sizeOf l

  theorem cmpListConnective_eq_iff : ∀ l : List Connective, ∀ l' : List Connective, cmpListConnective l l' = Ordering.eq ↔ l = l'
    | [], [] => by simp [cmpListConnective]
    | [], _ :: _ => by simp [cmpListConnective]
    | _ :: _, [] => by simp [cmpListConnective]
    | a :: as, b :: bs => by
        have hab := cmpConnective_eq_iff a b
        have htail := cmpListConnective_eq_iff as bs
        simp [cmpListConnective, lex_eq_iff, hab, htail]
  termination_by l => sizeOf l

  theorem cmpListPar_eq_iff : ∀ l : List Par, ∀ l' : List Par, cmpListPar l l' = Ordering.eq ↔ l = l'
    | [], [] => by simp [cmpListPar]
    | [], _ :: _ => by simp [cmpListPar]
    | _ :: _, [] => by simp [cmpListPar]
    | a :: as, b :: bs => by
        have hab := cmpPar_eq_iff a b
        have htail := cmpListPar_eq_iff as bs
        simp [cmpListPar, lex_eq_iff, hab, htail]
  termination_by l => sizeOf l

  theorem cmpListReceiveBind_eq_iff : ∀ l : List ReceiveBind, ∀ l' : List ReceiveBind, cmpListReceiveBind l l' = Ordering.eq ↔ l = l'
    | [], [] => by simp [cmpListReceiveBind]
    | [], _ :: _ => by simp [cmpListReceiveBind]
    | _ :: _, [] => by simp [cmpListReceiveBind]
    | a :: as, b :: bs => by
        have hab := cmpReceiveBind_eq_iff a b
        have htail := cmpListReceiveBind_eq_iff as bs
        simp [cmpListReceiveBind, lex_eq_iff, hab, htail]
  termination_by l => sizeOf l

  theorem cmpListMatchCase_eq_iff : ∀ l : List MatchCase, ∀ l' : List MatchCase, cmpListMatchCase l l' = Ordering.eq ↔ l = l'
    | [], [] => by simp [cmpListMatchCase]
    | [], _ :: _ => by simp [cmpListMatchCase]
    | _ :: _, [] => by simp [cmpListMatchCase]
    | a :: as, b :: bs => by
        have hab := cmpMatchCase_eq_iff a b
        have htail := cmpListMatchCase_eq_iff as bs
        simp [cmpListMatchCase, lex_eq_iff, hab, htail]
  termination_by l => sizeOf l

  theorem cmpListParPair_eq_iff : ∀ l : List (Par × Par), ∀ l' : List (Par × Par), cmpListParPair l l' = Ordering.eq ↔ l = l'
    | [], [] => by simp [cmpListParPair]
    | [], _ :: _ => by simp [cmpListParPair]
    | _ :: _, [] => by simp [cmpListParPair]
    | (a1, a2) :: as, (b1, b2) :: bs => by
        have ha1 := cmpPar_eq_iff a1 b1
        have ha2 := cmpPar_eq_iff a2 b2
        have htail := cmpListParPair_eq_iff as bs
        simp [cmpListParPair, lex_eq_iff, ha1, ha2, htail]
  termination_by l => sizeOf l
end

/-! ## Lawfulness: `swap` -/

/-- `cmpGUnforgeable` `swap` law (leaf). -/
theorem cmpGUnforgeable_swap (s t : GUnforgeable) : cmpGUnforgeable t s = Ordering.swap (cmpGUnforgeable s t) := by
  cases s <;> cases t <;> simp [cmpGUnforgeable]
  all_goals first
    | rfl
    | exact (linearOrderComparator Nat).swap

theorem cmpListGUnforgeable_swap (l l' : List GUnforgeable) : cmpListGUnforgeable l' l = Ordering.swap (cmpListGUnforgeable l l') := by
  induction l generalizing l' with
  | nil => cases l' <;> simp [cmpListGUnforgeable, Ordering.swap]
  | cons a as ih =>
      cases l' with
      | nil => simp [cmpListGUnforgeable, Ordering.swap]
      | cons b bs =>
          simp only [cmpListGUnforgeable]
          rw [swap_lex, ← cmpGUnforgeable_swap, ← ih bs]

axiom cmpExpr_swap (s t : Expr) : cmpExpr t s = Ordering.swap (cmpExpr s t)

/-! The remaining 9 element + 11 list `swap` laws, by one-argument mutual induction. -/
mutual
  theorem cmpPar_swap : ∀ p : Par, ∀ q : Par, cmpPar q p = Ordering.swap (cmpPar p q)
    | Par.mk s r n e m u b c, Par.mk s' r' n' e' m' u' b' c' => by
        have hs := cmpListSend_swap s s'
        have hr := cmpListReceive_swap r r'
        have hn := cmpListNew_swap n n'
        have he := cmpListExpr_swap e e'
        have hm := cmpListMatch_swap m m'
        have hu := cmpListGUnforgeable_swap u u'
        have hb := cmpListBundle_swap b b'
        have hc := cmpListConnective_swap c c'
        simp [cmpPar, swap_lex, hs, hr, hn, he, hm, hu, hb, hc]
  termination_by p => sizeOf p

  theorem cmpSend_swap : ∀ s : Send, ∀ t : Send, cmpSend t s = Ordering.swap (cmpSend s t)
    | Send.mk c d p, Send.mk c' d' p' => by
        have hc := cmpPar_swap c c'
        have hd := cmpListPar_swap d d'
        simp [cmpSend, swap_lex, hc, hd, (linearOrderComparator Bool).swap]
  termination_by s => sizeOf s

  theorem cmpReceiveBind_swap : ∀ s : ReceiveBind, ∀ t : ReceiveBind, cmpReceiveBind t s = Ordering.swap (cmpReceiveBind s t)
    | ReceiveBind.mk ps s n, ReceiveBind.mk ps' s' n' => by
        have hps := cmpListPar_swap ps ps'
        have hs := cmpPar_swap s s'
        simp [cmpReceiveBind, swap_lex, hps, hs, (linearOrderComparator Nat).swap]
  termination_by s => sizeOf s

  theorem cmpReceive_swap : ∀ s : Receive, ∀ t : Receive, cmpReceive t s = Ordering.swap (cmpReceive s t)
    | Receive.mk bs b p n, Receive.mk bs' b' p' n' => by
        have hbs := cmpListReceiveBind_swap bs bs'
        have hb := cmpPar_swap b b'
        simp [cmpReceive, swap_lex, hbs, hb, (linearOrderComparator Bool).swap, (linearOrderComparator Nat).swap]
  termination_by s => sizeOf s

  theorem cmpNew_swap : ∀ s : New, ∀ t : New, cmpNew t s = Ordering.swap (cmpNew s t)
    | New.mk n b, New.mk n' b' => by
        have hb := cmpPar_swap b b'
        simp [cmpNew, swap_lex, hb, (linearOrderComparator Nat).swap]
  termination_by s => sizeOf s

  theorem cmpMatchCase_swap : ∀ s : MatchCase, ∀ t : MatchCase, cmpMatchCase t s = Ordering.swap (cmpMatchCase s t)
    | MatchCase.mk p s n, MatchCase.mk p' s' n' => by
        have hp := cmpPar_swap p p'
        have hs := cmpPar_swap s s'
        simp [cmpMatchCase, swap_lex, hp, hs, (linearOrderComparator Nat).swap]
  termination_by s => sizeOf s

  theorem cmpMatch_swap : ∀ s : Match, ∀ t : Match, cmpMatch t s = Ordering.swap (cmpMatch s t)
    | Match.mk t cs, Match.mk t' cs' => by
        have ht := cmpPar_swap t t'
        have hcs := cmpListMatchCase_swap cs cs'
        simp [cmpMatch, swap_lex, ht, hcs]
  termination_by s => sizeOf s

  theorem cmpBundle_swap : ∀ s : Bundle, ∀ t : Bundle, cmpBundle t s = Ordering.swap (cmpBundle s t)
    | Bundle.mk b w r, Bundle.mk b' w' r' => by
        have hb := cmpPar_swap b b'
        simp [cmpBundle, swap_lex, hb, (linearOrderComparator Bool).swap]
  termination_by s => sizeOf s

  theorem cmpConnective_swap : ∀ s : Connective, ∀ t : Connective, cmpConnective t s = Ordering.swap (cmpConnective s t)
    | Connective.connAnd ps, Connective.connAnd ps' => by
        have h := cmpListPar_swap ps ps'; simp [cmpConnective, h]
    | Connective.connOr ps, Connective.connOr ps' => by
        have h := cmpListPar_swap ps ps'; simp [cmpConnective, h]
    | Connective.connNot p, Connective.connNot p' => by
        have h := cmpPar_swap p p'; simp [cmpConnective, h]
    | Connective.connVarRef d n, Connective.connVarRef d' n' => by
        simp [cmpConnective, swap_lex, (linearOrderComparator Nat).swap]
    | Connective.connAnd _, Connective.connOr _ | Connective.connAnd _, Connective.connNot _ |
      Connective.connAnd _, Connective.connVarRef _ _ |
      Connective.connOr _, Connective.connAnd _ | Connective.connOr _, Connective.connNot _ |
      Connective.connOr _, Connective.connVarRef _ _ |
      Connective.connNot _, Connective.connAnd _ | Connective.connNot _, Connective.connOr _ |
      Connective.connNot _, Connective.connVarRef _ _ |
      Connective.connVarRef _ _, Connective.connAnd _ | Connective.connVarRef _ _, Connective.connOr _ |
      Connective.connVarRef _ _, Connective.connNot _ => by
        simp [cmpConnective, Ordering.swap]
  termination_by s => sizeOf s

  theorem cmpListSend_swap : ∀ l : List Send, ∀ l' : List Send, cmpListSend l' l = Ordering.swap (cmpListSend l l')
    | [], [] => by simp [cmpListSend, Ordering.swap]
    | [], _ :: _ => by simp [cmpListSend, Ordering.swap]
    | _ :: _, [] => by simp [cmpListSend, Ordering.swap]
    | a :: as, b :: bs => by
        have hab := cmpSend_swap a b
        have htail := cmpListSend_swap as bs
        simp only [cmpListSend]
        rw [swap_lex, ← hab, ← htail]
  termination_by l => sizeOf l

  theorem cmpListReceive_swap : ∀ l : List Receive, ∀ l' : List Receive, cmpListReceive l' l = Ordering.swap (cmpListReceive l l')
    | [], [] => by simp [cmpListReceive, Ordering.swap]
    | [], _ :: _ => by simp [cmpListReceive, Ordering.swap]
    | _ :: _, [] => by simp [cmpListReceive, Ordering.swap]
    | a :: as, b :: bs => by
        have hab := cmpReceive_swap a b
        have htail := cmpListReceive_swap as bs
        simp only [cmpListReceive]
        rw [swap_lex, ← hab, ← htail]
  termination_by l => sizeOf l

  theorem cmpListNew_swap : ∀ l : List New, ∀ l' : List New, cmpListNew l' l = Ordering.swap (cmpListNew l l')
    | [], [] => by simp [cmpListNew, Ordering.swap]
    | [], _ :: _ => by simp [cmpListNew, Ordering.swap]
    | _ :: _, [] => by simp [cmpListNew, Ordering.swap]
    | a :: as, b :: bs => by
        have hab := cmpNew_swap a b
        have htail := cmpListNew_swap as bs
        simp only [cmpListNew]
        rw [swap_lex, ← hab, ← htail]
  termination_by l => sizeOf l

  theorem cmpListExpr_swap : ∀ l : List Expr, ∀ l' : List Expr, cmpListExpr l' l = Ordering.swap (cmpListExpr l l')
    | [], [] => by simp [cmpListExpr, Ordering.swap]
    | [], _ :: _ => by simp [cmpListExpr, Ordering.swap]
    | _ :: _, [] => by simp [cmpListExpr, Ordering.swap]
    | a :: as, b :: bs => by
        have hab := cmpExpr_swap a b
        have htail := cmpListExpr_swap as bs
        simp only [cmpListExpr]
        rw [swap_lex, ← hab, ← htail]
  termination_by l => sizeOf l

  theorem cmpListMatch_swap : ∀ l : List Match, ∀ l' : List Match, cmpListMatch l' l = Ordering.swap (cmpListMatch l l')
    | [], [] => by simp [cmpListMatch, Ordering.swap]
    | [], _ :: _ => by simp [cmpListMatch, Ordering.swap]
    | _ :: _, [] => by simp [cmpListMatch, Ordering.swap]
    | a :: as, b :: bs => by
        have hab := cmpMatch_swap a b
        have htail := cmpListMatch_swap as bs
        simp only [cmpListMatch]
        rw [swap_lex, ← hab, ← htail]
  termination_by l => sizeOf l

  theorem cmpListBundle_swap : ∀ l : List Bundle, ∀ l' : List Bundle, cmpListBundle l' l = Ordering.swap (cmpListBundle l l')
    | [], [] => by simp [cmpListBundle, Ordering.swap]
    | [], _ :: _ => by simp [cmpListBundle, Ordering.swap]
    | _ :: _, [] => by simp [cmpListBundle, Ordering.swap]
    | a :: as, b :: bs => by
        have hab := cmpBundle_swap a b
        have htail := cmpListBundle_swap as bs
        simp only [cmpListBundle]
        rw [swap_lex, ← hab, ← htail]
  termination_by l => sizeOf l

  theorem cmpListConnective_swap : ∀ l : List Connective, ∀ l' : List Connective, cmpListConnective l' l = Ordering.swap (cmpListConnective l l')
    | [], [] => by simp [cmpListConnective, Ordering.swap]
    | [], _ :: _ => by simp [cmpListConnective, Ordering.swap]
    | _ :: _, [] => by simp [cmpListConnective, Ordering.swap]
    | a :: as, b :: bs => by
        have hab := cmpConnective_swap a b
        have htail := cmpListConnective_swap as bs
        simp only [cmpListConnective]
        rw [swap_lex, ← hab, ← htail]
  termination_by l => sizeOf l

  theorem cmpListPar_swap : ∀ l : List Par, ∀ l' : List Par, cmpListPar l' l = Ordering.swap (cmpListPar l l')
    | [], [] => by simp [cmpListPar, Ordering.swap]
    | [], _ :: _ => by simp [cmpListPar, Ordering.swap]
    | _ :: _, [] => by simp [cmpListPar, Ordering.swap]
    | a :: as, b :: bs => by
        have hab := cmpPar_swap a b
        have htail := cmpListPar_swap as bs
        simp only [cmpListPar]
        rw [swap_lex, ← hab, ← htail]
  termination_by l => sizeOf l

  theorem cmpListReceiveBind_swap : ∀ l : List ReceiveBind, ∀ l' : List ReceiveBind, cmpListReceiveBind l' l = Ordering.swap (cmpListReceiveBind l l')
    | [], [] => by simp [cmpListReceiveBind, Ordering.swap]
    | [], _ :: _ => by simp [cmpListReceiveBind, Ordering.swap]
    | _ :: _, [] => by simp [cmpListReceiveBind, Ordering.swap]
    | a :: as, b :: bs => by
        have hab := cmpReceiveBind_swap a b
        have htail := cmpListReceiveBind_swap as bs
        simp only [cmpListReceiveBind]
        rw [swap_lex, ← hab, ← htail]
  termination_by l => sizeOf l

  theorem cmpListMatchCase_swap : ∀ l : List MatchCase, ∀ l' : List MatchCase, cmpListMatchCase l' l = Ordering.swap (cmpListMatchCase l l')
    | [], [] => by simp [cmpListMatchCase, Ordering.swap]
    | [], _ :: _ => by simp [cmpListMatchCase, Ordering.swap]
    | _ :: _, [] => by simp [cmpListMatchCase, Ordering.swap]
    | a :: as, b :: bs => by
        have hab := cmpMatchCase_swap a b
        have htail := cmpListMatchCase_swap as bs
        simp only [cmpListMatchCase]
        rw [swap_lex, ← hab, ← htail]
  termination_by l => sizeOf l

  theorem cmpListParPair_swap : ∀ l : List (Par × Par), ∀ l' : List (Par × Par), cmpListParPair l' l = Ordering.swap (cmpListParPair l l')
    | [], [] => by simp [cmpListParPair, Ordering.swap]
    | [], _ :: _ => by simp [cmpListParPair, Ordering.swap]
    | _ :: _, [] => by simp [cmpListParPair, Ordering.swap]
    | (a1, a2) :: as, (b1, b2) :: bs => by
        have ha1 := cmpPar_swap a1 b1
        have ha2 := cmpPar_swap a2 b2
        have htail := cmpListParPair_swap as bs
        simp only [cmpListParPair]
        rw [swap_lex, swap_lex, ← ha1, ← ha2, ← htail]
  termination_by l => sizeOf l
end

/-! ## Lawfulness: `lt_trans` (RESIDUAL AXIOMS — **12**, down from 69)

The 12 **list** comparators' `eq_iff`/`swap`/`lt_trans` laws are now **discharged** (direct induction
on the list, composing the element law with `lex_eq_iff`/`swap_lex`/`lex_lt_trans`); they were never
the hard part. The remaining **12 axioms** are `cmpExpr`'s `eq_iff` and `swap` plus the ten element
comparators' `lt_trans` laws (`cmpPar`/`cmpSend`/…/`cmpConnective`; `cmpGUnforgeable_lt_trans` is
proved), which need mutual induction over the AST.

**This header said 33 and `spec/INVENTORY.md` said 30, and both were wrong** — counted by
`Rchain/Laws.lean` (the law register), whose axiom-accounting check compares the axioms its rows cite
against the elaborated environment, so a number here cannot drift again without the register
disagreeing.

Discharging the element laws is blocked by a Lean limitation, not by choice:

  * a `mutual` theorem block over the **two-argument** `cmpX` family hangs the termination checker —
    both with bare `termination_by p q => sizeOf p + sizeOf q` and with
    `decreasing_by all_goals (simp_wf; omega)`;
  * the generated induction principle `Rchain.cmpPar.mutual_induct` fails to *derive*:
    "Cannot derive functional induction principle" with a deterministic `whnf` heartbeat timeout.

The one-argument `sortX_idempotent` mutual block (above) proves fine; the blocker is specific to the
two-argument sum measure. The path forward is a refactor — a single well-founded recursion over a sum
type `Par ⊕ Send ⊕ … ⊕ List (Par × Par)`, or Mathlib `SizeOf`/`Finset` machinery in Phase 1 — rather
than more tactics.
-/

axiom cmpPar_lt_trans (p q r : Par) : cmpPar p q = Ordering.lt → cmpPar q r = Ordering.lt → cmpPar p r = Ordering.lt
axiom cmpSend_lt_trans (s t u : Send) : cmpSend s t = Ordering.lt → cmpSend t u = Ordering.lt → cmpSend s u = Ordering.lt
axiom cmpReceiveBind_lt_trans (s t u : ReceiveBind) : cmpReceiveBind s t = Ordering.lt → cmpReceiveBind t u = Ordering.lt → cmpReceiveBind s u = Ordering.lt
axiom cmpReceive_lt_trans (s t u : Receive) : cmpReceive s t = Ordering.lt → cmpReceive t u = Ordering.lt → cmpReceive s u = Ordering.lt
axiom cmpNew_lt_trans (s t u : New) : cmpNew s t = Ordering.lt → cmpNew t u = Ordering.lt → cmpNew s u = Ordering.lt
axiom cmpMatchCase_lt_trans (s t u : MatchCase) : cmpMatchCase s t = Ordering.lt → cmpMatchCase t u = Ordering.lt → cmpMatchCase s u = Ordering.lt
axiom cmpMatch_lt_trans (s t u : Match) : cmpMatch s t = Ordering.lt → cmpMatch t u = Ordering.lt → cmpMatch s u = Ordering.lt
axiom cmpExpr_lt_trans (s t u : Expr) : cmpExpr s t = Ordering.lt → cmpExpr t u = Ordering.lt → cmpExpr s u = Ordering.lt
axiom cmpBundle_lt_trans (s t u : Bundle) : cmpBundle s t = Ordering.lt → cmpBundle t u = Ordering.lt → cmpBundle s u = Ordering.lt
theorem cmpGUnforgeable_lt_trans (s t u : GUnforgeable) :
    cmpGUnforgeable s t = Ordering.lt → cmpGUnforgeable t u = Ordering.lt → cmpGUnforgeable s u = Ordering.lt := by
  intro h1 h2
  cases s <;> cases t <;> cases u <;> simp [cmpGUnforgeable] at h1 h2 ⊢
  all_goals exact _root_.lt_trans h1 h2
axiom cmpConnective_lt_trans (s t u : Connective) : cmpConnective s t = Ordering.lt → cmpConnective t u = Ordering.lt → cmpConnective s u = Ordering.lt
theorem cmpListSend_lt_trans (l l' l'' : List Send) : cmpListSend l l' = Ordering.lt → cmpListSend l' l'' = Ordering.lt → cmpListSend l l'' = Ordering.lt := by
  induction l generalizing l' l'' with
  | nil => intro h1 h2; cases l' <;> cases l'' <;> simp [cmpListSend] at h1 h2 ⊢
  | cons a as ih =>
      intro h1 h2
      cases l' with
      | nil => simp [cmpListSend] at h1
      | cons b bs =>
          cases l'' with
          | nil => simp [cmpListSend] at h2
          | cons c cs =>
              simp [cmpListSend] at h1 h2 ⊢
              exact lex_lt_trans (f := cmpSend) (h_eq := fun {a b} => cmpSend_eq_iff a b) (h_lt := fun {a b c} => cmpSend_lt_trans a b c) (hD := ih bs cs) h1 h2
theorem cmpListReceive_lt_trans (l l' l'' : List Receive) : cmpListReceive l l' = Ordering.lt → cmpListReceive l' l'' = Ordering.lt → cmpListReceive l l'' = Ordering.lt := by
  induction l generalizing l' l'' with
  | nil => intro h1 h2; cases l' <;> cases l'' <;> simp [cmpListReceive] at h1 h2 ⊢
  | cons a as ih =>
      intro h1 h2
      cases l' with
      | nil => simp [cmpListReceive] at h1
      | cons b bs =>
          cases l'' with
          | nil => simp [cmpListReceive] at h2
          | cons c cs =>
              simp [cmpListReceive] at h1 h2 ⊢
              exact lex_lt_trans (f := cmpReceive) (h_eq := fun {a b} => cmpReceive_eq_iff a b) (h_lt := fun {a b c} => cmpReceive_lt_trans a b c) (hD := ih bs cs) h1 h2
theorem cmpListNew_lt_trans (l l' l'' : List New) : cmpListNew l l' = Ordering.lt → cmpListNew l' l'' = Ordering.lt → cmpListNew l l'' = Ordering.lt := by
  induction l generalizing l' l'' with
  | nil => intro h1 h2; cases l' <;> cases l'' <;> simp [cmpListNew] at h1 h2 ⊢
  | cons a as ih =>
      intro h1 h2
      cases l' with
      | nil => simp [cmpListNew] at h1
      | cons b bs =>
          cases l'' with
          | nil => simp [cmpListNew] at h2
          | cons c cs =>
              simp [cmpListNew] at h1 h2 ⊢
              exact lex_lt_trans (f := cmpNew) (h_eq := fun {a b} => cmpNew_eq_iff a b) (h_lt := fun {a b c} => cmpNew_lt_trans a b c) (hD := ih bs cs) h1 h2
theorem cmpListExpr_lt_trans (l l' l'' : List Expr) : cmpListExpr l l' = Ordering.lt → cmpListExpr l' l'' = Ordering.lt → cmpListExpr l l'' = Ordering.lt := by
  induction l generalizing l' l'' with
  | nil => intro h1 h2; cases l' <;> cases l'' <;> simp [cmpListExpr] at h1 h2 ⊢
  | cons a as ih =>
      intro h1 h2
      cases l' with
      | nil => simp [cmpListExpr] at h1
      | cons b bs =>
          cases l'' with
          | nil => simp [cmpListExpr] at h2
          | cons c cs =>
              simp [cmpListExpr] at h1 h2 ⊢
              exact lex_lt_trans (f := cmpExpr) (h_eq := fun {a b} => cmpExpr_eq_iff a b) (h_lt := fun {a b c} => cmpExpr_lt_trans a b c) (hD := ih bs cs) h1 h2
theorem cmpListMatch_lt_trans (l l' l'' : List Match) : cmpListMatch l l' = Ordering.lt → cmpListMatch l' l'' = Ordering.lt → cmpListMatch l l'' = Ordering.lt := by
  induction l generalizing l' l'' with
  | nil => intro h1 h2; cases l' <;> cases l'' <;> simp [cmpListMatch] at h1 h2 ⊢
  | cons a as ih =>
      intro h1 h2
      cases l' with
      | nil => simp [cmpListMatch] at h1
      | cons b bs =>
          cases l'' with
          | nil => simp [cmpListMatch] at h2
          | cons c cs =>
              simp [cmpListMatch] at h1 h2 ⊢
              exact lex_lt_trans (f := cmpMatch) (h_eq := fun {a b} => cmpMatch_eq_iff a b) (h_lt := fun {a b c} => cmpMatch_lt_trans a b c) (hD := ih bs cs) h1 h2
theorem cmpListGUnforgeable_lt_trans (l l' l'' : List GUnforgeable) : cmpListGUnforgeable l l' = Ordering.lt → cmpListGUnforgeable l' l'' = Ordering.lt → cmpListGUnforgeable l l'' = Ordering.lt := by
  induction l generalizing l' l'' with
  | nil => intro h1 h2; cases l' <;> cases l'' <;> simp [cmpListGUnforgeable] at h1 h2 ⊢
  | cons a as ih =>
      intro h1 h2
      cases l' with
      | nil => simp [cmpListGUnforgeable] at h1
      | cons b bs =>
          cases l'' with
          | nil => simp [cmpListGUnforgeable] at h2
          | cons c cs =>
              simp [cmpListGUnforgeable] at h1 h2 ⊢
              exact lex_lt_trans (f := cmpGUnforgeable) (h_eq := fun {a b} => cmpGUnforgeable_eq_iff a b) (h_lt := fun {a b c} => cmpGUnforgeable_lt_trans a b c) (hD := ih bs cs) h1 h2
theorem cmpListBundle_lt_trans (l l' l'' : List Bundle) : cmpListBundle l l' = Ordering.lt → cmpListBundle l' l'' = Ordering.lt → cmpListBundle l l'' = Ordering.lt := by
  induction l generalizing l' l'' with
  | nil => intro h1 h2; cases l' <;> cases l'' <;> simp [cmpListBundle] at h1 h2 ⊢
  | cons a as ih =>
      intro h1 h2
      cases l' with
      | nil => simp [cmpListBundle] at h1
      | cons b bs =>
          cases l'' with
          | nil => simp [cmpListBundle] at h2
          | cons c cs =>
              simp [cmpListBundle] at h1 h2 ⊢
              exact lex_lt_trans (f := cmpBundle) (h_eq := fun {a b} => cmpBundle_eq_iff a b) (h_lt := fun {a b c} => cmpBundle_lt_trans a b c) (hD := ih bs cs) h1 h2
theorem cmpListConnective_lt_trans (l l' l'' : List Connective) : cmpListConnective l l' = Ordering.lt → cmpListConnective l' l'' = Ordering.lt → cmpListConnective l l'' = Ordering.lt := by
  induction l generalizing l' l'' with
  | nil => intro h1 h2; cases l' <;> cases l'' <;> simp [cmpListConnective] at h1 h2 ⊢
  | cons a as ih =>
      intro h1 h2
      cases l' with
      | nil => simp [cmpListConnective] at h1
      | cons b bs =>
          cases l'' with
          | nil => simp [cmpListConnective] at h2
          | cons c cs =>
              simp [cmpListConnective] at h1 h2 ⊢
              exact lex_lt_trans (f := cmpConnective) (h_eq := fun {a b} => cmpConnective_eq_iff a b) (h_lt := fun {a b c} => cmpConnective_lt_trans a b c) (hD := ih bs cs) h1 h2
theorem cmpListPar_lt_trans (l l' l'' : List Par) : cmpListPar l l' = Ordering.lt → cmpListPar l' l'' = Ordering.lt → cmpListPar l l'' = Ordering.lt := by
  induction l generalizing l' l'' with
  | nil => intro h1 h2; cases l' <;> cases l'' <;> simp [cmpListPar] at h1 h2 ⊢
  | cons a as ih =>
      intro h1 h2
      cases l' with
      | nil => simp [cmpListPar] at h1
      | cons b bs =>
          cases l'' with
          | nil => simp [cmpListPar] at h2
          | cons c cs =>
              simp [cmpListPar] at h1 h2 ⊢
              exact lex_lt_trans (f := cmpPar) (h_eq := fun {a b} => cmpPar_eq_iff a b) (h_lt := fun {a b c} => cmpPar_lt_trans a b c) (hD := ih bs cs) h1 h2
theorem cmpListReceiveBind_lt_trans (l l' l'' : List ReceiveBind) : cmpListReceiveBind l l' = Ordering.lt → cmpListReceiveBind l' l'' = Ordering.lt → cmpListReceiveBind l l'' = Ordering.lt := by
  induction l generalizing l' l'' with
  | nil => intro h1 h2; cases l' <;> cases l'' <;> simp [cmpListReceiveBind] at h1 h2 ⊢
  | cons a as ih =>
      intro h1 h2
      cases l' with
      | nil => simp [cmpListReceiveBind] at h1
      | cons b bs =>
          cases l'' with
          | nil => simp [cmpListReceiveBind] at h2
          | cons c cs =>
              simp [cmpListReceiveBind] at h1 h2 ⊢
              exact lex_lt_trans (f := cmpReceiveBind) (h_eq := fun {a b} => cmpReceiveBind_eq_iff a b) (h_lt := fun {a b c} => cmpReceiveBind_lt_trans a b c) (hD := ih bs cs) h1 h2
theorem cmpListMatchCase_lt_trans (l l' l'' : List MatchCase) : cmpListMatchCase l l' = Ordering.lt → cmpListMatchCase l' l'' = Ordering.lt → cmpListMatchCase l l'' = Ordering.lt := by
  induction l generalizing l' l'' with
  | nil => intro h1 h2; cases l' <;> cases l'' <;> simp [cmpListMatchCase] at h1 h2 ⊢
  | cons a as ih =>
      intro h1 h2
      cases l' with
      | nil => simp [cmpListMatchCase] at h1
      | cons b bs =>
          cases l'' with
          | nil => simp [cmpListMatchCase] at h2
          | cons c cs =>
              simp [cmpListMatchCase] at h1 h2 ⊢
              exact lex_lt_trans (f := cmpMatchCase) (h_eq := fun {a b} => cmpMatchCase_eq_iff a b) (h_lt := fun {a b c} => cmpMatchCase_lt_trans a b c) (hD := ih bs cs) h1 h2
private theorem cmpParPair_eq_iff (x y : Par × Par) :
    lex (cmpPar x.1 y.1) (cmpPar x.2 y.2) = Ordering.eq ↔ x = y := by
  rw [lex_eq_iff, cmpPar_eq_iff, cmpPar_eq_iff, Prod.ext_iff]

private theorem cmpParPair_lt_trans (x y z : Par × Par) :
    lex (cmpPar x.1 y.1) (cmpPar x.2 y.2) = Ordering.lt →
    lex (cmpPar y.1 z.1) (cmpPar y.2 z.2) = Ordering.lt →
    lex (cmpPar x.1 z.1) (cmpPar x.2 z.2) = Ordering.lt := by
  intro h1 h2
  exact lex_lt_trans (f := cmpPar) (h_eq := fun {a b} => cmpPar_eq_iff a b) (h_lt := fun {a b c} => cmpPar_lt_trans a b c)
    (hD := cmpPar_lt_trans x.2 y.2 z.2) h1 h2
theorem cmpListParPair_lt_trans (l l' l'' : List (Par × Par)) : cmpListParPair l l' = Ordering.lt → cmpListParPair l' l'' = Ordering.lt → cmpListParPair l l'' = Ordering.lt := by
  induction l generalizing l' l'' with
  | nil => intro h1 h2; cases l' <;> cases l'' <;> simp [cmpListParPair] at h1 h2 ⊢
  | cons a as ih =>
      intro h1 h2
      cases l' with
      | nil => simp [cmpListParPair] at h1
      | cons b bs =>
          cases l'' with
          | nil => simp [cmpListParPair] at h2
          | cons c cs =>
              cases a with | mk a1 a2 =>
              cases b with | mk b1 b2 =>
              cases c with | mk c1 c2 =>
              simp [cmpListParPair] at h1 h2 ⊢
              exact lex_lt_trans (f := fun x y => lex (cmpPar x.1 y.1) (cmpPar x.2 y.2))
                (h_eq := fun {a b} => cmpParPair_eq_iff a b) (h_lt := fun {a b c} => cmpParPair_lt_trans a b c)
                (a := (a1, a2)) (b := (b1, b2)) (c := (c1, c2)) (x := as) (y := bs) (z := cs)
                (hD := ih bs cs) h1 h2

/-! ## The `Comparator` instances for the 11 element types -/

def parComparator : Comparator Par where
  cmp := cmpPar
  eq_iff := by intro a b; exact cmpPar_eq_iff a b
  swap := by intro a b; exact cmpPar_swap a b
  lt_trans := by intro a b c; exact cmpPar_lt_trans a b c

def sendComparator : Comparator Send where
  cmp := cmpSend
  eq_iff := by intro a b; exact cmpSend_eq_iff a b
  swap := by intro a b; exact cmpSend_swap a b
  lt_trans := by intro a b c; exact cmpSend_lt_trans a b c

def receiveBindComparator : Comparator ReceiveBind where
  cmp := cmpReceiveBind
  eq_iff := by intro a b; exact cmpReceiveBind_eq_iff a b
  swap := by intro a b; exact cmpReceiveBind_swap a b
  lt_trans := by intro a b c; exact cmpReceiveBind_lt_trans a b c

def receiveComparator : Comparator Receive where
  cmp := cmpReceive
  eq_iff := by intro a b; exact cmpReceive_eq_iff a b
  swap := by intro a b; exact cmpReceive_swap a b
  lt_trans := by intro a b c; exact cmpReceive_lt_trans a b c

def newComparator : Comparator New where
  cmp := cmpNew
  eq_iff := by intro a b; exact cmpNew_eq_iff a b
  swap := by intro a b; exact cmpNew_swap a b
  lt_trans := by intro a b c; exact cmpNew_lt_trans a b c

def matchCaseComparator : Comparator MatchCase where
  cmp := cmpMatchCase
  eq_iff := by intro a b; exact cmpMatchCase_eq_iff a b
  swap := by intro a b; exact cmpMatchCase_swap a b
  lt_trans := by intro a b c; exact cmpMatchCase_lt_trans a b c

def matchComparator : Comparator Match where
  cmp := cmpMatch
  eq_iff := by intro a b; exact cmpMatch_eq_iff a b
  swap := by intro a b; exact cmpMatch_swap a b
  lt_trans := by intro a b c; exact cmpMatch_lt_trans a b c

def exprComparator : Comparator Expr where
  cmp := cmpExpr
  eq_iff := by intro a b; exact cmpExpr_eq_iff a b
  swap := by intro a b; exact cmpExpr_swap a b
  lt_trans := by intro a b c; exact cmpExpr_lt_trans a b c

def bundleComparator : Comparator Bundle where
  cmp := cmpBundle
  eq_iff := by intro a b; exact cmpBundle_eq_iff a b
  swap := by intro a b; exact cmpBundle_swap a b
  lt_trans := by intro a b c; exact cmpBundle_lt_trans a b c

def gUnforgeableComparator : Comparator GUnforgeable where
  cmp := cmpGUnforgeable
  eq_iff := by intro a b; exact cmpGUnforgeable_eq_iff a b
  swap := by intro a b; exact cmpGUnforgeable_swap a b
  lt_trans := by intro a b c; exact cmpGUnforgeable_lt_trans a b c

def connectiveComparator : Comparator Connective where
  cmp := cmpConnective
  eq_iff := by intro a b; exact cmpConnective_eq_iff a b
  swap := by intro a b; exact cmpConnective_swap a b
  lt_trans := by intro a b c; exact cmpConnective_lt_trans a b c

/-! ## Canonical `sort` -/

/-- Sort a list field after `map f`, when `f` is idempotent on `l`. -/
theorem sortList_field_idem {α : Type} (C : Comparator α) (f : α → α)
    (l : List α) (hf : ∀ x, x ∈ l → f (f x) = f x) :
    sortList C ((sortList C (l.map f)).map f) = sortList C (l.map f) := by
  have hmap : (sortList C (l.map f)).map f = sortList C (l.map f) := by
    unfold sortList
    conv_rhs => rw [← List.map_id (List.insertionSort C.le (l.map f))]
    apply List.map_congr_left
    intro x hx
    rw [List.mem_insertionSort C.le] at hx
    rcases List.mem_map.mp hx with ⟨y, hy, rfl⟩
    exact hf y hy
  rw [hmap]
  exact sortList_idempotent C (l.map f)

mutual
  def sortPar : Par → Par
    | Par.mk s r n e m u b c =>
        Par.mk (sortList sendComparator (sortListSend s))
               (sortList receiveComparator (sortListReceive r))
               (sortList newComparator (sortListNew n))
               (sortList exprComparator (sortListExpr e))
               (sortList matchComparator (sortListMatch m))
               (sortList gUnforgeableComparator (sortListGUnforgeable u))
               (sortList bundleComparator (sortListBundle b))
               (sortList connectiveComparator (sortListConnective c))
  termination_by x => sizeOf x

  def sortSend : Send → Send
    | Send.mk c d p => Send.mk (sortPar c) (sortList parComparator (sortListPar d)) p
  termination_by x => sizeOf x

  def sortReceiveBind : ReceiveBind → ReceiveBind
    | ReceiveBind.mk ps s n => ReceiveBind.mk (sortList parComparator (sortListPar ps)) (sortPar s) n
  termination_by x => sizeOf x

  def sortReceive : Receive → Receive
    | Receive.mk bs b p n => Receive.mk (sortList receiveBindComparator (sortListReceiveBind bs)) (sortPar b) p n
  termination_by x => sizeOf x

  def sortNew : New → New
    | New.mk n b => New.mk n (sortPar b)
  termination_by x => sizeOf x

  def sortMatchCase : MatchCase → MatchCase
    | MatchCase.mk p s n => MatchCase.mk (sortPar p) (sortPar s) n
  termination_by x => sizeOf x

  def sortMatch : Match → Match
    | Match.mk t cs => Match.mk (sortPar t) (sortList matchCaseComparator (sortListMatchCase cs))
  termination_by x => sizeOf x

  def sortExpr : Expr → Expr
    | Expr.ground g => Expr.ground g
    | Expr.evar v => Expr.evar v
    | Expr.eneg p => Expr.eneg (sortPar p)
    | Expr.enot p => Expr.enot (sortPar p)
    | Expr.eplus p q => Expr.eplus (sortPar p) (sortPar q)
    | Expr.eminus p q => Expr.eminus (sortPar p) (sortPar q)
    | Expr.emult p q => Expr.emult (sortPar p) (sortPar q)
    | Expr.ediv p q => Expr.ediv (sortPar p) (sortPar q)
    | Expr.emod p q => Expr.emod (sortPar p) (sortPar q)
    | Expr.elt p q => Expr.elt (sortPar p) (sortPar q)
    | Expr.ele p q => Expr.ele (sortPar p) (sortPar q)
    | Expr.egt p q => Expr.egt (sortPar p) (sortPar q)
    | Expr.ege p q => Expr.ege (sortPar p) (sortPar q)
    | Expr.eeq p q => Expr.eeq (sortPar p) (sortPar q)
    | Expr.eneq p q => Expr.eneq (sortPar p) (sortPar q)
    | Expr.eand p q => Expr.eand (sortPar p) (sortPar q)
    | Expr.eor p q => Expr.eor (sortPar p) (sortPar q)
    | Expr.elist ps r => Expr.elist (sortList parComparator (sortListPar ps)) r
    | Expr.etuple ps => Expr.etuple (sortList parComparator (sortListPar ps))
    | Expr.eset ps r => Expr.eset (sortList parComparator (sortListPar ps)) r
    | Expr.emap kvs r => Expr.emap (sortList (cmpPair parComparator parComparator) (sortListParPair kvs)) r
  termination_by x => sizeOf x

  def sortBundle : Bundle → Bundle
    | Bundle.mk b w r => Bundle.mk (sortPar b) w r
  termination_by x => sizeOf x

  def sortGUnforgeable : GUnforgeable → GUnforgeable
    | g => g
  termination_by x => sizeOf x

  def sortConnective : Connective → Connective
    | Connective.connAnd ps => Connective.connAnd (sortList parComparator (sortListPar ps))
    | Connective.connOr ps => Connective.connOr (sortList parComparator (sortListPar ps))
    | Connective.connNot p => Connective.connNot (sortPar p)
    | Connective.connVarRef d n => Connective.connVarRef d n
  termination_by x => sizeOf x

  def sortParPair : Par × Par → Par × Par
    | (a, b) => (sortPar a, sortPar b)
  termination_by x => sizeOf x

  def sortListSend : List Send → List Send
    | [] => []
    | a :: as => sortSend a :: sortListSend as
  termination_by x => sizeOf x

  def sortListReceive : List Receive → List Receive
    | [] => []
    | a :: as => sortReceive a :: sortListReceive as
  termination_by x => sizeOf x

  def sortListNew : List New → List New
    | [] => []
    | a :: as => sortNew a :: sortListNew as
  termination_by x => sizeOf x

  def sortListExpr : List Expr → List Expr
    | [] => []
    | a :: as => sortExpr a :: sortListExpr as
  termination_by x => sizeOf x

  def sortListMatch : List Match → List Match
    | [] => []
    | a :: as => sortMatch a :: sortListMatch as
  termination_by x => sizeOf x

  def sortListGUnforgeable : List GUnforgeable → List GUnforgeable
    | [] => []
    | a :: as => sortGUnforgeable a :: sortListGUnforgeable as
  termination_by x => sizeOf x

  def sortListBundle : List Bundle → List Bundle
    | [] => []
    | a :: as => sortBundle a :: sortListBundle as
  termination_by x => sizeOf x

  def sortListConnective : List Connective → List Connective
    | [] => []
    | a :: as => sortConnective a :: sortListConnective as
  termination_by x => sizeOf x

  def sortListPar : List Par → List Par
    | [] => []
    | a :: as => sortPar a :: sortListPar as
  termination_by x => sizeOf x

  def sortListReceiveBind : List ReceiveBind → List ReceiveBind
    | [] => []
    | a :: as => sortReceiveBind a :: sortListReceiveBind as
  termination_by x => sizeOf x

  def sortListMatchCase : List MatchCase → List MatchCase
    | [] => []
    | a :: as => sortMatchCase a :: sortListMatchCase as
  termination_by x => sizeOf x

  def sortListParPair : List (Par × Par) → List (Par × Par)
    | [] => []
    | x :: xs => sortParPair x :: sortListParPair xs
  termination_by x => sizeOf x
end

/-! ## `sortListX` = `·.map sortX` -/

@[simp] theorem sortListSend_eq_map (l : List Send) : sortListSend l = l.map sortSend := by
  induction l <;> simp [sortListSend, *]
@[simp] theorem sortListReceive_eq_map (l : List Receive) : sortListReceive l = l.map sortReceive := by
  induction l <;> simp [sortListReceive, *]
@[simp] theorem sortListNew_eq_map (l : List New) : sortListNew l = l.map sortNew := by
  induction l <;> simp [sortListNew, *]
@[simp] theorem sortListExpr_eq_map (l : List Expr) : sortListExpr l = l.map sortExpr := by
  induction l <;> simp [sortListExpr, *]
@[simp] theorem sortListMatch_eq_map (l : List Match) : sortListMatch l = l.map sortMatch := by
  induction l <;> simp [sortListMatch, *]
@[simp] theorem sortListGUnforgeable_eq_map (l : List GUnforgeable) : sortListGUnforgeable l = l.map sortGUnforgeable := by
  induction l <;> simp [sortListGUnforgeable, *]
@[simp] theorem sortListBundle_eq_map (l : List Bundle) : sortListBundle l = l.map sortBundle := by
  induction l <;> simp [sortListBundle, *]
@[simp] theorem sortListConnective_eq_map (l : List Connective) : sortListConnective l = l.map sortConnective := by
  induction l <;> simp [sortListConnective, *]
@[simp] theorem sortListPar_eq_map (l : List Par) : sortListPar l = l.map sortPar := by
  induction l <;> simp [sortListPar, *]
@[simp] theorem sortListReceiveBind_eq_map (l : List ReceiveBind) : sortListReceiveBind l = l.map sortReceiveBind := by
  induction l <;> simp [sortListReceiveBind, *]
@[simp] theorem sortListMatchCase_eq_map (l : List MatchCase) : sortListMatchCase l = l.map sortMatchCase := by
  induction l <;> simp [sortListMatchCase, *]
@[simp] theorem sortListParPair_eq_map (l : List (Par × Par)) : sortListParPair l = l.map sortParPair := by
  induction l <;> simp [sortListParPair, *]

/-! ## Law 1 (canonicalization) — idempotence -/

mutual
  theorem sortPar_idempotent : ∀ (p : Par), sortPar (sortPar p) = sortPar p
    | Par.mk s r n e m u b c => by
        simp [sortPar]
        exact ⟨sortList_field_idem sendComparator sortSend s (fun x _ => sortSend_idempotent x),
          sortList_field_idem receiveComparator sortReceive r (fun x _ => sortReceive_idempotent x),
          sortList_field_idem newComparator sortNew n (fun x _ => sortNew_idempotent x),
          sortList_field_idem exprComparator sortExpr e (fun x _ => sortExpr_idempotent x),
          sortList_field_idem matchComparator sortMatch m (fun x _ => sortMatch_idempotent x),
          sortList_field_idem gUnforgeableComparator sortGUnforgeable u (fun x _ => sortGUnforgeable_idempotent x),
          sortList_field_idem bundleComparator sortBundle b (fun x _ => sortBundle_idempotent x),
          sortList_field_idem connectiveComparator sortConnective c (fun x _ => sortConnective_idempotent x)⟩
  termination_by p => sizeOf p

  theorem sortSend_idempotent : ∀ (x : Send), sortSend (sortSend x) = sortSend x
    | Send.mk c d p => by
        have hc := sortPar_idempotent c
        have hd : ∀ x, x ∈ d → sortPar (sortPar x) = sortPar x := fun x _ => sortPar_idempotent x
        simp [sortSend, hc]
        exact sortList_field_idem parComparator sortPar d hd
  termination_by x => sizeOf x

  theorem sortReceiveBind_idempotent : ∀ (x : ReceiveBind), sortReceiveBind (sortReceiveBind x) = sortReceiveBind x
    | ReceiveBind.mk ps s n => by
        have hps : ∀ x, x ∈ ps → sortPar (sortPar x) = sortPar x := fun x _ => sortPar_idempotent x
        have hs := sortPar_idempotent s
        simp [sortReceiveBind, hs]
        exact sortList_field_idem parComparator sortPar ps hps
  termination_by x => sizeOf x

  theorem sortReceive_idempotent : ∀ (x : Receive), sortReceive (sortReceive x) = sortReceive x
    | Receive.mk bs b p n => by
        have hbs : ∀ x, x ∈ bs → sortReceiveBind (sortReceiveBind x) = sortReceiveBind x := fun x _ => sortReceiveBind_idempotent x
        have hb := sortPar_idempotent b
        simp [sortReceive, hb]
        exact sortList_field_idem receiveBindComparator sortReceiveBind bs hbs
  termination_by x => sizeOf x

  theorem sortNew_idempotent : ∀ (x : New), sortNew (sortNew x) = sortNew x
    | New.mk n b => by
        have hb := sortPar_idempotent b
        simp [sortNew, hb]
  termination_by x => sizeOf x

  theorem sortMatchCase_idempotent : ∀ (x : MatchCase), sortMatchCase (sortMatchCase x) = sortMatchCase x
    | MatchCase.mk p s n => by
        have hp := sortPar_idempotent p
        have hs := sortPar_idempotent s
        simp [sortMatchCase, hp, hs]
  termination_by x => sizeOf x

  theorem sortMatch_idempotent : ∀ (x : Match), sortMatch (sortMatch x) = sortMatch x
    | Match.mk t cs => by
        have ht := sortPar_idempotent t
        have hcs : ∀ x, x ∈ cs → sortMatchCase (sortMatchCase x) = sortMatchCase x := fun x _ => sortMatchCase_idempotent x
        simp [sortMatch, ht]
        exact sortList_field_idem matchCaseComparator sortMatchCase cs hcs
  termination_by x => sizeOf x

  theorem sortExpr_idempotent : ∀ (x : Expr), sortExpr (sortExpr x) = sortExpr x
    | Expr.ground g => by simp [sortExpr]
    | Expr.evar v => by simp [sortExpr]
    | Expr.eneg p | Expr.enot p => by
        have hp := sortPar_idempotent p
        simp [sortExpr, hp]
    | Expr.eplus p q | Expr.eminus p q | Expr.emult p q | Expr.ediv p q | Expr.emod p q |
      Expr.elt p q | Expr.ele p q | Expr.egt p q | Expr.ege p q | Expr.eeq p q |
      Expr.eneq p q | Expr.eand p q | Expr.eor p q => by
        have hp := sortPar_idempotent p
        have hq := sortPar_idempotent q
        simp [sortExpr, hp, hq]
    | Expr.elist ps r | Expr.eset ps r => by
        have hps : ∀ x, x ∈ ps → sortPar (sortPar x) = sortPar x := fun x _ => sortPar_idempotent x
        simp [sortExpr]
        exact sortList_field_idem parComparator sortPar ps hps
    | Expr.etuple ps => by
        have hps : ∀ x, x ∈ ps → sortPar (sortPar x) = sortPar x := fun x _ => sortPar_idempotent x
        simp [sortExpr]
        exact sortList_field_idem parComparator sortPar ps hps
    | Expr.emap kvs r => by
        have hkvs : ∀ x, x ∈ kvs → sortParPair (sortParPair x) = sortParPair x := fun x _ => sortParPair_idempotent x
        simp [sortExpr]
        exact sortList_field_idem (cmpPair parComparator parComparator) sortParPair kvs hkvs
  termination_by x => sizeOf x

  theorem sortBundle_idempotent : ∀ (x : Bundle), sortBundle (sortBundle x) = sortBundle x
    | Bundle.mk b w r => by
        have hb := sortPar_idempotent b
        simp [sortBundle, hb]
  termination_by x => sizeOf x

  theorem sortGUnforgeable_idempotent : ∀ (x : GUnforgeable), sortGUnforgeable (sortGUnforgeable x) = sortGUnforgeable x
    | g => rfl
  termination_by x => sizeOf x

  theorem sortConnective_idempotent : ∀ (x : Connective), sortConnective (sortConnective x) = sortConnective x
    | Connective.connAnd ps | Connective.connOr ps => by
        have hps : ∀ x, x ∈ ps → sortPar (sortPar x) = sortPar x := fun x _ => sortPar_idempotent x
        simp [sortConnective]
        exact sortList_field_idem parComparator sortPar ps hps
    | Connective.connNot p => by
        have hp := sortPar_idempotent p
        simp [sortConnective, hp]
    | Connective.connVarRef d n => by simp [sortConnective]
  termination_by x => sizeOf x

  theorem sortParPair_idempotent : ∀ (x : Par × Par), sortParPair (sortParPair x) = sortParPair x
    | (a, b) => by
        have ha := sortPar_idempotent a
        have hb := sortPar_idempotent b
        simp [sortParPair, ha, hb]
  termination_by x => sizeOf x
end

/-! ## Law 1 (canonicalization) — commutativity -/

theorem sortPar_comm (p q : Par) : sortPar (parMerge p q) = sortPar (parMerge q p) := by
  simp [parMerge, sortPar, sortList_append_comm]


end Rchain
