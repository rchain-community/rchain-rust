# Formal specification & audit

The mathematical specification of the node lives in the [`spec/`](../../../spec/) tree, outside this
book. This page is an index to it; it does not duplicate its contents.

| Document | What it is |
|---|---|
| [`spec/RHO-CALCULUS.md`](../../../spec/RHO-CALCULUS.md) | The ρ-calculus core: the reflective sorted grammar (`Name = @Proc`, `Proc = *Name | …`), the `PSort` judgment, the flat 8-field `Par`, and the refinements. |
| [`spec/INVENTORY.md`](../../../spec/INVENTORY.md) | The **law catalog**: the calculus laws, the surface a client writes (grammar, lexing, normalization, matching, reply shapes, JSON), 4 for the Proof-of-Stake epoch (the boundary gate, the reward split, its conservation, and withdrawal staging) plus the delegator's stake (57), one for the fee consequence of a denied deploy (an open design question, not a settled behaviour), one for what a matched deploy is charged (the storage refund, and why its order is the rule), the depth guards (50), the progress set (51–54), the bounded-work and namespace rows (55–56), the finality gate's forward dual (58), the wire and session laws (59–65), the distributed-system set (66–71), the durable reset, the stop and the read construction (72–77), the genesis's developer allocation (78) and the schedule it unlocks on (79). One row per law, with source-of-truth, Rust realization, Lean/Coq target, status, and — for rows 30+ — the corpus and the consumer. |
| [`spec/conformance/`](../../../spec/conformance/) | The **conformance corpora**: one TSV per layer, *emitted* from the Lean definitions (`lake exe rchain-corpus --layer <l>`), committed, and read by a Rust consumer, one per corpus, that runs the same cases through the node and must agree 1:1. This is the artifact that binds the specification to the code — and the reason a definition cannot drift from its corpus without the gate noticing. |
| [`spec/API-SCHEMA.md`](../../../spec/API-SCHEMA.md) | The **response-shape contract**: what rholang code and HTTP clients receive, with the reply-channel rule and the `RhoExpr` wire shape. Its rows are held to the checked catalog by law 39's doc tie, not by convention. |
| [`spec/GENESIS.md`](../../../spec/GENESIS.md) | The genesis manifest: the constants a client hardcodes, the ceremony key arrangement, and the consumer evidence for each entry. |
| [`spec/TYPE-SYSTEM.md`](../../../spec/TYPE-SYSTEM.md) | The ρ→CoC type discipline: totality (`TotalOn`), refinement sigma-types, and the "no silent partiality" guarantee. |
| [`spec/AUDIT.md`](../../../spec/AUDIT.md) | The audit **check-off**: what is left, in three states, and the coverage that bounds it. The evidence behind it, and the Scala-deviation register (§6), are in [`spec/audit/passes.md`](../../../spec/audit/passes.md). |
| [`spec/TEST-COVERAGE.md`](../../../spec/TEST-COVERAGE.md) | The **test-coverage register**: the per-crate inventory, the law matrix, the risk tiers, and the exempt-module table. Machine-checked by [`tools/audit-status.sh`](../../../tools/audit-status.sh) (`make check-register`), which fails on an overstated count, a named test that does not exist, a source file with neither a test nor an exemption row, **or a law row claiming coverage without naming a Lean module, a corpus and a consumer that exist**. |
| [`spec/RUST-FIRST.md`](../../../spec/RUST-FIRST.md) | The native-system-contract design: which parts of the node are Rust-native rather than ports, and the simplifications each carries. |
| [`spec/RUST-VS-SCALA.md`](../../../spec/RUST-VS-SCALA.md) | How the rewrite makes the Scala's fragile patterns explicit, and where it deliberately deviates. |
| [`spec/Rchain/`](../../../spec/Rchain/) (Lean 4) | The machine-checked definitions and theorems (`lake build`). |
| [`spec/coq/`](../../../spec/coq/) (Coq) | The substitution / α-equivalence metatheory (`make`). |

The executable semantics of the language are the K-framework rules under
[`legacy/rholang/src/main/k/rholang/`](https://github.com/rchain-community/rchain-rust/blob/1b7583649/legacy/rholang/src/main/k/rholang/) — the operational
definition of Laws 2–6.

## The gates

| Gate | What it refuses | Runs as |
|---|---|---|
| [`tools/check-lean-conformance.sh`](../../../tools/check-lean-conformance.sh) | a failed Lean or Coq build, a `sorry`/`admit`, a module `Rchain.lean` does not import, a **stale or untracked corpus**, a corpus with no consumer, a consumer that disagrees with its corpus, and a law-39 catalog urn with no row in `spec/API-SCHEMA.md` | `make check-lean`, and the `formal` job in CI |
| [`tools/audit-type-system.sh`](../../../tools/audit-type-system.sh) | any production `panic!`/`unsafe`/silent conversion in the Rust crates — the no-silent-partiality discipline | `coverage.yml` |
| [`tools/audit-status.sh`](../../../tools/audit-status.sh) | a check-off that disagrees with `spec/findings.tsv`, a malformed row, a `todo` that names nothing, a `done` row whose evidence resolves to nothing, **a law cell naming a law the register does not define**, an open row naming no law, and more than `LAWLESS_CEILING` classified-free rows (a ratchet: the residue is printed every run). **2.5 s** — this replaced `tools/audit-test-register.sh` (1,453 lines, 78 s), which checked that documents agreed with each other |
| [`tools/audit-vendored-sources.sh`](../../../tools/audit-vendored-sources.sh) | an edit to the oracle's own vendored source text, which is a specification change wearing a comment's clothes |
| [`tools/check-review-ledger.sh`](../../../tools/check-review-ledger.sh) | a source file with neither a test nor an exemption, and a law row that claims coverage without evidence | `make check-register`, and `coverage.yml` |
| [`tools/check-wargame.sh`](../../../tools/check-wargame.sh) | a Table A row whose cell count disagrees with the page's own header, a row with no decided verdict or no falsifier, a falsifier naming a path or symbol that does not exist in the tree, and a page that declares no referee calibration | `ci.yml`'s static gates |
| [`tools/check-build-paths.sh`](../../../tools/check-build-paths.sh) | an artefact-producing `cargo build`/`install` that does not pass `--locked`, a release Dockerfile that declares a build argument reaching `cargo`, and a base image that is not digest-pinned | `ci.yml`'s static gates |
| ~~`tools/audit-test-register.sh`~~ | deleted 2026-09-27. Kept here so a reader arriving from an old citation knows where it went | was `make check-register` |

**A change to a law is a change to three files**: the Lean definition, the corpus emitted from it, and
the Rust consumer. The gate is what makes that true rather than aspirational — `make spec` is only the
Lean half, and a corpus left stale fails the gate.

`spec/audit/passes.md` §20 is the reader's index back the other way: every incident the audit records, mapped
to the law that now covers it and the case that fails if the behaviour returns.
