//! Interpreter utilities (port of `rholang/InterpreterUtil.scala`).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use rchain_block_storage::block_store::BlockStore;
use rchain_block_storage::dag::dag_storage::BlockDagStorage;
use rchain_block_storage::dag::finalizer::NoAdvance;
use rchain_crypto::hash::blake2b256_hash::Blake2b256Hash;
use rchain_crypto::hash::blake2b512_random::Blake2b512Random;
use rchain_models::ast::Par;
use rchain_models::block::state_hash::StateHash;
use rchain_models::block_hash::BlockHash;
use rchain_models::block_metadata::{BlockMetadata, FailureCause, SlashSeverity};
use rchain_models::casper::protocol::casper_message::{BlockMessage, SignedDeployData};
use rchain_models::fringe_data::FringeData;
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
    let evidence = crate::validate::slashed_evidence(block);
    // **A victim must be justified one of two ways, and both are re-derived here** (AUDIT C199, C200).
    //
    // The tier is *checked, not taken*: a victim must be an offender **at the tier this node derives**,
    // or a proposer would size the confiscation freely and the tiers would be advice. A slash recorded
    // before the tiers existed carries `Unspecified` — which takes everything — and a legacy record
    // derives the same, so an old block replays unchanged.
    //
    // The second way is evidence: a block the proposer attaches as proof of an **equivocation**, which
    // this node re-checks against **its own DAG** rather than believing. That is what lets the one
    // unambiguous Byzantine fault be punished without any node having to store the refused block.
    for (victim, tier) in &slashed {
        if justified.get(victim) == Some(tier) {
            continue;
        }
        let Some(bytes) = evidence.get(victim) else {
            return Ok(true);
        };
        if !crate::validate::equivocation_is_proved(dag, victim, bytes).await? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The tip height at which a finality stall was last logged. A `static` because the alternative is
/// threading a counter through every call for one log line — and the gate itself is stateless.
///
/// `i64::MIN` is the "never logged" sentinel, and the comparison is a `saturating_sub` for that reason:
/// plain subtraction would overflow on the first stall of a process's life, which is a panic in debug
/// and a wrapped negative in release — a rate limiter that never fires, on the path it exists for.
static LAST_STALL_LOG: AtomicI64 = AtomicI64::new(i64::MIN);
/// The last stall *kind* logged, so a **change of reason** is reported immediately.
///
/// **Why this exists, and it is the reason the discriminator has never been read.** The gate was
/// height-only — one line per `STALL_LOG_INTERVAL` (100) heights — and a short run's tip never leaves the
/// low thirties, so the *only* line a devnet can emit is the genesis-tip one. The 2026-09-30 campaign
/// measured finality pinning in 3 of 3 attempts and could not say why, because the instrument that
/// explains it was gated out of range. A stall that changes reason is the event worth reporting; the
/// height gate stays as the rate limit for a stall that does *not* change.
static LAST_STALL_DESC: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// How many heights may pass between two stall lines: one per hundred blocks keeps a stall that lasts
/// hundreds of blocks to a readable number of lines (#70's measurement ran ~160 blocks at finality 44).
const STALL_LOG_INTERVAL: i64 = 100;

/// The merge search's census, reported on the node's own log (#117). Wall-clock-gated rather than
/// height-gated: the instrument exists to be read off a *run*, and a run is measured in seconds. The
/// clock starts at the first report rather than at process start, so the first line is immediate.
static CENSUS_CLOCK: OnceLock<Instant> = OnceLock::new();
static LAST_CENSUS_LOG_MS: AtomicU64 = AtomicU64::new(0);
const CENSUS_LOG_INTERVAL_MS: u64 = 5000;

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
/// **The deploy identity of a block, for a state-hash disagreement** (#139).
///
/// The two hash values say a disagreement happened; this says *which deploy* was in flight when it
/// did, which is what the issue's close condition asks for ("names the deploy and the two hashes").
/// On this path the deploy that consumes the fringe is the `CloseBlock` system deploy — it anchors the
/// next epoch's seed to the fringe state — so whether one is present is said explicitly rather than
/// left for a reader to infer from a list of kinds.
fn describe_deploys(block: &BlockMessage) -> String {
    let user: Vec<String> = block
        .state
        .deploys
        .iter()
        .map(|d| {
            format!(
                "sig={} deployer={}",
                rchain_shared::base16::encode(&d.deploy.sig),
                rchain_shared::base16::encode(&d.deploy.deployer)
            )
        })
        .collect();
    let kinds: Vec<String> = block
        .state
        .system_deploys
        .iter()
        .map(|sd| format!("{:?}", sd))
        .collect();
    let close_block = block
        .state
        .system_deploys
        .iter()
        .any(|sd| format!("{sd:?}").contains("CloseBlock"));
    format!(
        "user_deploys={} [{}] system_deploys=[{}] close_block={}",
        user.len(),
        user.join(", "),
        kinds.join(", "),
        if close_block { "yes" } else { "no" }
    )
}

/// One line naming a state-hash disagreement and every input that could explain it (#139).
///
/// **Why this exists.** The failure reached an operator as
/// `Block <hex> failed validation: InvalidStateHash` — the variant name, no hashes, and no way to tell
/// a pre-state disagreement (this node's merge produced a different state) from a post-state one (the
/// merge agreed and the replay diverged). Those point at different causes, so a reproduction could not
/// "name the deploy and the two hashes" until this line existed.
///
/// The fringe fields are the load-bearing ones: the replay's `close_block` anchors the next epoch's
/// seed to the fringe state, and a node derives that state from **its own** DAG — it is not on the
/// wire (`validate_block_checkpoint`'s doc). So an empty `prev_fringe` with a `prev_fringe_lookup` of
/// the empty fringe is the signature of blocks that were *restored* rather than validated, and it
/// belongs in the line that is read when two nodes disagree.
#[allow(clippy::too_many_arguments)]
fn describe_state_mismatch(
    block: &BlockMessage,
    pre_state: &ParentsMergedState,
    predicate: &str,
    declared: &Blake2b256Hash,
    recomputed: Option<&Blake2b256Hash>,
) -> String {
    format!(
        "state-hash disagreement on {predicate}: block #{} {} by {} — declared {} vs recomputed {} \
         (pre_state={} fringe_state={} prev_fringe_lookup={} prev_fringe={:?}) {}",
        block.block_number,
        block.block_hash.to_hex(),
        short_id(&block.sender),
        declared.to_hex(),
        match recomputed {
            Some(h) => h.to_hex(),
            None => "none (the replay did not reach a state)".to_string(),
        },
        pre_state.pre_state_hash.to_hex(),
        pre_state.fringe_state.to_hex(),
        pre_state.prev_fringe_lookup.to_hex(),
        pre_state
            .prev_fringe
            .iter()
            .map(|h| h.to_hex())
            .collect::<Vec<_>>(),
        describe_deploys(block),
    )
}

/// One line naming a **`rejected_deploys`** disagreement — the one refusal on this path that used to say
/// nothing at all (#139, #140).
///
/// **Why it is its own line and not a `describe_state_mismatch` call.** The other refusals compare two
/// *hashes* and can name both sides as a pair. This one compares two **sets of rejected deploy ids** —
/// what this node's own merge filtered out against what the block says was filtered out — so the
/// evidence is not a scalar but *which ids are in one set and not the other*. The line therefore carries
/// the symmetric difference, capped at four ids a side, because these sets are peer-sized and this is a
/// `warn` per failing block.
///
/// The arm itself returned `Err(BlockStatus::InvalidRejectedDeploy)` with no log, so an operator saw a
/// status and no counts: the claimed set could be empty, or could be every deploy in the block, and the
/// log could not tell those apart.
fn describe_rejected_deploy_mismatch(
    block: &BlockMessage,
    pre_state: &ParentsMergedState,
) -> String {
    /// The ids present in `set` and absent from `other`, as hex, first four then a count of the rest.
    fn only(set: &BTreeSet<Vec<u8>>, other: &BTreeSet<Vec<u8>>) -> String {
        let diff: Vec<String> = set
            .difference(other)
            .map(|id| rchain_shared::base16::encode(id))
            .collect();
        match diff.len() {
            0 => "none".to_string(),
            n if n <= 4 => diff.join(", "),
            n => format!("{}, +{} more", diff[..4].join(", "), n - 4),
        }
    }
    format!(
        "state-hash disagreement on rejected-deploys: block #{} {} by {} — this node's merge computed {} \
         rejected ({} only here), the block claims {} ({} only there); fringe_state={} \
         prev_fringe_lookup={} {}",
        block.block_number,
        block.block_hash.to_hex(),
        short_id(&block.sender),
        pre_state.fringe_rejected_deploys.len(),
        only(&pre_state.fringe_rejected_deploys, &block.rejected_deploys),
        block.rejected_deploys.len(),
        only(&block.rejected_deploys, &pre_state.fringe_rejected_deploys),
        pre_state.fringe_state.to_hex(),
        pre_state.prev_fringe_lookup.to_hex(),
        describe_deploys(block),
    )
}

/// What a failed replay failed **on**, before `handle_errors` collapses it (#139, #140).
///
/// **Why the variant is worth a line of its own.** `handle_errors` keeps only two outcomes out of a
/// [`ReplayFailure`]: `InternalError` becomes an `Err` — the node's own fault, so no block is recorded
/// failed — and **every other variant becomes `Ok(None)`**, which is the same `InvalidStateHash` a plain
/// post-state mismatch produces. So "the replay disagreed about the post-state" and "the replay failed a
/// status, cost or comm check" reach an operator as one status, and the variant with its payload is what
/// separates them. It is read here, before that collapse.
///
/// The payloads are included rather than elided: a cost pair and a pair of booleans *are* the diagnosis,
/// and the error strings are already in this process's memory at this point. They can be peer-sized,
/// which is the same exposure `handle_errors` already has when it formats an `InternalError`'s cause into
/// an error.
fn describe_replay_failure(failure: &ReplayFailure) -> String {
    match failure {
        ReplayFailure::InternalError(cause) => format!("kind=InternalError cause={cause}"),
        ReplayFailure::ReplayStatusMismatch {
            initial_failed,
            replay_failed,
        } => format!(
            "kind=ReplayStatusMismatch initial_failed={initial_failed} replay_failed={replay_failed}"
        ),
        ReplayFailure::UnusedCommEvent(cause) => format!("kind=UnusedCommEvent cause={cause}"),
        ReplayFailure::ReplayCostMismatch {
            initial_cost,
            replay_cost,
        } => format!(
            "kind=ReplayCostMismatch initial_cost={initial_cost} replay_cost={replay_cost}"
        ),
        ReplayFailure::SystemDeployErrorMismatch {
            play_error,
            replay_error,
        } => format!(
            "kind=SystemDeployErrorMismatch play_error={play_error} replay_error={replay_error}"
        ),
    }
}

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
    Fut: std::future::Future<Output = Result<Arc<BlockIndex>, String>>,
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
        // **What the merge's conflict search was handed, from the running node (#117).** The search's
        // cost is a deterministic function of the conflict map, and the map's real shape — how wide a
        // scope gets, how dense its conflicts are — had never been observed, so the defect was argued
        // about with an assumed shape. `warn`, not `info`, so it is visible at the default level; the
        // five-second gate keeps a long run to a readable number of lines. See
        // `crate::merging::search_census`, and remove both with the fix.
        let elapsed_ms = CENSUS_CLOCK.get_or_init(Instant::now).elapsed().as_millis() as u64;
        if elapsed_ms.saturating_sub(LAST_CENSUS_LOG_MS.load(Ordering::Relaxed))
            >= CENSUS_LOG_INTERVAL_MS
        {
            LAST_CENSUS_LOG_MS.store(elapsed_ms, Ordering::Relaxed);
            log.warn(source, &crate::merging::search_census::summary());
        }
        // **Why finality is not advancing, in this node's own log.** On 2026-09-29 a two-validator
        // chain sat at finality 44 while the height ran to 202 and nothing in the logs said why (#70).
        // The reason comes back from the gate itself, so it cannot disagree with the decision it
        // explains, and it is rate-limited to one line per `STALL_LOG_INTERVAL` heights.
        if let Some(reason) = &pre_state.finality_stall {
            let tip = pre_state.max_block_num;
            // **Compare the rendered line, not the variant name.** The first version of this gate keyed on
            // the variant, and the devnet pin is `Support` from genesis onward — so the kind never changed
            // and the line never re-fired: the 2026-09-30 run produced three genesis-tip lines and nothing
            // about the pin. What moves is the *numbers* in the line (0 of 250 becomes 200 of 250 becomes
            // …), and those are the diagnosis, so the line firing on any change is what makes it readable.
            let line = describe_no_advance(reason);
            let changed = {
                let mut last = LAST_STALL_DESC.lock().unwrap_or_else(|p| p.into_inner());
                let changed = last.as_deref() != Some(line.as_str());
                if changed {
                    *last = Some(line.clone());
                }
                changed
            };
            if changed
                || tip.saturating_sub(LAST_STALL_LOG.load(Ordering::Relaxed)) >= STALL_LOG_INTERVAL
            {
                LAST_STALL_LOG.store(tip, Ordering::Relaxed);
                log.warn(
                    source,
                    &format!("finality did not advance at tip {tip}: {line}"),
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
            // The genesis has no parents, so the fringe it begins from is the empty one — the same
            // key a restored node's blocks carry, which is why the two are worth telling apart by the
            // *block number* in the log rather than by this field alone (#139).
            prev_fringe_lookup: FringeData::fringe_hash_of(&BTreeSet::new()),
            prev_fringe: BTreeSet::new(),
            fringe_bonds_map: block.bonds.clone(),
            fringe_rejected_deploys: BTreeSet::new(),
            pre_state_hash: genesis_pre_state_hash,
            rejected_deploys: BTreeSet::new(),
        }
    };

    let incoming_pre_state_hash = Blake2b256Hash::from_byte_array(block.pre_state_hash.as_bytes());
    // **Its own status, not a second way to say `InvalidStateHash`** (#139). `Ok(false)` reaches
    // `multi_parent_casper`'s `InvalidStateHash`, and so did this arm until it was split: an operator
    // could not tell a *pre*-state disagreement — this node's merge over the block's justifications
    // produced a different state — from a *post*-state one, where the merge agreed and the replay then
    // diverged. Those point at different causes, and naming which one fired is the first thing #139's
    // close condition asks for.
    let result: Result<bool, BlockStatus> = if incoming_pre_state_hash != pre_state.pre_state_hash {
        log.warn(
            source,
            &describe_state_mismatch(
                block,
                &pre_state,
                "pre-state",
                &incoming_pre_state_hash,
                Some(&pre_state.pre_state_hash),
            ),
        );
        Err(BlockStatus::InvalidPreStateHash)
    } else if pre_state.fringe_rejected_deploys != block.rejected_deploys {
        // **The one arm on this path that used to say nothing at all.** The other refusals carry a hash
        // disagreement and go through `describe_state_mismatch`; this one compared two sets and returned
        // `InvalidRejectedDeploy` with no log, so the claimed set could be empty or could be every deploy
        // in the block and the log could not tell those apart (Aria's #140 named the arm; this is it,
        // rewritten against the status split that landed after it).
        log.warn(
            source,
            &describe_rejected_deploy_mismatch(block, &pre_state),
        );
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
        // Read the recomputed hash *before* `handle_errors` consumes the result, so the disagreement
        // can be reported with both sides (#139).
        let recomputed = replay_result.as_ref().ok().cloned();
        // **A replay that failed and a replay that disagreed are different faults**, and `handle_errors`
        // collapses them: every variant but `InternalError` becomes `Ok(None)`, which is the same
        // `InvalidStateHash` a plain post-state mismatch produces. The variant and its payload are read
        // here, before that collapse (Aria's #140; the hash-side arms of that PR are already covered by
        // the disagreement lines above and below).
        if let Err(failure) = &replay_result {
            log.warn(
                source,
                &format!(
                    "replay failed on block #{} {} by {} — {} (declared post-state {} pre_state={} \
                     fringe_state={} prev_fringe_lookup={}) {}",
                    block.block_number,
                    block.block_hash.to_hex(),
                    short_id(&block.sender),
                    describe_replay_failure(failure),
                    post_state_hash.to_hex(),
                    pre_state.pre_state_hash.to_hex(),
                    pre_state.fringe_state.to_hex(),
                    pre_state.prev_fringe_lookup.to_hex(),
                    describe_deploys(block),
                ),
            );
        }
        let handled = handle_errors(&post_state_hash, replay_result)?;
        if handled.is_none() {
            log.warn(
                source,
                &describe_state_mismatch(
                    block,
                    &pre_state,
                    "post-state",
                    &post_state_hash,
                    recomputed.as_ref(),
                ),
            );
        }
        Ok(handled.is_some())
    };

    let validation_failed = match &result {
        Err(_) => true,
        Ok(valid) => !*valid,
    };
    let bmd = BlockMetadata {
        validated: true,
        validation_failed,
        // **The cause is not `mark_failed`'s to decide here.** Everything this function can refuse for
        // is a disagreement between the block and *this node's* state or replay — a pre-state hash it
        // did not derive, a rejected-deploy set from its own merge, a post-state it recomputed — so
        // the cause is `Divergence` and the failure is not the block's own fault (AUDIT C173). The
        // attribution is left unset (`mark_failed` sets both `slashable` and the cause from the
        // status; setting it here as well is what once made the flag survive a disagreement it did
        // not describe).
        failure_cause: validation_failed.then_some(FailureCause::Divergence),
        slash_severity: SlashSeverity::Unspecified,
        slashable: false,
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

    /// **The disagreement line names both hashes, the predicate, and whether a `CloseBlock` is
    /// present** (#139). Those three are the whole of what the issue's close condition asks a
    /// reproduction to produce, so the line that carries them is pinned rather than assumed: a later
    /// campaign reads *this* string out of a node's log, and a field silently dropped here would make
    /// every run's evidence unreadable in the same way, months after the change.
    ///
    /// The `close_block=yes` arm is deliberate: that is the system deploy that anchors the next
    /// epoch's seed to the fringe state, so it is the deploy a fringe-derived divergence appears on.
    /// A block and the pre-state it was merged against, with every field the mismatch lines read set to a
    /// distinct value. Shared so that a change to `BlockMessage` or `ParentsMergedState` breaks one
    /// fixture rather than one per line under test.
    fn fixture() -> (BlockMessage, ParentsMergedState) {
        let block = BlockMessage {
            version: 1,
            shard_id: "root".to_string(),
            block_hash: BlockHash::new([0xab; 32]),
            block_number: 42.try_into().unwrap(),
            sender: Validator::new([1u8; 65]),
            seq_num: 0.try_into().unwrap(),
            pre_state_hash: rchain_models::block::state_hash::StateHash::new([0x11; 32]),
            post_state_hash: rchain_models::block::state_hash::StateHash::new([0x22; 32]),
            justifications: vec![],
            bonds: BTreeMap::new(),
            rejected_deploys: BTreeSet::new(),
            rejected_blocks: BTreeSet::new(),
            rejected_senders: BTreeSet::new(),
            state: rchain_models::casper::protocol::casper_message::RholangState {
                deploys: vec![],
                system_deploys: vec![
                    rchain_models::casper::protocol::casper_message::ProcessedSystemDeploy::Succeeded {
                        event_list: vec![],
                        system_deploy:
                            rchain_models::casper::protocol::casper_message::SystemDeployData::CloseBlock,
                    },
                ],
            },
            sig_algorithm: "secp256k1".to_string(),
            sig: vec![1],
            timestamp: 0,
        };

        let pre_state = ParentsMergedState {
            finality_stall: None,
            justifications: vec![],
            max_block_num: 41,
            max_seq_nums: BTreeMap::new(),
            fringe: BTreeSet::new(),
            fringe_state: hash(0x33),
            prev_fringe_lookup: hash(0x44),
            prev_fringe: BTreeSet::new(),
            fringe_bonds_map: BTreeMap::new(),
            fringe_rejected_deploys: BTreeSet::new(),
            pre_state_hash: hash(0x11),
            rejected_deploys: BTreeSet::new(),
        };
        (block, pre_state)
    }

    /// A `BTreeSet<Vec<u8>>` of rejected deploy ids, for the fixture's overrides.
    fn ids(items: impl IntoIterator<Item = u8>) -> BTreeSet<Vec<u8>> {
        items.into_iter().map(|b| vec![b]).collect()
    }

    #[test]
    fn the_disagreement_line_names_both_hashes_and_the_predicate() {
        let (block, pre_state) = fixture();

        let line = describe_state_mismatch(
            &block,
            &pre_state,
            "post-state",
            &hash(0x22),
            Some(&hash(0x55)),
        );

        // Both sides of the disagreement.
        assert!(line.contains(&hash(0x22).to_hex()), "declared: {line}");
        assert!(line.contains(&hash(0x55).to_hex()), "recomputed: {line}");
        // Which comparison failed, and which block it was.
        assert!(line.contains("post-state"), "{line}");
        assert!(line.contains("#42"), "{line}");
        // The fringe inputs, without which a fringe-derived divergence cannot be read.
        assert!(line.contains(&hash(0x33).to_hex()), "fringe_state: {line}");
        assert!(
            line.contains(&hash(0x44).to_hex()),
            "prev_fringe_lookup: {line}"
        );
        // The deploy that consumes the fringe.
        assert!(line.contains("close_block=yes"), "{line}");

        // A replay that never produced a value is still a readable line rather than a panic or a
        // missing field — `handle_errors` returns `Ok(None)` for a soft failure as well as for a
        // mismatch.
        let none = describe_state_mismatch(&block, &pre_state, "post-state", &hash(0x22), None);
        assert!(none.contains("did not reach a state"), "{none}");
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

    /// **The arm that used to leave no trace** (#140). A `rejected_deploys` disagreement has to name both
    /// sets — so an empty claimed set is distinguishable from one holding every deploy — and, because the
    /// evidence is *which ids differ*, the difference between them.
    #[test]
    fn the_rejected_deploy_line_names_both_sets_and_what_differs() {
        let (mut block, mut pre_state) = fixture();
        // This node's merge filtered out `01` and `02`; the block claims `02` and `03`.
        pre_state.fringe_rejected_deploys = ids([0x01, 0x02]);
        block.rejected_deploys = ids([0x02, 0x03]);

        let line = describe_rejected_deploy_mismatch(&block, &pre_state);

        assert!(line.contains("rejected-deploys"), "{line}");
        assert!(line.contains("#42"), "{line}");
        // Both counts, which is what the arm logged nothing of before.
        assert!(line.contains("computed 2 rejected"), "computed: {line}");
        assert!(line.contains("claims 2"), "claimed: {line}");
        // The ids each side has and the other does not — the actual evidence.
        assert!(
            line.contains(&rchain_shared::base16::encode(&[0x01u8])),
            "only here: {line}"
        );
        assert!(
            line.contains(&rchain_shared::base16::encode(&[0x03u8])),
            "only there: {line}"
        );
        // The deploy that consumes the fringe, as the sibling disagreement line carries it.
        assert!(line.contains("close_block=yes"), "{line}");
    }

    /// …and a side with no ids of its own says `none` rather than printing an empty list, because that is
    /// what tells an operator the other set is a strict superset.
    #[test]
    fn the_rejected_deploy_line_says_none_when_one_side_is_a_superset() {
        let (mut block, mut pre_state) = fixture();
        pre_state.fringe_rejected_deploys = ids([0x01]);
        block.rejected_deploys = ids([0x01, 0x02]);

        let line = describe_rejected_deploy_mismatch(&block, &pre_state);

        assert!(line.contains("(none only here)"), "{line}");
        assert!(
            line.contains(&rchain_shared::base16::encode(&[0x02u8])),
            "only there: {line}"
        );
    }

    /// **Read before `handle_errors` collapses it** (#140). Four of the five variants become `Ok(None)` —
    /// the same status a plain post-state mismatch produces — so the variant is the only thing that
    /// separates "the replay disagreed" from "the replay failed a status, cost or comm check".
    #[test]
    fn the_replay_failure_line_names_every_variant_and_its_payload() {
        let cases = [
            (ReplayFailure::internal_error("boom"), "kind=InternalError"),
            (
                ReplayFailure::replay_status_mismatch(true, false),
                "initial_failed=true",
            ),
            (
                ReplayFailure::unused_comm_event("c"),
                "kind=UnusedCommEvent",
            ),
            (ReplayFailure::replay_cost_mismatch(7, 9), "replay_cost=9"),
            (
                ReplayFailure::system_deploy_error_mismatch("a", "b"),
                "play_error=a",
            ),
        ];

        let mut lines = BTreeSet::new();
        for (failure, needle) in &cases {
            let line = describe_replay_failure(failure);
            assert!(line.contains(needle), "{needle} missing from {line}");
            lines.insert(line);
        }
        // All five must be *distinguishable*: a line that named only the kind would pass the assertions
        // above while leaving the four soft-failed variants reading alike.
        assert_eq!(lines.len(), cases.len(), "one line per variant: {lines:?}");
    }
}
