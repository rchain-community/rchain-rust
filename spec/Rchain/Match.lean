import Rchain.Ty
import Rchain.FreeVars

/-!
# Laws 5 and 37 — spatial matching, defined

Law 5 used to be three `axiom`s: an *undefined* relation `spatialMatches`, its decidability, and
`pattern_binds_at_most_once` — whose statement (`∀ n m, freeVarOf p n → freeVarOf p m → n = m`, read
against `freeVarOf`'s meaning "level `n` occurs free in `p`") says a pattern has **at most one
variable**, which is false for `{"a": *x, "b": *y}`. As an axiom it made the development *unsound*:
anything follows from a false axiom. Recorded as AUDIT C26.

This module replaces all three with a definition and the theorems that hold of it:

- `spatialMatchCore` / `spatialMatchExprs` / `spatialMatchExpr` / `matchListPar` / `matchMap` — the
  matcher. It carries explicit **fuel** (`parNodes`, a node count over the shapes it traverses) because
  the recursion descends into the *pattern* in some clauses and into the *target* in others, so no
  single argument is structurally decreasing; fuel makes every clause structurally recursive on one
  parameter, which is what lets the kernel evaluate them and `decide` check cases against them — the
  corpus does exactly that, and it is how the Rust is held to this definition;
- `spatialMatches t p := spatialMatch t p = true` — the relation, now a definition, and
  `spatialMatches_decidable` — now an instance rather than an axiom;
- **law 5, correctly stated**: an accepted match binds each free level at most once
  (`spatialMatch_implies_linear`) — the Scala's `addedVars.distinct`, the Rust's `BindPattern::free_count`;
- law 37's tie to law 35 (`concrete_matches_iff_eq`): a pattern with nothing connective in it matches
  exactly the targets structurally equal to it, which is why `spatial_match`'s fast path
  (`if !pattern.connective_used { pattern == target }`) is sound. **Proof owed** — stated here, with its
  content checked by the corpus, and recorded as owed in `spec/INVENTORY.md` row 37.

One thing is owed rather than proven, and is named rather than left implicit: that proof. `fuel_saturation`
— the statement that the fuel is *enough* — is a **theorem** now (the family of depth-indexed bounds
below), so a fuel shortfall is ruled out rather than merely unobserved; the tie's proof is the residue,
and its content is checked by the corpus meanwhile.

**The boundary, stated rather than implied — and now named by a predicate.** The clauses cover:
variables, wildcards, ground values, and `[…]`/`Set(…)`/`{…}`/`(…)` with remainders and nesting — which
is `modelledPar` below, the predicate law 37's tie is stated over. Every other shape answers `false`,
which *fails closed*: the spec claims no match rather than guessing one, and since the corpus asserts
agreement with the node, a shape outside the boundary that the node *does* match shows up as a corpus
failure rather than a silent over-claim.

**A tuple was outside the boundary, and the port matches one** (AUDIT C44). The clauses had no `ETuple`
arm, and `spatial_matcher.rs:496-501` has one, so a tuple pattern that the node matches read as
"silence" in the model — the C19/C20/C22 shape, hidden this time behind the *documented* fact that
unmodelled shapes fail closed. The corpus's cases 15/16 are what found it: a declared verdict of `true`
for `@(1, 2)` against `(1, 2)` made `matchCases_decide` refuse to compile. The arm is added. The
arithmetic arms are the port's too (`:563-573`) and are deliberately **not** modelled: a datum is
evaluated before it is stored, so no reachable target carries one — a `spatialMatch` that said otherwise
would be right about the model and wrong about the node. That gap is why the tie carries `modelledPar`
rather than `connectiveUsed` alone.
-/

namespace Rchain

mutual
  /-- The number of nodes a `Par` presents to the matcher: **one for the `Par` itself**, one per
  expression, one per collection element and per map pair, descending through the collection forms. It
  bounds a match's step count.

  **The `Par` itself used to count as zero, and that was a defect in the measure** (found 2026-09-23,
  Programme D unit 8, while designing `fuel_saturation` rather than while testing). A `Par` whose
  `exprs` field is empty still costs the matcher **one fuel step** when it sits in a collection's
  element list: `matchListPar`'s "same patterns, fewer targets" branch drops one target per step
  (`:220-225` in the definition below), and that walk is not charged to any node the old measure could
  see. The consequence is a `false` where the truth is `true`, on a shape the port matches:

  `@[1, ..._]` against `[Nil, Nil, Nil, Nil, Nil, Nil, 1]` — the pattern's single element has six
  `Nil`s to walk past before it finds its counterpart (the port searches: `list_match` →
  `find_matches`, `spatial_matcher.rs:729-815`). Both terms measured `parNodes = 2`, so
  `matchFuel = 12`, while the walk plus the descent needs **13**. The model answered `false`; the node
  answers `true`. Six `Nil`s is exactly the boundary — at five the old measure answered `true` too, so
  only a *padded* target exposed it, which is why the 17 corpus cases that shipped did not
  (`spec/conformance/match.tsv` row 18 is the case now, and
  `the_walk_past_empty_pars_is_paid_for` below is the ratchet). -/
  def parNodes (p : Par) : Nat :=
    match p with
    | .mk _ _ _ es _ _ _ _ => 1 + parNodesExprs es

  /-- The nodes an expression list presents. -/
  def parNodesExprs (es : List Expr) : Nat :=
    match es with
    | [] => 0
    | e :: rest => parNodesExpr e + parNodesExprs rest

  /-- The nodes an expression presents. **The `etuple` arm was missing, and that was the same defect
  as the `Par`-counts-as-zero one** (found 2026-09-24, while attempting `fuel_saturation`): the tuple
  clause walks its elements through `matchListPos` exactly as the list arm does, but it fell into the
  `_ => 1` catch-all, so a tuple's *contents* were charged to no node and `matchFuel` did not grow with
  them. Every nesting level of a tuple costs the matcher 4 units (core → exprs → expr → listPos, then
  the element) and a `Par`-and-expression pair contributes exactly 4, so the arm is what makes a nested
  tuple *pay for itself* — without it the fuel is constant while the walk deepens, and the matcher
  answers `false` on a shape the node matches: `@( ( (1) ) )` against itself needs 13 and was given 12.
  `a_nested_tuple_is_paid_for` below is the ratchet; the shapes that exposed it (three-deep tuples with
  a wildcard leaf, and nested tuple/list/set/map mixtures) are what a `#eval` search over generated
  shapes found — 40 of 300 generated matching shapes were rejected, and none of them is in the corpus,
  which is why the 19 cases that shipped did not. -/
  def parNodesExpr (e : Expr) : Nat :=
    match e with
    | .elist ps _ => 1 + parNodesListPar ps
    | .eset ps _ => 1 + parNodesListPar ps
    | .etuple ps => 1 + parNodesListPar ps
    | .emap kvs _ => 1 + parNodesPairs kvs
    | _ => 1

  /-- The nodes a list of `Par`s presents. -/
  def parNodesListPar (ps : List Par) : Nat :=
    match ps with
    | [] => 0
    | p :: rest => parNodes p + parNodesListPar rest

  /-- The nodes a map's pairs present. -/
  def parNodesPairs (kvs : List (Par × Par)) : Nat :=
    match kvs with
    | [] => 0
    | (a, b) :: rest => parNodes a + parNodes b + parNodesPairs rest
end

-- Fuel for the matcher. A step consumes at least one node of the two terms, and the first three
-- steps of every match consume one each before a single element comparison happens (core →
-- expressions → expression), so the bound is doubled with a constant margin. It was one short, and
-- the corpus said so: `@[]` against `[]` and `@Set(1, ..._)` against `Set(1, 2)` both answered
-- **false** until this line changed — a fuel shortfall is a silent wrong answer, which is exactly
-- why `fuel_saturation` is a named obligation rather than an assumption.
--
-- **The `termination_by` experiment, run and answered (2026-09-23).** The plan for `fuel_saturation`
-- proposed annotating this block's five members with `termination_by fuel` so the equation compiler
-- would emit usable unfolding lemmas (which is what fixed `flatPar`). It does — and it *costs* the one
-- thing the checkers rely on: a well-founded definition is no longer reduced by the kernel, so
-- `decide` stops closing `spatialMatch arithmeticTuple arithmeticTuple = false` and
-- `matchCases_decide` (the corpus's own checker, 17 cases) alike. Recovering it is a simp-set exercise
-- per site — the fuel argument must be brought into *successor* form before any clause can be selected
-- (`matchFuel t p` is `2 * (… + …) + 4`, which no equation matches until `omega` or a `Nat.succ_pred`
-- rewrite shapes it), and then the `Bool` connectives need their own lemmas. So the annotation trades
-- a kernel-reducible matcher for symbolic unfolding, and the two proofs below start from that trade
-- rather than from the assumption that it is free. Reverted rather than kept, because an annotation
-- whose benefit is a *future* proof and whose cost is two working checkers is a net loss today.
-- **The route to `fuel_saturation`, worked out (2026-09-23, Programme D unit 8).** The obligation is
-- one mutual induction, and the shape is forced by what the definitions reduce to:
--
-- 1. **The family is the matcher's five members, and each is stated with its *own* bound**, because a
--    member is entered one fuel step deeper than the caller: `spatialMatchCore (m+1) t p` reduces to
--    `spatialMatchExprs m pexprs texprs` while `spatialMatchCore m t p` reduces to
--    `spatialMatchExprs (m-1) …`. So the goal that has to be proved for the core *is the next member's
--    statement at a one-shifted bound* — the members' bounds are depth-indexed, which is what the
--    doubling and the constant in `matchFuel` are for: three steps (core → exprs → expr) consume fuel
--    before a single element comparison, and every element then consumes at least one.
-- 2. **The measure is `parNodes`, and each recursive call is on a subterm**, so the side conditions are
--    arithmetic on it: `parNodesExpr e ≤ parNodesExprs (e :: rest)` for an element of an expression list,
--    `parNodes p' ≤ parNodesListPar ps` for an element of a collection, `parNodes a + parNodes b` for a
--    map pair — each provable by the definition alone (a `simp`/`omega` step per case), and each giving
--    the strict slack the next bound needs.
-- 3. **No `termination_by`.** The family is structural on both the fuel and the terms, which is what
--    keeps the kernel reducing it and `decide` checking cases (the reverted experiment above). The
--    induction is over the *terms*, with the fuel universally quantified above each member's bound —
--    the shape `coreSat : ∀ m, matchFuel t p ≤ m → spatialMatchCore (m+1) t p = spatialMatchCore m t p`,
--    and its four siblings with their own bounds.
--
-- What that leaves is the list inductions (`matchListPar` walks the pattern's elements, `matchMap` its
-- pairs) and the per-case arithmetic — volume of a known kind, with the corpus as the behavioural check
-- in the meantime. Recorded here so the next attempt starts from the reduction shapes rather than
-- rediscovering that the bounds must be depth-indexed.
--
-- **The first link, checked; and the second link is where the design has to be got right (same day).**
-- The core's obligation reduces in two steps (`cases m`, the zero case dying on `2 * n + 4 ≤ 0`, then
-- `simp only [spatialMatchCore, parNodes]`) to `spatialMatchExprs (k+1) pexprs texprs =
-- spatialMatchExprs k pexprs texprs` — the next member's statement, because `matchFuel`'s `+4` and its
-- `parNodes` sum are *the same expression* as the invariant at the entry point (`parNodes` of a `Par`
-- is its expression list's count). So the family is the right shape.
--
-- What the fixed schedule `2 * n + (4 - k)` then runs into is the **list member's element call**:
-- `matchListPar (f+1) patterns targets` calls `spatialMatchCore f t p` for an element pair, and the
-- core's constant is *fixed at 4* by `matchFuel`, so the derivation needs
-- `2 * (parNodes t + parNodes p) + 4 ≤ f` from the list's own `2 * (parNodesListPar patterns +
-- parNodesListPar targets) + c ≤ f + 1`. That is `2 * (n_list - n_elem) + c ≥ 5`, and `n_list - n_elem
-- = 0` for a **single-element** list — so `c ≥ 5`, i.e. the constants would have to *grow* down the
-- chain while the caller's hypothesis must *imply* the callee's. The fixed-depth-offset schedule is
-- therefore wrong, and what makes the obligation true is the *doubling*: every nesting level adds at
-- least one node to *each* side (the enclosing collection) and so adds ≥ 4 to `matchFuel` while
-- consuming only ~2 of it. The budget has to be stated as slack that the extra nodes supply — an
-- invariant like `2 * n + c ≤ f` with `c` re-derived per call site, and the single-element case is the
-- tight one to check — rather than as a constant per depth. That is the piece to design first next
-- time; the equality's content is unchanged (the corpus held it even when the constant was one short).
--
-- **Resolved, and the resolution is the measure — which was wrong** (same day, continued). The `c ≥ 5`
-- above was derived with the *old* `parNodes`, and the reason the constants had to grow was that a `Par`
-- with an empty `exprs` field counted as **zero** nodes: for a single-element list `parNodesListPar [p]`
-- was `parNodes p`, so the element's budget and the list's were the same number and the walk could only
-- be paid for out of the constant. That was not a fixed-schedule problem; it was a defect, and it made
-- the matcher wrong on a reachable shape (`parNodes`'s doc comment and
-- `the_walk_past_empty_pars_is_paid_for` below). Counting the `Par` itself makes
-- `parNodesListPar [p] = 1 + parNodes p`, and the element case becomes **exactly tight**: for an
-- element that is a collection, the supply is `2 * (parNodes t + parNodes p)`
-- `= 2 * ((2 + nl_t) + (2 + nl_p))` and the need is `B_core = 3 + B_list(sub) ≤ 2 * (nl_t + nl_p) + 8` —
-- equal, with no slack. So the invariant is `B_list ≤ 2 * (parNodesListPar patterns +
-- parNodesListPar targets) + 5`, with `matchFuel`'s `+4` covering the core → exprs → expr descent and
-- the list's `+5` covering a single-element list's one step. A budget that is tight rather than
-- comfortable is the right thing to have found *here*, while the measure could still be argued about:
-- the `+1` per `Par` is exactly the term that was missing, in the counterexample and in the induction.
def matchFuel (target pattern : Par) : Nat := 2 * (parNodes target + parNodes pattern) + 4

mutual
  /-- `spatialMatchCore fuel target pattern` — does `target` match `pattern` by shape, ignoring the
  linearity `spatialMatch` adds? A variable or wildcard binds anything; a ground value matches itself;
  a collection matches by the rule of its *form* — a list or tuple element-wise in order
  (`matchListPos`), a set or map by search for a distinct counterpart (`matchListPar`, `matchMap`).
  Anything else fails closed (see the boundary note above). -/
  def spatialMatchCore (fuel : Nat) (target pattern : Par) : Bool :=
    match fuel with
    | 0 => false -- under-supplied fuel: the corpus is what would notice (see `fuel_saturation`)
    | f + 1 =>
      match pattern, target with
      | .mk _ _ _ pexprs _ _ _ _, .mk _ _ _ texprs _ _ _ _ => spatialMatchExprs f pexprs texprs

  /-- A pattern's expression list against a target's: a lone pattern expression is the interesting
  case (a variable or wildcard binds the whole datum, a collection matches the lone collection); a
  multi-expression pattern is not modelled and fails closed. -/
  def spatialMatchExprs (fuel : Nat) (patterns : List Expr) (target : List Expr) : Bool :=
    match fuel with
    | 0 => false
    | f + 1 =>
      match patterns with
      | [.evar .wildcard] => true
      | [.evar (.free _)] => true
      | [p] => spatialMatchExpr f p target
      | _ => false

  /-- One pattern expression against a target `Par`'s expressions. -/
  def spatialMatchExpr (fuel : Nat) (pattern : Expr) (target : List Expr) : Bool :=
    match fuel with
    | 0 => false
    | f + 1 =>
      match pattern with
      | .evar .wildcard => true
      | .evar (.free _) => true
      | .evar (.bound _) => false -- `=x` refers to an enclosing binding, substituted before matching
      | .ground g => match target with
        | [.ground g'] => g == g'
        | _ => false
      -- A **list** is matched *positionally*, and this was wrong until AUDIT C48: the clause called
      -- the searcher, which let a pattern element find a counterpart anywhere in the target list. The
      -- port does not search lists — its `EList` arm is `fold_match`
      -- (`spatial_matcher.rs:467-493`, `SpatialMatcher.scala:482`), which pairs `tlist[0]` with
      -- `plist[0]` and recurses on the tails, with the remainder taking the *tail* only. So
      -- `@[1, ..._]` does **not** match `[Nil, 1]`, and `@[1, 3, ..._]` does not match `[1, 2, 3]`;
      -- the model claimed both. Only the *set* and *map* forms search (`list_match_single` →
      -- `find_matches`, the MBM assignment), and that is what the two members below are for.
      | .elist ps r => match target with
        | [.elist ts _] => matchListPos f ps ts r.isSome
        | _ => false
      -- A **tuple** matches element-wise, with no remainder to absorb a tail — the port's arm
      -- (`spatial_matcher.rs:496-501`, `fold_match(tlist, plist, None, …)`) which this clause set did
      -- not have. Its absence was invisible because the model *fails closed* (every unmodelled shape
      -- answers `false`), and a pattern that matches nothing produces silence rather than an error —
      -- the same shape as C19/C20/C22, where a missing clause read as a client bug. AUDIT C44; the
      -- corpus's cases 15/16 (`spec/conformance/match.tsv`) are what found it. It takes the same
      -- *positional* matcher as the list arm for the same reason (`fold_match(tlist, plist, None, …)`).
      | .etuple ps => match target with
        | [.etuple ts] => matchListPos f ps ts false
        | _ => false
      | .eset ps r => match target with
        | [.eset ts _] => matchListPar f ps ts r.isSome
        | _ => false
      | .emap kvs r => match target with
        | [.emap tks _] => matchMap f kvs tks r.isSome
        | _ => false
      | _ => false

  /-- `matchListPos fuel patterns targets absorb` — **lists and tuples**: `patterns`' elements match
  `targets`' **in order**, and when `absorb` is false (the pattern had no remainder) no target may be
  left over. This is the port's `fold_match` (`spatial_matcher.rs:596-629`), which pairs the heads and
  recurses on the tails — there is deliberately **no** branch that drops a target to look further, and
  adding one was the model's defect (AUDIT C48; the `elist` arm's comment above). The absorbed tail is
  not inspected, which is the port's rule for a *wildcard* remainder and is an over-approximation for a
  named one (`fold_match` additionally demands `locally_free_empty` of each absorbed element) — a state
  a stored datum cannot be in, since it carries no free variable. -/
  def matchListPos (fuel : Nat) (patterns : List Par) (targets : List Par) (absorb : Bool) : Bool :=
    match fuel with
    | 0 => false
    | f + 1 =>
      match patterns with
      | [] => absorb || targets.isEmpty
      | p :: ps =>
        match targets with
        | [] => false
        | t :: ts => spatialMatchCore f t p && matchListPos f ps ts absorb

  /-- `matchListPar fuel patterns targets absorb` — **sets and maps**: every pattern element has a
  *distinct* counterpart anywhere among the targets (`matchListPar`'s second branch drops a target to
  look further), and when `absorb` is false no target may be left over. This is the port's
  `list_match_single` → `find_matches` (`spatial_matcher.rs:729-815`), which is a bipartite assignment
  — it searches, because a `ParSet`'s elements are unordered. Applying it to lists as well is AUDIT
  C48: the `ESet` and `EMap` arms are the only two callers. -/
  def matchListPar (fuel : Nat) (patterns : List Par) (targets : List Par) (absorb : Bool) : Bool :=
    match fuel with
    | 0 => false
    | f + 1 =>
      match patterns with
      | [] => absorb || targets.isEmpty
      | p :: ps =>
        match targets with
        | [] => false
        | t :: ts =>
          (spatialMatchCore f t p && matchListPar f ps ts absorb)
          || matchListPar f patterns ts absorb -- same patterns, fewer targets

  /-- As `matchListPar`, over a map's key/value pairs: a pattern pair matches a *distinct* target pair
  only when both halves do. -/
  def matchMap (fuel : Nat) (pairs : List (Par × Par)) (targets : List (Par × Par)) (absorb : Bool) :
      Bool :=
    match fuel with
    | 0 => false
    | f + 1 =>
      match pairs with
      | [] => absorb || targets.isEmpty
      | (k1, k2) :: kvs =>
        match targets with
        | [] => false
        | (t1, t2) :: ts =>
          (spatialMatchCore f t1 k1 && spatialMatchCore f t2 k2 && matchMap f kvs ts absorb)
          || matchMap f pairs ts absorb -- same pairs, fewer targets
end

mutual
  /-- The free levels a pattern mentions, over the shapes the clauses above can match. A level inside
  an unmodelled shape (a send, a `match`) contributes nothing — such a pattern cannot be accepted, so
  the linearity check never has to see inside it. -/
  def freeLevelsOfPar (p : Par) : List Nat :=
    match p with
    | .mk _ _ _ es _ _ _ _ => freeLevelsExprs es

  /-- The levels an expression list mentions. -/
  def freeLevelsExprs (es : List Expr) : List Nat :=
    match es with
    | [] => []
    | e :: rest => freeLevelsExpr e ++ freeLevelsExprs rest

  /-- The levels an expression mentions. -/
  def freeLevelsExpr (e : Expr) : List Nat :=
    match e with
    | .evar (.free n) => [n]
    | .elist ps _ => freeLevelsOfListPar ps
    | .eset ps _ => freeLevelsOfListPar ps
    | .etuple ps => freeLevelsOfListPar ps
    | .emap kvs _ => freeLevelsOfPairs kvs
    | _ => []

  /-- The levels a list of `Par`s mentions. -/
  def freeLevelsOfListPar (ps : List Par) : List Nat :=
    match ps with
    | [] => []
    | p :: rest => freeLevelsOfPar p ++ freeLevelsOfListPar rest

  /-- The levels a map's key/value pairs mention (both halves). -/
  def freeLevelsOfPairs (kvs : List (Par × Par)) : List Nat :=
    match kvs with
    | [] => []
    | (a, b) :: rest => freeLevelsOfPar a ++ freeLevelsOfPar b ++ freeLevelsOfPairs rest
end

/-- `linear p` — no free level occurs twice, so a match of `p` binds each level at most once. -/
def linear (p : Par) : Bool := (freeLevelsOfPar p).Nodup

/-- `spatialMatch target pattern` — the structural clauses plus the linearity law 5 requires.
(`matches` is a reserved identifier in Lean, hence the name.) -/
def spatialMatch (target pattern : Par) : Bool :=
  spatialMatchCore (matchFuel target pattern) target pattern && linear pattern

/-- Law 37: the relation, as a definition — `spatialMatches t p` holds when the matcher accepts. -/
def spatialMatches (target pattern : Par) : Prop := spatialMatch target pattern = true

/-- Law 5's decidability, now an *instance* rather than an axiom: acceptance is computed by a
function, so it is decidable, and there is no silent partiality. (`Decidable` is data, not a
proposition, so this is an instance rather than a theorem.) -/
instance spatialMatches_decidable (target pattern : Par) : Decidable (spatialMatches target pattern) :=
  inferInstanceAs (Decidable (spatialMatch target pattern = true))

/-- Law 5, correctly stated: an accepted match binds each free level at most once. This replaces
`pattern_binds_at_most_once`, which was false as written (AUDIT C26) — the Scala's
`addedVars.distinct`, the Rust's `free_count`. -/
theorem spatialMatch_implies_linear {target pattern : Par} (h : spatialMatch target pattern = true) :
    linear pattern = true := by
  simp [spatialMatch] at h
  exact h.2

/-! ## Law 5, re-stated over the aggregation the port actually checks

`spatialMatch` conjoins `linear pattern`, so `spatialMatch_implies_linear` above holds by construction —
it is `h.2` of a conjunct inside a definition, not a statement about the matcher's clauses. Inside the
**matcher**, linearity is enforced in exactly one place, and the model below is it: `aggregate_updates`
(`rholang/src/matcher/spatial_matcher.rs:644-665`), reached only from the collection path (`list_match`'s
tail, `:800`). The element-pair path (`fold_match`, `:595-629`) and the conjunction path (`ConnAnd`,
`:325-334`) thread their binding maps with **no check at all** — a binding is a plain `insert`
(`:477-480`) — so a level bound twice would be silently overwritten, right-biased. Both halves are
stated, because a law about "bound at most once" that named only the checked path would be the same
mistake this pass exists to remove.

**But no term reaches either path with a twice-bound binder: the normalizer refuses it first.** Probed
on a devnet (AUDIT C42), all three shapes — `for (@[v, v] <- x)`, `for (v <- x & v <- y)`,
`for (@{"k": v, ...v} <- x)` — are rejected with `Free variable v is used twice as a binder … in
process/name context` (`rholang/src/normalizer.rs:111,289,590,1325`), upstream of the matcher; a
duplicated *datum* element is accepted, as it should be. So the matcher's aggregation-path error is
defence-in-depth on a state the front end cannot construct, and these theorems describe the matcher's
modules rather than reachable behaviour. -/

/-- The matcher's binding environment: the free levels bound so far, each to the `Par` it is bound to
    (the port's `FreeMap = BTreeMap<i32, Par>`). A list of pairs, so `decide` can read it. -/
abbrev FreeMap := List (Nat × Par)

/-- Bind a level, replacing any previous binding — the port's `fm.insert(level, …)`
    (`spatial_matcher.rs:477-480`). -/
def freeMapBind (fm : FreeMap) (l : Nat) (v : Par) : FreeMap :=
  (l, v) :: fm.filter (fun p => p.1 ≠ l)

/-- Fold one contributor's bindings in, right-biased — the port's `out.extend(f)`
    (`spatial_matcher.rs:659-663`). **No check happens here**, which is what the theorems below are
    about. -/
def freeMapMerge (fm f : FreeMap) : FreeMap := f.foldl (fun acc p => freeMapBind acc p.1 p.2) fm

/-- The `Par` a level is bound to, if any. -/
def FreeMap.lookup (fm : FreeMap) (l : Nat) : Option Par :=
  (fm.find? (fun p => p.1 == l)).map (fun p => p.2)

/-- The levels a contributor binds that the base does not already bind — the port's `added_vars`
    (`spatial_matcher.rs:645-653`). One difference the code cannot reach: a `BTreeMap`'s keys are unique,
    so a single contributor never repeats a level there, while the model's list can — and would then be
    rejected. The model is stricter only in a state the port cannot produce. -/
def newLevels (fm f : FreeMap) : List Nat :=
  (f.map (fun p => p.1)).filter (fun l => l ∉ fm.map (fun p => p.1))

/-- **The port's linearity check**: `aggregate_updates` (`spatial_matcher.rs:644-665`) returns
    `Err(BugFoundError("Aggregated updates conflicted with each other"))` when two contributors bind the
    same level the base does not already bind — the Rust compares a `BTreeSet` of the added variables
    against the `Vec` it built, so a repeat is exactly `¬ Nodup` — and folds them right-biased otherwise.
    One call site: `list_match`'s tail (`:800`). -/
def aggregateUpdates (fm : FreeMap) (fms : List FreeMap) : Option FreeMap :=
  let added := (fms.map (newLevels fm)).join
  if added.Nodup then some (fms.foldl freeMapMerge fm) else none

/-- **Law 5 on the checked path** — two contributors that both bind a level the base has not bound are
    **rejected**, which is the port's `BugFoundError` (`spatial_matcher.rs:654-658`). -/
theorem aggregateUpdates_rejects_double_bind (fm f g : FreeMap) (l : Nat)
    (hl : l ∉ fm.map (fun p => p.1)) (hf : l ∈ f.map (fun p => p.1))
    (hg : l ∈ g.map (fun p => p.1)) : aggregateUpdates fm [f, g] = none := by
  have hla : l ∈ newLevels fm f := List.mem_filter.mpr ⟨hf, by simpa using hl⟩
  have hlb : l ∈ newLevels fm g := List.mem_filter.mpr ⟨hg, by simpa using hl⟩
  have hdisj : ¬ (newLevels fm f).Disjoint (newLevels fm g) := fun h => h hla hlb
  simp [aggregateUpdates, List.nodup_append, hdisj]

/-- **And this is what the other two paths do instead** — the element-pair and conjunction paths bind
    with no check, so a repeated level is **overwritten**: the later binding wins, whatever the base held
    (`spatial_matcher.rs:477-480`, `:595-629`, `:325-334`). Reached only in the matcher's own terms — a
    parsed term cannot get here, because the normalizer refuses a twice-bound binder first (AUDIT C42). -/
theorem freeMapMerge_overwrites (f : FreeMap) (l : Nat) (v' : Par) :
    (freeMapMerge f [(l, v')]).lookup l = some v' := by
  simp [freeMapMerge, freeMapBind, FreeMap.lookup]

/-! ## The domain of the tie

The shapes the clauses above **cover**: a par whose only non-empty field is `exprs`, every
  expression of which is a ground value or one of the four collection forms, recursively. The tie below
  is stated over this predicate rather than over every par, because a shape outside it *fails closed*
  — every clause that does not apply answers `false` — and a law about "matches exactly the equal
  targets" cannot hold where the only answer is "no clause". An arithmetic pattern is the case: it is
  concrete (`connectiveUsed` is false of it), it equals itself, and no clause matches it, which is the
counterexample below. -/

mutual
  def modelledPar : Par → Bool
    | .mk s r n e m u b c =>
      s.isEmpty && r.isEmpty && n.isEmpty && modelledExprs e && m.isEmpty && u.isEmpty
        && b.isEmpty && c.isEmpty
  def modelledExprs : List Expr → Bool
    | [] => true
    | x :: xs => modelledExpr x && modelledExprs xs
  def modelledExpr : Expr → Bool
    | .ground _ => true
    | .elist ps _ => modelledPars ps
    | .eset ps _ => modelledPars ps
    | .etuple ps => modelledPars ps
    | .emap kvs _ => modelledPairs kvs
    | _ => false
  def modelledPars : List Par → Bool
    | [] => true
    | p :: ps => modelledPar p && modelledPars ps
  def modelledPairs : List (Par × Par) → Bool
    | [] => true
    | (a, b) :: kvs => modelledPar a && modelledPar b && modelledPairs kvs
end

/-- A tuple holding `1 + 2`: concrete, equal to itself, and outside every clause — the term that shows
    why the tie needs `modelledPar` and not just `connectiveUsed`. Its *tuple* half is what the corpus
    found first (case 15 of `spec/conformance/match.tsv`), and the fix there exposed this one: the port
    *does* have the arithmetic arms (`spatial_matcher.rs:563-573`), but a datum is evaluated before it is
    stored, so a target can never carry one — the model's `false` is right about every reachable term
    and the law's quantifier was what was wrong. -/
def oneExpr (e : Expr) : Par := Par.mk [] [] [] [e] [] [] [] []

def arithmeticTuple : Par :=
  oneExpr (.etuple [oneExpr (.eplus (oneExpr (.ground (.int 1))) (oneExpr (.ground (.int 2))))])

/-- **The unrestricted tie was false.** `spatialMatch p p` answers `false` for a concrete pattern while
    `p = p` holds. -/
theorem arithmetic_pattern_refutes_the_unrestricted_tie :
    connectiveUsed arithmeticTuple = false ∧ spatialMatch arithmeticTuple arithmeticTuple = false
      ∧ modelledPar arithmeticTuple = false := by
  refine ⟨?_, ?_, ?_⟩ <;> decide

/-- **Law 37's tie to law 35, and the justification of the Rust's fast path**: a *modelled*, concrete
pattern matches exactly the targets structurally equal to it — so `if !pattern.connective_used
{ pattern == target }` decides the same question the clauses would, for the shapes the clauses cover.

**The domain is part of the statement now.** It read `∀ target pattern, connectiveUsed pattern = false
→ spatialMatch target pattern = (target = pattern)`, which is false: `arithmeticTuple` is concrete and
equal to itself and no clause matches it (`arithmetic_pattern_refutes_the_unrestricted_tie`), and the
`modelledPar` hypotheses are what exclude it. Both sides carry one — the clauses read only the two
`exprs` fields, so a target with a send in it would "match" a pattern it is not equal to.

**Proof owed**: reducing the clauses to equality is an induction over the pattern — its expression list,
the collections' element lists, a map's pairs, the tuples, and the fuel. Its *content* is checked today
by `spec/conformance/match.tsv` (every verdict `decide`d against the definitions above, with the Rust
held to the same cases); the proof itself is recorded as owed in `spec/INVENTORY.md` row 37. -/
axiom concrete_matches_iff_eq (target pattern : Par) (h : connectiveUsed pattern = false)
    (hp : modelledPar pattern = true) (ht : modelledPar target = true) :
    spatialMatch target pattern = (target = pattern)

/-! ## Two ratchets: the fuel's measure, and the form that decides whether a walk is needed

Both theorems below are the *model* half of cases in `spec/conformance/match.tsv`, which is where the
node is held to them. They are theorems rather than remarks so that a change to the measure or to the
clauses has to face them.

The first is the fuel's ratchet: the **set** member searches, so it walks past target elements, and the
measure has to pay for that walk. It is one `Nil` from where the old measure still answered `true` —
five `Nil`s needed exactly the fuel the old measure gave, six needed more — which is why the 17 cases
that shipped, none of them padded, could not have found it.

The second is the *form*'s ratchet: the same padded shape in a **list** must **not** match, because
lists are positional. Read together they say the split is load-bearing in both directions. -/

/-- **The padded search the port accepts**: `@Set(1, ..._)` against `Set(Nil × 6, 1)`. The set member
    searches (`list_match_single` → `find_matches`, `spatial_matcher.rs:729-815`), so it walks past six
    empty pars; the fuel must pay for the walk. With a `Par` counted as zero nodes `matchFuel` was 12
    and the walk needed 13, so the model answered `false` where the node answers `true` — AUDIT C47.
    `spec/conformance/match.tsv` row 18 is this case. -/
theorem the_walk_past_empty_pars_is_paid_for :
    spatialMatch (oneExpr (.eset (List.replicate 6 nilPar ++ [oneExpr (.ground (.int 1))]) none))
        (oneExpr (.eset [oneExpr (.ground (.int 1))] (some .wildcard))) = true := by
  decide

/-- **The same padded shape in a list must not match**: `@[1, ..._]` against `[Nil, 1]`. A list is
    matched positionally (`fold_match`), so a pattern element cannot skip a leading target element —
    the port agrees (`spatial_matcher.rs:467-493`). AUDIT C48; `spec/conformance/match.tsv` row 19. -/
theorem a_list_pattern_cannot_skip_a_target_element :
    spatialMatch (oneExpr (.elist [nilPar, oneExpr (.ground (.int 1))] none))
        (oneExpr (.elist [oneExpr (.ground (.int 1))] (some .wildcard))) = false := by
  decide

/-- **The second instance of the measure defect, on a different arm**: `@((1, 2), (3, 4))` against
    itself. The tuple clause walks its elements (`matchListPos`), so each nesting level costs 4 units
    while contributing 4 to `matchFuel` — but `parNodesExpr` had no `etuple` arm, so a tuple's contents
    were charged to nothing and the fuel did not grow with the nesting. Two levels need 13 and were
    given 12, so the model answered `false` where the node answers `true`. Unlike the `Nil` padding of
    the set case, this shape needs **no** padding to expose it — the smallest nested tuple fails — and
    a `#eval` search over generated shapes found 40 of 300 matching shapes rejected. AUDIT C50;
    `spec/conformance/match.tsv` row 20 is this case. -/
theorem a_nested_tuple_is_paid_for :
    spatialMatch (oneExpr (.etuple [oneExpr (.etuple [oneExpr (.ground (.int 1)), oneExpr (.ground (.int 2))]),
        oneExpr (.etuple [oneExpr (.ground (.int 3)), oneExpr (.ground (.int 4))])]))
      (oneExpr (.etuple [oneExpr (.etuple [oneExpr (.ground (.int 1)), oneExpr (.ground (.int 2))]),
        oneExpr (.etuple [oneExpr (.ground (.int 3)), oneExpr (.ground (.int 4))])])) = true := by
  decide

/-- **A tuple's walk is charged to its own nodes**: the tuple's count is the two `Par` wrappers plus
    its elements' counts, which is what the missing arm denied — under the old measure the left side
    was 2 whatever the elements were, so a nested tuple's walk was free. With the `etuple` arm the
    budget grows by 8 per nesting level against a cost of 4. -/
theorem a_tuple_pays_for_its_own_contents :
    parNodes (oneExpr (.etuple [oneExpr (.ground (.int 1)), oneExpr (.ground (.int 2))]))
      = 2 + parNodes (oneExpr (.ground (.int 1))) + parNodes (oneExpr (.ground (.int 2))) := by
  decide

/-! ## Saturation — the fuel is enough

**The fuel is enough — and that is a theorem now rather than an assumption.** What has to be proved is
not "the walk terminates" but **saturation**: at `matchFuel` and past it, more fuel changes no answer.
A shortfall makes the matcher answer `false`, so a shortfall is a *silent wrong answer* — it was one
three times, and each is a case in `match.tsv` or an AUDIT entry (`@[]`/`[]` and `@Set(1, ..._)`/
`Set(1, 2)` until `matchFuel`'s constant was fixed; the padded target of row 18, C47; the nested tuple
of row 20, C50).

The proof is a `mutual` family with **one member per clause group and a depth-indexed bound each**,
because a member is entered one fuel step deeper than its caller: `spatialMatchCore (m+1) t p` reduces to
`spatialMatchExprs m pexprs texprs`, so the statement the core owes *is the next member's statement at a
one-shifted bound*. The bounds are the measure's arithmetic — `bCore` is `matchFuel` itself, and the
list bound is `2 * (parNodesListPar patterns + parNodesListPar targets) + 5`, which the element call uses
up exactly when the enclosing collection holds one element. The induction is over the *terms*, with the
fuel universally quantified above each member's bound, and the two side conditions are where the measure
earns its keep: the element call needs the element's own `parNodes` to dominate its `matchFuel`, and the
searcher's "same patterns, fewer targets" branch needs one target's `parNodes` to pay for the step. That
is *why* a `Par` counting as zero nodes, or a tuple's contents counting as nothing, breaks the invariant
rather than merely the arithmetic — both defects were found while designing this bound.

The `parNodes` equations below are stated by `rfl`, because the equation lemmas the compiler generates
for that `mutual` block are **not realizable** (`invalid projection ⟨head_ih, tail_ih⟩…`): a nested-mutual
measure is still definitionally reducible — which is what lets `decide` check the corpus against the
clauses — but its auto-generated equations are not usable as rewrite rules. Stating the shapes by `rfl`
is the workaround, and it doubles as documentation: these are the cases the arithmetic unfolds. -/

private theorem parNodes_mk (s r n e m u b c) :
    parNodes (Par.mk s r n e m u b c) = 1 + parNodesExprs e := rfl

private theorem parNodesExprs_nil : parNodesExprs ([] : List Expr) = 0 := rfl

private theorem parNodesExprs_cons (e rest) :
    parNodesExprs (e :: rest) = parNodesExpr e + parNodesExprs rest := rfl

private theorem parNodesExpr_elist (ps r) : parNodesExpr (.elist ps r) = 1 + parNodesListPar ps := rfl

private theorem parNodesExpr_eset (ps r) : parNodesExpr (.eset ps r) = 1 + parNodesListPar ps := rfl

private theorem parNodesExpr_etuple (ps) : parNodesExpr (.etuple ps) = 1 + parNodesListPar ps := rfl

private theorem parNodesExpr_emap (kvs r) : parNodesExpr (.emap kvs r) = 1 + parNodesPairs kvs := rfl

private theorem parNodesListPar_nil : parNodesListPar ([] : List Par) = 0 := rfl

private theorem parNodesListPar_cons (p rest) :
    parNodesListPar (p :: rest) = parNodes p + parNodesListPar rest := rfl

private theorem parNodes_pos (p : Par) : 1 ≤ parNodes p := by
  cases p with | mk s r n e m u b c => rw [parNodes_mk]; omega

private theorem parNodesPairs_cons (a b rest) :
    parNodesPairs ((a, b) :: rest) = parNodes a + parNodes b + parNodesPairs rest := rfl

private def bCore (t p : Par) : Nat := 2 * (parNodes t + parNodes p) + 4

private def bExprs (ps ts : List Expr) : Nat := 2 * (parNodesExprs ps + parNodesExprs ts) + 5

private def bExpr (e : Expr) (ts : List Expr) : Nat := 2 * (parNodesExpr e + parNodesExprs ts) + 4

private def bList (ps ts : List Par) : Nat := 2 * (parNodesListPar ps + parNodesListPar ts) + 5

private def bMap (kvs tks : List (Par × Par)) : Nat := 2 * (parNodesPairs kvs + parNodesPairs tks) + 5

set_option maxHeartbeats 1000000 in
mutual
  theorem coreSat (t p : Par)  (m : Nat) (hm : bCore t p ≤ m) :
      spatialMatchCore (m + 1) t p = spatialMatchCore m t p := by
      cases m with
      | zero => simp only [bCore] at hm; omega
      | succ k =>
        cases t with | mk ts tr tn te tm tu tb tc =>
        cases p with | mk ps pr pn pe pm pu pb pc =>
        simp only [spatialMatchCore]
        exact exprsSat pe te k (by simp only [bCore, parNodes_mk] at hm; simp only [bExprs]; omega)

  termination_by sizeOf t + sizeOf p
  theorem exprsSat (ps ts : List Expr)  (m : Nat) (hm : bExprs ps ts ≤ m) :
      spatialMatchExprs (m + 1) ps ts = spatialMatchExprs m ps ts := by
      cases m with
      | zero => simp only [bExprs] at hm; omega
      | succ k =>
        cases ps with
        | nil => simp only [spatialMatchExprs]
        | cons p rest =>
          cases rest with
          | cons q qs => simp only [spatialMatchExprs]
          | nil =>
            simp only [spatialMatchExprs]
            split
            · rfl
            · rfl
            · rename_i pats' p' h1 h2 heq
              injection heq with hp _
              rw [hp] at hm
              exact exprSat p' ts k (by simp only [bExprs, bExpr, parNodesExprs_cons, parNodesExprs_nil] at hm ⊢; omega)
            · rfl

  termination_by sizeOf ps + sizeOf ts
  theorem exprSat (e : Expr) (ts : List Expr)  (m : Nat) (hm : bExpr e ts ≤ m) :
      spatialMatchExpr (m + 1) e ts = spatialMatchExpr m e ts := by
      cases m with
      | zero => simp only [bExpr] at hm; omega
      | succ k =>
        simp only [spatialMatchExpr]
        split
        · rfl
        · rfl
        · rfl
        · split <;> rfl
        · split
          · refine listPosSat ?_ ?_ ?_ ?_ ?_
            simp only [bExpr, bList, parNodesExprs_cons, parNodesExprs_nil, parNodesExpr_elist,
              parNodesExpr_etuple, parNodesExpr_eset] at hm ⊢; omega
          · rfl
        · split
          · refine listPosSat ?_ ?_ ?_ ?_ ?_
            simp only [bExpr, bList, parNodesExprs_cons, parNodesExprs_nil, parNodesExpr_elist,
              parNodesExpr_etuple, parNodesExpr_eset] at hm ⊢; omega
          · rfl
        · split
          · refine listParSat ?_ ?_ ?_ ?_ ?_
            simp only [bExpr, bList, parNodesExprs_cons, parNodesExprs_nil, parNodesExpr_elist,
              parNodesExpr_etuple, parNodesExpr_eset] at hm ⊢; omega
          · rfl
        · split
          · refine mapSat ?_ ?_ ?_ ?_ ?_
            simp only [bExpr, bMap, parNodesExprs_cons, parNodesExprs_nil, parNodesExpr_emap] at hm ⊢; omega
          · rfl
        · rfl

  termination_by sizeOf e + sizeOf ts
  theorem listPosSat (ps ts : List Par) (absorb : Bool)  (m : Nat) (hm : bList ps ts ≤ m) :
      matchListPos (m + 1) ps ts absorb = matchListPos m ps ts absorb := by
      cases m with
      | zero => simp only [bList] at hm; omega
      | succ k =>
        cases ps with
        | nil => simp only [matchListPos]
        | cons p rest =>
          cases ts with
          | nil => simp only [matchListPos]
          | cons t tl =>
            change (spatialMatchCore (k + 1) t p && matchListPos (k + 1) rest tl absorb)
              = (spatialMatchCore k t p && matchListPos k rest tl absorb)
            have hpt : 1 ≤ parNodes t := parNodes_pos t
            have hpp : 1 ≤ parNodes p := parNodes_pos p
            rw [coreSat t p k (by simp only [bCore, bList, parNodesListPar_cons, parNodesListPar_nil, parNodes_mk] at hm ⊢; omega),
                listPosSat rest tl absorb k (by simp only [bList, parNodesListPar_cons, parNodesListPar_nil, parNodes_mk] at hm ⊢; omega)]

  termination_by sizeOf ps + sizeOf ts
  theorem listParSat (ps ts : List Par) (absorb : Bool)  (m : Nat) (hm : bList ps ts ≤ m) :
      matchListPar (m + 1) ps ts absorb = matchListPar m ps ts absorb := by
      cases m with
      | zero => simp only [bList] at hm; omega
      | succ k =>
        cases ps with
        | nil => simp only [matchListPar]
        | cons p rest =>
          cases ts with
          | nil => simp only [matchListPar]
          | cons t tl =>
            change ((spatialMatchCore (k + 1) t p && matchListPar (k + 1) rest tl absorb)
                || matchListPar (k + 1) (p :: rest) tl absorb)
              = ((spatialMatchCore k t p && matchListPar k rest tl absorb)
                || matchListPar k (p :: rest) tl absorb)
            have hpt : 1 ≤ parNodes t := parNodes_pos t
            have hpp : 1 ≤ parNodes p := parNodes_pos p
            rw [coreSat t p k (by simp only [bCore, bList, parNodesListPar_cons, parNodesListPar_nil, parNodes_mk] at hm ⊢; omega),
                listParSat rest tl absorb k (by simp only [bList, parNodesListPar_cons, parNodesListPar_nil, parNodes_mk] at hm ⊢; omega),
                listParSat (p :: rest) tl absorb k (by simp only [bList, parNodesListPar_cons, parNodesListPar_nil, parNodes_mk] at hm ⊢; omega)]

  termination_by sizeOf ps + sizeOf ts
  theorem mapSat (kvs tks : List (Par × Par)) (absorb : Bool)  (m : Nat) (hm : bMap kvs tks ≤ m) :
      matchMap (m + 1) kvs tks absorb = matchMap m kvs tks absorb := by
      cases m with
      | zero => simp only [bMap] at hm; omega
      | succ k =>
        cases kvs with
        | nil => simp only [matchMap]
        | cons kv rest =>
          cases kv with | mk k1 k2 =>
          cases tks with
          | nil => simp only [matchMap]
          | cons tk tl =>
            cases tk with | mk t1 t2 =>
            change ((spatialMatchCore (k + 1) t1 k1 && spatialMatchCore (k + 1) t2 k2
                && matchMap (k + 1) rest tl absorb)
                || matchMap (k + 1) ((k1, k2) :: rest) tl absorb)
              = ((spatialMatchCore k t1 k1 && spatialMatchCore k t2 k2 && matchMap k rest tl absorb)
                || matchMap k ((k1, k2) :: rest) tl absorb)
            have ht1 : 1 ≤ parNodes t1 := parNodes_pos t1
            have ht2 : 1 ≤ parNodes t2 := parNodes_pos t2
            have hk1 : 1 ≤ parNodes k1 := parNodes_pos k1
            have hk2 : 1 ≤ parNodes k2 := parNodes_pos k2
            rw [coreSat t1 k1 k (by simp only [bCore, bMap, parNodesPairs_cons, parNodes_mk] at hm ⊢; omega),
                coreSat t2 k2 k (by simp only [bCore, bMap, parNodesPairs_cons, parNodes_mk] at hm ⊢; omega),
                mapSat rest tl absorb k (by simp only [bMap, parNodesPairs_cons, parNodes_mk] at hm ⊢; omega),
                mapSat ((k1, k2) :: rest) tl absorb k (by simp only [bMap, parNodesPairs_cons, parNodes_mk] at hm ⊢; omega)]
  termination_by sizeOf kvs + sizeOf tks
end

/-- **The fuel is enough** — saturation: past `matchFuel`, one more unit changes no answer. This is
    `coreSat` at the measure's own bound, and it is the statement the matcher's fuel exists for: it makes
    a shortfall impossible rather than merely absent from the cases anyone tried. -/
theorem fuel_saturation (target pattern : Par) :
    spatialMatchCore (matchFuel target pattern + 1) target pattern
      = spatialMatchCore (matchFuel target pattern) target pattern :=
  coreSat target pattern (matchFuel target pattern) le_rfl

end Rchain
