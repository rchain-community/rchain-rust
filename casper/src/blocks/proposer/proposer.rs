//! Block proposer (port of `blocks/proposer/Proposer.scala`).
//!
//! `Proposer.apply` builds the dependency closures from the DAG/runtime; the `proposeEffect`
//! (broadcast via `CommUtil`) is supplied by the caller.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use rchain_block_storage::block_store::BlockStore;
use rchain_block_storage::dag::dag_storage::{BlockDagStorage, DeployId};
use rchain_block_storage::dag::liveness;
use rchain_block_storage::syntax::put_block;
use rchain_crypto::private_key::PrivateKey;
use rchain_crypto::signatures::secp256k1::Secp256k1;
use rchain_crypto::signatures::signed::Signed;
use rchain_models::block::state_hash::StateHash;
use rchain_models::block_hash::BlockHash;
use rchain_models::block_metadata::BlockMetadata;
use rchain_models::casper::protocol::casper_message::{BlockMessage, DeployData, SignedDeployData};
use rchain_models::validator::Validator;
use rchain_sdk::consensus::is_super_majority;
use rchain_shared::log::{Log, LogSource};
use rchain_shared::refined::{BlockHeight, NonNegI64};

use super::block_creator::BlockCreator;
use super::propose_result::{BlockCreatorResult, ProposeResult, ProposeStatus};
use crate::merging::BlockIndex;
use crate::multi_parent_casper::{get_pre_state_for_new_block, ValidateError, DEPLOY_LIFESPAN};
use crate::runtime_manager::RuntimeManager;
use crate::validator_identity::ValidatorIdentity;

type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

/// A proposal result signaled to the caller (port of `ProposerResult`).
#[derive(Clone, Debug)]
pub enum ProposerResult {
    Empty,
    Success {
        status: ProposeStatus,
        block: BlockMessage,
    },
    Failure {
        status: ProposeStatus,
        seq_number: i64,
        message: String,
    },
    Started {
        seq_number: i64,
    },
}

/// Whether `sender` is in the bonds map the DAG reports — extracted from the
/// `check_active_validator` closure so that its failure behaviour is testable without a proposer
/// fixture (the `load_node`/`load_node_from_store` split, applied to a DAG read).
///
/// **Fallible, because the oracle is.** `Proposer.scala:240-243` takes the bonds map through
/// `BlockDagStorage[F].lookupUnsafe`, which lifts an errored *or absent* lookup into an error in
/// `F` — so the Scala never answers "not bonded" for a DAG it could not read. The port's pre-fix body
/// flattened both into an empty bonds map, and an empty map answers `false` for every sender: a node
/// whose DAG could not be read would stop proposing and report `NotBonded`, with no error anywhere
/// (AUDIT C67, found by the U14 sweep; the falsifier's witnessing form is noted in the test below).
///
/// An *absent* block under a height-map key is an inconsistency, not an empty map: the height map
/// names blocks the message map holds, which is exactly what `lookupUnsafe` refuses.
pub(crate) async fn is_active_validator(
    dag: &Arc<dyn BlockDagStorage>,
    sender: &Validator,
) -> Result<bool, String> {
    let dag_repr = dag.get_representation().await;
    // The *newest* block's view of the active set - not the fringe's bond map, and not the lowest height's,
    // which is what this read before.
    //
    // Both of those are historical. The fringe's is the set as of the last finalised block, and the lowest
    // height's is the genesis's. A validator that bonds after either was fixed is in the chain's active set
    // but absent from them, and this check then refused to let it propose - forever, because nothing
    // refreshes those maps except a fringe that cannot advance without the very validators it expects to
    // speak. That is the whole of the "only the genesis validator ever proposes" symptom: the pool grows,
    // the active set grows, and everyone who joined afterwards is stuck read-only (#70).
    //
    // A block-carried map is safe *here* where it is not safe in the finaliser: this decides only whether
    // this node proposes, which changes nobody's state. The finaliser, which decides what finalises, reads
    // the bond map from the state.
    let bonds_map = match dag_repr.height_map.iter().next_back() {
        Some((height, hashes)) => match hashes.iter().next() {
            Some(h) => {
                dag.lookup(h)
                    .await?
                    .ok_or_else(|| {
                        format!(
                            "the DAG's height map names {} at height {}, but it is not in the DAG",
                            h.to_hex(),
                            i64::from(*height)
                        )
                    })?
                    .bonds_map
            }
            None => Default::default(),
        },
        None => Default::default(),
    };
    // A zero-stake entry is not a validator with a say; `select_active` never produces one.
    Ok(bonds_map
        .get(sender)
        .map(|stake| i64::from(*stake) > 0)
        .unwrap_or(false))
}

/// The block proposer (port of `Proposer`).
pub struct Proposer {
    get_latest_seq_number: Arc<dyn Fn(Validator) -> BoxFuture<i64> + Send + Sync>,
    check_active_validator:
        Arc<dyn Fn(&ValidatorIdentity) -> BoxFuture<Result<bool, String>> + Send + Sync>,
    create_block: Arc<
        dyn Fn(&ValidatorIdentity) -> BoxFuture<Result<BlockCreatorResult, String>> + Send + Sync,
    >,
    validate_block:
        Arc<dyn Fn(&BlockMessage) -> BoxFuture<Result<(), ValidateError>> + Send + Sync>,
    propose_effect: Arc<dyn Fn(&BlockMessage) -> BoxFuture<()> + Send + Sync>,
    validator: ValidatorIdentity,
    log: Arc<dyn Log>,
    consecutive_failures: Arc<AtomicU64>,
}

impl Proposer {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        get_latest_seq_number: Arc<dyn Fn(Validator) -> BoxFuture<i64> + Send + Sync>,
        check_active_validator: Arc<
            dyn Fn(&ValidatorIdentity) -> BoxFuture<Result<bool, String>> + Send + Sync,
        >,
        create_block: Arc<
            dyn Fn(&ValidatorIdentity) -> BoxFuture<Result<BlockCreatorResult, String>>
                + Send
                + Sync,
        >,
        validate_block: Arc<
            dyn Fn(&BlockMessage) -> BoxFuture<Result<(), ValidateError>> + Send + Sync,
        >,
        propose_effect: Arc<dyn Fn(&BlockMessage) -> BoxFuture<()> + Send + Sync>,
        validator: ValidatorIdentity,
        log: Arc<dyn Log>,
        consecutive_failures: Arc<AtomicU64>,
    ) -> Self {
        Proposer {
            get_latest_seq_number,
            check_active_validator,
            create_block,
            validate_block,
            propose_effect,
            validator,
            log,
            consecutive_failures,
        }
    }

    async fn do_propose(&self) -> Result<(ProposeResult, Option<BlockMessage>), String> {
        // A DAG that cannot be read is an error, not `NotBonded` (AUDIT C67): the oracle's
        // `lookupUnsafe` raises, and reporting "not bonded" would stop this node proposing on a wrong
        // reason with nothing in the log.
        match (self.check_active_validator)(&self.validator).await {
            Ok(true) => {}
            Ok(false) => {
                // The decision, logged where it is made — the same reason `close_block` logs its
                // boundary decision. `NotBonded` covers two states a validator can be in and an
                // operator cannot tell apart from the outside: *not in the pool*, and *in the pool but
                // not in the active set this epoch*. The second is new with the randomised draw
                // (`spec/RUST-VS-SCALA.md` §3 item 12) and it is the one that stops a node proposing
                // for a whole epoch with nothing else in the log: measured on a 3-validator devnet with
                // a cap of 2, where the drawn-out validator was the only one that could propose, the
                // chain halted and said nothing. The check itself reads the newest block's carried
                // bonds map, which is a state read, so every node agrees on the answer.
                eprintln!(
                    "[pos] not proposing: {} is not in the active set this node reads, \
                     which is the newest block's bond cache — either it is not bonded, or the \
                     draw left it out for this epoch",
                    rchain_shared::base16::encode(self.validator.public_key.bytes())
                );
                return Ok((
                    ProposeResult {
                        propose_status: ProposeStatus::NotBonded,
                    },
                    None,
                ));
            }
            Err(e) => return Err(format!("cannot decide whether this node is bonded: {e}")),
        }

        match (self.create_block)(&self.validator).await? {
            BlockCreatorResult::NoNewDeploys => Ok((
                ProposeResult {
                    propose_status: ProposeStatus::NoNewDeploys,
                },
                None,
            )),
            BlockCreatorResult::Created(block) => match (self.validate_block)(&block).await {
                Ok(()) => {
                    self.consecutive_failures.store(0, Ordering::Relaxed);
                    (self.propose_effect)(&block).await;
                    Ok((
                        ProposeResult {
                            propose_status: ProposeStatus::ProposeSuccess,
                        },
                        Some(block),
                    ))
                }
                Err(ValidateError::ValidationFailed(_, status)) => {
                    // Defence in depth: a self-created block that fails its own validation means the
                    // node's state accounting is inconsistent — surface it loudly (with the exact
                    // status + block/seq) and count it so the caller can halt autopropose.
                    self.consecutive_failures.fetch_add(1, Ordering::Relaxed);
                    self.log.error(
                        LogSource::new("casper.blocks.Proposer"),
                        &format!(
                            "Self-created block #{} (seq {}) failed validation: {status} — node state accounting is inconsistent.",
                            block.block_number, block.seq_num
                        ),
                    );
                    Err(format!(
                        "the node rejected its own block #{block} (seq {seq}): {status}. \
                         This is a node-side bug, not a problem with your request.",
                        block = block.block_number,
                        seq = block.seq_num,
                    ))
                }
                Err(ValidateError::Internal(e)) => {
                    self.consecutive_failures.fetch_add(1, Ordering::Relaxed);
                    self.log.error(
                        LogSource::new("casper.blocks.Proposer"),
                        &format!(
                            "Self-created block #{} (seq {}) failed validation with internal error: {e}",
                            block.block_number, block.seq_num
                        ),
                    );
                    Err(format!(
                        "the node rejected its own block #{block} (seq {seq}) with an internal \
                         error: {e}. This is a node-side bug, not a problem with your request.",
                        block = block.block_number,
                        seq = block.seq_num,
                    ))
                }
            },
        }
    }

    pub async fn propose(
        &self,
        is_async: bool,
        propose_id: tokio::sync::oneshot::Sender<ProposerResult>,
    ) -> Result<(ProposeResult, Option<BlockMessage>), String> {
        let validator = Validator::from_slice(self.validator.public_key.bytes());
        let next_seq = (self.get_latest_seq_number)(validator).await + 1;

        if is_async {
            let _ = propose_id.send(ProposerResult::Started {
                seq_number: next_seq,
            });
            self.do_propose().await
        } else {
            let result = self.do_propose().await;
            let proposer_result = match &result {
                Ok((result, Some(block))) => ProposerResult::Success {
                    status: result.propose_status.clone(),
                    block: block.clone(),
                },
                Ok((result, None)) => ProposerResult::Failure {
                    status: result.propose_status.clone(),
                    seq_number: next_seq,
                    message: result.propose_status.to_string(),
                },
                Err(e) => ProposerResult::Failure {
                    status: ProposeStatus::BugError(e.clone()),
                    seq_number: next_seq,
                    message: e.clone(),
                },
            };
            let _ = propose_id.send(proposer_result);
            result
        }
    }

    /// Build a `Proposer` from its dependencies (port of `Proposer.apply`). The DAG/block-store/
    /// runtime/block-index are captured by `Arc` so the returned proposer is `'static` and can be
    /// driven from a spawned task.
    #[allow(clippy::too_many_arguments)]
    pub fn apply<F, Fut>(
        validator_identity: ValidatorIdentity,
        shard_id: String,
        min_phlo_price: i64,
        epoch_length: i32,
        dummy_deploy_opt: Option<(PrivateKey, String)>,
        dag: Arc<dyn BlockDagStorage>,
        block_store: BlockStore,
        runtime: Arc<RuntimeManager>,
        block_index: F,
        propose_effect: Arc<dyn Fn(&BlockMessage) -> BoxFuture<()> + Send + Sync>,
        log: Arc<dyn Log>,
        consecutive_failures: Arc<AtomicU64>,
    ) -> Proposer
    where
        F: Fn(BlockHash) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Arc<BlockIndex>, String>> + Send + 'static,
    {
        let block_index = Arc::new(block_index);

        let get_latest_seq_number: Arc<dyn Fn(Validator) -> BoxFuture<i64> + Send + Sync> = {
            let dag = dag.clone();
            Arc::new(move |sender| {
                let dag = dag.clone();
                Box::pin(async move {
                    let dag_repr = dag.get_representation().await;
                    dag_repr
                        .dag_message_state
                        .latest_msgs
                        .get(&sender)
                        .map(|m| i64::from(m.sender_seq))
                        .unwrap_or(-1)
                })
            })
        };

        let check_active_validator: Arc<
            dyn Fn(&ValidatorIdentity) -> BoxFuture<Result<bool, String>> + Send + Sync,
        > = {
            let dag = dag.clone();
            Arc::new(move |vi: &ValidatorIdentity| {
                let sender = Validator::from_slice(vi.public_key.bytes());
                let dag = dag.clone();
                Box::pin(async move { is_active_validator(&dag, &sender).await })
            })
        };

        let create_block: Arc<
            dyn Fn(&ValidatorIdentity) -> BoxFuture<Result<BlockCreatorResult, String>>
                + Send
                + Sync,
        > = {
            let runtime = runtime.clone();
            let dag = dag.clone();
            let block_store = block_store.clone();
            let block_index = block_index.clone();
            let shard_id = shard_id.clone();
            let dummy_deploy_opt = dummy_deploy_opt.clone();
            Arc::new(move |vi: &ValidatorIdentity| {
                let runtime = runtime.clone();
                let dag = dag.clone();
                let block_store = block_store.clone();
                let block_index = block_index.clone();
                let vi = vi.clone();
                let shard_id = shard_id.clone();
                let dummy_deploy_opt = dummy_deploy_opt.clone();
                Box::pin(async move {
                    create_block(
                        runtime.as_ref(),
                        dag.as_ref(),
                        &block_store,
                        block_index.as_ref(),
                        &vi,
                        &shard_id,
                        epoch_length,
                        dummy_deploy_opt.as_ref(),
                    )
                    .await
                })
            })
        };

        let validate_block: Arc<
            dyn Fn(&BlockMessage) -> BoxFuture<Result<(), ValidateError>> + Send + Sync,
        > = {
            let runtime = runtime.clone();
            let dag = dag.clone();
            let block_store = block_store.clone();
            let block_index = block_index.clone();
            let shard_id = shard_id.clone();
            let log = log.clone();
            Arc::new(move |block: &BlockMessage| {
                let runtime = runtime.clone();
                let dag = dag.clone();
                let block_store = block_store.clone();
                let block_index = block_index.clone();
                let block = block.clone();
                let shard_id = shard_id.clone();
                let log = log.clone();
                Box::pin(async move {
                    match crate::multi_parent_casper::validate(
                        dag.as_ref(),
                        &block_store,
                        runtime.as_ref(),
                        &block,
                        &shard_id,
                        min_phlo_price,
                        block_index.as_ref(),
                        &log,
                    )
                    .await
                    {
                        Ok(meta) => {
                            // Persist the block body BEFORE recording it in the DAG. `insert`
                            // writes the deploy index + DAG state but not the full block, so a
                            // read landing between the two would otherwise hit "missing block".
                            put_block(&block_store, block.clone()).await.map_err(|e| {
                                ValidateError::Internal(format!("failed to store block body: {e}"))
                            })?;
                            dag.insert(meta, block.clone()).await.map_err(|e| {
                                ValidateError::Internal(format!(
                                    "failed to insert block into DAG: {e}"
                                ))
                            })?;
                            Ok(())
                        }
                        Err(err) => Err(err),
                    }
                })
            })
        };

        Proposer::new(
            get_latest_seq_number,
            check_active_validator,
            create_block,
            validate_block,
            propose_effect,
            validator_identity,
            log,
            consecutive_failures,
        )
    }
}

async fn get_block(
    block_store: &BlockStore,
    hash: &BlockHash,
) -> Result<Option<BlockMessage>, String> {
    let mut vals = block_store.get(&[*hash]).await?;
    Ok(vals.pop().flatten())
}

/// The most deploys one block may carry, before the slashes it also has to seed (AUDIT C123).
///
/// **The bound is the per-deploy randomness seed, which is indexed in a `u8`.** `BlockCreator::create`
/// derives each deploy's seed with `rand.split_byte(u8::try_from(selected.len() + i)?)` and the
/// `close_block` seed with `split_byte(u8::try_from(selected.len() + to_slash.len())?)`
/// (`block_creator.rs`), so a block's deploy count **plus** its slash count must fit in 255. Nothing
/// bounded the selection, and the pool's own cap is 10,000 — set for a different reason entirely — so a
/// pool of 256 valid deploys made `create` fail *before any state work*: the node could not produce a
/// block at all, on any shard, until the pool drained below 256.
///
/// **The cap is on the selection and not on the seed**: widening the index would hide the bound rather
/// than respect it, and the deploys a block cannot carry are not lost — the pool keeps them and the next
/// block takes them. The slash count needs no cap of its own: `to_slash` comes from the block's
/// justifications, which are bounded by `max-number-of-parents`, so it is small by construction.
pub const MAX_BLOCK_DEPLOYS: usize = 255; // `u8::MAX` — the width of the seed index

/// The deploys one block may carry, given how many validators it also has to seed — see
/// [`MAX_BLOCK_DEPLOYS`] for why the *sum* is what has to fit.
fn per_block_deploy_budget(slashes: usize) -> usize {
    MAX_BLOCK_DEPLOYS.saturating_sub(slashes)
}

/// The deploys this block will carry: the pool's valid entries in their canonical order, truncated to
/// `budget` (AUDIT C123).
///
/// "Valid" is the Scala proposer's filter — not future, not expired, not already in the DAG (the replay
/// guard). The order is canonical because the pool is a `BTreeMap` keyed by `DeployId`, so the prefix the
/// budget takes is the same for every node holding the same pool. The loop stops *reading* at the budget
/// as well as collecting, so an oversized pool costs no more DAG lookups than the block can carry.
async fn select_deploys(
    dag: &dyn BlockDagStorage,
    next_block_num: BlockHeight,
    budget: usize,
) -> Result<Vec<DeployId>, String> {
    let pooled = dag.pooled_deploys().await?;
    let mut deploys: Vec<DeployId> = Vec::new();
    for (id, d) in pooled {
        let future = d.data.valid_after_block_number > i64::from(next_block_num);
        let expired = d.data.valid_after_block_number < next_block_num - DEPLOY_LIFESPAN;
        let replay_attack = dag.lookup_by_deploy_id(&id).await?.is_some();
        if !(future || expired || replay_attack) {
            deploys.push(id);
            if deploys.len() == budget {
                break;
            }
        }
    }
    Ok(deploys)
}

#[allow(clippy::too_many_arguments)]
async fn create_block<'a, F, Fut>(
    runtime: &'a RuntimeManager,
    dag: &'a dyn BlockDagStorage,
    block_store: &'a BlockStore,
    block_index: &'a F,
    validator_identity: &ValidatorIdentity,
    shard_id: &str,
    epoch_length: i32,
    dummy_deploy_opt: Option<&(PrivateKey, String)>,
) -> Result<BlockCreatorResult, String>
where
    F: Fn(BlockHash) -> Fut + Sync,
    Fut: Future<Output = Result<Arc<BlockIndex>, String>>,
{
    let creators_validator_for_parents =
        Validator::from_slice(validator_identity.public_key.bytes());
    let pre_state = get_pre_state_for_new_block(
        dag,
        block_store,
        runtime,
        block_index,
        &creators_validator_for_parents,
    )
    .await?;
    let pre_state_hash = pre_state.pre_state_hash;
    let creators_validator = Validator::from_slice(validator_identity.public_key.bytes());
    let next_block_num = pre_state
        .justifications
        .iter()
        .map(|m| m.block_num)
        .max()
        .map(|m| m + NonNegI64::one())
        .unwrap_or_else(BlockHeight::zero);
    let parent_hashes: Vec<BlockHash> = pre_state
        .justifications
        .iter()
        .map(|m| m.block_hash)
        .collect();

    let pre_state_bonds = runtime
        .compute_bonds(&StateHash::from_slice(pre_state_hash.as_bytes()))
        .await?;
    let bonded: BTreeSet<Validator> = pre_state_bonds
        .iter()
        .filter(|(_, b)| i64::from(**b) > 0)
        .map(|(v, _)| *v)
        .collect();
    let to_slash: BTreeSet<Validator> = slashable_offenders(&pre_state.justifications, &bonded);
    if !to_slash.is_empty() {
        // The consequence, logged where it is decided. The validation failure that caused it is already
        // logged by the block processor; nothing connected the two, so a slashing used to be visible only as
        // the pool getting smaller later on (`getBonds` counting one fewer validator).
        eprintln!(
            "[pos] slashing {} bonded validator(s) whose block failed validation here: {}",
            to_slash.len(),
            to_slash
                .iter()
                .map(|v| rchain_shared::base16::encode(v.as_bytes())
                    .chars()
                    .take(8)
                    .collect::<String>())
                .collect::<Vec<_>>()
                .join(" ")
        );
    }

    // An epoch boundary is a block whose height is a positive multiple of `epoch_length` (matching
    // the PoS contract's `blockNumber % epochLength == 0`). A non-positive `epoch_length` disables
    // the epoch trigger (the native PoS lifecycle is otherwise block-driven).
    let change_epoch = epoch_length > 0
        && i64::from(next_block_num) != 0
        && i64::from(next_block_num) % i64::from(epoch_length) == 0;

    // Attestation suppression: no new state transitions, or not yet a super-majority.
    let dag_repr = dag.get_representation().await;
    // The ids are what this needs (the two consumers below are a union and a difference), so it
    // yields them directly rather than cloning the message that holds them — a `Message` clone used
    // to drag its whole ancestor set along (AUDIT C56).
    let seen = |h: &BlockHash| -> Vec<BlockHash> {
        dag_repr
            .dag_message_state
            .msg_map
            .get(h)
            .map(|m| m.seen.iter().copied().collect())
            .unwrap_or_default()
    };
    let parent_seen: BTreeSet<BlockHash> = parent_hashes.iter().flat_map(|h| seen(h)).collect();
    let fringe_seen: BTreeSet<BlockHash> = pre_state.fringe.iter().flat_map(|h| seen(h)).collect();
    let conflict_set: Vec<BlockHash> = parent_seen.difference(&fringe_seen).copied().collect();

    let has_deploys =
        |b: &BlockMessage| !b.state.system_deploys.is_empty() || !b.state.deploys.is_empty();

    let mut nothing_to_finalize = true;
    for h in &conflict_set {
        if let Some(b) = get_block(block_store, h).await? {
            if has_deploys(&b) {
                nothing_to_finalize = false;
                break;
            }
        }
    }

    // Attestation timing (the port's `newlySeen`/`waitingForSupermajority` guard, corrected).
    //
    // The port derived "newly seen" as `seen(our own latest message's justifications) \ seen(parents)`.
    // Our own message is one of the parents and everything it justifies is an ancestor of the parents, so
    // that difference is empty by construction: `new_state_transition` was always false and
    // `attestation_stake` always zero, hence `waiting_for_supermajority` always true. The empty-attestation
    // branch in `block_creator.rs` was therefore unreachable, and the dev-mode dummy deploy was the only
    // thing that ever got past this guard — which is why `--autopropose` could not produce a block on its
    // own. See #70.
    //
    // What the guard needs is (a) whether an unfinalized state transition exists to attest to, and (b) how
    // much stake is moving on the fringe we are building on. (b) is the senders of the *parents'* latest
    // messages, excluding ourselves: our own message is the attestation we are about to add, and it is
    // counted on the other side of the comparison.
    let parents: Vec<BlockMessage> = {
        let mut v = Vec::new();
        for h in &parent_hashes {
            if let Some(b) = get_block(block_store, h).await? {
                v.push(b);
            }
        }
        v
    };
    // The guard's inputs are named functions rather than inline sums, so that its behaviour is
    // falsifiable without a DAG: `moving_attestation_stake` (who counts as moving),
    // `attestation_suppressed` (the decision) and `cadence_due` (the pace bound). `tip` is the newest
    // height this node can see, which is the datum the recency checks need.
    let tip = pre_state
        .justifications
        .iter()
        .map(|m| m.block_num)
        .max()
        .unwrap_or_else(BlockHeight::zero);
    let new_state_transition = parents.iter().any(|b| has_deploys(b));
    // The live weight set: the one liveness rule, shared with the finalizer through `liveness` (#70
    // increment 2). It decides who counts as *moving*; the quorum itself stays measured against the
    // whole bonded map below, so a minority cannot attest its way to a supermajority.
    let live_bonds = liveness::live_weight_set(
        &pre_state_bonds,
        &liveness::latest_heights(
            pre_state
                .justifications
                .iter()
                .map(|m| (m.sender.clone(), m.block_num)),
        ),
        tip,
        liveness::LIVENESS_WINDOW,
    );
    let pre_state_bonds_stake: i128 = pre_state_bonds
        .values()
        .map(|s| i128::from(i64::from(*s)))
        .sum();
    let own_stake: i128 = pre_state_bonds
        .iter()
        .filter(|(v, _)| **v == creators_validator)
        .map(|(_, s)| i128::from(i64::from(*s)))
        .sum();
    let attestation_stake = moving_attestation_stake(&live_bonds, &creators_validator);
    let quorum_reachable =
        attestation_reaches_supermajority(attestation_stake, own_stake, pre_state_bonds_stake);

    let suppress_attestation = attestation_suppressed(
        nothing_to_finalize,
        new_state_transition,
        quorum_reachable,
        cadence_due(&pre_state.justifications, &creators_validator, tip),
    );

    // User deploys: filter future / expired / replayed, then cap at what the block can seed — the pool
    // may hold far more than one block can carry, and the leftover stays pooled (AUDIT C123).
    let mut deploys =
        select_deploys(dag, next_block_num, per_block_deploy_budget(to_slash.len())).await?;

    // Dev-mode dummy deploy: when there is nothing pooled to include, inject a signed `Nil` deploy so
    // `--autopropose` can keep producing blocks (port of Scala `Proposer.dummyDeployOpt`).
    if deploys.is_empty() {
        if let Some((key, term)) = dummy_deploy_opt {
            eprintln!(
                "No pooled deploys; injecting dummy deploy for block #{}",
                i64::from(next_block_num)
            );
            let data = DeployData {
                attachments: Vec::new(),
                term: term.clone(),
                timestamp: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0),
                phlo_price: 1,
                phlo_limit: 90000,
                valid_after_block_number: i64::from(next_block_num) - 1,
                shard_id: shard_id.to_string(),
            };
            let signed_deploy = {
                // `Signed` holds a `&'static dyn SignaturesAlg` (not `Sync`); drop it before the
                // `.await` below so the future stays `Send`.
                let signed = Signed::new(data, &Secp256k1, key).map_err(|e| e.to_string())?;
                SignedDeployData {
                    data: signed.data,
                    deployer: signed.pk.bytes().to_vec(),
                    sig: signed.sig,
                    sig_algorithm: signed.sig_algorithm.name().to_string(),
                }
            };
            let id = crate::multi_parent_casper::add_deploy(dag, &signed_deploy).await?;
            deploys.push(id);
        }
    }

    BlockCreator {
        id: validator_identity.clone(),
        shard_id: shard_id.to_string(),
    }
    .create(
        runtime,
        dag,
        &pre_state,
        &deploys,
        &to_slash,
        change_epoch,
        suppress_attestation,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `BlockDagStorage` whose `lookup` fails, so a DAG read error is observable as one, and whose
    /// representation has an **empty fringe with a non-empty height map** — the shape that sends
    /// `is_active_validator` to the `lookup` branch.
    struct FailingLookupDag {
        first_hash: BlockHash,
    }

    #[async_trait::async_trait]
    impl BlockDagStorage for FailingLookupDag {
        async fn get_representation(
            &self,
        ) -> Arc<rchain_block_storage::dag::representation::DagRepresentation> {
            Arc::new(
                rchain_block_storage::dag::representation::DagRepresentation {
                    dag_set: Arc::new(BTreeSet::new()),
                    child_map: Arc::new(std::collections::BTreeMap::new()),
                    height_map: Arc::new(std::collections::BTreeMap::from([(
                        BlockHeight::try_from(1).unwrap(),
                        BTreeSet::from([self.first_hash]),
                    )])),
                    dag_message_state:
                        rchain_block_storage::dag::message_state::DagMessageState::empty(),
                    fringe_states: std::collections::BTreeMap::new(),
                },
            )
        }

        /// The failing read: a DAG that cannot answer is the whole point of this stub.
        async fn lookup(
            &self,
            _block_hash: &BlockHash,
        ) -> Result<Option<rchain_models::block_metadata::BlockMetadata>, String> {
            Err("the DAG store is down".to_string())
        }

        // The rest of the trait is not on this path.
        async fn insert(
            &self,
            _block_metadata: rchain_models::block_metadata::BlockMetadata,
            _block: BlockMessage,
        ) -> Result<(), String> {
            todo!("not on the is_active_validator path")
        }
        async fn lookup_by_deploy_id(&self, _d: &DeployId) -> Result<Option<BlockHash>, String> {
            todo!("not on the is_active_validator path")
        }
        async fn add_deploy(&self, _d: SignedDeployData) -> Result<(), String> {
            todo!("not on the is_active_validator path")
        }
        async fn pooled_deploys(
            &self,
        ) -> Result<std::collections::BTreeMap<DeployId, SignedDeployData>, String> {
            todo!("not on the is_active_validator path")
        }
        async fn contains_deploy_in_pool(&self, _d: &DeployId) -> Result<bool, String> {
            todo!("not on the is_active_validator path")
        }
    }

    /// **A DAG read that fails is not "this node is not bonded"** (AUDIT C67).
    ///
    /// The oracle is `Proposer.scala:240-243`, whose `else` branch takes the bonds map through
    /// `BlockDagStorage[F].lookupUnsafe` — an errored lookup is an error in `F`, so the Scala never
    /// answers "not bonded" for it. The port flattened the error into an empty bonds map, and an
    /// empty map answers `false` for every sender, so a node whose DAG could not be read would stop
    /// proposing and report `NotBonded` — a consensus-adjacent wrong answer with no error anywhere.
    ///
    /// Falsifier, both forms. Pre-fix (witnessing): `is_active_validator` answered `false` for a
    /// failing `lookup`, and the assertion `!is_active_validator(..)` **passed on exactly that**
    /// (run 2026-09-24 before the change). Post-fix: the same call is an `Err` naming the failure.
    #[tokio::test]
    async fn a_dag_read_that_fails_is_not_an_inactive_validator() {
        let dag: Arc<dyn BlockDagStorage> = Arc::new(FailingLookupDag {
            first_hash: BlockHash::new([0x11; 32]),
        });
        let sender = Validator::new([0x22; rchain_models::validator::LENGTH]);

        let err = is_active_validator(&dag, &sender)
            .await
            .expect_err("a DAG that cannot be read must not answer \"not an active validator\"");
        assert!(
            err.contains("the DAG store is down"),
            "and the refusal must name the failure, got: {err}"
        );
    }

    /// A DAG whose pool is whatever the test hands it, and which has never seen a deploy — so the replay
    /// guard never fires. Every other trait method is off the selection path.
    struct PoolDag {
        pooled: std::collections::BTreeMap<DeployId, SignedDeployData>,
    }

    #[async_trait::async_trait]
    impl BlockDagStorage for PoolDag {
        async fn get_representation(
            &self,
        ) -> Arc<rchain_block_storage::dag::representation::DagRepresentation> {
            todo!("not on the selection path")
        }
        async fn insert(
            &self,
            _m: rchain_models::block_metadata::BlockMetadata,
            _b: BlockMessage,
        ) -> Result<(), String> {
            todo!("not on the selection path")
        }
        async fn lookup(
            &self,
            _h: &BlockHash,
        ) -> Result<Option<rchain_models::block_metadata::BlockMetadata>, String> {
            todo!("not on the selection path")
        }
        async fn lookup_by_deploy_id(&self, _d: &DeployId) -> Result<Option<BlockHash>, String> {
            Ok(None)
        }
        async fn add_deploy(&self, _d: SignedDeployData) -> Result<(), String> {
            todo!("not on the selection path")
        }
        async fn pooled_deploys(
            &self,
        ) -> Result<std::collections::BTreeMap<DeployId, SignedDeployData>, String> {
            Ok(self.pooled.clone())
        }
        async fn contains_deploy_in_pool(&self, _d: &DeployId) -> Result<bool, String> {
            todo!("not on the selection path")
        }
    }

    /// A pooled deploy valid at the block being proposed (`add_deploy`'s shape, minimal).
    fn pooled(sig: u16, valid_after: i64) -> SignedDeployData {
        SignedDeployData {
            data: DeployData {
                attachments: Vec::new(),
                term: "Nil".to_string(),
                timestamp: 0,
                phlo_price: 1,
                phlo_limit: 1,
                valid_after_block_number: valid_after,
                shard_id: "root".to_string(),
            },
            deployer: vec![0u8; 65],
            sig: sig.to_le_bytes().to_vec(),
            sig_algorithm: "secp256k1".to_string(),
        }
    }

    /// **The regression test for AUDIT C123's bound**, and it is arithmetic on purpose: the invariant the
    /// per-deploy seed needs is `deploys + slashes ≤ 255`. A test that only exercised a pool of 300 would
    /// pass while the *bound itself* drifted, so this asserts the bound.
    ///
    /// `budget(0) == 255` is the half that matters: it is what makes a selection of 256 unreachable, and
    /// 256 pooled deploys were exactly what stopped block production — `BlockCreator::create` computed
    /// `u8::try_from(selected.len() + to_slash.len())` and returned `Err` *before any state work*, so the
    /// node could not produce a block at all until the pool drained.
    #[test]
    fn the_per_block_budget_leaves_room_for_every_seed_the_block_needs() {
        assert_eq!(
            per_block_deploy_budget(0),
            MAX_BLOCK_DEPLOYS,
            "a block with no slashes may still not select 256 deploys"
        );
        for slashes in 0..=MAX_BLOCK_DEPLOYS {
            assert!(
                per_block_deploy_budget(slashes) + slashes <= MAX_BLOCK_DEPLOYS,
                "the seed index is a u8, so {slashes} slashes leave {} deploys",
                per_block_deploy_budget(slashes)
            );
        }
        // Past the bound the budget saturates rather than wrapping — a slash count that large is
        // unreachable (`to_slash` comes from the block's justifications), and a wrap here would be the
        // arithmetic that got C123 into trouble in the first place.
        assert_eq!(per_block_deploy_budget(MAX_BLOCK_DEPLOYS + 1), 0);
    }

    /// **The other half of C123**: the selection is a bounded, deterministic prefix of the pool's valid
    /// entries, so a pool larger than one block can carry produces a block instead of an error.
    #[tokio::test]
    async fn the_selection_is_capped_and_takes_the_pools_canonical_prefix() {
        // Two bytes per key, little-endian so the map's order is the index order — no cast and no
        // division, which also keeps this fixture out of the partiality gate's `div`/`cast` classes.
        let key = |i: u16| i.to_le_bytes().to_vec();
        let pooled_map: std::collections::BTreeMap<DeployId, SignedDeployData> =
            (0..300u16).map(|i| (key(i), pooled(i, 0))).collect();
        let dag = PoolDag { pooled: pooled_map };
        let next = BlockHeight::try_from(1).unwrap();

        let selected = select_deploys(&dag, next, per_block_deploy_budget(0))
            .await
            .unwrap();
        assert_eq!(
            selected.len(),
            MAX_BLOCK_DEPLOYS,
            "a pool three times the block's capacity must yield a block-sized selection"
        );
        // The prefix is the pool's **canonical order**, and the assertion takes that order from the map
        // rather than from a hand-written list: the first draft of this test spelled the expected keys
        // out and got them wrong (`Vec<u8>` keys sort lexicographically, so `[0,1]` precedes `[1,0]` and
        // the prefix is not the first N indices). What has to hold is the *property* — the selection is
        // the pool's own order, truncated — and that is what this states.
        let mut all_keys: Vec<DeployId> = dag.pooled.keys().cloned().collect();
        all_keys.sort();
        let expected: Vec<DeployId> = all_keys[..MAX_BLOCK_DEPLOYS].to_vec();
        assert_eq!(selected, expected);

        // A slash takes a seed as well, so it takes a deploy's place.
        let with_one_slash = select_deploys(&dag, next, per_block_deploy_budget(1))
            .await
            .unwrap();
        assert_eq!(with_one_slash.len(), MAX_BLOCK_DEPLOYS - 1);

        // And the pool's own filter still runs: an expired deploy is not selected at all.
        let mut with_expired = std::collections::BTreeMap::new();
        with_expired.insert(vec![0u8, 9u8], pooled(9, 0));
        with_expired.insert(vec![0u8, 10u8], pooled(10, -(DEPLOY_LIFESPAN + 1)));
        let dag = PoolDag {
            pooled: with_expired,
        };
        assert_eq!(
            select_deploys(&dag, next, per_block_deploy_budget(0))
                .await
                .unwrap(),
            vec![vec![0u8, 9u8]],
            "the expiry filter must still apply under the cap"
        );
    }

    fn proposer(create: BlockCreatorResult) -> Proposer {
        let get_seq: Arc<dyn Fn(Validator) -> BoxFuture<i64> + Send + Sync> =
            Arc::new(|_v| Box::pin(async { 0i64 }));
        let check_active: Arc<
            dyn Fn(&ValidatorIdentity) -> BoxFuture<Result<bool, String>> + Send + Sync,
        > = Arc::new(|_v| Box::pin(async { Ok(true) }));
        let create_block: Arc<
            dyn Fn(&ValidatorIdentity) -> BoxFuture<Result<BlockCreatorResult, String>>
                + Send
                + Sync,
        > = Arc::new(move |_v| {
            let create = create.clone();
            Box::pin(async move { Ok(create) })
        });
        let validate: Arc<
            dyn Fn(&BlockMessage) -> BoxFuture<Result<(), ValidateError>> + Send + Sync,
        > = Arc::new(|_b| Box::pin(async { Ok(()) }));
        let effect: Arc<dyn Fn(&BlockMessage) -> BoxFuture<()> + Send + Sync> =
            Arc::new(|_b| Box::pin(async {}));
        let validator = ValidatorIdentity::from_hex(
            "67e56582298859ddae725f972992a07c6c4fb9f62a8fff58ce3ca926a1063530",
        )
        .unwrap();
        let log: Arc<dyn Log> = Arc::new(rchain_shared::log::NopLog);
        let consecutive_failures = Arc::new(AtomicU64::new(0));
        Proposer::new(
            get_seq,
            check_active,
            create_block,
            validate,
            effect,
            validator,
            log,
            consecutive_failures,
        )
    }

    fn block() -> BlockMessage {
        BlockMessage {
            version: 1,
            shard_id: "root".to_string(),
            block_hash: BlockHash::new([1u8; 32]),
            block_number: 0.try_into().unwrap(),
            sender: Validator::new([0u8; 65]),
            seq_num: 0.try_into().unwrap(),
            pre_state_hash: StateHash::new([0u8; 32]),
            post_state_hash: StateHash::new([0u8; 32]),
            justifications: vec![],
            bonds: std::collections::BTreeMap::new(),
            rejected_deploys: std::collections::BTreeSet::new(),
            rejected_blocks: std::collections::BTreeSet::new(),
            rejected_senders: std::collections::BTreeSet::new(),
            state: rchain_models::casper::protocol::casper_message::RholangState::default(),
            sig_algorithm: "secp256k1".to_string(),
            sig: vec![],
            timestamp: 0,
        }
    }

    #[tokio::test]
    async fn no_new_deploys_returns_failure() {
        let p = proposer(BlockCreatorResult::NoNewDeploys);
        let (tx, rx) = tokio::sync::oneshot::channel();
        let (result, block_opt) = p.propose(false, tx).await.unwrap();
        assert_eq!(result.propose_status, ProposeStatus::NoNewDeploys);
        assert!(block_opt.is_none());
        assert!(matches!(rx.await.unwrap(), ProposerResult::Failure { .. }));
    }

    #[tokio::test]
    async fn created_block_returns_success() {
        let p = proposer(BlockCreatorResult::Created(block()));
        let (tx, rx) = tokio::sync::oneshot::channel();
        let (result, block_opt) = p.propose(false, tx).await.unwrap();
        assert_eq!(result.propose_status, ProposeStatus::ProposeSuccess);
        assert!(block_opt.is_some());
        assert!(matches!(rx.await.unwrap(), ProposerResult::Success { .. }));
    }
}

/// Whether this validator should attest now: its own weight plus the stake already moving on the fringe
/// reaches a supermajority.
///
/// The stake that counts as "moving" toward the quorum: the **live** weight set minus ourselves.
///
/// The set is the shared liveness rule ([`liveness::live_weight_set`], #70 increment 2) — the same
/// function the finalizer's partition ranges over, so "who is speaking" cannot differ between the two
/// consumers. The exclusion is ours to make here: our own message is the attestation we are about to
/// add, and [`attestation_reaches_supermajority`] counts it on the other side of the comparison.
///
/// **What this replaced, and why it was a defect (F4).** The stake used to be summed over the senders
/// of `pre_state.justifications`, which is `latest_msgs` — a map that keeps a silent sender's last
/// message **indefinitely** — so "moving" meant *has ever spoken*. Three equal validators with one dead
/// summed 200 (the live peer's message *plus* the dead one's stale one) and, with `own_stake`, reached
/// 300 of 300: a supermajority that does not exist. Pinned by
/// `a_silent_validators_stale_message_does_not_carry_the_quorum` in `attestation_suppression_tests`.
///
/// **The denominator is not this set.** The quorum is measured against the whole bonded map
/// (`pre_state_bonds`), not the live one: a quorum over the live stake would let any self-consistent
/// subset finalise, and under a partition each side would finalise its own view — two conflicting
/// finalisations. The issue is that an absent validator must not *block* the partition; it is not that
/// an absent validator's stake stops counting. See `liveness`' module doc and the §6 row.
fn moving_attestation_stake(live_bonds: &BTreeMap<Validator, NonNegI64>, own: &Validator) -> i128 {
    live_bonds
        .iter()
        .filter(|(v, _)| *v != own)
        .map(|(_, s)| i128::from(i64::from(*s)))
        .sum()
}

/// Whether this node has been quiet long enough to speak again, read from its own latest message in
/// `justifications` — so the pace bound needs no new state, no counter, and no persistence.
fn cadence_due(justifications: &[BlockMetadata], own: &Validator, tip: BlockHeight) -> bool {
    match justifications.iter().find(|m| &m.sender == own) {
        Some(mine) => liveness::heights_behind(tip, mine.block_num) > liveness::LIVENESS_WINDOW,
        // Nothing from us in the DAG yet: there is no quiet to have broken.
        None => true,
    }
}

/// Whether this node withholds its attestation for the block it is building.
///
/// A pair of bounds, and the order matters:
///
/// - nothing to finalise → suppress. An idle chain must not grow.
/// - the quorum is reachable → attest now. The ordinary case, unchanged.
/// - the quorum is unreachable, but a state transition exists to attest to **and** this node has itself
///   been quiet for longer than `ATTESTATION_WINDOW` → attest anyway, at that reduced cadence.
/// - otherwise → suppress.
///
/// **This used to be `nothing_to_finalize || !(new_state_transition || quorum)`, and that was #70's
/// F3.** `new_state_transition` ("any parent carries deploys", the ordinary case on a chain with
/// traffic) was OR-ed *inside* the quorum test, so it short-circuited it to `false`; suppression then
/// collapsed to `nothing_to_finalize`, which was also `false` because the deploy-bearing block was
/// unfinalized. A node that had lost over a third of its stake attested at **every** height — the
/// 276-blocks-in-a-minute run on #70 — and the contract stated in the attestation-tap comment at
/// `node_runtime.rs` ("a chain that has lost over a third of its stake does not spin") was not what the
/// code did. `new_state_transition` still licenses an attestation; it no longer licenses an unbounded
/// rate of them. Pinned by
/// `an_unreachable_supermajority_suppresses_even_with_a_deploy_bearing_parent`, and the cadence half by
/// `but_a_node_quiet_past_the_window_speaks_again`.
fn attestation_suppressed(
    nothing_to_finalize: bool,
    new_state_transition: bool,
    quorum_reachable: bool,
    cadence_due: bool,
) -> bool {
    if nothing_to_finalize {
        return true;
    }
    if quorum_reachable {
        return false;
    }
    !(new_state_transition && cadence_due)
}

/// Counting our own weight is the point. Without it, a validator whose attestation is exactly what would
/// complete the quorum can never be the one to add it — it is always "waiting for a supermajority" that
/// only its own message could create. With it, four validators at equal stake attest as soon as one of
/// them has moved (25% + 25% … 75% > 2/3), while a lone 10% validator still waits (10% + 0 < 2/3), which
/// is the anti-spam property the original rule was protecting.
fn attestation_reaches_supermajority(
    moving_stake: i128,
    own_stake: i128,
    total_stake: i128,
) -> bool {
    is_super_majority(moving_stake + own_stake, total_stake)
}

#[cfg(test)]
mod attestation_guard_tests {
    use super::attestation_reaches_supermajority;

    #[test]
    fn a_validator_counts_its_own_weight_toward_the_quorum() {
        // Four validators at 100 each: one other has moved, we hold 100 -> 200 of 400 is not > 2/3.
        assert!(!attestation_reaches_supermajority(100, 100, 400));
        // Two others have moved (the deploy-holder plus one) -> 300 of 400 is > 2/3, so we attest.
        assert!(attestation_reaches_supermajority(200, 100, 400));
        // A single node holding everything attests on its own.
        assert!(attestation_reaches_supermajority(0, 100, 100));
        // A lone 10% validator on a net where nobody has moved still waits.
        assert!(!attestation_reaches_supermajority(0, 10, 100));
        assert!(!attestation_reaches_supermajority(10, 10, 100));
    }
}

/// The two defects in the attestation guard, pinned as tests that are **red against this tree** rather
/// than described in prose — F4 (`moving_attestation_stake` cannot see recency) and F3
/// (`attestation_suppressed`'s quorum clause is short-circuited by a deploy-bearing parent).
///
/// Both were found by the #70 design pass on 2026-09-29; the issue's own comments do not contain
/// either, and three of them are stale against this tree. See the plan at
/// `~/.claude/plans/from-open-issues-and-nested-eclipse.md`.
#[cfg(test)]
mod attestation_suppression_tests {
    use super::{
        attestation_reaches_supermajority, attestation_suppressed, cadence_due,
        moving_attestation_stake,
    };
    use rchain_block_storage::dag::liveness;
    use rchain_models::block_hash::BlockHash;
    use rchain_models::block_metadata::BlockMetadata;
    use rchain_models::validator::Validator;
    use rchain_shared::refined::{BlockHeight, NonNegI64, SeqNum};
    use std::collections::{BTreeMap, BTreeSet};

    fn validator(byte: u8) -> Validator {
        Validator::new([byte; 65])
    }

    /// A sender's latest message, at `block_num`. `latest_msgs` keeps one of these per sender
    /// indefinitely, so an old `block_num` is the whole of the staleness signal a guard could read.
    fn latest(sender: Validator, block_num: i64) -> BlockMetadata {
        BlockMetadata {
            block_hash: BlockHash::new([0u8; 32]),
            block_num: BlockHeight::try_from(block_num).unwrap(),
            sender,
            seq_num: SeqNum::zero(),
            justifications: BTreeSet::new(),
            bonds_map: BTreeMap::new(),
            validated: true,
            validation_failed: false,
            slashable: false,
            fringe: BTreeSet::new(),
            fringe_state_hash: rchain_models::block::state_hash::StateHash::new([0u8; 32]),
            member_of_fringe: None,
        }
    }

    fn bonds(entries: &[(Validator, i64)]) -> BTreeMap<Validator, NonNegI64> {
        entries
            .iter()
            .map(|(v, s)| (v.clone(), NonNegI64::try_from(*s).unwrap()))
            .collect()
    }

    /// **F4: a silent validator's stale message must not carry the quorum.**
    ///
    /// Three equal validators. `b` has gone silent, so its latest message sits 7 heights behind `c`'s —
    /// and both stay in `latest_msgs` forever. Only `c` is moving, so the quorum is
    /// `100 + our own 100 = 200` of 300, which is *exactly* 2/3 and therefore **not** a supermajority
    /// (`two_thirds_is_not_supermajority`, `sdk/src/consensus.rs:24`). Counting `b`'s stale message
    /// makes it 300 of 300 and has this node attest at every height — the spin #70 reports.
    ///
    /// Red before the recency filter — and still red if it is deleted from `live_weight_set`: the
    /// moving stake then reads 200, which `own_stake` turns into 300 of 300.
    #[test]
    fn a_silent_validators_stale_message_does_not_carry_the_quorum() {
        let (a, b, c) = (validator(1), validator(2), validator(3));
        let bonded = bonds(&[(a.clone(), 100), (b.clone(), 100), (c.clone(), 100)]);
        let justifications = [latest(b.clone(), 3), latest(c.clone(), 10)];
        let tip = BlockHeight::try_from(10).unwrap();

        let live = liveness::live_weight_set(
            &bonded,
            &liveness::latest_heights(
                justifications
                    .iter()
                    .map(|m| (m.sender.clone(), m.block_num)),
            ),
            tip,
            liveness::LIVENESS_WINDOW,
        );
        let moving = moving_attestation_stake(&live, &a);
        assert_eq!(
            moving, 100,
            "only `c` is moving — `b`'s message is 7 heights stale and must not count toward quorum"
        );
        assert!(
            !attestation_reaches_supermajority(moving, 100, 300),
            "200 of 300 is exactly 2/3, which is not a supermajority"
        );
    }

    /// **The quorum is measured against the whole bonded map, not the live set** — the difference
    /// between "an absent validator must not block the partition" and "an absent validator's stake
    /// stops counting", and the second is what would cost safety: a quorum over the live stake is
    /// trivially reached by any self-consistent subset, so under a partition each side would finalise
    /// its own view. Pinned as the arithmetic that separates the two readings.
    #[test]
    fn the_quorum_is_measured_against_the_whole_bonded_map_not_the_live_one() {
        // Three equal validators, one silent: the live set is 200, the bonded map 300.
        assert!(
            attestation_reaches_supermajority(100, 100, 200),
            "against the live set alone this is a quorum — which is the shape that loses safety"
        );
        assert!(
            !attestation_reaches_supermajority(100, 100, 300),
            "against the whole bonded map it is exactly 2/3, and 2/3 is not a supermajority"
        );
    }

    /// **F3: a deploy-bearing parent must not defeat the quorum clause.**
    ///
    /// Four validators at 100, one peer moving: `100 + our own 100 = 200` of 400, unreachable. A parent
    /// carrying deploys is the ordinary case on a chain with traffic, and it used to short-circuit the
    /// quorum test entirely, so this node attested at **every** height — the 276-blocks-in-a-minute
    /// run. This pins the contract stated in the attestation-tap comment at `node_runtime.rs`: *a chain
    /// that has lost over a third of its stake does not spin.*
    #[test]
    fn an_unreachable_supermajority_suppresses_even_with_a_deploy_bearing_parent() {
        assert!(
            attestation_suppressed(
                false, // nothing_to_finalize: there *is* an unfinalized transition to attest to
                true,  // new_state_transition: a parent carries deploys — the ordinary case
                false, // quorum_reachable: 200 of 400 is not a supermajority
                false, // cadence_due: we spoke within the window
            ),
            "the quorum is unreachable (200 of 400), so suppression must not be defeated by a \
             deploy-bearing parent"
        );
    }

    /// **The other half of F3, and the reason the repair is not a blanket suppress.**
    ///
    /// A hard suppress would trap liveness: no messages → no tip movement → no fresh justifications →
    /// the quorum stays unreachable forever, and a peer that came back is never seen. So a node quiet
    /// for longer than the window attests anyway, which bounds the spin to one block per window instead
    /// of one per height.
    #[test]
    fn but_a_node_quiet_past_the_window_speaks_again() {
        assert!(
            !attestation_suppressed(false, true, false, true),
            "a node that has been quiet past the window must attest even while the quorum is out of \
             reach, or a stalled chain can never discover that a peer returned"
        );
    }

    /// The ordinary case, unchanged: a reachable quorum attests immediately, whatever the cadence.
    #[test]
    fn a_reachable_quorum_attests_without_waiting_for_the_cadence() {
        assert!(!attestation_suppressed(false, true, true, false));
        assert!(!attestation_suppressed(false, false, true, false));
    }

    /// And an idle chain still produces nothing: suppression outranks every other term.
    #[test]
    fn an_idle_chain_suppresses_whatever_else_is_true() {
        assert!(attestation_suppressed(true, true, true, true));
    }

    /// The pace bound is read from the DAG, so it needs no new state: our own latest message's height
    /// against the tip. Nothing from us at all counts as due — there is no quiet to have broken.
    #[test]
    fn the_cadence_is_read_from_our_own_latest_message() {
        let (a, b) = (validator(1), validator(2));
        let tip = BlockHeight::try_from(20).unwrap();

        assert!(cadence_due(&[], &a, tip), "nothing from us: speak");
        assert!(
            !cadence_due(&[latest(a.clone(), 19)], &a, tip),
            "we spoke one height ago"
        );
        assert!(
            cadence_due(
                &[latest(a.clone(), 20 - liveness::LIVENESS_WINDOW - 1)],
                &a,
                tip
            ),
            "quiet past the window"
        );
        // Someone else's message is not ours, and must not reset our cadence.
        assert!(cadence_due(&[latest(b, 20)], &a, tip));
    }
}

/// The validators a proposer may slash: bonded, and the sender of a justification whose failure is
/// attributable to the block rather than to this node's inability to replay it.
///
/// `validation_failed` alone is too broad - it is also set when this node cannot run the replay at all,
/// which says nothing about the sender (#70). `BlockMetadata::slashable` is the narrower, attributable
/// signal.
///
/// **The rule itself is not defined here** (AUDIT C110). It is
/// [`crate::validate::slashable_senders`], and it is shared with the validators who *check* the slash
/// — which is the point: while the rule existed only on this side, a proposer could name any bonded
/// validator and the receiving nodes had nothing to compare the name against. What stays local to the
/// proposer is the `bonded` filter, because "only take stake from someone who has some" is a policy
/// about who is worth slashing, not part of what makes a slash justified.
fn slashable_offenders(
    justifications: &[rchain_models::block_metadata::BlockMetadata],
    bonded: &BTreeSet<Validator>,
) -> BTreeSet<Validator> {
    crate::validate::slashable_senders(justifications)
        .into_iter()
        .filter(|sender| bonded.contains(sender))
        .collect()
}

#[cfg(test)]
mod slashable_offenders_tests {
    use super::slashable_offenders;
    use rchain_models::block_hash::BlockHash;
    use rchain_models::block_metadata::BlockMetadata;
    use rchain_models::validator::Validator;
    use rchain_shared::refined::{BlockHeight, SeqNum};
    use std::collections::{BTreeMap, BTreeSet};

    fn meta(sender: Validator, validation_failed: bool, slashable: bool) -> BlockMetadata {
        BlockMetadata {
            block_hash: BlockHash::new([0u8; 32]),
            block_num: BlockHeight::try_from(1).unwrap(),
            sender,
            seq_num: SeqNum::zero(),
            justifications: BTreeSet::new(),
            bonds_map: BTreeMap::new(),
            validated: true,
            validation_failed,
            slashable,
            fringe: BTreeSet::new(),
            fringe_state_hash: rchain_models::block::state_hash::StateHash::new([0u8; 32]),
            member_of_fringe: None,
        }
    }

    fn rule(justifications: &[BlockMetadata], bonded: &[Validator]) -> BTreeSet<Validator> {
        slashable_offenders(justifications, &bonded.iter().copied().collect())
    }

    #[test]
    fn a_bonded_sender_of_a_disagreeing_block_is_slashable() {
        let v = Validator::new([1u8; 65]);
        assert_eq!(rule(&[meta(v, true, true)], &[v]), BTreeSet::from([v]));
    }

    /// The point of the narrowing: a block this node could not replay says nothing about its sender, so a
    /// local problem must not cost an honest validator its stake.
    #[test]
    fn a_replay_that_could_not_run_is_not_slashable() {
        let v = Validator::new([1u8; 65]);
        assert!(
            rule(&[meta(v, true, false)], &[v]).is_empty(),
            "an unattributable failure must not cost the sender its stake"
        );
    }

    #[test]
    fn only_bonded_senders_are_slashable() {
        let bonded = Validator::new([1u8; 65]);
        let observer = Validator::new([2u8; 65]);
        assert_eq!(
            rule(
                &[meta(bonded, true, true), meta(observer, true, true)],
                &[bonded]
            ),
            BTreeSet::from([bonded])
        );
    }
}

#[cfg(test)]
mod active_validator_tests {
    use super::*;
    use rchain_block_storage::dag::dag_storage::BlockDagStorage;
    use rchain_block_storage::dag::representation::DagRepresentation;
    use rchain_models::block_metadata::BlockMetadata;
    use rchain_models::validator::Validator;
    use rchain_shared::refined::{BlockHeight, NonNegI64, SeqNum};
    use std::collections::{BTreeMap, BTreeSet};

    /// A DAG with two blocks: the genesis at height 1, whose `bonds_map` is the validator set the chain
    /// started with, and a newer block at height 9, whose map includes a validator that has bonded since.
    /// An empty fringe, which is the state a chain is in while its fringe is stalled.
    struct TwoHeightDag {
        genesis: BlockHash,
        newest: BlockHash,
        newcomer: Validator,
    }

    fn meta(block_hash: BlockHash, bonds: BTreeMap<Validator, NonNegI64>) -> BlockMetadata {
        BlockMetadata {
            block_hash,
            block_num: BlockHeight::try_from(1).unwrap(),
            sender: Validator::new([9u8; 65]),
            seq_num: SeqNum::zero(),
            justifications: BTreeSet::new(),
            bonds_map: bonds,
            validated: true,
            validation_failed: false,
            slashable: false,
            fringe: BTreeSet::new(),
            fringe_state_hash: rchain_models::block::state_hash::StateHash::new([0u8; 32]),
            member_of_fringe: None,
        }
    }

    #[async_trait::async_trait]
    impl BlockDagStorage for TwoHeightDag {
        async fn get_representation(&self) -> Arc<DagRepresentation> {
            Arc::new(DagRepresentation {
                dag_set: Arc::new(BTreeSet::new()),
                child_map: Arc::new(BTreeMap::new()),
                height_map: Arc::new(BTreeMap::from([
                    (
                        BlockHeight::try_from(1).unwrap(),
                        BTreeSet::from([self.genesis]),
                    ),
                    (
                        BlockHeight::try_from(9).unwrap(),
                        BTreeSet::from([self.newest]),
                    ),
                ])),
                dag_message_state: rchain_block_storage::dag::message_state::DagMessageState::empty(
                ),
                fringe_states: BTreeMap::new(),
            })
        }

        async fn lookup(&self, block_hash: &BlockHash) -> Result<Option<BlockMetadata>, String> {
            if *block_hash == self.genesis {
                // The set the chain started with: the newcomer is not in it.
                Ok(Some(meta(self.genesis, BTreeMap::new())))
            } else if *block_hash == self.newest {
                Ok(Some(meta(
                    self.newest,
                    BTreeMap::from([(self.newcomer, NonNegI64::try_from(100).unwrap())]),
                )))
            } else {
                Ok(None)
            }
        }

        async fn insert(&self, _m: BlockMetadata, _b: BlockMessage) -> Result<(), String> {
            todo!("not on the is_active_validator path")
        }
        async fn lookup_by_deploy_id(&self, _d: &DeployId) -> Result<Option<BlockHash>, String> {
            todo!("not on the is_active_validator path")
        }
        async fn add_deploy(&self, _d: SignedDeployData) -> Result<(), String> {
            todo!("not on the is_active_validator path")
        }
        async fn pooled_deploys(&self) -> Result<BTreeMap<DeployId, SignedDeployData>, String> {
            todo!("not on the is_active_validator path")
        }
        async fn contains_deploy_in_pool(&self, _d: &DeployId) -> Result<bool, String> {
            todo!("not on the is_active_validator path")
        }
    }

    /// **A validator that bonded after the chain started is active, and may propose.**
    ///
    /// This is the whole of the "only the first validator ever proposes" symptom (#70). The check used to
    /// read the bond map of the *lowest* height in the DAG - the genesis's - so a validator admitted later
    /// was never active here, no matter what the chain's own active set said. Since the fringe cannot
    /// advance without the validators it expects to speak, the node stayed read-only forever, and the
    /// network could not actually grow its validator set even though every admission step succeeded.
    #[tokio::test]
    async fn a_validator_bonded_after_the_genesis_is_active() {
        let newcomer = Validator::new([1u8; 65]);
        let dag: Arc<dyn BlockDagStorage> = Arc::new(TwoHeightDag {
            genesis: BlockHash::new([0u8; 32]),
            newest: BlockHash::new([7u8; 32]),
            newcomer,
        });
        assert!(
            is_active_validator(&dag, &newcomer).await.unwrap(),
            "the newest block's view of the active set is what counts, not the genesis's"
        );
        let stranger = Validator::new([2u8; 65]);
        assert!(
            !is_active_validator(&dag, &stranger).await.unwrap(),
            "and a validator in no block's set is still not active"
        );
    }
}
