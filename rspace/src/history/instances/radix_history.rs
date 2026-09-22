//! The `History` implementation over `RadixTreeImpl`.
//!
//! Mirrors `rspace/src/main/scala/coop/rchain/rspace/history/instances/RadixHistory.scala`.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use rchain_crypto::hash::blake2b256_hash::Blake2b256Hash;
use rchain_shared::typed_store::KeyValueTypedStore;

use crate::history::history::History;
use crate::history::history_action::HistoryAction;
use crate::history::key_segment::KeySegment;
use crate::history::radix_tree::{empty_root_hash, Node, RadixTreeImpl};

/// The radix-tree `History` (port of `RadixHistory`).
pub struct RadixHistory {
    root_hash: Blake2b256Hash,
    root_node: Node,
    impl_: Arc<RadixTreeImpl>,
    store: Arc<dyn KeyValueTypedStore<Blake2b256Hash, Vec<u8>>>,
}

impl RadixHistory {
    pub async fn new(
        root: Blake2b256Hash,
        store: Arc<dyn KeyValueTypedStore<Blake2b256Hash, Vec<u8>>>,
    ) -> Arc<dyn History> {
        let impl_ = Arc::new(RadixTreeImpl::new(store.clone()));
        let root_node = impl_.load_node(root, true).await;
        Arc::new(RadixHistory {
            root_hash: root,
            root_node,
            impl_,
            store,
        })
    }

    fn copy(&self, root_hash: Blake2b256Hash, root_node: Node, impl_: Arc<RadixTreeImpl>) -> Self {
        RadixHistory {
            root_hash,
            root_node,
            impl_,
            store: self.store.clone(),
        }
    }
}

fn has_no_duplicates(actions: &[HistoryAction]) -> bool {
    let keys: Vec<&KeySegment> = actions.iter().map(|a| a.key()).collect();
    let set: HashSet<&KeySegment> = keys.iter().copied().collect();
    set.len() == keys.len()
}

#[async_trait]
impl History for RadixHistory {
    fn root(&self) -> Blake2b256Hash {
        self.root_hash
    }

    async fn read(&self, key: &KeySegment) -> Option<Blake2b256Hash> {
        self.impl_.read(&self.root_node, key).await
    }

    async fn process(&self, actions: &[HistoryAction]) -> Result<Arc<dyn History>, String> {
        assert!(
            has_no_duplicates(actions),
            "Cannot process duplicate actions on one key."
        );
        let result = self.impl_.save_and_commit(&self.root_node, actions).await?;
        self.impl_.clear_read_cache();
        match result {
            Some((new_root_node, new_root_hash)) => Ok(Arc::new(self.copy(
                new_root_hash,
                new_root_node,
                self.impl_.clone(),
            ))),
            None => Ok(Arc::new(self.copy(
                self.root_hash,
                self.root_node.clone(),
                self.impl_.clone(),
            ))),
        }
    }

    async fn reset(&self, root: Blake2b256Hash) -> Arc<dyn History> {
        let impl_ = Arc::new(RadixTreeImpl::new(self.store.clone()));
        let root_node = impl_.load_node(root, true).await;
        Arc::new(self.copy(root, root_node, impl_))
    }
}

/// The empty root hash (port of `RadixHistory.emptyRootHash`).
pub fn empty_root() -> Blake2b256Hash {
    empty_root_hash()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc as StdArc;

    use rchain_shared::store_manager::{database, InMemoryStoreManager};
    use rchain_shared::typed_store::BytesCodec;

    use crate::history::codecs::Blake2b256HashCodec;

    /// A history over an in-memory store — the same construction the node's factory uses
    /// (`database` from a manager), minus the LMDB backend, so the store codec pair is the one the
    /// node actually runs with rather than a test stand-in.
    async fn history() -> (Arc<dyn History>, Blake2b256Hash) {
        let manager = InMemoryStoreManager::default();
        let store = database(
            &manager,
            "history",
            StdArc::new(Blake2b256HashCodec),
            StdArc::new(BytesCodec),
        )
        .await
        .expect("in-memory store");
        let root = empty_root();
        (RadixHistory::new(root, StdArc::new(store)).await, root)
    }

    fn key(b: u8) -> KeySegment {
        KeySegment::new(vec![b])
    }

    fn insert(byte: u8, value: u8) -> HistoryAction {
        HistoryAction::Insert {
            key: key(byte),
            hash: Blake2b256Hash::from_bytes([value; 32]),
        }
    }

    /// A fresh history is the empty root, and reading an absent key is `None` rather than an error
    /// or a panic — the read path runs before any commit on a new node.
    #[tokio::test]
    async fn a_new_history_is_empty_and_reads_absent_keys_as_none() {
        let (history, root) = history().await;
        assert_eq!(history.root(), root);
        assert_eq!(history.root(), empty_root_hash());
        assert_eq!(history.read(&key(1)).await, None);
    }

    /// Committing an insert moves the root, and the **same actions from the same starting root
    /// produce the same root** — the content-addressing property the state hash depends on.
    #[tokio::test]
    async fn the_same_actions_from_the_same_root_give_the_same_root() {
        let (first, _) = history().await;
        let (second, _) = history().await;
        let actions = [insert(1, 0x11), insert(2, 0x22)];

        let a = first.process(&actions).await.expect("commit");
        let b = second.process(&actions).await.expect("commit");
        assert_eq!(a.root(), b.root());
        assert_ne!(a.root(), empty_root(), "the root moved");

        // The committed values are readable at the new root…
        assert_eq!(
            a.read(&key(1)).await,
            Some(Blake2b256Hash::from_bytes([0x11; 32]))
        );
        assert_eq!(
            a.read(&key(2)).await,
            Some(Blake2b256Hash::from_bytes([0x22; 32]))
        );
        // …and the history it came from is unchanged: `process` returns a *new* history.
        assert_eq!(first.root(), empty_root());
        assert_eq!(first.read(&key(1)).await, None);
    }

    /// An empty action list commits nothing and returns a history at the same root (the
    /// `save_and_commit` `None` arm), rather than a new root that would split the state hash on a
    /// no-op.
    #[tokio::test]
    async fn committing_no_actions_leaves_the_root_where_it_was() {
        let (history, root) = history().await;
        let committed = history.process(&[]).await.expect("commit");
        assert_eq!(committed.root(), root);

        // The same on a non-empty history, where the arm is reachable in practice.
        let after = history.process(&[insert(1, 0x11)]).await.expect("commit");
        assert_ne!(after.root(), root);
        let no_op = after.process(&[]).await.expect("commit");
        assert_eq!(no_op.root(), after.root());
        assert_eq!(no_op.read(&key(1)).await, after.read(&key(1)).await);
    }

    /// Two actions on one key are refused by an assertion: `save_and_commit` would apply them in an
    /// unspecified order, and "insert then delete" has no deterministic merge.
    #[tokio::test]
    #[should_panic(expected = "Cannot process duplicate actions on one key.")]
    async fn two_actions_on_one_key_are_refused() {
        let (history, _) = history().await;
        let _ = history.process(&[insert(1, 0x11), insert(1, 0x22)]).await;
    }

    /// The duplicate check is on the **key**, not on the whole action: an insert and a delete on the
    /// same key are duplicates too (they are the ambiguous pair the assertion is about).
    #[test]
    fn the_duplicate_check_is_by_key_not_by_action() {
        assert!(has_no_duplicates(&[]));
        assert!(has_no_duplicates(&[insert(1, 0x11), insert(2, 0x22)]));
        assert!(!has_no_duplicates(&[insert(1, 0x11), insert(1, 0x11)]));
        assert!(
            !has_no_duplicates(&[insert(1, 0x11), insert(1, 0x22)]),
            "different values on one key are still a duplicate"
        );
        assert!(
            !has_no_duplicates(&[insert(1, 0x11), HistoryAction::Delete { key: key(1) }]),
            "an insert and a delete on one key are the ambiguous pair"
        );
    }

    /// `reset` returns to an earlier root and can re-read what was there: it is how a replay rewinds
    /// the state without replaying every commit.
    #[tokio::test]
    async fn reset_returns_to_an_earlier_root() {
        let (history, root) = history().await;
        let after = history.process(&[insert(1, 0x11)]).await.expect("commit");

        let rewound = after.reset(root).await;
        assert_eq!(rewound.root(), root);
        assert_eq!(rewound.read(&key(1)).await, None, "the insert is gone");

        // …and the history it was reset from still has it (the reset was on a new value).
        assert_eq!(after.root(), after.root());
        assert_eq!(
            after.read(&key(1)).await,
            Some(Blake2b256Hash::from_bytes([0x11; 32]))
        );

        // Resetting to the newer root again finds the value: the store kept both.
        let back = rewound.reset(after.root()).await;
        assert_eq!(back.root(), after.root());
        assert_eq!(
            back.read(&key(1)).await,
            Some(Blake2b256Hash::from_bytes([0x11; 32]))
        );
    }

    /// A delete commits like an insert, and the value stops being readable at the new root while
    /// the old root still has it — the two roots coexisting in one store is what makes the previous
    /// state hash of a block reproducible.
    #[tokio::test]
    async fn a_delete_removes_the_key_at_the_new_root_only() {
        let (history, _) = history().await;
        let inserted = history.process(&[insert(1, 0x11)]).await.expect("commit");
        let deleted = inserted
            .process(&[HistoryAction::Delete { key: key(1) }])
            .await
            .expect("commit");

        assert_ne!(deleted.root(), inserted.root());
        assert_eq!(deleted.read(&key(1)).await, None);
        assert_eq!(
            inserted.read(&key(1)).await,
            Some(Blake2b256Hash::from_bytes([0x11; 32])),
            "the earlier root still reads the value"
        );
    }
}
