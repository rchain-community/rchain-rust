# Determinism of the block state transition

> This document is the **specification of the node's state-transition determinism** — the statement that
> the post-state hash of a block is a *pure function* of its inputs, founded in the 22 laws
> ([`spec/INVENTORY.md`](../../spec/INVENTORY.md)). It is the target the Lean formalization
> (`spec/Rchain/`) proves, and the target the Rust port's `play` and `replay` paths both implement. The
> prior documents specify the components: [The 22 laws](the-19-laws.md), [The concurrency model](concurrency-model.md),
> and [Effect scheduling](effect-scheduling.md). This page fixes the *block* level: what the block
> creator computes and what block validation must recompute, and why they must agree.

## The invariant in one line

> **`postStateHash = transition(preState, block, seed)` is a pure function.** Block *creation* (play)
> and block *validation* (replay) are two implementations of the **same** function; for every block,
> `play(preState, block, seed) == replay(preState, block, seed)`. Any difference is a consensus
> violation — a block that one honest node accepts and another rejects.

Determinism is not a property of the raw `Reduce` relation (which is not even single-step deterministic
up to `≡`); it is a property of the **chosen canonical schedule**, and of the port faithfully realizing
*one* transition function in both the play and replay paths. The 22 laws turn that into a theorem:

| Law | What it guarantees for the block state transition |
|-----|-----------------------------------------------------|
| **4** — reduction (`⟶`) | The transition is a single deterministic reduction function; `new` yields fresh names deterministically from the seed; first-match-wins. |
| **1** — canonical total order | The post-state hash is order-independent (`sort` is idempotent and commutative). |
| **8** — deterministic COMM | Candidate and waiting-continuation selection is canonical (sorted-first by content hash). |
| **9** + **17** — merge monoid; RNG merge | Log merges and `Blake2b512Random` merges are commutative/associative (order-independent). |
| **10** — Merkle determinism | The state trie is content-addressed and collision-free. |
| **11** — replay determinism | Replay recomputes the recorded COMM trace exactly. |
| **19** — crypto | Canonical `Blake2b256` hash; `Blake2b512Random` is an associative splittable merge. |

## The play/replay sub-invariants

Play and replay are two code paths (`casper/src/runtime_manager.rs` vs `casper/src/runtime_replay.rs`)
that must compute the *same* transition. The following sub-invariants are the concrete obligations both
paths must satisfy. Each is tagged with the law(s) it realizes.

- **S1 — normalizer env.** Both paths normalize the deploy term with `NormalizerEnv(deploy)`, which
  binds `rho:rchain:deployerId → RhoDeployerId(deployer)` and `rho:rchain:deployId → RhoDeployId(sig)`.
  The env is part of the term's denotation, so a term that binds those URIs as
  `new x(`rho:rchain:deployerId`)` must resolve identically on both paths. *(Law 4.)*
- **S2 — seed derivation and split.** The block seed is a pure function of
  `(shard_id, block_number, sender, pre_state_hash)`; the per-deploy splits use the same indices
  `0`/`1`/`2` (pre-charge / user deploy / refund), and the block-level system-deploy splits use
  `deploy_count + k` (slashes) and `deploy_count + to_slash_count` (close). *(Laws 4, 19.)*
- **S3 — cost accounting.** Pre-charge is `phloLimit × phloPrice` (`totalPhloCharge`); refund is
  `max(0, phloLimit − cost) × phloPrice`. Cost is a deterministic function of the reduction. *(Law 4.)*
- **S4 — system-deploy construction.** Slash deploys are built over the sorted validator set, then
  close-block last, with identical arguments and seeds on both paths. *(Law 4.)*
- **S5 — native-store checkpointing.** The native (bonds/vault) overlay drains into the trie identically
  on both paths, so the content-addressed root is the same regardless of checkpoint cadence. *(Law 10.)*
- **S6 — reducer ordering.** Matching, merging, and dispatch iterate canonical (sorted) structures, so
  no `HashMap`/`HashSet` iteration order reaches the state hash. *(Laws 1, 8, 9, 17.)*

## Violations found in the port (and their remediation)

The port split Scala's single evaluator (`legacy/.../RuntimeSyntax.scala:527-535`, which both play and
replay call) into two Rust paths, which drifted. The audit found:

| # | Sub-invariant | Violation | Effect |
|---|---------------|-----------|--------|
| **D1** | S1 | Replay re-normalizes with an **empty** env (`evaluate(term)`), so `new x(`rho:rchain:deployerId`)` fails `add_urn` with `BugFoundError`. | Deterministic `InvalidStateHash` for every REV-transfer/bond/vault deploy. |
| **D2** | S3 | Play refunds `phloLimit`; replay refunds `(phloLimit − cost) × phloPrice`. | Latent consensus divergence, masked only because the native refund is a no-op. |
| **D3** | S2/S4 | Play uses the *requested* deploy count (`deploys.len()`) for the slash/close seed index; replay uses the *actual* count (`state.deploys.len()`). | Latent divergence if a requested deploy is absent at block-creation time. |
| **S6a** | S6 | `maximum_bipartite_match.rs` returns a `HashMap::into_iter()` in process-randomized order. | Currently commutative, but the only randomized container iteration in the reduction path. |
| **S6b** | S6 | `dispatch.rs` merges branch RNGs in un-sorted `data_list` order. | Correct only because `data_list` order is deterministic; fragile. |

The remediation is in the code (see the plan), and — critically — is pinned by the executable check
below so future drift is caught at compile/test time rather than rediscovered in consensus.

## The executable check

The spec's runnable form is a play↔replay regression test in `casper/tests/`: it plays a deploy that
binds `rho:rchain:deployerId` (the transfer idiom), replays it, and asserts
`play_post_state == replay_post_state` — plus a cost/refund check for a non-trivial-cost deploy. Any
future S1–S4 drift fails that test.

## What determinism does *not* require

Determinism is about the **state hash**, not about the recorded cost field or diagnostic output. Two
documented, non-diverging asymmetries remain and are explicitly out of scope of the hash:

- **Genesis-vault re-seed** — `replay_block` passes no vaults; safe because genesis is trusted and never
  re-validated (an asserted invariant, not a code path).
- **Concurrent-mode recorded cost** — under fork-join the *recorded* `PCost` at failure can differ from
  the sequential value; it affects the block's cost field, not the post-state hash.
