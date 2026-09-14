//! Pure Kademlia RPC handlers.
//!
//! Mirrors `comm/src/main/scala/coop/rchain/comm/discovery/KademliaHandleRPC.scala`.

use crate::discovery::kademlia_store::KademliaStore;
use crate::peer_node::PeerNode;

/// Handle an inbound ping (port of `handlePing`).
pub fn handle_ping(store: &dyn KademliaStore, peer: PeerNode) {
    store.update_last_seen(peer);
}

/// Handle an inbound lookup (port of `handleLookup`).
pub fn handle_lookup(store: &dyn KademliaStore, peer: PeerNode, id: &[u8]) -> Vec<PeerNode> {
    store.update_last_seen(peer);
    store.lookup(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use rchain_shared::refined::Port;

    use crate::discovery::kademlia_store;
    use crate::peer_node::NodeIdentifier;

    /// The real table-backed store (the same one the node builds), so the handlers run against the
    /// store they actually run with rather than a stub that agrees with them. The local key is 32
    /// bytes because the peer table measures distance over the key's bits — a shorter key is a
    /// *different* key space, and `update_last_seen` ignores a peer whose key length differs.
    fn store() -> Arc<dyn KademliaStore> {
        kademlia_store::table(&NodeIdentifier::new(vec![0x01; 32]))
    }

    fn peer(byte: u8) -> PeerNode {
        PeerNode::from(
            NodeIdentifier::new(vec![byte; 32]),
            "host".to_string(),
            Port::new(40400),
            Port::new(40404),
        )
    }

    /// A ping's whole effect is `update_last_seen`: the sender is remembered so a later lookup can
    /// return it. A handler that did nothing would leave the table unable to learn about the peers
    /// that ping it.
    #[test]
    fn handling_a_ping_remembers_the_sender() {
        let store = store();
        assert!(store.peers().is_empty(), "a fresh table is empty");

        let pinger = peer(2);
        handle_ping(store.as_ref(), pinger.clone());

        assert_eq!(
            store.peers(),
            vec![pinger.clone()],
            "the sender is now known"
        );

        // Pinging again refreshes the entry rather than duplicating it.
        handle_ping(store.as_ref(), pinger.clone());
        assert_eq!(store.peers(), vec![pinger]);
    }

    /// A lookup remembers the sender **and** answers from the table: the answer is the store's own
    /// lookup, so the handler does not invent peers the table does not have.
    #[test]
    fn handling_a_lookup_remembers_the_sender_and_answers_from_the_table() {
        let store = store();
        let asker = peer(3);

        // The asker is recorded on the way through, even though the lookup itself finds nothing.
        let found = handle_lookup(store.as_ref(), asker.clone(), &[0xFF; 32]);
        assert!(
            found.is_empty() || found == vec![asker.clone()],
            "the table only knows the asker: {found:?}"
        );
        assert_eq!(store.peers(), vec![asker.clone()], "the sender was seen");

        // A lookup answers with the peers **near** the key, and never with the peer whose key *is*
        // the key being looked up: `PeerTable::lookup` filters that entry out. So with the target
        // in the table, a lookup for the target's own key returns the asker and not the target —
        // the answer is the neighbourhood, not the thing itself.
        let target = peer(4);
        store.update_last_seen(target.clone());
        assert!(store.peers().contains(&target), "the target was added");

        let found = handle_lookup(store.as_ref(), asker.clone(), target.key());
        assert!(
            !found.contains(&target),
            "a key's own peer is never in the answer to a lookup for that key: {found:?}"
        );
        assert_eq!(
            found,
            vec![asker],
            "the asker is the target's only neighbour"
        );

        // …and a lookup for a key nobody holds returns the whole table, the target included.
        let found = handle_lookup(store.as_ref(), peer(9), &[0x55; 32]);
        assert_eq!(found.len(), store.peers().len(), "the table, minus nobody");
    }
}
