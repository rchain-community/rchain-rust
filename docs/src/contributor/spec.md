# Formal specification & audit

The mathematical specification of the node lives in the [`spec/`](../../../spec/) tree, outside this
book. This page is an index to it; it does not duplicate its contents.

| Document | What it is |
|---|---|
| [`spec/RHO-CALCULUS.md`](../../../spec/RHO-CALCULUS.md) | The ρ-calculus core: the reflective sorted grammar (`Name = @Proc`, `Proc = *Name | …`), the `PSort` judgment, the flat 8-field `Par`, and the refinements. |
| [`spec/INVENTORY.md`](../../../spec/INVENTORY.md) | The **29-law invariant catalog** — one row per law, with source-of-truth, Rust realization, Lean/Coq target, and status. |
| [`spec/TYPE-SYSTEM.md`](../../../spec/TYPE-SYSTEM.md) | The ρ→CoC type discipline: totality (`TotalOn`), refinement sigma-types, and the "no silent partiality" guarantee. |
| [`spec/AUDIT.md`](../../../spec/AUDIT.md) | The adversarial audit findings register and the Scala-deviation register. |
| [`spec/TEST-COVERAGE.md`](../../../spec/TEST-COVERAGE.md) | The **test-coverage register**: the per-crate inventory, the 29-law property matrix, the risk tiers, and the exempt-module table. Machine-checked by [`tools/audit-test-register.sh`](../../../tools/audit-test-register.sh) (`make check-register`), which fails on an overstated count, a named test that does not exist, or a source file with neither a test nor an exemption row. |
| [`spec/RUST-FIRST.md`](../../../spec/RUST-FIRST.md) | The native-system-contract design: which parts of the node are Rust-native rather than ports, and the simplifications each carries. |
| [`spec/RUST-VS-SCALA.md`](../../../spec/RUST-VS-SCALA.md) | How the rewrite makes the Scala's fragile patterns explicit, and where it deliberately deviates. |
| [`spec/Rchain/`](../../../spec/Rchain/) (Lean 4) | The machine-checked definitions and theorems (`lake build`). |
| [`spec/coq/`](../../../spec/coq/) (Coq) | The substitution / α-equivalence metatheory (`make`). |

The executable semantics of the language are the K-framework rules under
[`legacy/rholang/src/main/k/rholang/`](../../../legacy/rholang/src/main/k/rholang/) — the operational
definition of Laws 2–6.

The machine gate for the port's type-system conformance is
[`tools/audit-type-system.sh`](../../../tools/audit-type-system.sh), which fails the build on any
production `panic!`/`unsafe`/silent-conversion in the Rust crates.
