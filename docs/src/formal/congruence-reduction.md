# Structural congruence and reduction

Two relations define the *dynamics* of the ρ-calculus: **structural congruence** `≡` (which names
terms that are the same up to reordering and identity) and **reduction** `⟶` (which names the single
step of computation). Both are defined in [`spec/Rchain/Rho.lean`](../../../spec/Rchain/Rho.lean).

## Structural congruence `≡` (Law 2, core)

`≡` is the smallest equivalence relation closed under:

```
refl   : p ≡ p
symm   : p ≡ q  →  q ≡ p
trans  : p ≡ q  →  q ≡ r  →  p ≡ r
comm   : p | q  ≡  q | p
assoc  : (p|q)|r  ≡  p|(q|r)
ident  : p | Nil  ≡  p
par    : p ≡ p'  →  q ≡ q'  →  p|q ≡ p'|q'
```

In Lean these are the constructors of `StrCong`; the equivalence theorem `strCong_equivalence` is
proven, as are `strCong_comm`, `strCong_assoc`, `strCong_ident`, and `strCong_nil_left`. This is the
"par order + `| Nil` + associativity + congruence" fragment of Law 2. The *full* Law 2 (deep
α-equivalence plus `@`/`*`) is **not a second obligation**: with de Bruijn *levels* the representation is
canonical, so deep α is equality, and what is left is this structural congruence. `spec/coq/Laws.v`'s
`alpha_equiv` is an `Inductive` mirroring `StrCong` — refl/symm/trans are constructors, not axioms —
which is the same statement, with a weight invariant and a non-vacuity witness behind it.

The executable form is `name-equivalence.k` (names equivalent up to par order, `| Nil`, top-level
arithmetic, α, and added `@`/`*`).

## Reduction `⟶` (Law 4, core)

`⟶` has one axiom — **COMM** — and two congruence rules:

```
comm     : Name!(x) | for(Name ← …){P}   ⟶   P[subst]        -- a send meets a receive
parLeft  : p ⟶ p'  →  p | q  ⟶  p' | q
parRight : q ⟶ q'  →  p | q  ⟶  p | q'
```

In Lean:

```lean
inductive Reduce : Par → Par → Prop where
  | comm (chan data body : Par) :
      Reduce (parMerge (sendPar chan [data]) (receivePar chan body)) body
  | parLeft {p p' q : Par} : Reduce p p' → Reduce (parMerge p q) (parMerge p' q)
  | parRight {p q q' : Par} : Reduce q q' → Reduce (parMerge p q) (parMerge p q')
```

The `comm` rule says: a send of `data` on `chan` composed with a receive on `chan` with body `body`
reduces to `body` (with `data` substituted for the bound variable — the capture-avoiding substitution
of Law 3). The two `par` rules say reduction happens anywhere inside a parallel composition.

The K executable form is `processes-semantics.k` (`*@P ⇒ P`, `@*C ⇒ C`, parallel spawn) and
`sending-receiving.k` (sends → out-cells, receives → in-cells, paired into a comm event).

## The full Law 4

The *full* Law 4 adds three clauses beyond this core, stated (not yet proven) in
[`spec/Rchain/Reduce.lean`](../../../spec/Rchain/Reduce.lean):

- **Determinism (first-match-wins)** — *withdrawn*: the flat `Par` is **not** single-step deterministic
  up to `≡` (`Rchain.Concurrent.reduce_not_deterministic`). What holds is that an *isolated* redex
  reduces uniquely up to `≡` (`Rchain.Concurrent.reduce_redex_unique`), and full confluence is recovered
  only in the tree model (`Rchain.Tree.reduceT_confluent`). In the node, determinism is supplied by the
  **chosen schedule** (DFS + content-sorted first-match-wins, Laws 1/4/8), not by the raw relation.
- **`new` freshness** — `reduce_freeVars_subset`: reduction never introduces a free variable
  (`freeVars q ⊆ freeVars p`).
- **Replication** — `!P` re-inserts the redex after a comm, and persistent send/receive (`!!`/`<=`)
  are matched without being absorbed (the K rule `persistent-sending-receiving.k`).

## What these guarantee

`⟶` is **not** single-step deterministic up to `≡`: the flat `Par` is not confluent (a term with one
receive and two sends on one channel is a redex in two ways — see
`Rchain.Concurrent.reduce_not_deterministic`). What *does* hold is that an isolated redex reduces uniquely
up to `≡` (`Rchain.Concurrent.reduce_redex_unique`), and confluence is recovered only in the tree model
(`Rchain.Tree.reduceT_confluent`). Determinism in the node is therefore a property of the **chosen
schedule** — the sequential reducer's canonical order (DFS + content-sorted first-match-wins, Laws 1/4/8),
which the concurrent scheduler linearizes to — not of the raw relation. That is the property a
blockchain's consensus depends on: every node computes the same state transition from the same deploy.

> Next: reducing independent sub-processes simultaneously — [Concurrent reduction](concurrent-reduction.md).
