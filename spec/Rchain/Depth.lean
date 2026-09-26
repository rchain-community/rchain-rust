import Rchain.Par

/-!
# Law 50 — the AST-depth budget

`rholang/src/parser.rs` refuses a term whose **AST depth** exceeds `MAX_AST_DEPTH`. This module is
the quantity that bound is about.

**Why a law and not a comment (AUDIT C99).** The parser's older guards bound two *shapes* and compose
into nothing: `MAX_PARSE_DEPTH` bounds the parser's recursion, and a flat operator chain is built by a
loop — so it costs a bounded number of frames and produces an **AST of depth `n`** — and `d` levels of
`c` operators compose into an AST of depth `d × c` with both guards satisfied. Every consumer of the
term then recurses once per level (`normalize_proc`, `sort_par`/`sort_expr`, `well_scoped_par`,
`eval_expr_to_expr`, `spatial_match_core`, `build_string`), which on the node's 32 MiB worker stack is
a **process abort** at an AST depth of ~1,000 (a ~4 KB deploy) and 64 s of CPU at ~8,000. The fix is a
bound on the *term*, and this is the term's depth.

**What is here, and what is owed.** `parDepth` below is the definition — the depth of a `Par` as a
tree of subterms, field-wise on the flat `Par` exactly as `closed` and `freeVarOf` are. The obligation
that is **not** yet discharged is the walk: `rholang/src/parser.rs::exceeds_ast_depth` tests
`parDepth p ≤ MAX_AST_DEPTH` by an iterative, early-exiting traversal (iterative because the depth is
the thing a recursive walk cannot survive), and what ties the two is

  * `walkExceeds limit p = false → parDepth p ≤ limit` — the direction that matters: a **sound**
    refusal test, so a term the node accepts cannot be deeper than the bound; and its control,
  * `parDepth p ≤ limit → walkExceeds limit p = false` — completeness, so the walk is not refusing by
    accident.

Both are owed; the row is `owed` for exactly that reason, and `spec/INDUCTIVE`-style recipes are not
needed to say so. The shape to prove them in is `Rchain/FreeVars.lean`'s: one `mutual` block with a
member per type and per `List`, `termination_by … => sizeOf …`, and a second `mutual` block for the
walk so that the walk is an **independent recursion** rather than `decide (limit < parDepth p)` — a
walk defined as the depth it is checking would make both directions `rfl`, which is the vacuity
`Rchain/Laws.lean` records for law 22.

**The falsifier, when the proof lands**: dropping one arm of the walk's children function must make
the soundness direction false on a concrete term — `a_dropped_arm_breaks_soundness`. The Rust half of
that risk (a `Proc` constructor the walk does not descend into) is not a Lean claim at all: it is
pinned by `rholang/tests`' exhaustive-construct depth test and by the parser's own refusal tests.
-/

namespace Rchain

mutual
  /-- The depth of `p` as a tree of subterms: `1` at a leaf, `1 + max` over its children. The quantity
  `MAX_AST_DEPTH` bounds and every consumer of a term recurses over. -/
  def parDepth : Par → Nat
    | Par.mk s r nw e m u b c =>
        1 + max (listDepthSend s)
              (max (listDepthReceive r)
              (max (listDepthNew nw)
              (max (listDepthExpr e)
              (max (listDepthMatch m)
              (max (listDepthGUnforgeable u)
              (max (listDepthBundle b)
                   (listDepthConnective c)))))))
  termination_by p => sizeOf p

  def sendDepth : Send → Nat
    | Send.mk c d _ => 1 + max (parDepth c) (listDepthPar d)
  termination_by s => sizeOf s

  def receiveBindDepth : ReceiveBind → Nat
    | ReceiveBind.mk ps s _ => 1 + max (listDepthPar ps) (parDepth s)
  termination_by b => sizeOf b

  def receiveDepth : Receive → Nat
    | Receive.mk bs b _ _ => 1 + max (listDepthReceiveBind bs) (parDepth b)
  termination_by r => sizeOf r

  def newDepth : New → Nat
    | New.mk _ b => 1 + parDepth b
  termination_by n => sizeOf n

  def matchCaseDepth : MatchCase → Nat
    | MatchCase.mk p s _ => 1 + max (parDepth p) (parDepth s)
  termination_by m => sizeOf m

  def matchDepth : Match → Nat
    | Match.mk t cs => 1 + max (parDepth t) (listDepthMatchCase cs)
  termination_by m => sizeOf m

  def exprDepth : Expr → Nat
    | Expr.ground _ => 1
    | Expr.evar _ => 1
    | Expr.eneg p => 1 + parDepth p
    | Expr.enot p => 1 + parDepth p
    | Expr.eplus p q => 1 + max (parDepth p) (parDepth q)
    | Expr.eminus p q => 1 + max (parDepth p) (parDepth q)
    | Expr.emult p q => 1 + max (parDepth p) (parDepth q)
    | Expr.ediv p q => 1 + max (parDepth p) (parDepth q)
    | Expr.emod p q => 1 + max (parDepth p) (parDepth q)
    | Expr.elt p q => 1 + max (parDepth p) (parDepth q)
    | Expr.ele p q => 1 + max (parDepth p) (parDepth q)
    | Expr.egt p q => 1 + max (parDepth p) (parDepth q)
    | Expr.ege p q => 1 + max (parDepth p) (parDepth q)
    | Expr.eeq p q => 1 + max (parDepth p) (parDepth q)
    | Expr.eneq p q => 1 + max (parDepth p) (parDepth q)
    | Expr.eand p q => 1 + max (parDepth p) (parDepth q)
    | Expr.eor p q => 1 + max (parDepth p) (parDepth q)
    | Expr.ematches p q => 1 + max (parDepth p) (parDepth q)
    | Expr.eshortand p q => 1 + max (parDepth p) (parDepth q)
    | Expr.eshortor p q => 1 + max (parDepth p) (parDepth q)
    | Expr.elist ps _ => 1 + listDepthPar ps
    | Expr.etuple ps => 1 + listDepthPar ps
    | Expr.eset ps _ => 1 + listDepthPar ps
    | Expr.emap kvs _ => 1 + listDepthPair kvs
    | Expr.ebigint _ => 1
    | Expr.emethod _ p args => 1 + max (parDepth p) (listDepthPar args)
    | Expr.epercentPercent p q => 1 + max (parDepth p) (parDepth q)
    | Expr.eplusPlus p q => 1 + max (parDepth p) (parDepth q)
    | Expr.eminusMinus p q => 1 + max (parDepth p) (parDepth q)
  termination_by e => sizeOf e

  def bundleDepth : Bundle → Nat
    | Bundle.mk p _ _ => 1 + parDepth p
  termination_by b => sizeOf b

  /-- Every `GUnforgeable` is a leaf — no `termination_by` clause, because there is no recursion to
  justify (a clause here is an unused one, and Lean says so). -/
  def gUnforgeableDepth : GUnforgeable → Nat
    | GUnforgeable.gPrivate _ => 1
    | GUnforgeable.gDeployId _ => 1
    | GUnforgeable.gDeployerId => 1
    | GUnforgeable.gSysAuthToken => 1

  def connectiveDepth : Connective → Nat
    | Connective.connAnd ps => 1 + listDepthPar ps
    | Connective.connOr ps => 1 + listDepthPar ps
    | Connective.connNot p => 1 + parDepth p
    | Connective.connVarRef _ _ => 1
  termination_by c => sizeOf c

  def listDepthPar : List Par → Nat
    | [] => 0
    | a :: as => max (parDepth a) (listDepthPar as)
  termination_by l => sizeOf l

  def listDepthPair : List (Par × Par) → Nat
    | [] => 0
    | (a, b) :: as => max (parDepth a) (max (parDepth b) (listDepthPair as))
  termination_by l => sizeOf l

  def listDepthSend : List Send → Nat
    | [] => 0
    | a :: as => max (sendDepth a) (listDepthSend as)
  termination_by l => sizeOf l

  def listDepthReceive : List Receive → Nat
    | [] => 0
    | a :: as => max (receiveDepth a) (listDepthReceive as)
  termination_by l => sizeOf l

  def listDepthReceiveBind : List ReceiveBind → Nat
    | [] => 0
    | a :: as => max (receiveBindDepth a) (listDepthReceiveBind as)
  termination_by l => sizeOf l

  def listDepthNew : List New → Nat
    | [] => 0
    | a :: as => max (newDepth a) (listDepthNew as)
  termination_by l => sizeOf l

  def listDepthMatch : List Match → Nat
    | [] => 0
    | a :: as => max (matchDepth a) (listDepthMatch as)
  termination_by l => sizeOf l

  def listDepthMatchCase : List MatchCase → Nat
    | [] => 0
    | a :: as => max (matchCaseDepth a) (listDepthMatchCase as)
  termination_by l => sizeOf l

  def listDepthExpr : List Expr → Nat
    | [] => 0
    | a :: as => max (exprDepth a) (listDepthExpr as)
  termination_by l => sizeOf l

  def listDepthBundle : List Bundle → Nat
    | [] => 0
    | a :: as => max (bundleDepth a) (listDepthBundle as)
  termination_by l => sizeOf l

  def listDepthGUnforgeable : List GUnforgeable → Nat
    | [] => 0
    | a :: as => max (gUnforgeableDepth a) (listDepthGUnforgeable as)
  termination_by l => sizeOf l

  def listDepthConnective : List Connective → Nat
    | [] => 0
    | a :: as => max (connectiveDepth a) (listDepthConnective as)
  termination_by l => sizeOf l
end

/-- The node's own bound, `rholang/src/parser.rs::MAX_AST_DEPTH`. Kept as a named constant here so a
law statement can speak about it rather than about a literal: the number is the code's choice, and
the law is that the walk and `parDepth` agree about it. -/
def maxAstDepth : Nat := 768

/-- A concrete term at a known depth — the witness shape the owed directions are stated over. A unit
term is depth 1, and wrapping it in `n` unary nots gives depth `n + 1`, which is the arithmetic a
falsifying mutation has to break.

**Why this is a definition and not a `decide`d example** — the trap `spec/STYLE.md` records, met here:
`parDepth` is a `mutual` block over nested lists, so it needs `termination_by`, and a well-founded
definition is **not kernel-reducible** — `decide` cannot evaluate `parDepth (notsDepth 767)`, and the
attempt reports `reduction got stuck`. So law 50's two directions are proofs by induction, not
evaluations, and this term is their witness: `notsDepth n` has depth `n + 1`, so `notsDepth 767`
reaches `maxAstDepth` exactly and `notsDepth 768` is the first term over it. -/
def notsDepth (n : Nat) : Par :=
  Par.mk [] [] [] (List.replicate n (.enot (Par.mk [] [] [] [] [] [] [] []))) [] [] [] []

end Rchain
