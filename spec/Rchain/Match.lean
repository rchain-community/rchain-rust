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
