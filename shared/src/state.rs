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
    ///
    /// **Fallible, because the oracle is.** `RSpaceExporter.scala`'s `traverseHistory` returns
    /// `F[Vector[TrieNode]]` and its `getFromHistory` is `Blake2b256Hash => F[Option[ByteVector]]`,
    /// so "the store could not be read" and "there is no such node" are different answers. A total
    /// signature forced them into one, and the export path reported the result as an empty history or
    /// as corruption (AUDIT C63). `Ok(Vec::new())`/`Ok(None)` is the *absence* answer; `Err` is the
    /// store's.
    fn get_nodes(
        &self,
        start_path: &[(KeyHash, Option<u8>)],
        skip: usize,
        take: usize,
    ) -> Result<Vec<TrieNode<KeyHash>>, String>;

    /// Get history values (branch nodes) by key. Fallible for the same reason as
    /// [`TrieExporter::get_nodes`]; a key that is genuinely absent is simply not in the result.
    fn get_history_items<Value>(
        &self,
        keys: &[KeyHash],
        from_buffer: impl Fn(&[u8]) -> Value,
    ) -> Result<Vec<(KeyHash, Value)>, String>;

    /// Get data values (leaf nodes) by key. Fallible as above.
    fn get_data_items<Value>(
        &self,
        keys: &[KeyHash],
        from_buffer: impl Fn(&[u8]) -> Value,
    ) -> Result<Vec<(KeyHash, Value)>, String>;
}

/// Writes trie history/data items back (port of `TrieImporter`).
pub trait TrieImporter<KeyHash: Clone> {
    /// Set history values (branch nodes).
    ///
    /// **Fallible, because a refused write is not a completed import.** The settle was `()`: a store
    /// that refuses every write restored *less* state than it was handed and reported the same
    /// success a complete import reports — the wrong-state claim AUDIT C63's write half is about (the
    /// LFS sync is the path that lives on it).
    fn set_history_items<Value>(
        &mut self,
        data: &[(KeyHash, Value)],
        to_buffer: impl Fn(&Value) -> Vec<u8>,
    ) -> Result<(), String>;

    /// Set data values (leaf nodes). Fallible as above.
    fn set_data_items<Value>(
        &mut self,
        data: &[(KeyHash, Value)],
        to_buffer: impl Fn(&Value) -> Vec<u8>,
    ) -> Result<(), String>;

    /// Set the current root hash. Fallible as above.
    fn set_root(&mut self, key: KeyHash) -> Result<(), String>;
}

/// Checks whether the state is empty (port of `StateManager`).
pub trait StateManager {
    /// Whether the state is empty.
    ///
    /// Fallible: "there is no root" (an empty state) and "the roots store could not be read" were the
    /// same `false`/`true` answer, and a caller that acts on "the state is empty" would act on an
    /// unreadable store (AUDIT C63's residue).
    fn is_empty(&self) -> Result<bool, String>;
}
