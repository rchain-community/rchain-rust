//! Trie export/import and state-manager abstractions.
//!
//! Mirrors `shared/src/main/scala/coop/rchain/state/{TrieExporter,TrieImporter,StateManager}.scala`.
//! The Scala `F[_]` effect and `ByteBuffer` zero-copy handling are simplified to synchronous
//! `Vec<u8>` operations, matching the crate's `store` module convention.

/// A trie node with its path from the root (port of `TrieNode`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrieNode<KeyHash> {
    pub hash: KeyHash,
    pub is_leaf: bool,
    pub path: Vec<(KeyHash, Option<u8>)>,
}

/// Traverses a trie and converts it to path-indexed nodes (port of `TrieExporter`).
pub trait TrieExporter<KeyHash: Clone> {
    /// Get trie nodes with offset from the start path and a number of nodes.
    fn get_nodes(
        &self,
        start_path: &[(KeyHash, Option<u8>)],
        skip: usize,
        take: usize,
    ) -> Vec<TrieNode<KeyHash>>;

    /// Get history values (branch nodes) by key.
    fn get_history_items<Value>(
        &self,
        keys: &[KeyHash],
        from_buffer: impl Fn(&[u8]) -> Value,
    ) -> Vec<(KeyHash, Value)>;

    /// Get data values (leaf nodes) by key.
    fn get_data_items<Value>(
        &self,
        keys: &[KeyHash],
        from_buffer: impl Fn(&[u8]) -> Value,
    ) -> Vec<(KeyHash, Value)>;

    // --- The checked siblings ---------------------------------------------------------------------
    //
    // The three methods above are total, and an implementation whose store can *fail* has nowhere to
    // put the failure: it reads as "no nodes"/"no items", which the export path then reports as an
    // empty history or as corruption (AUDIT C53's class in a consumer; U11). The oracle is
    // `F`-shaped — `RSpaceExporter.scala`'s `traverseHistory` returns `F[Vector[TrieNode]]` and its
    // `getFromHistory` is `Blake2b256Hash => F[Option[ByteVector]]` — so the port's counterpart is a
    // *checked* accessor beside the total one, the same shape as `RadixTree`'s
    // `load_node_from_store`/`load_node`.
    //
    // The defaults delegate to the total form, so every existing implementation (including
    // `casper`'s) keeps compiling and keeps its behaviour: only an implementation backed by a store
    // that can fail overrides them.

    /// [`TrieExporter::get_nodes`], with a store that could not be read reported rather than
    /// flattened into an empty traversal.
    fn try_get_nodes(
        &self,
        start_path: &[(KeyHash, Option<u8>)],
        skip: usize,
        take: usize,
    ) -> Result<Vec<TrieNode<KeyHash>>, String> {
        Ok(self.get_nodes(start_path, skip, take))
    }

    /// [`TrieExporter::get_history_items`], fallible.
    fn try_get_history_items<Value>(
        &self,
        keys: &[KeyHash],
        from_buffer: impl Fn(&[u8]) -> Value,
    ) -> Result<Vec<(KeyHash, Value)>, String> {
        Ok(self.get_history_items(keys, from_buffer))
    }

    /// [`TrieExporter::get_data_items`], fallible.
    fn try_get_data_items<Value>(
        &self,
        keys: &[KeyHash],
        from_buffer: impl Fn(&[u8]) -> Value,
    ) -> Result<Vec<(KeyHash, Value)>, String> {
        Ok(self.get_data_items(keys, from_buffer))
    }
}

/// Writes trie history/data items back (port of `TrieImporter`).
pub trait TrieImporter<KeyHash: Clone> {
    /// Set history values (branch nodes).
    fn set_history_items<Value>(
        &mut self,
        data: &[(KeyHash, Value)],
        to_buffer: impl Fn(&Value) -> Vec<u8>,
    );

    /// Set data values (leaf nodes).
    fn set_data_items<Value>(
        &mut self,
        data: &[(KeyHash, Value)],
        to_buffer: impl Fn(&Value) -> Vec<u8>,
    );

    /// Set the current root hash.
    fn set_root(&mut self, key: KeyHash);
}

/// Checks whether the state is empty (port of `StateManager`).
pub trait StateManager {
    fn is_empty(&self) -> bool;
}
