//! gRPC Kademlia RPC server (mutual TLS; AUDIT C116).
//!
//! Mirrors `comm/src/main/scala/coop/rchain/comm/discovery/GrpcKademliaRPCServer.scala`.

use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use rchain_models::comm::discovery::kademlia_rpc_service_server::{
    KademliaRpcService, KademliaRpcServiceServer,
};
use rchain_models::comm::discovery::{Lookup, LookupResponse, Ping, Pong};
use rchain_shared::rate_limiter::KeyedRateLimiter;
use tonic::{Request, Response, Status};

use crate::discovery::{to_node, to_peer_node};
use crate::peer_node::PeerNode;
use crate::rp::handle_messages::is_local_address_resolved;
use crate::transport::grpc_transport_receiver::PeerId;

type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// Refuse when the claimed sender is not the identity the client certificate proved (AUDIT C116).
///
/// The mirror of C115's check one service over. Discovery messages name their sender and the routing
/// table is keyed on it, so an unbound sender lets a peer enter the table under an id it does not
/// hold — the difference from the transport path is only *what* gets poisoned. `None` (no proof) is
/// refused for the same reason it is there: client auth is mandatory, so a missing proof means the
/// invariant broke rather than that the sender is merely unverified.
fn sender_not_proven(proven: &Option<String>, claimed: &str) -> Option<Status> {
    match proven {
        Some(id) if id == claimed => None,
        Some(id) => Some(Status::permission_denied(format!(
            "Sender '{claimed}' does not match the identity its certificate proves ('{id}')"
        ))),
        None => Some(Status::permission_denied(
            "No client certificate on this connection, so the sender cannot be verified",
        )),
    }
}

/// Rate limit (requests/second) on the Kademlia RPC (documented Scala deviation: Scala has no
/// limit). Bounds request amplification from the `0.0.0.0:40404` surface. It is no longer the only
/// thing in front of that surface: since AUDIT C116 the service is behind the same mutual TLS as the
/// transport, so a caller is an authenticated peer rather than any host that can reach the port.
const DEFAULT_KADEMLIA_RATE_LIMIT_PER_SEC: u64 = 100;

/// Distinct peer identities the Kademlia admission limiter will track at once — the map is itself a
/// resource a caller would otherwise size, so it is capped (`KeyedRateLimiter`).
const MAX_TRACKED_KADEMLIA_PEERS: usize = 4_096;

/// The Kademlia RPC service (port of `GrpcKademliaRPCServer`).
pub struct GrpcKademliaRpcServer {
    network_id: String,
    ping_handler: Arc<dyn Fn(PeerNode) -> BoxFuture<()> + Send + Sync>,
    lookup_handler: Arc<dyn Fn(PeerNode, Vec<u8>) -> BoxFuture<Vec<PeerNode>> + Send + Sync>,
    rate_limiter: KeyedRateLimiter,
}

impl GrpcKademliaRpcServer {
    pub fn new<F, G>(network_id: String, ping_handler: F, lookup_handler: G) -> Self
    where
        F: Fn(PeerNode) -> BoxFuture<()> + Send + Sync + 'static,
        G: Fn(PeerNode, Vec<u8>) -> BoxFuture<Vec<PeerNode>> + Send + Sync + 'static,
    {
        GrpcKademliaRpcServer {
            network_id,
            ping_handler: Arc::new(ping_handler),
            lookup_handler: Arc::new(lookup_handler),
            rate_limiter: KeyedRateLimiter::new(
                DEFAULT_KADEMLIA_RATE_LIMIT_PER_SEC,
                MAX_TRACKED_KADEMLIA_PEERS,
            ),
        }
    }
}

#[async_trait]
impl KademliaRpcService for GrpcKademliaRpcServer {
    async fn send_ping(&self, request: Request<Ping>) -> Result<Response<Pong>, Status> {
        // The identity the client certificate proves, read before the request is consumed
        // (AUDIT C116). Without it the `sender` below is whatever the peer wrote — the same spoof
        // C115 closed on the transport, one service over.
        let proven = request
            .extensions()
            .get::<PeerId>()
            .cloned()
            .unwrap_or(PeerId(None));
        let ping = request.into_inner();
        if ping.network_id == self.network_id {
            if let Some(sender) = ping.sender.as_ref() {
                if let Ok(peer) = to_peer_node(sender) {
                    // **Identity, then the token, then the resolution** (W-N1-08). The order was the
                    // reverse: the token was spent before the sender was examined, so a caller drained
                    // the allowance with requests the node refused anyway, and
                    // `is_local_address_resolved` — a DNS lookup of a name the caller chose — ran
                    // *behind* the identity check that would have refused that caller outright. Now an
                    // unproven sender costs nothing, the token is charged only to a request the node
                    // will act on, and it is charged per proven identity instead of to one bucket the
                    // whole network shares (W-N1-05).
                    if let Some(error) = sender_not_proven(&proven.0, &peer.id.to_string()) {
                        return Err(error);
                    }
                    if !self
                        .rate_limiter
                        .allow(proven.0.as_deref().unwrap_or_default().as_bytes())
                    {
                        return Err(Status::resource_exhausted("kademlia rate limit exceeded"));
                    }
                    // Reject attacker-chosen private/loopback/link-local/unspecified hosts before
                    // they reach the routing table (SSRF guard; see FIX 5). Resolved first: a
                    // hostname is a way of aiming the node at its own network too (AUDIT C117).
                    if !is_local_address_resolved(&peer.endpoint.host).await {
                        (self.ping_handler)(peer).await;
                    }
                }
            }
        }
        Ok(Response::new(Pong {
            network_id: self.network_id.clone(),
        }))
    }

    async fn send_lookup(
        &self,
        request: Request<Lookup>,
    ) -> Result<Response<LookupResponse>, Status> {
        let proven = request
            .extensions()
            .get::<PeerId>()
            .cloned()
            .unwrap_or(PeerId(None));
        let lookup = request.into_inner();
        let nodes = if lookup.network_id == self.network_id {
            match lookup.sender.as_ref().and_then(|s| to_peer_node(s).ok()) {
                Some(sender) => {
                    // Identity, then the token, then the resolution — see `send_ping` (W-N1-08).
                    if let Some(error) = sender_not_proven(&proven.0, &sender.id.to_string()) {
                        return Err(error);
                    }
                    if !self
                        .rate_limiter
                        .allow(proven.0.as_deref().unwrap_or_default().as_bytes())
                    {
                        return Err(Status::resource_exhausted("kademlia rate limit exceeded"));
                    }
                    // Reject attacker-chosen private/loopback/link-local/unspecified hosts before
                    // they reach the routing table (SSRF guard; see FIX 5); resolved, for the same
                    // reason (AUDIT C117).
                    if is_local_address_resolved(&sender.endpoint.host).await {
                        Vec::new()
                    } else {
                        let peers = (self.lookup_handler)(sender, lookup.id).await;
                        peers.iter().map(to_node).collect()
                    }
                }
                _ => Vec::new(),
            }
        } else {
            Vec::new()
        };
        Ok(Response::new(LookupResponse {
            nodes,
            network_id: self.network_id.clone(),
        }))
    }
}

/// Serve the Kademlia RPC over **mutual TLS**, the same configuration the transport uses.
///
/// **The residual this closes** (AUDIT C116): the discovery service was plaintext and
/// unauthenticated on `0.0.0.0:40404`, with only a rate limit and the `is_local_address` SSRF guard in
/// front of it. Those bound *how much* a peer can do; neither established *who* it was, so a routing
/// table could be populated by a host asserting an id it did not hold. The handshake is the same
/// `hostname_trust_manager::server_config` the transport serves with — a client certificate is
/// mandatory, and the identity it proves is carried into the request as [`PeerId`] and checked
/// against the claimed sender, exactly as on the transport path.
///
/// Served through [`accept_tls`] rather than `Server::tls_config` because the node's trust model is
/// "any self-signed P-256 certificate, with the address checked separately", which tonic's
/// `ServerTlsConfig` (identity + CA root) cannot express. One accept path for both services means they
/// cannot drift in who they let in.
pub async fn serve(
    addr: SocketAddr,
    service: GrpcKademliaRpcServer,
    max_message_size: usize,
    tls: Arc<rustls::ServerConfig>,
) -> Result<(), String> {
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| e.to_string())?;
    let acceptor = tokio_rustls::TlsAcceptor::from(tls);
    let incoming = crate::transport::grpc_transport_receiver::accept_tls(
        listener,
        acceptor,
        crate::transport::grpc_transport_receiver::MAX_CONCURRENT_HANDSHAKES,
    );
    tonic::transport::Server::builder()
        // **The operator's inbound decode cap, installed here as it is on the transport** (AUDIT
        // C113's class, one service over). Without this line the server falls back to tonic's default
        // — 4 MiB, sixteen times the configured `protocol-server.grpc-max-recv-message-size` — so a
        // certificate-bearing peer could force a decode four megabytes wide while the operator
        // believed 256 KiB was in force. Both services are served by the same accept path, and now by
        // the same bound, so they cannot drift in what they will admit.
        .add_service(
            KademliaRpcServiceServer::new(service).max_decoding_message_size(max_message_size),
        )
        .serve_with_incoming(incoming)
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use rchain_models::comm::discovery::Node;

    /// Records what reached the handler, so the guards can be asserted by *absence* as well as by
    /// the returned value: a handler that is never called is the point of both guards here.
    #[derive(Default)]
    struct Handlers {
        pinged: Mutex<Vec<PeerNode>>,
        looked_up: Mutex<Vec<(PeerNode, Vec<u8>)>>,
        /// What the lookup handler returns.
        peers: Vec<PeerNode>,
    }

    fn server(h: Arc<Handlers>) -> GrpcKademliaRpcServer {
        let ping = h.clone();
        let lookup = h.clone();
        GrpcKademliaRpcServer::new(
            "testnet".to_string(),
            move |peer| {
                ping.pinged.lock().unwrap().push(peer);
                Box::pin(async {})
            },
            move |peer, key| {
                let peers = lookup.peers.clone();
                lookup.looked_up.lock().unwrap().push((peer, key));
                Box::pin(async move { peers })
            },
        )
    }

    fn node(id: &[u8], host: &str, port: u32) -> Node {
        Node {
            id: id.to_vec(),
            host: host.as_bytes().to_vec(),
            tcp_port: port,
            udp_port: port,
        }
    }

    /// **The end-to-end test C116 was missing.** The handler-seam tests above pin the guard's
    /// *decision*; only this pins that a real TLS handshake admits an honest peer and that the
    /// identity read at the accept is the one the client certificate actually carries — the part
    /// that cannot be observed without a socket. It is the Kademlia mirror of
    /// `transport::grpc_transport::tests:send_round_trips_over_socket`.
    ///
    /// **The client announces a public host on purpose.** Its `Ping` names `local`, and the SSRF
    /// guard rejects a loopback host — correctly, as C117 tightened. So the client's *announced*
    /// address is a documentation-range literal while the *dialled* address is loopback, which is the
    /// only arrangement in which the round trip exercises the identity check rather than the SSRF one.
    #[tokio::test]
    async fn a_ping_round_trips_over_mutual_tls() {
        use crate::discovery::grpc_kademlia_rpc::GrpcKademliaRpc;
        use crate::discovery::KademliaRpc;
        use crate::transport::generate_certificate_if_absent::generate_certificate;
        use crate::transport::hostname_trust_manager::{public_address_of_cert, server_config};
        use rchain_shared::refined::Port;
        use rustls::pki_types::CertificateDer;
        use std::time::Duration;

        fn node_id_of(cert_pem: &str) -> String {
            let (_, pem) = x509_parser::pem::parse_x509_pem(cert_pem.as_bytes()).unwrap();
            rchain_shared::base16::encode(
                &public_address_of_cert(&CertificateDer::from(pem.contents)).unwrap(),
            )
        }

        let (server_cert, server_key) = generate_certificate().expect("a server certificate");
        let (client_cert, client_key) = generate_certificate().expect("a client certificate");
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();

        let peer_at = |cert: &str, host: &str| {
            PeerNode::from(
                crate::peer_node::NodeIdentifier::new(rchain_shared::base16::unsafe_decode(
                    &node_id_of(cert),
                )),
                host.to_string(),
                Port::new(port),
                Port::new(port),
            )
        };
        // Dialled over loopback; announced as public. See the test's doc comment.
        let server_peer = peer_at(&server_cert, "127.0.0.1");
        let client_peer = peer_at(&client_cert, "203.0.113.7");

        let h = Arc::new(Handlers::default());
        let addr: std::net::SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
        let tls = server_config(&server_cert, &server_key).expect("a server TLS config");
        let srv = server(h.clone());
        tokio::spawn(async move {
            let _ = serve(addr, srv, 262_144, tls).await;
        });
        tokio::time::sleep(Duration::from_millis(200)).await;

        let rpc = GrpcKademliaRpc::new(
            client_peer,
            "testnet".to_string(),
            Duration::from_secs(5),
            &client_cert,
            &client_key,
        )
        .expect("a client TLS config");

        assert!(
            rpc.ping(&server_peer).await,
            "an honest peer must complete the mutual-TLS handshake and be answered"
        );
        assert_eq!(
            h.pinged.lock().unwrap().len(),
            1,
            "and its ping must reach the handler — a handshake that succeeds but a guard that \
             refuses everything would answer false, and this asserts both halves"
        );
    }

    fn peer(byte: u8) -> PeerNode {
        PeerNode::from(
            crate::peer_node::NodeIdentifier::new(vec![byte; 4]),
            "peer.example".to_string(),
            rchain_shared::refined::Port::new(40400),
            rchain_shared::refined::Port::new(40404),
        )
    }

    /// A ping carrying the identity its certificate would have proved (AUDIT C116).
    ///
    /// The extension is not decoration: without it the sender check refuses every request, which is
    /// how these tests found it. `proven` defaults to what the header claims, so an existing test
    /// keeps testing what it was written to test rather than accidentally testing the new guard.
    fn ping_from(network: &str, sender: Option<Node>, proven: Option<String>) -> Request<Ping> {
        let mut r = Request::new(Ping {
            network_id: network.to_string(),
            sender,
        });
        r.extensions_mut().insert(PeerId(proven));
        r
    }

    /// A lookup carrying the identity its certificate would have proved; see [`ping_from`].
    fn lookup(network: &str, sender: Option<Node>) -> Request<Lookup> {
        let proven = sender
            .as_ref()
            .map(|n| rchain_shared::base16::encode(&n.id));
        lookup_from(network, sender, proven)
    }

    /// A lookup with an explicitly chosen proof, so the spoof arm can state what the certificate
    /// proved rather than what the header claims.
    fn lookup_from(network: &str, sender: Option<Node>, proven: Option<String>) -> Request<Lookup> {
        let mut r = Request::new(Lookup {
            network_id: network.to_string(),
            sender,
            id: b"key".to_vec(),
        });
        r.extensions_mut().insert(PeerId(proven));
        r
    }

    fn ping(network: &str, sender: Option<Node>) -> Request<Ping> {
        let proven = sender
            .as_ref()
            .map(|n| rchain_shared::base16::encode(&n.id));
        ping_from(network, sender, proven)
    }

    /// **The regression test for AUDIT C116.** `is_local_address` bounds *where* a peer may point the
    /// routing table; it says nothing about *who* the peer is, and the entry's key is the sender id
    /// from the message. Without this check a host could enter the table under an id it does not hold,
    /// and be dialled, and be gossiped onward, as that node.
    ///
    /// All three arms, because the first is what makes the others mean anything: an honest sender
    /// reaches the handler, a sender whose certificate proves a different node is refused **and does
    /// not reach it**, and a connection that proved nothing is refused as well.
    #[tokio::test]
    async fn a_sender_must_be_the_identity_its_certificate_proves() {
        let h = Arc::new(Handlers::default());
        let s = server(h.clone());
        let sender = node(b"abcd", "203.0.113.7", 40400);

        // The certificate proves what the header claims: the handler runs.
        s.send_ping(ping("testnet", Some(sender.clone())))
            .await
            .expect("a sender matching its certificate is answered");
        assert_eq!(
            h.pinged.lock().unwrap().len(),
            1,
            "the honest ping must reach the handler"
        );

        // The spoof: the certificate proves some other node.
        let other = rchain_shared::base16::encode(b"someone-else");
        let status = s
            .send_ping(ping_from("testnet", Some(sender.clone()), Some(other)))
            .await
            .expect_err("a sender its certificate does not prove must be refused");
        assert_eq!(status.code(), tonic::Code::PermissionDenied);
        assert_eq!(
            h.pinged.lock().unwrap().len(),
            1,
            "a spoofed sender must never enter the routing table — this is the whole finding"
        );

        // No proof at all: client auth is mandatory, so `None` means the invariant broke.
        let status = s
            .send_ping(ping_from("testnet", Some(sender), None))
            .await
            .expect_err("a connection with no proven identity must be refused");
        assert_eq!(status.code(), tonic::Code::PermissionDenied);
        assert_eq!(h.pinged.lock().unwrap().len(), 1);

        // The lookup path carries the same check, and answers the same way.
        let status = s
            .send_lookup(lookup_from(
                "testnet",
                Some(node(b"id", "203.0.113.7", 40400)),
                Some(rchain_shared::base16::encode(b"someone-else")),
            ))
            .await
            .expect_err("a lookup sender its certificate does not prove must be refused");
        assert_eq!(status.code(), tonic::Code::PermissionDenied);
        assert!(h.looked_up.lock().unwrap().is_empty());
    }

    /// A ping on another network is ignored — the handler is not called — but still answered with
    /// *our* network id, so the peer learns which network it reached.
    #[tokio::test]
    async fn a_ping_on_another_network_never_reaches_the_handler() {
        let h = Arc::new(Handlers::default());
        let response = server(h.clone())
            .send_ping(ping("mainnet", Some(node(b"id", "203.0.113.7", 40400))))
            .await
            .expect("a mismatched network is answered, not refused");
        assert_eq!(response.into_inner().network_id, "testnet");
        assert!(
            h.pinged.lock().unwrap().is_empty(),
            "the handler must not run"
        );
    }

    /// **The SSRF guard.** An attacker-chosen private/loopback/link-local/unspecified host must
    /// never reach the routing table, because the table is what the node later *dials*: accepting
    /// one would let a peer point the node at its own internal network.
    ///
    /// **The last three entries are AUDIT C117 and they are the point of this test now.** The guard
    /// classified IP literals and nothing else, so `localhost` — and any DNS name whose A record
    /// points inward — passed it and was dialled. A name is a way of aiming the node at its own
    /// network exactly as a literal is. The unresolvable one is the other half of the rule: a name
    /// that resolves to nothing is not a place to dial, and "could not find out" must not read as
    /// "probably fine".
    #[tokio::test]
    async fn a_ping_claiming_a_local_host_never_reaches_the_handler() {
        for host in [
            "127.0.0.1",
            "0.0.0.0",
            "10.1.2.3",
            "172.16.0.9",
            "192.168.1.1",
            "169.254.169.254",
            "224.0.0.1",
            "::1",
            "fe80::1",
            // C117: names, not literals.
            "localhost",
            "localhost.localdomain",
            "this-host-does-not-exist.invalid",
        ] {
            let h = Arc::new(Handlers::default());
            server(h.clone())
                .send_ping(ping("testnet", Some(node(b"id", host, 40400))))
                .await
                .expect("answered");
            assert!(
                h.pinged.lock().unwrap().is_empty(),
                "{host} must be rejected before the handler"
            );
        }
    }

    /// The positive side of the same guard: a public host reaches the handler with the peer parsed
    /// from the message, and the ping is answered.
    #[tokio::test]
    async fn a_ping_from_a_public_host_reaches_the_handler() {
        let h = Arc::new(Handlers::default());
        let response = server(h.clone())
            .send_ping(ping("testnet", Some(node(b"abcd", "203.0.113.7", 40400))))
            .await
            .expect("answered");
        assert_eq!(response.into_inner().network_id, "testnet");

        let pinged = h.pinged.lock().unwrap();
        assert_eq!(pinged.len(), 1);
        assert_eq!(pinged[0].endpoint.host, "203.0.113.7");
        assert_eq!(pinged[0].id.key(), b"abcd".as_slice());
    }

    /// A sender that cannot be parsed (an out-of-range port) is ignored rather than failing the
    /// RPC: one malformed field must not turn into an error status the peer can distinguish.
    #[tokio::test]
    async fn a_malformed_sender_is_ignored_and_still_answered() {
        let h = Arc::new(Handlers::default());
        for sender in [None, Some(node(b"id", "203.0.113.7", 70_000))] {
            let response = server(h.clone())
                .send_ping(ping("testnet", sender))
                .await
                .expect("answered");
            assert_eq!(response.into_inner().network_id, "testnet");
        }
        assert!(h.pinged.lock().unwrap().is_empty());
    }

    /// The lookup side of the SSRF guard, and the happy path with the returned peers converted back
    /// to proto nodes.
    #[tokio::test]
    async fn a_lookup_guards_its_sender_and_maps_the_result() {
        let h = Arc::new(Handlers {
            peers: vec![peer(1), peer(2)],
            ..Handlers::default()
        });

        // A private sender: empty answer, handler untouched.
        let leaked = server(h.clone())
            .send_lookup(lookup("testnet", Some(node(b"id", "127.0.0.1", 40400))))
            .await
            .expect("answered");
        assert!(leaked.into_inner().nodes.is_empty());
        assert!(h.looked_up.lock().unwrap().is_empty());

        // A public sender: the handler runs and its peers come back as proto nodes.
        let answered = server(h.clone())
            .send_lookup(lookup("testnet", Some(node(b"id", "203.0.113.7", 40400))))
            .await
            .expect("answered");
        let nodes = answered.into_inner().nodes;
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].id, peer(1).key().to_vec());
        assert_eq!(h.looked_up.lock().unwrap().len(), 1);
    }

    /// The rate limit is a real bound on this unauthenticated surface: after
    /// `DEFAULT_KADEMLIA_RATE_LIMIT_PER_SEC` requests in the window, the next is refused with
    /// `ResourceExhausted` rather than served.
    #[tokio::test]
    async fn the_kademlia_rate_limit_refuses_past_its_bound() {
        let s = server(Arc::new(Handlers::default()));
        // A request that actually does work, so it is one the bound applies to: the ping carries a
        // sender whose identity its certificate proves, and a reachable public address. (This used to
        // be a sender-less ping, which the guard refuses to act on — a request that does no work and,
        // since W-N1-08, spends no token either.)
        let sender = node(b"abcd", "203.0.113.7", 40400);
        for i in 0..DEFAULT_KADEMLIA_RATE_LIMIT_PER_SEC {
            s.send_ping(ping("testnet", Some(sender.clone())))
                .await
                .unwrap_or_else(|e| panic!("request {i} must be admitted: {e}"));
        }
        let refused = s
            .send_ping(ping("testnet", Some(sender.clone())))
            .await
            .expect_err("the bound must refuse");
        assert_eq!(refused.code(), tonic::Code::ResourceExhausted);
    }

    /// **The allowance is spent on admitted work, not on work the node discards** (W-N1-08).
    ///
    /// The token was charged before the sender was examined, so a caller drained the node's discovery
    /// allowance with requests the node refused anyway — and, because the bucket was one for the whole
    /// node (W-N1-05), it drained everyone's. Here an honest peer's full allowance must survive a flood
    /// three times its size, every request of which is refused for a sender the flood cannot prove.
    ///
    /// Remove the reordering — put the `allow` back above the `sender_not_proven` check — and the
    /// honest peer's first request is refused.
    #[tokio::test]
    async fn refused_kademlia_requests_do_not_spend_the_allowance() {
        let s = server(Arc::new(Handlers::default()));
        let honest = node(b"honest", "203.0.113.7", 40400);
        let spoofed = node(b"spoofed", "203.0.113.7", 40400);
        // A certificate that proves some other node: every ping below is refused, and none of them
        // may cost the honest peer anything.
        let other = "ab".repeat(32);

        for i in 0..(DEFAULT_KADEMLIA_RATE_LIMIT_PER_SEC * 3) {
            let refused = s
                .send_ping(ping_from(
                    "testnet",
                    Some(spoofed.clone()),
                    Some(other.clone()),
                ))
                .await;
            assert!(refused.is_err(), "spoofed request {i} must be refused");
        }

        for i in 0..DEFAULT_KADEMLIA_RATE_LIMIT_PER_SEC {
            s.send_ping(ping("testnet", Some(honest.clone())))
                .await
                .unwrap_or_else(|e| {
                    panic!("the honest peer's allowance must be intact, but request {i}: {e}")
                });
        }
    }
}
