# Formal specification & audit

The mathematical specification of the node lives in the [`spec/`](../../../spec/) tree, outside this
book. This page is an index to it; it does not duplicate its contents.

| Document | What it is |
|---|---|
| [`spec/RHO-CALCULUS.md`](../../../spec/RHO-CALCULUS.md) | The ρ-calculus core: the reflective sorted grammar (`Name = @Proc`, `Proc = *Name | …`), the `PSort` judgment, the flat 8-field `Par`, and the refinements. |
| [`spec/INVENTORY.md`](../../../spec/INVENTORY.md) | The **law catalog**: the calculus laws covering the surface a client writes (grammar, lexing, normalization, matching, reply shapes, JSON), 4 for the Proof-of-Stake epoch (the boundary gate, the reward split, its conservation, and withdrawal staging), one for the fee consequence of a denied deploy (an open design question, not a settled behaviour), and one for what a matched deploy is charged (the storage refund, and why its order is the rule). One row per law, with source-of-truth, Rust realization, Lean/Coq target, status, and — for rows 30+ — the corpus and the consumer. |
| [`spec/conformance/`](../../../spec/conformance/) | The **conformance corpora**: one TSV per layer, *emitted* from the Lean definitions (`lake exe rchain-corpus --layer <l>`), committed, and read by a Rust consumer, one per corpus, that runs the same cases through the node and must agree 1:1. This is the artifact that binds the specification to the code — and the reason a definition cannot drift from its corpus without the gate noticing. |
| [`spec/API-SCHEMA.md`](../../../spec/API-SCHEMA.md) | The **response-shape contract**: what rholang code and HTTP clients receive, with the reply-channel rule and the `RhoExpr` wire shape. Its rows are held to the checked catalog by law 39's doc tie, not by convention. |
| [`spec/GENESIS.md`](../../../spec/GENESIS.md) | The genesis manifest: the constants a client hardcodes, the ceremony key arrangement, and the consumer evidence for each entry. |
| [`spec/TYPE-SYSTEM.md`](../../../spec/TYPE-SYSTEM.md) | The ρ→CoC type discipline: totality (`TotalOn`), refinement sigma-types, and the "no silent partiality" guarantee. |
| [`spec/AUDIT.md`](../../../spec/AUDIT.md) | The adversarial audit findings register and the Scala-deviation register. |
| [`spec/TEST-COVERAGE.md`](../../../spec/TEST-COVERAGE.md) | The **test-coverage register**: the per-crate inventory, the law matrix, the risk tiers, and the exempt-module table. Machine-checked by [`tools/audit-test-register.sh`](../../../tools/audit-test-register.sh) (`make check-register`), which fails on an overstated count, a named test that does not exist, a source file with neither a test nor an exemption row, **or a law row claiming coverage without naming a Lean module, a corpus and a consumer that exist**. |
| [`spec/RUST-FIRST.md`](../../../spec/RUST-FIRST.md) | The native-system-contract design: which parts of the node are Rust-native rather than ports, and the simplifications each carries. |
| [`spec/RUST-VS-SCALA.md`](../../../spec/RUST-VS-SCALA.md) | How the rewrite makes the Scala's fragile patterns explicit, and where it deliberately deviates. |
| [`spec/Rchain/`](../../../spec/Rchain/) (Lean 4) | The machine-checked definitions and theorems (`lake build`). |
| [`spec/coq/`](../../../spec/coq/) (Coq) | The substitution / α-equivalence metatheory (`make`). |

The executable semantics of the language are the K-framework rules under
[`legacy/rholang/src/main/k/rholang/`](../../../legacy/rholang/src/main/k/rholang/) — the operational
definition of Laws 2–6.

## The three gates

| Gate | What it refuses | Runs as |
|---|---|---|
| [`tools/check-lean-conformance.sh`](../../../tools/check-lean-conformance.sh) | a failed Lean or Coq build, a `sorry`/`admit`, a module `Rchain.lean` does not import, a **stale or untracked corpus**, a corpus with no consumer, a consumer that disagrees with its corpus, and a law-39 catalog urn with no row in `spec/API-SCHEMA.md` | `make check-lean`, and the `formal` job in CI |
| [`tools/audit-type-system.sh`](../../../tools/audit-type-system.sh) | any production `panic!`/`unsafe`/silent conversion in the Rust crates — the no-silent-partiality discipline | `coverage.yml` |
| [`tools/audit-test-register.sh`](../../../tools/audit-test-register.sh) | a register that overstates the tree, a named test that does not exist, a source file with neither a test nor an exemption, and a law row that claims coverage without evidence | `make check-register`, and `coverage.yml` |

**A change to a law is a change to three files**: the Lean definition, the corpus emitted from it, and
the Rust consumer. The gate is what makes that true rather than aspirational — `make spec` is only the
Lean half, and a corpus left stale fails the gate.

`spec/AUDIT.md` §20 is the reader's index back the other way: every incident the audit records, mapped
to the law that now covers it and the case that fails if the behaviour returns.
