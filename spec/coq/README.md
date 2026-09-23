# RChain formal specification — Coq track (Phase 0)

The Coq formalization, parallel to the Lean 4 track in [`../`](../). Coq is the home of the
**substitution / α-equivalence / programming-language metatheory** (capture-avoiding de Bruijn
substitution, α-equivalence), while Lean 4 covers the algebraic/order laws. The two tracks define the
same core `Proc` syntax and the same canonicalization `sort`, so their Phase-0 skeletons are
structurally identical.

## Building

Requires **Coq 8.x** (the files use only the standard library — `Arith`, `ZArith`, `String`, `List`;
no `Autosubst`/`ssreflect` yet).

```sh
make                 # = coq_makefile -f _CoqProject -o CoqMakefile && make -f CoqMakefile
```

Install Coq if needed: `opam install coq` (or `apt install coq`).

## Status

`Syntax.v` defines the flat `Par` ADT; `Sort.v` declares the canonical order as **axioms** (`cmpPar`,
`sortPar`, `sortPar_idempotent`, `sortPar_comm`). `Laws.v` states Laws 2–6 (`alpha_equiv`, `substPar`
with `subst_commutes_sort`, `reduce`, `spatial_matches` + `binds_at_most_once`, `closed` +
`closed_decidable`) as **axioms** — Coq owns the *definitions* (capture-avoiding de Bruijn
substitution, α-equivalence, Autosubst-style), which remain the Phase-1 obligations.

This is **no longer a mirror of the Lean track**, and the difference is the part worth stating: Lean's
law 1a (`sortPar_idempotent`/`sortPar_comm`) is *proved*, and its `substPar`, `spatialMatch` and
`freeVarOf` are *definitions*, not axioms; the comparison this paragraph used to draw was against a
Lean it no longer describes. See [`../LAWS.md`](../LAWS.md) for what the Lean half actually rests on —
the register is emitted from `Rchain/Laws.lean` and refused stale by the gate.

## Mapping to the Scala source of truth

| Coq | Scala |
|-----|-------|
| `score` / `cmpProc` / `parPair` | `models/.../rholang/sorter/ScoreTree.scala`, `ordering.scala` |
| `sort` | `models/.../rholang/sorter/ordering.scala` |
| `Proc` (binary Phase-0 form) | `models/src/main/protobuf/RhoTypes.proto` (`Par`/`Send`/`Receive`/`New`/`Match`) |

See [`../INVENTORY.md`](../INVENTORY.md) for the full catalog (<!-- counts:laws-entries -->49 laws and 58 entries<!-- counts:end -->).
