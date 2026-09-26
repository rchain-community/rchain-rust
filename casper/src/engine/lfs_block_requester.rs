//! Last Finalized State block requester (port of `engine/LfsBlockRequester.scala`).
//!
//! Downloads the blocks needed to reconstruct the last finalized state, following justifications
//! from the finalized fringe. The pure requester state is [`super::LfsState`]; the `request_blocks`
//! stream orchestration (request loop + response loop with an idle-resend timeout) is ported here
//! onto tokio channels, mirroring the fs2 `requestStream concurrently responseStream` structure.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use rchain_block_storage::block_store::BlockStore;
use rchain_models::block_hash::BlockHash;
use rchain_models::casper::protocol::casper_message::{BlockMessage, FinalizedFringe};
use rchain_shared::log::{Log, LogSource};

use super::{LfsState, ReceiveInfo};
use crate::protocol::comm_util::CommUtil;
use crate::validate;

/// The block-requester state keyed by block hash (port of `ST[BlockHash]`).
type St = LfsState<BlockHash>;

/// Validate a received block and, if accepted, request its justifications. Returns whether the
/// block was requested and its hash valid (port of `validateReceivedBlock`, minus the Scala
/// `lowerBound` acceptance cutoff — the full ancestry chain is walked to genesis).
async fn validate_received_block(
    st: &Arc<tokio::sync::Mutex<St>>,
    block: &BlockMessage,
    log: &dyn Log,
    source: LogSource,
) -> bool {
    let block_number = i64::from(block.block_number);
    let info = {
        let mut guard = st.lock().await;
        let (new_state, info) = guard.received(&block.block_hash, block_number);
        *guard = new_state;
        info
    };
    let ReceiveInfo {
        requested,
        last_latest,
        ..
    } = info;

    let block_hash_is_valid = requested && validate::block_hash(block);
    if requested && !block_hash_is_valid {
        log.warn(
            source,
            &format!(
                "Received block #{} with invalid hash. Ignored block.",
                block.block_number
            ),
        );
    }

    if block_hash_is_valid {
        if last_latest {
            log.info(source, "Latest blocks downloaded.");
        }
        // Always request the block's justifications: `dag.insert` requires every justification to
        // be present in the message map, so the requester must reconstruct the full ancestry chain
        // down to the genesis block (a syncing node's DAG is always empty).
        let justifications: BTreeSet<BlockHash> = block.justifications.iter().copied().collect();
        let mut guard = st.lock().await;
        *guard = guard.add(&justifications);
        return requested;
    }
    false
}

/// Save a received block to the store and mark it done (port of `saveBlock`).
///
/// **The write is the point of this function, so its failure is not discarded** (AUDIT C65): the
/// oracle's `saveBlock` is `containsBlock` then `putBlockToStore` in `F`, and a failed write fails
/// the sync attempt rather than being swallowed. Discarding it and marking the block done anyway
/// would leave the requester believing it holds a block it never persisted, and — because `done`
/// removes the key from the request map — it would never be requested again: a permanently missing
/// block in a "synced" state.
async fn save_block(
    st: &Arc<tokio::sync::Mutex<St>>,
    block_store: &BlockStore,
    block: &BlockMessage,
) -> Result<(), String> {
    let already_saved = block_store
        .contains(&[block.block_hash])
        .await?
        .first()
        .copied()
        .unwrap_or(false);
    if !already_saved {
        block_store
            .put(&[(block.block_hash, block.clone())])
            .await?;
    }
    let mut guard = st.lock().await;
    *guard = guard.done(&block.block_hash);
    Ok(())
}

/// Process an incoming block: validate it, save it, and trigger the next request (port of
/// `processBlock`).
async fn process_block(
    st: &Arc<tokio::sync::Mutex<St>>,
    block_store: &BlockStore,
    log: &dyn Log,
    source: LogSource,
    request_tx: &tokio::sync::mpsc::Sender<bool>,
    block: &BlockMessage,
) -> Result<(), String> {
    let is_valid = validate_received_block(st, block, log, source).await;
    if is_valid {
        save_block(st, block_store, block).await?;
    }
    // Trigger the request queue (without resending already-requested blocks).
    let _ = request_tx.send(false).await;
    Ok(())
}

/// Take the next set of hashes to request, enqueue existing ones for processing, and broadcast
/// requests for the missing ones (port of `requestNext`).
async fn request_next(
    st: &Arc<tokio::sync::Mutex<St>>,
    response_hash_tx: &tokio::sync::mpsc::UnboundedSender<BlockHash>,
    block_store: &BlockStore,
    comm_util: &CommUtil,
    resend: bool,
) -> Result<(), String> {
    let is_end = { st.lock().await.is_finished() };
    let hashes = {
        let mut guard = st.lock().await;
        let (new_state, hashes) = guard.get_next(resend);
        *guard = new_state;
        hashes
    };

    let hashes_vec: Vec<BlockHash> = hashes.iter().copied().collect();
    // A failed read must not read as "nothing to request" (AUDIT C65): `unwrap_or_default()` yields
    // an empty `Vec<bool>`, the `zip` below yields nothing, and every hash is silently neither
    // reported as existing nor requested — the walk then does nothing until the idle resend nudges
    // it. The oracle's `filterA(containsBlock)` propagates instead.
    let contains = block_store.contains(&hashes_vec).await?;
    let mut existing = Vec::new();
    let mut missing = Vec::new();
    for (h, c) in hashes_vec.into_iter().zip(contains) {
        if c {
            existing.push(h);
        } else {
            missing.push(h);
        }
    }

    for h in &existing {
        let _ = response_hash_tx.send(*h);
    }
    if !is_end && !missing.is_empty() {
        for h in &missing {
            comm_util.broadcast_request_for_block(h, Some(1)).await;
        }
    }
    Ok(())
}

/// Request all blocks needed for the last finalized state (port of `LfsBlockRequester.stream`).
///
/// Returns the final requester state (the Scala stream's `.last` element) once all blocks are
/// received.
pub async fn request_blocks(
    fringe: &FinalizedFringe,
    incoming_blocks: &mut tokio::sync::mpsc::Receiver<BlockMessage>,
    request_timeout: Duration,
    block_store: &BlockStore,
    comm_util: &CommUtil,
    log: &dyn Log,
) -> Result<St, String> {
    let source = LogSource::new("casper.engine.LfsBlockRequester");

    // Finalized block hashes from which LFS sync starts.
    let finalized_hashes: BTreeSet<BlockHash> = fringe.hashes.iter().copied().collect();
    let st = Arc::new(tokio::sync::Mutex::new(LfsState::new(
        finalized_hashes.clone(),
        finalized_hashes,
    )));

    // `true` triggers a resend of already-requested blocks.
    let (request_tx, mut request_rx) = tokio::sync::mpsc::channel::<bool>(2);
    let (response_hash_tx, mut response_hash_rx) =
        tokio::sync::mpsc::unbounded_channel::<BlockHash>();

    // "Light the fire!" / start the first request for blocks.
    let _ = request_tx.send(false).await;

    // Request loop: pull request triggers (or resend on idle timeout) and request next blocks,
    // terminating once all blocks are finished.
    let request_loop = async {
        loop {
            let resend = tokio::select! {
                r = request_rx.recv() => match r {
                    Some(r) => r,
                    None => return Ok::<(), String>(()),
                },
                _ = tokio::time::sleep(request_timeout) => {
                    log.warn(
                        source,
                        &format!("No block responses for {request_timeout:?}. Resending requests."),
                    );
                    true
                }
            };
            request_next(&st, &response_hash_tx, block_store, comm_util, resend).await?;
            if st.lock().await.is_finished() {
                return Ok(());
            }
        }
    };

    // Response loop: handle incoming blocks and existing-block hashes in parallel with the request
    // loop.
    let response_loop = async {
        loop {
            tokio::select! {
                block = incoming_blocks.recv() => {
                    match block {
                        Some(block) => {
                            process_block(&st, block_store, log, source, &request_tx, &block)
                                .await?;
                        }
                        None => return Ok::<(), String>(()),
                    }
                }
                hash = response_hash_rx.recv() => {
                    match hash {
                        Some(hash) => {
                            let block = block_store
                                .get(&[hash])
                                .await
                                .unwrap_or_default()
                                .into_iter()
                                .flatten()
                                .next();
                            if let Some(block) = block {
                                log.info(
                                    source,
                                    &format!("Process existing block #{}", block.block_number),
                                );
                                process_block(&st, block_store, log, source, &request_tx, &block)
                                    .await?;
                            }
                        }
                        None => return Ok(()),
                    }
                }
            }
        }
    };

    tokio::pin!(request_loop);
    tokio::pin!(response_loop);
    // Either loop ending ends the walk; an error from either fails the sync attempt, which is what
    // the oracle's `compile.drain` does with a store failure (AUDIT C65).
    tokio::select! {
        outcome = &mut request_loop => outcome?,
        outcome = &mut response_loop => outcome?,
    }

    let guard = st.lock().await;
    Ok(guard.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use rchain_block_storage::block_store::BlockStore;
    use rchain_block_storage::dag::codecs::{BlockHashCodec, BlockMessageCodec};
    use rchain_models::block::state_hash::StateHash;
    use rchain_models::block_hash::BlockHash;
    use rchain_models::casper::protocol::casper_message::RholangState;
    use rchain_models::validator::Validator;
    use rchain_shared::log::{LogSource, NopLog};
    use rchain_shared::store::InMemoryKeyValueStore;
    use rchain_shared::typed_store::{KeyValueTypedStore, KeyValueTypedStoreCodec};

    use crate::proto_util::hash_block;
    use async_trait::async_trait;
    use rchain_comm::errors::CommErr;
    use rchain_comm::peer_node::{NodeIdentifier, PeerNode};
    use rchain_comm::rp::rp_conf::{ClearConnectionsConf, RPConf};
    use rchain_comm::transport::chunker::Blob;
    use rchain_comm::transport::transport_layer::TransportLayer;
    use rchain_models::casper::protocol::packet_type_tag::FromPacket;
    use rchain_models::comm::protocol::Protocol;
    use rchain_shared::refined::{BlockHeight, SeqNum};
    use std::time::{Duration, Instant};

    use crate::protocol::casper_message_protocol::BlockRequestSerde;
    use crate::protocol::comm_util::{CommUtil, ConnectionsCell};

    fn source() -> LogSource {
        LogSource::new("casper.engine.LfsBlockRequester.test")
    }

    /// A block with the given justifications and a placeholder hash.
    fn block(justifications: &[BlockHash]) -> BlockMessage {
        BlockMessage {
            version: 1,
            shard_id: "root".to_string(),
            block_hash: BlockHash::new([0u8; 32]),
            block_number: 0.try_into().unwrap(),
            sender: Validator::new([1u8; 65]),
            seq_num: 0.try_into().unwrap(),
            pre_state_hash: StateHash::new([0u8; 32]),
            post_state_hash: StateHash::new([0u8; 32]),
            justifications: justifications.to_vec(),
            bonds: BTreeMap::new(),
            rejected_deploys: BTreeSet::new(),
            rejected_blocks: BTreeSet::new(),
            rejected_senders: BTreeSet::new(),
            state: RholangState::default(),
            sig_algorithm: "secp256k1".to_string(),
            sig: vec![],
            timestamp: 0,
        }
    }

    /// A block whose `block_hash` really is its content hash, so `validate::block_hash` accepts it.
    fn valid_block(justifications: &[BlockHash]) -> BlockMessage {
        let mut b = block(justifications);
        b.block_hash = hash_block(&b);
        b
    }

    /// A store holding one in-memory typed map.
    async fn store() -> BlockStore {
        Arc::new(KeyValueTypedStoreCodec::new(
            Arc::new(tokio::sync::Mutex::new(Box::new(
                InMemoryKeyValueStore::default(),
            ))),
            Arc::new(BlockHashCodec),
            Arc::new(BlockMessageCodec),
        ))
    }

    /// The state a node is in once it has *asked* for `hash`: `received` only reports
    /// `requested: true` for a key in `Requested` status, so a fixture that skipped this step would
    /// test the rejection path by accident (see the resend test).
    fn requested(hash: BlockHash) -> St {
        let seeded = St::new([hash].into_iter().collect(), BTreeSet::new());
        seeded.get_next(false).0
    }

    /// A block nobody asked for is dropped without even checking its hash: the requester only
    /// accepts what it requested, so an unsolicited block cannot grow the download set (or the
    /// store) from a peer's say-so.
    #[tokio::test]
    async fn an_unrequested_block_is_rejected() {
        let st = Arc::new(tokio::sync::Mutex::new(St::new(
            BTreeSet::new(),
            BTreeSet::new(),
        )));
        // Perfectly valid content — rejected purely for not having been requested.
        assert!(!validate_received_block(&st, &valid_block(&[]), &NopLog, source()).await);
    }

    /// A requested block whose hash does not match its content is rejected: the requester
    /// re-derives the hash rather than trusting the message, so a peer cannot substitute a
    /// different block for the one requested. Its justifications are not added either.
    ///
    /// The rejected key is nonetheless marked `Received` — faithful to `LfsBlockRequester
    /// .validateReceivedBlock`, which calls `st.received` before it checks the hash — and **nothing
    /// re-requests it**: `get_next(resend)` only re-requests `Init` or `Requested` keys, and this
    /// one is now `Received`. So a peer that answers a request with a forged block stalls the sync
    /// for that key in *both* implementations. Pinned as the latent liveness wart it is (AUDIT
    /// §16): if a guard is ever added, this test fails and is updated deliberately.
    #[tokio::test]
    async fn a_requested_block_with_a_forged_hash_is_rejected() {
        let forged = block(&[]);
        let st = Arc::new(tokio::sync::Mutex::new(requested(forged.block_hash)));

        assert!(
            !validate_received_block(&st, &forged, &NopLog, source()).await,
            "a hash that does not match the content must not be accepted"
        );
        let guard = st.lock().await;
        assert!(
            guard.get_next(false).1.is_empty(),
            "a rejected block must not be requested again in the normal path"
        );
        assert!(
            guard.get_next(true).1.is_empty(),
            "…nor by the resend path, which only re-requests `Requested` keys"
        );
        assert!(
            !guard.is_finished(),
            "…and the key is not `done`, so the requester cannot report finished either"
        );
    }

    /// An accepted block contributes its **justifications** to the download set — that is how the
    /// ancestry chain is walked — and the acceptance flag is reported back to the caller.
    #[tokio::test]
    async fn an_accepted_block_adds_its_justifications() {
        let justification = valid_block(&[]);
        let block = valid_block(&[justification.block_hash]);
        let st = Arc::new(tokio::sync::Mutex::new(requested(block.block_hash)));

        assert!(
            validate_received_block(&st, &block, &NopLog, source()).await,
            "a requested block with a matching hash is accepted"
        );
        let next = { st.lock().await.get_next(false) }.1;
        assert!(
            next.contains(&justification.block_hash),
            "the block's justifications become the next requests: {next:?}"
        );
    }

    /// Saving is idempotent: a block that arrives twice is stored once and finishes once. `done`
    /// only moves a `Received` key, so the two saves must leave the state finished rather than
    /// leaving the key behind — a requester that never reports finished stalls the sync.
    #[tokio::test]
    async fn saving_is_idempotent() {
        let block = valid_block(&[]);
        let block_store = store().await;
        let st = Arc::new(tokio::sync::Mutex::new(requested(block.block_hash)));

        // Receive it first (`done` requires the `Received` status), then save twice.
        assert!(validate_received_block(&st, &block, &NopLog, source()).await);
        save_block(&st, &block_store, &block).await.unwrap();
        save_block(&st, &block_store, &block).await.unwrap();

        assert_eq!(
            block_store
                .contains(&[block.block_hash])
                .await
                .expect("contains"),
            vec![true]
        );
        assert!(
            st.lock().await.is_finished(),
            "the only requested block was saved, so the requester is finished"
        );
    }
    fn peer(name: &str) -> PeerNode {
        PeerNode::from(
            NodeIdentifier::new(name.as_bytes().to_vec()),
            "host".to_string(),
            rchain_shared::refined::Port::new(40400),
            rchain_shared::refined::Port::new(40404),
        )
    }

    /// A chain of `n` blocks — block `i` justifies block `i - 1` — each with a real
    /// content-addressed hash, so the requester's `validate::block_hash` accepts it.
    fn chain(n: usize) -> Vec<BlockMessage> {
        let mut blocks: Vec<BlockMessage> = Vec::with_capacity(n);
        for i in 0..n {
            let justifications: Vec<BlockHash> =
                blocks.last().map(|b| b.block_hash).into_iter().collect();
            let mut b = valid_block(&justifications);
            b.block_number = BlockHeight::try_from(i as i64).expect("a small chain");
            b.seq_num = SeqNum::try_from(i as i64).expect("a small chain");
            b.block_hash = hash_block(&b);
            blocks.push(b);
        }
        blocks
    }

    /// A transport that *answers*: every `BlockRequest` it sees is served by pushing the requested
    /// block into the requester's incoming channel, the way a peer's `handle_block_request` does.
    struct ServingTransport {
        blocks: BTreeMap<BlockHash, BlockMessage>,
        incoming: tokio::sync::mpsc::Sender<BlockMessage>,
    }

    #[async_trait]
    impl TransportLayer for ServingTransport {
        async fn send(&self, _peer: &PeerNode, _msg: Protocol) -> CommErr<()> {
            Ok(())
        }

        async fn broadcast(&self, _peers: &[PeerNode], msg: Protocol) -> Vec<CommErr<()>> {
            let Some(rchain_models::comm::protocol::protocol::Message::Packet(packet)) =
                msg.message
            else {
                return Vec::new();
            };
            if packet.type_id != "BlockRequest" {
                return Vec::new();
            }
            let Ok(request) = BlockRequestSerde.parse(&packet.content) else {
                return Vec::new();
            };
            let hash = request.hash;
            if let Some(block) = self.blocks.get(&hash) {
                let _ = self.incoming.send(block.clone()).await;
            }
            Vec::new()
        }

        async fn stream(&self, _peers: &[PeerNode], _blob: Blob) {}
    }

    /// **U13's instrument (AUDIT C62's serving term): the LFS block walk advances on *responses*, not
    /// on the idle resend.**
    ///
    /// The devnet showed a fresh validator pulling blocks at ~1.5 per minute, with the requester's own
    /// `No block responses for 30s. Resending requests.` warning between them. On a chain a block's
    /// only justification is its parent, so the walk is one generation per block — which makes that
    /// rate **one idle timeout per block** rather than one round trip. This test isolates the
    /// requester: a chain of `N` blocks, a transport that answers every request immediately from the
    /// requester's own point of view, and the production `request_timeout` of 30 s. A
    /// response-driven walk finishes in milliseconds; a walk that waits for the resend takes
    /// `N × 30 s`. The bound sits at 5 s — three orders of magnitude above the healthy shape and
    /// below even one timeout, so it cannot pass by being lucky.
    #[tokio::test]
    async fn the_walk_advances_on_responses_not_on_the_idle_timeout() {
        const N: usize = 6;
        const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
        const BOUND: Duration = Duration::from_secs(5);

        let blocks = chain(N);
        let by_hash: BTreeMap<BlockHash, BlockMessage> =
            blocks.iter().map(|b| (b.block_hash, b.clone())).collect();
        let tip = blocks.last().expect("a non-empty chain").block_hash;
        let store = store().await;
        let (incoming_tx, mut incoming_rx) = tokio::sync::mpsc::channel(64);
        let transport = Arc::new(ServingTransport {
            blocks: by_hash,
            incoming: incoming_tx,
        });
        let connections: ConnectionsCell =
            Arc::new(tokio::sync::RwLock::new(vec![peer("bootstrap")]));
        let conf = RPConf {
            local: peer("local"),
            network_id: "testnet".to_string(),
            bootstrap: None,
            default_timeout: Duration::from_secs(10),
            max_num_of_connections: 10,
            clear_connections: ClearConnectionsConf {
                num_of_connections_pinged: 10,
            },
        };
        let comm_util = CommUtil::new(transport, conf, connections, Arc::new(NopLog));
        let fringe = FinalizedFringe {
            hashes: vec![tip],
            state_hash: StateHash::new([0u8; 32]),
        };

        let started = Instant::now();
        let state = request_blocks(
            &fringe,
            &mut incoming_rx,
            REQUEST_TIMEOUT,
            &store,
            &comm_util,
            &NopLog,
        )
        .await
        .expect("a healthy store");
        let elapsed = started.elapsed();
        println!(
            "walk of {N} blocks: {elapsed:?} (idle timeout {REQUEST_TIMEOUT:?}), finished: {}",
            state.is_finished()
        );

        assert!(state.is_finished(), "the walk completed");
        let stored = store
            .contains(
                &blocks
                    .iter()
                    .map(|b| b.block_hash)
                    .collect::<Vec<BlockHash>>(),
            )
            .await
            .expect("store readable");
        assert!(
            stored.iter().all(|c| *c),
            "every block in the chain was fetched and saved"
        );
        assert!(
            elapsed < BOUND,
            "a {N}-block walk against a peer that answers every request took {elapsed:?} (bound \
             {BOUND:?}, idle timeout {REQUEST_TIMEOUT:?}): the walk is advancing on the idle resend \
             rather than on the response, so each block costs one request timeout"
        );
    }
    /// A block store that fails what it is told to fail — the fault injection AUDIT C65's two
    /// falsifiers need, since both defects live on error paths that a healthy store never reaches.
    struct FailingStore {
        inner: BlockStore,
        fail_puts: bool,
        fail_reads: bool,
    }

    #[async_trait]
    impl KeyValueTypedStore<BlockHash, BlockMessage> for FailingStore {
        async fn get(&self, keys: &[BlockHash]) -> Result<Vec<Option<BlockMessage>>, String> {
            if self.fail_reads {
                return Err("store is unreadable".to_string());
            }
            self.inner.get(keys).await
        }

        async fn put(&self, pairs: &[(BlockHash, BlockMessage)]) -> Result<(), String> {
            if self.fail_puts {
                return Err("store refuses writes".to_string());
            }
            self.inner.put(pairs).await
        }

        async fn delete(&self, keys: &[BlockHash]) -> Result<usize, String> {
            self.inner.delete(keys).await
        }

        async fn contains(&self, keys: &[BlockHash]) -> Result<Vec<bool>, String> {
            if self.fail_reads {
                return Err("store is unreadable".to_string());
            }
            self.inner.contains(keys).await
        }

        async fn to_map(&self) -> Result<BTreeMap<BlockHash, BlockMessage>, String> {
            self.inner.to_map().await
        }
    }

    /// The comm state a walk runs with, pointed at a transport that answers every request.
    fn comm_state(
        blocks: &[BlockMessage],
        incoming: tokio::sync::mpsc::Sender<BlockMessage>,
    ) -> CommUtil {
        let by_hash: BTreeMap<BlockHash, BlockMessage> =
            blocks.iter().map(|b| (b.block_hash, b.clone())).collect();
        let connections: ConnectionsCell =
            Arc::new(tokio::sync::RwLock::new(vec![peer("bootstrap")]));
        let conf = RPConf {
            local: peer("local"),
            network_id: "testnet".to_string(),
            bootstrap: None,
            default_timeout: Duration::from_secs(10),
            max_num_of_connections: 10,
            clear_connections: ClearConnectionsConf {
                num_of_connections_pinged: 10,
            },
        };
        CommUtil::new(
            Arc::new(ServingTransport {
                blocks: by_hash,
                incoming,
            }),
            conf,
            connections,
            Arc::new(NopLog),
        )
    }

    /// **AUDIT C65, first half: a failed block write fails the walk, and the block is not marked
    /// done.**
    ///
    /// `save_block` read `already_saved` through `unwrap_or_default()` (a store error reading as "not
    /// saved"), **discarded** the result of `put`, and marked the block `done` regardless — so on a
    /// write failure the requester believed it held a block it never persisted, and `done` removes the
    /// key from the request map, which means it would never be requested again. The oracle's
    /// `saveBlock` is `containsBlock` then `putBlockToStore` in `F`: a failed write fails the sync
    /// attempt.
    ///
    /// Falsified in the witnessing form: with the discard restored (`let _ = block_store.put(..)`)
    /// this returns `Ok`, the block is marked done, and the store is empty — success reported over a
    /// silent loss, which is what the assertion below refuses.
    #[tokio::test]
    async fn a_failed_block_write_fails_the_walk_instead_of_marking_the_block_done() {
        let blocks = chain(3);
        let tip = blocks.last().expect("a non-empty chain").block_hash;
        let inner = store().await;
        let failing: BlockStore = Arc::new(FailingStore {
            inner: inner.clone(),
            fail_puts: true,
            fail_reads: false,
        });
        let (incoming_tx, mut incoming_rx) = tokio::sync::mpsc::channel(64);
        let comm_util = comm_state(&blocks, incoming_tx);
        let fringe = FinalizedFringe {
            hashes: vec![tip],
            state_hash: StateHash::new([0u8; 32]),
        };

        let outcome = request_blocks(
            &fringe,
            &mut incoming_rx,
            Duration::from_secs(5),
            &failing,
            &comm_util,
            &NopLog,
        )
        .await;

        assert!(
            outcome.is_err(),
            "a store that refuses the write must fail the walk; reporting success would leave the \
             block marked done with nothing persisted"
        );
        assert!(
            !inner.to_map().await.expect("readable").contains_key(&tip),
            "nothing was persisted, so nothing may be reported as saved"
        );
    }

    /// **AUDIT C65, second half: a failed `contains` must not read as "nothing to request".**
    ///
    /// `request_next` computed the request set from
    /// `block_store.contains(&hashes).await.unwrap_or_default()`: an error yields an **empty**
    /// `Vec<bool>`, the `zip` yields nothing, and every hash is silently neither reported as existing
    /// nor requested. The walk then does nothing until the idle resend nudges it — a stall that looks
    /// exactly like a pacing property. The oracle's `filterA(containsBlock)` propagates the error.
    ///
    /// Falsified in the witnessing form: with `unwrap_or_default()` restored the call neither fails
    /// nor progresses, so the bounded wait below fires and the assertion reports the stall. The bound
    /// is 5 s against a 1 s idle timeout, so a *timeout-driven* walk would have had five chances to
    /// do something and done nothing.
    #[tokio::test]
    async fn a_failed_store_read_fails_the_walk_instead_of_requesting_nothing() {
        let blocks = chain(3);
        let tip = blocks.last().expect("a non-empty chain").block_hash;
        let inner = store().await;
        let failing: BlockStore = Arc::new(FailingStore {
            inner,
            fail_puts: false,
            fail_reads: true,
        });
        let (incoming_tx, mut incoming_rx) = tokio::sync::mpsc::channel(64);
        let comm_util = comm_state(&blocks, incoming_tx);
        let fringe = FinalizedFringe {
            hashes: vec![tip],
            state_hash: StateHash::new([0u8; 32]),
        };

        let outcome = tokio::time::timeout(
            Duration::from_secs(5),
            request_blocks(
                &fringe,
                &mut incoming_rx,
                Duration::from_secs(1),
                &failing,
                &comm_util,
                &NopLog,
            ),
        )
        .await;

        match outcome {
            Ok(result) => assert!(
                result.is_err(),
                "a store whose reads fail must fail the walk, not report an empty request set"
            ),
            Err(_) => panic!(
                "the walk neither failed nor progressed: a failed `contains` read as \"nothing to \
                 request\", so it waited on the idle resend instead of surfacing the error"
            ),
        }
    }
}
