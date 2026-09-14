//! gRPC Kademlia RPC server (plaintext).
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
use rchain_shared::rate_limiter::RateLimiter;
use tonic::{Request, Response, Status};

use crate::discovery::{to_node, to_peer_node};
use crate::peer_node::PeerNode;
use crate::rp::handle_messages::is_local_address;

type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// Rate limit (requests/second) on the plaintext Kademlia RPC (documented Scala deviation: Scala
/// has no limit). Bounds sybil/routing-table pollution and peer-enumeration amplification from the
/// unauthenticated `0.0.0.0:40404` surface.
const DEFAULT_KADEMLIA_RATE_LIMIT_PER_SEC: u64 = 100;

/// The Kademlia RPC service (port of `GrpcKademliaRPCServer`).
pub struct GrpcKademliaRpcServer {
    network_id: String,
    ping_handler: Arc<dyn Fn(PeerNode) -> BoxFuture<()> + Send + Sync>,
    lookup_handler: Arc<dyn Fn(PeerNode, Vec<u8>) -> BoxFuture<Vec<PeerNode>> + Send + Sync>,
    rate_limiter: Arc<RateLimiter>,
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
            rate_limiter: Arc::new(RateLimiter::new(DEFAULT_KADEMLIA_RATE_LIMIT_PER_SEC)),
        }
    }
}

#[async_trait]
impl KademliaRpcService for GrpcKademliaRpcServer {
    async fn send_ping(&self, request: Request<Ping>) -> Result<Response<Pong>, Status> {
        if !self.rate_limiter.allow() {
            return Err(Status::resource_exhausted("kademlia rate limit exceeded"));
        }
        let ping = request.into_inner();
        if ping.network_id == self.network_id {
            if let Some(sender) = ping.sender.as_ref() {
                if let Ok(peer) = to_peer_node(sender) {
                    // Reject attacker-chosen private/loopback/link-local/unspecified hosts before
                    // they reach the routing table (SSRF guard; see FIX 5).
                    if !is_local_address(&peer.endpoint.host) {
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
        if !self.rate_limiter.allow() {
            return Err(Status::resource_exhausted("kademlia rate limit exceeded"));
        }
        let lookup = request.into_inner();
        let nodes = if lookup.network_id == self.network_id {
            match lookup.sender.as_ref().and_then(|s| to_peer_node(s).ok()) {
                // Reject attacker-chosen private/loopback/link-local/unspecified hosts before they
                // reach the routing table (SSRF guard; see FIX 5).
                Some(sender) if !is_local_address(&sender.endpoint.host) => {
                    let peers = (self.lookup_handler)(sender, lookup.id).await;
                    peers.iter().map(to_node).collect()
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

/// Serve the Kademlia RPC on the given port (plaintext).
///
/// Residual (documented, not fixed): the discovery service remains plaintext on `0.0.0.0`. The
/// SSRF guard above rejects private/loopback/link-local/unspecified inbound peer hosts, but the
/// transport itself is still unauthenticated and unencrypted.
pub async fn serve(addr: SocketAddr, service: GrpcKademliaRpcServer) -> Result<(), String> {
    tonic::transport::Server::builder()
        .add_service(KademliaRpcServiceServer::new(service))
        .serve(addr)
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

    fn peer(byte: u8) -> PeerNode {
        PeerNode::from(
            crate::peer_node::NodeIdentifier::new(vec![byte; 4]),
            "peer.example".to_string(),
            rchain_shared::refined::Port::new(40400),
            rchain_shared::refined::Port::new(40404),
        )
    }

    fn ping(network: &str, sender: Option<Node>) -> Request<Ping> {
        Request::new(Ping {
            network_id: network.to_string(),
            sender,
        })
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
            .send_lookup(Request::new(Lookup {
                network_id: "testnet".to_string(),
                sender: Some(node(b"id", "127.0.0.1", 40400)),
                id: b"key".to_vec(),
            }))
            .await
            .expect("answered");
        assert!(leaked.into_inner().nodes.is_empty());
        assert!(h.looked_up.lock().unwrap().is_empty());

        // A public sender: the handler runs and its peers come back as proto nodes.
        let answered = server(h.clone())
            .send_lookup(Request::new(Lookup {
                network_id: "testnet".to_string(),
                sender: Some(node(b"id", "203.0.113.7", 40400)),
                id: b"key".to_vec(),
            }))
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
        for i in 0..DEFAULT_KADEMLIA_RATE_LIMIT_PER_SEC {
            s.send_ping(ping("testnet", None))
                .await
                .unwrap_or_else(|e| panic!("request {i} must be admitted: {e}"));
        }
        let refused = s
            .send_ping(ping("testnet", None))
            .await
            .expect_err("the bound must refuse");
        assert_eq!(refused.code(), tonic::Code::ResourceExhausted);
    }
}
