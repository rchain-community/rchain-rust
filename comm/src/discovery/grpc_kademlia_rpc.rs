//! gRPC Kademlia RPC client (mutual TLS; AUDIT C116).
//!
//! Mirrors `comm/src/main/scala/coop/rchain/comm/discovery/GrpcKademliaRPC.scala`.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use http::Uri;
use hyper_util::rt::TokioIo;
use rchain_models::comm::discovery::kademlia_rpc_service_client::KademliaRpcServiceClient;
use rchain_models::comm::discovery::{Lookup, Ping};
use rustls::pki_types::ServerName;
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tonic::transport::{Channel, Endpoint};
use tower::service_fn;

use crate::discovery::{to_node, to_peer_node, KademliaRpc};
use crate::peer_node::PeerNode;
use crate::rp::handle_messages::is_local_address_resolved;

/// The gRPC Kademlia client (port of `GrpcKademliaRPC`).
pub struct GrpcKademliaRpc {
    local: PeerNode,
    network_id: String,
    timeout: Duration,
    /// The mutual-TLS configuration, built from the node's own certificate (AUDIT C116). The same one
    /// the transport client dials with, so discovery and transport agree on what a peer is.
    tls: Arc<rustls::ClientConfig>,
}

impl GrpcKademliaRpc {
    pub fn new(
        local: PeerNode,
        network_id: String,
        timeout: Duration,
        cert_pem: &str,
        key_pem: &str,
    ) -> Result<Self, String> {
        Ok(GrpcKademliaRpc {
            local,
            network_id,
            timeout,
            tls: crate::transport::hostname_trust_manager::client_config(cert_pem, key_pem)?,
        })
    }

    /// A mutual-TLS channel to `peer`, verifying the server certificate against the peer's node id.
    ///
    /// This was `http://` with a bare `connect()` — plaintext, with no check that the host answering
    /// is the peer it was dialled as. The connector mirrors [`crate::transport::grpc_transport_client`]
    /// exactly, including the `ServerName` derived from the peer id, which is what the server-side
    /// verifier compares the certificate against.
    async fn client(&self, peer: &PeerNode) -> Result<KademliaRpcServiceClient<Channel>, String> {
        let endpoint = Endpoint::from_shared(format!(
            "https://{}:{}",
            peer.endpoint.host,
            u16::from(peer.endpoint.udp_port)
        ))
        .map_err(|e| e.to_string())?;

        let connector = TlsConnector::from(self.tls.clone());
        let server_name = ServerName::try_from(peer.id.to_string()).map_err(|e| e.to_string())?;
        let host = peer.endpoint.host.clone();
        let port = peer.endpoint.udp_port;

        let channel = endpoint
            .connect_with_connector(service_fn(move |_: Uri| {
                let connector = connector.clone();
                let server_name = server_name.clone();
                let host = host.clone();
                async move {
                    let tcp = TcpStream::connect((host.as_str(), u16::from(port))).await?;
                    let tls = connector.connect(server_name, tcp).await?;
                    Ok::<_, std::io::Error>(TokioIo::new(tls))
                }
            }))
            .await
            .map_err(|e| e.to_string())?;
        Ok(KademliaRpcServiceClient::new(channel))
    }
}

#[async_trait]
impl KademliaRpc for GrpcKademliaRpc {
    async fn ping(&self, peer: &PeerNode) -> bool {
        let ping = Ping {
            sender: Some(to_node(&self.local)),
            network_id: self.network_id.clone(),
        };
        let result = async {
            let mut client = self.client(peer).await?;
            client
                .send_ping(ping)
                .await
                .map(|r| r.into_inner())
                .map_err(|e| e.to_string())
        };
        match tokio::time::timeout(self.timeout, result).await {
            Ok(Ok(pong)) => pong.network_id == self.network_id,
            _ => false,
        }
    }

    async fn lookup(&self, key: &[u8], peer: &PeerNode) -> Vec<PeerNode> {
        let lookup = Lookup {
            id: key.to_vec(),
            sender: Some(to_node(&self.local)),
            network_id: self.network_id.clone(),
        };
        let result = async {
            let mut client = self.client(peer).await?;
            client
                .send_lookup(lookup)
                .await
                .map(|r| r.into_inner())
                .map_err(|e| e.to_string())
        };
        match tokio::time::timeout(self.timeout, result).await {
            Ok(Ok(response)) if response.network_id == self.network_id => {
                peers_from_response(&response.nodes).await
            }
            _ => Vec::new(),
        }
    }
}

/// The peers a `LookupResponse` names, minus the ones the node must not dial.
///
/// **A response is a peer's message, and the node dials what it names** — `discover` records the
/// result into the routing table and `find_and_connect` connects to each entry. Until now the
/// private/loopback/link-local/unspecified guard ran only on the *inbound* sender path
/// (`grpc_kademlia_rpc_server`), so an authenticated peer answering a `Lookup` with `127.0.0.1`, or
/// with a name that resolves there, steered the victim into connecting to its own loopback services:
/// SSRF with the guard on the wrong side of the exchange (W-N1-03). Resolved rather than matched as a
/// literal, for the same reason the inbound guard resolves (AUDIT C117) — a hostname is a way of
/// aiming the node at its own network too.
async fn peers_from_response(nodes: &[rchain_models::comm::discovery::Node]) -> Vec<PeerNode> {
    let mut peers = Vec::new();
    for n in nodes {
        let Ok(peer) = to_peer_node(n) else { continue };
        if is_local_address_resolved(&peer.endpoint.host).await {
            continue;
        }
        peers.push(peer);
    }
    peers
}

#[cfg(test)]
mod tests {
    use super::*;

    use rchain_shared::refined::Port;

    fn local() -> PeerNode {
        PeerNode::from(
            crate::peer_node::NodeIdentifier::new(b"local".to_vec()),
            "127.0.0.1".to_string(),
            Port::new(40400),
            Port::new(40404),
        )
    }

    /// The client dials `udp_port` for the Kademlia service, so a peer is built with both ports set
    /// to the address under test.
    /// The peer's id must be the identity its certificate proves (AUDIT C116), so it is derived from
    /// the certificate rather than made up — a made-up id would be refused by the server's own check
    /// before the test could observe what it is about.
    fn peer_at_from_cert(cert_pem: &str, host: &str, port: u16) -> PeerNode {
        use crate::transport::hostname_trust_manager::public_address_of_cert;
        use rustls::pki_types::CertificateDer;
        let (_, pem) = x509_parser::pem::parse_x509_pem(cert_pem.as_bytes()).unwrap();
        let id = rchain_shared::base16::encode(
            &public_address_of_cert(&CertificateDer::from(pem.contents)).unwrap(),
        );
        PeerNode::from(
            crate::peer_node::NodeIdentifier::new(rchain_shared::base16::unsafe_decode(&id)),
            host.to_string(),
            Port::new(port),
            Port::new(port),
        )
    }

    /// A peer at `host:port` whose id is a real certificate's address.
    fn peer_at(host: &str, port: u16) -> PeerNode {
        let (cert, _key) = crate::transport::generate_certificate_if_absent::generate_certificate()
            .expect("a test certificate");
        peer_at_from_cert(&cert, host, port)
    }

    fn rpc(timeout: Duration) -> GrpcKademliaRpc {
        let (cert, key) = crate::transport::generate_certificate_if_absent::generate_certificate()
            .expect("a test certificate");
        GrpcKademliaRpc::new(local(), "testnet".to_string(), timeout, &cert, &key)
            .expect("the test certificate builds a client config")
    }

    /// A peer whose port has nothing listening is not a peer: `ping` is `false` and `lookup` is
    /// empty rather than an error escaping into discovery. Port 1 on loopback is reserved and
    /// never served, so the connection is refused immediately.
    #[tokio::test]
    async fn an_unreachable_peer_answers_nothing() {
        let rpc = rpc(Duration::from_secs(5));
        let peer = peer_at("127.0.0.1", 1);

        assert!(
            !rpc.ping(&peer).await,
            "a refused connection is not a live peer"
        );
        assert!(
            rpc.lookup(b"key", &peer).await.is_empty(),
            "a failed lookup yields no peers"
        );
    }

    /// A peer that accepts the connection but never speaks gRPC must not hang discovery: the
    /// timeout turns it into `false` / empty. The stub listener holds the socket open, which is what
    /// distinguishes "refused" (the case above) from "stalled".
    #[tokio::test]
    async fn a_peer_that_accepts_but_stalls_is_given_up_on() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind a stub listener");
        let port = listener.local_addr().expect("addr").port();
        // Accept and hold every connection without ever writing a byte.
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((socket, _)) = listener.accept().await {
                held.push(socket);
            }
        });

        let timeout = Duration::from_millis(200);
        let rpc = rpc(timeout);
        let peer = peer_at("127.0.0.1", port);

        let started = tokio::time::Instant::now();
        assert!(!rpc.ping(&peer).await, "a stalled handshake is not a ping");
        assert!(
            started.elapsed() >= timeout,
            "the client must wait its timeout, not fail instantly"
        );
        assert!(rpc.lookup(b"key", &peer).await.is_empty());
    }

    /// An unreachable *name* (not just a closed port) is also just `false`: DNS failure must not
    /// panic or propagate.
    #[tokio::test]
    async fn an_unresolvable_host_answers_nothing() {
        let rpc = rpc(Duration::from_millis(200));
        let peer = peer_at("this-host-does-not-exist.invalid", 40404);
        assert!(!rpc.ping(&peer).await);
        assert!(rpc.lookup(b"key", &peer).await.is_empty());
    }

    /// **A response's hosts are guarded on the way in, not only on the way out** (W-N1-03).
    ///
    /// `discover` records what a `LookupResponse` names into the routing table and `find_and_connect`
    /// dials each entry, so an authenticated peer answering a `Lookup` with `127.0.0.1` — or a name
    /// that resolves there — steered this node into its own loopback services. The
    /// private/loopback/link-local/unspecified guard already ran on the inbound *sender* path; this
    /// asserts it runs on the response path too, and that a public host is still carried.
    #[tokio::test]
    async fn a_lookup_response_naming_a_local_host_is_not_dialled() {
        use rchain_models::comm::discovery::Node;
        let node = |id: &[u8], host: &str| Node {
            id: id.to_vec(),
            host: host.as_bytes().to_vec(),
            tcp_port: 40400,
            udp_port: 40404,
        };
        let nodes = vec![
            node(b"loopback", "127.0.0.1"),
            node(b"localhost", "localhost"),
            node(b"unspecified", "0.0.0.0"),
            node(b"public", "203.0.113.7"),
        ];
        let peers = super::peers_from_response(&nodes).await;
        assert_eq!(
            peers.len(),
            1,
            "only the public host may survive, or the node dials itself: {peers:?}"
        );
        assert_eq!(peers[0].endpoint.host, "203.0.113.7");
    }
}
