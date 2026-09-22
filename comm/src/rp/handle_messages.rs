//! Inbound protocol message dispatch.
//!
//! Mirrors `comm/src/main/scala/coop/rchain/comm/rp/HandleMessages.scala`. The fs2 routing-message
//! queue becomes a `tokio::sync::mpsc::Sender`.

use std::net::IpAddr;

use rchain_models::comm::protocol::{protocol, Packet, Protocol};

use crate::errors::CommError;
use crate::peer_node::PeerNode;
use crate::rp::connect::{add_conn, refresh_conn, remove_conn};
use crate::rp::protocol_helper;
use crate::rp::rp_conf::RPConf;
use crate::transport::communication_response::CommunicationResponse;
use crate::transport::transport_layer::TransportLayer;

/// A routing packet addressed from a peer (port of `RoutingMessage`).
#[derive(Clone, Debug)]
pub struct RoutingMessage {
    pub peer: PeerNode,
    pub packet: Packet,
}

/// Whether a host is a local/private address (port of `HandleMessages.isLocalAddress`). Classifies
/// both IPv4 and IPv6 literals. Hostnames are not resolved (see the residual note below).
pub fn is_local_address(host: &str) -> bool {
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => {
            let o = ip.octets();
            ip.is_unspecified() // 0.0.0.0
                || ip.is_loopback() // 127/8
                || ip.is_multicast() // 224/4
                || (o[0] == 169 && o[1] == 254) // link-local 169.254/16
                || o[0] == 10 // 10/8
                || (o[0] == 172 && (16..=31).contains(&o[1])) // 172.16/12
                || (o[0] == 192 && o[1] == 168) // 192.168/16
        }
        Ok(IpAddr::V6(ip)) => {
            ip.is_unspecified() // ::
                || ip.is_loopback() // ::1
                || ip.is_multicast() // ff00::/8
                || ip.is_unique_local() // fc00::/7
                || ip.is_unicast_link_local() // fe80::/10
        }
        // Residual: a hostname that resolves to a private/loopback address is not classified here
        // (resolution is async and DNS-rebinding-prone). The IPv6-literal bypass is closed.
        Err(_) => false,
    }
}

/// Whether `peer` is on the same subnetwork class (public vs local) as the local node (port of
/// `checkPeerOnSameNetwork`).
pub fn check_peer_on_same_network(conf: &RPConf, peer: &PeerNode) -> bool {
    is_local_address(&conf.local.endpoint.host) == is_local_address(&peer.endpoint.host)
}

/// Dispatch an inbound protocol message (port of `handle`).
///
/// The connections cell is an `RwLock` rather than `&mut Vec` so the write lock is held only for the
/// brief mutation, never across the outbound `send` in the handshake path (H3).
///
/// Residual (documented, not fixed): the protocol `sender` (taken from the message header) is not
/// cryptographically bound to the TLS peer certificate. A peer presenting a self-signed certificate
/// may assert an arbitrary node id. The `MAX_CONNECTIONS` bound mitigates the resulting unbounded
/// growth of the connection table, but does not eliminate the identity-spoofing vector.
pub async fn handle<T: TransportLayer + ?Sized>(
    proto: Protocol,
    conf: &RPConf,
    transport: &T,
    connections: &tokio::sync::RwLock<Vec<PeerNode>>,
    routing_queue: &tokio::sync::mpsc::Sender<RoutingMessage>,
) -> CommunicationResponse {
    let sender = match protocol_helper::sender(&proto) {
        Ok(s) => s,
        Err(e) => return CommunicationResponse::not_handled(e),
    };
    match proto.message {
        Some(protocol::Message::Heartbeat(_)) => {
            let mut conns = connections.write().await;
            *conns = refresh_conn(&*conns, &sender);
            CommunicationResponse::handled_without_message()
        }
        Some(protocol::Message::ProtocolHandshake(_)) => {
            handle_protocol_handshake(transport, conf, connections, &sender).await
        }
        Some(protocol::Message::ProtocolHandshakeResponse(_)) => {
            let mut conns = connections.write().await;
            *conns = add_conn(&*conns, &[sender]);
            CommunicationResponse::handled_without_message()
        }
        Some(protocol::Message::Disconnect(_)) => {
            let mut conns = connections.write().await;
            *conns = remove_conn(&*conns, &[sender]);
            CommunicationResponse::handled_without_message()
        }
        Some(protocol::Message::Packet(packet)) => {
            let _ = routing_queue.try_send(RoutingMessage {
                peer: sender,
                packet,
            });
            CommunicationResponse::handled_without_message()
        }
        other => {
            CommunicationResponse::not_handled(CommError::UnexpectedMessage(format!("{other:?}")))
        }
    }
}

/// Handle an inbound protocol handshake (port of `handleProtocolHandshake`): accept only peers on
/// the same subnetwork class, respond with a handshake response, and record the connection. The
/// response `send` runs *before* the connections lock is taken, so a slow peer cannot stall the
/// connection table (H3).
pub async fn handle_protocol_handshake<T: TransportLayer + ?Sized>(
    transport: &T,
    conf: &RPConf,
    connections: &tokio::sync::RwLock<Vec<PeerNode>>,
    peer: &PeerNode,
) -> CommunicationResponse {
    if check_peer_on_same_network(conf, peer) {
        let response = protocol_helper::protocol_handshake_response(&conf.local, &conf.network_id);
        if transport.send(peer, response).await.is_ok() {
            let mut conns = connections.write().await;
            *conns = add_conn(&*conns, std::slice::from_ref(peer));
        }
    }
    CommunicationResponse::handled_without_message()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_private_addresses_as_local() {
        for host in [
            "0.0.0.0",
            "127.0.0.1",
            "10.0.0.1",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
        ] {
            assert!(is_local_address(host), "{host} should be local");
        }
    }

    #[test]
    fn classifies_public_addresses_as_remote() {
        for host in ["8.8.8.8", "1.2.3.4", "172.32.0.1", "192.169.0.1"] {
            assert!(!is_local_address(host), "{host} should be public");
        }
    }

    /// **The same-network check is about *locality class*, not equality.** A peer is on the same
    /// network when both ends are local (a private/devnet address) or both are public — so a
    /// localhost node accepts a LAN peer, and refuses a public one. The `handle` path uses this to
    /// decide whether to answer a handshake, so an inversion here would make a devnet unreachable
    /// (or a public node accept only public peers).
    #[test]
    fn the_same_network_check_compares_locality_class() {
        let conf = |local_host: &str| RPConf {
            local: peer("local", local_host),
            network_id: "testnet".to_string(),
            bootstrap: None,
            default_timeout: std::time::Duration::from_secs(10),
            max_num_of_connections: 10,
            clear_connections: crate::rp::rp_conf::ClearConnectionsConf {
                num_of_connections_pinged: 10,
            },
        };

        let local_conf = conf("127.0.0.1");
        assert!(
            check_peer_on_same_network(&local_conf, &peer("lan", "192.168.1.5")),
            "a local node accepts another private peer"
        );
        assert!(
            !check_peer_on_same_network(&local_conf, &peer("public", "8.8.8.8")),
            "…and refuses a public one"
        );

        let public_conf = conf("8.8.8.8");
        assert!(
            check_peer_on_same_network(&public_conf, &peer("other", "1.1.1.1")),
            "a public node accepts a public peer"
        );
        assert!(
            !check_peer_on_same_network(&public_conf, &peer("private", "10.0.0.1")),
            "…and refuses a private one"
        );
    }

    /// The same-network check is *symmetric* in its two arguments' roles, since it compares the two
    /// classifications — the property a caller relies on when it applies the check to either side.
    #[test]
    fn the_same_network_check_is_symmetric() {
        let conf = |host: &str| RPConf {
            local: peer("local", host),
            network_id: "testnet".to_string(),
            bootstrap: None,
            default_timeout: std::time::Duration::from_secs(10),
            max_num_of_connections: 10,
            clear_connections: crate::rp::rp_conf::ClearConnectionsConf {
                num_of_connections_pinged: 10,
            },
        };
        for (a, b) in [
            ("127.0.0.1", "10.0.0.1"),
            ("127.0.0.1", "8.8.8.8"),
            ("8.8.8.8", "1.1.1.1"),
        ] {
            assert_eq!(
                check_peer_on_same_network(&conf(a), &peer("p", b)),
                check_peer_on_same_network(&conf(b), &peer("p", a)),
                "({a}, {b}) must classify the same way as ({b}, {a})"
            );
        }
    }

    fn peer(name: &str, host: &str) -> PeerNode {
        use rchain_shared::refined::Port;
        PeerNode::from(
            crate::peer_node::NodeIdentifier::new(name.as_bytes().to_vec()),
            host.to_string(),
            Port::new(40400),
            Port::new(40404),
        )
    }
}
