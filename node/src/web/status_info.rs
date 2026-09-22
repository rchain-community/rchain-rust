//! Node status (port of `web/StatusInfo.scala`).
//!
//! The http4s `service` is deferred; the `status` builder is ported as a synchronous function over
//! the comm state (the cats-effect `ConnectionsCell`/`NodeDiscovery`/`RPConfAsk` are simplified to
//! `&[PeerNode]`/`&RPConf`).

use rchain_comm::peer_node::PeerNode;
use rchain_comm::rp::rp_conf::RPConf;
use serde::Serialize;

/// Lightweight node status (port of `StatusInfo.Status`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Status {
    pub address: String,
    pub version: String,
    pub peers: i32,
    pub nodes: i32,
}

/// Build the node status from the comm state (port of `StatusInfo.status`).
pub fn status(
    version: &str,
    connections: &[PeerNode],
    discovered: &[PeerNode],
    rp_conf: &RPConf,
) -> Status {
    Status {
        address: rp_conf.local.to_address(),
        version: version.to_string(),
        peers: connections.len() as i32,
        nodes: discovered.len() as i32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rchain_comm::peer_node::NodeIdentifier;
    use rchain_comm::rp::rp_conf::ClearConnectionsConf;
    use std::time::Duration;

    fn peer(id: u8, host: &str) -> PeerNode {
        PeerNode::from(
            NodeIdentifier::new(vec![id]),
            host.to_string(),
            rchain_shared::refined::Port::new(40400),
            rchain_shared::refined::Port::new(40404),
        )
    }

    fn rp_conf() -> RPConf {
        RPConf {
            local: peer(1, "localhost"),
            network_id: "testnet".to_string(),
            bootstrap: None,
            default_timeout: Duration::from_secs(10),
            max_num_of_connections: 100,
            clear_connections: ClearConnectionsConf {
                num_of_connections_pinged: 10,
            },
        }
    }

    #[test]
    fn status_fields_are_accessible() {
        let status = Status {
            address: "addr".to_string(),
            version: "v1".to_string(),
            peers: 3,
            nodes: 5,
        };
        assert_eq!(status.address, "addr");
        assert_eq!(status.version, "v1");
        assert_eq!(status.peers, 3);
        assert_eq!(status.nodes, 5);
    }

    #[test]
    fn status_builds_from_comm_state() {
        let connections = vec![peer(2, "a"), peer(3, "b")];
        let discovered = vec![peer(2, "a"), peer(3, "b"), peer(4, "c")];
        let status = status("v1.0", &connections, &discovered, &rp_conf());
        assert_eq!(
            status.address,
            "rnode://01@localhost?protocol=40400&discovery=40404"
        );
        assert_eq!(status.version, "v1.0");
        assert_eq!(status.peers, 2);
        assert_eq!(status.nodes, 3);
    }

    /// The degenerate comm state — before the first peer connects, which is where a node spends
    /// its first seconds and where a hardcoded or unwired counter shows up as a silent `0` rather
    /// than as a crash. The counts must be *derived* from the two slices, so an empty one reports
    /// zero while a non-empty one still counts.
    #[test]
    fn a_lone_node_reports_zero_peers_and_its_own_address() {
        let lone = status("v1.0", &[], &[], &rp_conf());
        assert_eq!(lone.peers, 0);
        assert_eq!(lone.nodes, 0);
        assert_eq!(
            lone.address, "rnode://01@localhost?protocol=40400&discovery=40404",
            "a node with no peers still names itself"
        );

        // One side empty, the other not: the two counts must not share a source.
        let only_connections = status("v1.0", &[peer(2, "a")], &[], &rp_conf());
        assert_eq!((only_connections.peers, only_connections.nodes), (1, 0));
        let only_discovered = status("v1.0", &[], &[peer(2, "a")], &rp_conf());
        assert_eq!((only_discovered.peers, only_discovered.nodes), (0, 1));
    }

    /// The field names are the `/api/status` wire contract; renaming one silently breaks every
    /// client, so the serialized shape is asserted here rather than left to the HTTP tests.
    #[test]
    fn the_serialized_field_names_are_the_wire_contract() {
        let built = status(
            "v1.0",
            &[peer(2, "a")],
            &[peer(2, "a"), peer(3, "b")],
            &rp_conf(),
        );
        let json = serde_json::to_value(&built).expect("a status serializes");
        assert_eq!(
            json,
            serde_json::json!({
                "address": "rnode://01@localhost?protocol=40400&discovery=40404",
                "version": "v1.0",
                "peers": 1,
                "nodes": 2,
            })
        );
    }
}
