//! Casper merge index data structures (Law 17: merge determinism).
//!
//! Ports the pure data types, conflict/dependency relations, and the effectful constructors
//! (`DeployChainIndex.apply`, `BlockIndex.apply`, `MergeScope.merge`) from `casper/.../merging/`.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

use rchain_block_storage::block_store::BlockStore;
use rchain_block_storage::dag::finalizer::Message;
use rchain_block_storage::dag::finalizer::NoAdvance;
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
    compute_relation_map_for_merge_set, resolve_conflict_set_with_census, SearchBudget,
    SearchCensus,
};
use rchain_shared::refined::NonNegI64;
use rchain_shared::serialize::Serialize;

use crate::block_random_seed::BlockRandomSeed;
use crate::event_converter::to_rspace_event;
use crate::interpreter_util::is_genesis_pre_state;
use crate::runtime_manager::RuntimeManager;

/// **The merge search's real input, which nothing observed (#117).**
///
/// `compute_rejection_options` (`sdk/src/dag/merging.rs`) expands one state per nonempty subset of the
/// conflict set that induces an acyclic subgraph of the conflict relation, and its cost is exponential
/// in how many chains can be accepted together. That much is now a pinned, deterministic fact. What was
/// *not* pinned — and what the defect was argued about with instead — is the shape the running node
/// actually hands it: how wide a merge scope gets and how dense its conflicts are. These accumulate
/// that, so the answer is read off a running node rather than assumed.
///
/// Atomics rather than a return value because `MergeScope::merge` is async and has no log handle; its
/// callers do, and they read this. Every value is a monotone maximum or a count, so a lost race can
/// only under-report, never invent.
///
/// **An instrument for #117: remove it with the fix.**
pub mod search_census {
    use super::SearchCensus;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

    /// Merges that ran a conflict search.
    pub static MERGES: AtomicU64 = AtomicU64::new(0);
    /// Widest conflict set seen (chains in one merge scope's conflict search).
    pub static MAX_KEYS: AtomicUsize = AtomicUsize::new(0);
    /// Densest conflict map seen (`(key, conflict)` pairs).
    pub static MAX_CONFLICTS: AtomicUsize = AtomicUsize::new(0);
    /// Most asymmetric pairs seen in one conflict map — `0` everywhere means the exact
    /// output-sensitive rewrite is the symmetric (maximal-independent-set) case.
    pub static MAX_ASYMMETRIC: AtomicUsize = AtomicUsize::new(0);
    /// Most states the search ever expanded on one merge.
    pub static MAX_EXPANDED: AtomicUsize = AtomicUsize::new(0);

    /// **The distributions, which the maxima above cannot give.** A maximum answers "how bad was the
    /// worst merge"; a bound on this class needs "how *often* is a scope wide, and how often is a merge
    /// expensive", because that is what a threshold is chosen from and what a price is set against
    /// (C182, #127's change record, Stage 1).
    ///
    /// **Two of them, because the first run showed width is not the cost.** The quiet devnet reached 43
    /// chains and 899,236 states while the census run's storm reached 1,663,395 at 35 chains — so the
    /// conflict *density* moves the cost more than the width does, and a threshold keyed on width alone
    /// would be keyed on the wrong quantity. `states_expanded` is the cost itself, and its distribution
    /// is what a threshold should be read off.
    pub const WIDTH_EDGES: [usize; 4] = [16, 32, 64, 128];
    /// The cost's edges, logarithmic: below 10³ nothing is worth gating, and 10⁶ is the order that
    /// reached gigabytes in the heap profile.
    ///
    /// **These edges are calibrated against the *old* step unit and the histogram is not comparable
    /// across C178.** Every count ever published under these buckets — the 899,236 and 1,663,395 and
    /// 2,026,511 figures C182 cites — was a count of **accepted-set** states expanded. The directed path
    /// now counts **distinct rejected sets** instead (`SearchCensus::expanded`,
    /// `sdk/src/dag/merging.rs`), which is a strictly smaller number for the same merge and a different
    /// quantity in the same units: on a dependency chain of `n` chains the old unit gave `2^n - 1` and
    /// the new one gives `n`. So a sample that moves bucket after the quotient moved for two reasons at
    /// once, and reading a bucket drop as "the merge got cheaper" is wrong until the distribution is
    /// re-measured on its own arm. That re-measurement is owed and is what C182 closes on.
    pub const EXPANDED_EDGES: [usize; 4] = [1_000, 10_000, 100_000, 1_000_000];

    /// Cumulative counts per bucket: `COUNTS[i]` is the number of samples `<= EDGES[i]`, and the last
    /// entry counts everything past the last edge (the shape Prometheus histograms use).
    pub type Buckets<const N: usize> = [AtomicUsize; N];
    pub static WIDTH_COUNTS: Buckets<5> = [const { AtomicUsize::new(0) }; 5];
    pub static EXPANDED_COUNTS: Buckets<5> = [const { AtomicUsize::new(0) }; 5];
    /// What has already been handed to the registry, per bucket, for each distribution.
    static WIDTH_PUBLISHED: Buckets<5> = [const { AtomicUsize::new(0) }; 5];
    static EXPANDED_PUBLISHED: Buckets<5> = [const { AtomicUsize::new(0) }; 5];

    /// The exact sums: `TOTAL_WIDTH / MERGES` and `TOTAL_EXPANDED / MERGES` are the **means**, where a
    /// histogram valued at bucket edges can only offer a mean *of edges* (C182's second defect).
    pub static TOTAL_WIDTH: AtomicU64 = AtomicU64::new(0);
    pub static TOTAL_EXPANDED: AtomicU64 = AtomicU64::new(0);

    /// Which bucket a value falls in: the first edge it is `<=`, or the open end past the last. Pure, so
    /// the boundaries can be tested without touching the process-wide counters — a test asserting bucket
    /// counts would race every other test in the binary that runs a merge, which is the fixture-isolation
    /// rule this repository already records.
    pub fn bucket_index(edges: &[usize], value: usize) -> usize {
        edges
            .iter()
            .position(|edge| value <= *edge)
            .unwrap_or(edges.len())
    }

    /// What is new since the last call, per bucket, as `(edge, samples)`; the last entry is the
    /// open-ended bucket. `record` accumulates, so a running total would double-count, and the publish
    /// rides the DAG's write path under the guard that path holds — nothing races this read-and-set.
    pub fn take_deltas(
        edges: &[usize],
        counts: &Buckets<5>,
        published: &Buckets<5>,
    ) -> [(usize, usize); 5] {
        let mut out = [(0usize, 0usize); 5];
        for i in 0..5 {
            let total = counts[i].load(Ordering::Relaxed);
            let seen = published[i].swap(total, Ordering::Relaxed);
            out[i] = (
                edges.get(i).copied().unwrap_or(usize::MAX),
                total.saturating_sub(seen),
            );
        }
        out
    }

    /// The width distribution's deltas. See `take_deltas`.
    pub fn take_width_deltas() -> [(usize, usize); 5] {
        take_deltas(&WIDTH_EDGES, &WIDTH_COUNTS, &WIDTH_PUBLISHED)
    }

    /// The cost distribution's deltas. See `take_deltas`.
    pub fn take_expanded_deltas() -> [(usize, usize); 5] {
        take_deltas(&EXPANDED_EDGES, &EXPANDED_COUNTS, &EXPANDED_PUBLISHED)
    }

    pub fn record(census: &SearchCensus) {
        MERGES.fetch_add(1, Ordering::Relaxed);
        TOTAL_WIDTH.fetch_add(census.keys as u64, Ordering::Relaxed);
        TOTAL_EXPANDED.fetch_add(census.expanded as u64, Ordering::Relaxed);
        MAX_KEYS.fetch_max(census.keys, Ordering::Relaxed);
        MAX_CONFLICTS.fetch_max(census.conflicts, Ordering::Relaxed);
        MAX_ASYMMETRIC.fetch_max(census.asymmetric, Ordering::Relaxed);
        MAX_EXPANDED.fetch_max(census.expanded, Ordering::Relaxed);
        let width = bucket_index(&WIDTH_EDGES, census.keys);
        WIDTH_COUNTS[width].fetch_add(1, Ordering::Relaxed);
        let cost = bucket_index(&EXPANDED_EDGES, census.expanded);
        EXPANDED_COUNTS[cost].fetch_add(1, Ordering::Relaxed);
    }

    /// One line, for a caller that has a `Log`.
    pub fn summary() -> String {
        format!(
            "merge search: {} merges · widest scope {} chains / {} conflict pairs / {} asymmetric · \
             most states expanded on one merge {} · width buckets {:?} · cost buckets {:?}",
            MERGES.load(Ordering::Relaxed),
            MAX_KEYS.load(Ordering::Relaxed),
            MAX_CONFLICTS.load(Ordering::Relaxed),
            MAX_ASYMMETRIC.load(Ordering::Relaxed),
            MAX_EXPANDED.load(Ordering::Relaxed),
            WIDTH_COUNTS.each_ref().map(|c| c.load(Ordering::Relaxed)),
            EXPANDED_COUNTS.each_ref().map(|c| c.load(Ordering::Relaxed)),
        )
    }
}

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
    /// Why the fringe derivation did not advance, when it did not — the gate's own reason
    /// (`NoAdvance`), carried here so the caller that holds a logger can report it. `None` means the
    /// fringe advanced or the walk had nothing to publish from the start.
    pub finality_stall: Option<NoAdvance<Validator>>,
    pub justifications: Vec<BlockMetadata>,
    pub max_block_num: i64,
    pub max_seq_nums: BTreeMap<Validator, i64>,
    pub fringe: BTreeSet<BlockHash>,
    pub fringe_state: Blake2b256Hash,
    /// **The fringe this computation started from, and the cache key it looked up** (#139).
    ///
    /// `fringe_state` above is where the merge *ended*; these two are where it began, and they are
    /// what makes a state disagreement legible: the replay's `close_block` anchors the next epoch's
    /// seed to the fringe state, so two nodes that began from different fringes replay the same block
    /// to different post-states. `prev_fringe` empty with `prev_fringe_lookup == hash(∅)` is the
    /// signature of a node whose restored blocks carry no fringe at all
    /// (`node_syncing.rs`'s `populate_dag`), and that is a fact about this node rather than about the
    /// block — which is why it is reported and not inferred.
    pub prev_fringe: BTreeSet<BlockHash>,
    pub prev_fringe_lookup: Blake2b256Hash,
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
        a: &BTreeSet<Arc<DeployChainIndex>>,
        b: &BTreeSet<Arc<DeployChainIndex>>,
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
    /// The block's deploy chains, `Arc`-shared.
    ///
    /// A `DeployChainIndex` carries an `EventLogIndex` (sets of produces and consumes) and a
    /// `StateChange`, so it is a large value — and the merge path does not merely read chains, it
    /// copies them into *sets keyed by chain*: the conflict and final scope sets, the
    /// accepted/rejected-against-the-finally sets, the mergeable-diff map, and every set and map the
    /// SDK's `resolve_conflict_set`, `compute_rejection_options` and relation maps build on top of
    /// those. Sharing makes each of those copies a refcount bump. Measured under a devnet fork storm
    /// (#117), that copy traffic was the dominant allocation site: a heap profile put
    /// `resolve_conflict_set`'s `BTreeMap::clone` at 99,982 allocations against a 35 MiB peak.
    ///
    /// Safe because a chain index is immutable once built, and every comparison the port relies on is
    /// preserved: `Ord` delegates to the inner `(host_block, post_state_hash)`, and `Eq`/`Hash` to
    /// the inner `deploys_with_cost`, exactly as they do for the owned value.
    pub deploy_chains: Vec<Arc<DeployChainIndex>>,
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
                    system_deploy: SystemDeployData::Slash { .. },
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
            chains.push(Arc::new(
                DeployChainIndex::apply(
                    host_block,
                    chain,
                    pre_state_hash,
                    post_state_hash,
                    history_repository,
                )
                .await?,
            ));
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
///
/// Entries are `Arc`-shared rather than owned. A `BlockIndex` holds a `Vec<DeployChainIndex>` and each
/// of those holds an `EventLogIndex` — sets of produces and consumes — so an entry is kilobytes to
/// megabytes, and this cache is read on *every* block validated and every block proposed: the merge
/// asks for each block of its conflict and final scopes, and a hit used to deep-copy the whole index.
/// Measured under a devnet fork storm (#117): ~750 requests over one stall with 711 of them hits, and
/// a heap profile put `DeployChainIndex::clone`/`Vec::clone` under this frame among the largest
/// allocating sites. Sharing is safe because an index is immutable once built.
/// Provisional entry-retention limit independent of finality. The 2026-09-29 restart logged 250
/// cached indices 30 seconds before a global OOM kill. That correlation does not establish the cache
/// as the cause, and the 43-chain merge scope in #117 does not measure the number of block indices.
/// Retaining at most 64 entries bounds this cache's references, not process RSS: individual indices
/// can be large and in-flight users hold their own `Arc`. Evicted indices are recomputed; the miss
/// cost and memory envelope still require measurement under the same controlled campaign.
const BLOCK_INDEX_CACHE_MAX_ENTRIES: usize = 64;

#[derive(Default)]
struct BlockIndexCache {
    entries: BTreeMap<BlockHash, Arc<BlockIndex>>,
    insertion_order: VecDeque<BlockHash>,
}

impl BlockIndexCache {
    fn get(&mut self, hash: &BlockHash) -> Option<Arc<BlockIndex>> {
        let index = self.entries.get(hash).map(Arc::clone)?;
        // Tiny (<=64) LRU queue: keep the blocks merges are actually reusing resident instead of
        // evicting a hot old entry merely because it was inserted before a cold one.
        self.insertion_order.retain(|queued| queued != hash);
        self.insertion_order.push_back(*hash);
        Some(index)
    }

    /// Insert one immutable index and evict the least-recently-used cached entries until the hard
    /// bound holds. Cache order is not consensus state, and unlike a finality-derived prune this
    /// bound keeps working while finality is stalled.
    fn insert(&mut self, hash: BlockHash, index: Arc<BlockIndex>) -> usize {
        self.entries.insert(hash, index);
        self.insertion_order.retain(|queued| queued != &hash);
        self.insertion_order.push_back(hash);

        let mut evicted = 0usize;
        while self.entries.len() > BLOCK_INDEX_CACHE_MAX_ENTRIES {
            let Some(oldest) = self.insertion_order.pop_front() else {
                break;
            };
            if self.entries.remove(&oldest).is_some() {
                evicted += 1;
            }
        }
        evicted
    }

    fn remove_many(&mut self, hashes: &[BlockHash]) -> usize {
        let before = self.entries.len();
        for hash in hashes {
            self.entries.remove(hash);
        }
        if self.entries.len() != before {
            let entries = &self.entries;
            self.insertion_order
                .retain(|hash| entries.contains_key(hash));
        }
        before - self.entries.len()
    }

    fn len(&self) -> usize {
        self.entries.len()
    }
}

static BLOCK_INDEX_CACHE: OnceLock<Mutex<BlockIndexCache>> = OnceLock::new();

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
    ///
    /// `fringe_state_hash` is the state hash of the last finalised fringe as of `block_hash`, and only
    /// the sidecar-regeneration path below uses it: that path *replays* the block, and a replay of a
    /// block that closes an epoch needs the value its close deploy anchored the next seed to. It is a
    /// parameter rather than a DAG read because the callers differ in what they have — validation has
    /// the value it just computed from the merge, and the node's indexing loops have the DAG.
    pub async fn get_block_index(
        runtime: &RuntimeManager,
        block_store: &BlockStore,
        block_hash: BlockHash,
        fringe_state_hash: Blake2b256Hash,
    ) -> Result<Arc<BlockIndex>, String> {
        INDEX_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let cache = BLOCK_INDEX_CACHE.get_or_init(|| Mutex::new(BlockIndexCache::default()));
        if let Some(idx) = cache
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&block_hash)
        {
            // A hit is a refcount bump, not a copy: see the cache's own comment.
            return Ok(idx);
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
                    regenerate_sidecars(
                        runtime,
                        &block,
                        &sender,
                        pre_state_hash,
                        post_state_hash,
                        fringe_state_hash,
                    )
                    .await?
                }
            },
            Err(err) if err.starts_with("Mergeable store invalid state hash") => {
                regenerate_sidecars(
                    runtime,
                    &block,
                    &sender,
                    pre_state_hash,
                    post_state_hash,
                    fringe_state_hash,
                )
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

        let shared = Arc::new(index);
        let (cache_len, capacity_evicted) = {
            let mut guard = cache.lock().unwrap_or_else(|p| p.into_inner());
            let capacity_evicted = guard.insert(block_hash, Arc::clone(&shared));
            (guard.len() as u64, capacity_evicted as u64)
        };
        if capacity_evicted > 0 {
            INDEX_CACHE_CAPACITY_EVICTED
                .fetch_add(capacity_evicted, std::sync::atomic::Ordering::Relaxed);
        }
        INDEX_CACHE_LEN.store(cache_len, std::sync::atomic::Ordering::Relaxed);
        Ok(shared)
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
        let pruned = guard.remove_many(hashes);
        // The oracle logs this (`Pruned N merging indices, new size: M`) and the port did not, which is
        // part of why the cache's growth was invisible (#60).
        if pruned > 0 {
            INDEX_CACHE_PRUNED.fetch_add(pruned as u64, std::sync::atomic::Ordering::Relaxed);
        }
        INDEX_CACHE_LEN.store(guard.len() as u64, std::sync::atomic::Ordering::Relaxed);
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
    fringe_state_hash: Blake2b256Hash,
) -> Result<(Vec<NumberChannelsDiff>, Vec<NativeStoreAction>), String> {
    let seq_num = i64::from(block.seq_num);
    if block.justifications.is_empty()
        && block.state.deploys.is_empty()
        && block.state.system_deploys.is_empty()
    {
        // A failed *cache* write must not fail indexing: the caller only needs the (empty) channels,
        // and the worst case is recomputing this block's index next start.
        if runtime
            .save_mergeable_channels(post_state_hash, sender, seq_num, &[], pre_state_hash)
            .await
            .is_err()
            || runtime
                .save_native_changes(post_state_hash, sender, seq_num, &[])
                .await
                .is_err()
        {
            INDEX_REPLAY_SAVE_FAILURES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        return Ok((Vec::new(), Vec::new()));
    }
    // The expensive path in #60: a full replay of the block from its own pre-state, taken whenever a
    // sidecar is absent (a block that arrived by LFS restore or was deep-replayed, rather than
    // proposed locally). Count and time it - a start-up that does this for every block used to
    // advertise nothing at all.
    INDEX_REPLAY_FALLBACKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let replay_started = std::time::Instant::now();
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
            &fringe_state_hash,
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
    INDEX_REPLAY_MILLIS.fetch_add(
        replay_started.elapsed().as_millis() as u64,
        std::sync::atomic::Ordering::Relaxed,
    );
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

/// How native writes relate the deploy chains of a merge (issue #83).
///
/// A native write is an **absolute value** computed from its block's own pre-state, not an effect
/// that composes: an epoch boundary's `close_block` writes the whole bond pool, a phlo charge writes
/// the staking vault's new balance. So two blocks writing one key can be merged only when one has
/// seen the other, and then the descendant's value is the merged one. Two *concurrent* writers of a
/// key cannot both be kept - concatenating them handed radix history the same key twice and
/// panicked every node at the first boundary with two sibling blocks, and keeping either value
/// silently discards the other's transition (two sibling phlo charges of equal size write equal
/// vault balances, and keeping one destroys the other's REV). Equal values are therefore not a
/// licence to merge: the relation is on keys, not values.
///
/// - **conflict**: chains of two different blocks that wrote a common key and neither of which has
///   seen the other. Resolution then rejects one side, as for a tuple-space conflict.
/// - **dependency**: a chain of a block that wrote a common key with a block it has seen depends on
///   that block's chains, so rejecting the ancestor rejects the value built on it.
///
/// A native-writing block is then accepted or rejected **whole**, and that is enforced after
/// resolution by host block (`reject_whole_blocks`) rather than by an edge between its own chains.
/// The chains of one block are equal under `DeployChainIndex`'s `Ord` (host block, post-state)
/// while unequal under its `Eq` (deploy ids), so resolution's `BTreeSet`s hold them as one class
/// for lookups but as separate members for iteration: `difference` removes only the member it
/// meets, and a two-chain block survives with one chain (`a_rejected_native_block_loses_every_chain`).
/// A same-block edge is worse - under that `Ord` it is a self-loop, and `sdk`'s `traverse_tree`
/// walks the dependency map without a visited set, so the merge never returns
/// (`a_native_block_with_two_chains_merges`).
struct NativeRelations<'a> {
    keys: &'a BTreeMap<Blake2b256Hash, BTreeSet<(u8, Blake2b256Hash)>>,
    ancestry: &'a BTreeMap<BlockHash, BTreeSet<BlockHash>>,
}

impl NativeRelations<'_> {
    fn sees(&self, a: &Blake2b256Hash, b: &Blake2b256Hash) -> bool {
        self.ancestry
            .get(&BlockHash::new(*a.as_bytes()))
            .is_some_and(|seen| seen.contains(&BlockHash::new(*b.as_bytes())))
    }

    fn overlap(&self, a: &Blake2b256Hash, b: &Blake2b256Hash) -> bool {
        match (self.keys.get(a), self.keys.get(b)) {
            (Some(ka), Some(kb)) => !ka.is_disjoint(kb),
            _ => false,
        }
    }

    fn conflicting(&self, a: &DeployChainIndex, b: &DeployChainIndex) -> bool {
        let (ha, hb) = (&a.host_block, &b.host_block);
        ha != hb && self.overlap(ha, hb) && !self.sees(ha, hb) && !self.sees(hb, ha)
    }

    /// Whether `a` depends on `b`.
    fn depends(&self, a: &DeployChainIndex, b: &DeployChainIndex) -> bool {
        let (ha, hb) = (&a.host_block, &b.host_block);
        ha != hb && self.overlap(ha, hb) && self.sees(ha, hb)
    }

    /// Close resolution's result over whole native-writing blocks. A block with any rejected chain
    /// loses all of them - its native writes are block-level and were computed with every chain's
    /// effects - and so does every block that depends on a rejected block: natively (it wrote a
    /// common key on top of it) or through the event logs (a dependency-map edge). The fixpoint is
    /// taken by host block, so it does not rest on the chains' `Ord`; see the type's comment.
    fn reject_whole_blocks(
        &self,
        conflict_set: &BTreeSet<Arc<DeployChainIndex>>,
        to_merge: BTreeSet<Arc<DeployChainIndex>>,
        rejected: BTreeSet<Arc<DeployChainIndex>>,
        dependency_map: &BTreeMap<Arc<DeployChainIndex>, BTreeSet<Arc<DeployChainIndex>>>,
    ) -> (
        BTreeSet<Arc<DeployChainIndex>>,
        BTreeSet<Arc<DeployChainIndex>>,
    ) {
        let scope_hosts: BTreeSet<Blake2b256Hash> =
            conflict_set.iter().map(|c| c.host_block).collect();
        let mut rejected_hosts: BTreeSet<Blake2b256Hash> = rejected
            .iter()
            .map(|c| c.host_block)
            .filter(|h| self.keys.contains_key(h))
            .collect();
        loop {
            let before = rejected_hosts.len();
            let natively_dependent: Vec<Blake2b256Hash> = scope_hosts
                .iter()
                .filter(|h| {
                    !rejected_hosts.contains(*h)
                        && rejected_hosts
                            .iter()
                            .any(|r| self.overlap(h, r) && self.sees(h, r))
                })
                .copied()
                .collect();
            rejected_hosts.extend(natively_dependent);
            let dependent: Vec<Blake2b256Hash> = conflict_set
                .iter()
                .filter(|c| rejected_hosts.contains(&c.host_block))
                .filter_map(|c| dependency_map.get(c))
                .flatten()
                .map(|d| d.host_block)
                .filter(|h| scope_hosts.contains(h))
                .collect();
            rejected_hosts.extend(dependent);
            if rejected_hosts.len() == before {
                break;
            }
        }
        if rejected_hosts.is_empty() {
            return (to_merge, rejected);
        }
        let to_merge = to_merge
            .into_iter()
            .filter(|c| !rejected_hosts.contains(&c.host_block))
            .collect();
        let rejected = rejected
            .into_iter()
            .chain(
                conflict_set
                    .iter()
                    .filter(|c| rejected_hosts.contains(&c.host_block))
                    .cloned(),
            )
            .collect();
        (to_merge, rejected)
    }

    /// The accepted blocks' native writes as one action per key: writers in ancestry order, so a
    /// descendant's value replaces its ancestor's. Two accepted writers of a key that are not so
    /// ordered would mean resolution kept a conflict, and that is refused rather than decided by
    /// iteration order.
    fn fold(
        &self,
        accepted: &BTreeSet<Blake2b256Hash>,
        native_by_block: &BTreeMap<Blake2b256Hash, &[NativeStoreAction]>,
    ) -> Result<Vec<NativeStoreAction>, String> {
        // A block has seen strictly more in-scope blocks than any ancestor of it, so this order
        // puts every ancestor before its descendants; the hash only makes the order total.
        let mut writers: Vec<&Blake2b256Hash> = accepted
            .iter()
            .filter(|h| native_by_block.get(*h).is_some_and(|a| !a.is_empty()))
            .collect();
        let seen_count = |h: &Blake2b256Hash| {
            self.ancestry
                .get(&BlockHash::new(*h.as_bytes()))
                .map_or(0, BTreeSet::len)
        };
        writers.sort_by_key(|h| (seen_count(h), **h));

        let mut by_key: BTreeMap<(u8, Blake2b256Hash), (Blake2b256Hash, NativeStoreAction)> =
            BTreeMap::new();
        for host in writers {
            for action in native_by_block.get(host).copied().unwrap_or_default() {
                let key = action.slot();
                if let Some((previous, _)) = by_key.get(&key) {
                    if !self.sees(host, previous) {
                        return Err(format!(
                            "blocks {} and {} both write native key {:02x}/{} and neither has seen \
                             the other; conflict resolution must reject one",
                            previous.to_hex(),
                            host.to_hex(),
                            key.0,
                            key.1.to_hex()
                        ));
                    }
                }
                by_key.insert(key, (*host, action.clone()));
            }
        }
        Ok(by_key.into_values().map(|(_, action)| action).collect())
    }
}

/// The scope of a merge: final (immutable) and conflict (alterable) blocks (port of `MergeScope`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergeScope {
    pub final_scope: BTreeSet<BlockHash>,
    pub conflict_scope: BTreeSet<BlockHash>,
    /// For each block of either scope, the blocks of either scope it has *seen* (its in-scope
    /// ancestors, itself excluded). Not in the Scala: native writes are absolute values rather than
    /// event logs, so whether two blocks writing one native key are a conflict or a dependency is a
    /// question about the DAG, not about their effects (issue #83). A block absent from the map has
    /// seen nothing in scope, i.e. every pair is read as concurrent.
    pub ancestry: BTreeMap<BlockHash, BTreeSet<BlockHash>>,
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

        // Restricted to the scope, so the map is as small as the merge rather than the chain.
        let in_scope: BTreeSet<BlockHash> = conflict_scope.union(&f_scope_ids).copied().collect();
        let ancestry: BTreeMap<BlockHash, BTreeSet<BlockHash>> = in_scope
            .iter()
            .filter_map(|id| {
                let seen: BTreeSet<BlockHash> = dag_data
                    .get(id)?
                    .seen
                    .iter()
                    .filter(|s| *s != id && in_scope.contains(s))
                    .copied()
                    .collect();
                (!seen.is_empty()).then_some((*id, seen))
            })
            .collect();

        Ok((
            MergeScope {
                final_scope: f_scope_ids,
                conflict_scope,
                ancestry,
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
        Fut: std::future::Future<Output = Result<Arc<BlockIndex>, String>>,
    {
        let conflict_indices: Vec<Arc<BlockIndex>> = {
            let mut v = Vec::new();
            for h in &merge_scope.conflict_scope {
                v.push(block_index(*h).await?);
            }
            v
        };
        let final_indices: Vec<Arc<BlockIndex>> = {
            let mut v = Vec::new();
            for h in &merge_scope.final_scope {
                v.push(block_index(*h).await?);
            }
            v
        };

        // Native effects are block-level, so index them by host block here. The final scope's blocks
        // are ancestors of the base state (their effects are already in it); the conflict scope's are
        // what this merge applies, and the map below keeps only the ones with an accepted chain.
        // Borrowed, not cloned: the actions live in the `BlockIndex` this scope already holds behind
        // an `Arc`, so the map only needs a view of them (#117 — the merge path copied these per
        // conflict block per merge).
        let native_by_block: BTreeMap<Blake2b256Hash, &[NativeStoreAction]> = conflict_indices
            .iter()
            .map(|b| {
                (
                    Blake2b256Hash::from_bytes(*b.block_hash.as_bytes()),
                    b.native_changes.as_slice(),
                )
            })
            .collect();
        // The native keys each block of either scope wrote, for the relations below. The final
        // scope's are needed too: a conflict-scope block that wrote a key concurrently with a
        // finalised writer of it is incompatible with the final state.
        let native_keys: BTreeMap<Blake2b256Hash, BTreeSet<(u8, Blake2b256Hash)>> =
            conflict_indices
                .iter()
                .chain(final_indices.iter())
                .filter(|b| !b.native_changes.is_empty())
                .map(|b| {
                    (
                        Blake2b256Hash::from_bytes(*b.block_hash.as_bytes()),
                        b.native_changes
                            .iter()
                            .map(NativeStoreAction::slot)
                            .collect(),
                    )
                })
                .collect();
        let conflict_set: BTreeSet<Arc<DeployChainIndex>> = conflict_indices
            .iter()
            .flat_map(|b| b.deploy_chains.iter().cloned())
            .collect();
        let final_set: BTreeSet<Arc<DeployChainIndex>> = final_indices
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
        let mut rejected_finally: BTreeSet<Arc<DeployChainIndex>> = BTreeSet::new();
        let mut accepted_finally: BTreeSet<Arc<DeployChainIndex>> = BTreeSet::new();
        for b in &final_indices {
            let rejected = rejections_map.get(&b.block_hash).copied();
            for chain in &b.deploy_chains {
                // Borrowed rather than cloned: only used for a set lookup below.
                let first_id = chain.deploys_with_cost.iter().next().map(|d| &d.id);
                let is_rejected = match (rejected, first_id) {
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
        // A view, not a copy: each diff is owned by the chain it describes, and those live in
        // `conflict_set` for the whole merge, so the map only needs to point at them (#117 — this
        // copied one diff map per chain per merge).
        let mergeable_diffs_map: BTreeMap<Arc<DeployChainIndex>, &NumberChannelsDiff> =
            conflict_set
                .iter()
                .map(|b| (Arc::clone(b), &b.event_log_index.number_channels_data))
                .collect();
        let mut all_channel_hashes: BTreeSet<Blake2b256Hash> = BTreeSet::new();
        for diffs in mergeable_diffs_map.values() {
            all_channel_hashes.extend(diffs.keys().copied());
        }
        let init_mergeable_values =
            read_mergeable_values(history_repository, base_state, &all_channel_hashes).await?;

        // A chain relation is its event-log relation, widened by its host block's native writes
        // (issue #83): see `NativeRelations`.
        let native = NativeRelations {
            keys: &native_keys,
            ancestry: &merge_scope.ancestry,
        };
        let (conflicts_map, dependency_map) = compute_relation_map_for_merge_set(
            &conflict_set,
            &final_set,
            |a, b| DeployChainIndex::deploys_are_conflicting(a, b) || native.conflicting(a, b),
            |a, b| DeployChainIndex::depends(a, b) || native.depends(a, b),
        );

        let ((to_merge, rejected), search) = resolve_conflict_set_with_census(
            &conflict_set,
            &accepted_finally,
            &rejected_finally,
            // The scope sets hold `Arc<DeployChainIndex>`, so the cost closure that the SDK's generic
            // `D` sees takes the shared handle; `rejection_cost` stays a plain `Fn(&DeployChainIndex)`
            // for every caller, and the deref coercion bridges the two here rather than at each call
            // site.
            |c: &Arc<DeployChainIndex>| rejection_cost(c),
            &conflicts_map,
            &dependency_map,
            &mergeable_diffs_map,
            &init_mergeable_values,
            // **The bound, and where its refusal goes.** `SearchBudget::NODE` is node-local policy, not
            // a protocol constant: an exceeded search returns **no answer**, this refuses the merge, and
            // the `Err` travels as `String` to `ValidateError::Internal` — a *drop*, which is the same
            // class as a missing dependency. It must never become a `ValidationFailed`: a budget is this
            // node's own resource policy, and marking the *block* for it would estrange the node from a
            // proposer for a local problem (C173). The restoring rule C173's fix added is keyed on a
            // view-dependent `Divergence` and does not cover a local budget — so the drop stays a drop,
            // and every node that does run the search inside its budget gets the identical answer.
            SearchBudget::NODE,
        )
        .map_err(|e| {
            format!(
                "merge search exceeded its budget ({} steps, {} options, budget {} steps / {} options):                  refused locally rather than answered from a partial enumeration",
                e.steps, e.options, e.budget.max_steps, e.budget.max_options
            )
        })?;
        // #117's instrument: what this merge handed the search. See `search_census`.
        search_census::record(&search);
        let (to_merge, rejected) =
            native.reject_whole_blocks(&conflict_set, to_merge, rejected, &dependency_map);

        // The native effects of the blocks whose chains survive conflict resolution. A block
        // contributes its native changes iff at least one of its chains is accepted - native effects
        // are per block, not per chain - and the relations above make that all-or-nothing for a block
        // that wrote anything native. Two accepted blocks writing one key are therefore ancestor and
        // descendant (concurrent writers conflict), and the descendant's value already includes the
        // ancestor's, so writers are applied ancestors first and the last write of a key wins.
        //
        // This replaces the first #83 fix (b5e024d), which kept one write per slot by taking the last
        // accepted host in hash order. That restored liveness, but it decided a disagreement by hash:
        // two concurrent blocks' absolute values are two transitions, and keeping either discards the
        // other's (see `NativeRelations`).
        let accepted_hosts: BTreeSet<Blake2b256Hash> =
            to_merge.iter().map(|c| c.host_block).collect();
        let native_changes = native.fold(&accepted_hosts, &native_by_block)?;

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
        to_merge: &BTreeSet<Arc<DeployChainIndex>>,
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

// --- observability for a start-up replay ------------------------------------
//
// Indexing every stored block is the expensive half of a restart, and the port reported nothing about
// it: no progress, no count, and no hint that an "unresponsive" node was in fact working through its
// DAG (#60). These counters make that visible; the node logs them from its block-index closure while a
// replay runs (`wire_block_processing` in `node_runtime.rs`).
static INDEX_CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static INDEX_REPLAY_FALLBACKS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static INDEX_REPLAY_MILLIS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static INDEX_REPLAY_SAVE_FAILURES: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);
static INDEX_CACHE_PRUNED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static INDEX_CACHE_CAPACITY_EVICTED: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);
static INDEX_CACHE_LEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A snapshot of the block-index counters, for a node's progress line.
#[derive(Clone, Copy, Debug, Default)]
pub struct IndexStats {
    pub calls: u64,
    pub replay_fallbacks: u64,
    pub replay_millis: u64,
    pub replay_save_failures: u64,
    pub cache_len: u64,
    pub cache_pruned: u64,
    pub cache_capacity_evicted: u64,
}

impl IndexStats {
    /// Read the counters. Relaxed ordering throughout: this is a progress report, not a barrier.
    pub fn read() -> Self {
        use std::sync::atomic::Ordering::Relaxed;
        IndexStats {
            calls: INDEX_CALLS.load(Relaxed),
            replay_fallbacks: INDEX_REPLAY_FALLBACKS.load(Relaxed),
            replay_millis: INDEX_REPLAY_MILLIS.load(Relaxed),
            replay_save_failures: INDEX_REPLAY_SAVE_FAILURES.load(Relaxed),
            cache_len: INDEX_CACHE_LEN.load(Relaxed),
            cache_pruned: INDEX_CACHE_PRUNED.load(Relaxed),
            cache_capacity_evicted: INDEX_CACHE_CAPACITY_EVICTED.load(Relaxed),
        }
    }

    /// One line, for a log: what indexing has cost so far.
    pub fn summary(&self) -> String {
        format!(
            "{} blocks indexed, {} replay fallbacks ({} ms, {} not persisted), index cache {} entries, {} pruned, cap {}, {} capacity evicted",
            self.calls,
            self.replay_fallbacks,
            self.replay_millis,
            self.replay_save_failures,
            self.cache_len,
            self.cache_pruned,
            BLOCK_INDEX_CACHE_MAX_ENTRIES,
            self.cache_capacity_evicted
        )
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

    /// The cache must stay bounded even when finality never advances, and eviction must not invalidate
    /// an index already handed to a merge. The latter is the reason entries are `Arc`-shared: removing
    /// the cache's reference only changes whether the next lookup recomputes it.
    #[test]
    fn capacity_eviction_bounds_the_cache_and_keeps_in_flight_indices_alive() {
        let first_hash = block_hash(0);
        let first = Arc::new(BlockIndex {
            block_hash: first_hash,
            deploy_chains: Vec::new(),
            native_changes: Vec::new(),
        });
        let mut cache = BlockIndexCache::default();
        assert_eq!(cache.insert(first_hash, Arc::clone(&first)), 0);

        let mut last = Arc::clone(&first);
        for id in 1..=BLOCK_INDEX_CACHE_MAX_ENTRIES {
            let hash = block_hash(u16::try_from(id).expect("cache test id fits in u16"));
            last = Arc::new(BlockIndex {
                block_hash: hash,
                deploy_chains: Vec::new(),
                native_changes: Vec::new(),
            });
            cache.insert(hash, Arc::clone(&last));
        }

        assert_eq!(cache.len(), BLOCK_INDEX_CACHE_MAX_ENTRIES);
        assert!(
            cache.get(&first_hash).is_none(),
            "the least-recently-used cache entry is evicted"
        );
        assert_eq!(
            first.block_hash, first_hash,
            "an in-flight Arc remains valid after cache eviction"
        );
        let cached_last = cache
            .get(&last.block_hash)
            .expect("the newest index remains cached");
        assert!(Arc::ptr_eq(&cached_last, &last));
    }

    /// A hit refreshes recency. This distinguishes the shipped policy from FIFO: after filling the
    /// cache, touching the oldest entry must keep it resident when one new entry arrives, and the
    /// second-oldest entry is the one evicted.
    #[test]
    fn a_cache_hit_refreshes_lru_recency() {
        let mut cache = BlockIndexCache::default();
        for id in 0..u16::try_from(BLOCK_INDEX_CACHE_MAX_ENTRIES).expect("cache bound fits in u16")
        {
            let hash = block_hash(id);
            cache.insert(
                hash,
                Arc::new(BlockIndex {
                    block_hash: hash,
                    deploy_chains: Vec::new(),
                    native_changes: Vec::new(),
                }),
            );
        }

        let oldest = block_hash(0);
        let second_oldest = block_hash(1);
        let _ = cache.get(&oldest).expect("oldest entry is resident");

        let newcomer = block_hash(
            u16::try_from(BLOCK_INDEX_CACHE_MAX_ENTRIES).expect("cache bound fits in u16"),
        );
        cache.insert(
            newcomer,
            Arc::new(BlockIndex {
                block_hash: newcomer,
                deploy_chains: Vec::new(),
                native_changes: Vec::new(),
            }),
        );

        assert!(
            cache.get(&oldest).is_some(),
            "the refreshed entry stays resident"
        );
        assert!(
            cache.get(&second_oldest).is_none(),
            "the least-recently-used entry is evicted"
        );
        assert!(cache.get(&newcomer).is_some());
        assert_eq!(cache.len(), BLOCK_INDEX_CACHE_MAX_ENTRIES);
    }

    /// Explicit finality pruning must also remove its bookkeeping entry; otherwise the queue used to
    /// enforce the hard bound would become the new unbounded structure.
    #[test]
    fn finality_pruning_removes_cache_order_bookkeeping_too() {
        let mut cache = BlockIndexCache::default();
        for id in 0..4u16 {
            let hash = block_hash(id);
            cache.insert(
                hash,
                Arc::new(BlockIndex {
                    block_hash: hash,
                    deploy_chains: Vec::new(),
                    native_changes: Vec::new(),
                }),
            );
        }

        assert_eq!(cache.remove_many(&[block_hash(1), block_hash(3)]), 2);
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.insertion_order.len(), 2);
        assert!(cache.get(&block_hash(1)).is_none());
        assert!(cache.get(&block_hash(3)).is_none());
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
    /// hazard for a Rust `BTreeSet<Arc<DeployChainIndex>>` (which uses `Ord`), because `Ord` is supposed to
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
    fn conflicts(a: &BTreeSet<Arc<DeployChainIndex>>, b: &BTreeSet<Arc<DeployChainIndex>>) -> bool {
        DeployChainIndex::branches_are_conflicting(a, b)
            .expect("a test chain's accumulation cannot overflow")
    }

    #[test]
    fn branch_conflicts_lift_the_chain_relation() {
        let a: BTreeSet<Arc<DeployChainIndex>> =
            [Arc::new(chain(1, 2, &[(1, 10, 1)]))].into_iter().collect();
        let b: BTreeSet<Arc<DeployChainIndex>> =
            [Arc::new(chain(3, 4, &[(1, 10, 9)]))].into_iter().collect();
        let c: BTreeSet<Arc<DeployChainIndex>> =
            [Arc::new(chain(5, 6, &[(2, 10, 2)]))].into_iter().collect();

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
            deploy_chains: vec![Arc::new(DeployChainIndex {
                host_block: key(3),
                deploys_with_cost: BTreeSet::from([DeployIdWithCost {
                    id: vec![42],
                    cost: 0,
                }]),
                pre_state_hash: base_state,
                post_state_hash: base_state,
                event_log_index: EventLogIndex::empty(),
                state_changes: StateChange::empty(),
            })],
            native_changes: vec![NativeStoreAction::Put {
                prefix: PREFIX_POS,
                key: branch_key,
                value: vec![7],
            }],
        };
        let block_index = move |_h: BlockHash| {
            let index = branch_index.clone();
            async move { Ok::<Arc<BlockIndex>, String>(Arc::new(index)) }
        };

        // The branch is the conflict scope; nothing has finalised.
        let scope = MergeScope {
            final_scope: BTreeSet::new(),
            conflict_scope: BTreeSet::from([child]),
            ancestry: BTreeMap::new(),
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

    /// **The reproduction for #83, at the granularity the defect lives at.**
    ///
    /// `merge` concatenates the native effects of every accepted host block (`native_by_block` ->
    /// `extend`). Each *block's* own list is duplicate-free by construction —
    /// `InMemNativeStore::drain_changes` maps a `BTreeMap<(prefix, key), _>`, one action per slot,
    /// and clears the overlay — but **two blocks at the same height can write the same slot**, and
    /// at an epoch boundary that is not a rare race: *every* proposer runs `close_block`, which
    /// writes the same five `PREFIX_POS` leaves (`bonds`, `active`, `withdrawers`,
    /// `pending_withdrawers`, `committed_rewards`) from the same pre-state. Two of those blocks both
    /// accepted into one merge produce the same key twice, and `RadixHistory::process` refuses the
    /// batch with the panic the issue reports — on every node, at the same height, because the
    /// inputs are identical.
    ///
    /// This is the deterministic half of the diagnosis: no devnet, no scheduler, two branches and
    /// one slot. What it asserts is the contract a fix owes, which is *not* merely "does not panic":
    /// the two concurrent writers **conflict**, so exactly one host is rejected and the slot holds
    /// the other's value whole. (The first fix kept the last host's write in hash order and rejected
    /// nothing; `boundary_merge_tests` shows why a kept-both merge loses a transition.)
    ///
    /// The fixture is chosen so that assertion can discriminate. The *values* are swapped between
    /// the two rounds, so the choice of which host survives must not depend on them: a rule that
    /// took the numerically larger value, or the last-arriving write, would pick differently.
    #[tokio::test]
    async fn two_branch_blocks_writing_one_native_slot_do_not_panic_the_merge() {
        let base_repo = empty_repo().await;
        let base_state = base_repo.root();
        let shared = key(9);

        let branch = |host: u8, value: u8| BlockIndex {
            block_hash: BlockHash::new([host; 32]),
            deploy_chains: vec![Arc::new(DeployChainIndex {
                host_block: key(host),
                deploys_with_cost: BTreeSet::from([DeployIdWithCost {
                    id: vec![host],
                    cost: 0,
                }]),
                pre_state_hash: base_state,
                post_state_hash: base_state,
                event_log_index: EventLogIndex::empty(),
                state_changes: StateChange::empty(),
            })],
            native_changes: vec![NativeStoreAction::Put {
                prefix: PREFIX_POS,
                key: shared,
                value: vec![value],
            }],
        };
        let hosts = [BlockHash::new([1u8; 32]), BlockHash::new([2u8; 32])];
        let mut rejected_hosts = Vec::new();

        for (first_value, second_value) in [(99u8, 22u8), (22u8, 99u8)] {
            let indexes: BTreeMap<BlockHash, BlockIndex> = [
                (hosts[0], branch(1, first_value)),
                (hosts[1], branch(2, second_value)),
            ]
            .into_iter()
            .collect();
            let block_index = move |h: BlockHash| {
                let index = indexes.get(&h).cloned();
                async move {
                    index
                        .map(Arc::new)
                        .ok_or_else(|| format!("no index for {h:?}"))
                }
            };
            let scope = MergeScope {
                final_scope: BTreeSet::new(),
                conflict_scope: BTreeSet::from(hosts),
                ancestry: BTreeMap::new(),
            };

            let (merged, rejected) = MergeScope::merge(
                &scope,
                base_state,
                &BTreeMap::<Blake2b256Hash, FringeData>::new(),
                &base_repo,
                &block_index,
                |_| 0,
            )
            .await
            .expect(
                "two accepted blocks writing one native slot must merge rather than panic (#83): \
                 an epoch boundary puts several `close_block`s at one height by construction",
            );

            assert_eq!(rejected.len(), 1, "the two concurrent writers conflict");
            let rejected_host = rejected.iter().next().expect("one rejected host")[0];
            rejected_hosts.push(rejected_host);
            let surviving_value = if rejected_host == 1 {
                second_value
            } else {
                first_value
            };
            let reader = base_repo.get_history_reader(merged).await;
            assert_eq!(
                reader
                    .get_native(PREFIX_POS, shared)
                    .await
                    .expect("a readable native leaf"),
                Some(vec![surviving_value]),
                "the slot holds the surviving host's write"
            );
        }
        assert_eq!(
            rejected_hosts[0], rejected_hosts[1],
            "which host is rejected must not depend on what the hosts wrote: every node decides it \
             from the same DAG, whatever the values"
        );
    }
}

/// Issue #83: two blocks that each wrote a native key, merged in one scope.
///
/// Every epoch boundary's `close_block` writes the same six PoS keys, so two unfinalised sibling
/// blocks at a boundary height - routine with three validators - used to reach radix history as one
/// batch with every key twice, and every node panicked (`Cannot process duplicate actions on one
/// key`) in the same second. These play the boundary for real on `NativeSystemState`, then merge.
#[cfg(test)]
mod boundary_merge_tests {
    use super::*;
    use std::sync::Arc;

    use rchain_crypto::public_key::PublicKey;
    use rchain_rholang::native_state::{NativeSystemState, PosGenesis, PosParams};
    use rchain_rholang::util::rev_address::RevAddress;
    use rchain_rspace::factory::create_history_repository;
    use rchain_rspace::native_store::InMemNativeStore;
    use rchain_shared::store_manager::InMemoryStoreManager;

    const EPOCH: i64 = 10;

    fn validator(byte: u8) -> Validator {
        Validator::from_slice(&[byte; 65])
    }

    fn nn(x: i64) -> NonNegI64 {
        NonNegI64::try_from(x).unwrap()
    }

    fn payer(byte: u8) -> (PublicKey, String) {
        let pk = PublicKey::new(vec![byte; 65]);
        let address = RevAddress::from_public_key(&pk).unwrap().to_base58();
        (pk, address)
    }

    /// The issue's genesis - bonds 100/100/50, `--epoch-length 10` - with two funded deployers.
    async fn genesis() -> (RhoHistoryRepository, Blake2b256Hash) {
        let manager = InMemoryStoreManager::default();
        let repo = create_history_repository::<
            SortedProc,
            BindPattern,
            ListParWithRandom,
            TaggedContinuation,
        >(&manager, "rspace")
        .await
        .unwrap();
        let store = Arc::new(InMemNativeStore::empty());
        let native = NativeSystemState::new(store.clone());
        native
            .install_genesis(&PosGenesis {
                bonds: [
                    (validator(1), nn(100)),
                    (validator(2), nn(100)),
                    (validator(3), nn(50)),
                ]
                .into_iter()
                .collect(),
                trusted: BTreeSet::new(),
                params: PosParams {
                    epoch_length: EPOCH,
                    quarantine_length: EPOCH,
                    minimum_bond: nn(1),
                    maximum_bond: nn(100),
                    number_of_active_validators: 10,
                    executor_share: nn(0),
                },
            })
            .unwrap();
        for byte in [8, 9] {
            native.set_vault_balance(&payer(byte).1, nn(1000));
        }
        let repo = repo
            .do_checkpoint_with_native(&[], &store.drain_changes())
            .await
            .unwrap();
        let root = repo.root();
        (repo, root)
    }

    /// What a block does besides closing itself.
    enum Body {
        Nothing,
        /// Validator 3 asks to withdraw: the stake change.
        Withdraw,
        /// A user deploy by `payer(byte)` whose phlo, 500, goes into the staking vault.
        Deploy(u8),
    }

    /// Play block `number` on `pre_state`: its body, then `close_block`. Returns the block's native
    /// writes.
    async fn play(
        repo: &RhoHistoryRepository,
        pre_state: Blake2b256Hash,
        number: i64,
        fringe: u8,
        body: Body,
    ) -> Vec<NativeStoreAction> {
        let store = Arc::new(InMemNativeStore::new(
            repo.get_native_reader(pre_state).await,
        ));
        let native = NativeSystemState::new(store.clone());
        match body {
            Body::Nothing => {}
            Body::Withdraw => native
                .withdraw(&validator(3), number)
                .await
                .unwrap()
                .unwrap(),
            Body::Deploy(byte) => native
                .pre_charge(&payer(byte).0, nn(500))
                .await
                .unwrap()
                .unwrap(),
        }
        native
            .close_block(number, Blake2b256Hash::create(&[fringe]))
            .await
            .unwrap()
            .unwrap();
        store.drain_changes()
    }

    fn block_hash(n: u8) -> BlockHash {
        BlockHash::new([n; 32])
    }

    /// A block index with one (close-block) chain, as every block has.
    fn index(n: u8, pre_state: Blake2b256Hash, native: Vec<NativeStoreAction>) -> BlockIndex {
        BlockIndex {
            block_hash: block_hash(n),
            deploy_chains: vec![Arc::new(DeployChainIndex {
                host_block: Blake2b256Hash::from_bytes([n; 32]),
                deploys_with_cost: BTreeSet::from([DeployIdWithCost {
                    id: vec![n],
                    cost: 0,
                }]),
                pre_state_hash: pre_state,
                post_state_hash: pre_state,
                event_log_index: EventLogIndex::empty(),
                state_changes: StateChange::empty(),
            })],
            native_changes: native,
        }
    }

    async fn merge(
        repo: &RhoHistoryRepository,
        base: Blake2b256Hash,
        blocks: Vec<BlockIndex>,
        ancestry: BTreeMap<BlockHash, BTreeSet<BlockHash>>,
    ) -> Result<(Blake2b256Hash, BTreeSet<Vec<u8>>), String> {
        let scope = MergeScope {
            final_scope: BTreeSet::new(),
            conflict_scope: blocks.iter().map(|b| b.block_hash).collect(),
            ancestry,
        };
        let lookup = move |h: BlockHash| {
            let found = blocks.iter().find(|b| b.block_hash == h).cloned();
            async move {
                found
                    .map(Arc::new)
                    .ok_or_else(|| format!("no index for {h:?}"))
            }
        };
        MergeScope::merge(
            &scope,
            base,
            &BTreeMap::<Blake2b256Hash, FringeData>::new(),
            repo,
            &lookup,
            |_| 0,
        )
        .await
    }

    /// Every native value the writes of `actions` leave behind, read back from `state`.
    async fn read_back(
        repo: &RhoHistoryRepository,
        state: Blake2b256Hash,
        actions: &[NativeStoreAction],
    ) -> Vec<Option<Vec<u8>>> {
        let reader = repo.get_native_reader(state).await;
        let mut out = Vec::new();
        for action in actions {
            let (prefix, key) = action.slot();
            out.push(reader.get_native(prefix, key).await.unwrap());
        }
        out
    }

    fn values(actions: &[NativeStoreAction]) -> Vec<Option<Vec<u8>>> {
        actions
            .iter()
            .map(|a| match a {
                NativeStoreAction::Put { value, .. } => Some(value.clone()),
                NativeStoreAction::Delete { .. } => None,
            })
            .collect()
    }

    /// Two sibling boundary blocks on one pre-state: the merge must neither panic nor error, must
    /// reject exactly one of them, and must leave exactly the other's boundary behind - never a mix.
    async fn siblings_resolve_to_one(a_body: Body, a_fringe: u8, b_body: Body, b_fringe: u8) {
        let (repo, base) = genesis().await;
        let a = play(&repo, base, EPOCH, a_fringe, a_body).await;
        let b = play(&repo, base, EPOCH, b_fringe, b_body).await;
        let (merged, rejected) = merge(
            &repo,
            base,
            vec![index(0xa0, base, a.clone()), index(0xb0, base, b.clone())],
            BTreeMap::new(),
        )
        .await
        .expect("sibling boundary blocks merge (#83)");

        assert_eq!(rejected.len(), 1, "exactly one sibling is rejected");
        let kept = if rejected.contains(&vec![0xa0]) {
            &b
        } else {
            &a
        };
        assert_eq!(
            read_back(&repo, merged, kept).await,
            values(kept),
            "the merged state is the surviving sibling's boundary, whole"
        );
    }

    /// The panic of #83 itself: identical boundaries (same pre-state, same fringe, no stake change).
    /// Equal writes are still two transitions, so one sibling is rejected rather than both kept.
    #[tokio::test]
    async fn identical_sibling_boundaries_merge() {
        siblings_resolve_to_one(Body::Nothing, 1, Body::Nothing, 1).await;
    }

    /// **The shape the merge search is really handed (#117).** Every other statement about the search's
    /// cost in this tree is made against a *chosen* shape (`sdk/tests/merging_scaling.rs` picks a fork;
    /// `sdk/src/dag/merging.rs`'s table picks four). This one is produced by the merge path itself, out
    /// of the fixtures the sibling-boundary tests use, and it answers the question the choice of fix
    /// turns on: **is the relation symmetric?** If it is, the exact output-sensitive rewrite is the
    /// classical maximal-independent-set enumeration and is output-polynomial; if it is not, the
    /// rewrite needs the directed argument instead.
    ///
    /// The assertion is on a process-wide maximum, and it is still deterministic: `MAX_ASYMMETRIC` is
    /// monotone, so it holds every merge this test binary ran, and any test whose real merge produced an
    /// asymmetric pair would raise it above zero regardless of the order the tests ran in.
    /// The width distribution's boundaries — the half of C182 that can be tested without racing the
    /// process-wide counters, since every other test in this binary that runs a merge updates them.
    #[test]
    fn the_width_buckets_are_inclusive_at_their_edges() {
        use crate::merging::search_census::{bucket_index, EXPANDED_EDGES, WIDTH_EDGES};
        assert_eq!(bucket_index(&WIDTH_EDGES, 0), 0);
        assert_eq!(bucket_index(&WIDTH_EDGES, 16), 0, "the edge is inclusive");
        assert_eq!(bucket_index(&WIDTH_EDGES, 17), 1);
        assert_eq!(bucket_index(&WIDTH_EDGES, 32), 1);
        assert_eq!(bucket_index(&WIDTH_EDGES, 33), 2);
        assert_eq!(bucket_index(&WIDTH_EDGES, 64), 2);
        assert_eq!(bucket_index(&WIDTH_EDGES, 65), 3);
        assert_eq!(bucket_index(&WIDTH_EDGES, 128), 3);
        assert_eq!(
            bucket_index(&WIDTH_EDGES, 129),
            4,
            "past the last edge is the open-ended bucket"
        );
        assert_eq!(bucket_index(&WIDTH_EDGES, 1_000_000), 4);
        // And the cost's edges, which are logarithmic — a merge that expanded a million states is the
        // order the heap profile reached, and it must not land in the same bucket as one that did a
        // thousand.
        assert_eq!(bucket_index(&EXPANDED_EDGES, 1_000), 0);
        assert_eq!(bucket_index(&EXPANDED_EDGES, 1_001), 1);
        assert_eq!(bucket_index(&EXPANDED_EDGES, 1_000_000), 3);
        assert_eq!(
            bucket_index(&EXPANDED_EDGES, 1_663_395),
            4,
            "the census run's worst merge"
        );
    }

    #[tokio::test]
    async fn the_merge_search_sees_the_shape_the_merge_builds() {
        siblings_resolve_to_one(Body::Nothing, 1, Body::Nothing, 1).await;
        eprintln!("{}", search_census::summary());
        assert_eq!(
            search_census::MAX_ASYMMETRIC.load(std::sync::atomic::Ordering::Relaxed),
            0,
            "the conflict relation the merge hands the search is not symmetric, so the exact \
             rewrite cannot be the symmetric (maximal-independent-set) case without more work"
        );
    }

    /// The proposers saw different finalised fringes, so their epoch seeds differ.
    #[tokio::test]
    async fn sibling_boundaries_with_different_fringes_merge() {
        siblings_resolve_to_one(Body::Nothing, 1, Body::Nothing, 2).await;
    }

    /// The stake changed in one sibling only (a withdrawal staged in the boundary block).
    #[tokio::test]
    async fn sibling_boundaries_with_a_stake_change_merge() {
        siblings_resolve_to_one(Body::Withdraw, 1, Body::Nothing, 1).await;
    }

    /// One sibling carried a deploy, so its reward pot - and the committed rewards - differ.
    #[tokio::test]
    async fn sibling_boundaries_with_different_pots_merge() {
        siblings_resolve_to_one(Body::Deploy(9), 1, Body::Nothing, 1).await;
    }

    /// Two concurrent blocks off a boundary, each charging a different deployer the same 500 of
    /// phlo: both write the staking vault's balance as `base + 500`. Keeping one value for both
    /// blocks - which is what de-duplicating equal writes does - credits the vault once for two
    /// debits and destroys 500 REV. The merge must keep REV conserved.
    #[tokio::test]
    async fn equal_concurrent_vault_writes_do_not_destroy_rev() {
        let (repo, base) = genesis().await;
        let x = play(&repo, base, EPOCH - 3, 1, Body::Deploy(8)).await;
        let y = play(&repo, base, EPOCH - 3, 1, Body::Deploy(9)).await;
        let (merged, rejected) = merge(
            &repo,
            base,
            vec![index(0xa0, base, x), index(0xb0, base, y)],
            BTreeMap::new(),
        )
        .await
        .expect("the merge");
        assert_eq!(rejected.len(), 1, "the two charges conflict");

        let total = |state: Blake2b256Hash| {
            let repo = repo.clone();
            async move {
                let native = NativeSystemState::new(Arc::new(InMemNativeStore::new(
                    repo.get_native_reader(state).await,
                )));
                let mut sum = i64::from(native.pos_vault_balance().await.unwrap());
                for byte in [8, 9] {
                    sum += native
                        .vault_balance(&payer(byte).1)
                        .await
                        .unwrap()
                        .map_or(0, i64::from);
                }
                sum
            }
        };
        assert_eq!(total(merged).await, total(base).await, "REV is conserved");
    }

    /// A block with a user deploy has at least two chains (the deploy's and the close-block's), and
    /// its native writes make them one unit. The merge must terminate on such a block: the
    /// dependency map is walked by traversals that do not tolerate a cycle.
    #[tokio::test]
    async fn a_native_block_with_two_chains_merges() {
        let (repo, base) = genesis().await;
        let x = play(&repo, base, EPOCH - 3, 1, Body::Deploy(8)).await;
        let mut block = index(0xa0, base, x);
        let mut second = (*block.deploy_chains[0]).clone();
        second.deploys_with_cost = BTreeSet::from([DeployIdWithCost {
            id: vec![0xa1],
            cost: 0,
        }]);
        block.deploy_chains.push(Arc::new(second));
        let merged = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            merge(&repo, base, vec![block], BTreeMap::new()),
        )
        .await
        .expect("the merge terminates")
        .expect("the merge");
        assert!(merged.1.is_empty(), "nothing conflicts with a lone block");
    }

    /// A rejected native block loses every chain, including a second one: its user deploy's
    /// effects go with the boundary it rode in on.
    #[tokio::test]
    async fn a_rejected_native_block_loses_every_chain() {
        let (repo, base) = genesis().await;
        let a = play(&repo, base, EPOCH, 1, Body::Nothing).await;
        let b = play(&repo, base, EPOCH, 2, Body::Nothing).await;
        let with_second_chain = |mut block: BlockIndex, id: u8| {
            let mut second = (*block.deploy_chains[0]).clone();
            second.deploys_with_cost = BTreeSet::from([DeployIdWithCost {
                id: vec![id],
                cost: 0,
            }]);
            block.deploy_chains.push(Arc::new(second));
            block
        };
        let (_, rejected) = merge(
            &repo,
            base,
            vec![
                with_second_chain(index(0xa0, base, a), 0xa1),
                with_second_chain(index(0xb0, base, b), 0xb1),
            ],
            BTreeMap::new(),
        )
        .await
        .expect("sibling boundary blocks with two chains each merge");
        let a_ids = BTreeSet::from([vec![0xa0], vec![0xa1]]);
        let b_ids = BTreeSet::from([vec![0xb0], vec![0xb1]]);
        assert!(
            rejected == a_ids || rejected == b_ids,
            "exactly one block is rejected, with every chain it carried: {rejected:?}"
        );
    }

    /// The same two charges, but the second block has seen the first: its vault balance already
    /// includes the first charge, so both blocks are kept and the descendant's value is the state.
    #[tokio::test]
    async fn an_ancestor_and_descendant_writing_one_key_both_merge() {
        let (repo, base) = genesis().await;
        let x = play(&repo, base, EPOCH - 3, 1, Body::Deploy(8)).await;
        let after_x = repo
            .reset(base)
            .await
            .unwrap()
            .do_checkpoint_with_native(&[], &x)
            .await
            .unwrap()
            .root();
        let y = play(&repo, after_x, EPOCH - 2, 1, Body::Deploy(9)).await;
        let ancestry = BTreeMap::from([(block_hash(0xb0), BTreeSet::from([block_hash(0xa0)]))]);
        let (merged, rejected) = merge(
            &repo,
            base,
            vec![index(0xa0, base, x), index(0xb0, after_x, y)],
            ancestry,
        )
        .await
        .expect("the merge");
        assert!(rejected.is_empty(), "a chain of writers is not a conflict");

        let native = NativeSystemState::new(Arc::new(InMemNativeStore::new(
            repo.get_native_reader(merged).await,
        )));
        let base_native = NativeSystemState::new(Arc::new(InMemNativeStore::new(
            repo.get_native_reader(base).await,
        )));
        assert_eq!(
            i64::from(native.pos_vault_balance().await.unwrap()),
            i64::from(base_native.pos_vault_balance().await.unwrap()) + 1000,
            "both charges reach the staking vault"
        );
    }
}

#[cfg(test)]
mod index_stats_tests {
    use super::{BlockIndex, IndexStats};

    /// The progress line must name every counter, because that line is the only place the numbers
    /// become visible (#60) - a summary that silently dropped one would hide it again.
    #[test]
    fn the_summary_names_every_counter() {
        let stats = IndexStats {
            calls: 3,
            replay_fallbacks: 1,
            replay_millis: 42,
            replay_save_failures: 1,
            cache_len: 3,
            cache_pruned: 2,
            cache_capacity_evicted: 4,
        };
        let line = stats.summary();
        for needle in [
            "3 blocks indexed",
            "1 replay fallbacks",
            "42 ms",
            "1 not persisted",
            "index cache 3 entries",
            "2 pruned",
            "4 capacity evicted",
        ] {
            assert!(
                line.contains(needle),
                "summary {line:?} must name {needle:?}"
            );
        }
    }

    /// `read` is a snapshot of process-wide counters: a second read agrees with the first, which is
    /// the property the node's progress line relies on.
    #[test]
    fn read_reports_the_live_counters() {
        let before = IndexStats::read().calls;
        // `prune_cache` on an empty list removes nothing, so it must not move the counters.
        BlockIndex::prune_cache(&[]);
        let after = IndexStats::read();
        assert_eq!(after.calls, before);
        assert!(after.summary().contains("blocks indexed"));
    }
}
