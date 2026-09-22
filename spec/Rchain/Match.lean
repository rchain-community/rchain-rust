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

Two things are owed rather than proven, and are named rather than left implicit: that proof, and
`fuel_saturation`. A fuel shortfall makes the matcher answer `false`, which the corpus reports as a
disagreement with the node — so both are checked behaviourally in the meantime.

**The boundary, stated rather than implied.** The clauses cover what the corpus and the failing
consumers use: variables, wildcards, ground values, and `[…]`/`Set(…)`/`{…}` with remainders and
nesting. Every other shape answers `false`, which *fails closed*: the spec claims no match rather than
guessing one, and since the corpus asserts agreement with the node, a shape outside the boundary that
the node *does* match shows up as a corpus failure rather than a silent over-claim.
-/

namespace Rchain

mutual
  /-- The number of nodes a `Par` presents to the matcher: one per expression, one per collection
  element and per map pair, descending through the collection forms. It bounds a match's step count. -/
  def parNodes (p : Par) : Nat :=
    match p with
    | .mk _ _ _ es _ _ _ _ => parNodesExprs es

  /-- The nodes an expression list presents. -/
  def parNodesExprs (es : List Expr) : Nat :=
    match es with
    | [] => 0
    | e :: rest => parNodesExpr e + parNodesExprs rest

  /-- The nodes an expression presents. -/
  def parNodesExpr (e : Expr) : Nat :=
    match e with
    | .elist ps _ => 1 + parNodesListPar ps
    | .eset ps _ => 1 + parNodesListPar ps
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
def matchFuel (target pattern : Par) : Nat := 2 * (parNodes target + parNodes pattern) + 4

mutual
  /-- `spatialMatchCore fuel target pattern` — does `target` match `pattern` by shape, ignoring the
  linearity `spatialMatch` adds? A variable or wildcard binds anything; a ground value matches itself;
  a collection matches when every named element has a *distinct* counterpart in the target and the
  remainder rule holds. Anything else fails closed (see the boundary note above). -/
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
      | .elist ps r => match target with
        | [.elist ts _] => matchListPar f ps ts r.isSome
        | _ => false
      | .eset ps r => match target with
        | [.eset ts _] => matchListPar f ps ts r.isSome
        | _ => false
      | .emap kvs r => match target with
        | [.emap tks _] => matchMap f kvs tks r.isSome
        | _ => false
      | _ => false

  /-- `matchListPar fuel patterns targets absorb` — every pattern has a **distinct** counterpart among
  the targets, and when `absorb` is false (the pattern had no remainder) no target may be left over. -/
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

/-- Law 37's tie to law 35, and the justification of the Rust's fast path: a pattern with no
connective, no free variable, no wildcard and no remainder matches exactly the targets structurally
equal to it, so `if !pattern.connective_used { pattern == target }` decides the same question the
clauses would.

**Proof owed**: reducing the clauses to equality is an induction over the pattern — its expression
list, the collections' element lists, a map's pairs, and the remainder rule. Its *content* is checked
today by `spec/conformance/match.tsv` (every verdict `decide`d against the definitions above, with the
Rust held to the same cases); the proof itself is recorded as owed in `spec/INVENTORY.md` row 37. -/
axiom concrete_matches_iff_eq (target pattern : Par) (h : connectiveUsed pattern = false) :
    spatialMatch target pattern = (target = pattern)

/-- The fuel is enough — stated as **saturation**: past `matchFuel`, more fuel changes nothing.

Owed, and checked behaviourally meanwhile: a shortfall makes the matcher answer `false`, and the
corpus reports that as a disagreement rather than passing silently (it did, twice: `refutable` was
false for `@[]`/`[]` and `@Set(1, ..._)`/`Set(1, 2)` until the constant in `matchFuel` was fixed). -/
axiom fuel_saturation (target pattern : Par) :
    spatialMatchCore (matchFuel target pattern + 1) target pattern
      = spatialMatchCore (matchFuel target pattern) target pattern

end Rchain
