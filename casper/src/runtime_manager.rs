//! The runtime manager façade (port of `casper/rholang/RuntimeManager.scala`, read-only surface).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use rchain_crypto::hash::blake2b256_hash::Blake2b256Hash;
use rchain_crypto::hash::blake2b512_random::Blake2b512Random;
use rchain_crypto::public_key::PublicKey;
use rchain_models::ast::{Expr, Par, Var};
use rchain_models::block::state_hash::StateHash;
use rchain_models::casper::protocol::casper_message::{
    Event, PCost, ProcessedDeploy, ProcessedSystemDeploy, SignedDeployData, SystemDeployData,
};
use rchain_models::normalizer_env::NormalizerEnv;
use rchain_models::par_ops::from_expr;
use rchain_models::rholang::RhoType::RhoName;
use rchain_models::runtime::{BindPattern, ListParWithRandom, TaggedContinuation};
use rchain_models::sorted::SortedProc;
use rchain_models::types::count_free_vars;
use rchain_models::validator::Validator;
use rchain_rholang::accounting::Cost;
use rchain_rholang::evaluate_result::EvaluateResult;
use rchain_rholang::merging::{
    calculate_num_channel_diff, encode_mergeable_key, get_number_with_rnd, DeployMergeableData,
    NumberChannel,
};
use rchain_rholang::native_state::{decode_bonds, pos_bonds_key, NativeSystemState};
use rchain_rholang::runtime::{ReplayRhoRuntime, RhoRuntime};
use rchain_rholang::storage::{RhoHistoryRepository, RhoMatch};
use rchain_rholang::system_processes::BlockData;
use rchain_rspace::hot_store::InMemHotStore;
use rchain_rspace::merger::event_log_index::NumberChannelsDiff;
use rchain_rspace::native_store::PREFIX_POS;
use rchain_rspace::rspace::RSpace;
use rchain_shared::refined::NonNegI64;
use rchain_shared::typed_store::KeyValueTypedStore;

use crate::event_converter::to_casper_event;
use crate::genesis::contracts::Vault;
use crate::rholang::{ReplayFailure, SystemDeployRuntimeResult, UserDeployRuntimeResult};
use crate::runtime_replay::RuntimeReplayOps;
use crate::system_deploy::{
    process_bool_result, EvalCollector, NativeSystemDeployOp, SystemDeploy, SystemDeployUserError,
};

/// The runtime manager (port of `RuntimeManager`). Deploy execution (user deploys + system
/// deploys), genesis/state computation, replay, and bond/validator queries are implemented.
///
/// The mergeable-channel store (port of `RuntimeManager.MergeableStore`).
pub type MergeableStore = Arc<dyn KeyValueTypedStore<Vec<u8>, Vec<DeployMergeableData>>>;

/// The phlo (gas) limit for a single exploratory deploy (documented Scala deviation: Scala runs
/// exploratory deploys with no limit). Mirrors the Repl bound in
/// `node/src/api/grpc/repl_grpc_service.rs`.
const EXPLORATORY_PHLO_LIMIT: i64 = 1_000_000_000;
/// The wall-clock deadline for a single exploratory deploy (documented Scala deviation).
const EXPLORATORY_EVAL_TIMEOUT: Duration = Duration::from_secs(60);
/// The reduction-step budget for exploratory deploys. Lower than the reducer default so the legible
/// "reduction step budget exceeded" fires well inside [`EXPLORATORY_EVAL_TIMEOUT`] (measured ~460
/// steps/s at depth 16k, so 10k steps completes in ~22s worst case — issue #12). The wall clock is
/// the backstop, not the operative bound.
const EXPLORATORY_MAX_REDUCE_STEPS: i64 = 10_000;

pub struct RuntimeManager {
    runtime: RhoRuntime,
    replay_runtime: ReplayRhoRuntime,
    history_repo: RhoHistoryRepository,
    mergeable_store: MergeableStore,
}

impl RuntimeManager {
    pub fn new(
        runtime: RhoRuntime,
        replay_runtime: ReplayRhoRuntime,
        history_repo: RhoHistoryRepository,
        mergeable_store: MergeableStore,
    ) -> Self {
        RuntimeManager {
            runtime,
            replay_runtime,
            history_repo,
            mergeable_store,
        }
    }

    pub fn get_history_repo(&self) -> &RhoHistoryRepository {
        &self.history_repo
    }

    pub fn get_mergeable_store(&self) -> &MergeableStore {
        &self.mergeable_store
    }

    pub fn runtime(&self) -> &RhoRuntime {
        &self.runtime
    }

    pub fn replay_runtime(&self) -> &ReplayRhoRuntime {
        &self.replay_runtime
    }

    /// Fork a fresh replay runtime seeded at `root` via the read-only history fork, for parallel
    /// block validation (replay is verify-only; each block replays from its own pre-state root).
    pub async fn fork_replay_runtime(
        &self,
        root: Blake2b256Hash,
    ) -> Result<ReplayRhoRuntime, String> {
        let reader = self.history_repo.get_history_reader(root).await;
        let hot = Arc::new(InMemHotStore::new(reader.base()));
        let (_play, replay) =
            RSpace::create_with_replay(self.history_repo.clone(), hot, Arc::new(RhoMatch));
        ReplayRhoRuntime::create(
            Arc::new(replay),
            self.history_repo.clone(),
            SortedProc::default(),
        )
        .await
        .map_err(|e| e.to_string())
    }

    /// Fork a fresh, isolated play runtime seeded at `root` (the play counterpart of
    /// `fork_replay_runtime`). Read/exploration paths (data-at-name, explore-deploy) run on this so
    /// they never mutate the shared play runtime the proposer is concurrently using to create blocks.
    pub async fn fork_play_runtime(&self, root: Blake2b256Hash) -> Result<RhoRuntime, String> {
        let reader = self.history_repo.get_history_reader(root).await;
        let hot = Arc::new(InMemHotStore::new(reader.base()));
        let (play, _replay) =
            RSpace::create_with_replay(self.history_repo.clone(), hot, Arc::new(RhoMatch));
        RhoRuntime::create(play, self.history_repo.clone(), SortedProc::default())
            .await
            .map_err(|e| e.to_string())
    }

    /// Load mergeable channels from the store (port of `loadMergeableChannels`).
    pub async fn load_mergeable_channels(
        &self,
        state_hash: &[u8],
        creator: &[u8],
        seq_num: i64,
    ) -> Result<Vec<NumberChannelsDiff>, String> {
        let state_hash = Blake2b256Hash::from_byte_array(state_hash);
        let key = encode_mergeable_key(&state_hash, creator, seq_num);
        let vals = self.mergeable_store.get(&[key]).await?;
        let res = vals
            .into_iter()
            .next()
            .flatten()
            .ok_or_else(|| format!("Mergeable store invalid state hash {:?}.", state_hash))?;
        Ok(res
            .into_iter()
            .map(|d| d.channels.into_iter().map(|c| (c.hash, c.diff)).collect())
            .collect())
    }

    /// Convert final mergeable-channel values to diffs and persist them (port of
    /// `saveMergeableChannels`).
    pub async fn save_mergeable_channels(
        &self,
        post_state_hash: Blake2b256Hash,
        creator: &[u8],
        seq_num: i64,
        channels_data: &[NumberChannelsDiff],
        pre_state_hash: Blake2b256Hash,
    ) -> Result<(), String> {
        let diffs = self
            .convert_number_channels_to_diff(channels_data, pre_state_hash)
            .await?;
        let deploy_channels: Vec<DeployMergeableData> = diffs
            .into_iter()
            .map(|data| DeployMergeableData {
                channels: data
                    .into_iter()
                    .map(|(hash, diff)| NumberChannel { hash, diff })
                    .collect(),
            })
            .collect();
        let key = encode_mergeable_key(&post_state_hash, creator, seq_num);
        self.mergeable_store.put(&[(key, deploy_channels)]).await?;
        Ok(())
    }

    /// Convert final number-channel values to per-deploy diffs (port of
    /// `convertNumberChannelsToDiff`).
    async fn convert_number_channels_to_diff(
        &self,
        channels_data: &[NumberChannelsDiff],
        pre_state_hash: Blake2b256Hash,
    ) -> Result<Vec<NumberChannelsDiff>, String> {
        let history_reader = self.history_repo.get_history_reader(pre_state_hash).await;
        let mut keys: BTreeSet<Blake2b256Hash> = BTreeSet::new();
        for m in channels_data {
            keys.extend(m.keys().copied());
        }
        let mut init_values: BTreeMap<Blake2b256Hash, i64> = BTreeMap::new();
        for k in &keys {
            let data = history_reader
                .get_data(*k)
                .await
                .map_err(|e| e.to_string())?;
            let num = match data.first() {
                Some(d) => get_number_with_rnd(&d.a)?.0,
                None => 0,
            };
            init_values.insert(*k, num);
        }
        Ok(calculate_num_channel_diff(channels_data, &init_values))
    }

    /// Read the `Par`s at a channel in the state identified by `hash` (port of `getData`). Runs on a
    /// forked runtime so the read cannot reset the shared play runtime out from under the proposer.
    pub async fn get_data(&self, hash: &StateHash, channel: &Par) -> Result<Vec<Par>, String> {
        let runtime = self.fork_play_runtime(to_blake(hash)).await?;
        runtime.reset(to_blake(hash)).await.map_err(|e| e)?;
        runtime
            .get_data_par(&SortedProc::new(channel.clone()))
            .await
            .map_err(|e| e.to_string())
    }

    /// Read the `ParBody` continuations at `channels` in the state identified by `hash` (port of
    /// `getContinuation`). Runs on a forked runtime for the same reason as `get_data`.
    pub async fn get_continuation(
        &self,
        hash: &StateHash,
        channels: &[Par],
    ) -> Result<Vec<(Vec<BindPattern>, Par)>, String> {
        let runtime = self.fork_play_runtime(to_blake(hash)).await?;
        runtime.reset(to_blake(hash)).await.map_err(|e| e)?;
        let channels: Vec<SortedProc> = channels
            .iter()
            .map(|c| SortedProc::new(c.clone()))
            .collect();
        runtime
            .get_continuation_par(&channels)
            .await
            .map_err(|e| e.to_string())
    }

    /// Execute a single deploy, returning its `ProcessedDeploy` + evaluation result (port of
    /// `processDeploy`).
    pub async fn process_deploy(
        &self,
        deploy: &SignedDeployData,
        rand: &Blake2b512Random,
    ) -> Result<(ProcessedDeploy, EvaluateResult), String> {
        let fallback = self.runtime.create_soft_checkpoint().await;
        // Bind `rho:rchain:deployerId` (and `rho:rchain:deployId`) so the deploy's free URI names
        // resolve during normalization (port of `NormalizerEnv(deploy).toEnv`).
        let normalizer_env = NormalizerEnv::new(deploy);
        // Enforce the deploy's declared phlo budget (port of Scala `set(phloLimit)`). The cost
        // balance is otherwise a single i32::MAX pool shared across the runtime's lifetime; seeding
        // it here both caps this deploy at `phlo_limit` and resets it between deploys, so one deploy
        // can no longer drain the pool and brick every subsequent deploy until restart.
        self.runtime
            .cost()
            .set(rchain_rholang::accounting::Cost::new(
                deploy.data.phlo_limit,
                "deploy",
            ));
        let eval_result = self
            .runtime
            .evaluate_with_env(&deploy.data.term, normalizer_env.to_env(), rand)
            .await
            .map_err(|e| e.to_string())?;
        let checkpoint = self.runtime.create_soft_checkpoint().await;
        let succeeded = eval_result.errors.is_empty();
        // Surface the reducer's failure reason in the processed deploy (issue #15): a failed user
        // deploy previously recorded only `errored: true` + cost, leaving the deployer no message.
        // `system_deploy_error` is the per-deploy error field the block already carries; explore-deploy
        // returns this same error text directly.
        let system_deploy_error = if succeeded {
            None
        } else {
            eval_result.errors.first().map(|e| e.to_string())
        };
        let deploy_log: Vec<Event> = checkpoint.log.iter().map(to_casper_event).collect();
        let processed = ProcessedDeploy {
            deploy: deploy.clone(),
            cost: PCost {
                // `PCost.cost` is a protobuf `uint64`; a negative (over-charged) cost is an
                // accounting anomaly. Reject it rather than silently clamping to 0.
                cost: u64::try_from(eval_result.cost.value)
                    .map_err(|_| format!("deploy cost is negative: {}", eval_result.cost.value))?,
            },
            deploy_log,
            is_failed: !succeeded,
            system_deploy_error,
        };
        if !succeeded {
            self.runtime.revert_to_soft_checkpoint(fallback).await;
        }
        Ok((processed, eval_result))
    }

    /// Run deploys from `start_hash` and return the post-state hash + processed deploys (port of
    /// `playDeploys`).
    pub async fn play_deploys(
        &self,
        start_hash: &Blake2b256Hash,
        terms: &[SignedDeployData],
        rand: &Blake2b512Random,
    ) -> Result<(Blake2b256Hash, Vec<UserDeployRuntimeResult>), String> {
        self.runtime.reset(*start_hash).await.map_err(|e| e)?;
        let mut results = Vec::new();
        for (i, d) in terms.iter().enumerate() {
            // The user-deploy split index (1) matches `processDeployWithMergeableData`, so the
            // genesis play random agrees with the replay (`RuntimeReplayOps`).
            let r = rand
                .split_byte(u8::try_from(i).map_err(|e| e.to_string())?)
                .split_byte(1);
            let (processed, eval_result) = self
                .process_deploy(d, &r)
                .await
                .map_err(|e| format!("deploy #{i} failed: {e}"))?;
            results.push(UserDeployRuntimeResult {
                deploy: processed,
                mergeable: BTreeMap::new(),
                eval_result,
            });
        }
        let checkpoint = self.runtime.create_checkpoint().await.map_err(|e| e)?;
        Ok((checkpoint.root, results))
    }

    /// Execute a user deploy with the pre-charge/refund system deploys (port of
    /// `playDeployWithCostAccounting`).
    pub async fn play_deploy_with_cost_accounting(
        &self,
        deploy: &SignedDeployData,
        rand: &Blake2b512Random,
    ) -> Result<UserDeployRuntimeResult, String> {
        let mut collector = EvalCollector::default();

        let pre_charge = SystemDeploy::pre_charge(
            deploy
                .data
                .total_phlo_charge()
                .ok_or_else(|| "phlo charge overflow".to_string())?,
            &PublicKey::new(deploy.deployer.clone()),
            rand.split_byte(0),
        );
        let (pre_result, pre_eval) = self.eval_system_deploy(&pre_charge).await?;
        let pre_checkpoint = self.runtime.create_soft_checkpoint().await;
        collector = collector.add(
            &pre_checkpoint
                .log
                .iter()
                .map(to_casper_event)
                .collect::<Vec<_>>(),
            &pre_eval.mergeable,
        );

        if let Err(e) = pre_result {
            let failed = ProcessedDeploy {
                deploy: deploy.clone(),
                cost: PCost { cost: 0 },
                deploy_log: collector.event_log.clone(),
                is_failed: true,
                system_deploy_error: Some(e.0),
            };
            return Ok(UserDeployRuntimeResult {
                deploy: failed,
                mergeable: BTreeMap::new(),
                eval_result: EvaluateResult {
                    cost: rchain_rholang::accounting::Cost::new(0, "pre-charge"),
                    errors: Vec::new(),
                    mergeable: BTreeSet::new(),
                },
            });
        }

        let (mut processed, eval_result) = self.process_deploy(deploy, &rand.split_byte(1)).await?;
        collector = collector.add(&processed.deploy_log, &eval_result.mergeable);

        let refund = SystemDeploy::refund(processed.refund_amount(), rand.split_byte(2));
        let _ = self.eval_system_deploy(&refund).await?;

        processed.deploy_log = collector.event_log.clone();
        Ok(UserDeployRuntimeResult {
            deploy: processed,
            mergeable: BTreeMap::new(),
            eval_result,
        })
    }

    /// Run deploys with cost accounting from `start_hash` (port of `playDeploys` with
    /// `playDeployWithCostAccounting`).
    pub async fn play_deploys_with_cost_accounting(
        &self,
        start_hash: &Blake2b256Hash,
        terms: &[SignedDeployData],
        rand: &Blake2b512Random,
    ) -> Result<(Blake2b256Hash, Vec<UserDeployRuntimeResult>), String> {
        self.runtime.reset(*start_hash).await.map_err(|e| e)?;
        let mut results = Vec::new();
        for (i, d) in terms.iter().enumerate() {
            let r = rand.split_byte(u8::try_from(i).map_err(|e| e.to_string())?);
            results.push(self.play_deploy_with_cost_accounting(d, &r).await?);
        }
        let checkpoint = self.runtime.create_checkpoint().await.map_err(|e| e)?;
        Ok((checkpoint.root, results))
    }

    /// Compute the genesis state from deploys (port of `computeGenesis`).
    ///
    /// Rust-first: the PoS bonds and active-validator set are installed as native state (not derived
    /// from an interpreted `Pos.rhox`), and `terms` is the (possibly empty) list of pure-library
    /// deploys.
    pub async fn compute_genesis(
        &self,
        terms: &[SignedDeployData],
        rand: &Blake2b512Random,
        block_data: BlockData,
        bonds: &BTreeMap<Validator, NonNegI64>,
        vaults: &[Vault],
    ) -> Result<(Blake2b256Hash, Blake2b256Hash, Vec<UserDeployRuntimeResult>), String> {
        let creator = block_data.sender.bytes().to_vec();
        let seq_num = i64::from(block_data.seq_num);
        self.runtime.set_block_data(block_data);
        let pre_state_hash = self.runtime.empty_state_hash().await?;
        self.runtime.reset(pre_state_hash).await.map_err(|e| e)?;
        let mut results = Vec::new();
        for (i, d) in terms.iter().enumerate() {
            let r = rand
                .split_byte(u8::try_from(i).map_err(|e| e.to_string())?)
                .split_byte(1);
            let (processed, eval_result) = self
                .process_deploy(d, &r)
                .await
                .map_err(|e| format!("deploy #{i} failed: {e}"))?;
            results.push(UserDeployRuntimeResult {
                deploy: processed,
                mergeable: BTreeMap::new(),
                eval_result,
            });
        }
        // Install the native system-contract state before the final checkpoint so it is
        // content-addressed into the post-state hash.
        let native = NativeSystemState::new(self.runtime.native_store());
        native.set_bonds(bonds);
        // Seed the initial REV vault balances from the genesis wallets file so pre-charge can
        // deduct phlo (the native vault map is otherwise empty, and every deploy fails pre-charge).
        for vault in vaults {
            native.set_vault_balance(&vault.rev_address.to_base58(), vault.initial_balance);
        }
        let checkpoint = self.runtime.create_checkpoint().await.map_err(|e| e)?;
        let mergeable_chs: Vec<NumberChannelsDiff> =
            results.iter().map(|r| r.mergeable.clone()).collect();
        self.save_mergeable_channels(
            checkpoint.root,
            &creator,
            seq_num,
            &mergeable_chs,
            pre_state_hash,
        )
        .await?;
        Ok((pre_state_hash, checkpoint.root, results))
    }

    /// Run a system deploy's source (port of `evaluateSystemSource`).
    async fn evaluate_system_source(
        &self,
        deploy: &SystemDeploy,
    ) -> Result<EvaluateResult, String> {
        self.runtime
            .evaluate_with_env(deploy.source, &deploy.normalizer_env, &deploy.rand)
            .await
            .map_err(|e| e.to_string())
    }

    /// Consume the result produced on the system deploy's return channel (port of
    /// `consumeSystemResult`).
    async fn consume_system_result(
        &self,
        deploy: &SystemDeploy,
    ) -> Result<Option<(TaggedContinuation, Vec<ListParWithRandom>)>, String> {
        let patterns = vec![SortedProc::new(from_expr(Expr::EVar(Box::new(
            Var::FreeVar(0),
        ))))];
        let pattern = BindPattern {
            free_count: count_free_vars(patterns[0].as_par()),
            patterns,
            remainder: None,
        };
        self.runtime
            .consume_result(
                &[SortedProc::new(deploy.return_channel.clone())],
                &[pattern],
            )
            .await
            .map_err(|e| e.to_string())
    }

    /// Evaluate a system deploy and extract its result (port of `evalSystemDeploy`).
    pub async fn eval_system_deploy(
        &self,
        deploy: &SystemDeploy,
    ) -> Result<(Result<(), SystemDeployUserError>, EvaluateResult), String> {
        if let Some(op) = &deploy.op {
            return self.eval_native_system_deploy(op).await;
        }
        let eval_result = self.evaluate_system_source(deploy).await?;
        if !eval_result.errors.is_empty() {
            return Err(format!(
                "Unexpected system errors: {:?}",
                eval_result.errors
            ));
        }
        let consumed = self.consume_system_result(deploy).await?;
        match consumed {
            Some((_, data)) => match data.as_slice() {
                [single] if single.pars.len() == 1 => {
                    let result = process_bool_result(single.pars[0].as_par());
                    Ok((result, eval_result))
                }
                _ => Err("Unexpected system-deploy result".to_string()),
            },
            None => Err("Unable to consume results of system deploy".to_string()),
        }
    }

    /// Evaluate a native system deploy (rust-first replacement for the rholang PoS/registry sources).
    async fn eval_native_system_deploy(
        &self,
        op: &NativeSystemDeployOp,
    ) -> Result<(Result<(), SystemDeployUserError>, EvaluateResult), String> {
        let native = NativeSystemState::new(self.runtime.native_store());
        let result = match op {
            NativeSystemDeployOp::PreCharge { deployer, amount } => {
                native.pre_charge(deployer, *amount).await?
            }
            NativeSystemDeployOp::Refund { amount } => native.refund(*amount).await?,
            NativeSystemDeployOp::CloseBlock => native.close_block().await?,
            NativeSystemDeployOp::Slash { validator } => native.slash(validator).await?,
        };
        let eval_result = EvaluateResult {
            cost: rchain_rholang::accounting::Cost::new(0, "native-system-deploy"),
            errors: Vec::new(),
            mergeable: BTreeSet::new(),
        };
        Ok((result.map_err(SystemDeployUserError), eval_result))
    }

    /// Play a single block-level system deploy from `state_hash` (port of `playSystemDeploy`).
    pub async fn play_system_deploy(
        &self,
        state_hash: &Blake2b256Hash,
        deploy: &SystemDeploy,
    ) -> Result<(Blake2b256Hash, SystemDeployRuntimeResult), String> {
        self.runtime.reset(*state_hash).await.map_err(|e| e)?;
        let (result, _eval_result) = self.eval_system_deploy(deploy).await?;
        let checkpoint = self.runtime.create_soft_checkpoint().await;
        let event_list: Vec<Event> = checkpoint.log.iter().map(to_casper_event).collect();
        let final_hash = self.runtime.create_checkpoint().await.map_err(|e| e)?.root;
        match result {
            Ok(()) => {
                let system_deploy = match &deploy.op {
                    Some(NativeSystemDeployOp::CloseBlock) => SystemDeployData::CloseBlock,
                    Some(NativeSystemDeployOp::Slash { validator }) => {
                        SystemDeployData::Slash(*validator)
                    }
                    _ => SystemDeployData::Empty,
                };
                let processed = ProcessedSystemDeploy::Succeeded {
                    event_list,
                    system_deploy,
                };
                Ok((
                    final_hash,
                    SystemDeployRuntimeResult {
                        deploy: processed,
                        mergeable: BTreeMap::new(),
                    },
                ))
            }
            Err(e) => Err(format!("System deploy failed: {}", e.0)),
        }
    }

    /// Compute the post-state from user deploys + block-level system deploys (port of
    /// `computeState`).
    pub async fn compute_state(
        &self,
        start_hash: &Blake2b256Hash,
        terms: &[SignedDeployData],
        system_deploys: &[SystemDeploy],
        rand: &Blake2b512Random,
        block_data: BlockData,
    ) -> Result<
        (
            Blake2b256Hash,
            Vec<UserDeployRuntimeResult>,
            Vec<SystemDeployRuntimeResult>,
        ),
        String,
    > {
        let creator = block_data.sender.bytes().to_vec();
        let seq_num = i64::from(block_data.seq_num);
        self.runtime.set_block_data(block_data);
        let (mut state_hash, processed_deploys) = self
            .play_deploys_with_cost_accounting(start_hash, terms, rand)
            .await?;
        let mut processed_system_deploys = Vec::new();
        for sd in system_deploys {
            let (new_hash, processed) = self.play_system_deploy(&state_hash, sd).await?;
            state_hash = new_hash;
            processed_system_deploys.push(processed);
        }
        let mut mergeable_chs: Vec<NumberChannelsDiff> = Vec::new();
        mergeable_chs.extend(processed_deploys.iter().map(|r| r.mergeable.clone()));
        mergeable_chs.extend(processed_system_deploys.iter().map(|r| r.mergeable.clone()));
        self.save_mergeable_channels(state_hash, &creator, seq_num, &mergeable_chs, *start_hash)
            .await?;
        Ok((state_hash, processed_deploys, processed_system_deploys))
    }

    /// Replay processed deploys + system deploys and verify the replayed state hash + mergeable
    /// channels (port of `replayComputeState`).
    pub async fn replay_compute_state(
        &self,
        start_hash: &Blake2b256Hash,
        terms: &[ProcessedDeploy],
        system_deploys: &[ProcessedSystemDeploy],
        rand: &Blake2b512Random,
        block_data: BlockData,
        with_cost_accounting: bool,
        bonds: &BTreeMap<Validator, NonNegI64>,
        vaults: &[Vault],
    ) -> Result<(Blake2b256Hash, Vec<NumberChannelsDiff>), ReplayFailure> {
        self.replay_compute_state_with(
            &self.replay_runtime,
            start_hash,
            terms,
            system_deploys,
            rand,
            block_data,
            with_cost_accounting,
            bonds,
            vaults,
        )
        .await
    }

    /// Replay using an explicit replay runtime (the per-block fork for parallel validation). The
    /// replay mutates only `replay_runtime`; the mergeable-channel save uses the shared store.
    #[allow(clippy::too_many_arguments)]
    pub async fn replay_compute_state_with(
        &self,
        replay_runtime: &ReplayRhoRuntime,
        start_hash: &Blake2b256Hash,
        terms: &[ProcessedDeploy],
        system_deploys: &[ProcessedSystemDeploy],
        rand: &Blake2b512Random,
        block_data: BlockData,
        with_cost_accounting: bool,
        bonds: &BTreeMap<Validator, NonNegI64>,
        vaults: &[Vault],
    ) -> Result<(Blake2b256Hash, Vec<NumberChannelsDiff>), ReplayFailure> {
        let creator = block_data.sender.bytes().to_vec();
        let seq_num = i64::from(block_data.seq_num);
        let (state_hash, mergeable_chs) = RuntimeReplayOps::new(replay_runtime)
            .replay_compute_state(
                start_hash,
                rand,
                terms,
                system_deploys,
                block_data,
                with_cost_accounting,
                bonds,
                vaults,
            )
            .await?;
        self.save_mergeable_channels(state_hash, &creator, seq_num, &mergeable_chs, *start_hash)
            .await
            .map_err(ReplayFailure::internal_error)?;
        Ok((state_hash, mergeable_chs))
    }

    /// Run a read-only exploratory deploy and capture its result (port of `playExploratoryDeploy`).
    pub async fn play_exploratory_deploy(
        &self,
        term: &str,
        hash: &StateHash,
    ) -> Result<Vec<Par>, String> {
        let rand = Blake2b512Random::default_random();
        let mut return_rand = rand.copy();
        let return_channel = RhoName::apply_bytes(return_rand.next());
        self.capture_results(hash, term, &rand, &return_channel)
            .await
    }

    async fn capture_results(
        &self,
        start: &StateHash,
        term: &str,
        rand: &Blake2b512Random,
        return_channel: &Par,
    ) -> Result<Vec<Par>, String> {
        // Fork a fresh, isolated play runtime at `start`: exploration must never mutate the shared
        // runtime the proposer uses to create blocks (a concurrent explore-deploy would otherwise
        // reset/re-evaluate the shared space mid-block and corrupt the block's post-state hash).
        let runtime = self.fork_play_runtime(to_blake(start)).await?;
        runtime.reset(to_blake(start)).await.map_err(|e| e)?;
        // Bound the exploratory evaluation (documented Scala deviation): a phlo cap + a
        // reduction-step budget (the operative bound; the legible "step budget exceeded" error must
        // fire well inside the wall-clock deadline) + a wall-clock backstop.
        runtime
            .cost()
            .set(Cost::new(EXPLORATORY_PHLO_LIMIT, "exploratory"));
        runtime.set_max_reduce_steps(EXPLORATORY_MAX_REDUCE_STEPS);
        let eval = match tokio::time::timeout(
            EXPLORATORY_EVAL_TIMEOUT,
            runtime.evaluate(term, rand),
        )
        .await
        {
            Ok(eval) => eval.map_err(|e| e.to_string())?,
            Err(_elapsed) => {
                // Dropping the outer evaluation future does NOT stop the spawned continuation tasks
                // (dropping a `JoinHandle` never aborts its task). Signal the reducer cooperatively:
                // the next step check in the detached task tree fails and the whole tree unwinds,
                // releasing the forked runtime (issue #12).
                runtime.cancel_reduce();
                return Err(format!(
                    "exploratory deploy timed out after {EXPLORATORY_EVAL_TIMEOUT:?}"
                ));
            }
        };
        if !eval.errors.is_empty() {
            return Err(format!("{:?}", eval.errors));
        }
        runtime
            .get_data_par(&SortedProc::new(return_channel.clone()))
            .await
            .map_err(|e| e.to_string())
    }

    /// Query the current active validators at `hash` (native PoS read, port of `getActiveValidators`).
    pub async fn get_active_validators(&self, hash: &StateHash) -> Result<Vec<Validator>, String> {
        // Active validators = every bonded validator (derived from the bonds leaf).
        Ok(self.compute_bonds(hash).await?.into_keys().collect())
    }

    /// Query the current bonds at `hash` (native PoS read, port of `computeBonds`).
    pub async fn compute_bonds(
        &self,
        hash: &StateHash,
    ) -> Result<BTreeMap<Validator, NonNegI64>, String> {
        let reader = self.history_repo.get_history_reader(to_blake(hash)).await;
        let bytes = reader
            .get_native(PREFIX_POS, pos_bonds_key())
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "bonds leaf not found in state".to_string())?;
        decode_bonds(&bytes)
    }
}

fn to_blake(hash: &StateHash) -> Blake2b256Hash {
    Blake2b256Hash::from_byte_array(hash.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rchain_models::runtime::{BindPattern, ListParWithRandom, TaggedContinuation};
    use rchain_rholang::merging::DeployMergeableDataCodec;
    use rchain_rspace::factory::create_history_repository;
    use rchain_shared::store_manager::{database, InMemoryStoreManager};
    use rchain_shared::typed_store::BytesCodec;

    /// A forked replay runtime (used for parallel block validation) must be constructible at the
    /// empty root and able to replay an empty deploy set.
    #[tokio::test]
    async fn fork_replay_runtime_replays_empty() {
        let manager = InMemoryStoreManager::default();
        let history = create_history_repository::<
            SortedProc,
            BindPattern,
            ListParWithRandom,
            TaggedContinuation,
        >(&manager, "rspace")
        .await
        .unwrap();
        let empty_root = history.root();
        let reader = history.get_history_reader(empty_root).await;
        let hot = Arc::new(InMemHotStore::new(reader.base()));
        let (play, replay) = RSpace::create_with_replay(history.clone(), hot, Arc::new(RhoMatch));
        let rho = RhoRuntime::create(play, history.clone(), SortedProc::default())
            .await
            .unwrap();
        let replay =
            ReplayRhoRuntime::create(Arc::new(replay), history.clone(), SortedProc::default())
                .await
                .unwrap();
        let mergeable_store = Arc::new(
            database(
                &manager,
                "mergeable",
                Arc::new(BytesCodec),
                Arc::new(DeployMergeableDataCodec),
            )
            .await
            .unwrap(),
        );
        let runtime = RuntimeManager::new(rho, replay, history, mergeable_store);

        let forked = runtime.fork_replay_runtime(empty_root).await.unwrap();
        let result = runtime
            .replay_compute_state_with(
                &forked,
                &empty_root,
                &[],
                &[],
                &Blake2b512Random::new_random(128),
                BlockData::empty(),
                false,
                &BTreeMap::new(),
                &[],
            )
            .await;
        assert!(result.is_ok());
    }
}
