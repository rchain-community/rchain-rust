//! gRPC Kademlia RPC client (plaintext).
//!
//! Mirrors `comm/src/main/scala/coop/rchain/comm/discovery/GrpcKademliaRPC.scala`.

use std::time::Duration;

use async_trait::async_trait;
use rchain_models::comm::discovery::kademlia_rpc_service_client::KademliaRpcServiceClient;
use rchain_models::comm::discovery::{Lookup, Ping};
use tonic::transport::{Channel, Endpoint};

use crate::discovery::{to_node, to_peer_node, KademliaRpc};
use crate::peer_node::PeerNode;

/// The gRPC Kademlia client (port of `GrpcKademliaRPC`).
pub struct GrpcKademliaRpc {
    local: PeerNode,
    network_id: String,
    timeout: Duration,
}

impl GrpcKademliaRpc {
    pub fn new(local: PeerNode, network_id: String, timeout: Duration) -> Self {
        GrpcKademliaRpc {
            local,
            network_id,
            timeout,
        }
    }

    async fn client(&self, peer: &PeerNode) -> Result<KademliaRpcServiceClient<Channel>, String> {
        let endpoint = Endpoint::from_shared(format!(
            "http://{}:{}",
            peer.endpoint.host,
            u16::from(peer.endpoint.udp_port)
        ))
        .map_err(|e| e.to_string())?;
        let channel = endpoint.connect().await.map_err(|e| e.to_string())?;
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
            Ok(Ok(response)) if response.network_id == self.network_id => response
                .nodes
                .iter()
                .filter_map(|n| to_peer_node(n).ok())
                .collect(),
            _ => Vec::new(),
        }
    }
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

    /// The client dials `udp_port` for the plaintext Kademlia service, so a peer is built with both
    /// ports set to the address under test.
    fn peer_at(host: &str, port: u16) -> PeerNode {
        PeerNode::from(
            crate::peer_node::NodeIdentifier::new(b"peer".to_vec()),
            host.to_string(),
            Port::new(port),
            Port::new(port),
        )
    }

    fn rpc(timeout: Duration) -> GrpcKademliaRpc {
        GrpcKademliaRpc::new(local(), "testnet".to_string(), timeout)
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
}
