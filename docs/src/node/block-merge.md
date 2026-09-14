# Block merging (RCHIP-02)

CBC-Casper lets a block have **multiple parents**, so validators can build on concurrent blocks
instead of serializing on one. That is only useful if a validator can turn *N* parent states into
**one** merged state — deterministically, on every node.

[RCHIP-02](https://github.com/rchain/rchip-proposals/issues/23) specifies how: don't re-**run** the
other parents' deploys, **merge their event logs**. This page is the map from that proposal to the
Rust node, plus the invariants that make the merge safe to finalize.

## The problem the RCHIP solves

The naive merge replays every deploy of every parent on top of the chosen base parent. Since parents
are re-merged at each step, the same deploys are executed again and again — the cost of a merge grows
with the *history*, not with the *divergence*.

RCHIP-02's proposal: keep, for each block, the changes it made (**event log**); when merging, detect
which changes **conflict**; apply only the non-conflicting ones; and mark the losers **denied** in the
block. The block still gets replayed **once** to check that its event log is correct — but merging no
longer re-executes other branches' deploys.

## The model

| RCHIP-02 | Rust |
|---|---|
| conflict set (events back to the last finalized block) | the **merge scope** — `MergeScope { final_scope, conflict_scope }` built from the merge fringe and the final fringe (`casper/src/merging.rs`) |
| per-deploy event log | `EventLogIndex` (`rspace/src/merger/event_log_index.rs`): the produces/consumes/joins a deploy touched, each classified (created / destroyed / copied by peek / existed in pre-state / touches a pre-state join), plus `NumberChannelsDiff` for mergeable channels |
| a block's changes | `BlockIndex` → `Vec<DeployChainIndex>`; each chain carries its `StateChange` (the trie-level diff) and `EventLogIndex` |
| denied deploys written into the block | `BlockMessage::rejected_deploys` (the `rejectedDeploys` field) |
| apply the surviving changes to the base state | `MergeScope::compute_merged_state` → `StateChange::combine` → `compute_trie_actions` (`rspace/src/merger/state_change_merger.rs`) → `HotStoreTrieAction`s → one checkpoint on the base history |
| "the deployer is not charged, the validator is not rewarded" | see *Open items* — the effects are excluded from the merged state; the accounting consequence is not implemented |

## The mergeability rules

Conflicts are decided from event logs, not by executing anything (`rspace/src/merger/event_log_merging_logic.rs`,
a port of `EventLogMergingLogic.scala`):

- `produces_created`, `consumes_created`, `produces_created_and_not_destroyed` classify a log's
  actions;
- `are_conflicting(a, b)` — two logs conflict when one creates/destroys what the other reads or
  writes in an incompatible way;
- `depends(a, b)` — one log depends on the other's effects, so they must both be kept, in order;
- a **shared deploy id** is always a conflict (`DeployChainIndex::deploys_are_conflicting`).

That is the counterpart of `MergeabilityRules.scala`; the RSpace-level rules that the Scala
implementation was missing (see the issue thread) are here, which is why the merge can be applied
without replay.

## Conflict resolution is a function, not a choice

`sdk/src/dag/merging.rs` computes the resolution:

1. `compute_conflicts_map` / `compute_dependency_map` → `compute_relation_map_for_merge_set`;
2. `compute_rejection_options` → the candidate sets of deploys to drop;
3. `compute_optimal_rejection` → **the unique minimum**, compared lexicographically by
   `(total cost, size, sorted deploy set)`;
4. `resolve_conflict_set` → `(accepted, rejected)`, honouring the finalized scope
   (`incompatible_with_final`), dependency closure (`with_dependencies`) and the mergeable-channel
   balances.

Because step 3 is a total, lexicographic minimum over a finite set, *every* node picks the same
winner: **merge determinism is Law 17**, and it is exactly why the tie-break includes the sorted set
rather than just the cost.

`MergeScope::compute_merged_state` then combines the accepted chains' `StateChange`s and applies the
mergeable-channel arithmetic — a channel whose value is a number merges by **summing the diffs**
(`rholang/src/merging.rs`; the diffs are checked for `i64` overflow rather than wrapped — `spec/AUDIT.md` §6).

## Validation: replay once, and check the denied set

A validating node does **not** trust a block's claimed merge. In `casper/src/interpreter_util.rs`
(`validate_block_checkpoint`):

1. it recomputes the **pre-state** from the block's parents (`get_pre_state_for_parents`), and rejects
   the block if the claimed `pre_state_hash` differs;
2. it rejects the block with `BlockStatus::InvalidRejectedDeploy` if the block's `rejected_deploys`
   set differs from the one the parents' merge produces — a validator cannot deny a deploy that
   should have survived (or keep one that should have been denied);
3. it **replays** the block once at that pre-state and checks the resulting state hash.

So the event-log merge is what makes the *state*; the single replay is the audit that the event log
was honest. Both are required, and both are deterministic.

## Determinism

- the merge scope, the conflict/dependency maps, the rejection options and the optimum are all
  functions of the DAG and the event logs — no clocks, no maps with undefined iteration order
  (`BTreeMap`/`BTreeSet` throughout);
- `StateChange::combine` is a **monoid** and the mergeable-channel sum is commutative (**Law 9**), so
  independent logs merge in any order;
- the trie update is content-addressed (**Law 10**), so the merged state hash is a function of the
  changes, not of the order they were applied in;
- `calculate_num_channel_diff` is checked, so a number channel cannot silently wrap into a wrong
  merged state (`spec/AUDIT.md` §6).

## Open items

- **Charging a denied deploy.** RCHIP-02 says the deployer should not be charged and the validator
  should not be rewarded for a denied deploy. The node records the denied set and excludes its
  effects from the merged state; the *fee* consequence is not implemented (a denied deploy was paid
  in the block that carried it). Treat as an open design question, not a settled behaviour.
- **The RCHIP's `RuntimeManager` obstacle is addressed differently.** The proposal notes that a global
  lock plus a singleton interpreter with global cost state prevented parallel execution. In this port
  block validation **forks a replay runtime** at the block's pre-state
  (`fork_replay_runtime`), so validations are self-contained; the scheduler's Laws 20–29 carry the
  concurrency story on the execution side.
- **The fast path is not benchmarked here.** The event-log merge avoids re-execution by construction,
  but this tree has no merge-cost benchmark; `docs/src/node/operating.md` covers running a node, not
  merge throughput.

> **Formal.** Merge monoid and commuting logs are **Law 9**; Merkle determinism is **Law 10**; replay
> determinism is **Law 11**; merge determinism (unique min-cost rejection) is **Law 17**. See
> [The 29 laws](../formal/the-29-laws.md), [Determinism](../formal/determinism.md) and
> [`spec/INVENTORY.md`](../../../spec/INVENTORY.md).
