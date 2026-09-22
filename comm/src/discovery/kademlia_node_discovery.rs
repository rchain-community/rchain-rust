//! Kademlia iterative node discovery.
//!
//! Mirrors `comm/src/main/scala/coop/rchain/comm/discovery/KademliaNodeDiscovery.scala`.

use std::collections::HashSet;

use rand::seq::SliceRandom;

use crate::discovery::{KademliaRpc, KademliaStore};
use crate::peer_node::{NodeIdentifier, PeerNode};

/// Return up to `limit` candidate peers (port of `KademliaNodeDiscovery.discover`).
pub async fn discover(id: &NodeIdentifier, store: &dyn KademliaStore, rpc: &dyn KademliaRpc) {
    let mut peers = store.peers();
    peers.shuffle(&mut rand::thread_rng());
    let dists = store.sparseness();
    let result = find(10, &dists, peers, HashSet::new(), id, store, rpc).await;
    for peer in result {
        store.update_last_seen(peer);
    }
}

/// Return the store's peers (port of `KademliaNodeDiscovery.peers`).
pub fn peers(store: &dyn KademliaStore) -> Vec<PeerNode> {
    store.peers()
}

async fn find(
    limit: usize,
    dists: &[usize],
    mut peer_set: Vec<PeerNode>,
    mut potentials: HashSet<PeerNode>,
    id: &NodeIdentifier,
    store: &dyn KademliaStore,
    rpc: &dyn KademliaRpc,
) -> Vec<PeerNode> {
    let mut i = 0;
    while !peer_set.is_empty() && potentials.len() < limit && i < dists.len() {
        let dist = dists[i];
        let mut target = id.key().to_vec();
        let byte_index = dist / 8;
        let different_bit = 1u8 << (dist % 8);
        target[byte_index] ^= different_bit;

        let head = peer_set.remove(0);
        let found = rpc.lookup(&target, &head).await;
        for p in filter(found, &potentials, id, store) {
            potentials.insert(p);
        }
        i += 1;
    }
    potentials.into_iter().collect()
}

fn filter(
    peers: Vec<PeerNode>,
    potentials: &HashSet<PeerNode>,
    id: &NodeIdentifier,
    store: &dyn KademliaStore,
) -> Vec<PeerNode> {
    peers
        .into_iter()
        .filter(|p| !potentials.contains(p) && p.id.key() != id.key())
        .filter(|p| store.find(p.id.key()).is_none())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use rchain_shared::refined::Port;

    use crate::discovery::kademlia_store::table;

    fn peer(byte: u8) -> PeerNode {
        PeerNode::from(
            NodeIdentifier::new(vec![byte; 32]),
            "host".to_string(),
            Port::new(40400),
            Port::new(40404),
        )
    }

    fn id(byte: u8) -> NodeIdentifier {
        NodeIdentifier::new(vec![byte; 32])
    }

    /// Records every lookup target, so the XOR arithmetic can be asserted directly.
    #[derive(Default)]
    struct RecordingRpc {
        targets: Mutex<Vec<(Vec<u8>, PeerNode)>>,
        /// What every lookup returns.
        found: Vec<PeerNode>,
    }

    #[async_trait::async_trait]
    impl KademliaRpc for RecordingRpc {
        async fn ping(&self, _node: &PeerNode) -> bool {
            true
        }
        async fn lookup(&self, key: &[u8], peer: &PeerNode) -> Vec<PeerNode> {
            self.targets
                .lock()
                .unwrap()
                .push((key.to_vec(), peer.clone()));
            self.found.clone()
        }
    }

    /// **The Kademlia distance step, pinned exactly.** Each iteration flips one bit of the local
    /// key: bit `d % 8` of byte `d / 8`, for the bucket index `d` that `sparseness()` reported. That
    /// flip *is* the "find a peer closer than this bucket" probe — an off-by-one in either division
    /// would probe the wrong neighbourhood and silently degrade discovery to a random walk.
    #[tokio::test]
    async fn each_lookup_targets_the_bit_flip_of_its_bucket() {
        let store = table(&id(0));
        let rpc = RecordingRpc {
            targets: Mutex::new(Vec::new()),
            found: Vec::new(),
        };
        let local = id(0);
        // Distances chosen to cover both a byte boundary and the last bit of a byte.
        let dists = vec![0usize, 1, 7, 8, 15];
        let peer_set = vec![peer(1), peer(2), peer(3), peer(4), peer(5)];

        let _ = find(
            10,
            &dists,
            peer_set,
            HashSet::new(),
            &local,
            store.as_ref(),
            &rpc,
        )
        .await;

        let targets = rpc.targets.lock().unwrap();
        assert_eq!(targets.len(), dists.len(), "one lookup per distance");
        for (i, (target, _)) in targets.iter().enumerate() {
            let dist = dists[i];
            let mut expected = vec![0u8; 32];
            expected[dist / 8] ^= 1u8 << (dist % 8);
            assert_eq!(
                target,
                &expected,
                "distance {dist} must flip bit {} of byte {}",
                dist % 8,
                dist / 8
            );
        }
    }

    /// The candidate filter's three exclusions: a peer already collected, the local node itself,
    /// and a peer the store already knows. Without them the walk would loop on the same peers (the
    /// `limit` would never be reached, and `discover` would never terminate).
    #[test]
    fn the_filter_excludes_self_potentials_and_known_peers() {
        let local = id(0);
        let store = table(&local);
        let known = peer(9);
        store.update_last_seen(known.clone());

        let mut potentials: HashSet<PeerNode> = HashSet::new();
        let collected = peer(1);
        potentials.insert(collected.clone());

        let found = vec![
            collected.clone(),
            peer(0), // the local node itself (id 0)
            known.clone(),
            peer(2),
            peer(3),
        ];
        let filtered = filter(found, &potentials, &local, store.as_ref());

        assert_eq!(
            filtered,
            vec![peer(2), peer(3)],
            "only genuinely new peers survive the filter"
        );
    }

    /// **`limit` bounds the number of *iterations*, not the size of the result**: the check is at
    /// the top of the walk (`while … && potentials.len() < limit`), so one lookup that returns many
    /// peers can fill the set past the limit and ends the walk on the spot. That is the Scala
    /// `find`'s shape too, and it is what makes a single well-connected peer enough to finish
    /// discovery — but it also means the candidate set is bounded by what one peer knows, not by
    /// `limit`. Both halves are pinned: the walk stops after the first lookup, and it still keeps
    /// all twelve peers that lookup returned.
    #[tokio::test]
    async fn a_lookup_that_fills_the_potential_set_ends_the_walk() {
        let local = id(0);
        let store = table(&local);
        let found: Vec<PeerNode> = (1..=12u8).map(peer).collect();
        let rpc = RecordingRpc {
            targets: Mutex::new(Vec::new()),
            found,
        };
        // Five seeds and five distances: a walk that did not stop early would make five lookups.
        let seeds: Vec<PeerNode> = (20..=24u8).map(peer).collect();

        let result = find(
            10,
            &[0, 1, 7, 8, 15],
            seeds,
            HashSet::new(),
            &local,
            store.as_ref(),
            &rpc,
        )
        .await;

        assert_eq!(
            rpc.targets.lock().unwrap().len(),
            1,
            "the set was full after the first lookup, so the walk must stop"
        );
        assert_eq!(result.len(), 12, "every peer that lookup returned is kept");
    }

    /// `discover` records what it found and never records the local node as a discovered peer.
    #[tokio::test]
    async fn discover_records_what_it_found_but_not_itself() {
        let local = id(0);
        let store = table(&local);
        let found: Vec<PeerNode> = (1..=12u8).map(peer).collect();
        let rpc = RecordingRpc {
            targets: Mutex::new(Vec::new()),
            found,
        };

        // One seed peer to start the walk from.
        store.update_last_seen(peer(20));
        discover(&local, store.as_ref(), &rpc).await;

        let known = store.peers();
        assert_eq!(known.len(), 13, "the seed plus the twelve it learned");
        assert!(
            !known.contains(&local_peer(&local)),
            "the local node is never recorded as a discovered peer"
        );
        assert!(
            !rpc.targets.lock().unwrap().is_empty(),
            "it must ask somebody"
        );
    }

    fn local_peer(local: &NodeIdentifier) -> PeerNode {
        PeerNode::from(
            local.clone(),
            "localhost".to_string(),
            Port::new(40400),
            Port::new(40404),
        )
    }

    /// An empty store asks nobody and records nothing — the walk's guard (`!peer_set.is_empty()`),
    /// which is what keeps a freshly started node from spinning.
    #[tokio::test]
    async fn discover_with_an_empty_store_is_a_no_op() {
        let local = id(0);
        let store = table(&local);
        let rpc = RecordingRpc {
            targets: Mutex::new(Vec::new()),
            found: Vec::new(),
        };

        discover(&local, store.as_ref(), &rpc).await;
        assert!(
            rpc.targets.lock().unwrap().is_empty(),
            "nothing to ask: no peer to ask through"
        );
        assert!(store.peers().is_empty());
    }
}
