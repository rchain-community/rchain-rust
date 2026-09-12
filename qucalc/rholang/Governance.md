# Governance — liquid democracy & liquid trust on the QuCalc substrate

The `rho:gov:*` system processes and the [`gov.rho`](gov.rho) library port the
**intent** of rgov's `rholang/core` (Group / Issue / Ballot / delegateVote /
castVote / tallyVotes) onto the QuCalc substrate — the same capability + ZFA
machinery as the [proof system](qucalc.rho). This is the RChain side of
quantum-os's governance layer.

> **Documentation**: the QuCalc documentation index (overview, mutual benefits,
> architecture, examples) lives in
> [`docs/src/qucalc/README.md`](../../docs/src/qucalc/README.md) (book Part VI).

## What it provides

| Capability | Rholang library (this repo) | Backing implementation |
|---|---|---|
| Liquid-democracy weights (transitive delegation, cycle/dead-end abstention) | [`gov.rho`](gov.rho) — `Gov!("weights", …)` | `qucalc::gov::resolve_weights` |
| Liquid trust (admin-rooted, strictly-decreasing trust web, `weight = 1 + level`) | [`gov.rho`](gov.rho) — `Gov!("levels", …)` | `qucalc::gov::trust_levels` |
| Accountability (⅔-quorum censure, floored at 2, with voucher slashing) | [`gov.rho`](gov.rho) — `Gov!("censureResult", …)` | `qucalc::gov::censure` |
| Weighted ranked-choice (IRV) / approval tally | [`gov.rho`](gov.rho) — `Gov!("tally", …)` | `qucalc::gov::tally_ranked` / `tally_approval` |
| Groups & membership (admin-gated, minted capability URI) | [`gov.rho`](gov.rho) — `Gov!("new" · "member" · "isMember", …)` | `rho:registry:insertArbitrary` / `lookup` |
| Signed decision of record | [`gov.rho`](gov.rho) — `Gov!("ratify", …)` | `rho:registry:insertSigned:secp256k1` |
| ZFA proof substrate (the capabilities governance rides on) | [`qucalc.rho`](qucalc.rho) — `zfa` / `grant` / `verify` / `fuse` / `ratify` | `qucalc::achieves_zfa`, `dialectical_synthesis` |
| Reusable primitives (rgov ports) | [`Directory.rho`](Directory.rho), [`Inbox.rho`](Inbox.rho), [`Chat.rho`](Chat.rho) | — |

The deterministic core lives in the **native** `rho:gov:*` system processes
(implemented in [`rholang/src/system_processes.rs`](../../rholang/src/system_processes.rs),
pure functions in [`qucalc/src/lib.rs`](../src/lib.rs)). The `gov.rho` library
holds the *signed envelopes* and composes the natives — mirroring quantum-os's
split of "signed envelopes + a deterministic, joiner-local tally."

## The governing rule

* a member who casts a ballot votes directly (overrides delegation);
* otherwise their vote flows transitively to their delegate's first voter;
* cycles / dead-ends abstain;
* base weight = `1 + trust level`, where trust descends from admins (level 5)
  **strictly decreasing** — two level-0 members cannot bootstrap each other.

Identity is **capability-backed, not a spoofable string**: a member's id is the
deployer's unforgeable `*deployerId`, and membership is a minted, registry-verified
capability. Envelopes are genuinely self-signed — you can only set your own
delegate / trust rating / censure / ballot.

## Rholang libraries (this repo)

* [`gov.rho`](gov.rho) — the governance library: groups, membership, self-signed
  envelopes, pure recomputations, and signed decision-of-record.
* [`qucalc.rho`](qucalc.rho) — the proof substrate: ZFA predicate, capability
  minting/verification, dialectical synthesis (`fuse`), and `ratify`.
* [`Directory.rho`](Directory.rho) — a capability-facet key/value store (port of rgov `Directory.rho`).
* [`Inbox.rho`](Inbox.rho) — a capability-facet message store (port of rgov `Inbox.rho`).
* [`Chat.rho`](Chat.rho) — a publish/subscribe mailbox (port of rgov `Chat.rho`).

## Rholang examples (mirroring the quantum-os demos)

Each example is a rholang program in [`../examples/`](../examples/) that mirrors a
quantum-os demo, using the `rho:qucalc:*` / `rho:gov:*` system processes:

| quantum-os demo | Rholang example (this repo) |
|---|---|
| SyllogismDemo (dialectical synthesis) | [`../examples/syllogism.rho`](../examples/syllogism.rho) |
| MultisigDemo (N-of-M quorum co-signature) | [`../examples/multisig.rho`](../examples/multisig.rho) |
| PromissoryNoteDemo (bearer note lifecycle) | [`../examples/promissory_note.rho`](../examples/promissory_note.rho) |
| AtomicSwapDemo (all-or-nothing exchange) | [`../examples/atomic_swap.rho`](../examples/atomic_swap.rho) |
| DiningPhilosophersDemo (deadlock-free resource acquisition) | [`../examples/dining_philosophers.rho`](../examples/dining_philosophers.rho) |
| Governance / Group_Decisions (liquid democracy) | [`../examples/liquid_democracy.rho`](../examples/liquid_democracy.rho) |
| (Rust dialectical-synthesis walkthrough) | [`../examples/ai_coprocessor.rs`](../examples/ai_coprocessor.rs) |

## Source of intent

The port replaces rgov's `rholang/core` contracts rather than running them
verbatim: [rchain-community/rgov — rholang/core](https://github.com/rchain-community/rgov/tree/master/rholang/core)
(`Group.rho`, `Issue.rho`, `Ballot.rho`, `Directory.rho`, `Inbox.rho`,
`Chat.rho`, `Kudos.rho`, `RevIssuer.rho`, `CrowdFund.rho`, `memberIdGovRev.rho`).
