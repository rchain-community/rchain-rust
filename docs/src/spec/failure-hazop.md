# Failure-mode HAZOP and disposition

**Reviewed revision.** `dev` at the merge of #281 (`5fc591e33`) — the tree every `file:line` below was
**re-derived** against. The pass began on that branch (`410af4cbf`, the tip of
`fix/n280-merge-loses-a-write`), and the citations were re-derived a second time after the branch was
squash-merged, because the fix for CI's hang moved `casper/src/merging.rs` by ~28 lines and this
repository's rule is that a citation is re-derived, never remapped by a delta
(`spec/audit/evidence/ocapn-hazop.md` §6, hypothesis **R4**). That rule paid for itself here: the drift
was 26 lines at one end of the file and 28 at the other, so a delta would have been wrong in both
directions.

**Why this study exists.** A merge lost a committed deploy's write at an epoch boundary, **unanimously,
with no counter and no log line anywhere moving** — four nodes agreeing on a state that three rounds
earlier had contained the deploy (#280 / C248). On 2026-10-08 the same class produced **four divergent
heads** on the public testnet with finality frozen at 101 for more than thirteen hours, and *nothing in
this tree recovered it*. Both were found by an operator reading a live chain, not by a test.

The property every row below is dispositioned against is the requirement, stated as the target:

> These failure modes should ideally be **unrepresentable**. Where they are not, they must **fail loudly
> and hard**, and then **hard-reset to a safe state and catch up** — never wedge silently.

**Method.** The worksheet format is this repository's own (`testnet-acceptance.md` §1): per study node,
a design intent, the eleven IEC 61882 guide words each dispositioned, and a row per credible deviation.
The study nodes **U1–U10 are reused verbatim**, so a hazard appears once and is dispositioned here.

**Companion, not a replacement.** The hazards and their root-cause analysis live in
`testnet-acceptance.md` §1 as rows `H-U*-*`; this page does not restate them. What it adds is the
**disposition** — which rung of the ladder below the deviation lands on, what the node does today, and
what would prove it fixed. Its rows are prefixed `F-U*` so the two worksheets compose, and each names its
`H-` sibling where one exists. A guide word this pass has not reached is declared **`owed`** rather than
left blank, and the debt is counted in §7 where a check reads it.

## 1. The disposition ladder

- **R1 — unrepresentable.** The bad state cannot be constructed: a refinement type, or an operation
  deleted rather than narrowed (`spec/TYPE-SYSTEM.md` §1.7; `tools/audit-type-system.sh`). The worked
  precedent is **C248**, which did not narrow `reject_whole_blocks` but *deleted* it, so "reject every
  chain of a block because of a write the block made" has no operation left to express it, and moved the
  relation onto the chain (`DeployChainIndex::native_effects`, `casper/src/merging.rs`).
- **R2 — fail loudly.** The failure is representable but must not be silent: a report struct + counters +
  a rate-limited log that re-fires on change + a named test. The `MergeReport` (invariants I1/I2,
  `casper/src/merging.rs:274`) and `NoAdvance<S>` + `describe_no_advance`
  (`block-storage/src/dag/finalizer.rs:61`, `casper/src/interpreter_util.rs:428`) are the shape.
- **R3 — hard reset, then catch up.** Detect a node that cannot self-heal, say so, then reset to a
  known-safe state and re-sync. **This rung does not exist in this tree for a running node** — see §5.
- **R4 — runbook.** Nothing better is affordable yet; the operator step is documented and made
  observable.

**What this ladder found.** Almost every path below already **fails loudly** — the merge refusal is a
`log.error` (`casper/src/blocks/block_processor.rs:172`), an I1/I2 violation a non-rate-limited `log.warn`
(`casper/src/interpreter_util.rs:535`), a finality stall a `log.warn` that re-fires on change
(`casper/src/interpreter_util.rs:519`), an ingress drop a `log.warn`
(`casper/src/engine/node_running.rs:664`). **Two paths are genuinely silent** (F-U8-01, F-U9-03). So the
gap is not mostly loudness: it is that the loud line has **no action behind it** — nothing re-queues the
dropped block, nothing restores the refused one, nothing resets the wedged node. R2 is largely won;
**R3 is entirely unbuilt**, and §5 names where it is owed.

## 2. Top events

`testnet-acceptance.md` §2 fixes four. Two are added here because the 2026-10-08 witness is a shape those
four do not separate, and the distinction is what decides the disposition.

| id | top event | source |
|---|---|---|
| **TE-1** | The chain halts and cannot recover | `testnet-acceptance.md` §2. **Live-witnessed 2026-10-08**: four divergent heads, finality frozen at 101 — `spec/audit/evidence/te-1-2026-10-09-four-divergent-heads.md` on branch `spec/te-1-witness` (not in this tree) |
| **TE-2** | A fragment of the net reaches finality that the rest rejects | `testnet-acceptance.md` §2 |
| **TE-3** | An outsider validator cannot safely join or leave | `testnet-acceptance.md` §2 |
| **TE-4** | State, disk or memory exhaustion halts the net | `testnet-acceptance.md` §2 |
| **TE-5** | **Node wedge** — a single node degrades and can neither progress nor recover itself, while the net runs on | new: the witness's restart experiments (a restart restores *production* and not *finality*), `AUDIT` C181 |
| **TE-6** | **Rewind-forcing divergence** — a state difference no local restore heals, so *the chain* must be rewound rather than a node | new: #280/#287 |

TE-5 and TE-6 are the two that R3 answers, and they are separated because TE-5 is per-node (a reset
heals it) while TE-6 is net-wide (only a reconciliation heals it — §5).

## 3. The worksheet

### U1 — Block production / proposal

**Design intent.** When and only when a trigger fires, the node builds at most one block per round per
validator, whose number and sequence are determined by its justification set, and offers it.

**Guide words.** row: PART OF · folded: LATE → NO, BEFORE → EARLY, AFTER → MORE · vacuous: REVERSE · owed: NO, MORE, LESS, AS WELL AS, OTHER THAN, EARLY

The REVERSE vacuity is the acceptance sheet's (`testnet-acceptance.md` §1 U1: the node has one directed
action, and `block_num`/`seq_num` are refined non-negative types whose validation refuses any parent at
or above the block's own number). The folds are its folds.

### Table A — deviations

| ID | Guide word | Deviation | RCA → causal chain | Current behaviour | Class | Disposition | Falsifier |
|---|---|---|---|---|---|---|---|
| **F-U1-01** | PART OF | The block is built, self-validated and inserted, but the **broadcast leg is fire-and-forget** — a proposal that reaches no peer while the node logs it as proposed (H-U1-05) | the callee returns no status and logs success unconditionally (`casper/src/protocol/comm_util.rs:182`) → the send error is swallowed one frame down in `send_to_peers` (`casper/src/protocol/comm_util.rs:83`), which drops `TransportLayer::broadcast`'s `Vec<CommErr<()>>` (`comm/src/transport/transport_layer.rs:18`) → the round cannot close on a block no peer holds | loud but false: the only line is a **success** log; nothing reports the drop | Drift | R2 | owed: a test that a failed broadcast is surfaced (no re-queue exists to ticket) |

### U2 — Attestation, round closure, quorum

**Design intent.** A validator answers a remote block at most once per sender per height; a round closes
only when every live bonded sender has a message above the boundary; the quorum that closes it is a
strict supermajority of the whole bonded map.

**Guide words.** row: LATE · folded: AS WELL AS → MORE, BEFORE → LESS, EARLY → LESS, NO → LATE, AFTER → LATE, PART OF → OTHER THAN · vacuous: REVERSE · owed: MORE, LESS, OTHER THAN

The folds and the REVERSE vacuity are the acceptance sheet's (§1 U2).

### Table A — deviations

| ID | Guide word | Deviation | RCA → causal chain | Current behaviour | Class | Disposition | Falsifier |
|---|---|---|---|---|---|---|---|
| **F-U2-01** | LATE | The **attestation guard is node-local while the finalizer's partition is per-block** — the guard suppresses on *this* node's seen view, the partition is derived from the block's own justifications, and the two can disagree (H-U1-01/03's root cause) | the guard reads `fringe_seen` from `pre_state.prev_fringe` and decides `nothing_to_finalize` (`casper/src/blocks/proposer/proposer.rs:955`, `:1037`) → the finalizer's live set is deterministic per block (`block-storage/src/dag/liveness.rs:153`, whose comment says "every node validating the same block derives the same live set") → a node can suppress where the partition has work, freezing the fringe while production continues | loud: the stall reason is logged (`casper/src/interpreter_util.rs:519`), rate-limited and re-firing on change | **Drift, not Split** | R2 | owed: the suppression must be counted on a surface (see F-U10-01) |

**This row corrects the seed catalogue.** The programme's plan called this "a `Split` risk". The two
surfaces *can* disagree, but validity is recomputed deterministically from the block's own justifications
by every peer — so the disagreement produces a **liveness stall, never a validity split**. Class is
`Drift`; the fix is not "make both readers ask one side".

### U3 — Finality / fringe / estimator

**Design intent.** The fringe advances only over a full partition whose every member has seen every
other, and a finalised block is never undone.

**Guide words.** row: NO · folded: MORE → AS WELL AS, PART OF → AS WELL AS, OTHER THAN → AS WELL AS, AFTER → LATE, BEFORE → EARLY · owed: LESS, AS WELL AS, LATE, REVERSE, EARLY

No word is vacuous at this node (the acceptance sheet's §1 U3 contests its own EARLY/BEFORE vacuity in
CH-U3-03, which this pass does not reopen). The folds are its folds.

### Table A — deviations

| ID | Guide word | Deviation | RCA → causal chain | Current behaviour | Class | Disposition | Falsifier |
|---|---|---|---|---|---|---|---|
| **F-U3-01** | NO | **While finality is stalled there is no fringe to sync from**, so the *only* path that restores a node is closed for exactly as long as the chain is broken — a restoring or joining node answers `"Finalized fringe is not available."` | the restore/sync path reads the **finalised** fringe → finality frozen at 101 means no new fringe is minted (`spec/audit/evidence/te-1-2026-10-09-four-divergent-heads.md`, "The dependency this exposes") → the node's recovery and the chain's recovery are the same dependency, so a stalled chain cannot be repaired by restarting nodes into it | loud, and already stated in the specification (`testnet-acceptance.md`:1147, "Recovery from the divergent finality itself: none") | Terminal | R3 (owner: #287) | owed: #287's acceptance drill |

### U4 — Merge & DAG search

**Design intent.** The merge of a branch set is deterministic, order-independent, and declines safely
when its budget is exhausted.

**Guide words.** row: MORE, AS WELL AS, OTHER THAN, PART OF · folded: NO → MORE, BEFORE → LATE, AFTER → LATE · vacuous: REVERSE, EARLY · owed: LESS, LATE

The vacuity reasons are the acceptance sheet's (§1 U4: a `min` over a materialised `BTreeSet` has no
order to reverse; the search is a synchronous pure function metered in work units, not time).

### Table A — deviations

| ID | Guide word | Deviation | RCA → causal chain | Current behaviour | Class | Disposition | Falsifier |
|---|---|---|---|---|---|---|---|
| **F-U4-01** | MORE | A **search-budget refusal, a native-relation refusal and a number-channel overflow all become one drop with no re-queue** — the node cannot advance at that height until the scope shrinks (H-U4-01) | the three refusals are `Err(String)` inside `MergeScope::merge` (`casper/src/merging.rs:1876`, `:1641`, `:2014`) → `validate_block_checkpoint` carries them out (`casper/src/interpreter_util.rs:485`) → `validate_block` maps them to `ValidateError::Internal` (`casper/src/multi_parent_casper.rs:850`) → `block_processor::apply` logs and `continue`s (`casper/src/blocks/block_processor.rs:168`), and the comment there states that no deferral loop is intended for this class | **loud** (`log.error`, `block_processor.rs:172`) and **actionless**: nothing retries, nothing re-queues | Terminal | R2 + R3 | owed: a test that a refused merge is re-queued rather than dropped (the search *budget* test exists — `sdk/src/dag/merging.rs:a_budget_refuses_without_answering_and_never_changes_the_answer` — but it witnesses the refusal, not the recovery) |
| **F-U4-02** | AS WELL AS | **The merge's two invariants are computed, reported and not enforced** — a merge that violates I1 or I2 still returns a state hash | I1 (every rejected chain conflicts with a kept one) and I2 (every kept chain's write is in the batch) are computed into `MergeReport` (`casper/src/merging.rs:1957`, `:1974`) and `MergeOutcome` is returned unconditionally right after (`casper/src/merging.rs:1981`) → the only consumer is `describe()`/`is_quiet()` (`casper/src/merging.rs:299`, `:322`) → the caller logs it when non-quiet (`casper/src/interpreter_util.rs:534`) | loud (`log.warn`, not rate-limited) but **informational**: the violating state is used | Void | R2 | owed: a counter for a non-quiet `MergeReport`, surfaced where an operator reads (the fields exist; nothing counts them) |
| **F-U4-03** | OTHER THAN | **Historical / closed — the R1 precedent.** `reject_whole_blocks` propagated one rejected chain to its *whole block*, so a rejection took chains that wrote no contended slot; the fix **deleted** the operation rather than narrowing it (C248) | the native relation was keyed on the host block (`NativeRelations::conflicting`), so at an epoch boundary every concurrent pair conflicted → the rejection closed over the block → chains died with their host | (closed) the operation no longer exists; the relation is on the chain (`DeployChainIndex::native_effects`) | Terminal | **R1** (worked precedent) | casper/src/merging.rs::a_rejected_boundary_chain_does_not_take_its_blocks_other_chains |
| **F-U4-04** | PART OF | **Historical / closed — the second R1 precedent, and it came from #281's review, not from this pass.** A block's native effects were drained **one action per slot, credited to that slot's last writer**, so when two deploys of one block wrote the same slot the earlier deploy's write was a value no chain could reconstruct | `NativeStoreAction`'s drain kept one action per key (the trie refuses two), so the sidecar's per-chain effects held the **last** writer only → had resolution rejected that later chain and kept the earlier, the merged state could not have rebuilt the earlier's value: rejecting one chain would have silently dropped a *different* deploy's write, one layer below the defect this fix was for (and I2 could not see it, the earlier chain's slot set already having been emptied) | (closed) `Slot` carries per-deploy write **history** now, and the drain produces two views — `by_deploy`, each deploy's own last write per slot, and `all`, the checkpoint's one action per slot | Terminal | **R1** (worked precedent) | rspace/src/native_store.rs::two_deploys_writing_one_slot_keep_their_own_writes |
| **F-U4-05** | MORE | **A type whose order disagreed with its equality made two distinct chains one key** — `DeployChainIndex` ordered by `(host_block, post_state_hash)` and compared by `deploys_with_cost`, which Rust forbids because `BTreeMap`/`BTreeSet`/`sort` read the order as the identity | the port carried the Scala split deliberately and **pinned it as a known hazard** ("visible rather than latent"); the chain-level native relation is what made it fire on an ordinary block, since two chains of one block share both host and post-state → a set of chains silently held one of them, and the dependency map it built held a key whose own value looked that key up again | realized **twice**: as a hang (how it was found — CI's job limit, not a failing assertion) and, with the loop fixed alone, as a merge that keeps one of two distinct chains — a **silent** lost chain | Terminal | **R1** (one key for `Eq`/`Ord`/`Hash`) | casper/src/merging.rs::a_chains_order_agrees_with_its_equality |

### U5 — LFS sync & join / bootstrap

**Design intent.** A node with no state restores a consistent prefix (the finalized fringe plus
tuple-space) from a peer, then replays its own blocks to the same state hashes the rest of the network
holds.

**Guide words.** row: NO, REVERSE · folded: AS WELL AS → MORE, OTHER THAN → REVERSE, BEFORE → EARLY, AFTER → LATE · owed: PART OF, LATE, EARLY, MORE, LESS

No word is vacuous at this node — the acceptance sheet's §1 U5 has an empty vacuous set and was flagged
for a second pass for exactly that reason. The folds are its folds. **This node carries the R3 gap.**

### Table A — deviations

| ID | Guide word | Deviation | RCA → causal chain | Current behaviour | Class | Disposition | Falsifier |
|---|---|---|---|---|---|---|---|
| **F-U5-01** | NO | **There is no reset-and-resync path for a running node.** `NodeSyncing` is entered only at boot, when the DAG is empty; once running, a permanent divergence has no way back but deleting the shard data dir by hand | the boot guard is an `if` chain — `repr.dag_set.is_empty() && standalone` → genesis, `else if repr.dag_set.is_empty()` → `NodeSyncing::new` (`casper/src/engine/node_launch.rs:249`, `:265`), `else` → straight to `NodeRunning` (`:316`) — and `NodeSyncing::new` is called **nowhere else**; a running node that diverges stays in `NodeRunning` and permanently refuses the block and its descendants (`casper/src/multi_parent_casper.rs:883`, neglected-invalid rule), while `populate_dag` is reachable only from the sync task the boot branch starts (`casper/src/engine/node_syncing.rs:515`) | loud (`log.warn` per refused block) and **permanent** — the node keeps serving and never rejoins the chain | Terminal / Unrestorable | **R3** (owner: #287) | owed: a test that a diverged running node re-enters sync |
| **F-U5-02** | REVERSE | **The divergence has no inverse.** The refusal rules are absorbing: the only local undo is capped at three attempts and then the refusal is permanent for the process lifetime | `restore_is_warranted` requires `stored.restore_attempts < RESTORE_ATTEMPT_LIMIT` (`casper/src/multi_parent_casper.rs:478`, the constant `= 3` at `:434`), the count is persisted so it survives restart (`:430`), and `restore_divergent_justifications` (`:652`) is the only rule that could clear the refusal — after three it never fires again | loud, bounded, and then **absorbing** (C173's shape: `Persistent`) | Terminal / Persistent | **R3** | spec/Rchain/Casper/Stranding.lean::the_refusal_is_persistent — *the guard on the fix*: the theorem is stated over the rule set **without** a restoring rule, so adding one (which is what R3 is) reddens it by construction |

**F-U5-02 is the programme's sharpest instrument.** The Lean module is named `Stranding.lean`, and its
own comment records that `the_refusal_is_persistent` is *false by construction* once a step unmarks in
the relation. So the R3 fix has a falsifier that is red on the old behaviour **before it is written** —
which is the plan's verification requirement, already satisfied.

### U6 — Bonding, epoch boundary, PoS

**Design intent.** A bond or withdrawal takes effect exactly once, at the next epoch boundary, on every
node identically.

**Guide words.** row: EARLY · folded: LESS → NO, PART OF → NO, BEFORE → EARLY, AFTER → LATE · vacuous: REVERSE · owed: NO, MORE, AS WELL AS, LATE, OTHER THAN

The REVERSE vacuity is the acceptance sheet's (§1 U6: no inverse transition exists — a bond cannot debit
the pool, and finality/state undo is a stated deferral).

### Table A — deviations

| ID | Guide word | Deviation | RCA → causal chain | Current behaviour | Class | Disposition | Falsifier |
|---|---|---|---|---|---|---|---|
| **F-U6-01** | EARLY | At an epoch boundary the concurrent boundary chains each compute a pot, and **which sibling's effect survives still depends on which chains are in scope** — so a boundary that carried a deploy computes a different pot from its empty siblings | C248 moved the native relation onto the chain and deleted `reject_whole_blocks`, which stopped the *whole-block* loss; the boundary chains *themselves* still contend, so the merged pot is scope-dependent (C248's residue clause 1, stated there as "Aria's criterion 2 is NOT met and must not be read as met") | loud only if a violation lands in `MergeReport`; the scope-dependence itself is **unreported** | Split | R1 | owed: two merges of one round under two scopes, comparing the merged state at the already-finalised height (C248's residue names this falsifier and records that it is not written) |

### U7 — Deploy pool, gas & admission

**Design intent.** A deploy is admitted once, held while valid, included exactly once or expired; the
cost of admitting is bounded per unit time.

**Guide words.** row: LATE · folded: NO → MORE, AS WELL AS → LESS, AFTER → LATE · vacuous: OTHER THAN · owed: MORE, LESS, PART OF, REVERSE, EARLY, BEFORE

The OTHER THAN vacuity is the acceptance sheet's (§1 U7: the signature verifier refuses any non-canonical
spelling, so a deploy cannot be admitted under a sig the pool keys on *other than* the one `repeat_deploy`
dedups on).

### Table A — deviations

| ID | Guide word | Deviation | RCA → causal chain | Current behaviour | Class | Disposition | Falsifier |
|---|---|---|---|---|---|---|---|
| **F-U7-01** | LATE | **A deploy expiring at the boundary can make the proposer reject its own block and stall permanently** (#284, found by the #144 campaign's void arm) | the frozen campaign at `74cda38f1` processed 95–99 of 200 deploys and exposed a deploy-expiry boundary wedge; the proposer's own self-validation then refuses the block it just built, which is the shape the 2026-10-08 witness records as `the block's rejected-deploy set does not match its parent` | loud (the self-validation failure is logged) and **sticky** — the stall persists | Terminal | R2 + R3 | owed: #284's reproduction (the campaign arm that exposed it is preserved as VOID) |

**A hypothesis, stated as one.** #284's mechanism and the `#106` self-validation failure in the
2026-10-08 witness share a shape — a block whose rejected-deploy set disagrees with its parent — and #280
remains open on that failure. This pass does **not** claim they are one defect; the falsifier that would
settle it is #280's log at `#100–#106`, which the witness partly supplies (§6).

### U8 — Wire protocol, transport, discovery

**Design intent.** A message from a bounded-adversary peer is authenticated, bounded in size and rate, and
cannot make the node do unbounded work.

**Guide words.** row: PART OF, MORE, NO · folded: LESS → MORE, OTHER THAN → PART OF · owed: REVERSE, AS WELL AS, BEFORE, AFTER, EARLY, LATE

No word is vacuous at this node — the acceptance sheet's §1 U8 contests its own REVERSE vacuity (CH-U8-02)
and records a candidate deviation instead; this pass does not reopen it. The folds are its folds.

### Table A — deviations

| ID | Guide word | Deviation | RCA → causal chain | Current behaviour | Class | Disposition | Falsifier |
|---|---|---|---|---|---|---|---|
| **F-U8-01** | PART OF | **`send_to_validate` swallows a store-read error** — an unreadable block in the batch is dropped with no log, no re-queue and no counter | `block_store.get(&[*hash]).await.ok()` (`casper/src/blocks/block_receiver.rs:377`) discards the `Err`; only `Some(block)` reaches `put_to_incoming_queue` (`:383`), so the hash is consumed by the batch and never re-requested | **silent** — the one genuinely silent path this pass found (no `log` call at all on the error arm) | Terminal | R2 | owed: a test that an unreadable block is logged and re-requested |
| **F-U8-02** | MORE | **A full ingress queue drops the block with a warning** — `try_send(...).is_err()` covers "full" and "receiver dropped" alike, and the block is gone | `casper/src/engine/node_running.rs:664` (BlockMessage arm; the same shape at `:746` for HasBlock) warns and returns; the bounded queue is `MAX_PENDING_BLOCKS` (`node/src/runtime/node_runtime.rs:852`) | loud (`log.warn`) and **actionless**: no re-request, no back-pressure on the sender | Terminal | R2 | owed: a test that a dropped ingress block is re-requested or the peer is told to slow |
| **F-U8-03** | NO | **The block retriever forgets a hash at capacity** — `CapacityReached` returns the state unchanged, so the hash is never recorded and never requested | `admit_hash` returns early when `state.len() >= MAX_REQUESTED_BLOCKS` (`casper/src/blocks/block_retriever.rs:116`) → `CapacityReached` is handled by a bare warn (`:227`) and the hash is neither inserted nor broadcast | loud (`log.warn`) and **forgetting**: recoverable only if a peer re-offers the same hash | Terminal | R2 | owed: a test that a capacity-reached hash is retained for a later request |

### U9 — State, storage & DAG residency

**Design intent.** The DAG and its stores retain exactly what the liveness and finality rules may still
need, at a residency an operator can size for.

**Guide words.** row: MORE, PART OF · folded: NO → MORE, AS WELL AS → MORE, REVERSE → MORE, OTHER THAN → MORE, EARLY → PART OF, BEFORE → PART OF, AFTER → LATE · owed: LATE, LESS

The folds are the acceptance sheet's (§1 U9 — absence of a release path *is* the over-retention; the
leaf-before-root commit *is* the partial-atomicity deviation). Its own LESS reason is recorded there as
false and reopened as `owed` here rather than inherited.

**A note on scope.** The acceptance sheet's node list has no *process-resource* node, so this pass rows
the node's retention and resource bounds here as well. The memo caches, the lock's poison policy and the
blocking pool are not stores — but they are the same question the design intent asks ("at a residency an
operator can size for"), and a node that dies of any of them reaches TE-4 identically.

### Table A — deviations

| ID | Guide word | Deviation | RCA → causal chain | Current behaviour | Class | Disposition | Falsifier |
|---|---|---|---|---|---|---|---|
| **F-U9-01** | MORE | **The global insert lock is held across all store I/O and both representation locks**, so one slow insert serializes *all* block admission | `insert` binds `let _guard = self.lock.lock().await` (`casper/src/dag.rs:489`) and that guard drops only at the end of the body (`:696`); inside it are sixteen further `.await` points and `representation.read().await` (`:502`, `:554`) and `.write().await` (`:649`), with the author's own comment "this function is serialized by `self.lock`" (`:647`) | not loud and not silent: the node is *slow*, not wedged, until a slow store makes admission stop entirely | Drift → Terminal | R2 | owed: a bound or a timing assertion on admission under a slow store |
| **F-U9-02** | PART OF | **`update_metadata` holds the representation write lock across an LMDB read**, against the discipline `insert` documents — so the lock discipline holds for *part* of the module | `let mut guard = self.representation.write().await` (`casper/src/dag.rs:729`) then `repr.height_map = self.block_metadata_store.height_map().await` (`:736`); `insert` documents the opposite at `:547` ("the guard is a *block expression* rather than a binding … a binding left in scope would hold the read lock across `fringe_data_store.put`") | not loud; a held write lock across an await is a stall if the store stalls | → Terminal | R2 | owed: a test or lint that a representation guard is not held across an await |
| **F-U9-03** | MORE | **A poisoned lock is recovered, not refused** — `rlock`/`wlock`/`mlock` call `unwrap_or_else(PoisonError::into_inner)`, so after a panic in another thread a **torn state is used silently** | `rspace/src/lock.rs:10` (read), `:16` (write), `:20` (mutex) all recover; the stores funnel through these accessors (`rspace/src/history/radix_tree.rs`, `rspace/src/rspace.rs`), so the policy is one function rather than a scatter — which is why one edit changes it | **silent** — no log, no counter, and a caller cannot tell a poisoned lock from a healthy one; the code does more than the intent (which is to refuse a torn state) | Terminal | R2 | owed: a poisoned lock must be an error or an event, never a silent recovery |
| **F-U9-04** | MORE | **The memo caches have no count bound** — the radix read/write caches and the hot-store channel memo grow until a checkpoint clears them, in a struct with no eviction path | `RadixTreeImpl`'s `cache_read`/`cache_write` are plain `HashMap`s (`rspace/src/history/radix_tree.rs:152`), the write cache cleared only at the end of `save_and_commit` (`:660`) and the read cache only from `RadixHistory::process` (`rspace/src/history/instances/radix_history.rs:73`); the channel memo is a `HistoryStoreCache` behind a `Mutex` with **no** `clear` method (`rspace/src/hot_store.rs:80`, `:137`), dropped only when the whole `InMemHotStore` is rebuilt at `create_checkpoint`/`reset`/`revert_to_soft_checkpoint` (`rspace/src/rspace.rs:627`, `:649`, `:702`) | not loud: the growth is invisible until memory is exhausted (TE-4) | Drift | R2 | owed: a count bound per cache, with the bound as an observable |
| **F-U9-05** | MORE | **The blocking pool is uncapped** — `spawn_blocking` runs on tokio's 512-thread default, each thread on the runtime's 32 MiB stack | the runtime is built without `max_blocking_threads` (`node/src/main.rs:91`; `thread_stack_size(32 * 1024 * 1024)` at `:92`), the setting appears **nowhere** in the workspace, and the production sites are `node/src/runtime/node_main.rs:45`, `:69` | not loud: a thread explosion reads as a slow node, and then as an OOM (TE-4) | Drift → Terminal | R2 | owed: a `max_blocking_threads` bound, set and tested |

### U10 — Operator surface

**Design intent.** An operator can observe block production, finality and the proposer-halt state, and
restart the node back to the same chain state.

**Guide words.** row: NO, AFTER, OTHER THAN · folded: REVERSE → NO, EARLY → LESS, AS WELL AS → PART OF, BEFORE → LATE · owed: MORE, LESS, PART OF, LATE

No word is vacuous at this node (the acceptance sheet's §1 U10 has none; its BEFORE/AFTER fold is
contested there in CH-U10-03 and this pass does not reopen it). The folds are its folds.

### Table A — deviations

| ID | Guide word | Deviation | RCA → causal chain | Current behaviour | Class | Disposition | Falsifier |
|---|---|---|---|---|---|---|---|
| **F-U10-01** | NO | **The wedge has no machine-readable surface**: the finality-stall *reason* (`NoAdvance`'s three exits) is log-only, so `/api/status` cannot say *why* finality stopped | `describe_no_advance` (`casper/src/interpreter_util.rs:428`) has exactly one call site, the warn at `:510`; the reason is carried on `ParentsMergedState.finality_stall` (`casper/src/merging.rs:419`, assigned `casper/src/multi_parent_casper.rs:347`) and reaches no response type — `ApiStatus` (`node/src/api/dto.rs:74`) has no field for it, and `/metrics` has no gauge | loud (`log.warn`, rate-limited, re-firing on change) and **only** in the log | Terminal | R2 | owed: a stall-reason field on `/api/status` (the three `ProposerHealth` counters are already there — `node/src/api/conversion.rs:49` — so the surface exists) |
| **F-U10-02** | AFTER | **The halt is sticky and not persisted**, so a restart makes the surface forget a halt it just reported | `ProposeHealth` is three `Arc<Atomic*>` fields (`casper/src/api/block_api.rs:85`); `note_timer_halted` is the only write (`:111`, called from `node/src/runtime/node_runtime.rs:1745`) and there is **no `store(false)` anywhere**; the process reads a fresh `false` after a restart | loud while the process lives; **forgotten after a restart** — an operator restarting to clear an alarm gets a false all-clear | Historic | R2 | owed: a test that a halt survives, or is re-derived at, a restart |
| **F-U10-03** | OTHER THAN | **`[pos]` diagnostics bypass the log sink** — three sites write to stderr with `eprintln!`, not the `Log` source, so they are outside rate-limiting, log routing and any scrape | `casper/src/blocks/proposer/proposer.rs:232`, `:861`, `:897` all call `eprintln!` where the module has a `log` in scope | loud on the terminal, invisible to every surface that reads the `Log` sink | —  | R2 | owed: route the three `[pos]` sites through the module's `Log` source |

## 4. The fault tree

```
                    the node cannot repair itself
                                │
    ┌────────────┬──────────────┼──────────────┬──────────────┐
    │            │              │              │              │
 the merge    the file       the lock      the operator   the buffers
 refuses      ingests       serializes     cannot see     never shrink
    │            │              │              │              │
 F-U4-01..05  F-U8-01        F-U9-01        F-U10-01       F-U9-04 caches
 F-U1-01      F-U8-02        F-U9-02        F-U10-02       F-U9-05 pool
 F-U5-02      F-U8-03        F-U9-03        F-U10-03       F-U2-01 guard
    │         F-U2-01           │              │          vs partition
    │            │              └──────┬───────┘              │
    └────────────┴─────────────────────┤                      │
                                       │                      │
                     ┌─────────────────┼──────────────────────┘
                     │                 │
                a NODE stalls     the CHAIN stalls       the net runs out
                 (TE-5)            (TE-1, TE-6)              (TE-4)
                     │                 │                       │
                F-U5-01 no reset   F-U3-01 no fringe      R2's bounds —
                path once running  to restore from, and    the three
                     │             F-U5-01's absence is    unbounded ones
                   ── R3 ──        what makes it permanent  are the whole
                (owner: #287)             │                 finding
                                      ── R3 ──
                                   (owner: #287)
```

**The tree's one sentence.** Every branch ends in the same place: a state the node cannot leave, or a
resource no rule releases — reached with a log line at most, and with no operation to undo it. The
loudness is nearly all won; the undoing is entirely owed.

## 5. The R3 gap, and its owner

**The gap.** A node that cannot locally heal has, today, exactly one way back: an operator stops it and
deletes the shard data dir by hand, so `dag_set` empties and `NodeSyncing` runs. That path is written
down — as a *comment*, in `casper/src/dag.rs:534` ("the only way back is deleting the shard data dir so
`dag_set` empties and `NodeSyncing` runs") — and it is the only one. For the *chain* (TE-6) there is no
path at all: the 2026-10-08 witness recorded four divergent heads, one restart experiment on two nodes
(which restored production and added two more heads), and finality frozen at 101 throughout.

**The owner.** The reconciliation utility is **#287** — "bring a busted network back to one chain without
a genesis". Its design is the R3 rung stated as an operator tool: read `last-finalized-block` from every
node and **stop if they disagree**; extract the deploys above it through the block APIs; stop the nodes,
wipe, resync from the finalised fringe; re-submit the extracted deploys in order and report every one
that fails; verify that all nodes agree on height **and block hashes**. Its invariants are this page's
TE-5/TE-6 separation in one line: *never* treat an unfinalised block as the truth, and *never* drop a
deploy silently.

**What stays in the tree when #287 lands.** The utility is the operator's tool (R4's runbook, made
executable). The *node-side* rung — a running node that detects its own unrecoverable divergence and
resets itself — is F-U5-01, and it is a separate unit: #287 repairs a chain from outside, while F-U5-01
would let a node rejoin one from inside. They share the detection (a divergence record that exhausts its
restore budget) and the reset primitive; they do not share a deliverable.

**Ships dark.** A reset is destructive. If F-U5-01 is built, it ships detect + log + count first, so an
operator sees what it *would* have done before it is allowed to do it — and F-U10-01's stall-reason
surface is the prerequisite, not a nicety.

## 6. What this pass corrects in the record

1. **The 2026-10-08 incident had two faces, and the register carries one.** C248's row says *"It is not
   C215: no two-node divergence, no `InvalidPreStateHash` between peers"*. The TE-1 witness for the same
   incident records the opposite above the write loss: `state-hash disagreement on pre-state: block
   #104` with three different `#104` hashes cited, and four distinct `#106` blocks. **Adjudication**: the
   clause is true of the *write-loss mechanism* (which is why no node refused a block at height 103) and
   false as a statement about the incident. #280 remains open on the second failure, and this is recorded
   rather than silently corrected.
2. **The log C248's residue clause 4 asked for now exists.** That clause says *"#280's second failure
   (validators stopping production) is not causally closed — it needs node A's log around #106."* The
   witness contains exactly that line — `ERROR Self-created block #106 (seq 106) failed validation: the
   block's rejected-deploy set does not match its parent` — and the two restart experiments that show the
   pre-restart failure was transient *process* state. The register's residue is therefore narrower than
   it reads.
3. **The witness mis-cites the reconciliation issue.** It says *"a candidate mechanism is proposed in the
   reconciliation issue (#283)"*. #283 is `perf(#144): measure soft-checkpoint cost before choosing the
   first TPS lever`; the reconciliation utility is **#287**. A citation defect in a committed artifact —
   the class the OCapN HAZOP recorded as R4, and the reason §7's provenance is a list rather than a
   gesture.
4. **The two documents label the same validators with different letters.** The #280 capture fixes
   `A 0410b8c5, B 041ed2a2, C 04d7707c, D 04dce59b`; the witness calls `041ed2a2` **D**, `04d7707c`
   **B** and `04dce59b` **C**. Same keys, rotated letters — so a reader cannot join the two by name, and
   the witness's "A and D share a host" conclusion is about `0410b8c5` and `04dce59b`, not about the
   capture's A and D. Worth fixing in the witness before it merges.
5. **The programme's own seed catalogue carries a claim this pass refutes.** It listed *"Unbounded
   merge-scope search — measured (`search_census`) but not bounded"* as an R1/R2 candidate. The
   measurement is real and **the bound is too**: `SearchBudget::NODE` (10,000,000 steps / 1,000,000
   options, `sdk/src/dag/merging.rs:280`) is applied to the merge search at `casper/src/merging.rs:1851`,
   and *exceeding* it is precisely the drop F-U4-01 rows. What is unbounded is the **neighbouring fold**,
   which the acceptance sheet already records: `fold_rejection`/`traverse_tree` take no budget and
   `traverse_tree` walks with no visited set (H-U4-04, U4's `PART OF`, `owed` here). So the seed's
   conclusion was right about the node and wrong about the site — it pointed at the one place a budget
   already exists.
6. **Two of the rows above were not found by this pass, and the page says so rather than implying
   otherwise.** F-U4-04 came from #281's review and F-U4-05 from the fix for the CI hang that review led
   to. Both deserve their row and both were missed here, and the reason is specific enough to be useful:
   this pass looked for hazards in **behaviour** — what a node does when a store read fails, a lock is
   poisoned, a search is refused — and neither of these is a behaviour. One is a **sidecar** that kept one
   action per slot; the other is a **type** whose order disagreed with its equality. The class they share
   is the one the ladder's first rung exists for, and it is reachable only by reading the types rather
   than the control flow — which is the pass this page has not yet run.

## 7. Provenance

read: `casper/src/merging.rs` (the three refusal sites, `MergeReport`, I1/I2, `MergeOutcome`),
`casper/src/blocks/block_processor.rs` (the drop arm and the deferral note), `casper/src/multi_parent_casper.rs`
(`ValidateError`, the `:850` funnel, `RESTORE_ATTEMPT_LIMIT`, `restore_is_warranted`, `mark_failed`, the
neglect rule), `casper/src/interpreter_util.rs` (`validate_block_checkpoint`, `describe_no_advance`, the
merge-report warn, the stall warn), `casper/src/dag.rs` (`insert` and its discipline comment,
`update_metadata`, the shard-dir comment), `casper/src/engine/node_launch.rs` (the boot `if` chain),
`casper/src/engine/node_syncing.rs` (`MAX_SYNC_ATTEMPTS`, the terminal stop, `populate_dag`),
`casper/src/blocks/block_receiver.rs`, `casper/src/blocks/block_retriever.rs`,
`casper/src/engine/node_running.rs`, `casper/src/blocks/proposer/proposer.rs`, `casper/src/api/block_api.rs`,
`casper/src/protocol/comm_util.rs`, `block-storage/src/dag/finalizer.rs`, `block-storage/src/dag/liveness.rs`,
`node/src/api/dto.rs`, `node/src/api/conversion.rs`, `node/src/runtime/node_runtime.rs`,
`node/src/runtime/node_main.rs`, `node/src/main.rs`, `sdk/src/dag/merging.rs` (`SearchBudget`),
`rspace/src/lock.rs`, `rspace/src/history/radix_tree.rs`, `rspace/src/history/instances/radix_history.rs`,
`rspace/src/hot_store.rs`, `rspace/src/rspace.rs`, `spec/Rchain/Casper/Stranding.lean`,
`docs/src/formal/progress.md` (the shape/cause vocabulary), `docs/src/spec/testnet-acceptance.md` §1–§2
(the study nodes, the top events, the TE-1 recovery verdict),
`spec/audit/evidence/n280-merge-loses-a-write-results.md`, the TE-1 witness at
`spec/audit/evidence/te-1-2026-10-09-four-divergent-heads.md` **on branch `spec/te-1-witness`** — it is
not in this tree, and every claim taken from it says so — and the reconciliation issue **#287**. ·
ran: `git rev-parse HEAD`, `git status`,
`git fetch`, `git show origin/spec/te-1-witness:<path>`, `grep`/`ls`/`sed` over the files above (read
only), and `tools/check-hazop-worksheet.sh`. The anchors were re-derived in this pass; the ones on
`casper/src/dag.rs:489/696/729/736` and the two test names in §3's F-U4-03/F-U5-02 were re-read directly
after the earlier reads, because a citation this page publishes is a claim.

not read: the whole of `casper/src/merging.rs` (3,800+ lines), `casper/src/dag.rs`, and
`casper/src/multi_parent_casper.rs` — the named functions only; `spec/AUDIT.md`'s and
`spec/findings.tsv`'s C248 row in the register's emitted form; the remaining `spec/audit/evidence/`
runs; and the `rspace` history and store internals beyond the named cache and lock sites.

**Debt.** falsifiers owed: 19 · node-words owed: 44

*(A `F-` row with an `owed` falsifier is a deviation this pass **dispositioned** but did not **prove**;
the number above is checked against the page by `tools/check-hazop-worksheet.sh`, so it cannot rot.
`node-words owed` counts the acceptance worksheet's `row`-declared guide words this pass has not yet
reached.)*
