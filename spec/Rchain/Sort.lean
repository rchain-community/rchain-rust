import Rchain.Par
import Rchain.Cmp

-- The budget is **measured**, not inherited: on this file's `eq_iff`/`swap` blocks a budget of
-- 1,000,000 heartbeats fails with 80 declarations over it, and 2,000,000 passes. This is 4,000,000 —
-- 2x the measured floor, and 25x below the 100,000,000 that stood here, which was a rumour nobody had
-- measured and which would have hidden a great deal. Heartbeat counts are deterministic for a given
-- Lean version and set of options, so this number is portable; the *recursion depth* limits this file's
-- docstrings record are not, which is why those remain described as attempts rather than settings.
set_option maxHeartbeats 4000000

/-!
# Canonicalization (Law 1) over the flat `Par`

Mirrors `models/src/main/scala/coop/rchain/models/rholang/sorter/ScoreTree.scala` and
`ordering.scala`: `Par` (and `ESet`/`EMap`) are canonicalized to a total order so that structural
equality of the sorted form *is* process equality up to α.

The total order is a hand-rolled structural `cmpPar`, lexicographic via `lex`, in the node's
**score-tag** order — the order `models/src/sorter.rs`'s `node_score(tag, children)` trees sort by,
which is *not* the port's field or declaration order (the `sort` corpus found four places where the
two disagreed; they are aligned and pinned, and the note above the comparator family lists them and
the boundary that remains). It is defined by **direct mutual recursion** (each list field has its own
comparator, so the recursion is fully first-order), which Lean accepts with a `sizeOf` measure.

Law 1 is `sortPar (sortPar p) = sortPar p` (idempotence) and
`sortPar (parMerge p q) = sortPar (parMerge q p)` (commutativity), proven from the total-order
bundle on each comparator via the generic `sortList` canonicality in `Rchain.Cmp`.
-/

namespace Rchain

open Comparator

/-! ## Leaf comparators (`Ground`, `Var`) -/

/-- `Bool`, compared the way the node's score tree compares it: `sort_gbool` (`models/src/sorter.rs`)
scores `true` as `0` and `false` as `1`, so **`true` sorts before `false`** — the reverse of
`linearOrderComparator Bool`. Pinned by the `sort` corpus (`@"c"!(false)` vs `@"c"!(true)` → `gt`,
`spec/conformance/sort.tsv`). -/
def cmpBool : Bool → Bool → Ordering
  | true, true => .eq
  | true, false => .lt
  | false, true => .gt
  | false, false => .eq

/-- `cmpBool` is a lawful comparator. -/
def boolComparator : Comparator Bool where
  cmp := cmpBool
  eq_iff := by intro a b; cases a <;> cases b <;> decide
  swap := by intro a b; cases a <;> cases b <;> rfl
  lt_trans := by
    intro a b c h1 h2
    cases a <;> cases b <;> cases c <;> simp_all [cmpBool]

/-! These three are deliberately **not** `@[simp]`: a `swap` lemma with both arguments as variables
    (`cmpBool b a = (cmpBool a b).swap`) makes `simp` loop when it unfolds `cmpGround` — tried, and
    `simp` hit `maxRecDepth`. The `eq_iff`/`swap` lemmas for `Ground`/`Var` above are `@[simp]` for
    the same reason only because they are declared *after* the instance whose proof would loop on
    them. -/
theorem cmpBool_eq_iff (a b : Bool) : cmpBool a b = Ordering.eq ↔ a = b := boolComparator.eq_iff
theorem cmpBool_swap (a b : Bool) : cmpBool b a = Ordering.swap (cmpBool a b) := boolComparator.swap
theorem cmpBool_lt_trans (a b c : Bool) (h1 : cmpBool a b = Ordering.lt) (h2 : cmpBool b c = Ordering.lt) :
    cmpBool a c = Ordering.lt := boolComparator.lt_trans h1 h2

/-- Structural comparison of `Ground`, in the node's **score-tag** order (`bool < int < str < uri`,
then `bytes` — see the tag caveat below). Every constructor but the last needs the `.lt`/`.gt` pair
after its own arm: the `.lt` arm answers "this constructor against anything later", the `.gt` arm
"anything earlier against this one", and both are reached only when no earlier group already decided
the pair.

**`bytes` is the one group whose position this model does not get right.** The node has no `GBytes`
ground: a byte array is `Expr::GByteArray` with tag `116` (`models/src/sorter.rs`), which sorts
*after* every var and every arithmetic/comparison/logical operator (`EVAR=100 … ENOT=112, EAND=113,
EOR=114`) and *before* `EMATCHES=118` and `EMOD=122` — while here `Ground.bytes` sits at the end of
the ground block, before `elist`(6) and hence before every operator. Aligning it is not a reorder but
a split of the `Expr.ground` group in `cmpExpr`, so it is recorded in the boundary list in the note
above the comparator family rather than silently left. -/
def cmpGround : Ground → Ground → Ordering
  | .bool b, .bool b' => cmpBool b b'
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
    case bool.bool x y => exact cmpBool_eq_iff x y
    all_goals first
      | exact cmp_eq_eq_iff
      | exact (linearOrderComparator Int).eq_iff
      | exact (listComparator (linearOrderComparator Nat)).eq_iff
  swap := by
    intro a b
    cases a <;> cases b <;> simp [cmpGround]
    case bool.bool x y => exact cmpBool_swap x y
    all_goals first
      | rfl
      | exact (linearOrderComparator Int).swap
      | exact (listComparator (linearOrderComparator Nat)).swap
  lt_trans := by
    intro a b c h1 h2
    cases a <;> cases b <;> cases c <;> simp [cmpGround] at h1 h2 ⊢
    case bool.bool.bool x y z => exact cmpBool_lt_trans x y z h1 h2
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
        lex (cmpListSend s s') (lex (cmpListReceive r r') (lex (cmpListExpr e e')
        (lex (cmpListNew n n') (lex (cmpListMatch m m') (lex (cmpListBundle b b')
        (lex (cmpListConnective c c') (cmpListGUnforgeable u u')))))))
  termination_by p q => sizeOf p + sizeOf q
  def cmpSend : Send → Send → Ordering
    | Send.mk c d p, Send.mk c' d' p' => lex (_root_.cmp p p') (lex (cmpPar c c') (cmpListPar d d'))
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
  /-- `Expr`, in the node's **score-tag** order (`models/src/sorter.rs`): the four grounds (`bool 1`,
      `int 2`, `str 3`, `uri 4`), then the collections `elist 6 < etuple 7 < eset 8 < emap 9`, then
      the vars and the operators `evar 100 < eneg 101 < emult 102 < ediv 103 < eplus 104 <
      eminus 105 < elt 106 < ele 107 < egt 108 < ege 109 < eeq 110 < eneq 111 < enot 112 <
      eand 113 < eor 114 < emod 122`. (`Ground.bytes` is the exception — see `cmpGround`.) The arms
      are in that order because the `.lt`/`.gt` fallback of a group is only reached when no earlier
      group matched, which is what makes the arm order *be* the group order. -/
  def cmpExpr : Expr → Expr → Ordering
    | Expr.ground g, Expr.ground g' => cmpGround g g'
    | Expr.ground _, _ => .lt | _, Expr.ground _ => .gt
    | Expr.elist ps r, Expr.elist ps' r' => lex (cmpListPar ps ps') (cmpOptionVar r r')
    | Expr.elist _ _, _ => .lt | _, Expr.elist _ _ => .gt
    | Expr.etuple ps, Expr.etuple ps' => cmpListPar ps ps'
    | Expr.etuple _, _ => .lt | _, Expr.etuple _ => .gt
    | Expr.eset ps r, Expr.eset ps' r' => lex (cmpListPar ps ps') (cmpOptionVar r r')
    | Expr.eset _ _, _ => .lt | _, Expr.eset _ _ => .gt
    | Expr.emap kvs r, Expr.emap kvs' r' => lex (cmpListParPair kvs kvs') (cmpOptionVar r r')
    | Expr.emap _ _, _ => .lt | _, Expr.emap _ _ => .gt
    | Expr.evar v, Expr.evar v' => cmpVar v v'
    | Expr.evar _, _ => .lt | _, Expr.evar _ => .gt
    | Expr.eneg p, Expr.eneg p' => cmpPar p p'
    | Expr.eneg _, _ => .lt | _, Expr.eneg _ => .gt
    | Expr.emult p q, Expr.emult p' q' => lex (cmpPar p p') (cmpPar q q')
    | Expr.emult _ _, _ => .lt | _, Expr.emult _ _ => .gt
    | Expr.ediv p q, Expr.ediv p' q' => lex (cmpPar p p') (cmpPar q q')
    | Expr.ediv _ _, _ => .lt | _, Expr.ediv _ _ => .gt
    | Expr.eplus p q, Expr.eplus p' q' => lex (cmpPar p p') (cmpPar q q')
    | Expr.eplus _ _, _ => .lt | _, Expr.eplus _ _ => .gt
    | Expr.eminus p q, Expr.eminus p' q' => lex (cmpPar p p') (cmpPar q q')
    | Expr.eminus _ _, _ => .lt | _, Expr.eminus _ _ => .gt
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
    | Expr.enot p, Expr.enot p' => cmpPar p p'
    | Expr.enot _, _ => .lt | _, Expr.enot _ => .gt
    | Expr.eand p q, Expr.eand p' q' => lex (cmpPar p p') (cmpPar q q')
    | Expr.eand _ _, _ => .lt | _, Expr.eand _ _ => .gt
    | Expr.eor p q, Expr.eor p' q' => lex (cmpPar p p') (cmpPar q q')
    | Expr.eor _ _, _ => .lt | _, Expr.eor _ _ => .gt
    | Expr.emod p q, Expr.emod p' q' => lex (cmpPar p p') (cmpPar q q')
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
        -- `tauto` at the end because the comparator's field order is the node's (`persistent` first
        -- for a `Send`, `exprs` before `news` … for a `Par`), while `Par.mk`/`Send.mk` equality
        -- decomposes in *declaration* order: the conjunction needs reordering, which `simp` will not
        -- do on its own.
        simp [cmpPar, lex_eq_iff, hs, hr, hn, he, hm, hu, hb, hc] <;> tauto
  termination_by p => sizeOf p

  theorem cmpSend_eq_iff : ∀ s : Send, ∀ t : Send, cmpSend s t = Ordering.eq ↔ s = t
    | Send.mk c d p, Send.mk c' d' p' => by
        have hc := cmpPar_eq_iff c c'
        have hd := cmpListPar_eq_iff d d'
        simp [cmpSend, lex_eq_iff, hc, hd, cmp_eq_eq_iff] <;> tauto
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

/-! ## Lawfulness: `lt_trans` (the residual axioms are counted by the register, not here)

The **list** comparators' `eq_iff`/`swap`/`lt_trans` laws are now **discharged** (direct induction
on the list, composing the element law with `lex_eq_iff`/`swap_lex`/`lex_lt_trans`); they were never
the hard part. The element comparators' `lt_trans` laws (`cmpPar`/`cmpSend`/…/`cmpConnective`;
`cmpGUnforgeable_lt_trans` is proved) need mutual induction over the AST, which is why some of them are
still axioms — and *which* ones, and how many, is `Rchain/Laws.lean`'s law 1b row to say.

**This header has twice carried a wrong number** (33, then 12, when the tree held 69 at one point and
the register counts a different set). It carries none now, deliberately: the register's axiom-accounting
check compares the axioms its rows cite against the elaborated environment, so a number written *there*
cannot drift without the build failing, and a number written *here* can. `tools/emit-lean-counts.sh`
holds the reader-facing documents to the same rule.

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

### The route, worked out (2026-09-23, after the cross-shard falsification sweep)

**Why the ten are genuinely mutual** (so the next attempt does not re-derive this): the **list** lemmas
above are proved *conditional on the element axioms* — `cmpListSend_lt_trans`'s body is literally
`lex_lt_trans … (h_lt := fun {a b c} => cmpSend_lt_trans a b c)` — while the element lemmas need the
list lemmas, because `cmpExpr`'s twenty constructors compare `cmpPar` and `cmpPar` compares `List Expr`.
So the cycle is real and runs through `cmpExpr` ↔ `cmpListExpr` ↔ `cmpPar`, not merely through the
definitions. That is why reordering cannot help and why a `mutual` block was tried.

**What the refactor should be — not a sum type.** `Rchain/Cmp.lean` already carries the algebra:
`Comparator` is a structure whose **fields are the laws** (`eq_iff`, `swap`, `lt_trans`), and it supplies
`listComparator : Comparator α → Comparator (List α)` and `cmpPair : Comparator α → Comparator β →
Comparator (α × β)`, both **built from their components' laws** — so a comparator *composed* from them
has `eq_iff`/`swap`/`lt_trans` **by construction**, and `cmpListF_lt_trans`/`lex_lt_trans` are the lemmas
to chain. Define the family by that composition (the hand-rolled mutual recursion was never needed for
the *semantics*), and all twelve axioms — the ten `lt_trans` plus `cmpExpr`'s `eq_iff`/`swap` — become
the `lt_trans`/`eq_iff`/`swap` *fields* of the composed comparators. Two things to preserve while doing
it: the corpus checkers (`matchCases_decide`, `flags.tsv`, `c21.tsv`) `decide` these functions, so the
composed definitions must stay `def`-unfoldable at concrete arguments; and `cmpExpr`'s twenty
constructors are where `Json.lean`'s **`rfl`-proved `@[simp]` arm lemmas** are the precedent to copy.

That is a multi-unit job (the family's definitions, the `sortX_*` lemmas that mention them, the corpus
re-emission) and the register will not let it land half-done: the axiom-accounting check fails if a row
cites an axiom that is gone, so each replacement and its rows move together.

### The idiom, validated on `cmpSend` (2026-09-23, later the same day)

The *shape* of the proof is no longer speculative, so what is settled is recorded here — the next attempt
should start from this code rather than from a description. First: the old obstruction ("a `mutual`
theorem block over the **two-argument** `cmpX` family hangs the termination checker") is a *measure*
problem, not a proof-shape problem. With one argument named in the `∀` — `∀ p : Par, ∀ q r : Par, …` —
and `termination_by p _ _ => sizeOf p`, the measure descends structurally, exactly as the `eq_iff` and
`swap` blocks below already do. Those blocks are the template, and they carry **both** the element and
the list laws, which is what the cycle requires (the eleven standalone list lemmas at the end of this
section are the roll-up of the same proofs from the current axioms).

Second, the part that had no recorded answer: a `lex` chain of eight components (as `cmpPar` is) cannot
be handed to `lex_lt_trans` component by component, because that lemma's tail argument must be a
**single** comparison function — the tail has to be packaged as a comparator on a product, which is what
`Cmp.lean`'s `cmpPairF` is for. Verified end to end on `cmpSend` (three levels: `Bool`, then
`Par × List Par`), with the axioms standing in for the induction hypotheses:

```lean
simp only [cmpSend] at h1 h2 ⊢
refine lex_lt_trans (f := (linearOrderComparator Bool).cmp)
  (h_eq := fun {a b} => (linearOrderComparator Bool).eq_iff (a := a) (b := b))
  (h_lt := fun {a b c} h1 h2 => (linearOrderComparator Bool).lt_trans h1 h2)
  (Dcmp := cmpPairF cmpPar cmpListPar) (x := (c, d)) (y := (c', d')) (z := (c'', d''))
  (hD := ?_) h1 h2
intro hx hy
exact lex_lt_trans (f := cmpPar) (h_eq := fun {a b} => cmpPar_eq_iff a b)
  (h_lt := fun {a b c} h1 h2 => cmpPar_lt_trans a b c h1 h2) (Dcmp := cmpListPar)
  (x := d) (y := d') (z := d'') (hD := fun h1 h2 => cmpListPar_lt_trans d d' d'' h1 h2) hx hy
```

Two details that cost time, so they are written down: `x`/`y`/`z` must be given **explicitly as the
packaged tuples**, because `cmpPairF` cannot reduce against a metavariable of product type (its match has
nothing to split); and the head must be named as a `Comparator`'s field (`(linearOrderComparator
Bool).cmp`), because `cmpSend`'s persistence flag is compared with `_root_.cmp` — the natural order,
unlike `Ground.bool`, whose polarity the node reverses (`cmpBool`).

**What remains is volume, not novelty — with one exception.** `cmpPar`'s chain is eight deep, so its tail
packages as a right-nested product and the proof nests seven `lex_lt_trans` applications; the small
structures are the same shape in fewer steps. The exception is `cmpExpr`: 21 groups mean a 21×21×21 case
analysis whose `simp` must **not** unfold the 21-arm match (the recorded reason its two laws are axioms),
so it needs the `cmpExpr` arm lemmas — `rfl`-proved, as `Json.lean`'s are — *before* the nesting for its
five shapes (grounds / the four collections / a var / monadic / binary). That is the first piece to do
next time, and it pays twice: the same arm lemmas are what `cmpExpr`'s `eq_iff` and `swap` need.

**The recorded blocker, measured (2026-09-23).** The note above says `simp`/`rw` "hit recursion depth" on
the 21-constructor function. Probed, it is worse than that: the straight attempt at `cmpExpr`'s `eq_iff`

    cases s <;> cases t <;> simp [cmpExpr, cmpGround_eq_iff, cmpVar_eq_iff, cmpPar_eq_iff,
      cmpListPar_eq_iff, cmpOptionVar, cmpListParPair_eq_iff]

with `set_option maxRecDepth 100000` ends in **`Stack overflow detected. Aborting.`** — not a limit the
tactic's option can raise, because the blow-up is in the native stack. So the arm lemmas are not a
convenience here, they are the only route: with `cmpExpr` never unfolded, `simp` cannot recurse. The
volume that follows is the 21×20 cross-constructor cases, which no single `rfl` lemma covers (the
fallback arms make each pair a *different* reduction), and that is why this stays time-boxed rather than
being the pass's objective — the `sort` corpus already pins the order against the node, so the residue is
"axioms that are tied", not "an order nothing checks").

### What the score order actually is, and where this model still differs (extracted, then aligned, 2026-09-23)

The `sort` corpus (`spec/conformance/sort.tsv` + `rholang/tests/lean_sort_corpus.rs`) ties the two
orders pairwise, and reading `models/src/sorter.rs`'s `sort_*` functions against this file gave the
scoped divergence. **Four structures were aligned to the node's score tags and are pinned by corpus
rows 13–19** (each verdict was *observed* from the node — the Rust consumer's diagnostic read it off
`sort_pars` — and the model was then corrected to it, not the other way round):

| where | the node's order (now this model's) | what it was | pinned by |
|---|---|---|---|
| `Send` | `persistent, chan, data…, connective_used` | `chan, data, persist` | `@"a"!!(1)` vs `@"b"!(1)` → **gt** |
| `Par` | `sends, receives, exprs, news, matches, bundles, connectives, unforgeables` | news before exprs, unforgeables before bundles | `new x in { Nil } \| 1` vs `[1]` → **lt** (and its reverse → **gt**) |
| `Expr` classes | tags: grounds(1-4) < `elist`(6) < `etuple`(7) < `eset`(8) < `emap`(9) < vars(50-52) < `evar`(100) < `eneg`(101) < `emult`(102) < `ediv`(103) < `eplus`(104) < `eminus`(105) < `elt`(106) < `ele`(107) < `egt`(108) < `ege`(109) < `eeq`(110) < `eneq`(111) < `enot`(112) < `eand`(113) < `eor`(114) < `emod`(122) | declaration order: grounds, `evar`, `eneg`, `enot`, `eplus`…`emod`, `elist`…`emap` | `[1]` vs `1 + 2` → **lt**; `1 * 2` vs `1 + 2` → **lt**; `1 - 2` vs `1 * 2` → **gt** |
| `Ground.bool` | `true` scores 0, `false` 1 — **`true` sorts first** (`cmpBool`) | `linearOrderComparator Bool` (false first) | `false` vs `true` → **gt** |

Each row is a falsifier, checked rather than assumed: restoring the old order (or flipping one
polarity) stops `Rchain.Corpus`'s `sortCases_decide` from compiling and the runtime reporter names the
row — done for all four before they were believed.

**The model's comparators are still an order on a *coarser* algebra** — 21 `Expr` constructors against
the node's 33 — so the alignment is partial by construction. The boundary, stated rather than fixed:
`Receive`'s `persistent, peek` first and its `bind_count` child; `ReceiveBind`'s `source` first with
`free_count` **dropped from the score**; `New`'s `uri` (sorted) and `injections` (key-sorted) children;
`Bundle`'s flags folded into the tag; `EList`/`ESet`/`EMap`'s `remainder` **before** the elements (and a
list with no remainder scoring `-1`, *before* `ABSENT = 0`); `MatchCase`/`Match`/`EMethod`'s
`connective_used`; `Var`'s `Empty`(0); `GUnforgeable`'s `gDeployerId`(10) **before** `gDeployId`(11) —
the reverse of the port's own enum order; `Connective`'s tags (400-409). And twelve of the node's 33
`Expr` constructors — `EMethod`(115), `EMatches`(118), `EPercentPercent`(119), `EPlusPlus`(120),
`EMinusMinus`(121), `EShortAnd`(123), `EShortOr`(124), `GBigInt`(13), `GByteArray`(116) and the rest —
**do not exist in this model at all**, so a term containing one cannot be compared here; nor can the
corpus carry such a row, since `1 && 2` parses to a method call rather than to `eand` and `1 %% 2` to
`EPercentPercent`. `GByteArray` is the sharp case: the model *has* the constructor (`Ground.bytes`) but
at the wrong tag — the node scores it 116, after every operator — and it cannot be pinned either way,
because the node's front end has no byte-array literal (`@"c"!(b"a")` is a `SyntaxError`), so the term
is unspellable rather than merely mis-scored. `Ground.bytes`'s position is therefore left as the one
known, unpinnable divergence, and `cmpGround`'s doc comment says so.
-/



axiom cmpExpr_lt_trans (s t u : Expr) : cmpExpr s t = Ordering.lt → cmpExpr t u = Ordering.lt → cmpExpr s u = Ordering.lt

/-! ### The `lt_trans` laws

`cmpExpr`'s element law is the one axiom left of law 1's residual: its three laws wait on the `cmpExpr`
arm lemmas (`Json.lean:356-386`'s precedent — 21 `rfl`-proved `@[simp]` arms), because `cmpExpr`'s 21
groups mean a 21×21×21 case analysis whose `simp` must not unfold the 21-arm match.

Everything else in the family is proved, and the `Par` half of it is **one `mutual` block** below,
because the family is one strongly connected component. What stays *outside* the block, and why: the two
`GUnforgeable` laws and `cmpListExpr_lt_trans` are not in the cycle (the latter is about `Expr`, and
`Expr`'s own element law is the axiom above), and the block needs them — `Par` carries unforgeables and
exprs, so their laws have to be declared first. -/

private theorem cmpParPair_eq_iff (x y : Par × Par) :
    lex (cmpPar x.1 y.1) (cmpPar x.2 y.2) = Ordering.eq ↔ x = y := by
  rw [lex_eq_iff, cmpPar_eq_iff, cmpPar_eq_iff, Prod.ext_iff]


theorem cmpGUnforgeable_lt_trans (s t u : GUnforgeable) :
    cmpGUnforgeable s t = Ordering.lt → cmpGUnforgeable t u = Ordering.lt → cmpGUnforgeable s u = Ordering.lt := by
  intro h1 h2
  cases s <;> cases t <;> cases u <;> simp [cmpGUnforgeable] at h1 h2 ⊢
  all_goals exact _root_.lt_trans h1 h2

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


/-! ## The `Par` cycle, as one block

The `lt_trans` laws whose types mention `Par`: `cmpPar` itself, the element laws, and the list
laws — one strongly connected component, so one `mutual` block. Every same-block call is a
*partial application on fields* (`h_lt := cmpListSend_lt_trans s s' s''`), which is the only
shape this block's termination checker accepts; `Cmp.lean`'s `lex_lt_trans_at` is what makes
that spelling available at each level. -/

mutual
  theorem cmpPar_lt_trans : ∀ p : Par, ∀ q r : Par,
      cmpPar p q = Ordering.lt → cmpPar q r = Ordering.lt → cmpPar p r = Ordering.lt
    | Par.mk s r n e m u b c, Par.mk s' r' n' e' m' u' b' c',
      Par.mk s'' r'' n'' e'' m'' u'' b'' c'' => by
      intro h1 h2
      simp only [cmpPar] at h1 h2 ⊢
      refine lex_lt_trans_at (f := cmpListSend)
        (h_eq := fun {a b} => cmpListSend_eq_iff a b)
        (a := s) (b := s') (c := s'')
        (h_lt := (cmpListSend_lt_trans s s' s''))
        (Dcmp := cmpPairF cmpListReceive (cmpPairF cmpListExpr (cmpPairF cmpListNew (cmpPairF cmpListMatch (cmpPairF cmpListBundle (cmpPairF cmpListConnective (cmpListGUnforgeable)))))))
        (x := (r, (e, (n, (m, (b, (c, u)))))))
        (y := (r', (e', (n', (m', (b', (c', u')))))))
        (z := (r'', (e'', (n'', (m'', (b'', (c'', u'')))))))
        (hD := ?_) h1 h2
      intro hx hy
      refine lex_lt_trans_at (f := cmpListReceive)
        (h_eq := fun {a b} => cmpListReceive_eq_iff a b)
        (a := r) (b := r') (c := r'')
        (h_lt := (cmpListReceive_lt_trans r r' r''))
        (Dcmp := cmpPairF cmpListExpr (cmpPairF cmpListNew (cmpPairF cmpListMatch (cmpPairF cmpListBundle (cmpPairF cmpListConnective (cmpListGUnforgeable))))))
        (x := (e, (n, (m, (b, (c, u))))))
        (y := (e', (n', (m', (b', (c', u'))))))
        (z := (e'', (n'', (m'', (b'', (c'', u''))))))
        (hD := ?_) hx hy
      intro hx hy
      refine lex_lt_trans (f := cmpListExpr)
        (h_eq := fun {a b} => cmpListExpr_eq_iff a b)
        (h_lt := fun {a b c} h1 h2 => cmpListExpr_lt_trans a b c h1 h2)
        (Dcmp := cmpPairF cmpListNew (cmpPairF cmpListMatch (cmpPairF cmpListBundle (cmpPairF cmpListConnective (cmpListGUnforgeable)))))
        (x := (n, (m, (b, (c, u)))))
        (y := (n', (m', (b', (c', u')))))
        (z := (n'', (m'', (b'', (c'', u'')))))
        (hD := ?_) hx hy
      intro hx hy
      refine lex_lt_trans_at (f := cmpListNew)
        (h_eq := fun {a b} => cmpListNew_eq_iff a b)
        (a := n) (b := n') (c := n'')
        (h_lt := (cmpListNew_lt_trans n n' n''))
        (Dcmp := cmpPairF cmpListMatch (cmpPairF cmpListBundle (cmpPairF cmpListConnective (cmpListGUnforgeable))))
        (x := (m, (b, (c, u))))
        (y := (m', (b', (c', u'))))
        (z := (m'', (b'', (c'', u''))))
        (hD := ?_) hx hy
      intro hx hy
      refine lex_lt_trans_at (f := cmpListMatch)
        (h_eq := fun {a b} => cmpListMatch_eq_iff a b)
        (a := m) (b := m') (c := m'')
        (h_lt := (cmpListMatch_lt_trans m m' m''))
        (Dcmp := cmpPairF cmpListBundle (cmpPairF cmpListConnective (cmpListGUnforgeable)))
        (x := (b, (c, u)))
        (y := (b', (c', u')))
        (z := (b'', (c'', u'')))
        (hD := ?_) hx hy
      intro hx hy
      refine lex_lt_trans_at (f := cmpListBundle)
        (h_eq := fun {a b} => cmpListBundle_eq_iff a b)
        (a := b) (b := b') (c := b'')
        (h_lt := (cmpListBundle_lt_trans b b' b''))
        (Dcmp := cmpPairF cmpListConnective (cmpListGUnforgeable))
        (x := (c, u))
        (y := (c', u'))
        (z := (c'', u''))
        (hD := ?_) hx hy
      intro hx hy
      refine lex_lt_trans_at (f := cmpListConnective)
        (h_eq := fun {a b} => cmpListConnective_eq_iff a b)
        (a := c) (b := c') (c := c'')
        (h_lt := (cmpListConnective_lt_trans c c' c''))
        (Dcmp := cmpListGUnforgeable)
        (x := u) (y := u') (z := u'')
        (hD := fun h1 h2 => cmpListGUnforgeable_lt_trans u u' u'' h1 h2) hx hy
  termination_by p => sizeOf p

  theorem cmpSend_lt_trans : ∀ x1 : Send, ∀ x2 x3 : Send,
      cmpSend x1 x2 = Ordering.lt → cmpSend x2 x3 = Ordering.lt → cmpSend x1 x3 = Ordering.lt
    | Send.mk c d p, Send.mk c' d' p', Send.mk c'' d'' p'', h1, h2 => by
        simp only [cmpSend] at h1 h2 ⊢
        refine lex_lt_trans (f := (Comparator.linearOrderComparator Bool).cmp)
          (h_eq := fun {a b} => (Comparator.linearOrderComparator Bool).eq_iff (a := a) (b := b))
          (h_lt := fun {a b c} h1 h2 => (Comparator.linearOrderComparator Bool).lt_trans h1 h2)
          (Dcmp := cmpPairF cmpPar (cmpListPar))
          (x := (c, d))
          (y := (c', d'))
          (z := (c'', d''))
          (hD := ?_) h1 h2
        intro hx hy
        refine lex_lt_trans_at (f := cmpPar)
          (h_eq := fun {a b} => cmpPar_eq_iff a b)
          (a := c) (b := c') (c := c'')
          (h_lt := (cmpPar_lt_trans c c' c''))
          (Dcmp := cmpListPar)
          (x := d) (y := d') (z := d'')
          (hD := cmpListPar_lt_trans d d' d'') hx hy
  termination_by x1 => sizeOf x1

  theorem cmpReceiveBind_lt_trans : ∀ x1 : ReceiveBind, ∀ x2 x3 : ReceiveBind,
      cmpReceiveBind x1 x2 = Ordering.lt → cmpReceiveBind x2 x3 = Ordering.lt → cmpReceiveBind x1 x3 = Ordering.lt
    | ReceiveBind.mk ps s n, ReceiveBind.mk ps' s' n', ReceiveBind.mk ps'' s'' n'', h1, h2 => by
        simp only [cmpReceiveBind] at h1 h2 ⊢
        refine lex_lt_trans_at (f := cmpListPar)
          (h_eq := fun {a b} => cmpListPar_eq_iff a b)
          (a := ps) (b := ps') (c := ps'')
          (h_lt := (cmpListPar_lt_trans ps ps' ps''))
          (Dcmp := cmpPairF cmpPar ((Comparator.linearOrderComparator Nat).cmp))
          (x := (s, n))
          (y := (s', n'))
          (z := (s'', n''))
          (hD := ?_) h1 h2
        intro hx hy
        refine lex_lt_trans_at (f := cmpPar)
          (h_eq := fun {a b} => cmpPar_eq_iff a b)
          (a := s) (b := s') (c := s'')
          (h_lt := (cmpPar_lt_trans s s' s''))
          (Dcmp := (Comparator.linearOrderComparator Nat).cmp)
          (x := n) (y := n') (z := n'')
          (hD := fun h1 h2 => (Comparator.linearOrderComparator Nat).lt_trans h1 h2) hx hy
  termination_by x1 => sizeOf x1

  theorem cmpReceive_lt_trans : ∀ x1 : Receive, ∀ x2 x3 : Receive,
      cmpReceive x1 x2 = Ordering.lt → cmpReceive x2 x3 = Ordering.lt → cmpReceive x1 x3 = Ordering.lt
    | Receive.mk bs b p n, Receive.mk bs' b' p' n', Receive.mk bs'' b'' p'' n'', h1, h2 => by
        simp only [cmpReceive] at h1 h2 ⊢
        refine lex_lt_trans_at (f := cmpListReceiveBind)
          (h_eq := fun {a b} => cmpListReceiveBind_eq_iff a b)
          (a := bs) (b := bs') (c := bs'')
          (h_lt := (cmpListReceiveBind_lt_trans bs bs' bs''))
          (Dcmp := cmpPairF cmpPar (cmpPairF (Comparator.linearOrderComparator Bool).cmp ((Comparator.linearOrderComparator Nat).cmp)))
          (x := (b, (p, n)))
          (y := (b', (p', n')))
          (z := (b'', (p'', n'')))
          (hD := ?_) h1 h2
        intro hx hy
        refine lex_lt_trans_at (f := cmpPar)
          (h_eq := fun {a b} => cmpPar_eq_iff a b)
          (a := b) (b := b') (c := b'')
          (h_lt := (cmpPar_lt_trans b b' b''))
          (Dcmp := cmpPairF (Comparator.linearOrderComparator Bool).cmp ((Comparator.linearOrderComparator Nat).cmp))
          (x := (p, n))
          (y := (p', n'))
          (z := (p'', n''))
          (hD := ?_) hx hy
        intro hx hy
        refine lex_lt_trans (f := (Comparator.linearOrderComparator Bool).cmp)
          (h_eq := fun {a b} => (Comparator.linearOrderComparator Bool).eq_iff (a := a) (b := b))
          (h_lt := fun {a b c} h1 h2 => (Comparator.linearOrderComparator Bool).lt_trans h1 h2)
          (Dcmp := (Comparator.linearOrderComparator Nat).cmp)
          (x := n) (y := n') (z := n'')
          (hD := fun h1 h2 => (Comparator.linearOrderComparator Nat).lt_trans h1 h2) hx hy
  termination_by x1 => sizeOf x1

  theorem cmpNew_lt_trans : ∀ x1 : New, ∀ x2 x3 : New,
      cmpNew x1 x2 = Ordering.lt → cmpNew x2 x3 = Ordering.lt → cmpNew x1 x3 = Ordering.lt
    | New.mk n b, New.mk n' b', New.mk n'' b'', h1, h2 => by
        simp only [cmpNew] at h1 h2 ⊢
        refine lex_lt_trans (f := (Comparator.linearOrderComparator Nat).cmp)
          (h_eq := fun {a b} => (Comparator.linearOrderComparator Nat).eq_iff (a := a) (b := b))
          (h_lt := fun {a b c} h1 h2 => (Comparator.linearOrderComparator Nat).lt_trans h1 h2)
          (Dcmp := cmpPar)
          (x := b) (y := b') (z := b'')
          (hD := cmpPar_lt_trans b b' b'') h1 h2
  termination_by x1 => sizeOf x1

  theorem cmpMatchCase_lt_trans : ∀ x1 : MatchCase, ∀ x2 x3 : MatchCase,
      cmpMatchCase x1 x2 = Ordering.lt → cmpMatchCase x2 x3 = Ordering.lt → cmpMatchCase x1 x3 = Ordering.lt
    | MatchCase.mk p src fc, MatchCase.mk p' src' fc', MatchCase.mk p'' src'' fc'', h1, h2 => by
        simp only [cmpMatchCase] at h1 h2 ⊢
        refine lex_lt_trans_at (f := cmpPar)
          (h_eq := fun {a b} => cmpPar_eq_iff a b)
          (a := p) (b := p') (c := p'')
          (h_lt := (cmpPar_lt_trans p p' p''))
          (Dcmp := cmpPairF cmpPar ((Comparator.linearOrderComparator Nat).cmp))
          (x := (src, fc))
          (y := (src', fc'))
          (z := (src'', fc''))
          (hD := ?_) h1 h2
        intro hx hy
        refine lex_lt_trans_at (f := cmpPar)
          (h_eq := fun {a b} => cmpPar_eq_iff a b)
          (a := src) (b := src') (c := src'')
          (h_lt := (cmpPar_lt_trans src src' src''))
          (Dcmp := (Comparator.linearOrderComparator Nat).cmp)
          (x := fc) (y := fc') (z := fc'')
          (hD := fun h1 h2 => (Comparator.linearOrderComparator Nat).lt_trans h1 h2) hx hy
  termination_by x1 => sizeOf x1

  theorem cmpMatch_lt_trans : ∀ x1 : Match, ∀ x2 x3 : Match,
      cmpMatch x1 x2 = Ordering.lt → cmpMatch x2 x3 = Ordering.lt → cmpMatch x1 x3 = Ordering.lt
    | Match.mk t1 cs, Match.mk t1' cs', Match.mk t1'' cs'', h1, h2 => by
        simp only [cmpMatch] at h1 h2 ⊢
        refine lex_lt_trans_at (f := cmpPar)
          (h_eq := fun {a b} => cmpPar_eq_iff a b)
          (a := t1) (b := t1') (c := t1'')
          (h_lt := (cmpPar_lt_trans t1 t1' t1''))
          (Dcmp := cmpListMatchCase)
          (x := cs) (y := cs') (z := cs'')
          (hD := cmpListMatchCase_lt_trans cs cs' cs'') h1 h2
  termination_by x1 => sizeOf x1

  theorem cmpBundle_lt_trans : ∀ x1 : Bundle, ∀ x2 x3 : Bundle,
      cmpBundle x1 x2 = Ordering.lt → cmpBundle x2 x3 = Ordering.lt → cmpBundle x1 x3 = Ordering.lt
    | Bundle.mk b w r, Bundle.mk b' w' r', Bundle.mk b'' w'' r'', h1, h2 => by
        simp only [cmpBundle] at h1 h2 ⊢
        refine lex_lt_trans_at (f := cmpPar)
          (h_eq := fun {a b} => cmpPar_eq_iff a b)
          (a := b) (b := b') (c := b'')
          (h_lt := (cmpPar_lt_trans b b' b''))
          (Dcmp := cmpPairF (Comparator.linearOrderComparator Bool).cmp ((Comparator.linearOrderComparator Bool).cmp))
          (x := (w, r))
          (y := (w', r'))
          (z := (w'', r''))
          (hD := ?_) h1 h2
        intro hx hy
        refine lex_lt_trans (f := (Comparator.linearOrderComparator Bool).cmp)
          (h_eq := fun {a b} => (Comparator.linearOrderComparator Bool).eq_iff (a := a) (b := b))
          (h_lt := fun {a b c} h1 h2 => (Comparator.linearOrderComparator Bool).lt_trans h1 h2)
          (Dcmp := (Comparator.linearOrderComparator Bool).cmp)
          (x := r) (y := r') (z := r'')
          (hD := fun h1 h2 => (Comparator.linearOrderComparator Bool).lt_trans h1 h2) hx hy
  termination_by x1 => sizeOf x1

  theorem cmpListSend_lt_trans : ∀ l : List Send, ∀ l' l'' : List Send,
      cmpListSend l l' = Ordering.lt → cmpListSend l' l'' = Ordering.lt → cmpListSend l l'' = Ordering.lt
    | [], l', l'', h1, h2 => by cases l' <;> cases l'' <;> simp [cmpListSend] at h1 h2 ⊢
    | a :: as, [], l'', h1, _ => by simp [cmpListSend] at h1
    | a :: as, b :: bs, [], _, h2 => by simp [cmpListSend] at h2
    | a :: as, b :: bs, c :: cs, h1, h2 => by
        simp only [cmpListSend] at h1 h2 ⊢
        exact lex_lt_trans_at (f := cmpSend) (h_eq := fun {a b} => cmpSend_eq_iff a b)
          (a := a) (b := b) (c := c) (h_lt := (cmpSend_lt_trans a b c))
          (Dcmp := cmpListSend) (x := as) (y := bs) (z := cs)
          (hD := cmpListSend_lt_trans as bs cs) h1 h2
  termination_by l => sizeOf l

  theorem cmpListReceive_lt_trans : ∀ l : List Receive, ∀ l' l'' : List Receive,
      cmpListReceive l l' = Ordering.lt → cmpListReceive l' l'' = Ordering.lt → cmpListReceive l l'' = Ordering.lt
    | [], l', l'', h1, h2 => by cases l' <;> cases l'' <;> simp [cmpListReceive] at h1 h2 ⊢
    | a :: as, [], l'', h1, _ => by simp [cmpListReceive] at h1
    | a :: as, b :: bs, [], _, h2 => by simp [cmpListReceive] at h2
    | a :: as, b :: bs, c :: cs, h1, h2 => by
        simp only [cmpListReceive] at h1 h2 ⊢
        exact lex_lt_trans_at (f := cmpReceive) (h_eq := fun {a b} => cmpReceive_eq_iff a b)
          (a := a) (b := b) (c := c) (h_lt := (cmpReceive_lt_trans a b c))
          (Dcmp := cmpListReceive) (x := as) (y := bs) (z := cs)
          (hD := cmpListReceive_lt_trans as bs cs) h1 h2
  termination_by l => sizeOf l

  theorem cmpListNew_lt_trans : ∀ l : List New, ∀ l' l'' : List New,
      cmpListNew l l' = Ordering.lt → cmpListNew l' l'' = Ordering.lt → cmpListNew l l'' = Ordering.lt
    | [], l', l'', h1, h2 => by cases l' <;> cases l'' <;> simp [cmpListNew] at h1 h2 ⊢
    | a :: as, [], l'', h1, _ => by simp [cmpListNew] at h1
    | a :: as, b :: bs, [], _, h2 => by simp [cmpListNew] at h2
    | a :: as, b :: bs, c :: cs, h1, h2 => by
        simp only [cmpListNew] at h1 h2 ⊢
        exact lex_lt_trans_at (f := cmpNew) (h_eq := fun {a b} => cmpNew_eq_iff a b)
          (a := a) (b := b) (c := c) (h_lt := (cmpNew_lt_trans a b c))
          (Dcmp := cmpListNew) (x := as) (y := bs) (z := cs)
          (hD := cmpListNew_lt_trans as bs cs) h1 h2
  termination_by l => sizeOf l

  theorem cmpListMatch_lt_trans : ∀ l : List Match, ∀ l' l'' : List Match,
      cmpListMatch l l' = Ordering.lt → cmpListMatch l' l'' = Ordering.lt → cmpListMatch l l'' = Ordering.lt
    | [], l', l'', h1, h2 => by cases l' <;> cases l'' <;> simp [cmpListMatch] at h1 h2 ⊢
    | a :: as, [], l'', h1, _ => by simp [cmpListMatch] at h1
    | a :: as, b :: bs, [], _, h2 => by simp [cmpListMatch] at h2
    | a :: as, b :: bs, c :: cs, h1, h2 => by
        simp only [cmpListMatch] at h1 h2 ⊢
        exact lex_lt_trans_at (f := cmpMatch) (h_eq := fun {a b} => cmpMatch_eq_iff a b)
          (a := a) (b := b) (c := c) (h_lt := (cmpMatch_lt_trans a b c))
          (Dcmp := cmpListMatch) (x := as) (y := bs) (z := cs)
          (hD := cmpListMatch_lt_trans as bs cs) h1 h2
  termination_by l => sizeOf l

  theorem cmpListBundle_lt_trans : ∀ l : List Bundle, ∀ l' l'' : List Bundle,
      cmpListBundle l l' = Ordering.lt → cmpListBundle l' l'' = Ordering.lt → cmpListBundle l l'' = Ordering.lt
    | [], l', l'', h1, h2 => by cases l' <;> cases l'' <;> simp [cmpListBundle] at h1 h2 ⊢
    | a :: as, [], l'', h1, _ => by simp [cmpListBundle] at h1
    | a :: as, b :: bs, [], _, h2 => by simp [cmpListBundle] at h2
    | a :: as, b :: bs, c :: cs, h1, h2 => by
        simp only [cmpListBundle] at h1 h2 ⊢
        exact lex_lt_trans_at (f := cmpBundle) (h_eq := fun {a b} => cmpBundle_eq_iff a b)
          (a := a) (b := b) (c := c) (h_lt := (cmpBundle_lt_trans a b c))
          (Dcmp := cmpListBundle) (x := as) (y := bs) (z := cs)
          (hD := cmpListBundle_lt_trans as bs cs) h1 h2
  termination_by l => sizeOf l

  theorem cmpListConnective_lt_trans : ∀ l : List Connective, ∀ l' l'' : List Connective,
      cmpListConnective l l' = Ordering.lt → cmpListConnective l' l'' = Ordering.lt → cmpListConnective l l'' = Ordering.lt
    | [], l', l'', h1, h2 => by cases l' <;> cases l'' <;> simp [cmpListConnective] at h1 h2 ⊢
    | a :: as, [], l'', h1, _ => by simp [cmpListConnective] at h1
    | a :: as, b :: bs, [], _, h2 => by simp [cmpListConnective] at h2
    | a :: as, b :: bs, c :: cs, h1, h2 => by
        simp only [cmpListConnective] at h1 h2 ⊢
        exact lex_lt_trans_at (f := cmpConnective) (h_eq := fun {a b} => cmpConnective_eq_iff a b)
          (a := a) (b := b) (c := c) (h_lt := (cmpConnective_lt_trans a b c))
          (Dcmp := cmpListConnective) (x := as) (y := bs) (z := cs)
          (hD := cmpListConnective_lt_trans as bs cs) h1 h2
  termination_by l => sizeOf l

  theorem cmpListPar_lt_trans : ∀ l : List Par, ∀ l' l'' : List Par,
      cmpListPar l l' = Ordering.lt → cmpListPar l' l'' = Ordering.lt → cmpListPar l l'' = Ordering.lt
    | [], l', l'', h1, h2 => by cases l' <;> cases l'' <;> simp [cmpListPar] at h1 h2 ⊢
    | a :: as, [], l'', h1, _ => by simp [cmpListPar] at h1
    | a :: as, b :: bs, [], _, h2 => by simp [cmpListPar] at h2
    | a :: as, b :: bs, c :: cs, h1, h2 => by
        simp only [cmpListPar] at h1 h2 ⊢
        exact lex_lt_trans_at (f := cmpPar) (h_eq := fun {a b} => cmpPar_eq_iff a b)
          (a := a) (b := b) (c := c) (h_lt := (cmpPar_lt_trans a b c))
          (Dcmp := cmpListPar) (x := as) (y := bs) (z := cs)
          (hD := cmpListPar_lt_trans as bs cs) h1 h2
  termination_by l => sizeOf l

  theorem cmpListReceiveBind_lt_trans : ∀ l : List ReceiveBind, ∀ l' l'' : List ReceiveBind,
      cmpListReceiveBind l l' = Ordering.lt → cmpListReceiveBind l' l'' = Ordering.lt → cmpListReceiveBind l l'' = Ordering.lt
    | [], l', l'', h1, h2 => by cases l' <;> cases l'' <;> simp [cmpListReceiveBind] at h1 h2 ⊢
    | a :: as, [], l'', h1, _ => by simp [cmpListReceiveBind] at h1
    | a :: as, b :: bs, [], _, h2 => by simp [cmpListReceiveBind] at h2
    | a :: as, b :: bs, c :: cs, h1, h2 => by
        simp only [cmpListReceiveBind] at h1 h2 ⊢
        exact lex_lt_trans_at (f := cmpReceiveBind) (h_eq := fun {a b} => cmpReceiveBind_eq_iff a b)
          (a := a) (b := b) (c := c) (h_lt := (cmpReceiveBind_lt_trans a b c))
          (Dcmp := cmpListReceiveBind) (x := as) (y := bs) (z := cs)
          (hD := cmpListReceiveBind_lt_trans as bs cs) h1 h2
  termination_by l => sizeOf l

  theorem cmpListMatchCase_lt_trans : ∀ l : List MatchCase, ∀ l' l'' : List MatchCase,
      cmpListMatchCase l l' = Ordering.lt → cmpListMatchCase l' l'' = Ordering.lt → cmpListMatchCase l l'' = Ordering.lt
    | [], l', l'', h1, h2 => by cases l' <;> cases l'' <;> simp [cmpListMatchCase] at h1 h2 ⊢
    | a :: as, [], l'', h1, _ => by simp [cmpListMatchCase] at h1
    | a :: as, b :: bs, [], _, h2 => by simp [cmpListMatchCase] at h2
    | a :: as, b :: bs, c :: cs, h1, h2 => by
        simp only [cmpListMatchCase] at h1 h2 ⊢
        exact lex_lt_trans_at (f := cmpMatchCase) (h_eq := fun {a b} => cmpMatchCase_eq_iff a b)
          (a := a) (b := b) (c := c) (h_lt := (cmpMatchCase_lt_trans a b c))
          (Dcmp := cmpListMatchCase) (x := as) (y := bs) (z := cs)
          (hD := cmpListMatchCase_lt_trans as bs cs) h1 h2
  termination_by l => sizeOf l

  theorem cmpListParPair_lt_trans : ∀ l : List (Par × Par), ∀ l' l'' : List (Par × Par),
      cmpListParPair l l' = Ordering.lt → cmpListParPair l' l'' = Ordering.lt → cmpListParPair l l'' = Ordering.lt
    | [], l', l'', h1, h2 => by cases l' <;> cases l'' <;> simp [cmpListParPair] at h1 h2 ⊢
    | a :: as, [], l'', h1, _ => by simp [cmpListParPair] at h1
    | a :: as, b :: bs, [], _, h2 => by simp [cmpListParPair] at h2
    | (a1, a2) :: as, (b1, b2) :: bs, (c1, c2) :: cs, h1, h2 => by
        simp only [cmpListParPair] at h1 h2 ⊢
        exact lex_lt_trans_at (f := fun x y => lex (cmpPar x.1 y.1) (cmpPar x.2 y.2))
          (h_eq := fun {a b} => cmpParPair_eq_iff a b)
          (a := (a1, a2)) (b := (b1, b2)) (c := (c1, c2))
          (h_lt := (cmpParPair_lt_trans (a1, a2) (b1, b2) (c1, c2)))
          (Dcmp := cmpListParPair) (x := as) (y := bs) (z := cs)
          (hD := cmpListParPair_lt_trans as bs cs) h1 h2
  termination_by l => sizeOf l

  theorem cmpConnective_lt_trans : ∀ x1 : Connective, ∀ x2 x3 : Connective,
      cmpConnective x1 x2 = Ordering.lt → cmpConnective x2 x3 = Ordering.lt →
        cmpConnective x1 x3 = Ordering.lt
    | s, t, u, h1, h2 => by
      cases s <;> cases t <;> cases u 
      all_goals simp only [cmpConnective] at h1 h2 ⊢
      all_goals first
        | exact cmpListPar_lt_trans _ _ _ h1 h2
        | exact cmpPar_lt_trans _ _ _ h1 h2
        | exact Comparator.lex_lt_trans (f := (Comparator.linearOrderComparator Nat).cmp)
            (h_eq := fun {a b} => (Comparator.linearOrderComparator Nat).eq_iff (a := a) (b := b))
            (h_lt := fun {a b c} h1 h2 => (Comparator.linearOrderComparator Nat).lt_trans h1 h2)
            (Dcmp := (Comparator.linearOrderComparator Nat).cmp)
            (hD := fun h1 h2 => (Comparator.linearOrderComparator Nat).lt_trans h1 h2) h1 h2
        | simp_all
  termination_by x1 => sizeOf x1

  private theorem cmpParPair_lt_trans : ∀ q1 q2 q3 : Par × Par,
      lex (cmpPar q1.1 q2.1) (cmpPar q1.2 q2.2) = Ordering.lt →
      lex (cmpPar q2.1 q3.1) (cmpPar q2.2 q3.2) = Ordering.lt →
      lex (cmpPar q1.1 q3.1) (cmpPar q1.2 q3.2) = Ordering.lt
    | (x1, x2), (y1, y2), (z1, z2), h1, h2 => by
      simp only at h1 h2 ⊢
      exact lex_lt_trans_at (f := cmpPar) (h_eq := fun {a b} => cmpPar_eq_iff a b)
        (a := x1) (b := y1) (c := z1) (h_lt := (cmpPar_lt_trans x1 y1 z1))
        (Dcmp := cmpPar) (x := x2) (y := y2) (z := z2)
        (hD := cmpPar_lt_trans x2 y2 z2) h1 h2
  termination_by q1 => sizeOf q1
end

/-! ### `cmpPar_lt_trans`: landed as a block member (2026-09-24)

The proof is the seven-deep ladder the earlier note recorded — one level per component of `cmpPar`'s
eight-component chain, the tail packaged as a right-nested `Comparator.cmpPairF` over a right-nested
tuple — and it is a **member of the `mutual` block** above now rather than a theorem of its own, because
the family is one strongly connected component (`Par`'s fields are lists of `Send`, whose data is a list
of `Par`).

What blocked the earlier attempt is worth keeping, because it is the shape of the whole conversion and it
was measured rather than argued: a `fun {a b c} …` lambda whose body calls a *same-block* member cannot
pass the termination checker — the call's arguments are not structural subterms of the member's own
arguments, so `termination_by p _ _ => sizeOf p` has nothing to say about them. `Cmp.lean`'s
`lex_lt_trans_at` is the pointwise form of `lex_lt_trans` that fixes it: its `h_lt` is taken at the
*specific* triple, so every level writes `(a := s) (b := s') (c := s'') (h_lt := cmpListSend_lt_trans s
s' s'')` — fields, not lambdas. Four other things the probe found, all recorded here rather than
rediscovered: a member whose field name collides with its statement's binder (`cmpReceiveBind`'s `s` —
its statement names its binders `x1 x2 x3` now); the last component of a chain needs
`(Dcmp := …) (x := u)` rather than another `lex_lt_trans` level; a *generic* level (`Bool`, `Nat`) keeps
the general `lex_lt_trans`, because rewriting that one to the pointwise form breaks it; and
`cmpConnective`'s `first | …` cascade stays as written. -/

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

