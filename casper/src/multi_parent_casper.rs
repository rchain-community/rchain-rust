//! The multi-parent CBC-Casper façade (port of `MultiParentCasper.scala`).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use rchain_block_storage::block_store::BlockStore;
use rchain_block_storage::dag::dag_storage::{BlockDagStorage, DeployId};
use rchain_block_storage::dag::finalizer::{Finalizer, Message};
use rchain_block_storage::dag::liveness;
use rchain_block_storage::dag::message_map;
use rchain_crypto::hash::blake2b256_hash::Blake2b256Hash;
use rchain_models::block::state_hash::StateHash;
use rchain_models::block_hash::BlockHash;
use rchain_models::block_metadata::{BlockMetadata, FailureCause, SlashSeverity};
use rchain_models::casper::protocol::casper_message::{BlockMessage, SignedDeployData};
use rchain_models::fringe_data::FringeData;
use rchain_models::normalizer_env::NormalizerEnv;
use rchain_models::validator::Validator;
use rchain_shared::log::{Log, LogSource};
use rchain_shared::refined::BlockHeight;

use crate::block_status::BlockStatus;
use crate::interpreter_util::validate_block_checkpoint;
use crate::merging::{
    BlockIndex, DeployChainIndex, MergeOutcome, MergeReport, MergeScope, ParentsMergedState,
};
use crate::runtime_manager::RuntimeManager;

/// A deploy-parsing error (port of `ParsingError`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsingError(pub String);

/// A block-validation failure (port of `MultiParentCasper.validate`'s error channel).
///
/// A validation failure marks the block invalid but still returns its (failed) metadata so the DAG
/// can record it; an internal error is a store/runtime lookup failure with no block outcome.
#[derive(Clone, Debug)]
pub enum ValidateError {
    ValidationFailed(BlockMetadata, BlockStatus),
    /// A self-created block was refused on insert as an equivocation: the sequence number it derived
    /// from a now-stale parent-set snapshot is already taken by its own earlier block. Not a
    /// validation failure — the block is internally consistent, the DAG just advanced under it — so
    /// the proposer treats it as "not due" rather than counting it toward the halt (§48). Only the
    /// proposer's self-insert produces this; `validate` itself never returns it.
    SelfEquivocation,
    Internal(String),
}

/// The size of the deploy safety range (port of `deployLifespan`).
pub const DEPLOY_LIFESPAN: i64 = 50;

/// Build a `ParsingError` from details (port of `parsingError`).
pub fn parsing_error(details: impl Into<String>) -> ParsingError {
    ParsingError(format!("Parsing error: {}", details.into()))
}

/// Look up the last finalized block (port of `lastFinalizedBlock`).
pub async fn last_finalized_block(
    dag: &dyn BlockDagStorage,
    block_store: &BlockStore,
) -> Result<BlockMessage, String> {
    let repr = dag.get_representation().await;
    let hash = repr
        .last_finalized_block_hash()
        .ok_or_else(|| "no finalized block in the DAG".to_string())?;
    let mut vals = block_store.get(&[hash]).await?;
    vals.pop()
        .flatten()
        .ok_or_else(|| format!("missing finalized block {}", hash.to_hex()))
}

/// Add a deploy to the deploy pool and return its id (port of `addDeploy`).
pub async fn add_deploy(
    dag: &dyn BlockDagStorage,
    deploy: &SignedDeployData,
) -> Result<DeployId, String> {
    dag.add_deploy(deploy.clone()).await?;
    Ok(deploy.sig.clone())
}

/// Parse-check a deploy term, then add the deploy to the pool (port of `deploy`).
pub async fn deploy(
    dag: &dyn BlockDagStorage,
    deploy: &SignedDeployData,
) -> Result<DeployId, ParsingError> {
    // Normalize against the deploy's environment (deployer id / deploy id), so a term that
    // references `rho:rchain:deployerId`/`deployId` parses the same way it will when processed.
    let normalizer_env = NormalizerEnv::new(deploy);
    rchain_rholang::normalizer::source_to_adt_with_env(&deploy.data.term, normalizer_env.to_env())
        .map_err(|e| parsing_error(format!("Error in parsing term: \n{e}")))?;
    add_deploy(dag, deploy).await.map_err(parsing_error)
}

async fn get_block_unsafe(
    block_store: &BlockStore,
    hash: &BlockHash,
) -> Result<BlockMessage, String> {
    let mut vals = block_store.get(&[*hash]).await?;
    vals.pop()
        .flatten()
        .ok_or_else(|| format!("missing block {}", hash.to_hex()))
}

/// Compute the merged pre-state for a set of parent blocks (port of `getPreStateForParents`).
pub async fn get_pre_state_for_parents<F, Fut>(
    dag: &dyn BlockDagStorage,
    block_store: &BlockStore,
    runtime: &RuntimeManager,
    parent_hashes: &BTreeSet<BlockHash>,
    block_index: &F,
) -> Result<ParentsMergedState, String>
where
    F: Fn(BlockHash) -> Fut,
    Fut: std::future::Future<Output = Result<Arc<BlockIndex>, String>>,
{
    if parent_hashes.is_empty() {
        return Err(
            "Parents must not be empty to calculate pre-state. Genesis block pre-state is loaded from config."
                .to_string(),
        );
    }

    let dag_repr = dag.get_representation().await;
    let msg_map = &dag_repr.dag_message_state.msg_map;

    let mut justifications: Vec<BlockMetadata> = Vec::new();
    for h in parent_hashes {
        let meta = dag
            .lookup(h)
            .await?
            .ok_or_else(|| format!("missing justification {}", h.to_hex()))?;
        justifications.push(meta);
    }

    let parents: BTreeSet<Message<BlockHash, Validator>> = parent_hashes
        .iter()
        .map(|h| {
            msg_map
                .get(h)
                .cloned()
                .ok_or_else(|| format!("parent not in message map: {}", h.to_hex()))
        })
        .collect::<Result<_, String>>()?;

    // Currently finalized fringe.
    let prev_fringe = message_map::latest_fringe(msg_map, &parents);
    let prev_fringe_hashes: BTreeSet<BlockHash> = prev_fringe.iter().map(|m| m.id).collect();
    // **The participation the epoch's absence rule reads** (B4, #150), taken here rather than at the
    // struct literal below: this is the last point at which the fringe's `Message` set — and not merely
    // its block hashes — is still alive, and it must come from the *same* `prev_fringe` the state hash
    // is looked up under. Deriving it later, or from `dag_repr.latest_fringe()` separately, is the one
    // way play and replay could disagree about which fringe was meant. A validator with no message in
    // the fringe is simply absent from the map, which the rule reads as silence.
    let participation: BTreeMap<Validator, BlockHeight> =
        liveness::latest_heights(prev_fringe.iter().map(|m| (m.sender, m.height)));
    let fringe_record = dag_repr
        .fringe_states
        .get(&FringeData::fringe_hash_of(&prev_fringe_hashes))
        .ok_or_else(|| {
            format!(
                "Fringe state not available in state cache, fringe: {:?}",
                prev_fringe_hashes
            )
        })?;
    let prev_fringe_state = fringe_record.state_hash;
    let prev_fringe_rejected_deploys = fringe_record.rejected_deploys.clone();
    // Captured here rather than at the struct literal, because `prev_fringe_hashes` is moved into
    // `new_fringe` below (#139). These two are what a state disagreement is reported with: the fringe
    // the node began from, and the key it looked that fringe up under.
    let prev_fringe_for_report = prev_fringe_hashes.clone();
    let prev_fringe_lookup = FringeData::fringe_hash_of(&prev_fringe_hashes);

    // Bonds map: from the newest justification's *state* while nothing has finalised, else from the PoS
    // contract at the fringe.
    let bonds_map = if prev_fringe.is_empty() {
        // There is no fringe state to ask, so take the newest justification's state — not the bond map
        // carried inside the blocks. Those are each claimant's own view, and they *necessarily* disagree
        // the moment the bond set changes, because the justifications are the validators' latest messages
        // and they straddle the change. Requiring them to agree therefore wedged a chain permanently on
        // the first bond or withdrawal before its first finalisation, with no way back (#73). A block's
        // state follows from the DAG alone, so honest nodes agree on it by construction.
        //
        // The genesis never reaches this branch: it has no parents, and an empty parent set is refused
        // above (its pre-state comes from config).
        let newest = newest_justification(&justifications)
            .ok_or_else(|| "no justifications to read the bonds map from".to_string())?;
        let newest_block = get_block_unsafe(block_store, &newest.block_hash).await?;
        let state_hash = StateHash::from_slice(newest_block.post_state_hash.as_bytes());
        match runtime.compute_bonds(&state_hash).await {
            Ok(bonds) => bonds,
            // The newest parent's state is not readable (pruned history). Fall back to the maps the
            // justifications carry, and to the rule that they must agree: that is what this branch did
            // unconditionally before, so a chain in this state behaves exactly as it used to.
            Err(err) => {
                let mut iter = justifications.iter().map(|j| j.bonds_map.clone());
                let first = iter.next().unwrap_or_default();
                for other in iter {
                    if other != first {
                        return Err(format!(
                            "justifications disagree on the bonds map, and the newest justification's \
                             state is unavailable: {err}"
                        ));
                    }
                }
                first
            }
        }
    } else {
        let state_hash = StateHash::from_slice(prev_fringe_state.as_bytes());
        runtime.compute_bonds(&state_hash).await?
    };

    // If a new fringe is finalized, merge it.
    //
    // Through the liveness rule (`liveness`), so the partition a candidate must satisfy ranges over the
    // bonded validators that are **still speaking** while the quorum stays the whole bonded map: a
    // validator that has stopped producing messages no longer caps finality, and a minority still
    // cannot finalise alone (#70). This is the same call the creator makes, which is what keeps a
    // block's `fringe` and this node's derivation of it the same value.
    let finalizer = Finalizer::new(msg_map);
    let (_parent_fringe, new_fringe_opt, no_advance) =
        liveness::calculate_finalization_detailed(&finalizer, &parents, &bonds_map);
    let new_fringe_hashes: Option<BTreeSet<BlockHash>> =
        new_fringe_opt.map(|f| f.iter().map(|m| m.id).collect());

    let new_fringe_result = match &new_fringe_hashes {
        Some(fringe) => {
            let (m_scope, base_opt) =
                MergeScope::from_dag(fringe, &prev_fringe_hashes, &dag_repr.child_map, msg_map)?;
            let base_state = match base_opt {
                Some(h) => Blake2b256Hash::from_byte_array(
                    get_block_unsafe(block_store, &h)
                        .await?
                        .post_state_hash
                        .as_bytes(),
                ),
                None => prev_fringe_state,
            };
            let result = MergeScope::merge(
                &m_scope,
                base_state,
                &dag_repr.fringe_states,
                runtime.get_history_repo(),
                block_index,
                DeployChainIndex::deploy_chain_cost,
            )
            .await?;
            Some(result)
        }
        None => None,
    };
    // The fringe merge's outcome, or the previous fringe's when nothing finalised. Its **report** is
    // carried rather than logged here: this function has no logger (#280).
    let fringe_merge = new_fringe_result.unwrap_or(MergeOutcome {
        state: prev_fringe_state,
        rejected_deploys: prev_fringe_rejected_deploys,
        report: MergeReport::default(),
    });
    let (fringe_state, fringe_rejected_deploys) =
        (fringe_merge.state, fringe_merge.rejected_deploys.clone());

    let max_height = justifications
        .iter()
        .map(|m| i64::from(m.block_num))
        .max()
        .unwrap_or(-1);
    let max_seq_nums: BTreeMap<Validator, i64> = justifications
        .iter()
        .map(|m| (m.sender, i64::from(m.seq_num)))
        .collect();
    let new_fringe = new_fringe_hashes.unwrap_or(prev_fringe_hashes);

    // Merge the conflict scope (non-finalized blocks above the fringe).
    let conflict_merge = if parent_hashes.len() == 1 {
        let parent = parent_hashes
            .iter()
            .next()
            .ok_or_else(|| "expected one parent".to_string())?;
        let block = get_block_unsafe(block_store, parent).await?;
        MergeOutcome {
            state: Blake2b256Hash::from_byte_array(block.post_state_hash.as_bytes()),
            rejected_deploys: BTreeSet::new(),
            report: MergeReport::default(),
        }
    } else {
        let (m_scope, base_opt) =
            MergeScope::from_dag(parent_hashes, &new_fringe, &dag_repr.child_map, msg_map)?;
        let base_state = match base_opt {
            Some(h) => Blake2b256Hash::from_byte_array(
                get_block_unsafe(block_store, &h)
                    .await?
                    .post_state_hash
                    .as_bytes(),
            ),
            None => fringe_state,
        };
        MergeScope::merge(
            &m_scope,
            base_state,
            &dag_repr.fringe_states,
            runtime.get_history_repo(),
            block_index,
            DeployChainIndex::deploy_chain_cost,
        )
        .await?
    };
    let (pre_state_hash, cs_rejected_deploys) = (
        conflict_merge.state,
        conflict_merge.rejected_deploys.clone(),
    );
    // **The report an operator gets, and where it comes from** (#280). Both merges' — the fringe's
    // and the conflict scope's — because either can be the one that drops something, and the
    // conflict scope's is the one that did on the live net.
    let merge_report = MergeReport {
        conflict_chains: fringe_merge.report.conflict_chains
            + conflict_merge.report.conflict_chains,
        kept_chains: fringe_merge.report.kept_chains + conflict_merge.report.kept_chains,
        rejected_chains: fringe_merge.report.rejected_chains
            + conflict_merge.report.rejected_chains,
        dropped_native_slots: fringe_merge
            .report
            .dropped_native_slots
            .iter()
            .chain(conflict_merge.report.dropped_native_slots.iter())
            .cloned()
            .collect(),
        rejected_cost_accounted_chains: fringe_merge.report.rejected_cost_accounted_chains
            + conflict_merge.report.rejected_cost_accounted_chains,
        rejections_without_a_conflict: fringe_merge
            .report
            .rejections_without_a_conflict
            .iter()
            .chain(conflict_merge.report.rejections_without_a_conflict.iter())
            .cloned()
            .collect(),
        unapplied_kept_writes: fringe_merge
            .report
            .unapplied_kept_writes
            .iter()
            .chain(conflict_merge.report.unapplied_kept_writes.iter())
            .copied()
            .collect(),
        native_writer_of_slot: conflict_merge.report.native_writer_of_slot.clone(),
    };

    Ok(ParentsMergedState {
        finality_stall: no_advance,
        merge_report,
        justifications,
        max_block_num: max_height,
        max_seq_nums,
        fringe: new_fringe,
        fringe_state,
        // Where this computation started (#139): the fringe the node's own DAG named, and the cache
        // key it looked that fringe up under. `prev_fringe` empty is the restore-shape signature.
        prev_fringe_lookup,
        prev_fringe: prev_fringe_for_report,
        fringe_bonds_map: bonds_map,
        participation,
        fringe_rejected_deploys,
        pre_state_hash,
        rejected_deploys: cs_rejected_deploys,
    })
}

/// **The participation an epoch boundary's absence rule reads** (B4, #150), derived from the DAG for a
/// block whose merged pre-state is not in hand.
///
/// It exists for the index-regeneration replay, which carries a `fringe_state_hash` but no
/// `ParentsMergedState` — and it is deliberately the *same* derivation `get_pre_state_for_parents`
/// performs, down to the `validation_failed` filter `validate_block_checkpoint` applies before it asks
/// for a pre-state. It has to be: `regenerate_sidecars` compares the post-state hash it replays against
/// the block's own, so a map derived any other way is a different post-state and the block becomes
/// un-indexable.
pub async fn participation_for_block(
    dag: &dyn BlockDagStorage,
    block: &BlockMessage,
) -> Result<BTreeMap<Validator, BlockHeight>, String> {
    let dag_repr = dag.get_representation().await;
    let msg_map = &dag_repr.dag_message_state.msg_map;
    let mut parents: BTreeSet<Message<BlockHash, Validator>> = BTreeSet::new();
    for j in &block.justifications {
        if let Some(meta) = dag.lookup(j).await? {
            if meta.validation_failed {
                continue;
            }
            let m = msg_map.get(&meta.block_hash).cloned().ok_or_else(|| {
                format!("parent not in message map: {}", meta.block_hash.to_hex())
            })?;
            parents.insert(m);
        }
    }
    // An empty parent set has no fringe, and no participation: the genesis shape, and the arm
    // `validate_block_checkpoint` handles separately.
    let fringe = message_map::latest_fringe(msg_map, &parents);
    Ok(liveness::latest_heights(
        fringe.iter().map(|m| (m.sender, m.height)),
    ))
}

/// Compute the pre-state for a new block from the DAG's latest messages (port of
/// `getPreStateForNewBlock`).
pub async fn get_pre_state_for_new_block<F, Fut>(
    dag: &dyn BlockDagStorage,
    block_store: &BlockStore,
    runtime: &RuntimeManager,
    block_index: &F,
    sender: &Validator,
    escape: bool,
) -> Result<ParentsMergedState, String>
where
    F: Fn(BlockHash) -> Fut,
    Fut: std::future::Future<Output = Result<Arc<BlockIndex>, String>>,
{
    let dag_repr = dag.get_representation().await;
    // **The round snapshot, not `latest_msgs`.** The fringe gate cannot finalise a parent set made of
    // every sender's newest message — see `DagMessageState::round_parents` — so the proposer justifies
    // the messages as of the last round boundary instead.
    let parents = if escape {
        dag_repr
            .dag_message_state
            .parents_for_new_block_escaping(sender)
    } else {
        dag_repr.dag_message_state.parents_for_new_block()
    };
    let parent_hashes: BTreeSet<BlockHash> = parents.into_iter().map(|m| m.id).collect();
    get_pre_state_for_parents(dag, block_store, runtime, &parent_hashes, block_index).await
}

/// How many times one record may be re-validated before the node gives up on it.
///
/// The count is **persisted** on the metadata ([`BlockMetadata::restore_attempts`]), so a restart
/// resumes the budget rather than handing the record a fresh one.
pub const RESTORE_ATTEMPT_LIMIT: u32 = 3;

/// How many records **one incoming block** may cause a revalidation of.
///
/// This is the bound that keeps the rule out of C180's class: a revalidation costs one merge plus one
/// replay, the same order as validating the block itself, so the work a single block can provoke is
/// this constant times a cost the protocol already pays.
pub const RESTORE_BUDGET_PER_BLOCK: usize = 2;

/// **The restoring rule — law 53a's missing inverse** (AUDIT C173, #125).
///
/// A justification this node recorded failed for a **view-dependent** reason gets one bounded chance
/// to clear, re-running the ordinary checks against this node's *current* DAG. This is the "revalidation
/// of the failed metadata" that C173's `owes` names, and it is what makes law 53a's
/// `the_refusal_is_persistent` false — the guard the law states is meant to fire when this lands.
///
/// **What makes it a rule and not a retry loop is the bound, and the bound has three parts.** Without
/// all three it is C180's class, work whose cost grows with an input nothing bounds:
///
/// - **keyed on the cause.** Only [`FailureCause::Divergence`] is eligible — a refusal that followed
///   from this node's own state or replay, where a block valid everywhere else reaches the same
///   verdict. An `Attributable` record is the block's own fault and is never re-validated; a `Cascade`
///   record is not about this block at all, and it clears when its parent's does.
/// - **capped per record**, at [`RESTORE_ATTEMPT_LIMIT`], and the count survives a restart.
/// - **budgeted per incoming block**, at [`RESTORE_BUDGET_PER_BLOCK`].
///
/// A successful revalidation clears `validation_failed` and the cause while keeping `slashable`
/// **exactly as the record had it** — the attribution split the issue asks for ("clearing the record
/// must not clear `slashable`"). For a `Divergence` record that is `false` by construction, so the
/// restore cannot mint slash evidence either way. A failed one spends an attempt and leaves the record.
///
/// **What this does not do, said plainly so it is not mistaken for more.** It re-runs the checks
/// against the current DAG; it does not manufacture agreement. A node whose view has converged restores
/// the block. A node whose view is *still* divergent does not, and is right to keep refusing a block it
/// cannot verify. The rule gives the refused state an inverse, which is what `Terminal` demands; it
/// does not promise the inverse always fires, and no rule could.
/// Whether a stored record is one the restoring rule may re-validate.
///
/// The **first** of the rule's three bounds, and the one that makes it reason-keyed rather than a
/// retry loop: only a `Divergence` — a refusal that came from this node's own state or replay, where a
/// block valid everywhere else reaches the same verdict — may be re-validated. An `Attributable`
/// record is the block's own fault and is permanent; a `Cascade` record is not about this block at all
/// and clears when its parent's does. The attempt cap is the second bound; the caller applies the
/// third (a budget per incoming block).
fn restore_is_warranted(stored: &BlockMetadata) -> bool {
    stored.validation_failed
        && stored.failure_cause == Some(FailureCause::Divergence)
        && stored.restore_attempts < RESTORE_ATTEMPT_LIMIT
}

/// **Clear this node's own spent failure record, if it has one** — C190's repair, and the other half of
/// what the update path (C193) made possible.
///
/// **The shape.** H-2 records a validation-failed block in the message map and keeps it out of
/// `latest_msgs` — so the proposer's parent set cannot see it — and the block creator derives `seq_num`
/// from that parent set. A node holding a failure record of *its own* block therefore derives a sequence
/// number it has already spent, and the H-1 gate refuses the block it built; three refusals halt the
/// autopropose timer. Reproduced in `finalization.rs` and registered as C190.
///
/// **Why the node has to do it itself.** The restoring rule fires on an *incoming* block's justifications
/// (`restore_divergent_justifications`), and `neglected_invalid_block` refuses every child of a failed
/// bonded block — so no inbound block will ever justify the node's own failed record, and the record is
/// otherwise permanent for the process's lifetime. The node is the only party that can clear it.
///
/// **The guard keeps the trigger narrow.** Only a `Divergence` record with attempts left is revalidated
/// (`restore_is_warranted`) — the cause whose whole meaning is "this node's view was wrong", which is the
/// one a re-read can settle. An `Attributable` failure is the block's own fault and stays.
///
/// **Cost**: one message-map scan per proposal, O(N) in the number of messages, on a 2-second timer. It is
/// not free and it is deliberate — the alternative is an index nothing else needs, for a defect that is
/// otherwise unreachable from the proposer's side.
///
/// Returns whether a record was **cleared**, which is the caller's signal that the block is visible to the
/// arithmetic again and a re-derived parent set will advance.
#[allow(clippy::too_many_arguments)]
pub async fn clear_own_failure_record<F, Fut>(
    dag: &dyn BlockDagStorage,
    block_store: &BlockStore,
    runtime: &RuntimeManager,
    shard_id: &str,
    min_phlo_price: i64,
    max_number_of_parents: i32,
    block_index: &F,
    sender: &Validator,
    log: &Arc<dyn Log>,
) -> bool
where
    F: Fn(BlockHash) -> Fut,
    Fut: std::future::Future<Output = Result<Arc<BlockIndex>, String>>,
{
    let source = LogSource::new("casper.interpreter.restore");
    // The candidates are this sender's messages that its own arithmetic cannot see: in the message map,
    // *newer* than its latest message, and therefore absent from the round snapshot the parent set is
    // built from. `latest` is kept beside the list so the instrument below can name the two structures.
    let (candidates, latest) = {
        let repr = dag.get_representation().await;
        let latest = repr
            .dag_message_state
            .latest_msgs
            .get(sender)
            .map(|m| m.sender_seq);
        let candidates = repr
            .dag_message_state
            .msg_map
            .values()
            .filter(|m| {
                m.sender == *sender
                    && match latest {
                        Some(l) => m.sender_seq > l,
                        None => true,
                    }
            })
            .map(|m| m.id)
            .collect::<Vec<BlockHash>>();
        (candidates, latest)
    };

    for hash in candidates {
        let Ok(Some(stored)) = dag.lookup(&hash).await else {
            continue;
        };
        if !restore_is_warranted(&stored) {
            continue;
        }
        // The instrument for C190's owed run. The audit named the two structures — `block_creator.rs`
        // derives `seq_num` from the parent set built from `latest_msgs`, while the H-1 gate scans
        // `msg_map` — but had only shape-matched them, and no recorded line said which one the arithmetic
        // read. This is that line: the node holds its own block at a sequence number its `latest_msgs`
        // entry does not carry, so the next proposal's derivation cannot see the number it already spent.
        log.warn(
            source,
            &format!(
                "this node's own block {} (seq {}) is in msg_map but not latest_msgs (latest seq {}): the \
                 next proposal derives its sequence number without seeing the one this record spent \
                 (AUDIT C190)",
                hash.to_hex(),
                stored.seq_num,
                latest.map(|l| l.to_string()).unwrap_or_else(|| "none".to_string()),
            ),
        );
        let msg = match block_store.get(&[hash]).await {
            Ok(mut v) => v.pop().flatten(),
            Err(e) => {
                // A read that fails is not "this block has no message" (C249's class): the scan would
                // skip a hash it never actually looked at.
                log.error(
                    source,
                    &format!("Failed to read block {} to re-validate: {e}", hash.to_hex()),
                );
                None
            }
        };
        let Some(msg) = msg else {
            continue;
        };
        let outcome = validate_checks(
            dag,
            block_store,
            runtime,
            &msg,
            shard_id,
            min_phlo_price,
            max_number_of_parents,
            block_index,
            log,
        )
        .await;
        let record = revalidated_record(&stored, outcome.ok());
        let cleared = !record.validation_failed;
        match dag.update_metadata(record).await {
            Ok(()) if cleared => {
                log.warn(
                    source,
                    &format!(
                        "cleared this node's own failure record for {}: the block validates now, so the \
                         proposer can advance past the sequence number it had spent (AUDIT C190)",
                        hash.to_hex()
                    ),
                );
                return true;
            }
            // The attempt is spent either way (`revalidated_record` counts it), so a record that keeps
            // failing stops being retried at `RESTORE_ATTEMPT_LIMIT` rather than on every proposal.
            Ok(()) => continue,
            Err(e) => log.error(
                source,
                &format!(
                    "could not clear this node's own failure record for {}: {e}",
                    hash.to_hex()
                ),
            ),
        }
    }
    false
}

/// The record to store after one revalidation attempt: `Some(fresh)` when the block passed, `None`
/// when it did not.
///
/// **`slashable` is carried, not recomputed.** Clearing the refusal must not clear attribution — the
/// restore gives the refused state an inverse, it does not withdraw the verdict about whose fault the
/// failure was. For a `Divergence` record the carried value is `false` by construction, so the restore
/// cannot mint slash evidence in either direction.
///
/// The attempt is spent either way, which is what makes the cap a cap: a record that keeps failing to
/// restore stops being re-validated rather than being retried on every block that justifies it.
fn revalidated_record(stored: &BlockMetadata, fresh: Option<BlockMetadata>) -> BlockMetadata {
    let attempts = stored.restore_attempts + 1;
    match fresh {
        Some(fresh) => BlockMetadata {
            validation_failed: false,
            failure_cause: None,
            slash_severity: SlashSeverity::Unspecified,
            slashable: stored.slashable,
            restore_attempts: attempts,
            ..fresh
        },
        None => BlockMetadata {
            restore_attempts: attempts,
            ..stored.clone()
        },
    }
}

#[allow(clippy::too_many_arguments)]
async fn restore_divergent_justifications<F, Fut>(
    dag: &dyn BlockDagStorage,
    block_store: &BlockStore,
    runtime: &RuntimeManager,
    block: &BlockMessage,
    shard_id: &str,
    min_phlo_price: i64,
    max_number_of_parents: i32,
    block_index: &F,
    log: &Arc<dyn Log>,
) where
    F: Fn(BlockHash) -> Fut,
    Fut: std::future::Future<Output = Result<Arc<BlockIndex>, String>>,
{
    let source = LogSource::new("casper.interpreter.restore");
    let mut budget = RESTORE_BUDGET_PER_BLOCK;

    for j in &block.justifications {
        if budget == 0 {
            return;
        }
        let stored = match dag.lookup(j).await {
            Ok(Some(meta)) => meta,
            // A missing or unreadable justification is not this rule's business: the checks refuse the
            // block and name the reason more precisely than a restore failure could.
            Ok(None) | Err(_) => continue,
        };
        if !restore_is_warranted(&stored) {
            continue;
        }

        let msg = match block_store.get(&[*j]).await {
            Ok(mut v) => v.pop().flatten(),
            Err(e) => {
                // **A read error does not say the block is gone.** The comment below describes a *pruned*
                // store; this is a store that cannot answer, and the erasure made the two the same thing
                // (C249's class). The budget is not spent either way — this only says which happened.
                log.error(
                    source,
                    &format!("Failed to read block {} to restore: {e}", j.to_hex()),
                );
                None
            }
        };
        let Some(msg) = msg else {
            // The record says the block failed here, but the block itself is gone (a pruned store).
            // There is nothing to re-validate, and the budget is for revalidations.
            continue;
        };

        budget -= 1;
        let attempts = stored.restore_attempts + 1;
        let outcome = validate_checks(
            dag,
            block_store,
            runtime,
            &msg,
            shard_id,
            min_phlo_price,
            max_number_of_parents,
            block_index,
            log,
        )
        .await;
        let record = revalidated_record(&stored, outcome.ok());
        let restored = !record.validation_failed;

        // **`update_metadata`, not `insert` — and that is AUDIT C193.** This writes the record of a block
        // the DAG already holds, and `insert` returns `Ok(())` for a hash it knows without writing
        // anything: the rule logged "cleared the failure record" while the store kept the failure, and
        // the failure it exists to clear was never cleared. The update path also takes the block back
        // into the height map and into `latest_msgs`, which H-2 had excluded it from.
        //
        // **Store and index together, or neither.** The update runs `BlockMetadataStore::add`, whose
        // contiguity check a *restored* block can legitimately fail: a failed block is excluded from
        // the height map and a restored one enters it. A refusal leaves the store as it was and spends
        // the attempt — it must never leave the store and the index disagreeing (AUDIT C172's shape).
        if let Err(e) = dag.update_metadata(record).await {
            log.error(
                source,
                &format!(
                    "could not {} {} after revalidation: {e}",
                    if restored { "restore" } else { "record" },
                    j.to_hex()
                ),
            );
            continue;
        }

        if restored {
            log.warn(
                source,
                &format!(
                    "cleared the failure record for {} on attempt {attempts}: the block validates \
                     against this node's view again",
                    j.to_hex()
                ),
            );
        }
    }
}

/// Validate a block, giving a refused one its inverse first (port of `MultiParentCasper.validate`).
///
/// **The restoring rule runs here, and the position is the point (AUDIT C173, #125).** Law 53a is
/// `Terminal`: the record `mark_failed` writes is never cleared, and the rules then refuse every block
/// above it, so a node that marked one block failed is estranged from its sender for good. The fix is
/// a rule that restores, and it has to run **before** the checks, because the record has four readers
/// and they are not in one place: `block_number` (inside `block_summary`), `neglected_invalid_block`,
/// the merge's parent set in `interpreter_util.rs`, and `get_parents_metadata` in `proto_util.rs`.
pub async fn validate<F, Fut>(
    dag: &dyn BlockDagStorage,
    block_store: &BlockStore,
    runtime: &RuntimeManager,
    block: &BlockMessage,
    shard_id: &str,
    min_phlo_price: i64,
    max_number_of_parents: i32,
    block_index: &F,
    log: &Arc<dyn Log>,
) -> Result<BlockMetadata, ValidateError>
where
    F: Fn(BlockHash) -> Fut,
    Fut: std::future::Future<Output = Result<Arc<BlockIndex>, String>>,
{
    restore_divergent_justifications(
        dag,
        block_store,
        runtime,
        block,
        shard_id,
        min_phlo_price,
        max_number_of_parents,
        block_index,
        log,
    )
    .await;

    validate_checks(
        dag,
        block_store,
        runtime,
        block,
        shard_id,
        min_phlo_price,
        max_number_of_parents,
        block_index,
        log,
    )
    .await
}

/// The checks themselves, with no restoring: block summary, replay checkpoint, bonds cache,
/// neglected-invalid-block, and phlo price (port of the body of `MultiParentCasper.validate`).
///
/// Split out from [`validate`] so the restoring rule can re-run **exactly these** on a stored block
/// without recursing into another restore — a revalidation that could itself revalidate would make
/// [`RESTORE_BUDGET_PER_BLOCK`] a bound on nothing.
#[allow(clippy::too_many_arguments)]
async fn validate_checks<F, Fut>(
    dag: &dyn BlockDagStorage,
    block_store: &BlockStore,
    runtime: &RuntimeManager,
    block: &BlockMessage,
    shard_id: &str,
    min_phlo_price: i64,
    max_number_of_parents: i32,
    block_index: &F,
    log: &Arc<dyn Log>,
) -> Result<BlockMetadata, ValidateError>
where
    F: Fn(BlockHash) -> Fut,
    Fut: std::future::Future<Output = Result<Arc<BlockIndex>, String>>,
{
    let init_block_meta = BlockMetadata::from_block(block);

    // Block summary (justification regression, sequence/block number, deploy checks, phlo price).
    match crate::validate::block_summary(
        dag,
        block_store,
        block,
        shard_id,
        DEPLOY_LIFESPAN,
        min_phlo_price,
        max_number_of_parents,
    )
    .await
    {
        Ok(Ok(())) => {}
        Ok(Err(status)) => {
            return Err(ValidateError::ValidationFailed(
                mark_failed(&init_block_meta, status),
                status,
            ))
        }
        Err(e) => {
            return Err(ValidateError::Internal(format!(
                "block summary failed: {e}"
            )))
        }
    }

    // Replay validation.
    let (block_metadata, validated) =
        validate_block_checkpoint(runtime, dag, block_store, block, block_index, log)
            .await
            .map_err(|e| ValidateError::Internal(format!("validateBlockCheckpoint failed: {e}")))?;
    match validated {
        Err(status) => {
            return Err(ValidateError::ValidationFailed(
                mark_failed(&block_metadata, status),
                status,
            ))
        }
        Ok(true) => {}
        // `Ok(false)` is now **only** the post-state predicate: the pre-state mismatch has its own
        // status (#139), so this arm and `Err(status)` above are no longer two spellings of one
        // failure.
        Ok(false) => {
            return Err(ValidateError::ValidationFailed(
                mark_failed(&block_metadata, BlockStatus::InvalidStateHash),
                BlockStatus::InvalidStateHash,
            ))
        }
    }

    // Bonds cache.
    match crate::validate::bonds_cache(runtime, block).await {
        Ok(Ok(())) => {}
        Ok(Err(status)) => {
            return Err(ValidateError::ValidationFailed(
                mark_failed(&block_metadata, status),
                status,
            ))
        }
        Err(e) => return Err(ValidateError::Internal(format!("bondsCache failed: {e}"))),
    }

    // Neglected invalid block.
    match crate::validate::neglected_invalid_block(dag, block).await {
        Ok(Ok(())) => {}
        Ok(Err(status)) => {
            return Err(ValidateError::ValidationFailed(
                mark_failed(&block_metadata, status),
                status,
            ))
        }
        Err(e) => {
            return Err(ValidateError::Internal(format!(
                "neglectedInvalidBlock failed: {e}"
            )))
        }
    }

    // Build/cache the block index.
    // `block_metadata` is the metadata this validation recomputed, whose `fringe_state_hash` came from
    // the merge — the same value the block's own close deploy used, so the regeneration path below (if
    // it is ever taken) replays to the same seed leaf. The index is best-effort here: a failure leaves
    // it to be rebuilt on the next lookup, which is why the result is discarded.
    //
    // The index takes the DAG so that the participation the epoch's absence rule needs can be derived
    // on the fallback arm only, rather than walked here for every block. `pre_state.participation` is
    // not in scope at this point, and `participation_for_block` is the same derivation anyway.
    let _ = BlockIndex::get_block_index(
        runtime,
        dag,
        block_store,
        block.block_hash,
        Blake2b256Hash::from_byte_array(block_metadata.fringe_state_hash.as_bytes()),
    )
    .await;

    Ok(block_metadata)
}

/// Mark a block unusable here, recording **why** (AUDIT C173), and whether the sender may be slashed
/// for it (AUDIT C198).
///
/// **Two questions, and they are not the same one.** `ValidationFailed` carries the `BlockStatus`,
/// and the status answers both: `failure_cause` is *whose fault the refusal is*, and
/// [`BlockStatus::is_slashing_offence`] is *whether that fault may take the sender's bond*. The
/// second is narrower, because three statuses read an input the **receiving node** owns (see that
/// method) and a node that reaches the opposite verdict refuses the slashing block rather than
/// tolerating the difference — a split, not a disagreement.
///
/// Before C173 every `ValidateError::ValidationFailed` went through a `mark_failed_attributable` that
/// set `slashable` unconditionally — including `NeglectedInvalidBlock`, a refusal of a child *because
/// a justification failed* — so one transient failure fabricated slash evidence against every
/// validator above it (#125).
///
/// The genuinely unattributable case (an unreadable pre-state, a store error, an unrecoverable
/// mergeable-channel sidecar) still surfaces as `ValidateError::Internal` and inserts **no** metadata
/// at all, so it cannot become an offender (#70, #76).
///
/// `restore_attempts` is carried over rather than reset, so the cap in
/// [`restore_divergent_justifications`] is a per-block-lifetime budget: a record that flaps between
/// restored and failed cannot buy unlimited revalidations.
fn mark_failed(meta: &BlockMetadata, status: BlockStatus) -> BlockMetadata {
    BlockMetadata {
        validated: true,
        validation_failed: true,
        slashable: status.is_slashing_offence(),
        slash_severity: status
            .slash_severity()
            .unwrap_or(SlashSeverity::Unspecified),
        failure_cause: Some(status.failure_cause()),
        ..meta.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The deploy lifespan is a **consensus-visible parameter**: it decides which deploys a block may
    /// still include, so two nodes disagreeing on it disagree about validity. Pinned as a value, not
    /// merely as a constant, because a silent change here is a chain split rather than a tuning
    /// choice.
    #[test]
    fn the_deploy_lifespan_is_pinned() {
        assert_eq!(DEPLOY_LIFESPAN, 50);
    }

    /// A parsing error keeps the details it was built from — the only thing that makes a malformed
    /// block or deploy debuggable once the raw bytes are gone.
    #[test]
    fn a_parsing_error_carries_its_details() {
        let err = parsing_error("bad justification list");
        assert!(
            err.0.contains("bad justification list"),
            "the details must survive: {:?}",
            err.0
        );
        assert!(
            err.0.starts_with("Parsing error:"),
            "and be labelled as a parsing failure: {:?}",
            err.0
        );
    }

    /// `ValidateError` distinguishes the two outcomes a validator can have, and the distinction is
    /// load-bearing: a block that **fails validation is still a block** (its metadata and status are
    /// returned so the DAG can record it), whereas an internal error has no block outcome at all. A
    /// refactor that collapsed the two would make a store failure look like an invalid block.
    #[test]
    fn validate_error_separates_an_invalid_block_from_an_internal_failure() {
        let status = BlockStatus::InvalidStateHash;
        let metadata = BlockMetadata {
            block_hash: BlockHash::new([0u8; 32]),
            block_num: rchain_shared::refined::BlockHeight::try_from(1).expect("height"),
            sender: rchain_models::validator::Validator::from_slice(&[0u8; 65]),
            seq_num: rchain_shared::refined::SeqNum::zero(),
            justifications: std::collections::BTreeSet::new(),
            bonds_map: std::collections::BTreeMap::new(),
            validated: false,
            validation_failed: true,
            slashable: false,
            failure_cause: None,
            slash_severity: SlashSeverity::Unspecified,
            restore_attempts: 0,
            member_of_fringe: None,
            fringe: std::collections::BTreeSet::new(),
            fringe_state_hash: rchain_crypto::hash::blake2b256_hash::Blake2b256Hash::from_bytes(
                [0u8; 32],
            )
            .into(),
        };

        let invalid = ValidateError::ValidationFailed(metadata.clone(), status);
        match invalid {
            ValidateError::ValidationFailed(returned, returned_status) => {
                assert_eq!(returned.block_hash, metadata.block_hash);
                assert_eq!(returned_status, status);
                assert!(
                    returned.validation_failed,
                    "an invalid block is still returned, marked failed"
                );
            }
            ValidateError::Internal(_) => panic!("a validation failure must not read as internal"),
            ValidateError::SelfEquivocation => {
                panic!("a validation failure must not read as a self-equivocation")
            }
        }

        let internal = ValidateError::Internal("store unavailable".to_string());
        assert!(
            matches!(internal, ValidateError::Internal(message) if message.contains("store")),
            "an internal error carries its cause and no block outcome"
        );
    }
}

/// The justification whose state answers for the bond map while nothing has finalised: the newest, by
/// height and then by hash so every node picks the same one. See the call site for why the bond maps the
/// blocks carry cannot be used instead ([#73]).
///
/// [#73]: https://github.com/rchain-community/rchain-rust/issues/73
fn newest_justification(justifications: &[BlockMetadata]) -> Option<&BlockMetadata> {
    justifications.iter().max_by(|a, b| {
        a.block_num
            .cmp(&b.block_num)
            .then_with(|| a.block_hash.cmp(&b.block_hash))
    })
}

#[cfg(test)]
mod newest_justification_tests {
    use super::newest_justification;
    use rchain_models::block_hash::BlockHash;
    use rchain_models::block_metadata::BlockMetadata;
    use rchain_models::block_metadata::SlashSeverity;
    use rchain_shared::refined::BlockHeight;
    use std::collections::{BTreeMap, BTreeSet};

    fn meta(byte: u8, block_num: i64) -> BlockMetadata {
        BlockMetadata {
            block_hash: BlockHash::new([byte; 32]),
            block_num: BlockHeight::try_from(block_num).unwrap(),
            sender: rchain_models::validator::Validator::new([0u8; 65]),
            seq_num: 0.try_into().unwrap(),
            justifications: BTreeSet::new(),
            bonds_map: BTreeMap::new(),
            validated: true,
            validation_failed: false,
            slashable: false,
            failure_cause: None,
            slash_severity: SlashSeverity::Unspecified,
            restore_attempts: 0,
            fringe: BTreeSet::new(),
            fringe_state_hash: rchain_models::block::state_hash::StateHash::new([0u8; 32]),
            member_of_fringe: None,
        }
    }

    #[test]
    fn the_tallest_justification_answers_for_the_bonds() {
        let js = [meta(1, 5), meta(2, 9), meta(3, 7)];
        let picked = newest_justification(&js).expect("one of them");
        assert_eq!(i64::from(picked.block_num), 9);
    }

    /// Equal heights must not depend on the order of the slice: two nodes holding the same DAG have to
    /// read the same bond map, or one of them refuses a block the other accepted.
    #[test]
    fn equal_heights_break_by_hash_so_every_node_agrees() {
        let forwards = [meta(1, 9), meta(2, 9)];
        let backwards = [meta(2, 9), meta(1, 9)];
        let expected = meta(2, 9);
        assert_eq!(
            newest_justification(&forwards)
                .expect("one of them")
                .block_hash,
            newest_justification(&backwards)
                .expect("one of them")
                .block_hash
        );
        assert_eq!(
            newest_justification(&forwards)
                .expect("one of them")
                .block_hash,
            expected.block_hash
        );
    }

    #[test]
    fn no_justifications_answers_for_nothing() {
        assert!(newest_justification(&[]).is_none());
    }
}

#[cfg(test)]
mod restore_tests {
    use super::{restore_is_warranted, revalidated_record, RESTORE_ATTEMPT_LIMIT};
    use rchain_models::block_hash::BlockHash;
    use rchain_models::block_metadata::SlashSeverity;
    use rchain_models::block_metadata::{BlockMetadata, FailureCause};
    use rchain_models::validator::Validator;
    use rchain_shared::refined::{BlockHeight, SeqNum};
    use std::collections::{BTreeMap, BTreeSet};

    fn meta(failed: bool, cause: Option<FailureCause>, attempts: u32) -> BlockMetadata {
        BlockMetadata {
            block_hash: BlockHash::new([1u8; 32]),
            block_num: BlockHeight::try_from(1).unwrap(),
            sender: Validator::new([1u8; 65]),
            seq_num: SeqNum::zero(),
            justifications: BTreeSet::new(),
            bonds_map: BTreeMap::new(),
            validated: true,
            validation_failed: failed,
            slashable: false,
            failure_cause: cause,
            slash_severity: SlashSeverity::Unspecified,
            restore_attempts: attempts,
            fringe: BTreeSet::new(),
            fringe_state_hash: rchain_models::block::state_hash::StateHash::new([0u8; 32]),
            member_of_fringe: None,
        }
    }

    /// **The rule is keyed on the reason, and that is what keeps it out of C180's class.**
    ///
    /// A `Divergence` — this node's own state or replay disagreeing with a block that is valid
    /// elsewhere — is the only restorable cause. An `Attributable` refusal is the block's own fault
    /// and stays; a `Cascade` is not about the block at all. A record with no cause at all (one
    /// written before the field existed) is ineligible too, which is the safe direction.
    #[test]
    fn only_a_divergence_is_restorable() {
        assert!(restore_is_warranted(&meta(
            true,
            Some(FailureCause::Divergence),
            0
        )));
        assert!(
            !restore_is_warranted(&meta(true, Some(FailureCause::Attributable), 0)),
            "the block's own fault is permanent, however many children justify it"
        );
        assert!(
            !restore_is_warranted(&meta(true, Some(FailureCause::Cascade), 0)),
            "a cascade is not about this block; it clears when its parent's record does"
        );
        assert!(
            !restore_is_warranted(&meta(true, None, 0)),
            "an unknown cause must not be re-validated"
        );
        assert!(
            !restore_is_warranted(&meta(false, None, 0)),
            "a record that did not fail is not a refusal"
        );
    }

    /// The attempt cap is a budget, not a latch: at the limit the record stops being re-validated.
    #[test]
    fn the_attempt_cap_is_reached_not_exceeded() {
        assert!(restore_is_warranted(&meta(
            true,
            Some(FailureCause::Divergence),
            RESTORE_ATTEMPT_LIMIT - 1
        )));
        assert!(
            !restore_is_warranted(&meta(
                true,
                Some(FailureCause::Divergence),
                RESTORE_ATTEMPT_LIMIT
            )),
            "a record at the limit must not be re-validated again"
        );
    }

    /// **The falsifier's unit: a successful revalidation clears the refusal** — which is the step
    /// law 53a's `the_refusal_is_persistent` says does not exist, and whose absence is the
    /// estrangement.
    ///
    /// `slashable` is carried rather than recomputed: clearing the record must not clear attribution.
    #[test]
    fn a_successful_revalidation_clears_the_refusal_and_keeps_attribution() {
        let stored = BlockMetadata {
            slashable: true,
            ..meta(true, Some(FailureCause::Divergence), 1)
        };
        let fresh = meta(false, None, 0);

        let record = revalidated_record(&stored, Some(fresh));
        assert!(
            !record.validation_failed,
            "the restoring rule's whole point: the record is cleared"
        );
        assert_eq!(record.failure_cause, None, "and the cause with it");
        assert_eq!(record.restore_attempts, 2, "the attempt is spent");
        assert!(
            record.slashable,
            "clearing the record must not clear attribution"
        );
        // The revalidation's own metadata is what is stored — its fringe and pre-state are the fresh
        // ones, not the ones from the failed attempt.
        assert!(record.validated);
    }

    /// A revalidation that fails leaves the refusal. Without this the rule would clear a record on
    /// any child arriving, which would be worse than not clearing it: the node would then accept the
    /// child of a block it still cannot verify.
    #[test]
    fn a_failed_revalidation_keeps_the_record_and_spends_the_attempt() {
        let stored = meta(true, Some(FailureCause::Divergence), 1);
        let record = revalidated_record(&stored, None);

        assert!(record.validation_failed, "still refused");
        assert_eq!(record.failure_cause, Some(FailureCause::Divergence));
        assert_eq!(record.restore_attempts, 2);
        // Everything else is untouched — the failure is the same failure it was.
        assert_eq!(record.block_hash, stored.block_hash);
    }
}

#[cfg(test)]
mod mark_failed_tests {
    use super::mark_failed;
    use crate::block_status::BlockStatus;
    use rchain_models::block_hash::BlockHash;
    use rchain_models::block_metadata::SlashSeverity;
    use rchain_models::block_metadata::{BlockMetadata, FailureCause};
    use rchain_models::validator::Validator;
    use rchain_shared::refined::{BlockHeight, SeqNum};
    use std::collections::{BTreeMap, BTreeSet};

    fn meta() -> BlockMetadata {
        BlockMetadata {
            block_hash: BlockHash::new([0u8; 32]),
            block_num: BlockHeight::try_from(1).unwrap(),
            sender: Validator::new([1u8; 65]),
            seq_num: SeqNum::zero(),
            justifications: BTreeSet::new(),
            bonds_map: BTreeMap::new(),
            validated: false,
            validation_failed: false,
            slashable: false,
            failure_cause: None,
            slash_severity: SlashSeverity::Unspecified,
            restore_attempts: 0,
            fringe: BTreeSet::new(),
            fringe_state_hash: rchain_models::block::state_hash::StateHash::new([0u8; 32]),
            member_of_fringe: None,
        }
    }

    /// `mark_failed` marks the block unusable, records the cause the status implies, and decides the
    /// offence from the status.
    #[test]
    fn mark_failed_marks_the_block_and_records_the_cause() {
        let marked = mark_failed(&meta(), BlockStatus::InvalidBlockNumber);
        assert!(marked.validation_failed, "still unusable here");
        assert_eq!(marked.failure_cause, Some(FailureCause::Attributable));
        assert!(marked.validated);
        assert!(
            marked.slashable,
            "and its own bad block_number is an offence"
        );
    }

    /// **Two questions, and this test is where they part company.**
    ///
    /// `failure_cause` is *whose fault the refusal is* (AUDIT C173 — before that split, every
    /// `ValidateError::ValidationFailed` set `slashable`, so `NeglectedInvalidBlock`, a child refused
    /// *because a justification failed*, was an offender). `slashable` is *whether that fault may
    /// take the sender's bond* (AUDIT C198), and it is narrower: a status can be `Attributable` and
    /// still not be an offence, when the input it reads belongs to the receiving node.
    ///
    /// Red before each split, in turn: `NeglectedInvalidBlock` was slashable before C173, and
    /// `TooManyJustifications` was before C198.
    #[test]
    fn an_attributable_refusal_is_not_always_an_offence() {
        // The block's own fault, from the block and the DAG alone: an offence.
        let offence = mark_failed(&meta(), BlockStatus::InvalidBlockNumber);
        assert_eq!(offence.failure_cause, Some(FailureCause::Attributable));
        assert!(offence.slashable);

        // `Attributable`, and **not** an offence: `max-number-of-parents` is this node's own setting,
        // so a peer with a different one would refuse the slashing block instead of agreeing to it.
        let local = mark_failed(&meta(), BlockStatus::TooManyJustifications);
        assert_eq!(local.failure_cause, Some(FailureCause::Attributable));
        assert!(local.validation_failed, "still unusable here");
        assert!(
            !local.slashable,
            "a knob the receiving node owns cannot be evidence against the sender"
        );

        // Not the block's fault at all — C173's split, unchanged.
        let cascaded = mark_failed(&meta(), BlockStatus::NeglectedInvalidBlock);
        assert!(cascaded.validation_failed);
        assert_eq!(cascaded.failure_cause, Some(FailureCause::Cascade));
        assert!(
            !cascaded.slashable,
            "a child refused because its parent failed"
        );

        let diverged = mark_failed(&meta(), BlockStatus::InvalidStateHash);
        assert!(diverged.validation_failed);
        assert_eq!(diverged.failure_cause, Some(FailureCause::Divergence));
        assert!(
            !diverged.slashable,
            "a node whose state differs is not evidence"
        );
    }
}
