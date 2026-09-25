//! Block DAG key-value storage (port of `casper/dag/BlockDagKeyValueStorage.scala`).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use async_trait::async_trait;

use rchain_block_storage::dag::dag_storage::{BlockDagStorage, DeployId};
use rchain_block_storage::dag::finalizer::Message;
use rchain_block_storage::dag::message_map;
use rchain_block_storage::dag::message_state::DagMessageState;
use rchain_block_storage::dag::representation::DagRepresentation;
use rchain_crypto::hash::blake2b256_hash::Blake2b256Hash;
use rchain_models::block_hash::BlockHash;
use rchain_models::block_metadata::BlockMetadata;
use rchain_models::casper::protocol::casper_message::{BlockMessage, SignedDeployData};
use rchain_models::fringe_data::FringeData;
use rchain_models::validator::Validator;
use rchain_shared::metrics::{Metrics, MetricsNop, Source};
use rchain_shared::refined::SeqNum;
use rchain_shared::typed_store::KeyValueTypedStore;

use crate::block_metadata_store::BlockMetadataStore;
use crate::merging::BlockIndex;

/// Upper bound on the number of distinct deploys held in the pending pool. Reached by the
/// `add_deploy` chokepoint so a remote flood cannot exhaust the deploy store (documented Scala
/// deviation — the Scala pool is unbounded).
pub const MAX_POOLED_DEPLOYS: usize = 10_000;

/// Build a `Message` from a `BlockMetadata` given the current message map (port of
/// `BlockDagKeyValueStorage.messageFromBlockMetadata`). Returns `None` when a justification is
/// absent from `msg_map` (the Scala `Map.apply` throws).
pub fn message_from_block_metadata(
    block: &BlockMetadata,
    msg_map: &BTreeMap<BlockHash, Message<BlockHash, Validator>>,
) -> Option<Message<BlockHash, Validator>> {
    let mut seen: BTreeSet<BlockHash> = BTreeSet::new();
    for p in &block.justifications {
        seen.extend(msg_map.get(p)?.seen.iter().copied());
    }
    seen.insert(block.block_hash);
    Some(Message {
        id: block.block_hash,
        height: block.block_num,
        sender: block.sender,
        sender_seq: block.seq_num,
        bonds_map: block.bonds_map.clone(),
        parents: block.justifications.clone(),
        fringe: block.fringe.clone(),
        seen: Arc::new(seen),
    })
}

/// The concrete block DAG storage (port of `BlockDagKeyValueStorage`). Fringe pruning (the
/// `BlockIndex` cache) and deploy-pool expiry run on finalization.
pub struct BlockDagKeyValueStorage {
    representation: tokio::sync::RwLock<Arc<DagRepresentation>>,
    lock: tokio::sync::Mutex<()>,
    block_metadata_store: Arc<BlockMetadataStore>,
    fringe_data_store: Arc<dyn KeyValueTypedStore<Blake2b256Hash, FringeData>>,
    deploy_index: Arc<dyn KeyValueTypedStore<DeployId, BlockHash>>,
    deploy_store: Arc<dyn KeyValueTypedStore<DeployId, SignedDeployData>>,
    /// Where the DAG's own gauges go (defaults to the no-op sink; the node wires its
    /// `MetricsRegistry` in). This is the attribution instrument for the structure's cost, in place
    /// of the profiler this environment cannot run (AUDIT C56's owed paragraph).
    metrics: Arc<dyn Metrics + Send + Sync>,
}

impl BlockDagKeyValueStorage {
    /// Rebuild the in-memory DAG representation from the stores (port of `BlockDagKeyValueStorage.create`).
    pub async fn create(
        block_metadata_store: Arc<BlockMetadataStore>,
        fringe_data_store: Arc<dyn KeyValueTypedStore<Blake2b256Hash, FringeData>>,
        deploy_index: Arc<dyn KeyValueTypedStore<DeployId, BlockHash>>,
        deploy_store: Arc<dyn KeyValueTypedStore<DeployId, SignedDeployData>>,
    ) -> Result<Self, String> {
        let dag_set = block_metadata_store.dag_set().await;
        let child_map = block_metadata_store.child_map_data().await;
        let height_map = block_metadata_store.height_map().await;

        let mut dag_msg_state = DagMessageState::<BlockHash, Validator>::empty();
        let mut fringe_states: BTreeMap<Blake2b256Hash, FringeData> = BTreeMap::new();
        // H-1's gate, re-applied to a *restored* store. `insert` refuses a second message by the same
        // sender reusing a `seq_num`, but this fold rebuilds the message map from persisted metadata
        // without that check, and the persisted layer never looks at `(sender, seq_num)` at all
        // (`validate_dag_state` reads the height map's contiguity). So a store that already holds a
        // fork — written by a build that predates the gate, or by any path that writes metadata
        // directly — used to restore straight into the equivocation the gate exists to keep out, and
        // the H-1 stall it prevents was live again with nothing saying so. Tracked as a map from
        // `(sender, seq_num)` to the block that claimed it rather than `insert`'s scan over
        // `msg_map`, so the restore stays O(N) (AUDIT C55).
        let mut claimed_by: BTreeMap<(Validator, SeqNum), BlockHash> = BTreeMap::new();

        for hash in height_map.values().flatten() {
            if dag_msg_state.msg_map.contains_key(hash) {
                continue;
            }
            let block = block_metadata_store.get_unchecked(hash).await?;
            let msg = message_from_block_metadata(&block, &dag_msg_state.msg_map)
                .ok_or_else(|| "justification not present in message map".to_string())?;
            if let Some(previous) = claimed_by.insert((msg.sender, msg.sender_seq), msg.id) {
                return Err(format!(
                    "equivocation detected in the stored DAG: sender {} reuses sequence number {} \
                     for blocks {} and {}",
                    rchain_shared::base16::encode(msg.sender.as_bytes()),
                    i64::from(msg.sender_seq),
                    previous.to_hex(),
                    msg.id.to_hex()
                ));
            }
            // In place: this loop folds the whole stored chain through the state, and the
            // persistent-shaped `insert_msg` clones the map (and every message's `seen` set) on
            // each step — Θ(N³) over a stored chain, which is what made a 5,844-block restart take
            // longer than the devnet waits for one (AUDIT C55).
            dag_msg_state.insert_msg_mut(&msg);
            // Keyed by the hash of the fringe — the store's own key (`FringeData.fringe_hash`), so
            // the in-memory map is a cache of the persisted one and a lookup is one hash compare
            // rather than a comparison of whole fringe sets (AUDIT C56's owed paragraph).
            let fringe_hash = FringeData::fringe_hash_of(&msg.fringe);
            // The entry API rather than `contains_key` + `insert`: the borrow is held across the
            // store read, so a fringe that is already cached costs no lookup at all.
            if let std::collections::btree_map::Entry::Vacant(entry) =
                fringe_states.entry(fringe_hash)
            {
                let fd = fringe_data_store
                    .get(&[fringe_hash])
                    .await?
                    .into_iter()
                    .next()
                    .flatten();
                if let Some(fd) = fd {
                    entry.insert(fd);
                }
            }
        }

        let representation = DagRepresentation {
            dag_set,
            child_map,
            height_map,
            dag_message_state: dag_msg_state,
            fringe_states,
        };

        Ok(BlockDagKeyValueStorage {
            representation: tokio::sync::RwLock::new(Arc::new(representation)),
            lock: tokio::sync::Mutex::new(()),
            block_metadata_store,
            fringe_data_store,
            deploy_index,
            deploy_store,
            metrics: Arc::new(MetricsNop),
        })
    }

    /// Point the DAG's gauges at a sink (the node's `MetricsRegistry`) and publish what the
    /// structure already holds. Defaults to `MetricsNop`, so a test or a tool that never attaches a
    /// sink pays nothing.
    ///
    /// The publish is on attach as well as per insert: a restart has a whole chain to report before
    /// it accepts its first block, and `/metrics` should say so.
    pub fn with_metrics(mut self, metrics: Arc<dyn Metrics + Send + Sync>) -> Self {
        self.metrics = metrics;
        // Synchronous, and no guard is held here — `try_read` rather than an await, because this is
        // the constructor path. A caller that attached the sink while a writer held the lock would
        // simply publish nothing until its next insert.
        if let Ok(representation) = self.representation.try_read() {
            self.set_gauges(
                representation.message_count(),
                representation.seen_entries(),
                representation.fringe_states.len(),
                representation.index_entries(),
                representation.logical_bytes(),
            );
        }
        self
    }

    /// Push the five gauges (no lock: the caller has the values).
    fn set_gauges(
        &self,
        messages: usize,
        seen_entries: usize,
        fringe_states: usize,
        index_entries: usize,
        logical_bytes: usize,
    ) {
        let source = Source::base().sub("dag");
        let count = |n: usize| i64::try_from(n).unwrap_or(i64::MAX);
        self.metrics.set_gauge(&source, "messages", count(messages));
        self.metrics
            .set_gauge(&source, "seen_entries", count(seen_entries));
        self.metrics
            .set_gauge(&source, "fringe_states", count(fringe_states));
        self.metrics
            .set_gauge(&source, "index_entries", count(index_entries));
        self.metrics
            .set_gauge(&source, "logical_bytes", count(logical_bytes));
    }

    /// Expire deploys from the pool whose `valid_after_block_number` is older than the deploy
    /// lifespan (port of `removeExpiredFromPool`). Without this the pool grows without bound and
    /// stale deploys are re-proposed.
    async fn expire_deploys(&self, latest_block_number: i64) -> Result<(), String> {
        let pooled = self.deploy_store.to_map().await?;
        let expired: Vec<DeployId> = pooled
            .iter()
            .filter(|(_, d)| {
                latest_block_number - d.data.valid_after_block_number
                    > crate::multi_parent_casper::DEPLOY_LIFESPAN
            })
            .map(|(id, _)| id.clone())
            .collect();
        if !expired.is_empty() {
            self.deploy_store.delete(&expired).await?;
        }
        Ok(())
    }
}

#[async_trait]
impl BlockDagStorage for BlockDagKeyValueStorage {
    async fn get_representation(&self) -> Arc<DagRepresentation> {
        // A pointer copy under a lock held for nanoseconds — this used to be a full Θ(N²) deep copy
        // with the read lock held throughout (AUDIT C56's owed paragraph).
        self.representation.read().await.clone()
    }

    async fn insert(
        &self,
        block_metadata: BlockMetadata,
        block: BlockMessage,
    ) -> Result<(), String> {
        let _guard = self.lock.lock().await;
        if self
            .block_metadata_store
            .contains(&block_metadata.block_hash)
            .await
        {
            return Ok(());
        }

        // H-1: equivocation detection — a second, distinct block by the same sender reusing a
        // `seq_num`. Reject it before any partial write so an equivocating validator can neither
        // enter the DAG nor stall finalization.
        {
            let repr = self.representation.read().await;
            let equivocating = repr.dag_message_state.msg_map.values().any(|m| {
                m.sender == block_metadata.sender && m.sender_seq == block_metadata.seq_num
            });
            if equivocating {
                return Err(
                    "equivocation detected: sender produced two blocks with the same sequence number"
                        .to_string(),
                );
            }
        }

        // Add block metadata to the index.
        self.block_metadata_store
            .add(block_metadata.clone())
            .await?;

        // Index each deploy to this block, and remove it from the pending pool so
        // `pooled_deploys` no longer lists it (a deploy leaves the pool once included).
        let deploy_hashes: Vec<DeployId> = block
            .state
            .deploys
            .iter()
            .map(|d| d.deploy.sig.clone())
            .collect();
        if !deploy_hashes.is_empty() {
            let pairs: Vec<(DeployId, BlockHash)> = deploy_hashes
                .iter()
                .map(|h| (h.clone(), block.block_hash))
                .collect();
            self.deploy_index.put(&pairs).await?;
            self.deploy_store.delete(&deploy_hashes).await?;
        }

        // Compute fringe diff and store fringe data.
        let fringe_hash = FringeData::fringe_hash_of(&block_metadata.fringe);

        // One read of the representation, and the guard is a *block expression* rather than a
        // binding: everything below is synchronous, so the copy this used to take
        // (`dag_message_state.clone()`, Θ(N²) in the messages' `seen` sets, once per block) bought
        // nothing except releasing the lock — and a binding left in scope would hold the read lock
        // across `fringe_data_store.put`, making the write at the end of this function wait on
        // itself. The borrow checker cannot catch a stale guard; the block can.
        let fringe_diff: BTreeSet<BlockHash> = {
            let repr = self.representation.read().await;
            let msg_map = &repr.dag_message_state.msg_map;
            let mut justifications: BTreeSet<Message<BlockHash, Validator>> = BTreeSet::new();
            for j in &block_metadata.justifications {
                let msg = msg_map
                    .get(j)
                    .ok_or_else(|| "justification not present in message map".to_string())?;
                justifications.insert(msg.clone());
            }
            let prev_fringe = message_map::latest_fringe(msg_map, &justifications);
            let mut fringe_seen: BTreeSet<BlockHash> = BTreeSet::new();
            for f in &block_metadata.fringe {
                let msg = msg_map
                    .get(f)
                    .ok_or_else(|| "fringe block not present in message map".to_string())?;
                fringe_seen.extend(msg.seen.iter().copied());
            }
            let prev_seen: BTreeSet<BlockHash> = prev_fringe
                .iter()
                .flat_map(|m| m.seen.iter().copied())
                .collect();
            fringe_seen.difference(&prev_seen).copied().collect()
        };

        let fringe_data = FringeData {
            fringe_hash,
            fringe: block_metadata.fringe.clone(),
            fringe_diff: fringe_diff.clone(),
            state_hash: Blake2b256Hash::from_byte_array(
                block_metadata.fringe_state_hash.as_bytes(),
            ),
            rejected_deploys: block.rejected_deploys.clone(),
            rejected_blocks: block.rejected_blocks.clone(),
            rejected_senders: block.rejected_senders.clone(),
        };
        self.fringe_data_store
            .put(&[(fringe_hash, fringe_data.clone())])
            .await?;

        // Mark the newly-finalized blocks' metadata with their member fringe.
        for h in &fringe_diff {
            let meta = self.block_metadata_store.get_unchecked(h).await?;
            let updated = BlockMetadata {
                member_of_fringe: Some(fringe_hash),
                ..meta
            };
            self.block_metadata_store.add(updated).await?;
        }

        // Update the in-memory DAG representation.
        let dag_set = self.block_metadata_store.dag_set().await;
        let child_map = self.block_metadata_store.child_map_data().await;
        let height_map = self.block_metadata_store.height_map().await;
        let mut prune_cache_ids: Vec<BlockHash> = Vec::new();
        let latest_block_number: i64;
        {
            // The guard is bound so that `Arc::make_mut` has somewhere to borrow the `&mut` from; it
            // is dropped at the end of this block, before the awaits below. `make_mut` copies only
            // when a reader still holds a snapshot, and this function is serialized by `self.lock`,
            // so that is at most one copy per insert — where every reader used to pay one.
            let mut guard = self.representation.write().await;
            let repr = Arc::make_mut(&mut *guard);
            let msg = message_from_block_metadata(&block_metadata, &repr.dag_message_state.msg_map)
                .ok_or_else(|| "justification not present in message map".to_string())?;
            // H-2: a validation-failed block is recorded in the map (for `neglectedInvalidBlock`
            // and justification-regression) but must not become the sender's latest message.
            if block_metadata.validation_failed {
                repr.dag_message_state.insert_msg_without_latest_mut(&msg);
            } else {
                repr.dag_message_state.insert_msg_mut(&msg);
            }
            repr.fringe_states.insert(fringe_hash, fringe_data);
            repr.dag_set = dag_set;
            repr.child_map = child_map;
            repr.height_map = height_map;

            // H-4: when finalization advanced, collect the block-index cache entries prunable
            // below the newly-finalized fringe.
            if !fringe_diff.is_empty() {
                let msg_map = &repr.dag_message_state.msg_map;
                let latest_msgs: BTreeSet<_> = repr
                    .dag_message_state
                    .latest_msgs
                    .values()
                    .cloned()
                    .collect();
                let lowest = message_map::lowest_fringe(msg_map, &latest_msgs);
                let lowest_ids: BTreeSet<BlockHash> = lowest.iter().map(|m| m.id).collect();
                let prunable = message_map::prune_fringe(msg_map, &lowest_ids, &repr.child_map);
                prune_cache_ids = prunable.iter().map(|m| m.id).collect();
            }
            latest_block_number = repr.latest_block_number();
            // Published here, from the representation the write guard already holds — a second read
            // lock would deadlock against the guard this block is inside of.
            self.set_gauges(
                repr.message_count(),
                repr.seen_entries(),
                repr.fringe_states.len(),
                repr.index_entries(),
                repr.logical_bytes(),
            );
        }

        BlockIndex::prune_cache(&prune_cache_ids);
        self.expire_deploys(latest_block_number).await?;
        Ok(())
    }

    async fn lookup(&self, block_hash: &BlockHash) -> Result<Option<BlockMetadata>, String> {
        self.block_metadata_store.get(block_hash).await
    }

    async fn lookup_by_deploy_id(&self, deploy_id: &DeployId) -> Result<Option<BlockHash>, String> {
        let vals = self.deploy_index.get(&[deploy_id.clone()]).await?;
        Ok(vals.into_iter().next().flatten())
    }

    async fn add_deploy(&self, deploy: SignedDeployData) -> Result<(), String> {
        // Soft DoS bound: reject before writing once the pool reaches `MAX_POOLED_DEPLOYS`
        // distinct deploys (documented Scala deviation — the Scala pool is unbounded).
        if self.deploy_store.count().await? >= MAX_POOLED_DEPLOYS {
            return Err("deploy pool is full".to_string());
        }
        self.deploy_store
            .put(&[(deploy.sig.clone(), deploy)])
            .await?;
        Ok(())
    }

    async fn pooled_deploys(&self) -> Result<BTreeMap<DeployId, SignedDeployData>, String> {
        self.deploy_store.to_map().await
    }

    async fn contains_deploy_in_pool(&self, deploy_id: &DeployId) -> Result<bool, String> {
        let vals = self.deploy_store.contains(&[deploy_id.clone()]).await?;
        Ok(vals.into_iter().next().unwrap_or(false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rchain_block_storage::dag::codecs::{
        Blake2b256HashCodec, BlockHashCodec, BlockMetadataCodec, FringeDataCodec,
        SignedDeployDataCodec,
    };
    use rchain_models::block::state_hash::StateHash;
    use rchain_shared::refined::{BlockHeight, NonNegI64, SeqNum};
    use rchain_shared::store::{InMemoryKeyValueStore, KeyValueStore};
    use rchain_shared::typed_store::{BytesCodec, KeyValueTypedStoreCodec};

    type Shared = Arc<tokio::sync::Mutex<Box<dyn KeyValueStore + Send + Sync>>>;

    fn in_memory() -> Shared {
        Arc::new(tokio::sync::Mutex::new(Box::new(
            InMemoryKeyValueStore::default(),
        )))
    }

    fn hash(byte: u8) -> BlockHash {
        let mut bytes = [0u8; 32];
        bytes[0] = byte;
        BlockHash::new(bytes)
    }

    fn meta(hash: BlockHash, parents: &[BlockHash], block_num: i64) -> BlockMetadata {
        BlockMetadata {
            block_hash: hash,
            block_num: BlockHeight::try_from(block_num).unwrap(),
            sender: Validator::new([0u8; 65]),
            seq_num: 0.try_into().unwrap(),
            justifications: parents.iter().copied().collect(),
            bonds_map: BTreeMap::new(),
            validated: true,
            validation_failed: false,
            fringe: BTreeSet::new(),
            fringe_state_hash: StateHash::new([0u8; 32]),
            member_of_fringe: None,
        }
    }

    fn block(hash: BlockHash) -> BlockMessage {
        BlockMessage {
            version: 1,
            shard_id: "root".to_string(),
            block_hash: hash,
            block_number: 0.try_into().unwrap(),
            sender: Validator::new([0u8; 65]),
            seq_num: 0.try_into().unwrap(),
            pre_state_hash: StateHash::new([0u8; 32]),
            post_state_hash: StateHash::new([0u8; 32]),
            justifications: vec![],
            bonds: BTreeMap::new(),
            rejected_deploys: BTreeSet::new(),
            rejected_blocks: BTreeSet::new(),
            rejected_senders: BTreeSet::new(),
            state: rchain_models::casper::protocol::casper_message::RholangState::default(),
            sig_algorithm: "secp256k1".to_string(),
            sig: vec![],
            timestamp: 0,
        }
    }

    async fn empty_metadata_store() -> Arc<BlockMetadataStore> {
        Arc::new(
            BlockMetadataStore::create(Arc::new(KeyValueTypedStoreCodec::new(
                in_memory(),
                Arc::new(BlockHashCodec),
                Arc::new(BlockMetadataCodec),
            )))
            .await
            .unwrap(),
        )
    }

    async fn build_storage_over(
        metadata_store: Arc<BlockMetadataStore>,
    ) -> Arc<BlockDagKeyValueStorage> {
        Arc::new(build_storage_over_unshared(metadata_store).await)
    }

    /// The same storage, not yet wrapped in an `Arc` — so a test can attach a metrics sink
    /// (`with_metrics` consumes it) the way the node does.
    async fn build_storage_over_unshared(
        metadata_store: Arc<BlockMetadataStore>,
    ) -> BlockDagKeyValueStorage {
        let fringe_store: Arc<dyn KeyValueTypedStore<Blake2b256Hash, FringeData>> =
            Arc::new(KeyValueTypedStoreCodec::new(
                in_memory(),
                Arc::new(Blake2b256HashCodec),
                Arc::new(FringeDataCodec),
            ));
        let deploy_index: Arc<dyn KeyValueTypedStore<DeployId, BlockHash>> =
            Arc::new(KeyValueTypedStoreCodec::new(
                in_memory(),
                Arc::new(BytesCodec),
                Arc::new(BlockHashCodec),
            ));
        let deploy_store: Arc<dyn KeyValueTypedStore<DeployId, SignedDeployData>> =
            Arc::new(KeyValueTypedStoreCodec::new(
                in_memory(),
                Arc::new(BytesCodec),
                Arc::new(SignedDeployDataCodec),
            ));
        BlockDagKeyValueStorage::create(metadata_store, fringe_store, deploy_index, deploy_store)
            .await
            .unwrap()
    }

    async fn build_storage() -> Arc<BlockDagKeyValueStorage> {
        build_storage_over(empty_metadata_store().await).await
    }

    #[tokio::test]
    async fn insert_and_lookup_round_trip() {
        let storage = build_storage().await;
        let genesis_hash = hash(0);
        storage
            .insert(meta(genesis_hash, &[], 0), block(genesis_hash))
            .await
            .unwrap();

        assert_eq!(
            storage
                .lookup(&genesis_hash)
                .await
                .unwrap()
                .unwrap()
                .block_hash,
            genesis_hash
        );
        assert!(storage.lookup(&hash(9)).await.unwrap().is_none());
        assert!(storage.get_representation().await.contains(&genesis_hash));
    }

    #[tokio::test]
    async fn insert_rejects_equivocation_same_seq_num() {
        let storage = build_storage().await;
        let first = hash(1);
        let second = hash(2);

        storage
            .insert(meta(first, &[], 0), block(first))
            .await
            .unwrap();

        // A second, distinct block by the same sender reusing `seq_num` is rejected (H-1), before
        // any partial write.
        let err = storage.insert(meta(second, &[], 0), block(second)).await;
        assert!(err.is_err(), "equivocation must be rejected");
        assert!(
            err.unwrap_err().contains("equivocation"),
            "error should mention equivocation"
        );

        // The first block's metadata is still present and unchanged.
        let stored = storage.lookup(&first).await.unwrap().unwrap();
        assert_eq!(stored.block_hash, first);
    }

    #[tokio::test]
    async fn deploy_pool_round_trip() {
        let storage = build_storage().await;
        let deploy = SignedDeployData {
            data: rchain_models::casper::protocol::casper_message::DeployData {
                attachments: Vec::new(),
                term: "Nil".to_string(),
                timestamp: 0,
                phlo_price: 1,
                phlo_limit: 1,
                valid_after_block_number: 0,
                shard_id: "root".to_string(),
            },
            deployer: vec![1, 2, 3],
            sig: vec![9, 9, 9],
            sig_algorithm: "secp256k1".to_string(),
        };
        storage.add_deploy(deploy.clone()).await.unwrap();
        assert!(storage.contains_deploy_in_pool(&deploy.sig).await.unwrap());
        assert_eq!(
            storage.lookup_by_deploy_id(&deploy.sig).await.unwrap(),
            None
        );

        let pooled = storage.pooled_deploys().await.unwrap();
        assert_eq!(pooled[&deploy.sig], deploy);
    }

    #[tokio::test]
    async fn insert_removes_included_deploy_from_pool() {
        let storage = build_storage().await;
        let signed = deploy_with_id(7);
        storage.add_deploy(signed.clone()).await.unwrap();
        assert!(storage.contains_deploy_in_pool(&signed.sig).await.unwrap());

        let processed = rchain_models::casper::protocol::casper_message::ProcessedDeploy {
            deploy: signed.clone(),
            cost: rchain_models::casper::protocol::casper_message::PCost { cost: 0 },
            deploy_log: vec![],
            is_failed: false,
            system_deploy_error: None,
        };
        let block_hash = hash(1);
        let mut b = block(block_hash);
        b.state.deploys = vec![processed];
        storage.insert(meta(block_hash, &[], 1), b).await.unwrap();

        assert!(
            !storage.contains_deploy_in_pool(&signed.sig).await.unwrap(),
            "an included deploy must leave the pool"
        );
        assert_eq!(
            storage.lookup_by_deploy_id(&signed.sig).await.unwrap(),
            Some(block_hash)
        );
        assert!(storage.pooled_deploys().await.unwrap().is_empty());
    }

    fn deploy_with_id(id: usize) -> SignedDeployData {
        SignedDeployData {
            data: rchain_models::casper::protocol::casper_message::DeployData {
                attachments: Vec::new(),
                term: "Nil".to_string(),
                timestamp: 0,
                phlo_price: 1,
                phlo_limit: 1,
                valid_after_block_number: 0,
                shard_id: "root".to_string(),
            },
            deployer: vec![0],
            sig: id.to_le_bytes().to_vec(),
            sig_algorithm: "secp256k1".to_string(),
        }
    }

    #[tokio::test]
    async fn add_deploy_rejects_when_pool_full() {
        let storage = build_storage().await;
        for id in 0..MAX_POOLED_DEPLOYS {
            storage.add_deploy(deploy_with_id(id)).await.unwrap();
        }
        let err = storage.add_deploy(deploy_with_id(MAX_POOLED_DEPLOYS)).await;
        assert!(err.is_err(), "deploy pool must reject once full");
        assert!(err.unwrap_err().contains("full"));
    }

    fn chain_hash(i: usize) -> BlockHash {
        let mut bytes = [0u8; 32];
        bytes[..8].copy_from_slice(&(i as u64).to_le_bytes());
        BlockHash::new(bytes)
    }

    /// Store a chain of `n` blocks: block `i` sits at height `i` and justifies block `i - 1`, so
    /// its `seen` set is its whole ancestry — which is what a restore has to fold.
    async fn store_chain(store: &BlockMetadataStore, n: usize) {
        let mut hashes: Vec<BlockHash> = Vec::with_capacity(n);
        for i in 0..n {
            let h = chain_hash(i);
            let parents: Vec<BlockHash> = if i == 0 {
                Vec::new()
            } else {
                vec![hashes[i - 1]]
            };
            // One sender, a strictly increasing `seq_num` — the shape H-1 and `sequence_number`
            // require of a chain a validator actually produced. The plain `meta` helper reuses
            // `seq_num 0` for every block, which is a *fork* by those rules: legal to store (the
            // persisted layer checks only that the height map is contiguous) and exactly what
            // `create`'s H-1 re-check refuses on restore — which is the point of that re-check, so
            // this fixture had to become a state the port admits.
            store
                .add(meta_by(0, i as i64, h, &parents, i as i64))
                .await
                .unwrap();
            hashes.push(h);
        }
    }

    /// AUDIT C56's owed paragraph, first half — **reading** the DAG must not copy it.
    ///
    /// `get_representation` returned `DagRepresentation` **by value**, so every caller paid a deep
    /// copy of `msg_map`: N messages, each carrying its whole ancestor set behind `seen`, i.e.
    /// Σ|seen| × 32 B ≈ 23 MB *per read* at N=1,200 and ≈ 550 MB at the 5,844-block devnet chain —
    /// with the read lock held for the whole copy. Roughly 35 production sites read it, including
    /// every per-block path and every peer-request handler, which is what grew a peer-serving node
    /// by ~0.86 GiB per block. A reader now takes a refcount (`Arc<DagRepresentation>`).
    ///
    /// A cost bound, not a semantic one: the value is identical either way, which is why no test of
    /// behaviour could see it.
    ///
    /// **Two instruments, because one did not survive its own falsification (2026-09-24).** The
    /// timing bound alone was calibrated against the *pre-Stage-3* tree — the doc-comment figure of
    /// 27.3 s is that artifact, where `Message.seen` was still a plain `BTreeSet`. With `seen` shared
    /// the same restoration costs **459 ms**, which a 5 s bound clears, so the tripwire passed with
    /// the copy put back. It now asserts the *mechanism* as well: every read must hand back the same
    /// allocation (`Arc::ptr_eq`), which is exactly "the DAG was not rebuilt" and is deterministic —
    /// immune to machine load, and 0.17 ms measured against 459 ms, where a timing bound separating
    /// them by 2,700× would have to sit close enough to the floor to be flaky. The 5 s bound stays
    /// for the slower regression: the *whole* pre-pass shape (copying and unshared `seen`) is the
    /// 27.3 s case, which both instruments catch. `--nocapture` prints the window.
    #[tokio::test]
    async fn reading_the_dag_representation_does_not_copy_the_message_state() {
        const N: usize = 1200;
        const READS: usize = 200;
        const BOUND: std::time::Duration = std::time::Duration::from_secs(5);

        let metadata_store = empty_metadata_store().await;
        store_chain(&metadata_store, N).await;
        let storage = build_storage_over(metadata_store).await;

        // No insert runs here, so the stored `Arc` cannot be replaced under us: every read of an
        // unchanged DAG must return that same allocation.
        let first = storage.get_representation().await;
        let started = std::time::Instant::now();
        for _ in 0..READS {
            let repr = storage.get_representation().await;
            // The read did happen, and returned the whole DAG — so the tripwire cannot pass by
            // reading something smaller.
            assert_eq!(repr.dag_message_state.msg_map.len(), N);
            assert!(
                Arc::ptr_eq(&first, &repr),
                "a read of a {N}-block DAG returned a different allocation than the previous one: \
                 `get_representation` is rebuilding the representation instead of sharing it"
            );
        }
        let elapsed = started.elapsed();
        println!("{READS} representation reads at N={N}: {elapsed:?}");

        assert!(
            elapsed < BOUND,
            "{READS} representation reads of a {N}-block DAG took {elapsed:?} (bound {BOUND:?}): the \
             DAG is being copied per read instead of shared"
        );
    }

    /// AUDIT C56's owed paragraph, second half — **adding** a block must not copy the DAG either.
    ///
    /// `insert` cloned `dag_message_state` once per block (again Θ(N) messages, though no longer
    /// Θ(N) each) purely to avoid holding the read guard across the awaits further down; everything
    /// it needed from the state is synchronous, so the copy bought nothing. The guard is now scoped
    /// to the synchronous region and the write path takes `Arc::make_mut`, which copies only while a
    /// reader holds a snapshot — at most once per insert, serialized by the storage's own lock.
    ///
    /// **What this test can and cannot see (measured 2026-09-24).** The transient copy — HEAD's
    /// `self.representation.read().await.dag_message_state.clone()`, restored verbatim — costs
    /// **2.2 ms per insert** at N=1,200 against the insert's own 4.6 ms: 50 inserts take **339 ms
    /// copying against 230 ms fixed**. A 1.47× ratio cannot carry a timing bound, and the 3 s bound
    /// it used to carry did not catch it. The ratio is small *because* Stage 3 landed: sharing
    /// `seen` turned the copy into N refcount bumps, so the doc-comment figure of 7.44 s describes
    /// the pre-Stage-3 tree rather than this falsifier. And with no allocation counter available (a
    /// `#[global_allocator]` needs `unsafe`, which this crate graph does not admit) that transient
    /// copy leaves no trace a test can read — only its cost, which nothing here measures.
    ///
    /// So the assertion is the mechanism's *persistent* half, which is deterministic: with no reader
    /// holding a snapshot, `insert` must take `Arc::make_mut`'s no-copy path, leaving the
    /// representation at the same allocation. An unconditional `Arc::new((**guard).clone())` — the
    /// plausible way to reintroduce the per-block copy — fails here. The 5 s bound remains as a
    /// gross-regression smoke bound (the full pre-pass shape lands in the seconds, and C55's own
    /// tripwire covers the cubic one); it is deliberately not tight enough to be the copy detector,
    /// and is not claimed to be. The per-*read* half is pinned deterministically by
    /// `reading_the_dag_representation_does_not_copy_the_message_state`.
    #[tokio::test]
    async fn adding_a_block_does_not_copy_the_message_state() {
        const N: usize = 1200;
        const K: usize = 50;
        const BOUND: std::time::Duration = std::time::Duration::from_secs(5);

        let metadata_store = empty_metadata_store().await;
        store_chain(&metadata_store, N).await;
        let storage = build_storage_over(metadata_store).await;

        // `as_ptr` off a temporary, so the read does not itself hold a snapshot across the inserts
        // (which would force `make_mut` to copy, legitimately).
        let before = Arc::as_ptr(&storage.get_representation().await);

        // A distinct `seq_num` per block, continuing the stored chain's sequence: `store_chain` now
        // gives its blocks sender `[0; 65]` with seq `0..N`, and a second block from the same sender
        // at the same seq is an equivocation (H-1) — refused at ingress and on restore alike.
        let mut tip = chain_hash(N - 1);
        let started = std::time::Instant::now();
        for k in 0..K {
            let next = chain_hash(N + k);
            let mut m = meta(next, &[tip], (N + k) as i64);
            m.seq_num = ((N + k) as i64).try_into().unwrap();
            storage.insert(m, block(next)).await.unwrap();
            tip = next;
        }
        let elapsed = started.elapsed();
        println!("{K} inserts at N={N}: {elapsed:?}");

        let repr = storage.get_representation().await;
        assert_eq!(repr.dag_message_state.msg_map.len(), N + K);
        assert_eq!(
            before,
            Arc::as_ptr(&repr),
            "inserting into an unread DAG replaced the representation: the message state is being \
             copied per block instead of extended in place"
        );

        assert!(
            elapsed < BOUND,
            "{K} inserts into a {N}-block DAG took {elapsed:?} (bound {BOUND:?})"
        );
    }

    /// A metrics sink that records what the DAG publishes, so the gauges can be asserted rather than
    /// eyeballed on a live `/metrics`.
    #[derive(Default)]
    struct RecordingMetrics {
        gauges: std::sync::Mutex<BTreeMap<(String, String), i64>>,
    }

    impl RecordingMetrics {
        fn gauge(&self, source: &str, name: &str) -> Option<i64> {
            self.gauges
                .lock()
                .unwrap()
                .get(&(source.to_string(), name.to_string()))
                .copied()
        }
    }

    impl rchain_shared::metrics::Metrics for RecordingMetrics {
        fn increment_counter(&self, _s: &rchain_shared::metrics::Source, _n: &str, _d: i64) {}
        fn increment_sampler(&self, _s: &rchain_shared::metrics::Source, _n: &str, _d: i64) {}
        fn sample(&self, _s: &rchain_shared::metrics::Source, _n: &str) {}
        fn set_gauge(&self, source: &rchain_shared::metrics::Source, name: &str, value: i64) {
            self.gauges
                .lock()
                .unwrap()
                .insert((source.0.clone(), name.to_string()), value);
        }
        fn increment_gauge(&self, _s: &rchain_shared::metrics::Source, _n: &str, _d: i64) {}
        fn decrement_gauge(&self, _s: &rchain_shared::metrics::Source, _n: &str, _d: i64) {}
        fn record(&self, _s: &rchain_shared::metrics::Source, _n: &str, _v: i64, _c: i64) {}
    }

    /// AUDIT C56's owed paragraph, third part — the DAG index is held **once**: the representation's
    /// `dag_set`/`child_map`/`height_map` are the store's own allocations.
    ///
    /// `BlockMetadataStore`'s accessors used to hand out a full clone per call, and `insert` called
    /// all three on every block, so the index lived twice and was rebuilt per insert. `Arc`-sharing
    /// it is a *value-identical* change (the digest test above pins that), so the assertion is the
    /// mechanism: pointer identity between the store's index and the representation's, which is
    /// exactly "one index, one update point".
    ///
    /// Falsified against this tree (2026-09-24): returning a fresh `Arc::new((*map).clone())` from the
    /// accessors — the plausible way to reintroduce the second copy while keeping the type — fails
    /// the first `ptr_eq`. The old shape (a plain `BTreeSet` field) cannot even be compared this way,
    /// which is why the mechanism, not the bound, is the tripwire.
    #[tokio::test]
    async fn the_index_is_shared_with_the_representation_not_copied() {
        let metadata_store = empty_metadata_store().await;
        store_chain(&metadata_store, 8).await;
        let storage = build_storage_over(metadata_store.clone()).await;
        let repr = storage.get_representation().await;

        assert!(
            Arc::ptr_eq(&metadata_store.dag_set().await, &repr.dag_set),
            "the representation's dag_set is the store's allocation, not a copy of it"
        );
        assert!(
            Arc::ptr_eq(&metadata_store.child_map_data().await, &repr.child_map),
            "the representation's child_map is the store's allocation"
        );
        assert!(
            Arc::ptr_eq(&metadata_store.height_map().await, &repr.height_map),
            "the representation's height_map is the store's allocation"
        );
        // …and it is the same *index*, not merely a shared empty one.
        assert_eq!(repr.dag_set.len(), 8);
        assert_eq!(repr.child_map.len(), 8);
        assert_eq!(repr.height_map.len(), 8);

        // An insert keeps them shared: the store's new index is the representation's, while the
        // snapshot taken above keeps the old one (copy-on-write, which is what makes the read path
        // cheap and the writer's copy at most one per insert).
        let mut m = meta(chain_hash(8), &[chain_hash(7)], 8);
        m.seq_num = 8.try_into().unwrap();
        storage.insert(m, block(chain_hash(8))).await.unwrap();
        let after = storage.get_representation().await;
        assert!(
            Arc::ptr_eq(&metadata_store.dag_set().await, &after.dag_set),
            "an insert moves the pointer, it does not replace the index"
        );
        assert_eq!(after.dag_set.len(), 9);
        assert_eq!(
            repr.dag_set.len(),
            8,
            "the snapshot taken before the insert keeps the index it read"
        );
    }

    /// Stage 6's observability claim — the DAG publishes its own gauges, and `logical_bytes` is the
    /// number the residency claims are stated in.
    ///
    /// The assertion is the published gauge *equals* the representation's own accounting (so the
    /// instrument cannot report a number the structure does not have) and that it moves when the DAG
    /// does (so a frozen gauge is not mistaken for a flat cost). Falsified against this tree: with the
    /// `set_gauges` call removed — the shape before this stage, where nothing published anything and
    /// `/metrics` served the reporter's placeholder — every `gauge(..)` below is `None`.
    #[tokio::test]
    async fn the_dag_publishes_its_own_gauges() {
        let metrics = Arc::new(RecordingMetrics::default());
        let metadata_store = empty_metadata_store().await;
        store_chain(&metadata_store, 4).await;
        let storage = Arc::new(
            build_storage_over_unshared(metadata_store)
                .await
                .with_metrics(metrics.clone()),
        );

        // Attaching the sink published what `create` rebuilt — a restart reports its whole chain
        // before it accepts its first block.
        assert_eq!(metrics.gauge("rchain.dag", "messages"), Some(4));

        // `seq` continues the stored chain's sender sequence (see `store_chain`), and `i` the height.
        for (seq, i) in [(4i64, 4usize), (5, 5)] {
            let mut m = meta(chain_hash(i), &[chain_hash(i - 1)], i as i64);
            m.seq_num = seq.try_into().unwrap();
            storage.insert(m, block(chain_hash(i))).await.unwrap();
        }

        let repr = storage.get_representation().await;
        let expected = i64::try_from(repr.logical_bytes()).expect("fits i64");
        assert_eq!(metrics.gauge("rchain.dag", "messages"), Some(6));
        assert_eq!(
            metrics.gauge("rchain.dag", "seen_entries"),
            Some(i64::try_from(repr.seen_entries()).expect("fits i64")),
            "the published seen-entry count is the representation's own"
        );
        assert_eq!(
            metrics.gauge("rchain.dag", "logical_bytes"),
            Some(expected),
            "the published accounting is the representation's own"
        );
        assert_eq!(
            metrics.gauge("rchain.dag", "index_entries"),
            Some(i64::try_from(repr.index_entries()).expect("fits i64"))
        );
        // Every block in this chain declares the empty fringe, so the DAG caches exactly one
        // fringe state — the gauge counts entries, not fringes ever seen.
        assert_eq!(metrics.gauge("rchain.dag", "fringe_states"), Some(1));
        assert!(
            expected > 0 && repr.seen_entries() > 0,
            "a non-empty DAG has a non-zero account"
        );
        // The negative control for a representation-only change, in exact units: this chain's account
        // is a fixed number. Stage 5's re-keying of `fringe_states`, its `Arc`-ing of the index and
        // this stage's own gauges must not move it — the accounting reads the *messages*, so a change
        // that moved this value would be a change to the DAG rather than to how it is held.
        assert_eq!(
            repr.logical_bytes(),
            2032,
            "the chain's logical bytes are a fixed value, not a function of how the DAG is held"
        );
        assert_eq!(
            repr.seen_entries(),
            21,
            "and so is its seen-entry count (Σ|seen| = 21 for this chain)"
        );

        // It moves: one more block, and `logical_bytes` and `seen_entries` grow (a chain's `seen` is
        // its whole ancestry). `seq 6` continues the same sender's sequence (stored `0..4`, inserted
        // `4`, `5`), rather than reusing a number the chain already spent.
        let mut m = meta(chain_hash(6), &[chain_hash(5)], 6);
        m.seq_num = 6.try_into().unwrap();
        storage.insert(m, block(chain_hash(6))).await.unwrap();
        assert!(
            metrics
                .gauge("rchain.dag", "logical_bytes")
                .expect("published")
                > expected,
            "the account grows with the DAG"
        );
        assert_eq!(metrics.gauge("rchain.dag", "messages"), Some(7));
    }

    /// A canonical digest of the representation's **value**: every message and every fringe datum,
    /// independent of how the maps are keyed or whether they are shared. The fringe data is digested
    /// *sorted on `fringe_hash`*, so re-keying `fringe_states` (5a) or `Arc`-ing the index (5c)
    /// cannot move the digest — only a change to the data can.
    ///
    /// This is the negative control the pass's own rule asks for on a representation-only change, and
    /// it is falsifiable in both directions: it *moves* when the value moves (below), so a constant
    /// digest is evidence rather than a tautology.
    fn representation_digest(repr: &Arc<DagRepresentation>) -> String {
        let mut parts: Vec<String> = Vec::new();
        for (id, m) in &repr.dag_message_state.msg_map {
            parts.push(format!(
                "m {} h{} s{} p{} f{} seen{}",
                id.to_hex(),
                i64::from(m.height),
                i64::from(m.sender_seq),
                m.parents.len(),
                m.fringe.len(),
                m.seen.len()
            ));
        }
        parts.push(format!(
            "index {} {} {}",
            repr.dag_set.len(),
            repr.child_map.len(),
            repr.height_map.len()
        ));
        let mut fringes: Vec<&rchain_models::fringe_data::FringeData> =
            repr.fringe_states.values().collect();
        fringes.sort_by_key(|fd| fd.fringe_hash);
        for fd in fringes {
            parts.push(format!(
                "f {} n{} d{} r{}",
                fd.fringe_hash.to_hex(),
                fd.fringe.len(),
                fd.fringe_diff.len(),
                fd.rejected_deploys.len()
            ));
        }
        let refs: Vec<&[u8]> = parts.iter().map(|p| p.as_bytes()).collect();
        Blake2b256Hash::create_many(&refs).to_hex()
    }

    /// The digest of a stored chain is a fixed value, and it moves when the DAG does — the negative
    /// control for a representation-only change (AUDIT C56's owed paragraph).
    ///
    /// `the_representation_digest` is computed from a two-block chain; the assert on the second half
    /// is what makes the first half evidence: inserting a block changes the digest, so a digest that
    /// did not move between two trees would mean the DAG's value really is unchanged.
    #[tokio::test]
    async fn the_representation_digest_pins_the_value_and_moves_with_it() {
        let metadata_store = empty_metadata_store().await;
        store_chain(&metadata_store, 2).await;
        let storage = build_storage_over(metadata_store).await;
        // Continuing the stored chain's one sender: `store_chain` gives each of its blocks a strictly
        // increasing `seq_num`, so the continuation continues *that* sequence and takes the next
        // height (`seq`, i.e. parent + 1), rather than restarting at `seq 1` on top of a fixture that
        // used to reuse `seq 0` throughout.
        for (seq, h) in [(2i64, chain_hash(2)), (3, chain_hash(3))] {
            let mut m = meta(h, &[chain_hash(usize::try_from(seq - 1).unwrap())], seq);
            m.seq_num = seq.try_into().unwrap();
            storage.insert(m, block(h)).await.unwrap();
        }

        let before = representation_digest(&storage.get_representation().await);
        assert_eq!(
            before, "be621d6c14454e0e4f69c81205ef0fe21457f8aea08db843dba63cbdb725c0c0",
            "the representation's value for a 4-block chain"
        );

        let mut m = meta(chain_hash(4), &[chain_hash(3)], 4);
        m.seq_num = 4.try_into().unwrap();
        storage.insert(m, block(chain_hash(4))).await.unwrap();
        let after = representation_digest(&storage.get_representation().await);
        assert_ne!(before, after, "the digest moves when the DAG does");
    }

    /// AUDIT C56's owed paragraph, first part — `fringe_states` is keyed by the **fringe store's own
    /// key**, so the in-memory map is a faithful cache of the persisted one rather than a second
    /// index with a different identity.
    ///
    /// The key is `FringeData::fringe_hash_of(fringe)` — a hash over the sorted fringe, which is what
    /// `fringe_data_store` is keyed by (`insert`'s `put` and `create`'s `get` both use it). The old
    /// key was the fringe *set* itself: every lookup compared whole sets, every insert built a fresh
    /// set key, and the map's identity for a fringe could in principle disagree with the store's.
    ///
    /// **How this is falsified (2026-09-24).** The map now *is* the store's index, so the claim is
    /// checkable by construction rather than by a bound: this test reads the map with the store's key
    /// and asserts the map's key, the datum's own `fringe_hash`, the persisted datum and the
    /// finalized block's recorded `member_of_fringe` all agree. Restoring
    /// `BTreeMap<BTreeSet<BlockHash>, FringeData>` makes the assertion unstatable — the map's `get`
    /// takes a set while the store's takes the hash, so the two indexes cannot be compared at all —
    /// and the compile failure is the falsifier; there is no runtime bound here to calibrate against
    /// a tree that has moved (the pass has paid for that mistake twice), and the *value* half is the
    /// negative control: `logical_bytes` and the fringe datum are unchanged by the re-keying.
    #[tokio::test]
    async fn fringe_states_are_keyed_by_the_stores_own_key() {
        let storage = build_storage().await;
        let (genesis, left, right, tip) = (hash(0), hash(1), hash(2), hash(3));

        // Two children of genesis, then a block that justifies only one of them but declares both
        // as its fringe — so the fringe diff is non-empty and a block is marked with its member
        // fringe (`insert`'s `member_of_fringe`), which is the record this test compares against.
        for (seq, h, parents) in [
            (0i64, genesis, vec![]),
            (1, left, vec![genesis]),
            (2, right, vec![genesis]),
        ] {
            let mut m = meta(h, &parents, seq);
            m.seq_num = seq.try_into().unwrap();
            storage.insert(m, block(h)).await.unwrap();
        }
        let fringe: BTreeSet<BlockHash> = [left, right].into_iter().collect();
        let mut m = meta(tip, &[left], 3);
        m.seq_num = 3.try_into().unwrap();
        m.fringe = fringe.clone();
        storage.insert(m, block(tip)).await.unwrap();

        let repr = storage.get_representation().await;
        let key = FringeData::fringe_hash_of(&fringe);

        // The map is keyed by the store's key: the empty fringe the genesis blocks declared is
        // cached under *its* hash, and this fringe under its own — two distinct hashes, no set.
        assert_eq!(
            repr.fringe_states.len(),
            2,
            "the empty fringe and this one, each by its own hash"
        );
        assert!(
            repr.fringe_states
                .contains_key(&FringeData::fringe_hash_of(&BTreeSet::new())),
            "an empty fringe has its own key"
        );
        let cached = repr
            .fringe_states
            .get(&key)
            .unwrap_or_else(|| panic!("no fringe data cached under the store's key {key:?}"));
        assert_eq!(cached.fringe_hash, key, "the datum's own key agrees");
        assert_eq!(cached.fringe, fringe, "and its fringe is the fringe");

        // The persisted store answers the same key with the same datum: one identity, two places.
        let persisted = storage
            .fringe_data_store
            .get(&[key])
            .await
            .unwrap()
            .into_iter()
            .next()
            .flatten()
            .expect("the fringe data was persisted under that key");
        assert_eq!(persisted, *cached);

        // And the finalized block records that same key as its member fringe.
        assert_eq!(
            storage
                .lookup(&right)
                .await
                .unwrap()
                .unwrap()
                .member_of_fringe,
            Some(key),
            "a block finalized by the fringe is marked with the fringe's key"
        );
    }

    /// AUDIT C55 — restoring a stored chain must not rebuild the message state once per block.
    ///
    /// `BlockDagKeyValueStorage::create` folds every stored block through the message state. The
    /// oracle folds through a *persistent* map — `DagMessageState.scala`'s `msgMap + (k -> v)` over
    /// Scala's immutable `Map`, structurally shared, O(log N). A `BTreeMap` has no structural
    /// sharing, so the same-shaped `insert_msg` deep-copies the map per block, and every entry's
    /// own `seen` set with it, making the restore Θ(N³) in copies and Θ(N²) in memory. On
    /// 2026-09-24 a 5,844-block devnet bootstrap spent longer inside this loop than
    /// `tools/devnet.sh` waits for `/api/v1/status`, at ~100% of one core and with no further log
    /// line — which read as a hang and was reported as one.
    ///
    /// The bound is a cost bound, not a semantic one: the rebuild produces the same state either
    /// way, which is exactly why nothing caught it. Measured on the authoring machine at N=1500 in
    /// a debug test build: 5.1 s in place, 108.6 s copying — a 21× gap, widening with N (the one is
    /// Θ(N²), the other Θ(N³)). The constants below sit at N=1200, where that gap is 3.3 s against
    /// 55.6 s: the bound leaves the fixed fold 4.6× of headroom (so a CI runner several times
    /// slower does not trip it) and the copying fold 3.7× over it. Recalibrate from those two
    /// numbers if either side ever changes.
    #[tokio::test]
    async fn restoring_a_stored_chain_is_not_cubic_in_the_message_state() {
        const N: usize = 1200;
        const BOUND: std::time::Duration = std::time::Duration::from_secs(15);

        let metadata_store = empty_metadata_store().await;
        store_chain(&metadata_store, N).await;

        let started = std::time::Instant::now();
        let storage = build_storage_over(metadata_store).await;
        let elapsed = started.elapsed();

        // The fold did happen — so the tripwire cannot pass by never doing the work.
        let repr = storage.get_representation().await;
        assert_eq!(repr.dag_message_state.msg_map.len(), N);

        assert!(
            elapsed < BOUND,
            "restoring a {N}-block chain took {elapsed:?} (bound {BOUND:?}): the message state is \
             being rebuilt per block instead of extended in place"
        );
    }

    /// `meta`, with the sender and sequence number chosen by the caller: H1b needs a failed block by
    /// one validator and a child by another.
    fn meta_by(
        sender_byte: u8,
        seq: i64,
        hash: BlockHash,
        parents: &[BlockHash],
        block_num: i64,
    ) -> BlockMetadata {
        BlockMetadata {
            sender: Validator::new([sender_byte; 65]),
            seq_num: SeqNum::try_from(seq).unwrap(),
            ..meta(hash, parents, block_num)
        }
    }

    /// `block`, with the fields a validator reads.
    fn block_with(
        hash: BlockHash,
        block_number: i64,
        sender_byte: u8,
        seq: i64,
        justifications: &[BlockHash],
        bonds: &[(u8, i64)],
    ) -> BlockMessage {
        BlockMessage {
            block_number: BlockHeight::try_from(block_number).unwrap(),
            sender: Validator::new([sender_byte; 65]),
            seq_num: SeqNum::try_from(seq).unwrap(),
            justifications: justifications.to_vec(),
            bonds: bonds
                .iter()
                .map(|(b, s)| (Validator::new([*b; 65]), NonNegI64::try_from(*s).unwrap()))
                .collect(),
            ..block(hash)
        }
    }

    /// **H1b, measured: the descent order the model requires is violated by a state the port's own
    /// validators admit.**
    ///
    /// `Descends` (`spec/Rchain/Casper/Dag.lean:278-284`) says every parent a message names resolves
    /// to a message *lower* than it. The port enforces that only for **unfailed** parents:
    /// `validate::block_number` skips failed justifications (`validate.rs:131`), while a failed
    /// block's recorded height is its **claimed** `block_num` (`message_from_block_metadata`:
    /// `height: block.block_num`) with nothing bounding it. So a block that fails validation claiming
    /// height 999 enters the DAG at 999, and a later block whose number is computed from its unfailed
    /// justifications sits *below* that parent.
    ///
    /// Reachability is this test's route, which is the block processor's own call
    /// (`blocks/block_processor.rs:55,:132` → `insert`): the failed block is already in the DAG, and a
    /// peer then sends a block naming it. `neglected_invalid_block` is what makes the *bonded* case
    /// unreachable and this one reachable — see the test below.
    #[tokio::test]
    async fn h1b_a_failed_parent_above_the_childs_height_breaks_the_descent_order() {
        let storage = build_storage().await;
        let genesis = hash(0);
        storage
            .insert(meta(genesis, &[], 0), block(genesis))
            .await
            .unwrap();

        // A block that fails validation while claiming height 999. H-2 records it in the message map
        // with that claimed number, and keeps it out of `latest_msgs`.
        let failed = hash(1);
        let mut failed_meta = meta_by(1, 0, failed, &[genesis], 999);
        failed_meta.validation_failed = true;
        storage.insert(failed_meta, block(failed)).await.unwrap();

        // The child: proposer `2`, height 1 (genesis 0 + 1) per `block_number`, justifying the failed
        // block. Validator `1` is not in the child's bond map, so the neglect rule lets it through.
        let child = hash(2);
        let child_block = block_with(child, 1, 2, 0, &[genesis, failed], &[]);
        assert!(
            matches!(
                crate::validate::block_number(&*storage, &child_block)
                    .await
                    .unwrap(),
                Ok(())
            ),
            "the port computes this child's height from its unfailed justifications"
        );
        assert!(
            matches!(
                crate::validate::neglected_invalid_block(&*storage, &child_block)
                    .await
                    .unwrap(),
                Ok(())
            ),
            "an unbonded failed justification is not neglected"
        );

        storage
            .insert(meta_by(2, 0, child, &[genesis, failed], 1), child_block)
            .await
            .unwrap();

        // The model's order does not hold of the DAG the port just built: the failed parent's height
        // (999) is not lower than the child's (1), so `Descends` is false of this state.
        let repr = storage.get_representation().await;
        let parent = repr.dag_message_state.msg_map.get(&failed).unwrap();
        let me = repr.dag_message_state.msg_map.get(&child).unwrap();
        assert_eq!(i64::from(parent.height), 999);
        assert_eq!(i64::from(me.height), 1);
        assert!(
            parent.height > me.height,
            "the parent is higher than the message naming it: Descends (278-284) is violated"
        );
    }

    /// The route the H1b brief named — a child *forced* to justify a bonded failed block — is not
    /// how the port behaves: `neglected_invalid_block` is faithful to the oracle
    /// (`legacy/casper/.../Validate.scala:340-360`) and **refuses** a child that justifies a failed
    /// block whose sender is still bonded. Nothing here forces such a justification; the rule above
    /// is what a child is refused for. Reachability of the `Descends` violation therefore runs through
    /// the unbonded case, and the bonded one is closed.
    #[tokio::test]
    async fn h1b_a_justified_bonded_failed_block_is_refused_rather_than_forced() {
        let storage = build_storage().await;
        let genesis = hash(0);
        storage
            .insert(meta(genesis, &[], 0), block(genesis))
            .await
            .unwrap();
        let failed = hash(1);
        let mut failed_meta = meta_by(1, 0, failed, &[genesis], 999);
        failed_meta.validation_failed = true;
        storage.insert(failed_meta, block(failed)).await.unwrap();

        // Same child, but validator `1` now holds stake in the child's bond map.
        let child_block = block_with(hash(2), 1, 2, 0, &[genesis, failed], &[(1, 100)]);
        let verdict = crate::validate::neglected_invalid_block(&*storage, &child_block)
            .await
            .unwrap();
        assert!(
            !matches!(verdict, Ok(())),
            "justifying a bonded failed block is refused, not forced: {verdict:?}"
        );
    }

    /// **H1c.** A store that already contains a fork is refused on restore, exactly as `insert`
    /// refuses it at ingress.
    ///
    /// `create` rebuilds the message map from persisted metadata, and the persisted layer checks only
    /// that the height numbers are contiguous (`validate_dag_state`) — it never looks at
    /// `(sender, seq_num)`. So a store holding two distinct blocks by one sender reusing a sequence
    /// number — written by a build that predates H-1's gate, or by any path that writes metadata
    /// directly — restores into a DAG holding the equivocation that gate exists to keep out, and the
    /// H-1 stall it prevents is live again with nothing saying so. This test writes such a store and
    /// asserts the refusal names both blocks; a `create` without the re-check returns `Ok` here and
    /// this assertion fails.
    #[tokio::test]
    async fn h1c_a_store_holding_an_equivocation_is_refused_on_restore() {
        let metadata_store = empty_metadata_store().await;
        let genesis = hash(0);
        metadata_store.add(meta(genesis, &[], 0)).await.unwrap();
        // Two distinct blocks, one sender, one `seq_num`: the fork `insert` refuses (H-1).
        metadata_store
            .add(meta_by(1, 0, hash(1), &[genesis], 1))
            .await
            .unwrap();
        metadata_store
            .add(meta_by(1, 0, hash(2), &[genesis], 1))
            .await
            .unwrap();

        let fringe_store: Arc<dyn KeyValueTypedStore<Blake2b256Hash, FringeData>> =
            Arc::new(KeyValueTypedStoreCodec::new(
                in_memory(),
                Arc::new(Blake2b256HashCodec),
                Arc::new(FringeDataCodec),
            ));
        let deploy_index: Arc<dyn KeyValueTypedStore<DeployId, BlockHash>> =
            Arc::new(KeyValueTypedStoreCodec::new(
                in_memory(),
                Arc::new(BytesCodec),
                Arc::new(BlockHashCodec),
            ));
        let deploy_store: Arc<dyn KeyValueTypedStore<DeployId, SignedDeployData>> =
            Arc::new(KeyValueTypedStoreCodec::new(
                in_memory(),
                Arc::new(BytesCodec),
                Arc::new(SignedDeployDataCodec),
            ));

        let err = match BlockDagKeyValueStorage::create(
            metadata_store,
            fringe_store,
            deploy_index,
            deploy_store,
        )
        .await
        {
            Ok(_) => panic!("a store that already contains a fork must not restore silently"),
            Err(e) => e,
        };
        assert!(err.contains("equivocation"), "{err}");
        assert!(err.contains("sequence number"), "{err}");
    }
}
