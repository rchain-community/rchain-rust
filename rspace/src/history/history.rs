//! The abstract Merkle history (key-addressable hash reads + batched mutation).
//!
//! Mirrors `rspace/src/main/scala/coop/rchain/rspace/history/History.scala`.

use std::sync::Arc;

use async_trait::async_trait;
use rchain_crypto::hash::blake2b256_hash::Blake2b256Hash;

use crate::history::history_action::HistoryAction;
use crate::history::key_segment::KeySegment;
use crate::history::radix_tree::empty_root_hash;

/// The radix-history interface (port of `History[F]`).
#[async_trait]
pub trait History: Send + Sync {
    /// Read the value stored at `key` (port of `read`).
    ///
    /// Fallible: the oracle's `read` is `F[Option[Blake2b256Hash]]`, so a node that cannot be read is
    /// an error rather than "no value at this key" (AUDIT C53).
    async fn read(&self, key: &KeySegment) -> Result<Option<Blake2b256Hash>, String>;

    /// Apply a batch of insert/update/delete actions (port of `process`).
    async fn process(&self, actions: &[HistoryAction]) -> Result<Arc<dyn History>, String>;

    /// The current root hash (port of `root`).
    fn root(&self) -> Blake2b256Hash;

    /// Return a `History` rooted at `root` (port of `reset`).
    ///
    /// Fallible for the same reason as [`History::read`]: the root node is *loaded*, and a store that
    /// cannot be read must not become an empty root (AUDIT C53).
    async fn reset(&self, root: Blake2b256Hash) -> Result<Arc<dyn History>, String>;
}

/// The hash of the empty history root (port of `History.emptyRootHash`).
pub fn empty_root_hash_value() -> Blake2b256Hash {
    empty_root_hash()
}
