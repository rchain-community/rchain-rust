//! Casper comm utilities (port of `protocol/CommUtil.scala`).
//!
//! `CommUtil` is a thin facade over the transport layer that broadcasts packets to (a random
//! subset of) the current connections, streams blobs, retries sends, and requests blocks. The
//! Scala `ConnectionsCell.random` becomes [`random_connections`](rchain_comm::rp::connect::random_connections).

use std::sync::Arc;
use std::time::Duration;

use rchain_comm::peer_node::PeerNode;
use rchain_comm::rp::connect::random_connections;
use rchain_comm::rp::protocol_helper;
use rchain_comm::rp::rp_conf::RPConf;
use rchain_comm::transport::chunker::Blob;
use rchain_comm::transport::transport_layer::TransportLayer;
use rchain_comm::transport::transport_layer_syntax;
use rchain_models::block_hash::BlockHash;
use rchain_models::casper::protocol::casper_message::{
    BlockHashMessage, BlockRequest, FinalizedFringeRequest, ForkChoiceTipRequest, HasBlockRequest,
};
use rchain_models::casper::protocol::packet_type_tag::ToPacket;
use rchain_models::comm::protocol::{Packet, Protocol};
use rchain_shared::log::{Log, LogSource};

use crate::protocol::casper_message_protocol::{
    BlockHashMessageSerde, BlockRequestSerde, FinalizedFringeRequestSerde,
    ForkChoiceTipRequestSerde, HasBlockRequestSerde,
};

/// A shared, mutable list of current connections (port of `ConnectionsCell[F]`).
pub type ConnectionsCell = Arc<tokio::sync::RwLock<Vec<PeerNode>>>;

/// Maximum number of retries for a bootstrap request before giving up (M8). Documented deviation:
/// Scala's `keepOnRequestingTillRunning` retries forever.
const MAX_BOOTSTRAP_RETRIES: u32 = 10;

/// A standalone (bootstrap) node tried to send to the bootstrap node (port of
/// `StandaloneNodeSendToBootstrapError`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StandaloneNodeSendToBootstrapError;

impl std::fmt::Display for StandaloneNodeSendToBootstrapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("standalone node cannot send to the bootstrap node")
    }
}

impl std::error::Error for StandaloneNodeSendToBootstrapError {}

/// Comm utilities (port of `CommUtil[F]`).
pub struct CommUtil {
    transport: Arc<dyn TransportLayer>,
    conf: RPConf,
    connections: ConnectionsCell,
    log: Arc<dyn Log>,
    log_source: LogSource,
}

impl CommUtil {
    pub fn new(
        transport: Arc<dyn TransportLayer>,
        conf: RPConf,
        connections: ConnectionsCell,
        log: Arc<dyn Log>,
    ) -> Self {
        CommUtil {
            transport,
            conf,
            connections,
            log,
            log_source: LogSource::new("casper.protocol.CommUtil"),
        }
    }

    /// Broadcast a packet (in one piece) to up to `scope_size` random peers (port of `sendToPeers`).
    pub async fn send_to_peers(&self, message: &Packet, scope_size: Option<usize>) {
        let max = scope_size.unwrap_or(self.conf.max_num_of_connections);
        let peers = {
            let connections = self.connections.read().await;
            random_connections(&connections, max)
        };
        let msg = protocol_helper::packet(&self.conf.local, &self.conf.network_id, message.clone());
        self.transport.broadcast(&peers, msg).await;
    }

    /// Broadcast a packet in chunks (stream) to up to `scope_size` random peers (port of
    /// `streamToPeers`).
    pub async fn stream_to_peers(&self, packet: &Packet, scope_size: Option<usize>) {
        let max = scope_size.unwrap_or(self.conf.max_num_of_connections);
        let peers = {
            let connections = self.connections.read().await;
            random_connections(&connections, max)
        };
        let blob = Blob {
            sender: self.conf.local.clone(),
            packet: packet.clone(),
        };
        self.transport.stream(&peers, blob).await;
    }

    /// Send a packet with retry until it succeeds (port of `sendWithRetry`).
    pub async fn send_with_retry(
        &self,
        message: &Packet,
        peer: &PeerNode,
        retry_after: Duration,
        msg_type_name: &str,
    ) {
        let msg = protocol_helper::packet(&self.conf.local, &self.conf.network_id, message.clone());
        self.log.info(
            self.log_source,
            &format!("Starting to request {msg_type_name}"),
        );
        self.keep_on_requesting_till_running(peer, &msg, retry_after, msg_type_name)
            .await;
    }

    /// Retry sending `msg` to `peer` until it succeeds (port of `keepOnRequestingTillRunning`).
    ///
    /// Bounded to [`MAX_BOOTSTRAP_RETRIES`] attempts (M8): a dead/unreachable bootstrap must not
    /// block node startup forever (documented deviation — Scala retries indefinitely).
    async fn keep_on_requesting_till_running(
        &self,
        peer: &PeerNode,
        msg: &Protocol,
        retry_after: Duration,
        msg_type_name: &str,
    ) {
        let mut attempts = 0u32;
        loop {
            match self.transport.send(peer, msg.clone()).await {
                Ok(_) => {
                    self.log.info(
                        self.log_source,
                        &format!("Successfully sent {msg_type_name} to {peer}"),
                    );
                    break;
                }
                Err(error) => {
                    attempts += 1;
                    if attempts >= MAX_BOOTSTRAP_RETRIES {
                        self.log.error(
                            self.log_source,
                            &format!(
                                "Giving up sending {msg_type_name} to {peer} after {attempts} attempts (last error: {error:?})."
                            ),
                        );
                        break;
                    }
                    self.log.warn(
                        self.log_source,
                        &format!(
                            "Failed to send {msg_type_name} to {peer} because of {error:?}. Retrying in {retry_after:?}..."
                        ),
                    );
                    tokio::time::sleep(retry_after).await;
                }
            }
        }
    }

    /// Request a block from a peer (port of `requestForBlock`).
    pub async fn request_for_block(&self, peer: &PeerNode, hash: &BlockHash) {
        self.log.debug(
            self.log_source,
            &format!("Requesting {} from {}.", hash.to_hex(), peer.endpoint.host),
        );
        let packet = BlockRequestSerde.mk_packet(&BlockRequest {
            hash: hash.as_bytes().to_vec(),
        });
        transport_layer_syntax::send_to_peer(self.transport.as_ref(), &self.conf, peer, packet)
            .await;
    }

    // --- CommUtil syntax extensions (port of `CommUtilOps`) --------------------------------

    /// Broadcast a block hash to peers (port of `sendBlockHash`).
    pub async fn send_block_hash(&self, hash: &BlockHash, block_creator: &[u8]) {
        let msg = BlockHashMessage {
            block_hash: *hash,
            block_creator: block_creator.to_vec(),
        };
        let packet = BlockHashMessageSerde.mk_packet(&msg);
        self.send_to_peers(&packet, None).await;
        self.log.info(
            self.log_source,
            &format!("Sent hash {} to peers", hash.to_hex()),
        );
    }

    /// Broadcast a has-block request to peers (port of `broadcastHasBlockRequest`).
    pub async fn broadcast_has_block_request(&self, hash: &BlockHash) {
        let packet = HasBlockRequestSerde.mk_packet(&HasBlockRequest {
            hash: hash.as_bytes().to_vec(),
        });
        self.send_to_peers(&packet, None).await;
    }

    /// Broadcast a request for a block (port of `broadcastRequestForBlock`).
    pub async fn broadcast_request_for_block(&self, hash: &BlockHash, scope_size: Option<usize>) {
        let packet = BlockRequestSerde.mk_packet(&BlockRequest {
            hash: hash.as_bytes().to_vec(),
        });
        self.send_to_peers(&packet, scope_size).await;
    }

    /// Request the fork-choice tip from peers (port of `sendForkChoiceTipRequest`).
    pub async fn send_fork_choice_tip_request(&self) {
        let packet = ForkChoiceTipRequestSerde.mk_packet(&ForkChoiceTipRequest);
        self.send_to_peers(&packet, None).await;
        self.log
            .info(self.log_source, "Requested fork tip from peers");
    }

    /// Request the finalized fringe from the bootstrap node (port of `requestFinalizedFringe`).
    pub async fn request_finalized_fringe(
        &self,
        trim_state: bool,
    ) -> Result<(), StandaloneNodeSendToBootstrapError> {
        let bootstrap = self
            .conf
            .bootstrap
            .clone()
            .ok_or(StandaloneNodeSendToBootstrapError)?;
        let msg = FinalizedFringeRequest {
            identifier: String::new(),
            trim_state,
        };
        let packet = FinalizedFringeRequestSerde.mk_packet(&msg);
        self.send_with_retry(
            &packet,
            &bootstrap,
            Duration::from_secs(10),
            "FinalizedFringeRequest",
        )
        .await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use rchain_comm::errors::CommErr;
    use rchain_comm::peer_node::NodeIdentifier;
    use rchain_comm::rp::rp_conf::ClearConnectionsConf;
    use rchain_models::casper::protocol::packet_type_tag::FromPacket;
    use rchain_shared::log::NopLog;

    /// Records every call and fails `send` while `fail_sends` is positive. The retry bound is a
    /// deviation from Scala (which retries forever), so it needs a transport that really fails —
    /// a mock that always succeeds cannot reach the loop's give-up arm at all.
    #[derive(Default)]
    struct MockTransport {
        sends: std::sync::Mutex<Vec<(PeerNode, Protocol)>>,
        broadcasts: std::sync::Mutex<Vec<(Vec<PeerNode>, Protocol)>>,
        streams: std::sync::Mutex<Vec<(Vec<PeerNode>, Blob)>>,
        fail_sends: std::sync::atomic::AtomicUsize,
    }

    impl MockTransport {
        fn failing(times: usize) -> Self {
            let t = MockTransport::default();
            t.fail_sends
                .store(times, std::sync::atomic::Ordering::SeqCst);
            t
        }
        fn send_count(&self) -> usize {
            self.sends.lock().unwrap().len()
        }
    }

    #[async_trait]
    impl TransportLayer for MockTransport {
        async fn send(&self, peer: &PeerNode, msg: Protocol) -> CommErr<()> {
            self.sends.lock().unwrap().push((peer.clone(), msg));
            // `usize::MAX` is the "always fails" setting, and the only one that reaches the bound.
            let remaining = self.fail_sends.load(std::sync::atomic::Ordering::SeqCst);
            if remaining > 0 {
                self.fail_sends
                    .store(remaining - 1, std::sync::atomic::Ordering::SeqCst);
                Err(rchain_comm::errors::CommError::PeerUnavailable(
                    peer.clone(),
                ))
            } else {
                Ok(())
            }
        }
        async fn broadcast(&self, peers: &[PeerNode], msg: Protocol) -> Vec<CommErr<()>> {
            self.broadcasts.lock().unwrap().push((peers.to_vec(), msg));
            peers.iter().map(|_| Ok(())).collect()
        }
        async fn stream(&self, peers: &[PeerNode], blob: Blob) {
            self.streams.lock().unwrap().push((peers.to_vec(), blob));
        }
    }

    fn peer(name: &str, port: u16) -> PeerNode {
        PeerNode::from(
            NodeIdentifier::new(name.as_bytes().to_vec()),
            "host".to_string(),
            rchain_shared::refined::Port::new(port),
            rchain_shared::refined::Port::new(port),
        )
    }

    fn conf(local: &PeerNode, bootstrap: Option<PeerNode>, max: usize) -> RPConf {
        RPConf {
            local: local.clone(),
            network_id: "testnet".to_string(),
            bootstrap,
            default_timeout: Duration::from_secs(10),
            max_num_of_connections: max,
            clear_connections: ClearConnectionsConf {
                num_of_connections_pinged: 10,
            },
        }
    }

    /// `CommUtil` over the mock, with `n` connections already established.
    fn comm_util_with(
        transport: Arc<MockTransport>,
        conf: RPConf,
        connections: Vec<PeerNode>,
    ) -> CommUtil {
        CommUtil::new(
            transport,
            conf,
            Arc::new(tokio::sync::RwLock::new(connections)),
            Arc::new(NopLog),
        )
    }

    fn packet(type_id: &str) -> Packet {
        Packet {
            type_id: type_id.to_string(),
            content: Vec::new(),
        }
    }

    /// Decode a protocol back to its packet, so an assertion can name the wire type the caller
    /// actually sent rather than trusting the wrapper.
    fn unpack(proto: &Protocol) -> Packet {
        protocol_helper::to_packet(proto).expect("the broadcast carries a packet")
    }

    fn hash(byte: u8) -> BlockHash {
        BlockHash::new([byte; 32])
    }

    #[tokio::test]
    async fn request_finalized_fringe_is_an_error_when_there_is_no_bootstrap() {
        let local = peer("local", 40400);
        let transport = Arc::new(MockTransport::default());
        let comm = comm_util_with(transport.clone(), conf(&local, None, 10), Vec::new());

        let err = comm
            .request_finalized_fringe(false)
            .await
            .expect_err("a standalone node has no bootstrap to ask");
        assert_eq!(err, StandaloneNodeSendToBootstrapError);
        assert_eq!(
            err.to_string(),
            "standalone node cannot send to the bootstrap node"
        );
        // The guard is a precondition, not a filter: nothing was put on the wire.
        assert_eq!(transport.send_count(), 0);
    }

    #[tokio::test]
    async fn request_finalized_fringe_asks_the_bootstrap_for_the_fringe() {
        let local = peer("local", 40400);
        let bootstrap = peer("bootstrap", 40401);
        let transport = Arc::new(MockTransport::default());
        let comm = comm_util_with(
            transport.clone(),
            conf(&local, Some(bootstrap.clone()), 10),
            Vec::new(),
        );

        comm.request_finalized_fringe(true)
            .await
            .expect("a bootstrap");
        let sends = transport.sends.lock().unwrap();
        assert_eq!(sends.len(), 1, "one successful attempt, then stop");
        assert_eq!(sends[0].0, bootstrap, "the request goes to the bootstrap");
        assert_eq!(unpack(&sends[0].1).type_id, "FinalizedFringeRequest");
        // The retry loop's own behaviour is covered by the zero-delay tests below; driving it from
        // here would sleep the hard-coded ten seconds between attempts.
    }

    /// The give-up arm: a transport that never succeeds must not hang the caller. Scala's
    /// `keepOnRequestingTillRunning` retries forever (AUDIT.md), so this bound is a deviation and
    /// the test is what keeps it from silently disappearing.
    #[tokio::test]
    async fn send_with_retry_gives_up_after_the_bounded_number_of_attempts() {
        let local = peer("local", 40400);
        let remote = peer("remote", 40402);
        let transport = Arc::new(MockTransport::failing(usize::MAX));
        let comm = comm_util_with(transport.clone(), conf(&local, None, 10), Vec::new());

        // A zero retry delay keeps the ten attempts instantaneous; the bound, not the wait, is
        // what is under test.
        comm.send_with_retry(
            &packet("BlockRequest"),
            &remote,
            Duration::ZERO,
            "BlockRequest",
        )
        .await;
        assert_eq!(
            transport.send_count(),
            MAX_BOOTSTRAP_RETRIES as usize,
            "the loop is bounded, and the bound is the named constant"
        );
    }

    #[tokio::test]
    async fn send_with_retry_recovers_after_a_transient_failure() {
        let local = peer("local", 40400);
        let remote = peer("remote", 40402);
        let transport = Arc::new(MockTransport::failing(2));
        let comm = comm_util_with(transport.clone(), conf(&local, None, 10), Vec::new());

        comm.send_with_retry(
            &packet("BlockRequest"),
            &remote,
            Duration::ZERO,
            "BlockRequest",
        )
        .await;
        assert_eq!(
            transport.send_count(),
            3,
            "two retries then the attempt that succeeds"
        );
    }

    #[tokio::test]
    async fn send_with_retry_stops_at_the_first_success() {
        let local = peer("local", 40400);
        let remote = peer("remote", 40402);
        let transport = Arc::new(MockTransport::default());
        let comm = comm_util_with(transport.clone(), conf(&local, None, 10), Vec::new());

        comm.send_with_retry(
            &packet("BlockRequest"),
            &remote,
            Duration::ZERO,
            "BlockRequest",
        )
        .await;
        assert_eq!(transport.send_count(), 1, "one attempt, then stop");
    }

    #[tokio::test]
    async fn send_to_peers_caps_the_scope_and_defaults_to_the_configured_maximum() {
        let local = peer("local", 40400);
        let transport = Arc::new(MockTransport::default());
        let connections: Vec<PeerNode> = (0..12)
            .map(|i| peer(&format!("p{i}"), 40400 + i as u16))
            .collect();
        let comm = comm_util_with(
            transport.clone(),
            conf(&local, None, 10),
            connections.clone(),
        );

        comm.send_to_peers(&packet("ForkChoiceTipRequest"), Some(2))
            .await;
        comm.send_to_peers(&packet("ForkChoiceTipRequest"), None)
            .await;

        let broadcasts = transport.broadcasts.lock().unwrap();
        assert_eq!(broadcasts.len(), 2);
        assert_eq!(broadcasts[0].0.len(), 2, "an explicit scope is honoured");
        assert_eq!(
            broadcasts[1].0.len(),
            10,
            "no scope means the configured maximum, not every connection"
        );
        // The peers are drawn from the connection table (randomly ordered, hence the set compare).
        for sent in broadcasts[0].0.iter() {
            assert!(connections.contains(sent), "{sent:?} is not a connection");
        }
    }

    /// A node with no connections broadcasts to nobody. The call still reaches the transport
    /// (matching Scala, which does not special-case the empty list) — what matters is that the
    /// peer list is empty, so no send is attempted.
    #[tokio::test]
    async fn send_to_peers_over_no_connections_addresses_nobody() {
        let local = peer("local", 40400);
        let transport = Arc::new(MockTransport::default());
        let comm = comm_util_with(transport.clone(), conf(&local, None, 10), Vec::new());

        comm.send_to_peers(&packet("BlockHashMessage"), None).await;
        let broadcasts = transport.broadcasts.lock().unwrap();
        assert_eq!(broadcasts.len(), 1);
        assert!(broadcasts[0].0.is_empty(), "no connections, no recipients");
    }

    #[tokio::test]
    async fn stream_to_peers_streams_a_blob_to_the_scoped_peers() {
        let local = peer("local", 40400);
        let transport = Arc::new(MockTransport::default());
        let connections: Vec<PeerNode> = (0..5)
            .map(|i| peer(&format!("p{i}"), 40400 + i as u16))
            .collect();
        let comm = comm_util_with(
            transport.clone(),
            conf(&local, None, 2),
            connections.clone(),
        );

        comm.stream_to_peers(&packet("BlockResponse"), None).await;
        let streams = transport.streams.lock().unwrap();
        assert_eq!(streams.len(), 1);
        assert_eq!(
            streams[0].0.len(),
            2,
            "the configured maximum applies to a stream too, not just a broadcast"
        );
        for sent in &streams[0].0 {
            assert!(connections.contains(sent), "{sent:?} is not a connection");
        }
        // The blob carries the sender and the packet: a stream that lost either would be
        // undecodable at the receiver.
        assert_eq!(streams[0].1.sender, local);
        assert_eq!(streams[0].1.packet.type_id, "BlockResponse");
    }

    /// The syntax wrappers each name themselves on the wire; a copy-paste that reuses the wrong
    /// serde would still "broadcast something", so the type id is asserted, not the count alone.
    #[tokio::test]
    async fn the_syntax_wrappers_broadcast_their_own_packet_type() {
        let local = peer("local", 40400);
        let transport = Arc::new(MockTransport::default());
        let comm = comm_util_with(
            transport.clone(),
            conf(&local, None, 10),
            vec![peer("p", 40401)],
        );

        comm.send_block_hash(&hash(1), &[9u8; 65]).await;
        comm.broadcast_has_block_request(&hash(2)).await;
        comm.broadcast_request_for_block(&hash(3), Some(1)).await;
        comm.send_fork_choice_tip_request().await;

        let broadcasts = transport.broadcasts.lock().unwrap();
        let ids: Vec<String> = broadcasts
            .iter()
            .map(|(peers, proto)| {
                assert_eq!(peers.len(), 1, "one connection, one recipient");
                unpack(proto).type_id
            })
            .collect();
        assert_eq!(
            ids,
            [
                "BlockHashMessage",
                "HasBlockRequest",
                "BlockRequest",
                "ForkChoiceTipRequest"
            ],
            "each wrapper names itself on the wire"
        );
    }

    #[tokio::test]
    async fn request_for_block_targets_one_peer_and_names_the_block() {
        let local = peer("local", 40400);
        let remote = peer("remote", 40402);
        let transport = Arc::new(MockTransport::default());
        let comm = comm_util_with(transport.clone(), conf(&local, None, 10), Vec::new());

        comm.request_for_block(&remote, &hash(7)).await;
        let sends = transport.sends.lock().unwrap();
        assert_eq!(sends.len(), 1, "a block request is unicast, not broadcast");
        assert_eq!(sends[0].0, remote);
        assert_eq!(unpack(&sends[0].1).type_id, "BlockRequest");
    }

    /// `protocol_helper::packet` is what stamps the sender and network id on every outgoing
    /// message, so a message that skipped it would be rejected by a peer's routing layer.
    #[tokio::test]
    async fn an_outgoing_block_hash_carries_the_block_hash_and_its_creator() {
        let local = peer("local", 40400);
        let transport = Arc::new(MockTransport::default());
        let comm = comm_util_with(
            transport.clone(),
            conf(&local, None, 10),
            vec![peer("p", 40401)],
        );

        comm.send_block_hash(&hash(3), &[9u8; 65]).await;
        let broadcasts = transport.broadcasts.lock().unwrap();
        let decoded = BlockHashMessageSerde
            .parse_from(&unpack(&broadcasts[0].1))
            .expect("the advertised hash round-trips");
        assert_eq!(decoded.block_hash, hash(3));
        assert_eq!(decoded.block_creator, vec![9u8; 65]);
        // The header is what the receiver authenticates: it must name this node and network.
        let header = broadcasts[0]
            .1
            .header
            .clone()
            .expect("a packet always carries a header");
        assert_eq!(
            header.sender.clone().expect("sender").id,
            local.id.key().to_vec()
        );
        assert_eq!(header.network_id, "testnet");
    }
}
