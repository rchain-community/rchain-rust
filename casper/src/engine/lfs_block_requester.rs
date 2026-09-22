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
async fn save_block(
    st: &Arc<tokio::sync::Mutex<St>>,
    block_store: &BlockStore,
    block: &BlockMessage,
) {
    let already_saved = block_store
        .contains(&[block.block_hash])
        .await
        .unwrap_or_default()
        .first()
        .copied()
        .unwrap_or(false);
    if !already_saved {
        let _ = block_store.put(&[(block.block_hash, block.clone())]).await;
    }
    let mut guard = st.lock().await;
    *guard = guard.done(&block.block_hash);
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
) {
    let is_valid = validate_received_block(st, block, log, source).await;
    if is_valid {
        save_block(st, block_store, block).await;
    }
    // Trigger the request queue (without resending already-requested blocks).
    let _ = request_tx.send(false).await;
}

/// Take the next set of hashes to request, enqueue existing ones for processing, and broadcast
/// requests for the missing ones (port of `requestNext`).
async fn request_next(
    st: &Arc<tokio::sync::Mutex<St>>,
    response_hash_tx: &tokio::sync::mpsc::UnboundedSender<BlockHash>,
    block_store: &BlockStore,
    comm_util: &CommUtil,
    resend: bool,
) {
    let is_end = { st.lock().await.is_finished() };
    let hashes = {
        let mut guard = st.lock().await;
        let (new_state, hashes) = guard.get_next(resend);
        *guard = new_state;
        hashes
    };

    let hashes_vec: Vec<BlockHash> = hashes.iter().copied().collect();
    let contains = block_store.contains(&hashes_vec).await.unwrap_or_default();
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
) -> St {
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
                    None => return,
                },
                _ = tokio::time::sleep(request_timeout) => {
                    log.warn(
                        source,
                        &format!("No block responses for {request_timeout:?}. Resending requests."),
                    );
                    true
                }
            };
            request_next(&st, &response_hash_tx, block_store, comm_util, resend).await;
            if st.lock().await.is_finished() {
                return;
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
                            process_block(&st, block_store, log, source, &request_tx, &block).await;
                        }
                        None => return,
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
                                process_block(&st, block_store, log, source, &request_tx, &block).await;
                            }
                        }
                        None => return,
                    }
                }
            }
        }
    };

    tokio::pin!(request_loop);
    tokio::pin!(response_loop);
    tokio::select! {
        _ = &mut request_loop => {},
        _ = &mut response_loop => {},
    }

    let guard = st.lock().await;
    guard.clone()
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
    use rchain_shared::typed_store::KeyValueTypedStoreCodec;

    use crate::proto_util::hash_block;

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
        save_block(&st, &block_store, &block).await;
        save_block(&st, &block_store, &block).await;

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
}
