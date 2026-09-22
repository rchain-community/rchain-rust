//! Kademlia routing-table store.
//!
//! Mirrors `comm/src/main/scala/coop/rchain/comm/discovery/KademliaStore.scala`. The store is
//! synchronous (the `F[_]` effect and gauge metrics are dropped), wrapping the synchronous
//! `PeerTable`.

use std::sync::Arc;

use crate::discovery::peer_table::{PeerTable, REDUNDANCY};
use crate::peer_node::{NodeIdentifier, PeerNode};

/// The Kademlia store (port of `KademliaStore[F]`).
pub trait KademliaStore: Send + Sync {
    fn peers(&self) -> Vec<PeerNode>;
    fn sparseness(&self) -> Vec<usize>;
    fn update_last_seen(&self, peer_node: PeerNode);
    fn lookup(&self, key: &[u8]) -> Vec<PeerNode>;
    fn find(&self, key: &[u8]) -> Option<PeerNode>;
    fn remove(&self, key: &[u8]);
}

/// Build a `PeerTable`-backed store (port of `KademliaStoreInstances.table`).
pub fn table(id: &NodeIdentifier) -> Arc<dyn KademliaStore> {
    Arc::new(TableKademliaStore {
        table: PeerTable::new(id.key().to_vec(), REDUNDANCY),
    })
}

struct TableKademliaStore {
    table: PeerTable<PeerNode>,
}

impl KademliaStore for TableKademliaStore {
    fn peers(&self) -> Vec<PeerNode> {
        self.table.peers()
    }

    fn sparseness(&self) -> Vec<usize> {
        self.table.sparseness()
    }

    fn update_last_seen(&self, peer_node: PeerNode) {
        self.table.update_last_seen(peer_node);
    }

    fn lookup(&self, key: &[u8]) -> Vec<PeerNode> {
        self.table.lookup(key)
    }

    fn find(&self, key: &[u8]) -> Option<PeerNode> {
        self.table.find(key)
    }

    fn remove(&self, key: &[u8]) {
        self.table.remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rchain_shared::refined::Port;

    fn node(byte: u8) -> PeerNode {
        PeerNode::from(
            NodeIdentifier::new(vec![byte; 32]),
            "host".to_string(),
            Port::new(40400),
            Port::new(40404),
        )
    }

    fn local(byte: u8) -> NodeIdentifier {
        NodeIdentifier::new(vec![byte; 32])
    }

    #[test]
    fn an_unknown_key_is_not_found() {
        let store = table(&local(0));
        assert_eq!(store.find(&[1u8; 32]), None);
        assert_eq!(store.find(&[0u8; 31]), None, "a short key matches nothing");
    }

    #[test]
    fn update_find_lookup_and_remove_round_trip() {
        let store = table(&local(0));
        let peer = node(1); // distance 7 from the all-zero local key

        store.update_last_seen(peer.clone());
        assert_eq!(store.find(peer.key()), Some(peer.clone()));
        assert_eq!(store.peers(), vec![peer.clone()]);
        // Looked up from the local key, the table's own origin: the peer is the closest thing the
        // table has, so it is what a caller asking "who do I gossip to" gets back.
        assert_eq!(store.lookup(&[0u8; 32]), vec![peer.clone()]);

        store.remove(peer.key());
        assert_eq!(store.find(peer.key()), None);
        assert!(store.peers().is_empty());
        assert!(store.lookup(&[0u8; 32]).is_empty());
    }

    /// The one surprising part of `lookup`: asked about a key, the table deliberately excludes the
    /// peer *holding* that key (you do not ask a peer where itself is). Pinned because it is the
    /// difference between "lookup returns the closest peer" and what the code actually does.
    #[test]
    fn lookup_excludes_the_peer_named_by_the_key() {
        let store = table(&local(0));
        let peer = node(1);
        store.update_last_seen(peer.clone());

        assert_eq!(
            store.lookup(peer.key()),
            Vec::new(),
            "asking about a peer's own key excludes that peer even when it is the only one there"
        );
        assert_eq!(
            store.find(peer.key()),
            Some(peer),
            "…but `find` still finds it"
        );
    }

    /// `update_last_seen` is the re-affirmation path: a peer seen again must not be duplicated,
    /// which is what keeps `peers()` a set rather than a log.
    #[test]
    fn updating_the_same_peer_twice_does_not_duplicate_it() {
        let store = table(&local(0));
        let peer = node(1);

        store.update_last_seen(peer.clone());
        store.update_last_seen(peer.clone());
        assert_eq!(store.peers(), vec![peer]);
    }

    /// `table(id)` builds a fresh table per call, so two stores never share state — including two
    /// stores built from the *same* identifier, which is what a second call at the same node does.
    #[test]
    fn two_stores_do_not_share_state() {
        let id = local(0);
        let first = table(&id);
        let second = table(&id);
        let peer = node(1);

        first.update_last_seen(peer.clone());
        assert_eq!(first.find(peer.key()), Some(peer.clone()));
        assert_eq!(
            second.find(peer.key()),
            None,
            "a store built for the same node must not see another's peers"
        );
        assert!(second.peers().is_empty());
    }

    /// The local identifier is the lookup origin, not decoration: `update_last_seen` files a peer
    /// under its distance from the *local* key, so moving the local key moves the peer's bucket.
    #[test]
    fn peers_are_filed_by_distance_from_the_stores_own_identifier() {
        let near = node(0x40); // 1 common bit with an all-zero local key, 0 with an all-0x80 one
        let far = node(0xC0); // the mirror image
        let zero = table(&local(0));
        let high = table(&local(0x80));

        for store in [&zero, &high] {
            store.update_last_seen(near.clone());
            store.update_last_seen(far.clone());
        }

        // Both keys are present in both stores, and `find` locates each via the local key's bucket.
        assert_eq!(zero.find(near.key()), Some(near.clone()));
        assert_eq!(zero.find(far.key()), Some(far.clone()));
        assert_eq!(high.find(near.key()), Some(near.clone()));
        assert_eq!(high.find(far.key()), Some(far));
        // A lookup from the local key reports the closer of the two first, and which one is closer
        // is decided by the local key — the whole point of a distance-indexed table.
        assert_eq!(
            zero.lookup(&[0u8; 32]).first(),
            Some(&near),
            "from an all-zero node, 0x40 is the nearest"
        );
        assert_eq!(
            high.lookup(&[0x80u8; 32]).first(),
            Some(&node(0xC0)),
            "from a 0x80 node, 0xC0 is the nearest"
        );
    }

    #[test]
    fn sparseness_ranks_every_bucket_of_an_empty_table() {
        let store = table(&local(0));
        // 256-bit keys, so 256 buckets; with no peers they all tie and the order is the identity.
        assert_eq!(store.sparseness(), (0..256).collect::<Vec<usize>>());
    }
}
