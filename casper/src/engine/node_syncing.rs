//! Node syncing state machine (port of `engine/NodeSyncing.scala`).
//!
//! Drives the Last Finalized State sync (blocks + tuple space) from the bootstrap node: it
//! accepts the finalized fringe from the bootstrap, runs the `LfsBlockRequester` and
//! `LfsTupleSpaceRequester` streams, then populates the DAG from the received blocks.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use rchain_block_storage::approved_store::{ApprovedStore, FINALIZED_FRINGE_KEY};
use rchain_block_storage::block_store::BlockStore;
use rchain_block_storage::dag::dag_storage::BlockDagStorage;
use rchain_block_storage::syntax::insert_genesis;
use rchain_comm::peer_node::PeerNode;
use rchain_comm::rp::rp_conf::RPConf;
use rchain_comm::transport::transport_layer::TransportLayer;
use rchain_crypto::hash::blake2b256_hash::Blake2b256Hash;
use rchain_models::block_hash::BlockHash;
use rchain_models::block_metadata::BlockMetadata;
use rchain_models::casper::protocol::casper_message::{
    BlockMessage, CasperMessage, FinalizedFringe, StoreItemsMessage,
};
use rchain_rspace::state::RSpaceImporter;
use rchain_shared::log::{Log, LogSource};

use super::lfs_block_requester::request_blocks;
use super::lfs_tuple_space_requester::{request_tuple_space, request_tuple_space_roots};
use crate::protocol::comm_util::CommUtil;
use crate::validator_identity::ValidatorIdentity;

/// The node-syncing engine (port of the `NodeSyncing` class).
pub struct NodeSyncing<I: RSpaceImporter> {
    transport: Arc<dyn TransportLayer>,
    conf: RPConf,
    block_store: BlockStore,
    dag: Arc<dyn BlockDagStorage>,
    approved_store: ApprovedStore,
    comm_util: Arc<CommUtil>,
    log: Arc<dyn Log>,
    log_source: LogSource,
    #[allow(dead_code)] // reserved (Scala stores it; unused in the syncing path)
    validator_id: Option<ValidatorIdentity>,
    #[allow(dead_code)] // reserved (Scala stores it; consumed by the caller)
    trim_state: bool,
    importer: Option<I>,
    incoming_blocks_tx: tokio::sync::mpsc::Sender<BlockMessage>,
    incoming_blocks_rx: Option<tokio::sync::mpsc::Receiver<BlockMessage>>,
    tuple_space_tx: tokio::sync::mpsc::Sender<StoreItemsMessage>,
    tuple_space_rx: Option<tokio::sync::mpsc::Receiver<StoreItemsMessage>>,
    start_requester: bool,
    finished: Arc<tokio::sync::Notify>,
}

impl<I: RSpaceImporter + Send + 'static> NodeSyncing<I> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        transport: Arc<dyn TransportLayer>,
        conf: RPConf,
        block_store: BlockStore,
        dag: Arc<dyn BlockDagStorage>,
        approved_store: ApprovedStore,
        comm_util: Arc<CommUtil>,
        log: Arc<dyn Log>,
        validator_id: Option<ValidatorIdentity>,
        trim_state: bool,
        importer: I,
    ) -> Self {
        let (incoming_blocks_tx, incoming_blocks_rx) = tokio::sync::mpsc::channel(50);
        let (tuple_space_tx, tuple_space_rx) = tokio::sync::mpsc::channel(50);
        NodeSyncing {
            transport,
            conf,
            block_store,
            dag,
            approved_store,
            comm_util,
            log,
            log_source: LogSource::new("casper.engine.NodeSyncing"),
            validator_id,
            trim_state,
            importer: Some(importer),
            incoming_blocks_tx,
            incoming_blocks_rx: Some(incoming_blocks_rx),
            tuple_space_tx,
            tuple_space_rx: Some(tuple_space_rx),
            start_requester: true,
            finished: Arc::new(tokio::sync::Notify::new()),
        }
    }

    /// A future that completes when syncing finishes (port of `finished.get`).
    pub async fn wait(&self) {
        self.finished.notified().await;
    }

    /// A cloneable handle to the syncing-finished notification, for waiting concurrently with the
    /// `handle` loop without holding the engine's mutex.
    pub fn finished_handle(&self) -> Arc<tokio::sync::Notify> {
        self.finished.clone()
    }

    /// Handle an incoming casper message (port of `handle`).
    pub async fn handle(&mut self, peer: &PeerNode, msg: &CasperMessage) -> Result<(), String> {
        match msg {
            CasperMessage::FinalizedFringe(fringe) => {
                self.on_finalized_fringe_message(peer, fringe).await
            }
            CasperMessage::StoreItemsMessage(s) => {
                self.log.info(
                    self.log_source,
                    &format!(
                        "Received StoreItems(history: {}, data: {}) from {peer}.",
                        s.history_items.len(),
                        s.data_items.len()
                    ),
                );
                let _ = self.tuple_space_tx.send(s.clone()).await;
                Ok(())
            }
            CasperMessage::BlockMessage(b) => {
                self.log.info(
                    self.log_source,
                    &format!("BlockMessage received #{} from {peer}.", b.block_number),
                );
                let _ = self.incoming_blocks_tx.send(b.clone()).await;
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Handle a finalized-fringe message, starting the LFS sync once from the bootstrap node (port
    /// of `onFinalizedFringeMessage`).
    async fn on_finalized_fringe_message(
        &mut self,
        sender: &PeerNode,
        fringe: &FinalizedFringe,
    ) -> Result<(), String> {
        let sender_is_bootstrap = self
            .conf
            .bootstrap
            .as_ref()
            .map(|b| b == sender)
            .unwrap_or(false);
        if !sender_is_bootstrap {
            self.log.info(
                self.log_source,
                "Fringe message ignored, not received from bootstrap node.",
            );
        }

        let start = if self.start_requester {
            if sender_is_bootstrap {
                self.start_requester = false;
                true
            } else {
                false
            }
        } else {
            false
        };

        if start {
            self.log.info(
                self.log_source,
                &format!(
                    "Received finalized fringe from bootstrap node ({}).",
                    fringe
                        .hashes
                        .iter()
                        .map(|h| h.to_hex())
                        .collect::<Vec<_>>()
                        .join(" ")
                ),
            );

            // Spawn the LFS sync in the background. Awaiting it here deadlocks: the sync drains
            // `tuple_space_rx`/`incoming_blocks_rx`, which are only fed by `handle` (the
            // StoreItemsMessage/BlockMessage branches) running in this same dispatch loop, which is
            // currently blocked inside this call. Spawning lets `handle` return and keep routing.
            let fringe = fringe.clone();
            let transport = self.transport.clone();
            let conf = self.conf.clone();
            let block_store = self.block_store.clone();
            let dag = self.dag.clone();
            let approved_store = self.approved_store.clone();
            let comm_util = self.comm_util.clone();
            let log = self.log.clone();
            let finished = self.finished.clone();
            let importer = match self.importer.take() {
                Some(importer) => importer,
                None => {
                    self.log.error(
                        self.log_source,
                        "LFS sync requested twice; importer already taken",
                    );
                    return Err("LFS sync already started: importer already taken".to_string());
                }
            };
            let incoming_blocks_rx = match self.incoming_blocks_rx.take() {
                Some(rx) => rx,
                None => {
                    self.log.error(
                        self.log_source,
                        "LFS sync requested twice; incoming-blocks receiver already taken",
                    );
                    return Err(
                        "LFS sync already started: incoming-blocks receiver already taken"
                            .to_string(),
                    );
                }
            };
            let tuple_space_rx = match self.tuple_space_rx.take() {
                Some(rx) => rx,
                None => {
                    self.log.error(
                        self.log_source,
                        "LFS sync requested twice; tuple-space receiver already taken",
                    );
                    return Err(
                        "LFS sync already started: tuple-space receiver already taken".to_string(),
                    );
                }
            };
            tokio::spawn(async move {
                let source = LogSource::new("casper.engine.NodeSyncing");
                match run_approved_state_sync(
                    &fringe,
                    transport,
                    conf,
                    block_store,
                    dag,
                    comm_util,
                    log.clone(),
                    importer,
                    incoming_blocks_rx,
                    tuple_space_rx,
                )
                .await
                {
                    Ok(()) => {
                        if let Err(e) = approved_store
                            .put(&[(FINALIZED_FRINGE_KEY, fringe.clone())])
                            .await
                        {
                            log.error(source, &format!("Failed to store approved block: {e}"));
                        }
                        log.info(source, "LFS state is successfully restored.");
                    }
                    Err(e) => log.error(source, &format!("LFS state sync failed: {e}")),
                }
                finished.notify_waiters();
            });
        }
        Ok(())
    }
}

/// Download the approved (last finalized) state — blocks + tuple space in parallel — and populate the
/// DAG (port of `requestApprovedState`). Free function so it can be spawned off the dispatch loop.
#[allow(clippy::too_many_arguments)]
async fn run_approved_state_sync<I: RSpaceImporter + Send + 'static>(
    fringe: &FinalizedFringe,
    transport: Arc<dyn TransportLayer>,
    conf: RPConf,
    block_store: BlockStore,
    dag: Arc<dyn BlockDagStorage>,
    comm_util: Arc<CommUtil>,
    log: Arc<dyn Log>,
    mut importer: I,
    mut incoming_blocks_rx: tokio::sync::mpsc::Receiver<BlockMessage>,
    mut tuple_space_rx: tokio::sync::mpsc::Receiver<StoreItemsMessage>,
) -> Result<(), String> {
    let source = LogSource::new("casper.engine.NodeSyncing");
    let block_fut = request_blocks(
        fringe,
        &mut incoming_blocks_rx,
        Duration::from_secs(30),
        &block_store,
        comm_util.as_ref(),
        log.as_ref(),
    );
    let tuple_fut = request_tuple_space(
        fringe,
        &mut tuple_space_rx,
        Duration::from_secs(120),
        transport.as_ref(),
        &conf,
        &mut importer,
        log.as_ref(),
    );

    let (block_st, tuple_res) = tokio::join!(block_fut, tuple_fut);
    tuple_res.map_err(|e| e.to_string())?;

    // The fringe tuple-space request above only hydrates the finalized-fringe root itself. Casper's
    // read/validation APIs (explore, data-at-name, and mergeable-sidecar regeneration during block
    // indexing) open the pre/post RSpace root of specific downloaded blocks directly, not just the
    // fringe root - request those too, or an observer's local history reader has no root to open for
    // anything but the exact fringe state once restore finishes.
    let fringe_root = Blake2b256Hash::from_byte_array(fringe.state_hash.as_bytes());
    let block_roots = collect_block_state_roots(&block_store, &block_st.height_map).await?;
    let extra_roots: Vec<Blake2b256Hash> = block_roots
        .into_iter()
        .filter(|root| *root != fringe_root)
        .collect();
    if !extra_roots.is_empty() {
        log.info(
            source,
            &format!(
                "Requesting tuple-space data for {} approved block state roots.",
                extra_roots.len()
            ),
        );
        request_tuple_space_roots(
            &extra_roots,
            &mut tuple_space_rx,
            Duration::from_secs(120),
            transport.as_ref(),
            &conf,
            &mut importer,
            log.as_ref(),
        )
        .await
        .map_err(|e| e.to_string())?;
    }

    log.info(source, "Rholang state received and saved to store.");
    populate_dag(
        dag.as_ref(),
        &block_store,
        log.as_ref(),
        &block_st.height_map,
    )
    .await?;
    Ok(())
}

/// Collect the pre/post RSpace state roots of every downloaded approved block, so the caller can
/// request tuple-space data for any that aren't the finalized-fringe root already hydrated above.
async fn collect_block_state_roots(
    block_store: &BlockStore,
    height_map: &BTreeMap<i64, BTreeSet<BlockHash>>,
) -> Result<BTreeSet<Blake2b256Hash>, String> {
    let mut roots = BTreeSet::new();

    for hash in height_map.values().flat_map(|s| s.iter().copied()) {
        let block = block_store
            .get(&[hash])
            .await?
            .into_iter()
            .flatten()
            .next()
            .ok_or_else(|| format!("missing block {}", hash.to_hex()))?;
        roots.insert(Blake2b256Hash::from_byte_array(
            block.pre_state_hash.as_bytes(),
        ));
        roots.insert(Blake2b256Hash::from_byte_array(
            block.post_state_hash.as_bytes(),
        ));
    }

    Ok(roots)
}

/// Insert the received blocks into the DAG (port of `populateDag`, minus the Scala `minHeight`
/// filter — the full ancestry chain is now downloaded and must be inserted).
async fn populate_dag(
    dag: &dyn BlockDagStorage,
    block_store: &BlockStore,
    log: &dyn Log,
    height_map: &BTreeMap<i64, BTreeSet<BlockHash>>,
) -> Result<(), String> {
    let source = LogSource::new("casper.engine.NodeSyncing");
    log.info(source, "Adding blocks for approved state to DAG.");

    // Insert blocks in ascending height order (parents before children): `dag.insert` requires each
    // block's justifications to already be present in the message map, and a block's justifications
    // always sit at strictly lower heights (block height = 1 + max justification height). The prior
    // `.reverse()` inserted the newest block first — whose justification was not yet in the map —
    // so LFS sync failed with "justification not present in message map".
    for hash in height_map.values().flat_map(|s| s.iter().copied()) {
        let block = block_store
            .get(&[hash])
            .await?
            .into_iter()
            .flatten()
            .next()
            .ok_or_else(|| format!("missing block {}", hash.to_hex()))?;
        let block_height = i64::from(block.block_number);
        log.info(
            source,
            &format!("Adding #{} {}.", block.block_number, hash.to_hex()),
        );
        if block_height == 0 {
            // Genesis block: insert with validated metadata (fringe empty, fringe_state =
            // pre_state), matching `insert_genesis`, so the validator is bonded and can build on
            // block 0.
            insert_genesis(dag, block).await?;
        } else {
            let bmd = BlockMetadata::from_block(&block);
            dag.insert(bmd, block).await?;
        }
    }

    log.info(source, "Blocks for approved state added to DAG.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rchain_block_storage::dag::codecs::{
        Blake2b256HashCodec, BlockHashCodec, BlockMessageCodec, BlockMetadataCodec,
        FringeDataCodec, SignedDeployDataCodec,
    };
    use rchain_block_storage::dag::dag_storage::DeployId;
    use rchain_crypto::hash::blake2b256_hash::Blake2b256Hash;
    use rchain_models::block::state_hash::StateHash;
    use rchain_models::casper::protocol::casper_message::{
        BlockMessage, RholangState, SignedDeployData,
    };
    use rchain_models::fringe_data::FringeData;
    use rchain_models::validator::Validator;
    use rchain_shared::log::NopLog;
    use rchain_shared::refined::BlockHeight;
    use rchain_shared::store::{InMemoryKeyValueStore, KeyValueStore};
    use rchain_shared::typed_store::{BytesCodec, KeyValueTypedStore, KeyValueTypedStoreCodec};

    use crate::block_metadata_store::BlockMetadataStore;
    use crate::dag::BlockDagKeyValueStorage;

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

    fn chain_block(block_num: i64, justification: Option<BlockHash>) -> BlockMessage {
        let justifications: Vec<BlockHash> = justification.into_iter().collect();
        BlockMessage {
            version: 1,
            shard_id: "root".to_string(),
            block_hash: hash(block_num as u8),
            block_number: BlockHeight::try_from(block_num).unwrap(),
            sender: Validator::new([0u8; 65]),
            seq_num: block_num.try_into().unwrap(),
            pre_state_hash: StateHash::new([0u8; 32]),
            post_state_hash: StateHash::new([0u8; 32]),
            justifications,
            bonds: BTreeMap::new(),
            rejected_deploys: BTreeSet::new(),
            rejected_blocks: BTreeSet::new(),
            rejected_senders: BTreeSet::new(),
            state: RholangState::default(),
            sig_algorithm: "secp256k1".to_string(),
            sig: vec![],
        }
    }

    async fn build_dag() -> Arc<BlockDagKeyValueStorage> {
        let metadata_store = Arc::new(
            BlockMetadataStore::create(Arc::new(KeyValueTypedStoreCodec::new(
                in_memory(),
                Arc::new(BlockHashCodec),
                Arc::new(BlockMetadataCodec),
            )))
            .await
            .unwrap(),
        );
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

    /// Regression test for the LFS-sync "justification not present in message map" failure: the
    /// DAG must be populated parents-before-children, i.e. in ascending block height order. The
    /// prior code reversed the height map and inserted the newest block first, whose justification
    /// was not yet in the message map.
    #[tokio::test]
    async fn populate_dag_inserts_parents_before_children() {
        let n = 5i64;
        // A single-validator chain, each block justifying its predecessor.
        let blocks: Vec<BlockMessage> = (0..n)
            .map(|i| chain_block(i, (i > 0).then(|| hash(i as u8 - 1))))
            .collect();

        let block_store: BlockStore = Arc::new(KeyValueTypedStoreCodec::new(
            in_memory(),
            Arc::new(BlockHashCodec),
            Arc::new(BlockMessageCodec),
        ));
        for b in &blocks {
            block_store.put(&[(b.block_hash, b.clone())]).await.unwrap();
        }

        let dag = build_dag().await;
        let mut height_map: BTreeMap<i64, BTreeSet<BlockHash>> = BTreeMap::new();
        for b in &blocks {
            height_map
                .entry(i64::from(b.block_number))
                .or_default()
                .insert(b.block_hash);
        }

        populate_dag(dag.as_ref(), &block_store, &NopLog, &height_map)
            .await
            .expect("populate_dag must succeed when blocks are inserted parents-first");

        // The whole chain is in the DAG, so the latest block number equals the chain length.
        assert_eq!(dag.get_representation().await.latest_block_number(), n);
    }
}
