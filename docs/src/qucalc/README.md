# QuCalc — native AI + group support for rholo

QuCalc is the Rust-first substrate that brings **native AI** (a deterministic ZFA
proof system with dialectical synthesis) and **native group support** (liquid
democracy + liquid trust governance) into the rholo node as *system contracts* —
not as external oracles, not as off-chain services.

It has two halves that mirror quantum-os's own split of "signed envelopes + a
deterministic, joiner-local recomputation":

1. **Native system processes** — `rho:qucalc:{zfa,grant,verify,fuse}` and
   `rho:gov:{resolveWeights,trustLevels,censure,tally}` are installed by the node
   and executed exactly like any other system contract (gas-metered, replay-
   deterministic, unforgeable).
2. **Pure decision core** — [`qucalc::gov`](../../../qucalc/src/lib.rs) and the
   ZFA/dialectical arithmetic are total, order-insensitive folds, so every peer
   reproduces the identical result from the same signed inputs.

## Why this matters — the mutual benefit

### rholo gains native AI

`rho:qucalc:*` gives every rholo deploy a *capability-based AI proof primitive*:

- **`zfa`** — verify a twist history is a half-spin ZFA closure
  (`pauli_closed ∧ count_balanced`), with its scalar phase returned. Exact
  integer complex arithmetic ({−1, 0, 1}), never floating point, so the
  predicate is deterministic and replay-safe on-chain.
- **`grant` / `verify`** — mint a ZFA-balanced proof as a content-addressed
  registry capability and re-verify it **across deploys**. A proof *persists* in
  the registry like any other unforgeable name.
- **`fuse`** — dialectical synthesis (Blanket Fusion): thesis ⊕ antithesis through
  a shared middle term collapses to a stable fluxoid, minted as a capability.

### rholo gains group support

`rho:gov:*` + [`gov.rho`](../../../qucalc/rholang/gov.rho) bring liquid-democracy
governance natively on-chain — groups, membership-as-capability, transitive
delegation, admin-rooted trust, ⅔-quorum censure with voucher slashing, and
weighted IRV/approval tally. Identity is bound to the unforgeable `*deployerId`,
so a member can only set their *own* delegate / rating / censure / ballot.

### A basis for integrating other AIs

The substrate is deliberately **pluggable**. quantum-os demonstrates the pattern
with its neuro-symbolic coprocessor (`qlf_ai_coprocessor`, ported here as
[`ai_coprocessor.rs`](../../../qucalc/examples/ai_coprocessor.rs)): any AI model
whose output can be mapped to a ZFA twist sequence (or a signed envelope) can plug
into the same `rho:qucalc:*` / `rho:gov:*` surface. The capability +
unforgeable-identity machinery is model-agnostic — it does not care *which* AI
produced the proof, only that the proof is ZFA-closed and the envelope is
self-signed. That is the seam along which additional AI systems integrate.

In exchange, [quantum-os](https://github.com/rhovision-org/quantum-os) and the
[Quantum Logical Framework](https://github.com/rhovision-org/quantum-logical-framework)
(both forked into the Rho Vision org, `rhovision-org`) gain what rholo already provides:
unforgeable names, a persistent content-addressed registry, consensus and
replay-determinism, and a gas market for computation.

The concrete **AI / agent / decision-support** documents this integration is built
on are indexed in [`references.md`](references.md): native AI
([`AI.md`](https://github.com/rhovision-org/quantum-logical-framework/blob/main/AI.md),
[`TheQuantumBrain.md`](https://github.com/rhovision-org/quantum-logical-framework/blob/main/TheQuantumBrain.md),
[`DAICE_QLF_Framing.md`](https://github.com/rhovision-org/quantum-logical-framework/blob/main/DAICE_QLF_Framing.md)),
agent support
([`Developer_Guide.md`](https://github.com/rhovision-org/quantum-os/blob/main/Developer_Guide.md),
[`MyRoom.md`](https://github.com/rhovision-org/quantum-os/blob/main/MyRoom.md)), and
decision support
([`Governance.md`](https://github.com/rhovision-org/quantum-os/blob/main/Governance.md),
[`Group_Decisions.md`](https://github.com/rhovision-org/quantum-os/blob/main/Group_Decisions.md),
[`Consensus.md`](https://github.com/rhovision-org/quantum-os/blob/main/Consensus.md)).

## Documentation map

| Document | Contents |
|---|---|
| [`quantum-to-rho.md`](quantum-to-rho.md) | The justification for quantum operators on rholo, and the precise translation of each quantum operator into the ρ-calculus and rholang. |
| [`multi-stakeholder-governance.md`](multi-stakeholder-governance.md) | The rationale for on-chain multi-stakeholder governance, and how the `rho:gov:*` layer realizes it (adapted from a Google Doc). |
| [`architecture.md`](architecture.md) | The technical architecture: crate layout, ZFA predicate, dialectical synthesis, governance core, system processes, determinism guarantees. |
| [`extensions.md`](extensions.md) | How to call the eight system processes: signatures, argument shapes, return values, and worked examples. Start here to use QuCalc from a deploy. |
| [`examples.md`](examples.md) | Catalog of the runnable rholang examples and the Rust coprocessor. |
| [`references.md`](references.md) | Upstream AI / agent / decision-support documents in the forked quantum-os and quantum-logical-framework repos. |
| [`Governance.md`](../../../qucalc/rholang/Governance.md) | The governance design: liquid democracy + liquid trust, the governing rule, and the self-signed envelope model. |

## Where things live

```
qucalc/
├── src/lib.rs            # ZFA predicate + dialectical synthesis + qucalc::gov (pure core)
├── src/main.rs           # census superposition demo (ways-as-coefficient invariant)
├── examples/             # runnable demos (.rho + ai_coprocessor.rs)
└── rholang/              # libraries: gov.rho, qucalc.rho, Directory/Inbox/Chat.rho
    └── Governance.md     # governance design doc
```

The system-contract wiring lives in
[`rholang/src/system_processes.rs`](../../../rholang/src/system_processes.rs).
