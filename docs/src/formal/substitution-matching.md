# Substitution and matching

Two operations make COMM actually *do* something: **substitution** (Law 3) moves the sent data into
the receive body, and **spatial matching** (Law 5) decides whether a message fits a pattern.

## Substitution (Law 3)

A comm substitutes the sent data for the receive's bound variable. That substitution must be
**capture-avoiding** and respect de Bruijn levels: renaming bound variables so a free variable of the
substituted term is never captured by a binder it lands under.

The minimal substitution the type system needs is in
[`spec/Rchain/Ty.lean`](../../../spec/Rchain/Ty.lean) (`subst`, `substExpr`, `substListExpr`), and it is
proven **sort-preserving**:

```lean
theorem subst_classify (σ : Subst) (p : Par) : classify (subst σ p) = classify p
theorem subst_preserves_sort (σ : Subst) {t : Par} {s : PSort} (h : HasSort t s) : HasSort (subst σ t) s
```

The **deep** capture-avoiding substitution is *defined* in
[`spec/Rchain/Subst.lean`](../../../spec/Rchain/Subst.lean) — a `mutual` family mirroring
`rholang/src/substitute.rs` — and both of its laws are theorems:

```lean
theorem sort_subst (σ : Var → Par) (t : Par) :
    sortPar (substPar σ 0 t) = sortPar (substPar σ 0 (sortPar t))
theorem subst_closed (σ : Var → Par) (t : Par)
    (hσ : ∀ v, Closed (σ v)) (h : Closed t) : Closed (substPar σ t)
```

`sort_subst` is the exact Law-3 statement: **canonicalization commutes with substitution**
(`sort(subst t) = subst(sort t)`) — with `sortPar` on both sides because the file's `substPar` is the
*no-sort* core the port's `substitute_par_no_sort` is. It is the depth-`0` instance of
`sortPar_subst`, one of the 22 members of the `mutual` block that states the law one type at a time.
`subst_closed` is the companion guarantee that substitution does not introduce free variables, and it
carries the **closed-image hypothesis** without which it is false (`Closed` counts a *bound*
occurrence as closed, so an open image escapes into a closed term); the Rust's own test carries the
same hypothesis.

The Coq track owns the *definition* of capture-avoiding de Bruijn substitution (Autosubst-style):
`substPar` and `subst_commutes_sort` in [`spec/coq/Laws.v`](../../../spec/coq/Laws.v). The K executable
form is `free.k` (the free-variable function) together with the substitution module it references.

## Spatial matching (Laws 5, 37, 38)

Matching walks a pattern and a message together. **The matcher is a definition, not a postulate** —
and that is a correction worth reading about. This page used to show three axioms:

```lean
-- NO LONGER EXISTS (AUDIT C26): the middle one was FALSE as written,
-- `∀ n m, freeVarOf p n → freeVarOf p m → n = m` — a pattern like `{"a": *x, "b": *y}`
-- binds two levels, so by ex falso anything followed from it.
def BindsAtMostOnce (p : Par) : Prop := …
axiom spatialMatches : Par → Par → Prop
axiom spatialMatches_decidable (target pattern : Par) : Decidable (spatialMatches target pattern)
```

What is there now ([`spec/Rchain/Match.lean`](../../../spec/Rchain/Match.lean)):

```lean
def spatialMatchCore (fuel : Nat) (target pattern : Par) : Bool   -- the matcher, with fuel
def spatialMatch (target pattern : Par) : Bool
def spatialMatches (target pattern : Par) : Prop := spatialMatch target pattern = true
instance spatialMatches_decidable (target pattern : Par) : Decidable (spatialMatches target pattern)
theorem spatialMatch_implies_linear {target pattern : Par}
    (h : spatialMatch target pattern = true) : …               -- Law 5, correctly stated
axiom concrete_matches_iff_eq (target pattern : Par)
    (h : connectiveUsed pattern = false)
    (hp : modelledPar pattern = true) (ht : modelledPar target = true) :
    spatialMatch target pattern = (target = pattern)           -- Law 37↔35, owed
axiom fuel_saturation (target pattern : Par) : …               -- owed
```

Three things follow, and each is the point of a row:

- **Decidability is an instance, not an axiom** — the matcher *computes* match-or-no-match, so the
  totality guarantee is a fact about a function rather than a postulate.
- **Linearity is a theorem** (`spatialMatch_implies_linear`), stated of a matcher that exists. The
  invariant itself — a pattern binds each free level at most once — is `addedVars.distinct` in the
  Scala oracle and the `freeCount` fields of `BindPattern`/`ReceiveBind`/`MatchCase` in Rust.
- **The matcher carries fuel**, which is what makes the corpus's verdicts `decide`-able, and it is why
  the two axioms above are owed rather than proved: saturation past `matchFuel` is a real induction
  over the matcher's five mutually recursive functions.

Law 37 is *soundness and completeness* — the result set equals the relation's, with partial collections,
wildcards and remainders included (`spec/conformance/match.tsv`, `rholang/tests/lean_match_corpus.rs`);
`modelledPar` is the domain the tie is stated over — the shapes the clauses cover. It is not
decoration: an arithmetic pattern is connective-free and equal to itself and no clause matches it, so
the tie without it was **false** (`arithmetic_pattern_refutes_the_unrestricted_tie`, AUDIT C44), and the
same audit found the `ETuple` clause missing from a matcher the port has one for.

`concrete_matches_iff_eq` is the half that justifies the port's fast path
(`if !pattern.connective_used { pattern == target }`), which is why the two are one row. Law 38 is what
happens when nothing matches: **no step and no error** ([Laws 30–43](laws-30-43.md)).

The K executable form splits matching into several rules: `matching-function.k` (the general arity
matcher), `specific-matching-rules.k` (variable binding and substitution), `exact-matching-function.k`
(the "look through the looking glass once" exact match for patterns within patterns), and
`matching-with-par.k` (matching a parallel composition greedily).

## How they combine in a COMM

A comm is, in one line:

```
send chan!(x) | receive for(pat ← chan){ body }   ⟶   body[ x / pat ]
```

1. **Match** `pat` against `x` (Law 5): if they don't fit, no comm — and **nothing else happens
   either**: no step, no error, no log line (Law 38). That silence is correct behaviour and it is why
   a broken pattern is indistinguishable from a pattern that legitimately matched nothing, which is
   how AUDIT C9–C22's defects stayed invisible.
2. **Bind** the pattern's free variables to the matched sub-terms.
3. **Substitute** (Law 3) those bindings into `body`, capture-avoiding.

The result is `body` with the message woven in — the single primitive that the whole language is built
on.

> Next: the guarantee that none of this can go wrong — [Closedness and the Calculus of
> Constructions](closedness-coc.md).
