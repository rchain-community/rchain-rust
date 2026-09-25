//! Casper merge index data structures (Law 17: merge determinism).
//!
//! Ports the pure data types, conflict/dependency relations, and the effectful constructors
//! (`DeployChainIndex.apply`, `BlockIndex.apply`, `MergeScope.merge`) from `casper/.../merging/`.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};
use std::sync::{Mutex, OnceLock};

use rchain_block_storage::block_store::BlockStore;
use rchain_block_storage::dag::finalizer::Message;
use rchain_block_storage::dag::message_map;
use rchain_crypto::hash::blake2b256_hash::Blake2b256Hash;
use rchain_models::block_hash::BlockHash;
use rchain_models::block_metadata::BlockMetadata;
use rchain_models::casper::protocol::casper_message::{
    BlockMessage, Event, ProcessedDeploy, ProcessedSystemDeploy, SystemDeployData,
};
use rchain_models::fringe_data::FringeData;
use rchain_models::runtime::{BindPattern, ListParWithRandom, TaggedContinuation};
use rchain_models::sorted::SortedProc;
use rchain_models::validator::Validator;
use rchain_rholang::merging::{calculate_number_channel_merge, read_mergeable_values};
use rchain_rholang::storage::RhoHistoryRepository;
use rchain_rholang::system_processes::BlockData;
use rchain_rspace::history::history_repository::HistoryRepository;
use rchain_rspace::hot_store_trie_action::HotStoreTrieAction;
use rchain_rspace::merger::event_log_index::{EventLogIndex, NumberChannelsDiff};
use rchain_rspace::merger::event_log_merging_logic::{are_conflicting, depends};
use rchain_rspace::merger::state_change::StateChange;
use rchain_rspace::merger::state_change_merger::compute_trie_actions;
use rchain_rspace::native_store::NativeStoreAction;
use rchain_rspace::trace::event::{Event as REvent, Produce};
use rchain_sdk::dag::merging::{
    compute_dependency_map, compute_greedy_non_intersecting_branches,
    compute_relation_map_for_merge_set, resolve_conflict_set,
};
use rchain_shared::refined::NonNegI64;
use rchain_shared::serialize::Serialize;

use crate::block_random_seed::BlockRandomSeed;
use crate::event_converter::to_rspace_event;
use crate::interpreter_util::is_genesis_pre_state;
use crate::runtime_manager::RuntimeManager;

/// A deploy id paired with its execution cost (port of `DeployIdWithCost`).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DeployIdWithCost {
    pub id: Vec<u8>,
    pub cost: i64,
}

/// The index of a single deploy (port of `DeployIndex`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeployIndex {
    pub deploy_id: Vec<u8>,
    pub cost: i64,
    pub event_log_index: EventLogIndex,
}

impl Ord for DeployIndex {
    fn cmp(&self, other: &Self) -> Ordering {
        self.deploy_id.cmp(&other.deploy_id)
    }
}

impl PartialOrd for DeployIndex {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl DeployIndex {
    pub const SYS_SLASH_DEPLOY_COST: i64 = 0;
    pub const SYS_CLOSE_BLOCK_DEPLOY_COST: i64 = 0;
    pub const SYS_EMPTY_DEPLOY_COST: i64 = 0;

    pub fn sys_slash_deploy_id() -> Vec<u8> {
        vec![1]
    }
    pub fn sys_close_block_deploy_id() -> Vec<u8> {
        vec![2]
    }
    pub fn sys_empty_deploy_id() -> Vec<u8> {
        vec![3]
    }
}

/// The merged state seen by a block's parents (port of `ParentsMergedState`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParentsMergedState {
    pub justifications: Vec<BlockMetadata>,
    pub max_block_num: i64,
    pub max_seq_nums: BTreeMap<Validator, i64>,
    pub fringe: BTreeSet<BlockHash>,
    pub fringe_state: Blake2b256Hash,
    pub fringe_bonds_map: BTreeMap<Validator, NonNegI64>,
    pub fringe_rejected_deploys: BTreeSet<Vec<u8>>,
    pub pre_state_hash: Blake2b256Hash,
    pub rejected_deploys: BTreeSet<Vec<u8>>,
}

/// The index of deploys depending on each other within a single block (port of `DeployChainIndex`).
#[derive(Clone, Debug)]
pub struct DeployChainIndex {
    pub host_block: Blake2b256Hash,
    pub deploys_with_cost: BTreeSet<DeployIdWithCost>,
    pub pre_state_hash: Blake2b256Hash,
    pub post_state_hash: Blake2b256Hash,
    pub event_log_index: EventLogIndex,
    pub state_changes: StateChange,
}

// Equality/hash are over `deploysWithCost` only (the Scala override), to speed up rejection-option
// computation.
impl PartialEq for DeployChainIndex {
    fn eq(&self, other: &Self) -> bool {
        self.deploys_with_cost == other.deploys_with_cost
    }
}

impl Eq for DeployChainIndex {}

impl Hash for DeployChainIndex {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.deploys_with_cost.hash(state);
    }
}

// Ordering is over `(hostBlock, postStateHash)` (the Scala `Ordering.by`), distinct from equality
// (which is over `deploysWithCost`).
impl PartialOrd for DeployChainIndex {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for DeployChainIndex {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.host_block, self.post_state_hash).cmp(&(other.host_block, other.post_state_hash))
    }
}

impl DeployChainIndex {
    /// The total cost of the deploy chain (port of `deployChainCost`).
    pub fn deploy_chain_cost(r: &DeployChainIndex) -> i64 {
        r.deploys_with_cost.iter().map(|d| d.cost).sum()
    }

    /// Whether `a` depends on `b` (port of `depends`).
    pub fn depends(a: &DeployChainIndex, b: &DeployChainIndex) -> bool {
        depends(&a.event_log_index, &b.event_log_index)
    }

    /// Whether two sets of deploy chains conflict (port of `branchesAreConflicting`).
    ///
    /// Fallible because combining a set's indices can overflow the numeric-channel accumulation
    /// (AUDIT C41): the alternative was to answer `true` — "conflicting" — on an un-computable sum,
    /// which would be a silent semantic choice where the merge's own arithmetic returns an error.
    pub fn branches_are_conflicting(
        a: &BTreeSet<DeployChainIndex>,
        b: &BTreeSet<DeployChainIndex>,
    ) -> Result<bool, String> {
        let a_ids: BTreeSet<&Vec<u8>> = a
            .iter()
            .flat_map(|d| d.deploys_with_cost.iter().map(|x| &x.id))
            .collect();
        let b_ids: BTreeSet<&Vec<u8>> = b
            .iter()
            .flat_map(|d| d.deploys_with_cost.iter().map(|x| &x.id))
            .collect();
        let mut a_event = EventLogIndex::empty();
        for d in a {
            a_event = EventLogIndex::combine(&a_event, &d.event_log_index)?;
        }
        let mut b_event = EventLogIndex::empty();
        for d in b {
            b_event = EventLogIndex::combine(&b_event, &d.event_log_index)?;
        }
        Ok(!a_ids.is_disjoint(&b_ids) || are_conflicting(&a_event, &b_event))
    }

    /// Whether two deploy chains conflict (port of `deploysAreConflicting`).
    pub fn deploys_are_conflicting(a: &DeployChainIndex, b: &DeployChainIndex) -> bool {
        let a_ids: BTreeSet<&Vec<u8>> = a.deploys_with_cost.iter().map(|x| &x.id).collect();
        let b_ids: BTreeSet<&Vec<u8>> = b.deploys_with_cost.iter().map(|x| &x.id).collect();
        !a_ids.is_disjoint(&b_ids) || are_conflicting(&a.event_log_index, &b.event_log_index)
    }

    /// Build a deploy chain from its member deploys + pre/post state (port of
    /// `DeployChainIndex.apply`).
    pub async fn apply<C, P, A, K>(
        host_block: Blake2b256Hash,
        deploys: &BTreeSet<DeployIndex>,
        pre_state_hash: Blake2b256Hash,
        post_state_hash: Blake2b256Hash,
        history_repository: &HistoryRepository<C, P, A, K>,
    ) -> Result<DeployChainIndex, String>
    where
        C: Serialize<C> + Send + Sync + 'static,
        P: Serialize<P> + Send + Sync + 'static,
        A: Serialize<A> + Send + Sync + 'static,
        K: Serialize<K> + Send + Sync + 'static,
    {
        let deploys_with_cost: BTreeSet<DeployIdWithCost> = deploys
            .iter()
            .map(|d| DeployIdWithCost {
                id: d.deploy_id.clone(),
                cost: d.cost,
            })
            .collect();
        let mut event_log_index = EventLogIndex::empty();
        for d in deploys {
            event_log_index = EventLogIndex::combine(&event_log_index, &d.event_log_index)?;
        }

        let pre_reader = history_repository.get_history_reader(pre_state_hash).await;
        let pre_binary = pre_reader.reader_binary();
        let post_reader = history_repository.get_history_reader(post_state_hash).await;
        let post_binary = post_reader.reader_binary();

        let state_changes =
            StateChange::apply(pre_binary.as_ref(), post_binary.as_ref(), &event_log_index).await?;

        Ok(DeployChainIndex {
            host_block,
            deploys_with_cost,
            pre_state_hash,
            post_state_hash,
            event_log_index,
            state_changes,
        })
    }
}

/// The index of a block: its deploy chains (port of `BlockIndex`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockIndex {
    pub block_hash: BlockHash,
    pub deploy_chains: Vec<DeployChainIndex>,
    /// The native state mutations this block folded into its post-state (PoS pool/active/trusted/
    /// params, vault balances, registry, the HTTP oracle). Block-level because a hard checkpoint drains
    /// them per deploy-set, not per deploy. Carried so [`MergeScope::merge`] can re-apply them: the
    /// tuple-space `StateChange`s below reconstruct branch state from deploy effects, and native state
    /// has no such effect log, so without this a merge silently reverts every native write in the
    /// merged branches (issue #74).
    pub native_changes: Vec<NativeStoreAction>,
}

impl BlockIndex {
    /// Build an `EventLogIndex` from a deploy's events against the pre-state (port of
    /// `BlockIndex.createEventLogIndex`).
    pub async fn create_event_log_index<C, P, A, K>(
        events: &[Event],
        history_repository: &HistoryRepository<C, P, A, K>,
        pre_state_hash: Blake2b256Hash,
        mergeable_chs: NumberChannelsDiff,
    ) -> Result<EventLogIndex, String>
    where
        C: Serialize<C> + Send + Sync + 'static,
        P: Serialize<P> + Send + Sync + 'static,
        A: Serialize<A> + Send + Sync + 'static,
        K: Serialize<K> + Send + Sync + 'static,
    {
        let pre_reader = history_repository.get_history_reader(pre_state_hash).await;
        let rspace_events: Vec<REvent> = events.iter().map(to_rspace_event).collect();

        // Collect the distinct produces referenced by the trace to resolve the two pre-state
        // predicates (`produceExistsInPreState` and `produceTouchesPreStateJoin`).
        let mut produces: BTreeSet<Produce> = BTreeSet::new();
        for e in &rspace_events {
            match e {
                REvent::Produce(p) => {
                    produces.insert(p.clone());
                }
                REvent::Comm(c) => {
                    for p in &c.produces {
                        produces.insert(p.clone());
                    }
                }
                REvent::Consume(_) => {}
            }
        }

        let mut exists_in_pre_state: BTreeSet<Produce> = BTreeSet::new();
        let mut touches_pre_state_join: BTreeSet<Produce> = BTreeSet::new();
        for p in &produces {
            let data = pre_reader
                .get_data(p.channels_hash)
                .await
                .map_err(|e| e.to_string())?;
            if data.iter().any(|d| d.source == *p) {
                exists_in_pre_state.insert(p.clone());
            }
            let joins = pre_reader
                .get_joins(p.channels_hash)
                .await
                .map_err(|e| e.to_string())?;
            if joins.iter().any(|j| j.len() > 1) {
                touches_pre_state_join.insert(p.clone());
            }
        }

        Ok(EventLogIndex::apply(
            &rspace_events,
            |p| exists_in_pre_state.contains(p),
            |p| touches_pre_state_join.contains(p),
            mergeable_chs,
        ))
    }

    /// Build a `BlockIndex` from the processed deploys + mergeable-channel data (port of
    /// `BlockIndex.apply`).
    #[allow(clippy::too_many_arguments)]
    pub async fn apply<C, P, A, K>(
        block_hash: BlockHash,
        usr_processed_deploys: &[ProcessedDeploy],
        sys_processed_deploys: &[ProcessedSystemDeploy],
        pre_state_hash: Blake2b256Hash,
        post_state_hash: Blake2b256Hash,
        history_repository: &HistoryRepository<C, P, A, K>,
        mergeable_chan_data: &[NumberChannelsDiff],
        native_changes: Vec<NativeStoreAction>,
    ) -> Result<BlockIndex, String>
    where
        C: Serialize<C> + Send + Sync + 'static,
        P: Serialize<P> + Send + Sync + 'static,
        A: Serialize<A> + Send + Sync + 'static,
        K: Serialize<K> + Send + Sync + 'static,
    {
        let usr_count = usr_processed_deploys.len();
        let deploy_count = usr_count + sys_processed_deploys.len();
        let mrg_count = mergeable_chan_data.len();
        if deploy_count != mrg_count {
            return Err(format!(
                "Cache of mergeable channels ({mrg_count}) doesn't match deploys count ({deploy_count})."
            ));
        }

        let (usr_mergeable, sys_mergeable) = mergeable_chan_data.split_at(usr_count);

        let mut deploy_indices: BTreeSet<DeployIndex> = BTreeSet::new();

        // User deploy indices (failed deploys are skipped).
        for (d, merge_chs) in usr_processed_deploys.iter().zip(usr_mergeable.iter()) {
            if d.is_failed {
                continue;
            }
            let event_log_index = Self::create_event_log_index(
                &d.deploy_log,
                history_repository,
                pre_state_hash,
                merge_chs.clone(),
            )
            .await?;
            deploy_indices.insert(DeployIndex {
                deploy_id: d.deploy.sig.clone(),
                cost: i64::try_from(d.cost.cost).map_err(|e| e.to_string())?,
                event_log_index,
            });
        }

        // System deploy indices (only `Succeeded` blocks contribute).
        for (sd, merge_chs) in sys_processed_deploys.iter().zip(sys_mergeable.iter()) {
            let (id, log) = match sd {
                ProcessedSystemDeploy::Succeeded {
                    event_list,
                    system_deploy: SystemDeployData::Slash(_),
                } => (sys_deploy_id(&block_hash, 1), event_list),
                ProcessedSystemDeploy::Succeeded {
                    event_list,
                    system_deploy: SystemDeployData::CloseBlock,
                } => (sys_deploy_id(&block_hash, 2), event_list),
                ProcessedSystemDeploy::Succeeded {
                    event_list,
                    system_deploy: SystemDeployData::Empty,
                } => (sys_deploy_id(&block_hash, 3), event_list),
                ProcessedSystemDeploy::Failed { .. } => continue,
            };
            let event_log_index = Self::create_event_log_index(
                log,
                history_repository,
                pre_state_hash,
                merge_chs.clone(),
            )
            .await?;
            deploy_indices.insert(DeployIndex {
                deploy_id: id,
                cost: 0,
                event_log_index,
            });
        }

        // Deploys in a block execute sequentially, so there are only dependencies (no conflicts).
        let dependency_map = compute_dependency_map(&deploy_indices, &deploy_indices, |l, r| {
            depends(&l.event_log_index, &r.event_log_index)
        });
        let deploy_chains =
            compute_greedy_non_intersecting_branches(&deploy_indices, &dependency_map);

        let host_block = Blake2b256Hash::from_bytes(*block_hash.as_bytes());
        let mut chains = Vec::new();
        for chain in &deploy_chains {
            chains.push(
                DeployChainIndex::apply(
                    host_block,
                    chain,
                    pre_state_hash,
                    post_state_hash,
                    history_repository,
                )
                .await?,
            );
        }

        Ok(BlockIndex {
            block_hash,
            deploy_chains: chains,
            native_changes,
        })
    }
}

/// Build the system-deploy id as `blockHash ++ prefix` (port of `blockHash.concat(SYS_*_DEPLOY_ID)`).
fn sys_deploy_id(block_hash: &BlockHash, prefix: u8) -> Vec<u8> {
    let mut id = block_hash.as_bytes().to_vec();
    id.push(prefix);
    id
}

/// The in-memory block-index cache (port of `BlockIndex.cache`).
static BLOCK_INDEX_CACHE: OnceLock<Mutex<BTreeMap<BlockHash, BlockIndex>>> = OnceLock::new();

async fn get_block_unsafe(
    block_store: &BlockStore,
    hash: &BlockHash,
) -> Result<BlockMessage, String> {
    let mut vals = block_store.get(&[*hash]).await?;
    vals.pop()
        .flatten()
        .ok_or_else(|| format!("missing block {}", hash.to_hex()))
}

impl BlockIndex {
    /// Load (or compute + cache) a block index (port of `BlockIndex.getBlockIndex`).
    pub async fn get_block_index(
        runtime: &RuntimeManager,
        block_store: &BlockStore,
        block_hash: BlockHash,
    ) -> Result<BlockIndex, String> {
        let cache = BLOCK_INDEX_CACHE.get_or_init(|| Mutex::new(BTreeMap::new()));
        if let Some(idx) = cache
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&block_hash)
        {
            return Ok(idx.clone());
        }

        let block = get_block_unsafe(block_store, &block_hash).await?;
        let sender = block.sender.as_bytes().to_vec();
        let pre_state_hash = Blake2b256Hash::from_byte_array(block.pre_state_hash.as_bytes());
        let post_state_hash = Blake2b256Hash::from_byte_array(block.post_state_hash.as_bytes());
        let mergeable_result = runtime
            .load_mergeable_channels(
                post_state_hash.as_bytes(),
                &sender,
                i64::from(block.seq_num),
            )
            .await;
        let native_recorded = runtime
            .load_native_changes(
                post_state_hash.as_bytes(),
                &sender,
                i64::from(block.seq_num),
            )
            .await?;
        // Both sidecars are needed: mergeable channels to rebuild the tuple-space effects, native
        // changes to re-apply the effects the merge cannot see (issue #74). If either is missing -
        // an LFS-restored or deep-replayed block, or a block indexed before the native sidecar
        // existed - replay the block once to reproduce both, and persist them so the next lookup is
        // a read. A store error on the mergeable read is still a store error.
        let (mergeable_chs, native_changes) = match mergeable_result {
            Ok(channels) => match native_recorded {
                Some(native) => (channels, native),
                None => {
                    regenerate_sidecars(runtime, &block, &sender, pre_state_hash, post_state_hash)
                        .await?
                }
            },
            Err(err) if err.starts_with("Mergeable store invalid state hash") => {
                regenerate_sidecars(runtime, &block, &sender, pre_state_hash, post_state_hash)
                    .await?
            }
            Err(err) => return Err(err),
        };

        let index = BlockIndex::apply(
            block.block_hash,
            &block.state.deploys,
            &block.state.system_deploys,
            pre_state_hash,
            post_state_hash,
            runtime.get_history_repo(),
            &mergeable_chs,
            native_changes,
        )
        .await?;

        cache
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(block_hash, index.clone());
        Ok(index)
    }

    /// Remove cached block indices for `hashes` (port of `BlockIndex.cache.remove` in `pruneDiff`).
    ///
    /// The cache is advisory: a pruned entry is simply recomputed on the next `get_block_index`, so
    /// pruning is always safe (it never affects correctness, only recomputation cost). This bounds
    /// the otherwise-unbounded in-memory cache as the fringe finalizes.
    pub fn prune_cache(hashes: &[BlockHash]) {
        let Some(cache) = BLOCK_INDEX_CACHE.get() else {
            return;
        };
        let mut guard = cache.lock().unwrap_or_else(|p| p.into_inner());
        for h in hashes {
            guard.remove(h);
        }
    }
}

/// Reproduce and persist a block's mergeable-channel **and** native-changes sidecars by replaying the
/// block from its own pre-state.
///
/// Called when either sidecar is missing: an LFS-restored or deep-replayed block, or one indexed
/// before the native sidecar existed. The replay is the same one validation runs, so it reproduces
/// both effects exactly; `replay_compute_state_with` persists them, and the native changes are read
/// back here for the index. The genesis (an empty block) has no deploys to replay, so both sidecars
/// are recorded empty - its native state is installed by `compute_genesis` outside the block, and the
/// genesis is never a merge parent.
async fn regenerate_sidecars(
    runtime: &RuntimeManager,
    block: &BlockMessage,
    sender: &[u8],
    pre_state_hash: Blake2b256Hash,
    post_state_hash: Blake2b256Hash,
) -> Result<(Vec<NumberChannelsDiff>, Vec<NativeStoreAction>), String> {
    let seq_num = i64::from(block.seq_num);
    if block.justifications.is_empty()
        && block.state.deploys.is_empty()
        && block.state.system_deploys.is_empty()
    {
        runtime
            .save_mergeable_channels(post_state_hash, sender, seq_num, &[], pre_state_hash)
            .await?;
        runtime
            .save_native_changes(post_state_hash, sender, seq_num, &[])
            .await?;
        return Ok((Vec::new(), Vec::new()));
    }
    let forked = runtime.fork_replay_runtime(pre_state_hash).await?;
    let rand = BlockRandomSeed::random_generator_from_block(block);
    let with_cost_accounting = !block.justifications.is_empty();
    let (computed, channels) = runtime
        .replay_compute_state_with(
            &forked,
            &pre_state_hash,
            &block.state.deploys,
            &block.state.system_deploys,
            &rand,
            BlockData::from_block(block),
            with_cost_accounting,
            // Genesis PoS descriptors; consumed only on the genesis replay path
            // (`is_genesis_pre_state`). See `interpreter_util.rs`.
            runtime.genesis_pos(),
            // The genesis vault balances, and only for the genesis. The claim that stood here —
            // "this is always non-genesis block replay" — is what AUDIT C46 falsified on a
            // 3-validator devnet: the genesis is what the finalized fringe points at when a validator
            // joins, and that validator is the one whose indexing takes this branch. Its replay
            // computes a different post-state without the balances (they are installed natively,
            // outside the block's deploys) and then refuses block #0 forever.
            if is_genesis_pre_state(&pre_state_hash) {
                runtime.genesis_vaults()
            } else {
                &[]
            },
        )
        .await
        .map_err(|e| {
            format!(
                "failed to regenerate mergeable channels for block {}: {e:?}",
                block.block_hash.to_hex()
            )
        })?;
    if computed != post_state_hash {
        return Err(format!(
            "regenerated mergeable channels for block {} but replay computed {} instead of {}",
            block.block_hash.to_hex(),
            computed.to_hex(),
            post_state_hash.to_hex()
        ));
    }
    // `replay_compute_state_with` already persisted both sidecars; read the native changes back for
    // the index the caller is building.
    let native_changes = forked.last_native_changes();
    Ok((channels, native_changes))
}

/// The finalization decisions the final scope's blocks carry, by block. A block belongs to exactly
/// one fringe, so this is the whole of `merge`'s rejection lookup; a block absent from the map is
/// "not rejected" (`merge`'s own reading of an absent entry).
///
/// **Bounded by the question, not by the DAG.** This used to walk *every* fringe in `fringe_states`
/// and clone every `rejected_deploys` set into the map — on every merge, for a lookup that then
/// touched at most the final scope's blocks (AUDIT C56's owed paragraph). The map now holds a
/// *borrow* of each fringe's set and only for blocks the final scope asks about, so the cost is
/// O(final scope) set-membership probes and zero clones; with the whole-DAG shape restored the map
/// holds one entry per fringe member and `rejections_are_indexed_for_the_final_scope_only` fails.
fn rejections_for<'a>(
    fringe_states: &'a BTreeMap<Blake2b256Hash, FringeData>,
    final_hashes: &BTreeSet<BlockHash>,
) -> BTreeMap<BlockHash, &'a BTreeSet<Vec<u8>>> {
    fringe_states
        .values()
        .flat_map(|fd| {
            fd.fringe
                .iter()
                .filter(|h| final_hashes.contains(h))
                .map(move |h| (*h, &fd.rejected_deploys))
        })
        .collect()
}

/// The scope of a merge: final (immutable) and conflict (alterable) blocks (port of `MergeScope`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergeScope {
    pub final_scope: BTreeSet<BlockHash>,
    pub conflict_scope: BTreeSet<BlockHash>,
}

impl MergeScope {
    /// Create a merge scope from the DAG (port of `MergeScope.fromDag`).
    pub fn from_dag(
        merge_fringe: &BTreeSet<BlockHash>,
        final_fringe: &BTreeSet<BlockHash>,
        child_map: &BTreeMap<BlockHash, BTreeSet<BlockHash>>,
        dag_data: &BTreeMap<BlockHash, Message<BlockHash, Validator>>,
    ) -> Result<(MergeScope, Option<BlockHash>), String> {
        let prune_fringe: BTreeSet<BlockHash> =
            message_map::prune_fringe(dag_data, final_fringe, child_map)
                .iter()
                .map(|m| m.id)
                .collect();
        MergeScope::from_fringes(merge_fringe, final_fringe, &prune_fringe, dag_data)
    }

    /// Create a merge scope from explicit fringes (port of `MergeScope.fromFringes`).
    pub fn from_fringes(
        merge_fringe: &BTreeSet<BlockHash>,
        final_fringe: &BTreeSet<BlockHash>,
        prune_fringe: &BTreeSet<BlockHash>,
        dag_data: &BTreeMap<BlockHash, Message<BlockHash, Validator>>,
    ) -> Result<(MergeScope, Option<BlockHash>), String> {
        // Every fringe is checked against the DAG before use, as before — but the scope itself is
        // computed over ids, so no message is copied. A `Message` carries its `seen` set (Θ(N) for
        // a chain of N blocks), and cloning the operands and the result per block was Θ(N²) in
        // copies (AUDIT C56).
        let require_in_dag = |fringe: &BTreeSet<BlockHash>, what: &str| -> Result<(), String> {
            match fringe.iter().find(|h| !dag_data.contains_key(h)) {
                Some(h) => Err(format!("{what} not in dag: {}", h.to_hex())),
                None => Ok(()),
            }
        };
        require_in_dag(merge_fringe, "merge fringe")?;
        require_in_dag(final_fringe, "final fringe")?;
        require_in_dag(prune_fringe, "prune fringe")?;

        let c_scope_ids = message_map::between(dag_data, merge_fringe, final_fringe);
        let f_scope_ids = message_map::between(dag_data, final_fringe, prune_fringe);

        let base_msg = if f_scope_ids.is_empty() {
            let genesis = message_map::find_with_empty_parents(dag_data)
                .ok_or_else(|| "Final scope is empty but no genesis found.".to_string())?;
            Some(genesis.id)
        } else {
            None
        };
        let conflict_scope: BTreeSet<BlockHash> = c_scope_ids
            .difference(&base_msg.into_iter().collect())
            .copied()
            .collect();

        Ok((
            MergeScope {
                final_scope: f_scope_ids,
                conflict_scope,
            },
            base_msg,
        ))
    }

    /// Merge the conflict scope into the base state, returning the new state hash + rejected
    /// deploy ids (port of `MergeScope.merge`).
    pub async fn merge<F, Fut>(
        merge_scope: &MergeScope,
        base_state: Blake2b256Hash,
        fringe_states: &BTreeMap<Blake2b256Hash, FringeData>,
        history_repository: &RhoHistoryRepository,
        block_index: &F,
        rejection_cost: impl Fn(&DeployChainIndex) -> i64,
    ) -> Result<(Blake2b256Hash, BTreeSet<Vec<u8>>), String>
    where
        F: Fn(BlockHash) -> Fut,
        Fut: std::future::Future<Output = Result<BlockIndex, String>>,
    {
        let conflict_indices: Vec<BlockIndex> = {
            let mut v = Vec::new();
            for h in &merge_scope.conflict_scope {
                v.push(block_index(*h).await?);
            }
            v
        };
        let final_indices: Vec<BlockIndex> = {
            let mut v = Vec::new();
            for h in &merge_scope.final_scope {
                v.push(block_index(*h).await?);
            }
            v
        };

        // Native effects are block-level, so index them by host block here. The final scope's blocks
        // are ancestors of the base state (their effects are already in it); the conflict scope's are
        // what this merge applies, and the map below keeps only the ones with an accepted chain.
        let native_by_block: BTreeMap<Blake2b256Hash, Vec<NativeStoreAction>> = conflict_indices
            .iter()
            .map(|b| {
                (
                    Blake2b256Hash::from_bytes(*b.block_hash.as_bytes()),
                    b.native_changes.clone(),
                )
            })
            .collect();

        let conflict_set: BTreeSet<DeployChainIndex> = conflict_indices
            .iter()
            .flat_map(|b| b.deploy_chains.iter().cloned())
            .collect();
        let final_set: BTreeSet<DeployChainIndex> = final_indices
            .iter()
            .flat_map(|b| b.deploy_chains.iter().cloned())
            .collect();

        // Finalization decisions made in the final set. `rejections_map.get` treats absence as "not
        // rejected", so only the blocks the final scope actually asks about are indexed — this used
        // to walk *every* fringe and clone every `rejected_deploys` set into the map on every merge,
        // for a lookup that then touched at most the final scope's blocks (AUDIT C56's owed
        // paragraph). The values are borrowed: nothing is cloned at all.
        let rejections_map = rejections_for(
            fringe_states,
            &final_indices
                .iter()
                .map(|b| b.block_hash)
                .collect::<BTreeSet<BlockHash>>(),
        );
        let mut rejected_finally: BTreeSet<DeployChainIndex> = BTreeSet::new();
        let mut accepted_finally: BTreeSet<DeployChainIndex> = BTreeSet::new();
        for b in &final_indices {
            let rejected = rejections_map.get(&b.block_hash).copied();
            for chain in &b.deploy_chains {
                let first_id = chain.deploys_with_cost.iter().next().map(|d| d.id.clone());
                let is_rejected = match (rejected, &first_id) {
                    (Some(rej), Some(id)) => rej.contains(id),
                    _ => false,
                };
                if is_rejected {
                    rejected_finally.insert(chain.clone());
                } else {
                    accepted_finally.insert(chain.clone());
                }
            }
        }

        // Mergeable channels.
        let mergeable_diffs_map: BTreeMap<DeployChainIndex, NumberChannelsDiff> = conflict_set
            .iter()
            .map(|b| (b.clone(), b.event_log_index.number_channels_data.clone()))
            .collect();
        let mut all_channel_hashes: BTreeSet<Blake2b256Hash> = BTreeSet::new();
        for diffs in mergeable_diffs_map.values() {
            all_channel_hashes.extend(diffs.keys().copied());
        }
        let init_mergeable_values =
            read_mergeable_values(history_repository, base_state, &all_channel_hashes).await?;

        let (conflicts_map, dependency_map) = compute_relation_map_for_merge_set(
            &conflict_set,
            &final_set,
            DeployChainIndex::deploys_are_conflicting,
            DeployChainIndex::depends,
        );

        let (to_merge, rejected) = resolve_conflict_set(
            &conflict_set,
            &accepted_finally,
            &rejected_finally,
            rejection_cost,
            &conflicts_map,
            &dependency_map,
            &mergeable_diffs_map,
            &init_mergeable_values,
        );

        // The native effects of the blocks whose chains survive conflict resolution, in ascending
        // host-block order (a `BTreeSet` of `Blake2b256Hash`), and so deterministically across nodes.
        // A block contributes its native changes iff at least one of its chains is accepted - native
        // effects are per block, not per chain, so "some chain accepted" is the closest available
        // attribution, and rejecting a branch drops its native writes with its tuple-space ones. When
        // two accepted blocks write the same native key, the later one wins; in an honest DAG they do
        // not (a PoS membership change is applied by one branch), and making that a conflict is the
        // refinement recorded on the issue.
        let accepted_hosts: BTreeSet<Blake2b256Hash> =
            to_merge.iter().map(|c| c.host_block).collect();
        let mut native_changes: Vec<NativeStoreAction> = Vec::new();
        for host in &accepted_hosts {
            if let Some(actions) = native_by_block.get(host) {
                native_changes.extend(actions.iter().cloned());
            }
        }

        let new_state = MergeScope::compute_merged_state(
            &to_merge,
            base_state,
            history_repository,
            &native_changes,
        )
        .await?;
        let rejected_ids: BTreeSet<Vec<u8>> = rejected
            .iter()
            .flat_map(|d| d.deploys_with_cost.iter().map(|x| x.id.clone()))
            .collect();
        Ok((new_state, rejected_ids))
    }

    /// Merge a set of deploy chains into the base state and produce the new state hash (port of
    /// `MergeScope.computeMergedState`).
    ///
    /// `native_changes` are the native state mutations of the blocks contributing to `to_merge`.
    /// They are folded into the same checkpoint as the tuple-space changes: the `StateChange`s
    /// reconstruct only tuple space, so without them a merge would revert every native write the
    /// merged branches made (issue #74).
    pub async fn compute_merged_state(
        to_merge: &BTreeSet<DeployChainIndex>,
        base_state: Blake2b256Hash,
        history_repository: &RhoHistoryRepository,
        native_changes: &[NativeStoreAction],
    ) -> Result<Blake2b256Hash, String> {
        let history_reader = history_repository.get_history_reader(base_state).await;
        let base_reader = history_reader.reader_binary();

        // Combine all state changes + mergeable diffs.
        let all_changes = to_merge.iter().fold(StateChange::empty(), |acc, b| {
            StateChange::combine(&acc, &b.state_changes)
        });
        let mut mergeable_diffs: NumberChannelsDiff = BTreeMap::new();
        for b in to_merge {
            for (k, v) in &b.event_log_index.number_channels_data {
                let entry = mergeable_diffs.entry(*k).or_insert(0);
                let sum = entry.checked_add(*v).ok_or_else(|| {
                    format!(
                        "number channel diff accumulation overflow: {entry} + {v} does not fit i64"
                    )
                })?;
                *entry = sum;
            }
        }

        // Pre-compute the mergeable-channel override actions.
        let mut overrides: BTreeMap<
            Blake2b256Hash,
            HotStoreTrieAction<SortedProc, BindPattern, ListParWithRandom, TaggedContinuation>,
        > = BTreeMap::new();
        for (hash, diff) in &mergeable_diffs {
            let changes = all_changes
                .datums_changes
                .get(hash)
                .cloned()
                .unwrap_or_default();
            let action =
                calculate_number_channel_merge(*hash, *diff, &changes, history_reader.as_ref())
                    .await?;
            overrides.insert(*hash, action);
        }

        let trie_actions = compute_trie_actions(
            &all_changes,
            base_reader.as_ref(),
            mergeable_diffs.clone(),
            |hash, _changes, _chs| overrides.get(hash).cloned(),
        )
        .await?;

        let reset_repo = history_repository
            .reset(base_state)
            .await
            .map_err(|e| e.to_string())?;
        let new_repo = reset_repo
            .do_checkpoint_with_native(&trie_actions, native_changes)
            .await
            .map_err(|e| e.to_string())?;
        Ok(new_repo.root())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn chain(id: u8, cost: i64) -> DeployChainIndex {
        DeployChainIndex {
            host_block: Blake2b256Hash::from_bytes([id; 32]),
            deploys_with_cost: BTreeSet::from([DeployIdWithCost { id: vec![id], cost }]),
            pre_state_hash: Blake2b256Hash::from_bytes([0u8; 32]),
            post_state_hash: Blake2b256Hash::from_bytes([0u8; 32]),
            event_log_index: EventLogIndex::empty(),
            state_changes: StateChange::empty(),
        }
    }

    #[test]
    fn deploy_chain_cost_sums() {
        let a = chain(1, 5);
        let b = chain(2, 7);
        assert_eq!(DeployChainIndex::deploy_chain_cost(&a), 5);
        assert_eq!(DeployChainIndex::deploy_chain_cost(&b), 7);
    }

    #[test]
    fn equality_is_by_deploys_with_cost() {
        let a = chain(1, 5);
        let mut b = chain(1, 5);
        // Different host block — equality is over deploysWithCost only.
        b.host_block = Blake2b256Hash::from_bytes([9; 32]);
        assert_eq!(a, b);
        // Different cost — not equal.
        let c = chain(1, 6);
        assert_ne!(a, c);
    }

    #[test]
    fn deploys_are_conflicting_by_shared_id() {
        let a = chain(1, 5);
        let mut b = chain(1, 5);
        // Same deploy id -> conflicting (shared id), regardless of event logs.
        b.host_block = Blake2b256Hash::from_bytes([9; 32]);
        assert!(DeployChainIndex::deploys_are_conflicting(&a, &b));
        // Different ids, empty event logs -> not conflicting.
        assert!(!DeployChainIndex::deploys_are_conflicting(
            &chain(1, 5),
            &chain(2, 5)
        ));
    }

    fn msg(id: u8, parents: &[u8], seen: &[u8]) -> Message<BlockHash, Validator> {
        let hash = |b: u8| BlockHash::new([b; 32]);
        Message {
            id: hash(id),
            height: rchain_shared::refined::BlockHeight::zero(),
            sender: Validator::new([id; 65]),
            sender_seq: rchain_shared::refined::SeqNum::zero(),
            bonds_map: BTreeMap::new(),
            parents: parents.iter().map(|&b| hash(b)).collect(),
            fringe: BTreeSet::new(),
            seen: Arc::new(seen.iter().map(|&b| hash(b)).collect()),
        }
    }

    #[test]
    fn merge_scope_genesis_returns_genesis_as_base() {
        let g = msg(1, &[], &[1]);
        let c = msg(2, &[1], &[1, 2]);
        let dag = BTreeMap::from([(g.id, g.clone()), (c.id, c.clone())]);

        let (scope, base) = MergeScope::from_fringes(
            &[c.id].into_iter().collect(),
            &BTreeSet::new(),
            &BTreeSet::new(),
            &dag,
        )
        .unwrap();
        // Final scope empty -> genesis is the base; conflict scope excludes it.
        assert!(scope.final_scope.is_empty());
        assert_eq!(scope.conflict_scope, [c.id].into_iter().collect());
        assert_eq!(base, Some(g.id));
    }

    /// A non-empty final fringe: the final scope is what the final fringe has *seen* (a block's
    /// `seen` set includes itself, so the fringe block is in it), there is no base block to return,
    /// and the conflict scope is the merge fringe's own ancestors minus the final scope — here
    /// empty, because the merge fringe is the final fringe.
    #[test]
    fn merge_scope_with_a_final_fringe_has_no_base() {
        let g = msg(1, &[], &[1]);
        let c = msg(2, &[1], &[1, 2]);
        let dag = BTreeMap::from([(g.id, g.clone()), (c.id, c.clone())]);

        let (scope, base) = MergeScope::from_fringes(
            &[c.id].into_iter().collect(),
            &[c.id].into_iter().collect(),
            &BTreeSet::new(),
            &dag,
        )
        .expect("a well-formed dag");

        assert_eq!(
            scope.final_scope,
            [g.id, c.id].into_iter().collect::<BTreeSet<_>>(),
            "the final scope is what the final fringe has seen"
        );
        assert!(
            scope.conflict_scope.is_empty(),
            "the merge fringe is the final fringe, so nothing is left to merge"
        );
        assert_eq!(base, None, "a non-empty final scope has no base block");
    }

    /// A merge fringe that has *seen* a block the final fringe has not: that block is the conflict
    /// scope, because it is what merging has to reconcile.
    #[test]
    fn merge_scope_conflicts_are_the_fringe_blocks_the_final_fringe_has_not_seen() {
        // g <- a <- b, with `a` seen only by `b`; the final fringe is `a`, the merge fringe is `b`.
        let g = msg(1, &[], &[1]);
        let a = msg(2, &[1], &[1, 2]);
        let b = msg(3, &[2], &[1, 2, 3]);
        let dag = BTreeMap::from([(g.id, g.clone()), (a.id, a.clone()), (b.id, b.clone())]);

        let (scope, base) = MergeScope::from_fringes(
            &[b.id].into_iter().collect(),
            &[a.id].into_iter().collect(),
            &BTreeSet::new(),
            &dag,
        )
        .expect("a well-formed dag");

        assert_eq!(
            scope.final_scope,
            [g.id, a.id].into_iter().collect::<BTreeSet<_>>()
        );
        assert_eq!(
            scope.conflict_scope,
            [b.id].into_iter().collect::<BTreeSet<_>>(),
            "b is seen by the merge fringe only, so it is what must be merged"
        );
        assert_eq!(base, None);
    }

    /// A fringe hash that is **not in the DAG** is an error naming which fringe and which hash —
    /// never a silent omission, which would merge a scope that is quietly missing a branch.
    #[test]
    fn a_fringe_hash_absent_from_the_dag_is_an_error_naming_the_fringe() {
        let g = msg(1, &[], &[1]);
        let c = msg(2, &[1], &[1, 2]);
        let dag = BTreeMap::from([(g.id, g.clone()), (c.id, c.clone())]);
        let missing = BlockHash::new([9u8; 32]);

        let err = MergeScope::from_fringes(
            &[missing].into_iter().collect(),
            &BTreeSet::new(),
            &BTreeSet::new(),
            &dag,
        )
        .expect_err("the merge fringe is not in the dag");
        assert!(
            err.starts_with("merge fringe not in dag: "),
            "the error names the fringe: {err}"
        );
        assert!(err.contains(&missing.to_hex()), "{err}");

        let err = MergeScope::from_fringes(
            &BTreeSet::new(),
            &[missing].into_iter().collect(),
            &BTreeSet::new(),
            &dag,
        )
        .expect_err("the final fringe is not in the dag");
        assert!(err.starts_with("final fringe not in dag: "), "{err}");

        let err = MergeScope::from_fringes(
            &BTreeSet::new(),
            &BTreeSet::new(),
            &[missing].into_iter().collect(),
            &dag,
        )
        .expect_err("the prune fringe is not in the dag");
        assert!(err.starts_with("prune fringe not in dag: "), "{err}");
    }

    /// `prune_cache` is advisory: with nothing cached it returns without touching anything, and the
    /// hashes it is given are simply not there. The interesting property is that it never panics on
    /// a hashes-not-present input — it runs on the finalization path, where a panic would stop the
    /// node.
    #[test]
    fn pruning_a_cache_that_holds_nothing_is_a_no_op() {
        BlockIndex::prune_cache(&[]);
        BlockIndex::prune_cache(&[BlockHash::new([1u8; 32]), BlockHash::new([2u8; 32])]);
    }

    fn block_hash(id: u16) -> BlockHash {
        let mut bytes = [0u8; 32];
        bytes[..2].copy_from_slice(&id.to_le_bytes());
        BlockHash::new(bytes)
    }

    fn fringe_data(state: u8, blocks: impl Iterator<Item = u16>) -> FringeData {
        let fringe: BTreeSet<BlockHash> = blocks.map(block_hash).collect();
        FringeData {
            fringe_hash: FringeData::fringe_hash_of(&fringe),
            fringe,
            fringe_diff: BTreeSet::new(),
            state_hash: Blake2b256Hash::from_bytes([state; 32]),
            rejected_deploys: [vec![state]].into_iter().collect(),
            rejected_blocks: BTreeSet::new(),
            rejected_senders: BTreeSet::new(),
        }
    }

    /// AUDIT C56's owed paragraph, second part — the merge indexes rejections for the **final
    /// scope**, not for every fringe in the DAG.
    ///
    /// `MergeScope::merge` built `rejections_map` by walking every fringe in `fringe_states` and
    /// cloning each `rejected_deploys` set into it — 50 fringes × 10 blocks, so 500 clones per merge
    /// here — and then read the map only for the final scope's blocks, of which there are three
    /// (`rejections_map.get` has always treated absence as "not rejected"). The map is now built
    /// from the final scope's own hashes and holds a *borrow* of each fringe's set.
    ///
    /// **Falsified against this tree (2026-09-24)**: with the whole-DAG shape restored inside
    /// `rejections_for` — `for fd in fringe_states.values() { for h in &fd.fringe { insert } }` —
    /// the map holds 500 entries and this fails on its first assertion. The second assertion is the
    /// one that keeps the bound honest: the three entries must be the *right* fringes' rejections,
    /// so the size cannot be met by indexing the wrong blocks.
    #[test]
    fn rejections_are_indexed_for_the_final_scope_only() {
        let mut fringes: BTreeMap<Blake2b256Hash, FringeData> = BTreeMap::new();
        for f in 0..50u16 {
            let fd = fringe_data(f as u8, (0..10u16).map(|b| f * 10 + b));
            fringes.insert(fd.fringe_hash, fd);
        }
        // The final scope: the last fringe's first three blocks.
        let final_hashes: BTreeSet<BlockHash> =
            [490u16, 491, 492].into_iter().map(block_hash).collect();

        let map = rejections_for(&fringes, &final_hashes);
        assert_eq!(
            map.len(),
            3,
            "one entry per final-scope block, not one per fringe member"
        );
        let last_fringe = fringe_data(49, (490..500u16).map(|b| b));
        for h in &final_hashes {
            let rejected = map.get(h).expect("a final-scope block in a fringe");
            assert_eq!(
                *rejected, &fringes[&last_fringe.fringe_hash].rejected_deploys,
                "the entry must be its own fringe's rejections"
            );
        }
    }
}

#[cfg(test)]
mod merge_relation_tests {
    use super::*;

    use rchain_rspace::trace::event::Produce as RProduce;

    fn hash(byte: u8) -> Blake2b256Hash {
        Blake2b256Hash::from_bytes([byte; 32])
    }

    fn deploy_id(byte: u8, cost: i64) -> DeployIdWithCost {
        DeployIdWithCost {
            id: vec![byte],
            cost,
        }
    }

    /// A deploy index whose event log touches one produce (so conflicts/dependencies can be built).
    fn deploy_index(id: Vec<u8>, cost: i64, channel: u8) -> DeployIndex {
        let produce = RProduce::apply(&format!("chan{channel}"), &format!("datum{channel}"), false);
        DeployIndex {
            deploy_id: id,
            cost,
            event_log_index: EventLogIndex {
                produces_linear: [produce.clone()].into_iter().collect(),
                ..Default::default()
            },
        }
    }

    fn chain(host: u8, post: u8, deploys: &[(u8, i64, u8)]) -> DeployChainIndex {
        DeployChainIndex {
            host_block: hash(host),
            deploys_with_cost: deploys
                .iter()
                .map(|(id, cost, _)| deploy_id(*id, *cost))
                .collect(),
            pre_state_hash: hash(0),
            post_state_hash: hash(post),
            event_log_index: deploys
                .iter()
                .fold(EventLogIndex::empty(), |acc, (_, _, ch)| {
                    // A test chain's diffs are small; an error here would be a test bug, not a merge path.
                    EventLogIndex::combine(&acc, &deploy_index(vec![1], 0, *ch).event_log_index)
                        .expect("a test chain's accumulation cannot overflow")
                }),
            state_changes: StateChange::empty(),
        }
    }

    /// `DeployIndex` orders by **deploy id**, not cost: the rejection-option search sorts deploys by
    /// id, so two deploys with the same cost are distinguished only by their id.
    #[test]
    fn a_deploy_index_orders_by_id() {
        let a = deploy_index(vec![1], 100, 1);
        let b = deploy_index(vec![2], 1, 2);
        assert!(a < b, "id 1 sorts before id 2 regardless of cost");
        assert_eq!(
            a.deploy_id,
            vec![1],
            "the ordering key is the id field itself"
        );
        assert_eq!(DeployIndex::sys_slash_deploy_id(), vec![1]);
        assert_eq!(DeployIndex::sys_close_block_deploy_id(), vec![2]);
        assert_eq!(DeployIndex::sys_empty_deploy_id(), vec![3]);
        assert_eq!(DeployIndex::SYS_SLASH_DEPLOY_COST, 0);
    }

    /// **`DeployChainIndex`'s equality and ordering are over *different* fields** — equality over the
    /// deploy set (the Scala override, to speed up rejection-option computation), ordering over
    /// `(host_block, post_state_hash)`. That mismatch is faithful to the Scala, and it is a real
    /// hazard for a Rust `BTreeSet<DeployChainIndex>` (which uses `Ord`), because `Ord` is supposed to
    /// agree with `Eq`: a set can then hold two members that compare unequal yet `==` each other.
    /// Pinned here so the mismatch is visible rather than latent.
    #[test]
    fn chain_equality_is_over_the_deploys_and_ordering_over_the_hashes() {
        let same_deploys_different_hashes = chain(1, 2, &[(1, 10, 1)]);
        let mut other = chain(9, 8, &[(1, 10, 1)]);
        other.pre_state_hash = hash(7);

        assert_eq!(
            same_deploys_different_hashes, other,
            "equality is over the deploy set"
        );
        assert_ne!(
            same_deploys_different_hashes.cmp(&other),
            Ordering::Equal,
            "…while the ordering is over (host_block, post_state_hash): the two notions disagree"
        );

        // The same deploy set with the *same* hashes agrees on both.
        let identical = chain(1, 2, &[(1, 10, 1)]);
        assert_eq!(same_deploys_different_hashes, identical);
        assert_eq!(
            same_deploys_different_hashes.cmp(&identical),
            Ordering::Equal
        );

        // A different deploy set orders by the hash pair, not by the deploys.
        let different = chain(1, 3, &[(2, 5, 1)]);
        assert_ne!(same_deploys_different_hashes, different);
        assert!(
            same_deploys_different_hashes < different,
            "post state hash 2 < 3"
        );
    }

    /// `deploy_chain_cost` is the sum of the member costs (the merge's cost objective).
    #[test]
    fn the_chain_cost_is_the_sum_of_its_deploys() {
        let c = chain(1, 2, &[(1, 10, 1), (2, 5, 2), (3, 0, 3)]);
        assert_eq!(DeployChainIndex::deploy_chain_cost(&c), 15);
        assert_eq!(DeployChainIndex::deploy_chain_cost(&chain(1, 2, &[])), 0);
    }

    /// Two chains that share a deploy id conflict (the same deploy cannot be in both branches), and
    /// so do two chains whose event logs conflict — but *disjoint, non-conflicting* chains do not.
    #[test]
    fn chain_conflicts_cover_the_id_overlap_and_the_event_relation() {
        let a = chain(1, 2, &[(1, 10, 1)]);
        let shared_id = chain(3, 4, &[(1, 10, 9)]);
        let disjoint = chain(5, 6, &[(2, 10, 2)]);

        assert!(
            DeployChainIndex::deploys_are_conflicting(&a, &shared_id),
            "a shared deploy id is a conflict"
        );
        assert!(
            !DeployChainIndex::deploys_are_conflicting(&a, &disjoint),
            "disjoint deploys on different channels do not conflict"
        );

        // Two chains that destroy the *same* produce (different ids, same channel) conflict through
        // their event logs, not through their ids — the second half of the relation.
        let produced = RProduce::apply(&"shared".to_string(), &"datum".to_string(), false);
        let destroys = |id: u8, host: u8, produced: RProduce| DeployChainIndex {
            host_block: hash(host),
            deploys_with_cost: [deploy_id(id, 0)].into_iter().collect(),
            pre_state_hash: hash(0),
            post_state_hash: hash(host),
            event_log_index: EventLogIndex {
                produces_consumed: [produced].into_iter().collect(),
                ..Default::default()
            },
            state_changes: StateChange::empty(),
        };
        let first = destroys(10, 1, produced.clone());
        let second = destroys(11, 2, produced.clone());
        assert!(
            DeployChainIndex::deploys_are_conflicting(&first, &second),
            "both destroy the same produce: a conflict with no shared id"
        );
        assert!(
            !DeployChainIndex::deploys_are_conflicting(&first, &disjoint),
            "and a chain that touches neither the produce nor the id does not conflict"
        );
    }

    /// `branches_are_conflicting` lifts the same test to *sets* of chains: a shared id anywhere in
    /// either branch is a conflict, and so is a conflicting pair of combined event logs. It is
    /// fallible (AUDIT C41), so this test unwraps: a test chain's diffs are small.
    fn conflicts(a: &BTreeSet<DeployChainIndex>, b: &BTreeSet<DeployChainIndex>) -> bool {
        DeployChainIndex::branches_are_conflicting(a, b)
            .expect("a test chain's accumulation cannot overflow")
    }

    #[test]
    fn branch_conflicts_lift_the_chain_relation() {
        let a: BTreeSet<DeployChainIndex> = [chain(1, 2, &[(1, 10, 1)])].into_iter().collect();
        let b: BTreeSet<DeployChainIndex> = [chain(3, 4, &[(1, 10, 9)])].into_iter().collect();
        let c: BTreeSet<DeployChainIndex> = [chain(5, 6, &[(2, 10, 2)])].into_iter().collect();

        assert!(conflicts(&a, &b));
        assert!(!conflicts(&a, &c));
        // An empty branch conflicts with nothing.
        assert!(!conflicts(&BTreeSet::new(), &a));
    }

    /// `depends` between chains is the event-log dependency: a target that consumed what the source
    /// created depends on it. With disjoint logs it does not.
    #[test]
    fn chain_dependency_follows_the_event_logs() {
        let produce = RProduce::apply(&"chan".to_string(), &"datum".to_string(), false);
        let source = DeployChainIndex {
            host_block: hash(1),
            deploys_with_cost: [deploy_id(1, 0)].into_iter().collect(),
            pre_state_hash: hash(0),
            post_state_hash: hash(1),
            event_log_index: EventLogIndex {
                produces_linear: [produce.clone()].into_iter().collect(),
                ..Default::default()
            },
            state_changes: StateChange::empty(),
        };
        // The target consumed the same produce.
        let target = DeployChainIndex {
            host_block: hash(2),
            deploys_with_cost: [deploy_id(2, 0)].into_iter().collect(),
            pre_state_hash: hash(1),
            post_state_hash: hash(2),
            event_log_index: EventLogIndex {
                produces_consumed: [produce].into_iter().collect(),
                ..Default::default()
            },
            state_changes: StateChange::empty(),
        };

        assert!(DeployChainIndex::depends(&target, &source));
        assert!(!DeployChainIndex::depends(&source, &target));
        assert!(!DeployChainIndex::depends(&source, &source));
    }

    /// The deploy id is what the wire types carry too: `DeployIdWithCost` hashes and orders by both
    /// fields (the pair), unlike `DeployIndex` which orders by the id alone.
    #[test]
    fn a_deploy_with_cost_orders_by_id_then_cost() {
        let a = deploy_id(1, 100);
        let b = deploy_id(1, 5);
        let c = deploy_id(2, 0);
        assert!(b < a, "same id: the lower cost sorts first");
        assert!(a < c, "a lower id sorts first regardless of cost");
        // Hashing covers both fields (a `HashSet` keyed by the pair).
        let mut set = std::collections::HashSet::new();
        set.insert(a.clone());
        assert!(set.contains(&a));
    }
}

/// The falsifier for issue #74: a multi-parent merge must carry the **native** writes of the branches
/// it merges.
///
/// `DeployChainIndex::state_changes` is a `StateChange` - datums, continuations and joins only. Native
/// system-contract state (PoS pool/active/trusted/params, vault balances, registry) lives under
/// dedicated trie prefixes and produces no Rholang event, so it has no representation there. Before
/// this change `compute_merged_state` checkpointed a branch's tuple-space effects without its native
/// ones, and every PoS write in a merged branch - a `trust`, a `bond`, an epoch's activation, a
/// slashed stake - silently reverted. That is the state loss #74 reported (`getBonds` 3 -> 1 with no
/// slashing and no error), and it is why a one-validator chain (single parent, no merge) grew its
/// validator set while a multi-validator chain could not.
#[cfg(test)]
mod native_merge_tests {
    use super::*;

    use rchain_rspace::factory::create_history_repository;
    use rchain_rspace::native_store::PREFIX_POS;
    use rchain_shared::store_manager::InMemoryStoreManager;

    fn key(byte: u8) -> Blake2b256Hash {
        Blake2b256Hash::from_bytes([byte; 32])
    }

    async fn empty_repo() -> RhoHistoryRepository {
        let manager = InMemoryStoreManager::default();
        create_history_repository::<SortedProc, BindPattern, ListParWithRandom, TaggedContinuation>(
            &manager, "rspace",
        )
        .await
        .expect("an in-memory history repository")
    }

    #[tokio::test]
    async fn a_merge_carries_the_native_writes_of_the_branches_it_merges() {
        // A base state that already holds a native leaf, so the test also shows the merge keeps the
        // base's own native state.
        let base_key = key(1);
        let base_repo = empty_repo()
            .await
            .do_checkpoint_with_native(
                &[],
                &[NativeStoreAction::Put {
                    prefix: PREFIX_POS,
                    key: base_key,
                    value: vec![9],
                }],
            )
            .await
            .expect("the base checkpoint");
        let base_state = base_repo.root();

        // One branch block whose only effect is a native write: a PoS call produces no tuple-space
        // chain, which is exactly the shape that was dropped.
        let branch_key = key(2);
        let child = BlockHash::new([3u8; 32]);
        let branch_index = BlockIndex {
            block_hash: child,
            deploy_chains: vec![DeployChainIndex {
                host_block: key(3),
                deploys_with_cost: BTreeSet::from([DeployIdWithCost {
                    id: vec![42],
                    cost: 0,
                }]),
                pre_state_hash: base_state,
                post_state_hash: base_state,
                event_log_index: EventLogIndex::empty(),
                state_changes: StateChange::empty(),
            }],
            native_changes: vec![NativeStoreAction::Put {
                prefix: PREFIX_POS,
                key: branch_key,
                value: vec![7],
            }],
        };
        let block_index = move |_h: BlockHash| {
            let index = branch_index.clone();
            async move { Ok::<BlockIndex, String>(index) }
        };

        // The branch is the conflict scope; nothing has finalised.
        let scope = MergeScope {
            final_scope: BTreeSet::new(),
            conflict_scope: BTreeSet::from([child]),
        };
        let (merged, _rejected) = MergeScope::merge(
            &scope,
            base_state,
            &BTreeMap::<Blake2b256Hash, FringeData>::new(),
            &base_repo,
            &block_index,
            |_| 0,
        )
        .await
        .expect("the merge");

        let reader = base_repo.get_history_reader(merged).await;
        assert!(
            reader
                .get_native(PREFIX_POS, branch_key)
                .await
                .expect("a readable native leaf")
                .is_some(),
            "the branch's native write must survive the merge (#74)"
        );
        assert!(
            reader
                .get_native(PREFIX_POS, base_key)
                .await
                .expect("a readable native leaf")
                .is_some(),
            "and the base's native state must still be there"
        );
    }
}
