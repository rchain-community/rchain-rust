//! NodeRunning engine (port of `engine/NodeRunning.scala`).
//!
//! The message handlers wire the transport layer to the block store / DAG / block retriever: block
//! hash broadcasts and has-block messages feed the retriever, block requests are served from the
//! store, fork-choice-tip / finalized-fringe requests are served from the DAG, and store-items
//! (LFS state-sync) requests are served from the RSpace exporter.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rchain_block_storage::block_store::BlockStore;
use rchain_block_storage::dag::dag_storage::BlockDagStorage;
use rchain_comm::peer_node::PeerNode;
use rchain_comm::rp::rp_conf::RPConf;
use rchain_comm::transport::transport_layer::TransportLayer;
use rchain_comm::transport::transport_layer_syntax;
use rchain_crypto::hash::blake2b256_hash::Blake2b256Hash;
use rchain_models::block::state_hash::StateHash;
use rchain_models::block_hash::BlockHash;
use rchain_models::casper::protocol::casper_message::{
    BlockMessage, BlockRequest, CasperMessage, FinalizedFringe, HasBlock, HasBlockRequest,
    StoreItemsMessage, StoreItemsMessageRequest,
};
use rchain_models::casper::protocol::packet_type_tag::ToPacket;
use rchain_models::fringe_data::FringeData;
use rchain_rspace::state::RSpaceExporter;
use rchain_shared::log::{Log, LogSource};
use rchain_shared::refined::BlockHeight;

use crate::blocks::block_receiver::not_validated;
use crate::blocks::block_retriever::{AdmitHashReason, BlockRetriever};
use crate::protocol::casper_message_protocol::{
    BlockMessageSerde, FinalizedFringeSerde, HasBlockSerde, StoreItemsMessageSerde,
};
use crate::validator_identity::ValidatorIdentity;

/// Handle a peer-broadcast block hash (port of `handleBlockHashMessage`).
pub async fn handle_block_hash_message(
    block_retriever: &BlockRetriever,
    log: &dyn Log,
    source: LogSource,
    peer: &PeerNode,
    hash: &BlockHash,
    ignore: bool,
) {
    if ignore {
        log.debug(
            source,
            &format!("Ignoring {} hash broadcast", hash.to_hex()),
        );
    } else {
        log.debug(
            source,
            &format!(
                "Incoming BlockHashMessage {} from {}",
                hash.to_hex(),
                peer.endpoint.host
            ),
        );
        let _ = block_retriever
            .admit_hash(hash, Some(peer), AdmitHashReason::HashBroadcastRecieved)
            .await;
    }
}

/// Handle a peer reporting that it has a particular block (port of `handleHasBlockMessage`).
pub async fn handle_has_block_message(
    block_retriever: &BlockRetriever,
    log: &dyn Log,
    source: LogSource,
    peer: &PeerNode,
    hash: &BlockHash,
    ignore: bool,
) {
    if ignore {
        log.debug(
            source,
            &format!("Ignoring {} HasBlockMessage", hash.to_hex()),
        );
    } else {
        log.debug(
            source,
            &format!(
                "Incoming HasBlockMessage {} from {}",
                hash.to_hex(),
                peer.endpoint.host
            ),
        );
        let _ = block_retriever
            .admit_hash(hash, Some(peer), AdmitHashReason::HasBlockMessageReceived)
            .await;
    }
}

/// Per-peer block-request limit (requests/second). Generous enough for sync bursts while bounding
/// the outbound bandwidth a single peer can pull.
const DEFAULT_BLOCK_REQUEST_LIMIT_PER_SEC: u32 = 100;

/// Upper bound on `StoreItemsMessageRequest.take` — a peer-controlled state-sync walk is capped so
/// a `take=i32::MAX` cannot traverse/serialize the whole trie.
const MAX_STORE_ITEMS_TAKE: usize = 10_000;

/// Byte cap on a store-items page this node will serve (AUDIT C73). `MAX_STORE_ITEMS_TAKE` bounds how
/// many nodes a request may name; this bounds how much data the walk may hand back, because a node's
/// *value* size is not bounded by the request at all — the maximal `take` with fat values is tens of
/// megabytes of the responder's memory per request, repeated per peer.
///
/// **Refused, never truncated.** The requester recomputes the page it expects and requires the received
/// keys to match (`validate_state_items`), so a short page is not a smaller answer — it is a wrong state
/// claim the importer would then validate its own traversal against (the same reason the unreadable-store
/// path drops rather than serving an empty page, AUDIT C63). This handler has no error reply to send, so
/// the policy is the one it already applies to an over-large `take`: drop, log, and let the requester
/// time out and retry. The oracle has no such cap (it serves whatever it is asked), which makes this a
/// registered deviation — the responder's only defence that is not a lie.
///
/// 32 MiB is ~10× the largest *legitimate* page (`PAGE_SIZE = 750` nodes × 4 KiB items ≈ 3 MB) and below
/// the ~41 MB the maximal `take` produces at that item size, so the two caps bracket the same hostile
/// request from both sides.
pub const MAX_STORE_ITEMS_BYTES: usize = 32 * 1024 * 1024;

/// Per-peer fixed-window rate limiter for block requests (H4): bounds the outbound bandwidth a
/// single peer can pull by requesting blocks. Documented Scala deviation: Scala serves every block
/// request with no limit.
pub struct PeerRateLimiter {
    max_per_sec: u32,
    state: Mutex<BTreeMap<Vec<u8>, (Instant, u32)>>,
}

impl PeerRateLimiter {
    pub fn new(max_per_sec: u32) -> Self {
        PeerRateLimiter {
            max_per_sec,
            state: Mutex::new(BTreeMap::new()),
        }
    }

    /// Admit a request from `peer` if it is within the per-peer one-second window.
    pub fn allow(&self, peer_key: &[u8]) -> bool {
        let now = Instant::now();
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        // Prune peers whose window expired long ago, so a connection churn with many distinct node
        // ids (Kademlia-injectable) cannot grow this map without bound (R30).
        state.retain(|_, (last, _)| now.duration_since(*last) < Duration::from_secs(60));
        let entry = state.entry(peer_key.to_vec()).or_insert((now, 0));
        if now.duration_since(entry.0) >= Duration::from_secs(1) {
            entry.0 = now;
            entry.1 = 0;
        }
        entry.1 += 1;
        entry.1 <= self.max_per_sec
    }
}

/// Serve a peer's request for a block (port of `handleBlockRequest`), throttled per-peer.
pub async fn handle_block_request(
    transport: &dyn TransportLayer,
    conf: &RPConf,
    block_store: &BlockStore,
    log: &dyn Log,
    source: LogSource,
    peer: &PeerNode,
    br: &BlockRequest,
    limiter: &PeerRateLimiter,
) {
    let hash = BlockHash::from_slice(&br.hash);
    if !limiter.allow(peer.key()) {
        log.info(
            source,
            &format!(
                "Received request for block {} from {peer}. Dropped: per-peer block-request rate limit exceeded.",
                hash.to_hex()
            ),
        );
        return;
    }
    // A store that cannot be read skips the answer: an unanswered request is retryable, where a
    // `false` answer would be a state claim (AUDIT C67).
    let has_block = match block_is_known(block_store, &hash).await {
        Ok(known) => known,
        Err(e) => {
            log.error(
                source,
                &format!(
                    "Received request for block {} from {peer}. Dropped: the block store could not be read: {e}",
                    hash.to_hex()
                ),
            );
            return;
        }
    };
    if has_block {
        let stored = match block_by_hash(block_store, &hash).await {
            Ok(block) => block,
            Err(e) => {
                log.error(
                    source,
                    &format!(
                        "Received request for block {} from {peer}. Dropped: the block store could not be read: {e}",
                        hash.to_hex()
                    ),
                );
                return;
            }
        };
        if let Some(block) = stored {
            transport_layer_syntax::stream_to_peer(
                transport,
                conf,
                peer,
                BlockMessageSerde.mk_packet(&block),
            )
            .await;
        }
        log.info(
            source,
            &format!(
                "Received request for block {} from {peer}. Response sent.",
                hash.to_hex()
            ),
        );
    } else {
        log.info(
            source,
            &format!(
                "Received request for block {} from {peer}. No response given since block not found.",
                hash.to_hex()
            ),
        );
    }
}

/// Respond to a peer's has-block query (port of `handleHasBlockRequest`).
pub async fn handle_has_block_request(
    transport: &dyn TransportLayer,
    conf: &RPConf,
    peer: &PeerNode,
    hbr: &HasBlockRequest,
    has_block: bool,
) {
    if has_block {
        transport_layer_syntax::send_to_peer(
            transport,
            conf,
            peer,
            HasBlockSerde.mk_packet(&HasBlock {
                hash: hbr.hash.clone(),
            }),
        )
        .await;
    }
}

/// Respond to a peer's fork-choice-tip request (port of `handleForkChoiceTipRequest`).
pub async fn handle_fork_choice_tip_request(
    transport: &dyn TransportLayer,
    conf: &RPConf,
    dag: &dyn BlockDagStorage,
    log: &dyn Log,
    source: LogSource,
    peer: &PeerNode,
) {
    log.info(
        source,
        &format!("Received ForkChoiceTipRequest from {}", peer.endpoint.host),
    );
    let repr = dag.get_representation().await;
    let tips: Vec<BlockHash> = repr
        .dag_message_state
        .latest_msgs
        .values()
        .map(|m| m.id)
        .collect();
    for tip in &tips {
        transport_layer_syntax::send_to_peer(
            transport,
            conf,
            peer,
            HasBlockSerde.mk_packet(&HasBlock {
                hash: tip.as_bytes().to_vec(),
            }),
        )
        .await;
    }
    log.info(
        source,
        &format!(
            "Sending tips {} to {}",
            tips.iter()
                .map(|t| t.to_hex())
                .collect::<Vec<_>>()
                .join(" "),
            peer.endpoint.host
        ),
    );
}

/// Stream a finalized fringe to a peer (port of `handleFinalizedFringeRequest`).
pub async fn handle_finalized_fringe_request(
    transport: &dyn TransportLayer,
    conf: &RPConf,
    log: &dyn Log,
    source: LogSource,
    peer: &PeerNode,
    fringe: &FinalizedFringe,
) {
    log.info(
        source,
        &format!("Received FinalizedFringeRequest from {peer}"),
    );
    transport_layer_syntax::stream_to_peer(
        transport,
        conf,
        peer,
        FinalizedFringeSerde.mk_packet(fringe),
    )
    .await;
    log.info(source, &format!("FinalizedFringe sent to {peer}"));
}

/// Serve a peer's store-items (state-sync) request from the exporter (port of
/// `handleStoreItemsRequest`), unless the operator disabled the exporter.
///
/// `disable_state_exporter` is the one thing that turns this node into a non-server of state: the
/// Scala gates the whole handler on it (`NodeRunning.scala:314-320`) after logging
/// "the node is configured to not respond to StoreItemsMessage", and does nothing else. The flag's
/// point is an operator's choice about who may pull this node's trie (a state-exfiltration and
/// bandwidth-amplification surface), so the refusal must be *before* the walk and before any send.
#[allow(clippy::too_many_arguments)]
pub async fn handle_store_items_request<E: RSpaceExporter>(
    transport: &dyn TransportLayer,
    conf: &RPConf,
    exporter: &E,
    log: &dyn Log,
    log_source: LogSource,
    peer: &PeerNode,
    req: &StoreItemsMessageRequest,
    disable_state_exporter: bool,
) {
    if disable_state_exporter {
        log.info(
            log_source,
            &format!(
                "Received StoreItemsMessage request but the node is configured to not respond to \
                 StoreItemsMessage, from {}.",
                peer.endpoint.host
            ),
        );
        return;
    }
    // Validate-on-ingress: a negative skip/take would wrap to a huge `usize` via `as` and make
    // `get_nodes` iterate the whole trie.
    let (Ok(skip), Ok(take)) = (usize::try_from(req.skip), usize::try_from(req.take)) else {
        log.info(
            log_source,
            "Dropping store-items request with negative skip/take",
        );
        return;
    };
    // Bound the walk: a peer-controlled `take` of i32::MAX would traverse and serialize the whole
    // trie (state exfiltration + CPU/IO/bandwidth amplification, repeatable per peer).
    if take > MAX_STORE_ITEMS_TAKE {
        log.info(
            log_source,
            &format!("Dropping store-items request with take {take} > {MAX_STORE_ITEMS_TAKE}"),
        );
        return;
    }
    // A store the exporter cannot read is logged and dropped, the way an over-large `take` is above:
    // this handler has no error reply to send, and serving an *empty* page would be a state claim —
    // the importer would then validate its own traversal against it (AUDIT C63).
    let nodes = match exporter.get_nodes(&req.start_path, skip, take) {
        Ok(nodes) => nodes,
        Err(e) => {
            log.error(
                log_source,
                &format!("Dropping store-items request: the trie could not be read: {e}"),
            );
            return;
        }
    };
    let history_keys: Vec<Blake2b256Hash> = nodes
        .iter()
        .filter(|n| !n.is_leaf)
        .map(|n| n.hash)
        .collect();
    let data_keys: Vec<Blake2b256Hash> =
        nodes.iter().filter(|n| n.is_leaf).map(|n| n.hash).collect();
    let history_items = match exporter.get_history_items(&history_keys, |b: &[u8]| b.to_vec()) {
        Ok(items) => items,
        Err(e) => {
            log.error(
                log_source,
                &format!("Dropping store-items request: history items unreadable: {e}"),
            );
            return;
        }
    };
    let data_items = match exporter.get_data_items(&data_keys, |b: &[u8]| b.to_vec()) {
        Ok(items) => items,
        Err(e) => {
            log.error(
                log_source,
                &format!("Dropping store-items request: data items unreadable: {e}"),
            );
            return;
        }
    };
    // The byte cap (AUDIT C73): see `MAX_STORE_ITEMS_BYTES` for why this refuses rather than truncates.
    // It is checked here, after assembly and before the response exists, so an oversized page is never
    // serialised and never sent.
    let page_bytes: usize = history_items
        .iter()
        .map(|(_, v)| v.len())
        .chain(data_items.iter().map(|(_, v)| v.len()))
        .sum();
    if page_bytes > MAX_STORE_ITEMS_BYTES {
        log.error(
            log_source,
            &format!(
                "Dropping store-items request: the page is {page_bytes} bytes > \
                 {MAX_STORE_ITEMS_BYTES} — refusing rather than truncating, since a short page is a \
                 wrong state claim (the requester recomputes it)"
            ),
        );
        return;
    }
    let last_path = nodes.last().map(|n| n.path.clone()).unwrap_or_default();

    let response = StoreItemsMessage {
        start_path: req.start_path.clone(),
        last_path,
        history_items,
        data_items,
    };
    log.info(
        log_source,
        &format!(
            "Sending {} history and {} data store items to {}",
            response.history_items.len(),
            response.data_items.len(),
            peer.endpoint.host
        ),
    );
    // Stream the response (matching Scala's `streamToPeer`); the unary `send_to_peer` path does
    // not deliver to the syncing peer, which stalls LFS state sync.
    transport_layer_syntax::stream_to_peer(
        transport,
        conf,
        peer,
        StoreItemsMessageSerde.mk_packet(&response),
    )
    .await;
}

/// Bound on the inbound block queue. The channel is created upstream (node crate) with
/// `tokio::sync::mpsc::channel(MAX_PENDING_BLOCKS)`; `NodeRunning` only holds the bounded sender and
/// drops blocks (via `try_send`) rather than blocking the inbound task when it is full.
pub const MAX_PENDING_BLOCKS: usize = 1024;

/// Whether the block store holds `hash` — the `contains` half of the receiver's presence reads.
///
/// **A store that cannot be read is an error, not "unknown".** The oracle reads presence inside `F`
/// (`BlockStore[F].contains`, `NodeRunning.scala:127,231`), so a store error is an error there and its
/// caller logs it and moves on; the port's `unwrap_or_default()` answered `false` — "unknown" — and
/// each handler acted on that negative, silently. A store answering fewer presence bits than keys is
/// refused too, rather than read as "not known".
pub(crate) async fn block_is_known(
    block_store: &BlockStore,
    hash: &BlockHash,
) -> Result<bool, String> {
    let contains = block_store.contains(&[*hash]).await?;
    contains.first().copied().ok_or_else(|| {
        format!(
            "the block store answered no presence bit for {}",
            hash.to_hex()
        )
    })
}

/// The block at `hash`, if the store holds it — the `get` half of the same reads.
///
/// `Ok(None)` is a block the store genuinely does not hold; an unreadable store is an `Err`, because
/// the oracle's `getUnsafe` (`BlockStoreSyntax.scala:33-35`) lifts both into an error in `F`.
pub(crate) async fn block_by_hash(
    block_store: &BlockStore,
    hash: &BlockHash,
) -> Result<Option<BlockMessage>, String> {
    Ok(block_store
        .get(&[*hash])
        .await?
        .into_iter()
        .flatten()
        .next())
}

/// The running-state engine (port of the `NodeRunning` class): message handling and the
/// store-items (LFS state-sync) serving are ported.
pub struct NodeRunning<E: RSpaceExporter> {
    transport: Arc<dyn TransportLayer>,
    conf: RPConf,
    block_store: BlockStore,
    dag: Arc<dyn BlockDagStorage>,
    block_retriever: Arc<BlockRetriever>,
    log: Arc<dyn Log>,
    log_source: LogSource,
    validator_id: Option<ValidatorIdentity>,
    incoming_blocks: tokio::sync::mpsc::Sender<BlockMessage>,
    block_request_limit: Arc<PeerRateLimiter>,
    exporter: E,
    /// Operator switch: refuse store-items (state-sync) requests (port of the Scala's
    /// `disableStateExporter`).
    disable_state_exporter: bool,
}

impl<E: RSpaceExporter> NodeRunning<E> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        transport: Arc<dyn TransportLayer>,
        conf: RPConf,
        block_store: BlockStore,
        dag: Arc<dyn BlockDagStorage>,
        block_retriever: Arc<BlockRetriever>,
        log: Arc<dyn Log>,
        validator_id: Option<ValidatorIdentity>,
        incoming_blocks: tokio::sync::mpsc::Sender<BlockMessage>,
        exporter: E,
        disable_state_exporter: bool,
    ) -> Self {
        NodeRunning {
            transport,
            conf,
            block_store,
            dag,
            block_retriever,
            log,
            log_source: LogSource::new("casper.engine.NodeRunning"),
            validator_id,
            incoming_blocks,
            block_request_limit: Arc::new(PeerRateLimiter::new(
                DEFAULT_BLOCK_REQUEST_LIMIT_PER_SEC,
            )),
            exporter,
            disable_state_exporter,
        }
    }

    /// Handle an incoming casper message from a peer (port of `handle`).
    pub async fn handle(&self, peer: &PeerNode, msg: &CasperMessage) {
        match msg {
            CasperMessage::BlockHashMessage(bhm) => {
                let hash = bhm.block_hash;
                let ignore = match block_is_known(&self.block_store, &hash).await {
                    Ok(known) => known,
                    Err(e) => {
                        self.log.error(
                            self.log_source,
                            &format!(
                                "Dropping a block-hash message for {}: the block store could not be read: {e}",
                                hash.to_hex()
                            ),
                        );
                        return;
                    }
                };
                handle_block_hash_message(
                    &self.block_retriever,
                    self.log.as_ref(),
                    self.log_source,
                    peer,
                    &hash,
                    ignore,
                )
                .await;
            }
            CasperMessage::BlockMessage(b) => {
                if let Some(id) = &self.validator_id {
                    if b.sender.as_bytes().as_slice() == id.public_key.bytes() {
                        self.log.warn(
                            self.log_source,
                            &format!(
                                "There is another node {peer} proposing using the same private key as you. \
                                 Or did you restart your node?"
                            ),
                        );
                    }
                }
                let known = match block_is_known(&self.block_store, &b.block_hash).await {
                    Ok(known) => known,
                    Err(e) => {
                        self.log.error(
                            self.log_source,
                            &format!(
                                "Dropping a block message for {}: the block store could not be read: {e}",
                                b.block_hash.to_hex()
                            ),
                        );
                        return;
                    }
                };
                if known {
                    self.log.debug(
                        self.log_source,
                        &format!(
                            "Ignoring BlockMessage #{} from {}",
                            b.block_number, peer.endpoint.host
                        ),
                    );
                } else {
                    if self.incoming_blocks.try_send(b.clone()).is_err() {
                        self.log.warn(
                            self.log_source,
                            &format!(
                                "Block ingress queue full or closed; dropping block {} from {peer}",
                                b.block_hash.to_hex()
                            ),
                        );
                    } else {
                        self.log.debug(
                            self.log_source,
                            &format!(
                                "Incoming BlockMessage #{} from {}",
                                b.block_number, peer.endpoint.host
                            ),
                        );
                    }
                }
            }
            CasperMessage::BlockRequest(br) => {
                handle_block_request(
                    self.transport.as_ref(),
                    &self.conf,
                    &self.block_store,
                    self.log.as_ref(),
                    self.log_source,
                    peer,
                    br,
                    self.block_request_limit.as_ref(),
                )
                .await;
            }
            CasperMessage::HasBlockRequest(hbr) => {
                let repr = self.dag.get_representation().await;
                let hash = BlockHash::from_slice(&hbr.hash);
                let has_block = repr.contains(&hash);
                handle_has_block_request(self.transport.as_ref(), &self.conf, peer, hbr, has_block)
                    .await;
            }
            CasperMessage::HasBlock(hb) => {
                let hash = BlockHash::from_slice(&hb.hash);
                let known = match block_is_known(&self.block_store, &hash).await {
                    Ok(known) => known,
                    Err(e) => {
                        self.log.error(
                            self.log_source,
                            &format!(
                                "Dropping a has-block message for {}: the block store could not be read: {e}",
                                hash.to_hex()
                            ),
                        );
                        return;
                    }
                };
                if known {
                    let validated = match not_validated(&self.block_store, self.dag.as_ref(), &hash)
                        .await
                    {
                        Ok(validated) => validated,
                        Err(e) => {
                            self.log.error(
                                self.log_source,
                                &format!("Dropping a has-block message for {}: {e}", hash.to_hex()),
                            );
                            return;
                        }
                    };
                    if validated {
                        let stored = match block_by_hash(&self.block_store, &hash).await {
                            Ok(block) => block,
                            Err(e) => {
                                self.log.error(
                                    self.log_source,
                                    &format!(
                                        "Dropping a has-block message for {}: the block store could not be read: {e}",
                                        hash.to_hex()
                                    ),
                                );
                                return;
                            }
                        };
                        if let Some(block) = stored {
                            if self.incoming_blocks.try_send(block).is_err() {
                                self.log.warn(
                                    self.log_source,
                                    &format!(
                                        "Block ingress queue full or closed; dropping block {} from {peer}",
                                        hash.to_hex()
                                    ),
                                );
                            }
                        }
                    }
                } else {
                    self.log.debug(
                        self.log_source,
                        &format!(
                            "Incoming HasBlockMessage {} from {}",
                            hash.to_hex(),
                            peer.endpoint.host
                        ),
                    );
                    let _ = self
                        .block_retriever
                        .admit_hash(&hash, Some(peer), AdmitHashReason::HasBlockMessageReceived)
                        .await;
                }
            }
            CasperMessage::ForkChoiceTipRequest(_) => {
                handle_fork_choice_tip_request(
                    self.transport.as_ref(),
                    &self.conf,
                    self.dag.as_ref(),
                    self.log.as_ref(),
                    self.log_source,
                    peer,
                )
                .await;
            }
            CasperMessage::FinalizedFringeRequest(_) => {
                let repr = self.dag.get_representation().await;
                let latest_fringe_hashes: BTreeSet<BlockHash> =
                    repr.latest_fringe().iter().map(|m| m.id).collect();
                let fringe_response = if latest_fringe_hashes.is_empty() {
                    // Fresh genesis: the shard-choice fringe is empty (no finalized messages), so the
                    // "chosen" state is the genesis block (block 0). Hand it over so the syncing
                    // validator downloads block 0 + its post-state (bonds) instead of ending up
                    // unbonded with an empty DAG.
                    let genesis_hash = repr
                        .height_map
                        .get(&BlockHeight::zero())
                        .and_then(|s| s.iter().next().copied());
                    match genesis_hash {
                        Some(genesis_hash) => self
                            .block_store
                            .get(&[genesis_hash])
                            .await
                            .ok()
                            .and_then(|mut v| v.pop().flatten())
                            .map(|b| FinalizedFringe {
                                hashes: vec![genesis_hash],
                                state_hash: b.post_state_hash,
                            }),
                        None => None,
                    }
                } else {
                    repr.fringe_states
                        .get(&FringeData::fringe_hash_of(&latest_fringe_hashes))
                        .map(|fringe_data| FinalizedFringe {
                            hashes: latest_fringe_hashes.iter().copied().collect(),
                            state_hash: StateHash::from_slice(fringe_data.state_hash.as_bytes()),
                        })
                };
                if let Some(fringe_response) = fringe_response {
                    handle_finalized_fringe_request(
                        self.transport.as_ref(),
                        &self.conf,
                        self.log.as_ref(),
                        self.log_source,
                        peer,
                        &fringe_response,
                    )
                    .await;
                    self.log.info(
                        self.log_source,
                        &format!(
                            "Sent fringe response ({}).",
                            fringe_response
                                .hashes
                                .iter()
                                .map(|h| h.to_hex())
                                .collect::<Vec<_>>()
                                .join(" ")
                        ),
                    );
                }
            }
            CasperMessage::StoreItemsMessageRequest(req) => {
                handle_store_items_request(
                    self.transport.as_ref(),
                    &self.conf,
                    &self.exporter,
                    self.log.as_ref(),
                    self.log_source,
                    peer,
                    req,
                    self.disable_state_exporter,
                )
                .await;
            }
            CasperMessage::StoreItemsMessage(_) => {}
            CasperMessage::FinalizedFringe(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A block store whose every read fails, so a read error is observable as one.
    struct FailingBlockStore;

    #[async_trait::async_trait]
    impl rchain_shared::typed_store::KeyValueTypedStore<BlockHash, BlockMessage> for FailingBlockStore {
        async fn get(&self, _keys: &[BlockHash]) -> Result<Vec<Option<BlockMessage>>, String> {
            Err("the block store is down".to_string())
        }
        async fn put(&self, _pairs: &[(BlockHash, BlockMessage)]) -> Result<(), String> {
            Err("the block store is down".to_string())
        }
        async fn delete(&self, _keys: &[BlockHash]) -> Result<usize, String> {
            Err("the block store is down".to_string())
        }
        async fn contains(&self, _keys: &[BlockHash]) -> Result<Vec<bool>, String> {
            Err("the block store is down".to_string())
        }
        async fn to_map(
            &self,
        ) -> Result<std::collections::BTreeMap<BlockHash, BlockMessage>, String> {
            Err("the block store is down".to_string())
        }
    }

    /// **A store that cannot be read is not "the block is not known"** (AUDIT C67).
    ///
    /// The oracle reads presence inside `F` (`BlockStore[F].contains`, `NodeRunning.scala:127,231`),
    /// so a store error is an error there; the port's `unwrap_or_default()` answered `false` — "not
    /// known" — and the *consequence* differs per handler (a duplicate block admitted, a block
    /// re-saved, a response never sent), while the silence is the same everywhere: nothing tells the
    /// operator the block store is failing.
    ///
    /// Falsifier, both forms. Pre-fix (witnessing): the failing store was answered with `false` and
    /// `None`, and those assertions **passed on exactly that** (run 2026-09-24 before the change).
    /// Post-fix: both are `Err`, naming the failure.
    #[tokio::test]
    async fn a_store_that_cannot_be_read_is_not_an_unknown_block() {
        let block_store: BlockStore = Arc::new(FailingBlockStore);
        let hash = BlockHash::new([0x11; 32]);

        let err = block_is_known(&block_store, &hash)
            .await
            .expect_err("a block store that cannot be read must not answer \"unknown block\"");
        assert!(
            err.contains("the block store is down"),
            "and the refusal must name the failure, got: {err}"
        );
        let err = block_by_hash(&block_store, &hash)
            .await
            .expect_err("…nor \"no block\" for the read that returns it");
        assert!(
            err.contains("the block store is down"),
            "and the refusal must name the failure, got: {err}"
        );
    }
    use async_trait::async_trait;
    use rchain_block_storage::dag::codecs::{BlockHashCodec, BlockMessageCodec};
    use rchain_comm::errors::CommErr;
    use rchain_comm::peer_node::NodeIdentifier;
    use rchain_comm::rp::rp_conf::{ClearConnectionsConf, RPConf};
    use rchain_comm::transport::chunker::Blob;
    use rchain_comm::transport::transport_layer::TransportLayer;
    use rchain_models::comm::protocol::Protocol;
    use rchain_models::validator::Validator;
    use rchain_shared::log::NopLog;
    use rchain_shared::store::InMemoryKeyValueStore;
    use rchain_shared::typed_store::KeyValueTypedStoreCodec;
    use std::collections::{BTreeMap, BTreeSet};
    use std::time::Duration;

    use crate::protocol::comm_util::{CommUtil, ConnectionsCell};

    fn peer(name: &str, port: u16) -> PeerNode {
        PeerNode::from(
            NodeIdentifier::new(name.as_bytes().to_vec()),
            "host".to_string(),
            rchain_shared::refined::Port::new(port),
            rchain_shared::refined::Port::new(port),
        )
    }

    fn hash(byte: u8) -> BlockHash {
        BlockHash::new([byte; 32])
    }

    fn block(hash: BlockHash) -> BlockMessage {
        BlockMessage {
            version: 1,
            shard_id: "root".to_string(),
            block_hash: hash,
            block_number: 0.try_into().unwrap(),
            sender: Validator::new([1u8; 65]),
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

    async fn block_store(blocks: Vec<BlockMessage>) -> BlockStore {
        let store: BlockStore = Arc::new(KeyValueTypedStoreCodec::new(
            Arc::new(tokio::sync::Mutex::new(Box::new(
                InMemoryKeyValueStore::default(),
            ))),
            Arc::new(BlockHashCodec),
            Arc::new(BlockMessageCodec),
        ));
        let pairs: Vec<(BlockHash, BlockMessage)> =
            blocks.into_iter().map(|b| (b.block_hash, b)).collect();
        store.put(&pairs).await.unwrap();
        store
    }

    fn conf(local: &PeerNode) -> RPConf {
        RPConf {
            local: local.clone(),
            network_id: "testnet".to_string(),
            bootstrap: None,
            default_timeout: Duration::from_secs(10),
            max_num_of_connections: 10,
            clear_connections: ClearConnectionsConf {
                num_of_connections_pinged: 10,
            },
        }
    }

    #[derive(Default)]
    struct MockTransport {
        sends: std::sync::Mutex<Vec<(PeerNode, Protocol)>>,
        streams: std::sync::Mutex<Vec<(Vec<PeerNode>, Blob)>>,
    }

    #[async_trait]
    impl TransportLayer for MockTransport {
        async fn send(&self, peer: &PeerNode, msg: Protocol) -> CommErr<()> {
            self.sends.lock().unwrap().push((peer.clone(), msg));
            Ok(())
        }
        async fn broadcast(&self, peers: &[PeerNode], msg: Protocol) -> Vec<CommErr<()>> {
            for peer in peers {
                self.sends.lock().unwrap().push((peer.clone(), msg.clone()));
            }
            peers.iter().map(|_| Ok(())).collect()
        }
        async fn stream(&self, peers: &[PeerNode], blob: Blob) {
            self.streams.lock().unwrap().push((peers.to_vec(), blob));
        }
    }

    /// A store-items server for the gate test: one leaf and one internal node, so the handler has
    /// something to send when it is allowed to.
    struct MockExporter;

    impl rchain_shared::state::TrieExporter<Blake2b256Hash> for MockExporter {
        fn get_nodes(
            &self,
            _start_path: &[(Blake2b256Hash, Option<u8>)],
            _skip: usize,
            _take: usize,
        ) -> Result<Vec<rchain_shared::state::TrieNode<Blake2b256Hash>>, String> {
            let leaf = Blake2b256Hash::from_bytes([7u8; 32]);
            let branch = Blake2b256Hash::from_bytes([8u8; 32]);
            Ok(vec![
                rchain_shared::state::TrieNode {
                    hash: leaf,
                    is_leaf: true,
                    path: vec![],
                },
                rchain_shared::state::TrieNode {
                    hash: branch,
                    is_leaf: false,
                    path: vec![(branch, None)],
                },
            ])
        }

        fn get_history_items<Value>(
            &self,
            keys: &[Blake2b256Hash],
            from_buffer: impl Fn(&[u8]) -> Value,
        ) -> Result<Vec<(Blake2b256Hash, Value)>, String> {
            Ok(keys.iter().map(|k| (*k, from_buffer(&[1u8]))).collect())
        }

        fn get_data_items<Value>(
            &self,
            keys: &[Blake2b256Hash],
            from_buffer: impl Fn(&[u8]) -> Value,
        ) -> Result<Vec<(Blake2b256Hash, Value)>, String> {
            Ok(keys.iter().map(|k| (*k, from_buffer(&[2u8]))).collect())
        }
    }

    impl RSpaceExporter for MockExporter {
        fn get_root(&self) -> Result<Option<Blake2b256Hash>, String> {
            Ok(Some(Blake2b256Hash::from_bytes([8u8; 32])))
        }
    }

    /// A store-items server whose page is `count` nodes of `payload` bytes each — what an LFS
    /// state-sync request gets from a real trie exporter.
    struct PageExporter {
        count: usize,
        payload: usize,
    }

    impl PageExporter {
        fn node(&self, i: usize) -> rchain_shared::state::TrieNode<Blake2b256Hash> {
            let mut bytes = [0u8; 32];
            bytes[..8].copy_from_slice(&(i as u64).to_le_bytes());
            let hash = Blake2b256Hash::from_bytes(bytes);
            rchain_shared::state::TrieNode {
                hash,
                is_leaf: i % 2 == 0,
                path: vec![(hash, None)],
            }
        }

        fn items<Value>(
            &self,
            count: usize,
            keys: &[Blake2b256Hash],
            from_buffer: impl Fn(&[u8]) -> Value,
        ) -> Result<Vec<(Blake2b256Hash, Value)>, String> {
            let bytes = vec![1u8; self.payload];
            Ok(keys
                .iter()
                .take(count)
                .map(|k| (*k, from_buffer(&bytes)))
                .collect())
        }
    }

    impl rchain_shared::state::TrieExporter<Blake2b256Hash> for PageExporter {
        fn get_nodes(
            &self,
            _start_path: &[(Blake2b256Hash, Option<u8>)],
            _skip: usize,
            take: usize,
        ) -> Result<Vec<rchain_shared::state::TrieNode<Blake2b256Hash>>, String> {
            Ok((0..take.min(self.count)).map(|i| self.node(i)).collect())
        }

        fn get_history_items<Value>(
            &self,
            keys: &[Blake2b256Hash],
            from_buffer: impl Fn(&[u8]) -> Value,
        ) -> Result<Vec<(Blake2b256Hash, Value)>, String> {
            self.items(self.count, keys, from_buffer)
        }

        fn get_data_items<Value>(
            &self,
            keys: &[Blake2b256Hash],
            from_buffer: impl Fn(&[u8]) -> Value,
        ) -> Result<Vec<(Blake2b256Hash, Value)>, String> {
            self.items(self.count, keys, from_buffer)
        }
    }

    impl RSpaceExporter for PageExporter {
        fn get_root(&self) -> Result<Option<Blake2b256Hash>, String> {
            Ok(Some(Blake2b256Hash::from_bytes([8u8; 32])))
        }
    }

    /// **A measurement, not a tripwire** (AUDIT C61): what one store-items page costs the shard's
    /// dispatch loop.
    ///
    /// `handle_store_items_request` is awaited *inline* — `node_launch.rs` serves a shard's messages
    /// from one `while let Some(pm) = packet_rx.recv().await { engine.handle(..).await }` loop, and
    /// this handler walks the exporter, materialises the page, serialises it and hands it to the
    /// transport before returning (`node_running.rs`). Nothing else for that shard — a block, a
    /// fork-choice tip, another peer's request — is handled meanwhile, and the routing loop above it
    /// (`node_runtime.rs`'s `for tx in targets { tx.send(..).await }` into a 50-deep channel)
    /// backpressures on the same stall. The plan records this rather than fixing it: moving the
    /// handler off the loop is a behaviour change, and a bounded page is the cheaper answer.
    ///
    /// Measured here in a debug test build, against a page whose items are 4 KiB (a node with
    /// children), with the *response* the handler produced (so the number is the page's, not a
    /// synthetic chunker call): an LFS page (`PAGE_SIZE = 750`) and the largest page the cap admits
    /// (`MAX_STORE_ITEMS_TAKE = 10_000`). `--nocapture` prints both. The gRPC write itself is not
    /// included — `MockTransport` records the blob — so this is the shard-side cost, which is the
    /// part that holds the loop.
    #[tokio::test]
    async fn a_store_items_page_costs_the_dispatch_loop_this_long() {
        const ITEM_BYTES: usize = 4096;

        // The second case is the largest page the *byte* cap admits at this item size, not the largest
        // `take`: 10,000 nodes of 4 KiB is ~41 MB, which `MAX_STORE_ITEMS_BYTES` now refuses outright
        // rather than truncating (AUDIT C73, and its own test). Measuring a refused page would measure
        // nothing, so this is 6,000 nodes ≈ 24 MB — the biggest loop turn a peer can actually buy.
        for (label, nodes) in [("LFS page", 750usize), ("byte-capped page", 6_000)] {
            let local = peer("src", 40400);
            let remote = peer("peer", 40400);
            let transport = Arc::new(MockTransport::default());
            let exporter = PageExporter {
                count: nodes,
                payload: ITEM_BYTES,
            };

            let started = std::time::Instant::now();
            handle_store_items_request(
                transport.as_ref(),
                &conf(&local),
                &exporter,
                &NopLog,
                LogSource::new("test"),
                &remote,
                &StoreItemsMessageRequest {
                    start_path: vec![],
                    skip: 0,
                    take: i32::try_from(nodes).expect("nodes fit i32"),
                },
                false,
            )
            .await;
            let served = started.elapsed();

            let streams = transport.streams.lock().unwrap();
            let blob = &streams.last().expect("the page was streamed").1;
            let page_bytes = blob.packet.content.len();
            // The measured page is the page: one item per requested node, `ITEM_BYTES` each plus
            // the keys and the protobuf framing.
            assert!(
                page_bytes >= nodes * ITEM_BYTES,
                "{label}: served {page_bytes} B for {nodes} nodes, expected at least {}",
                nodes * ITEM_BYTES
            );
            // Chunking is part of the same loop turn (`stream_to_peer` → `TransportLayer::stream`
            // → `chunk_it`). At the configured stream limit (256 MiB) this page is one data chunk,
            // so this is what the page's copy costs inside the loop.
            let chunked = std::time::Instant::now();
            let chunks = rchain_comm::transport::chunker::chunk_it(
                "testnet",
                blob,
                usize::try_from(268_435_456i64).expect("fits usize"),
            )
            .expect("chunks");
            let chunking = chunked.elapsed();
            assert_eq!(chunks.len(), 2, "{label}: header + one data chunk");
            println!(
                "{label}: {nodes} nodes, {page_bytes} B served in {served:?} + chunked in \
                 {chunking:?} — one dispatch loop turn"
            );
        }
    }

    /// A store-items request is answered when the operator has left the exporter enabled.
    ///
    /// The two tests below are the pair the Scala's flag is for (`NodeRunning.scala:314-320`): the
    /// node either serves its trie to a peer or refuses before touching it. The refusal has to be
    /// *before* the walk and before any send — a node that refuses after traversing has already
    /// spent the bandwidth the flag exists to save.
    #[tokio::test]
    async fn store_items_request_is_served_when_the_exporter_is_enabled() {
        let local = peer("src", 40400);
        let remote = peer("peer", 40400);
        let transport = Arc::new(MockTransport::default());

        handle_store_items_request(
            transport.as_ref(),
            &conf(&local),
            &MockExporter,
            &NopLog,
            LogSource::new("test"),
            &remote,
            &StoreItemsMessageRequest {
                start_path: vec![],
                skip: 0,
                take: 10,
            },
            false,
        )
        .await;

        let streams = transport.streams.lock().unwrap();
        assert_eq!(streams.len(), 1, "one state-sync response");
        assert_eq!(streams[0].0, vec![remote.clone()]);
        assert_eq!(streams[0].1.packet.type_id, "StoreItemsMessage");
    }

    /// …and is refused, silently, when the operator has disabled the exporter — no response at all.
    #[tokio::test]
    async fn store_items_request_is_refused_when_the_exporter_is_disabled() {
        let local = peer("src", 40400);
        let remote = peer("peer", 40400);
        let transport = Arc::new(MockTransport::default());

        handle_store_items_request(
            transport.as_ref(),
            &conf(&local),
            &MockExporter,
            &NopLog,
            LogSource::new("test"),
            &remote,
            &StoreItemsMessageRequest {
                start_path: vec![],
                skip: 0,
                take: 10,
            },
            true,
        )
        .await;

        assert!(
            transport.streams.lock().unwrap().is_empty(),
            "a node with `disable-state-exporter` must not stream its trie to a peer"
        );
    }

    #[tokio::test]
    async fn handle_block_request_streams_block_when_present() {
        let local = peer("src", 40400);
        let remote = peer("peer", 40400);
        let transport = Arc::new(MockTransport::default());
        let h = hash(1);
        let store = block_store(vec![block(h)]).await;
        let log = NopLog;

        handle_block_request(
            transport.as_ref(),
            &conf(&local),
            &store,
            &log,
            LogSource::new("test"),
            &remote,
            &BlockRequest {
                hash: h.as_bytes().to_vec(),
            },
            &PeerRateLimiter::new(100),
        )
        .await;

        let streams = transport.streams.lock().unwrap();
        assert_eq!(streams.len(), 1);
        assert_eq!(streams[0].0, vec![remote.clone()]);
        assert_eq!(streams[0].1.packet.type_id, "BlockMessage");
    }

    #[tokio::test]
    async fn handle_block_request_does_not_stream_absent_block() {
        let local = peer("src", 40400);
        let remote = peer("peer", 40400);
        let transport = Arc::new(MockTransport::default());
        let store = block_store(vec![]).await;
        let log = NopLog;

        handle_block_request(
            transport.as_ref(),
            &conf(&local),
            &store,
            &log,
            LogSource::new("test"),
            &remote,
            &BlockRequest {
                hash: hash(1).as_bytes().to_vec(),
            },
            &PeerRateLimiter::new(100),
        )
        .await;

        assert!(transport.streams.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn handle_has_block_request_sends_has_block_when_present() {
        let local = peer("src", 40400);
        let remote = peer("peer", 40400);
        let transport = Arc::new(MockTransport::default());

        handle_has_block_request(
            transport.as_ref(),
            &conf(&local),
            &remote,
            &HasBlockRequest {
                hash: hash(1).as_bytes().to_vec(),
            },
            true,
        )
        .await;

        let sends = transport.sends.lock().unwrap();
        assert_eq!(sends.len(), 1);
        assert_eq!(sends[0].0, remote);
        let packet = rchain_comm::rp::protocol_helper::to_packet(&sends[0].1).unwrap();
        assert_eq!(packet.type_id, "HasBlock");
    }

    #[tokio::test]
    async fn handle_has_block_request_sends_nothing_when_absent() {
        let local = peer("src", 40400);
        let remote = peer("peer", 40400);
        let transport = Arc::new(MockTransport::default());

        handle_has_block_request(
            transport.as_ref(),
            &conf(&local),
            &remote,
            &HasBlockRequest {
                hash: hash(1).as_bytes().to_vec(),
            },
            false,
        )
        .await;

        assert!(transport.sends.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn handle_has_block_message_requests_unknown_block_from_peer() {
        let local = peer("src", 40400);
        let remote = peer("peer", 40400);
        let transport = Arc::new(MockTransport::default());
        let connections: ConnectionsCell = Arc::new(tokio::sync::RwLock::new(Vec::new()));
        let comm_util = Arc::new(CommUtil::new(
            transport.clone(),
            conf(&local),
            connections,
            Arc::new(NopLog),
        ));
        let retriever = BlockRetriever::new(comm_util, Arc::new(NopLog));

        handle_has_block_message(
            &retriever,
            &NopLog,
            LogSource::new("test"),
            &remote,
            &hash(1),
            false,
        )
        .await;

        let sends = transport.sends.lock().unwrap();
        assert_eq!(sends.len(), 1);
        assert_eq!(sends[0].0, remote);
        let packet = rchain_comm::rp::protocol_helper::to_packet(&sends[0].1).unwrap();
        assert_eq!(packet.type_id, "BlockRequest");
    }
    /// **AUDIT C73**: a page over the byte cap is refused — dropped, never truncated.
    ///
    /// `MAX_STORE_ITEMS_TAKE` bounds how many nodes a request may name; nothing bounded how much data
    /// they carry, so the maximal `take` with fat values is tens of megabytes of the responder's memory
    /// per request, per peer. The page is now dropped rather than shortened: the requester recomputes
    /// the page it expects and requires the received keys to match (`validate_state_items`), so a short
    /// page is a wrong state claim rather than a smaller answer — the same policy this handler already
    /// applies to an unreadable store (C63) and to an over-large `take`.
    ///
    /// Both directions in one test, so neither can pass vacuously: the over-cap page must not be sent,
    /// and a legitimate page must still be served (the drop cannot be "this handler stopped working").
    /// Falsified against this tree: with the cap's check removed, the over-cap page is streamed and the
    /// first assertion fails.
    #[tokio::test]
    async fn a_page_over_the_byte_cap_is_dropped_not_truncated() {
        let local = peer("src", 40400);
        let remote = peer("peer", 40400);

        // Over the cap: 200 nodes × 200 KiB items, split about evenly into history and data.
        const FAT_NODES: usize = 200;
        const FAT_ITEM: usize = 200 * 1024;
        let fat_bytes = FAT_NODES
            .checked_mul(FAT_ITEM)
            .expect("the fixture's page size is small");
        assert!(
            fat_bytes > MAX_STORE_ITEMS_BYTES,
            "the fixture must exceed the cap ({fat_bytes} bytes), or this test proves nothing"
        );
        let transport = Arc::new(MockTransport::default());
        handle_store_items_request(
            transport.as_ref(),
            &conf(&local),
            &PageExporter {
                count: FAT_NODES,
                payload: FAT_ITEM,
            },
            &NopLog,
            LogSource::new("test"),
            &remote,
            &StoreItemsMessageRequest {
                start_path: vec![],
                skip: 0,
                take: i32::try_from(FAT_NODES).expect("nodes fit i32"),
            },
            false,
        )
        .await;
        assert!(
            transport.streams.lock().unwrap().is_empty(),
            "an over-cap page must be dropped: a truncated page is a wrong state claim, and the peer \
             cannot tell it from a real one"
        );

        // Under the cap: the largest legitimate page (`PAGE_SIZE = 750` × 4 KiB ≈ 3 MB) is served.
        let transport = Arc::new(MockTransport::default());
        handle_store_items_request(
            transport.as_ref(),
            &conf(&local),
            &PageExporter {
                count: 750,
                payload: 4096,
            },
            &NopLog,
            LogSource::new("test"),
            &remote,
            &StoreItemsMessageRequest {
                start_path: vec![],
                skip: 0,
                take: 750,
            },
            false,
        )
        .await;
        assert_eq!(
            transport.streams.lock().unwrap().len(),
            1,
            "a legitimate page must still be served — otherwise the drop above is the handler \
             refusing everything"
        );
    }
}
