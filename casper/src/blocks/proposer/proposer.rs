//! Block proposer (port of `blocks/proposer/Proposer.scala`).
//!
//! `Proposer.apply` builds the dependency closures from the DAG/runtime; the `proposeEffect`
//! (broadcast via `CommUtil`) is supplied by the caller.

use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use rchain_block_storage::block_store::BlockStore;
use rchain_block_storage::dag::dag_storage::{BlockDagStorage, DeployId};
use rchain_block_storage::syntax::put_block;
use rchain_crypto::private_key::PrivateKey;
use rchain_crypto::signatures::secp256k1::Secp256k1;
use rchain_crypto::signatures::signed::Signed;
use rchain_models::block::state_hash::StateHash;
use rchain_models::block_hash::BlockHash;
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
    let fringe = dag_repr.dag_message_state.latest_fringe();
    let bonds_map = if let Some(m) = fringe.iter().next() {
        m.bonds_map.clone()
    } else if let Some((height, hashes)) = dag_repr.height_map.iter().next() {
        match hashes.iter().next() {
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
        }
    } else {
        Default::default()
    };
    Ok(bonds_map.contains_key(sender))
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
        Fut: Future<Output = Result<BlockIndex, String>> + Send + 'static,
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
            Arc::new(move |block: &BlockMessage| {
                let runtime = runtime.clone();
                let dag = dag.clone();
                let block_store = block_store.clone();
                let block_index = block_index.clone();
                let block = block.clone();
                let shard_id = shard_id.clone();
                Box::pin(async move {
                    match crate::multi_parent_casper::validate(
                        dag.as_ref(),
                        &block_store,
                        runtime.as_ref(),
                        &block,
                        &shard_id,
                        min_phlo_price,
                        block_index.as_ref(),
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
    Fut: Future<Output = Result<BlockIndex, String>>,
{
    let pre_state = get_pre_state_for_new_block(dag, block_store, runtime, block_index).await?;
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
    let offenders: BTreeSet<Validator> = pre_state
        .justifications
        .iter()
        .filter(|m| m.validation_failed)
        .map(|m| m.sender)
        .collect();

    let pre_state_bonds = runtime
        .compute_bonds(&StateHash::from_slice(pre_state_hash.as_bytes()))
        .await?;
    let bonded: BTreeSet<Validator> = pre_state_bonds
        .iter()
        .filter(|(_, b)| i64::from(**b) > 0)
        .map(|(v, _)| *v)
        .collect();
    let to_slash: BTreeSet<Validator> = offenders.intersection(&bonded).copied().collect();

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
    let new_state_transition = parents.iter().any(|b| has_deploys(b));
    let new_senders: BTreeSet<Validator> = pre_state
        .justifications
        .iter()
        .map(|m| m.sender.clone())
        .filter(|s| *s != creators_validator)
        .collect();
    let attestation_stake: i128 = pre_state_bonds
        .iter()
        .filter(|(v, _)| new_senders.contains(v))
        .map(|(_, s)| i128::from(i64::from(*s)))
        .sum();
    let pre_state_bonds_stake: i128 = pre_state_bonds
        .values()
        .map(|s| i128::from(i64::from(*s)))
        .sum();
    let own_stake: i128 = pre_state_bonds
        .iter()
        .filter(|(v, _)| **v == creators_validator)
        .map(|(_, s)| i128::from(i64::from(*s)))
        .sum();
    let waiting_for_supermajority = !(new_state_transition
        || attestation_reaches_supermajority(attestation_stake, own_stake, pre_state_bonds_stake));

    let suppress_attestation = nothing_to_finalize || waiting_for_supermajority;

    // User deploys: filter future / expired / replayed.
    let pooled = dag.pooled_deploys().await?;
    let mut deploys: Vec<DeployId> = Vec::new();
    for (id, d) in pooled {
        let future = d.data.valid_after_block_number > i64::from(next_block_num);
        let expired = d.data.valid_after_block_number < next_block_num - DEPLOY_LIFESPAN;
        let replay_attack = dag.lookup_by_deploy_id(&id).await?.is_some();
        if !(future || expired || replay_attack) {
            deploys.push(id);
        }
    }

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
