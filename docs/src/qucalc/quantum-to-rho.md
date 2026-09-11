# Quantum operators → the ρ-calculus & rholang

This page answers two questions that come up together: **why** put quantum
operators into RChain at all, and **how** the quantum objects translate into the
ρ-calculus and, concretely, into rholang. It is the bridge document between the
[overview](README.md) and the [architecture](architecture.md) deep-dive.

## 1. The justification

RChain's substrate is the **ρ-calculus** — a higher-order process calculus in
which *names are quoted processes* and the only primitive interaction is
communication on unforgeable channels. QuCalc is not a physics demo bolted onto
the node; it is the ρ-calculus wearing quantum clothes, for three reasons:

### 1.1 ZFA is a capability-security model (Curry–Howard for capabilities)

Zero Free Action (ZFA) says: **a valid process is one whose phase excursions
cancel — a "closed" computation with no free action left over.** In the
ρ-calculus, the corresponding notion is *capability*: an unforgeable name whose
possession *is* authorization, and a well-formed process is one with no
unbound/free names. The two coincide:

| Quantum (ZFA) | ρ-calculus |
|---|---|
| twist history | process (a quoted term) |
| ZFA-balanced closure (Pauli-closed ∧ count-balanced) | a *closed* process — no free variables |
| possessing a fluxoid (capability token) | possessing the unforgeable name |
| free action that fails to cancel | an open term that cannot be reduced |

So "is this proof valid?" and "is this process well-formed?" are the *same*
question — which is exactly what you want for on-chain, capability-secured
computation.

### 1.2 Determinism — the non-negotiable

Every peer must reproduce the *identical* result from the same inputs, or
consensus breaks. The ZFA arithmetic is therefore **exact integer complex**
(entries in {−1, 0, 1}) — never floating point, never `rand`, never wall-clock.
A Pauli matrix product is a finite fold over an 8-symbol alphabet; a floating-
point or randomized implementation would be un-replayable. This is the same
discipline the rest of the node enforces (see [the determinism guarantees](architecture.md#determinism--replay-guarantees)).

### 1.3 Native, metered, unforgeable — not an oracle

`rho:qucalc:*` and `rho:gov:*` are installed as **system contracts**, executed
exactly like `rho:io:stdout` or `rho:registry:lookup`: gas-metered,
deterministic, and bound to the unforgeable `*deployerId`. There is no external
AI oracle and no trusted third party — the proof primitive and the governance
arithmetic run *inside* the node.

Together these give RChain **native AI** (a capability-based proof primitive) and
**group support** (liquid democracy + liquid trust), on a substrate that is a
pluggable basis for other AI systems (see the [overview](README.md)).

## 2. The translation: quantum operators → ρ-calculus → rholang

### 2.1 The 8-twist alphabet

Each quantum operator is one symbol in an 8-symbol alphabet, a Pauli generator or
±identity. In rholang it is simply an `Int` in `0..7`:

| twist | symbol | Pauli | rholang value |
|---|---|---|---|
| 0 | `^` | +σ_y | `0` |
| 1 | `v` | −σ_y | `1` |
| 2 | `>` | +σ_x | `2` |
| 3 | `<` | −σ_x | `3` |
| 4 | `/` | +σ_z | `4` |
| 5 | `\` | −σ_z | `5` |
| 6 | `+` | +I | `6` |
| 7 | `-` | −I | `7` |

A *history* is a sequence of twists — in rholang, a `List[Int]`. This is the
wire form the system processes accept (`parse_twists`).

### 2.2 Composition = the matrix fold = process reduction

Multiplying the 2×2 Pauli matrices left-to-right is a **fold** (`pauli_fold`) —
and a fold over a sequence is precisely sequential composition in the
ρ-calculus. Because the arithmetic is exact integer complex, the fold is a total,
deterministic function from `List[Int]` to a 2×2 complex matrix.

### 2.3 ZFA closure = a closed ρ-calculus term = a valid name

A history is **Pauli-closed** when the fold lands in the scalar group
{±I, ±iI} (`pauli_phase`), and **count-balanced** when even and odd twists are
equally numerous (`count_balanced`). Both together is **ZFA**
(`achieves_zfa`):

```
achieves_zfa(h)  =  pauli_closed(h) ∧ count_balanced(h)
```

A ZFA-balanced history is a *fluxoid* — the quantum analogue of a closed process.
This is the predicate the `rho:qucalc:zfa` system contract exposes:

```
rho:qucalc:zfa(twists, ret)   →   (zfa: Bool, phase: Int)
```

### 2.4 Capabilities = content-addressed unforgeable names

A ZFA proof is *minted* as a capability — in ρ-calculus terms, an unforgeable
name; in rholang terms, a **content-addressed registry URI**
(`rho:id:` + z-base-32 of `blake2b256` of the twist list). Possession of the URI
is possession of the capability, and the value is persisted in the registry so
the proof survives across deploys:

```
rho:qucalc:grant(twists, ret)    →   capUri | Nil      (only if ZFA)
rho:qucalc:verify(cap, ret)      →   Bool              (re-check across deploys)
```

### 2.5 Dialectical synthesis = reduction (COMM-like)

The Aristotle syllogism is **Blanket Fusion**: two premises `S+` and `-P` share a
middle term `+-`; concatenating them and annihilating the shared gauge pair is
the synthesis `S P` — the same "consumes two terms, emits one" shape as a
ρ-calculus reduction. The middle term sits exactly at the premise seam, so the
residue is the subject followed by the predicate:

```
S + M    premise₁ (subject ⊕ middle⁺)
- + P    premise₂ (middle⁻ ⊕ predicate)
  ⋮
S P      synthesis — a stable ZFA fluxoid
```

This is `dialectical_synthesis`, exposed as:

```
rho:qucalc:fuse(subject, predicate, ret)   →   (geometry, capUri) | Nil
```

### 2.6 Multiplicity = the merge monoid (Law 9)

Quantum superpositions carry a *ways* coefficient — a closure class with N ways
is **one term**, not N terms. In the ρ-calculus this is the **merge monoid**
(Law 9 of the [25 laws](../formal/the-25-laws.md)): identical terms collapse into
a term plus a multiplicity, never duplicated. Merging two 600M-way terms is one
integer add, not 600M term copies — the same invariant that keeps the tuple
space canonical.

### 2.7 Summary table

| Quantum operator / concept | ρ-calculus | rholang |
|---|---|---|
| Pauli generator (±σ, ±I) | — (atomic symbol) | `Int` 0..7 |
| twist history | process (quoted term) | `List[Int]` |
| matrix fold | sequential composition | `qucalc::pauli_fold` |
| Pauli-closed ∧ count-balanced (ZFA) | closed term (no free names) | `qucalc::achieves_zfa` |
| fluxoid / proof | capability (unforgeable name) | registry URI `rho:id:…` |
| mint / verify | name creation / lookup | `rho:qucalc:grant` / `rho:qucalc:verify` |
| Blanket Fusion (synthesis) | reduction (COMM) | `rho:qucalc:fuse` |
| `ways` multiplicity | merge monoid (Law 9) | `qucalc::WeightedClass.ways` |
| unforgeable identity | `*deployerId` | `RhoDeployerId` |

## 3. Where the pieces live

- Pure core (no I/O, no non-determinism): [`qucalc/src/lib.rs`](../../../qucalc/src/lib.rs)
  — `pauli_fold`, `pauli_phase`, `count_balanced`, `achieves_zfa`,
  `dialectical_synthesis`, and the `gov` folds.
- System-contract wiring: [`rholang/src/system_processes.rs`](../../../rholang/src/system_processes.rs)
  — the `rho:qucalc:*` / `rho:gov:*` handlers and wire-form parsing.
- Rholang libraries: [`qucalc.rho`](../../../qucalc/rholang/qucalc.rho) and
  [`gov.rho`](../../../qucalc/rholang/gov.rho).
