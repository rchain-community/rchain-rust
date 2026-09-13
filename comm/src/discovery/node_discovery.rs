//! Node discovery abstraction.
//!
//! Mirrors `comm/src/main/scala/coop/rchain/comm/discovery/NodeDiscovery.scala`.

use std::sync::Arc;

use async_trait::async_trait;

use crate::discovery::kademlia_node_discovery;
use crate::discovery::{KademliaRpc, KademliaStore};
use crate::peer_node::{NodeIdentifier, PeerNode};

/// The node discovery interface (port of `NodeDiscovery[F]`).
#[async_trait]
pub trait NodeDiscovery: Send + Sync {
    async fn discover(&self);
    fn peers(&self) -> Vec<PeerNode>;
}

/// The Kademlia-backed discovery (port of `NodeDiscoveryInstances.kademlia`).
pub struct KademliaNodeDiscovery {
    id: NodeIdentifier,
    store: Arc<dyn KademliaStore>,
    rpc: Arc<dyn KademliaRpc>,
}

impl KademliaNodeDiscovery {
    pub fn new(
        id: NodeIdentifier,
        store: Arc<dyn KademliaStore>,
        rpc: Arc<dyn KademliaRpc>,
    ) -> Self {
        KademliaNodeDiscovery { id, store, rpc }
    }
}

#[async_trait]
impl NodeDiscovery for KademliaNodeDiscovery {
    async fn discover(&self) {
        kademlia_node_discovery::discover(&self.id, self.store.as_ref(), self.rpc.as_ref()).await;
    }

    fn peers(&self) -> Vec<PeerNode> {
        kademlia_node_discovery::peers(self.store.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use rchain_shared::refined::Port;

    use crate::discovery::kademlia_store;
    use crate::peer_node::NodeIdentifier;

    /// A Kademlia RPC that answers with a fixed peer list and records what it was asked — the same
    /// shape as `kademlia_node_discovery.rs`'s own stub, kept here because that one is private to
    /// its module.
    #[derive(Default)]
    struct StubRpc {
        neighbours: Vec<PeerNode>,
        pings: Mutex<Vec<PeerNode>>,
        lookups: Mutex<Vec<(Vec<u8>, PeerNode)>>,
    }

    #[async_trait]
    impl KademliaRpc for StubRpc {
        async fn ping(&self, node: &PeerNode) -> bool {
            self.pings.lock().unwrap().push(node.clone());
            true
        }

        async fn lookup(&self, key: &[u8], peer: &PeerNode) -> Vec<PeerNode> {
            self.lookups
                .lock()
                .unwrap()
                .push((key.to_vec(), peer.clone()));
            self.neighbours.clone()
        }
    }

    fn peer(byte: u8) -> PeerNode {
        PeerNode::from(
            NodeIdentifier::new(vec![byte; 32]),
            "host".to_string(),
            Port::new(40400),
            Port::new(40404),
        )
    }

    fn discovery(rpc: Arc<StubRpc>) -> KademliaNodeDiscovery {
        KademliaNodeDiscovery::new(
            NodeIdentifier::new(vec![1u8; 32]),
            kademlia_store::table(&NodeIdentifier::new(vec![1u8; 32])),
            rpc,
        )
    }

    /// `peers()` is the **store's** peers, not the RPC's answer: discovery is what fills the table,
    /// and reporting the RPC's neighbours here would report peers this node has not recorded.
    #[tokio::test]
    async fn peers_comes_from_the_store() {
        let rpc = Arc::new(StubRpc {
            neighbours: vec![peer(9)],
            ..StubRpc::default()
        });
        let discovery = discovery(rpc.clone());

        assert!(discovery.peers().is_empty(), "a fresh store knows nobody");
        assert!(
            rpc.lookups.lock().unwrap().is_empty() && rpc.pings.lock().unwrap().is_empty(),
            "asking for the peers does not perform discovery"
        );
    }

    /// Discovery needs a **seed**: `discover` starts from the table's own peers, so with an empty
    /// table there is nobody to ask and the call returns without touching the RPC. A node with no
    /// peers must not spin asking nobody (the bootstrap peer is what seeds the table).
    #[tokio::test]
    async fn discover_on_an_empty_table_asks_nobody() {
        let rpc = Arc::new(StubRpc {
            neighbours: vec![peer(9)],
            ..StubRpc::default()
        });
        let discovery = discovery(rpc.clone());

        discovery.discover().await;

        assert!(
            rpc.lookups.lock().unwrap().is_empty() && rpc.pings.lock().unwrap().is_empty(),
            "no seed peer means no RPC call"
        );
        assert!(discovery.peers().is_empty(), "and nothing was learned");
    }

    /// With a seed peer in the table, `discover` asks **that** peer for the neighbours of a key
    /// derived from this node's own id — the Kademlia "flip a bit of your own key" walk, so the
    /// target differs from the local key in exactly one bit — and records what it learns.
    #[tokio::test]
    async fn discover_asks_a_seeded_peer_and_records_what_it_learns() {
        let rpc = Arc::new(StubRpc {
            neighbours: vec![peer(3)],
            ..StubRpc::default()
        });
        let discovery = discovery(rpc.clone());

        // Seed the table the way the connection path does.
        discovery
            .store
            .update_last_seen(peer(2));
        assert_eq!(discovery.peers(), vec![peer(2)]);

        discovery.discover().await;

        let lookups = rpc.lookups.lock().unwrap();
        assert_eq!(lookups.len(), 1, "one lookup, from the one seed peer");
        assert_eq!(lookups[0].1, peer(2), "asked of the seed peer");
        let target = &lookups[0].0;
        let local = vec![1u8; 32];
        assert_eq!(target.len(), local.len(), "the target is a key of the same width");
        let differing_bits: u32 = target
            .iter()
            .zip(local.iter())
            .map(|(a, b)| (a ^ b).count_ones())
            .sum();
        assert_eq!(
            differing_bits, 1,
            "the walk flips exactly one bit of the local key: {target:?}"
        );

        assert!(
            discovery.peers().contains(&peer(3)),
            "the peer the RPC returned is recorded: {:?}",
            discovery.peers()
        );
    }
}
