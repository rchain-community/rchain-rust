//! Interpreter utilities (port of `rholang/InterpreterUtil.scala`).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use rchain_block_storage::block_store::BlockStore;
use rchain_block_storage::dag::dag_storage::BlockDagStorage;
use rchain_block_storage::dag::finalizer::NoAdvance;
use rchain_crypto::hash::blake2b256_hash::Blake2b256Hash;
use rchain_crypto::hash::blake2b512_random::Blake2b512Random;
use rchain_models::ast::Par;
use rchain_models::block::state_hash::StateHash;
use rchain_models::block_hash::BlockHash;
use rchain_models::block_metadata::BlockMetadata;
use rchain_models::casper::protocol::casper_message::{BlockMessage, SignedDeployData};
use rchain_models::validator::Validator;
use rchain_rholang::errors::RholangError;
use rchain_rholang::runtime::ReplayRhoRuntime;
use rchain_rholang::system_processes::BlockData;
use rchain_shared::base16;
use rchain_shared::log::{Log, LogSource};

use crate::block_random_seed::BlockRandomSeed;
use crate::block_status::BlockStatus;
use crate::merging::{BlockIndex, ParentsMergedState};
use crate::multi_parent_casper::get_pre_state_for_parents;
use crate::rholang::{ReplayFailure, SystemDeployRuntimeResult, UserDeployRuntimeResult};
use crate::runtime_manager::RuntimeManager;
use crate::system_deploy::SystemDeploy;

/// Parse + normalize a rholang term (port of `mkTerm`).
pub fn mk_term(rho: &str, env: &BTreeMap<String, Par>) -> Result<Par, RholangError> {
    rchain_rholang::normalizer::source_to_adt_with_env(rho, env).map(Par::from)
}

/// Replay a block's deploys and return the computed state hash (port of `replayBlock`). The replay
/// mutates only `replay_runtime` (a per-block fork); the mergeable-channel save uses `runtime`.
/// `fringe_state_hash` is the state hash of the last finalised fringe as of `block` — the value its
/// close system deploy anchored the next epoch's seed to. The caller computes it from the DAG
/// (`pre_state.fringe_state`) rather than reading it off the block, because the block's header does
/// not carry it: a field the validator recomputes anyway is a second derivation waiting to fall out
/// of step, and one that a proposer would be choosing if it were on the wire.
pub async fn replay_block(
    runtime: &RuntimeManager,
    replay_runtime: &ReplayRhoRuntime,
    block: &BlockMessage,
    fringe_state_hash: &Blake2b256Hash,
    rand: &Blake2b512Random,
) -> Result<Blake2b256Hash, ReplayFailure> {
    let start_hash = Blake2b256Hash::from_byte_array(block.pre_state_hash.as_bytes());
    let block_data = BlockData::from_block(block);
    let with_cost_accounting = !block.justifications.is_empty();
    let (state_hash, _mergeable) = runtime
        .replay_compute_state_with(
            replay_runtime,
            &start_hash,
            &block.state.deploys,
            &block.state.system_deploys,
            rand,
            block_data,
            fringe_state_hash,
            with_cost_accounting,
            // Genesis PoS descriptors (pool/trusted/params) come from the network's genesis
            // configuration; the trie is authoritative for every non-genesis block, so this value is
            // only consumed on the genesis replay path.
            runtime.genesis_pos(),
            // The genesis vault balances, and only for the genesis: they are not carried on the block
            // (they are installed at genesis) and the pre-state of a later block already holds them,
            // so re-installing there would clobber post-genesis balances (AUDIT C46).
            if is_genesis_pre_state(&start_hash) {
                runtime.genesis_vaults()
            } else {
                &[]
            },
        )
        .await?;
    Ok(state_hash)
}

/// Map a replay result into an `Option` of the matching state hash (port of `handleErrors`).
pub fn handle_errors(
    ts_hash: &Blake2b256Hash,
    result: Result<Blake2b256Hash, ReplayFailure>,
) -> Result<Option<Blake2b256Hash>, String> {
    match result {
        Ok(computed) => {
            if *ts_hash == computed {
                Ok(Some(computed))
            } else {
                Ok(None)
            }
        }
        Err(ReplayFailure::InternalError(cause)) => Err(format!(
            "Internal errors encountered while processing deploy: {cause}"
        )),
        Err(_) => Ok(None),
    }
}

/// Compute the post-state + processed deploys from a deploy sequence (port of
/// `computeDeploysCheckpoint`).
#[allow(clippy::too_many_arguments)]
pub async fn compute_deploys_checkpoint(
    runtime: &RuntimeManager,
    deploys: &[SignedDeployData],
    system_deploys: &[SystemDeploy],
    rand: &Blake2b512Random,
    block_data: BlockData,
    pre_state_hash: &Blake2b256Hash,
    fringe_state_hash: &Blake2b256Hash,
) -> Result<
    (
        Blake2b256Hash,
        Vec<UserDeployRuntimeResult>,
        Vec<SystemDeployRuntimeResult>,
    ),
    String,
> {
    runtime
        .compute_state(
            pre_state_hash,
            deploys,
            system_deploys,
            rand,
            block_data,
            fringe_state_hash,
        )
        .await
}

/// The hard-coded empty-state (genesis pre-state) hash (port of `emptyStateHashFixed`).
///
/// Recomputed after the native system-process install + de-blessed genesis (the empty state is now
/// "system processes installed + empty native state", with no registry-bootstrap echo).
pub fn empty_state_hash_fixed() -> Blake2b256Hash {
    Blake2b256Hash::from_byte_array(&base16::unsafe_decode(
        "0e5751c026e543b2e8ab2eb06099daa1d1e5df47778f7787faab45cdf12fe3a8",
    ))
}

/// Whether `pre_state_hash` is the genesis block's pre-state — the empty state.
///
/// This is the one replay that must re-install the network's genesis descriptors (the native PoS
/// state and the initial REV vault balances), because `compute_genesis` installs both as native
/// state *outside* the block's deploys, so no deploy in the genesis recreates them (AUDIT C46).
/// Every other block's pre-state already carries those balances, so re-installing there would
/// clobber a post-genesis one — the condition is what keeps the re-install from being unconditional.
pub fn is_genesis_pre_state(pre_state_hash: &Blake2b256Hash) -> bool {
    *pre_state_hash == empty_state_hash_fixed()
}

/// Does this block slash anyone its own justifications do not hold responsible (AUDIT C110)?
///
/// **Why the check lives here and not in the pure-check list.** The pure checks read the block and
/// nothing else, and the *evidence* for a slash is not in the block: `BlockMessage.justifications` is
/// a list of `BlockHash`, and the `slashable` flag is `BlockMetadata`'s — in memory only, explicitly
/// "neither [] carried in the protobuf" (see its doc). So a validator can only ask the question
/// against **its own DAG view**, which means the answer depends on what *this node* observed about
/// those justifications, and that is exactly the property wanted: the proposer's opinion of the victim
/// carries no weight, and a node that never saw the offending block cannot be made to rubber-stamp
/// the punishment.
///
/// A block with no slashes short-circuits before any lookup, so the cost is paid only by blocks that
/// take someone's stake. A justification this node does not have is simply absent from the metadata
/// list — it then contributes no offender, so an unknown justification cannot license a slash, which
/// is the safe direction.
async fn slash_is_unjustified(
    dag: &dyn BlockDagStorage,
    block: &BlockMessage,
) -> Result<bool, String> {
    let slashed = crate::validate::slashed_validators(block);
    if slashed.is_empty() {
        return Ok(false);
    }
    let mut metadata = Vec::with_capacity(block.justifications.len());
    for j in &block.justifications {
        if let Some(meta) = dag.lookup(j).await? {
            metadata.push(meta);
        }
    }
    let justified = crate::validate::slashable_senders(&metadata);
    Ok(!slashed.is_subset(&justified))
}

/// The tip height at which a finality stall was last logged. A `static` because the alternative is
/// threading a counter through every call for one log line — and the gate itself is stateless.
///
/// `i64::MIN` is the "never logged" sentinel, and the comparison is a `saturating_sub` for that reason:
/// plain subtraction would overflow on the first stall of a process's life, which is a panic in debug
/// and a wrapped negative in release — a rate limiter that never fires, on the path it exists for.
static LAST_STALL_LOG: AtomicI64 = AtomicI64::new(i64::MIN);

/// How many heights may pass between two stall lines: one per hundred blocks keeps a stall that lasts
/// hundreds of blocks to a readable number of lines (#70's measurement ran ~160 blocks at finality 44).
const STALL_LOG_INTERVAL: i64 = 100;

/// A validator id, short enough for a log line: its first four bytes in hex.
///
/// The slice cannot be short — a `Validator` is a fixed 65-byte array (`models/src/validator.rs:10`,
/// an uncompressed secp256k1 key), so this is a *display* truncation and not a guard. Said plainly
/// because the four bytes are the only handle an operator has on which validator the gate is waiting
/// for, and a reader who thinks the length is a risk will "fix" it into a lossy form that hides one.
fn short_id(v: &Validator) -> String {
    v.as_bytes()[..4]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The gate's reason for not advancing, as one line an operator can act on.
///
/// The three are the gate's three exits and they imply different faults, which is the whole point of
/// logging them rather than "finality stalled": a partition no layer covers is a different defect from a
/// layer whose stake is short, and both differ from "the derivation would publish what is already
/// finalised".
fn describe_no_advance(reason: &NoAdvance<Validator>) -> String {
    match reason {
        NoAdvance::Coverage { missing, senders } => format!(
            "the unfinalized region has no layer covering the partition — {} of its senders have a \
             minimum message and {} do not ({})",
            senders.len(),
            missing.len(),
            missing
                .iter()
                .map(short_id)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        NoAdvance::Support {
            supporting,
            total,
            full_partitions,
            candidates,
        } => format!(
            "a layer exists but its supporting stake is not a supermajority — {supporting} of {total} \
             ({full_partitions} full partition(s) among {candidates} candidate(s))"
        ),
        NoAdvance::AlreadyPublished => {
            "the derivation would publish the fringe that is already finalized".to_string()
        }
    }
}

/// Validate a block by recomputing its pre-state and replaying its deploys (port of
/// `validateBlockCheckpoint`). Returns the block metadata plus a `bool` (valid) / `BlockStatus`
/// (rejectable) outcome.
pub async fn validate_block_checkpoint<F, Fut>(
    runtime: &RuntimeManager,
    dag: &dyn BlockDagStorage,
    block_store: &BlockStore,
    block: &BlockMessage,
    block_index: &F,
    log: &Arc<dyn Log>,
) -> Result<(BlockMetadata, Result<bool, BlockStatus>), String>
where
    F: Fn(BlockHash) -> Fut,
    Fut: std::future::Future<Output = Result<BlockIndex, String>>,
{
    // Non-failed parent hashes.
    let mut parents: Vec<BlockHash> = Vec::new();
    for j in &block.justifications {
        if let Some(meta) = dag.lookup(j).await? {
            if !meta.validation_failed {
                parents.push(meta.block_hash);
            }
        }
    }
    let parents_set: BTreeSet<BlockHash> = parents.iter().copied().collect();

    let source = LogSource::new("casper.interpreter.validate");
    let pre_state = if !parents_set.is_empty() {
        let pre_state =
            get_pre_state_for_parents(dag, block_store, runtime, &parents_set, block_index).await?;
        // **Why finality is not advancing, in this node's own log.** On 2026-09-29 a two-validator
        // chain sat at finality 44 while the height ran to 202 and nothing in the logs said why (#70).
        // The reason comes back from the gate itself, so it cannot disagree with the decision it
        // explains, and it is rate-limited to one line per `STALL_LOG_INTERVAL` heights.
        if let Some(reason) = &pre_state.finality_stall {
            let tip = pre_state.max_block_num;
            if tip.saturating_sub(LAST_STALL_LOG.load(Ordering::Relaxed)) >= STALL_LOG_INTERVAL {
                LAST_STALL_LOG.store(tip, Ordering::Relaxed);
                log.warn(
                    source,
                    &format!(
                        "finality did not advance at tip {tip}: {}",
                        describe_no_advance(reason)
                    ),
                );
            }
        }
        pre_state
    } else {
        // Genesis block: no parents.
        let genesis_pre_state_hash = empty_state_hash_fixed();
        ParentsMergedState {
            finality_stall: None,
            justifications: Vec::new(),
            max_block_num: 0,
            max_seq_nums: BTreeMap::from([(block.sender, 0)]),
            fringe: BTreeSet::new(),
            fringe_state: genesis_pre_state_hash,
            fringe_bonds_map: block.bonds.clone(),
            fringe_rejected_deploys: BTreeSet::new(),
            pre_state_hash: genesis_pre_state_hash,
            rejected_deploys: BTreeSet::new(),
        }
    };

    let incoming_pre_state_hash = Blake2b256Hash::from_byte_array(block.pre_state_hash.as_bytes());
    let result: Result<bool, BlockStatus> = if incoming_pre_state_hash != pre_state.pre_state_hash {
        Ok(false)
    } else if pre_state.fringe_rejected_deploys != block.rejected_deploys {
        Err(BlockStatus::InvalidRejectedDeploy)
    } else if slash_is_unjustified(dag, block).await? {
        Err(BlockStatus::UnjustifiedSlash)
    } else {
        let rand = BlockRandomSeed::random_generator_from_block(block);
        let post_state_hash = Blake2b256Hash::from_byte_array(block.post_state_hash.as_bytes());
        // Fork a fresh replay runtime at the block's pre-state (read-only history fork) so block
        // validation is self-contained and can run concurrently with other blocks.
        let forked = runtime
            .fork_replay_runtime(pre_state.pre_state_hash)
            .await?;
        let replay_result =
            replay_block(runtime, &forked, block, &pre_state.fringe_state, &rand).await;
        let handled = handle_errors(&post_state_hash, replay_result)?;
        Ok(handled.is_some())
    };

    let validation_failed = match &result {
        Err(_) => true,
        Ok(valid) => !*valid,
    };
    let bmd = BlockMetadata {
        validated: true,
        validation_failed,
        // A completed validation: the disagreement is between the block and the state, so it is
        // attributable to the block. Where a replay cannot be run at all, `mark_failed` is used and
        // sets this false instead.
        slashable: validation_failed,
        fringe: pre_state.fringe,
        fringe_state_hash: StateHash::from_slice(pre_state.fringe_state.as_bytes()),
        ..BlockMetadata::from_block(block)
    };

    Ok((bmd, result))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(byte: u8) -> Blake2b256Hash {
        Blake2b256Hash::from_bytes([byte; 32])
    }

    #[test]
    fn handle_errors_accepts_matching_hash() {
        assert_eq!(handle_errors(&hash(1), Ok(hash(1))).unwrap(), Some(hash(1)));
    }

    #[test]
    fn handle_errors_rejects_mismatching_hash() {
        assert_eq!(handle_errors(&hash(1), Ok(hash(2))).unwrap(), None);
    }

    #[test]
    fn handle_errors_raises_internal_error() {
        let r = handle_errors(&hash(1), Err(ReplayFailure::internal_error("boom")));
        assert!(r.is_err());
    }

    /// The condition the genesis-vault re-install hangs on (AUDIT C46). It has to be *exactly* the
    /// genesis: re-installing the balances on a later block's replay would clobber post-genesis
    /// balances, which is why the fix is conditional rather than unconditional.
    #[test]
    fn is_genesis_pre_state_is_true_only_for_the_empty_state() {
        assert!(
            is_genesis_pre_state(&empty_state_hash_fixed()),
            "the genesis pre-state is the empty state"
        );
        assert!(
            !is_genesis_pre_state(&hash(0xab)),
            "a post-genesis pre-state must not re-install the genesis vaults"
        );
    }

    #[test]
    fn handle_errors_soft_fails_on_replay_status_mismatch() {
        let r = handle_errors(
            &hash(1),
            Err(ReplayFailure::replay_status_mismatch(true, false)),
        );
        assert_eq!(r.unwrap(), None);
    }
}
