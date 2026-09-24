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
        let mut fringe_states: BTreeMap<BTreeSet<BlockHash>, FringeData> = BTreeMap::new();

        for hash in height_map.values().flatten() {
            if dag_msg_state.msg_map.contains_key(hash) {
                continue;
            }
            let block = block_metadata_store.get_unchecked(hash).await?;
            let msg = message_from_block_metadata(&block, &dag_msg_state.msg_map)
                .ok_or_else(|| "justification not present in message map".to_string())?;
            // In place: this loop folds the whole stored chain through the state, and the
            // persistent-shaped `insert_msg` clones the map (and every message's `seen` set) on
            // each step — Θ(N³) over a stored chain, which is what made a 5,844-block restart take
            // longer than the devnet waits for one (AUDIT C55).
            dag_msg_state.insert_msg_mut(&msg);
            if !fringe_states.contains_key(&msg.fringe) {
                let fringe_hash = FringeData::fringe_hash_of(&msg.fringe);
                let fd = fringe_data_store
                    .get(&[fringe_hash])
                    .await?
                    .into_iter()
                    .next()
                    .flatten();
                if let Some(fd) = fd {
                    fringe_states.insert(msg.fringe.clone(), fd);
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
        })
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
            repr.fringe_states.insert(msg.fringe.clone(), fringe_data);
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
    use rchain_shared::refined::BlockHeight;
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
        Arc::new(
            BlockDagKeyValueStorage::create(
                metadata_store,
                fringe_store,
                deploy_index,
                deploy_store,
            )
            .await
            .unwrap(),
        )
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
            store.add(meta(h, &parents, i as i64)).await.unwrap();
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

        // A distinct `seq_num` per block: every stored block carries sender `[0; 65]` and seq 0, and
        // a second block from the same sender at the same seq is refused as an equivocation (H-1).
        let mut tip = chain_hash(N - 1);
        let started = std::time::Instant::now();
        for k in 0..K {
            let next = chain_hash(N + k);
            let mut m = meta(next, &[tip], (N + k) as i64);
            m.seq_num = (k as i64 + 1).try_into().unwrap();
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
}
