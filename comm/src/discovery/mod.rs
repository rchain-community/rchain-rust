//! Peer discovery (Kademlia).
//!
//! Mirrors `comm/src/main/scala/coop/rchain/comm/discovery/`.

use rchain_models::comm::discovery::Node;
use rchain_shared::refined::Port;

use crate::errors::CommError;
use crate::peer_node::{Endpoint, NodeIdentifier, PeerNode};

pub mod grpc_kademlia_rpc;
pub mod grpc_kademlia_rpc_server;
pub mod kademlia_handle_rpc;
pub mod kademlia_node_discovery;
pub mod kademlia_rpc;
pub mod kademlia_store;
pub mod node_discovery;
pub mod peer_table;

pub use kademlia_rpc::KademliaRpc;
pub use kademlia_store::KademliaStore;
pub use node_discovery::NodeDiscovery;

/// Convert a Kademlia proto `Node` to a `PeerNode` (port of `discovery.toPeerNode`).
pub fn to_peer_node(node: &Node) -> Result<PeerNode, CommError> {
    Ok(PeerNode {
        id: NodeIdentifier::new(node.id.clone()),
        endpoint: Endpoint {
            host: String::from_utf8_lossy(&node.host).to_string(),
            tcp_port: Port::try_from(node.tcp_port)
                .map_err(|e| CommError::ParseError(format!("invalid tcp port: {e}")))?,
            udp_port: Port::try_from(node.udp_port)
                .map_err(|e| CommError::ParseError(format!("invalid udp port: {e}")))?,
        },
    })
}

/// Convert a `PeerNode` to a Kademlia proto `Node` (port of `discovery.toNode`).
pub fn to_node(peer: &PeerNode) -> Node {
    Node {
        id: peer.key().to_vec(),
        host: peer.endpoint.host.as_bytes().to_vec(),
        tcp_port: u32::from(peer.endpoint.tcp_port),
        udp_port: u32::from(peer.endpoint.udp_port),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &[u8], host: &[u8], tcp: u32, udp: u32) -> Node {
        Node {
            id: id.to_vec(),
            host: host.to_vec(),
            tcp_port: tcp,
            udp_port: udp,
        }
    }

    fn peer(name: &str) -> PeerNode {
        PeerNode::from(
            NodeIdentifier::new(name.as_bytes().to_vec()),
            "host".to_string(),
            Port::new(40400),
            Port::new(40404),
        )
    }

    /// The proto boundary round trips: the id is the raw key, the host is UTF-8 text, and the two
    /// ports keep their kinds (`tcp_port`/`udp_port`) — a swap here would break every handshake.
    #[test]
    fn a_peer_node_round_trips_through_the_proto() {
        let p = peer("alpha");
        let wire = to_node(&p);
        assert_eq!(wire.id, b"alpha");
        assert_eq!(wire.tcp_port, 40400);
        assert_eq!(wire.udp_port, 40404);
        assert_eq!(to_peer_node(&wire).expect("round trip"), p);
    }

    /// A non-UTF-8 host is carried lossily rather than failing the conversion (the proto field is
    /// byte-oriented), matching the Scala `new String(host)` shape. The ports are what can fail.
    #[test]
    fn a_non_utf8_host_is_carried_lossily() {
        let wire = node(b"id", &[0xff, 0xfe], 1, 2);
        let p = to_peer_node(&wire).expect("a bad host is not an error");
        assert!(!p.endpoint.host.is_empty(), "{p:?}");
    }

    /// **The two error arms.** A port outside `u16` is rejected, and the message says *which* port —
    /// a peer advertising a bad Kademlia port must not be accepted with a truncated one.
    #[test]
    fn an_out_of_range_port_is_rejected_by_name() {
        let bad_tcp = node(b"id", b"host", 70_000, 40404);
        let err = to_peer_node(&bad_tcp).expect_err("tcp port out of range");
        assert!(format!("{err}").contains("invalid tcp port"), "{err}");

        let bad_udp = node(b"id", b"host", 40400, 70_000);
        let err = to_peer_node(&bad_udp).expect_err("udp port out of range");
        assert!(format!("{err}").contains("invalid udp port"), "{err}");
    }

    /// The boundary accepts the whole valid range, inclusive — an off-by-one here would reject a
    /// legitimate port 65535.
    #[test]
    fn the_port_boundary_is_inclusive() {
        assert!(
            to_peer_node(&node(b"id", b"h", 0, 0)).is_ok(),
            "zero is a port"
        );
        assert!(
            to_peer_node(&node(b"id", b"h", 65_535, 65_535)).is_ok(),
            "65535 is a port"
        );
        assert!(to_peer_node(&node(b"id", b"h", 65_536, 0)).is_err());
    }
}
