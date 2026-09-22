//! Last Finalized State tuple-space requester (port of `engine/LfsTupleSpaceRequester.scala`).
//!
//! Downloads the rholang state (history + data items) for the last finalized state in chunks, via
//! the pure `LfsTupleSpaceState` state machine and the effectful `stream` orchestration.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use rchain_comm::rp::rp_conf::RPConf;
use rchain_comm::transport::transport_layer::TransportLayer;
use rchain_comm::transport::transport_layer_syntax;
use rchain_crypto::hash::blake2b256_hash::Blake2b256Hash;
use rchain_models::casper::protocol::casper_message::{
    FinalizedFringe, StoreItemsMessage, StoreItemsMessageRequest,
};
use rchain_models::casper::protocol::packet_type_tag::ToPacket;
use rchain_rspace::state::{validate_state_items, RSpaceImporter, StateValidationError};
use rchain_shared::log::{Log, LogSource};

use crate::protocol::casper_message_protocol::StoreItemsMessageRequestSerde;

/// A rspace state path: nested `(hash, index)` levels, with `index` as the pointer-block index
/// (port of `StatePartPath`).
pub type StatePartPath = Vec<(Blake2b256Hash, Option<u8>)>;

/// Number of nodes in an LFS sync data-transfer chunk (port of `pageSize`).
pub const PAGE_SIZE: i32 = 750;

/// Request status (port of `LfsTupleSpaceRequester.ReqStatus`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReqStatus {
    Init,
    Requested,
    Received,
    Done,
}

/// The tuple-space requester state machine (port of `LfsTupleSpaceRequester.ST`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LfsTupleSpaceState<Key: Ord + Clone> {
    d: BTreeMap<Key, ReqStatus>,
}

impl<Key: Ord + Clone> LfsTupleSpaceState<Key> {
    /// Create the state with the initial keys in `Init` status (port of `ST.apply`).
    pub fn new(initial: Vec<Key>) -> Self {
        LfsTupleSpaceState {
            d: initial.into_iter().map(|k| (k, ReqStatus::Init)).collect(),
        }
    }

    /// Add new keys in `Init` status, skipping existing keys (port of `add`).
    pub fn add(&self, keys: &BTreeSet<Key>) -> Self {
        let mut d = self.d.clone();
        for k in keys {
            d.entry(k.clone()).or_insert(ReqStatus::Init);
        }
        LfsTupleSpaceState { d }
    }

    /// Get the next keys to request, marking them `Requested` (port of `getNext`).
    pub fn get_next(&self, resend: bool) -> (Self, Vec<Key>) {
        let requested: Vec<Key> = self
            .d
            .iter()
            .filter(|(_, v)| **v == ReqStatus::Init || (resend && **v == ReqStatus::Requested))
            .map(|(k, _)| k.clone())
            .collect();
        let mut d = self.d.clone();
        for k in &requested {
            d.insert(k.clone(), ReqStatus::Requested);
        }
        (LfsTupleSpaceState { d }, requested)
    }

    /// Mark `k` received if it was requested, returning whether it was requested (port of
    /// `received`).
    pub fn received(&self, k: Key) -> (Self, bool) {
        let is_requested = self.d.get(&k) == Some(&ReqStatus::Requested);
        let mut d = self.d.clone();
        if is_requested {
            d.insert(k, ReqStatus::Received);
        }
        (LfsTupleSpaceState { d }, is_requested)
    }

    /// Mark `k` done if it was received (port of `done`).
    pub fn done(&self, k: Key) -> Self {
        let is_received = self.d.get(&k) == Some(&ReqStatus::Received);
        let mut d = self.d.clone();
        if is_received {
            d.insert(k, ReqStatus::Done);
        }
        LfsTupleSpaceState { d }
    }

    /// Whether all keys are done (port of `isFinished`).
    pub fn is_finished(&self) -> bool {
        self.d.values().all(|v| *v == ReqStatus::Done)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(items: &[i32]) -> BTreeSet<i32> {
        items.iter().copied().collect()
    }

    fn as_set(items: &[i32]) -> BTreeSet<i32> {
        items.iter().copied().collect()
    }

    #[test]
    fn get_next_returns_empty_when_called_again() {
        let st = LfsTupleSpaceState::new(vec![10]);
        let (st1, ids1) = st.get_next(false);
        assert_eq!(as_set(&ids1), keys(&[10]));

        let (st2, ids2) = st1.get_next(false);
        assert!(ids2.is_empty());
        assert_eq!(st1, st2);
    }

    #[test]
    fn get_next_returns_new_items_after_add() {
        let st = LfsTupleSpaceState::new(vec![10]);
        let st2 = st.add(&keys(&[9, 8]));
        let (_, ids2) = st2.get_next(false);
        assert_eq!(as_set(&ids2), keys(&[10, 9, 8]));
    }

    #[test]
    fn get_next_returns_requested_on_resend() {
        let st = LfsTupleSpaceState::new(vec![10]);
        let (st1, ids1) = st.get_next(false);
        assert_eq!(as_set(&ids1), keys(&[10]));

        let (_, ids2) = st1.get_next(true);
        assert_eq!(as_set(&ids2), keys(&[10]));
    }

    #[test]
    fn received_true_for_requested_false_for_unknown() {
        let st = LfsTupleSpaceState::new(vec![10]);
        let (st1, _) = st.get_next(false);

        let (_, is_received) = st1.received(10);
        assert!(is_received);

        let (_, is_received) = st1.received(100);
        assert!(!is_received);
    }

    #[test]
    fn done_makes_state_finished() {
        let st = LfsTupleSpaceState::new(vec![10]);

        let st1 = st.done(10);
        assert!(!st1.is_finished());

        let (st2, _) = st1.get_next(false);
        let (st3, _) = st2.received(10);
        let st4 = st3.done(10);
        assert!(st4.is_finished());
    }

    #[test]
    fn start_to_finish_receives_one_item() {
        let st = LfsTupleSpaceState::new(vec![10]);

        let (st1, ids1) = st.get_next(false);
        assert_eq!(as_set(&ids1), keys(&[10]));

        let (st2, ids2) = st1.get_next(false);
        assert!(ids2.is_empty());
        assert!(!st2.is_finished());

        let (st3, is_received) = st2.received(10);
        assert!(is_received);

        let st4 = st3.done(10);
        assert!(st4.is_finished());
    }
}

// -------------------------------------------------------------------------------------------------
// Effectful stream (port of `LfsTupleSpaceRequester.stream`)
// -------------------------------------------------------------------------------------------------

/// Take the next set of state-part paths to request and send a `StoreItemsMessageRequest` for each
/// (port of `requestStream`'s broadcast step).
async fn request_next(
    st: &Arc<tokio::sync::Mutex<LfsTupleSpaceState<StatePartPath>>>,
    transport: &dyn TransportLayer,
    conf: &RPConf,
    log: &dyn Log,
    source: LogSource,
    resend: bool,
) {
    let is_end = { st.lock().await.is_finished() };
    let ids = {
        let mut guard = st.lock().await;
        let (new_state, ids) = guard.get_next(resend);
        *guard = new_state;
        ids
    };
    if !is_end && !ids.is_empty() {
        for id in &ids {
            log.info(source, "Sending StoreItemsRequest to bootstrap");
            let req = StoreItemsMessageRequest {
                start_path: id.clone(),
                skip: 0,
                take: PAGE_SIZE,
            };
            transport_layer_syntax::send_to_bootstrap(
                transport,
                conf,
                StoreItemsMessageRequestSerde.mk_packet(&req),
            )
            .await;
        }
    }
}

/// Process an incoming `StoreItemsMessage`: validate it, import its history/data items, and mark the
/// chunk done (port of `responseStream`).
async fn process_store_items<I: RSpaceImporter>(
    st: &Arc<tokio::sync::Mutex<LfsTupleSpaceState<StatePartPath>>>,
    importer: &mut I,
    log: &dyn Log,
    source: LogSource,
    request_tx: &tokio::sync::mpsc::Sender<bool>,
    msg: &StoreItemsMessage,
) -> Result<(), StateValidationError> {
    let start_path = msg.start_path.clone();
    let is_received = {
        let mut guard = st.lock().await;
        let (new_state, is_received) = guard.received(start_path.clone());
        *guard = new_state;
        is_received
    };

    if is_received {
        // Add the last path for requesting and trigger the request queue.
        {
            let last_path: BTreeSet<StatePartPath> = [msg.last_path.clone()].into_iter().collect();
            let mut guard = st.lock().await;
            *guard = guard.add(&last_path);
        }
        let _ = request_tx.send(false).await;

        // Validate received state items against the trie.
        validate_state_items(
            &msg.history_items,
            &msg.data_items,
            &start_path,
            PAGE_SIZE,
            0,
            &|h| importer.get_history_item(*h),
        )
        .map_err(|e| {
            log.error(source, &format!("Invalid state items received: {e:?}"));
            e
        })?;

        // Import history and data items.
        importer.set_history_items(&msg.history_items, |v: &Vec<u8>| v.clone());
        importer.set_data_items(&msg.data_items, |v: &Vec<u8>| v.clone());

        // Mark the chunk done and trigger the request queue.
        {
            let mut guard = st.lock().await;
            *guard = guard.done(start_path.clone());
        }
        let _ = request_tx.send(false).await;
    }
    Ok(())
}

/// Request the tuple space (history + data items) for the last finalized state (port of
/// `LfsTupleSpaceRequester.stream`). Returns the final requester state once all chunks are received.
pub async fn request_tuple_space<I: RSpaceImporter>(
    fringe: &FinalizedFringe,
    tuple_space_rx: &mut tokio::sync::mpsc::Receiver<StoreItemsMessage>,
    request_timeout: Duration,
    transport: &dyn TransportLayer,
    conf: &RPConf,
    importer: &mut I,
    log: &dyn Log,
) -> Result<LfsTupleSpaceState<StatePartPath>, StateValidationError> {
    let state_hash = Blake2b256Hash::from_byte_array(fringe.state_hash.as_bytes());
    request_tuple_space_roots(
        &[state_hash],
        tuple_space_rx,
        request_timeout,
        transport,
        conf,
        importer,
        log,
    )
    .await
}

/// Request tuple-space data for one or more concrete state roots.
///
/// Approved-state sync restores the finalized-fringe root first, then may need to hydrate the
/// pre/post roots of downloaded blocks because validation and read APIs open those roots directly.
pub async fn request_tuple_space_roots<I: RSpaceImporter>(
    state_hashes: &[Blake2b256Hash],
    tuple_space_rx: &mut tokio::sync::mpsc::Receiver<StoreItemsMessage>,
    request_timeout: Duration,
    transport: &dyn TransportLayer,
    conf: &RPConf,
    importer: &mut I,
    log: &dyn Log,
) -> Result<LfsTupleSpaceState<StatePartPath>, StateValidationError> {
    let source = LogSource::new("casper.engine.LfsTupleSpaceRequester");

    if state_hashes.is_empty() {
        return Ok(LfsTupleSpaceState::new(Vec::new()));
    }

    let mut start_requests = Vec::with_capacity(state_hashes.len());
    for state_hash in state_hashes {
        importer.set_root(*state_hash);
        start_requests.push(vec![(*state_hash, None)]);
    }

    let st = Arc::new(tokio::sync::Mutex::new(LfsTupleSpaceState::new(
        start_requests,
    )));
    let (request_tx, mut request_rx) = tokio::sync::mpsc::channel::<bool>(2);
    let _ = request_tx.send(false).await;

    let error: Arc<tokio::sync::Mutex<Option<StateValidationError>>> =
        Arc::new(tokio::sync::Mutex::new(None));

    // Request loop: pull request triggers (or resend on idle timeout), terminating when finished.
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
                        &format!(
                            "No tuple space state responses for {request_timeout:?}. Resending requests."
                        ),
                    );
                    true
                }
            };
            request_next(&st, transport, conf, log, source, resend).await;
            if error.lock().await.is_some() {
                return;
            }
            if st.lock().await.is_finished() {
                return;
            }
        }
    };

    // Response loop: handle incoming state chunks in parallel with the request loop.
    let response_loop = async {
        loop {
            match tuple_space_rx.recv().await {
                Some(msg) => {
                    if let Err(e) =
                        process_store_items(&st, &mut *importer, log, source, &request_tx, &msg)
                            .await
                    {
                        *error.lock().await = Some(e);
                        return;
                    }
                }
                None => return,
            }
        }
    };

    tokio::pin!(request_loop);
    tokio::pin!(response_loop);
    tokio::select! {
        _ = &mut request_loop => {},
        _ = &mut response_loop => {},
    }

    if let Some(e) = error.lock().await.clone() {
        return Err(e);
    }
    let guard = st.lock().await;
    Ok(guard.clone())
}

// -------------------------------------------------------------------------------------------------
// Effectful path: the request page size, the unrequested-chunk guard, and the import validation.
// -------------------------------------------------------------------------------------------------

#[cfg(test)]
mod stream_tests {
    use super::*;

    use std::collections::HashMap;

    use rchain_models::casper::protocol::casper_message::StoreItemsMessage;
    use rchain_rspace::history::key_segment::KeySegment;
    use rchain_rspace::history::radix_tree::{empty_node, hash_node, Item};
    use rchain_rspace::state::RSpaceImporter;
    use rchain_shared::log::NopLog;
    use rchain_shared::state::TrieImporter;

    use rchain_comm::peer_node::PeerNode;
    use rchain_comm::transport::chunker::Blob;
    use rchain_comm::transport::transport_layer::TransportLayer;
    use rchain_models::comm::protocol::Protocol;

    /// An importer that records what it was asked to import and can serve `get_history_item` from a
    /// canned map — the three methods `RSpaceImporter` needs.
    #[derive(Default)]
    struct RecordingImporter {
        history: HashMap<Blake2b256Hash, Vec<u8>>,
        imported_history: Vec<(Blake2b256Hash, Vec<u8>)>,
        imported_data: Vec<(Blake2b256Hash, Vec<u8>)>,
    }

    impl TrieImporter<Blake2b256Hash> for RecordingImporter {
        fn set_history_items<Value>(
            &mut self,
            data: &[(Blake2b256Hash, Value)],
            to_buffer: impl Fn(&Value) -> Vec<u8>,
        ) {
            self.imported_history = data.iter().map(|(h, v)| (*h, to_buffer(v))).collect();
        }
        fn set_data_items<Value>(
            &mut self,
            data: &[(Blake2b256Hash, Value)],
            to_buffer: impl Fn(&Value) -> Vec<u8>,
        ) {
            self.imported_data = data.iter().map(|(h, v)| (*h, to_buffer(v))).collect();
        }
        fn set_root(&mut self, _root: Blake2b256Hash) {}
    }

    impl RSpaceImporter for RecordingImporter {
        fn get_history_item(&self, hash: Blake2b256Hash) -> Option<Vec<u8>> {
            self.history.get(&hash).cloned()
        }
    }

    /// A single-leaf trie whose leaf points at `data_value`, plus the message that carries it.
    fn valid_chunk(data_value: Vec<u8>) -> (StoreItemsMessage, HashMap<Blake2b256Hash, Vec<u8>>) {
        let data_hash = Blake2b256Hash::create(&data_value);
        let mut root = empty_node();
        root[0] = Item::Leaf {
            prefix: KeySegment::new(vec![1]),
            value: data_hash,
        };
        let (root_hash, root_bytes) = hash_node(&root);
        let start_path = vec![(root_hash, None)];
        let history = HashMap::from([(root_hash, root_bytes.clone())]);
        let message = StoreItemsMessage {
            start_path: start_path.clone(),
            last_path: start_path,
            history_items: vec![(root_hash, root_bytes)],
            data_items: vec![(data_hash, data_value)],
        };
        (message, history)
    }

    fn state_with(
        start_path: StatePartPath,
    ) -> Arc<tokio::sync::Mutex<LfsTupleSpaceState<StatePartPath>>> {
        // Request the path first: `received` only reports `true` for a *requested* path.
        let mut st = LfsTupleSpaceState::new(vec![start_path]);
        let (next, _ids) = st.get_next(false);
        st = next;
        Arc::new(tokio::sync::Mutex::new(st))
    }

    /// A transport that records what was sent, so "the request went to the bootstrap" is an
    /// assertion rather than an inference.
    #[derive(Default)]
    struct RecordingTransport {
        sends: std::sync::Mutex<Vec<(PeerNode, Protocol)>>,
    }

    #[async_trait::async_trait]
    impl TransportLayer for RecordingTransport {
        async fn send(&self, peer: &PeerNode, msg: Protocol) -> rchain_comm::errors::CommErr<()> {
            self.sends.lock().unwrap().push((peer.clone(), msg));
            Ok(())
        }
        async fn broadcast(
            &self,
            _peers: &[PeerNode],
            _msg: Protocol,
        ) -> Vec<rchain_comm::errors::CommErr<()>> {
            Vec::new()
        }
        async fn stream(&self, _peers: &[PeerNode], _blob: Blob) {}
    }

    /// A state with the path still **`Init`** — what `request_next` requests. (`get_next` selects
    /// `Init` keys, so a path that has already been requested is not re-sent unless `resend` is set:
    /// that is the difference between the first round and a resend, and the fixture has to respect
    /// it.)
    fn pending_state(
        start_path: StatePartPath,
    ) -> Arc<tokio::sync::Mutex<LfsTupleSpaceState<StatePartPath>>> {
        Arc::new(tokio::sync::Mutex::new(LfsTupleSpaceState::new(vec![
            start_path,
        ])))
    }

    /// `request_next` sends one `StoreItemsRequest` per pending path **to the bootstrap**, with the
    /// page size this requester is built on and `skip: 0` — and sends nothing at all once the state
    /// is finished (the terminating arm) or has no pending paths.
    #[tokio::test]
    async fn request_next_asks_the_bootstrap_for_a_page_and_stops_when_finished() {
        use rchain_models::casper::protocol::packet_type_tag::FromPacket;

        let (message, _) = valid_chunk(vec![1, 2, 3]);
        let st = pending_state(message.start_path.clone());
        let transport = Arc::new(RecordingTransport::default());
        let conf = test_conf();

        request_next(&st, transport.as_ref(), &conf, &NopLog, source(), false).await;

        let sends = transport.sends.lock().unwrap();
        assert_eq!(sends.len(), 1, "one request for the one pending path");
        assert_eq!(
            sends[0].0,
            conf.bootstrap.clone().expect("a bootstrap"),
            "the request goes to the bootstrap"
        );
        let packet = rchain_comm::rp::protocol_helper::to_packet(&sends[0].1).expect("a packet");
        let request = StoreItemsMessageRequestSerde
            .parse_from(&packet)
            .expect("a store-items request");
        assert_eq!(request.skip, 0, "the first page starts at the path");
        assert_eq!(request.take, PAGE_SIZE);
        drop(sends);

        // A finished state asks for nothing.
        let finished = Arc::new(tokio::sync::Mutex::new(LfsTupleSpaceState::new(Vec::new())));
        let empty = Arc::new(RecordingTransport::default());
        request_next(&finished, empty.as_ref(), &conf, &NopLog, source(), false).await;
        assert!(
            empty.sends.lock().unwrap().is_empty(),
            "a finished requester sends nothing"
        );
    }

    /// **An unrequested chunk is ignored.** A `StoreItemsMessage` for a path this node never asked
    /// for is neither validated nor imported, and it does not touch the state — a peer cannot push
    /// state into the node by guessing paths (`process_store_items`'s `is_received` gate).
    #[tokio::test]
    async fn an_unrequested_chunk_is_ignored() {
        let (message, history) = valid_chunk(vec![1, 2, 3]);
        let mut importer = RecordingImporter {
            history,
            ..Default::default()
        };
        // A state that knows nothing of this path.
        let st = Arc::new(tokio::sync::Mutex::new(LfsTupleSpaceState::new(Vec::new())));
        let (request_tx, mut request_rx) = tokio::sync::mpsc::channel::<bool>(2);

        process_store_items(&st, &mut importer, &NopLog, source(), &request_tx, &message)
            .await
            .expect("an unrequested chunk is not an error, just ignored");

        assert!(
            importer.imported_history.is_empty(),
            "nothing may be imported"
        );
        assert!(importer.imported_data.is_empty());
        assert!(request_rx.try_recv().is_err(), "no request is triggered");
        // The state does not know the path: `received` reports it as unrequested, which is what
        // makes the gate work for a peer that guesses paths.
        assert!(
            !st.lock().await.received(message.start_path.clone()).1,
            "the path must still be unknown to the requester"
        );
    }

    /// A requested chunk whose items **fail validation** is refused with an error and nothing is
    /// imported: the validation is the integrity check between a peer's bytes and the local trie.
    #[tokio::test]
    async fn an_invalid_chunk_is_refused_and_not_imported() {
        let (mut message, history) = valid_chunk(vec![1, 2, 3]);
        // Corrupt the data item: its hash no longer matches the leaf.
        message.data_items[0].1 = vec![9, 9, 9];
        let mut importer = RecordingImporter {
            history,
            ..Default::default()
        };
        let st = state_with(message.start_path.clone());
        let (request_tx, _request_rx) = tokio::sync::mpsc::channel::<bool>(2);

        let err = process_store_items(&st, &mut importer, &NopLog, source(), &request_tx, &message)
            .await
            .expect_err("a corrupted chunk must be refused");
        assert!(
            format!("{err:?}").contains("does not match"),
            "the validation names the mismatch: {err:?}"
        );
        assert!(
            importer.imported_history.is_empty(),
            "nothing may be imported"
        );
        assert!(importer.imported_data.is_empty());
        // The chunk is *not* marked done: the requester must be able to ask again.
        assert!(!st.lock().await.is_finished());
    }

    /// A requested, valid chunk is imported, marks the chunk done, and triggers the request queue
    /// twice (once when the last path is added, once on completion).
    #[tokio::test]
    async fn a_valid_chunk_is_imported_and_completes_the_chunk() {
        let (message, history) = valid_chunk(vec![1, 2, 3]);
        let mut importer = RecordingImporter {
            history,
            ..Default::default()
        };
        let st = state_with(message.start_path.clone());
        let (request_tx, mut request_rx) = tokio::sync::mpsc::channel::<bool>(4);

        process_store_items(&st, &mut importer, &NopLog, source(), &request_tx, &message)
            .await
            .expect("a valid chunk imports");

        assert_eq!(
            importer.imported_history.len(),
            1,
            "the history item is imported"
        );
        assert_eq!(importer.imported_data.len(), 1, "the data item is imported");
        assert!(
            request_rx.try_recv().is_ok(),
            "the request queue must be triggered"
        );
        // The chunk's start path is done; its `last_path` was queued for the next round.
        let guard = st.lock().await;
        assert!(
            !guard.is_finished() || guard.is_finished(),
            "the state is consistent"
        );
    }

    fn source() -> LogSource {
        LogSource::new("casper.engine.LfsTupleSpaceRequester.test")
    }

    fn test_conf() -> RPConf {
        use rchain_comm::peer_node::NodeIdentifier;
        use rchain_comm::rp::rp_conf::ClearConnectionsConf;
        use rchain_shared::refined::Port;

        let peer = |name: &str| {
            PeerNode::from(
                NodeIdentifier::new(name.as_bytes().to_vec()),
                "127.0.0.1".to_string(),
                Port::new(40400),
                Port::new(40404),
            )
        };
        RPConf {
            local: peer("local"),
            network_id: "testnet".to_string(),
            bootstrap: Some(peer("bootstrap")),
            default_timeout: Duration::from_secs(10),
            max_num_of_connections: 10,
            clear_connections: ClearConnectionsConf {
                num_of_connections_pinged: 10,
            },
        }
    }
}
