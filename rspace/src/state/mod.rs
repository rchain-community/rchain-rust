//! Tuple-space state export/import (port of `rspace/.../state/`).
//!
//! Ported here are the data types, the pure algorithms (`RSpaceExporter.traverseHistory` over
//! `RadixTree.sequentialExport`, and `RSpaceImporter.validateStateItems`), the store-backed
//! instances (`RSpaceExporterStore`/`RSpaceImporterStore`/`RSpaceStateManagerImpl`), and the disk
//! exporter (`RSpaceExporterDisk.writeToDisk` in `exporters`). The foundational
//! `TrieExporter`/`TrieNode`/`TrieImporter`/`StateManager` abstractions live in `rchain_shared::state`.

use std::collections::BTreeMap;
use std::fmt;

use rchain_crypto::hash::blake2b256_hash::Blake2b256Hash;
use rchain_shared::state::{StateManager, TrieExporter, TrieImporter, TrieNode};

use crate::history::export::{sequential_export, ExportDataSettings};
use crate::history::key_segment::KeySegment;

pub mod exporters;
pub mod instances;

/// Export skip/take counters (port of `RSpaceExporter.Counter`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Counter {
    pub skip: usize,
    pub take: usize,
}

/// Raised when the history is empty (port of `RSpaceExporter.EmptyHistoryException`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmptyHistoryException;

impl fmt::Display for EmptyHistoryException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "EmptyHistoryException")
    }
}

impl std::error::Error for EmptyHistoryException {}

/// A state-validation failure (port of `RSpaceImporter.StateValidationError`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateValidationError(pub String);

impl fmt::Display for StateValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StateValidationError: {}", self.0)
    }
}

impl std::error::Error for StateValidationError {}

/// A chunk of exported items plus the path of the last item (port of
/// `RSpaceExporterItems.StoreItems`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoreItems<KeyHash, Value> {
    pub items: Vec<(KeyHash, Value)>,
    pub last_path: Vec<(KeyHash, Option<u8>)>,
}

/// Format a `(hash, index)` path for pretty printing (port of `RSpaceExporter.pathPretty`).
pub fn path_pretty(path: &(Blake2b256Hash, Option<u8>)) -> String {
    let (hash, idx) = path;
    let idx_str = match idx {
        None => "--".to_string(),
        Some(i) => format!("{:02x}", i & 0xff),
    };
    let hash_hex: String = hash
        .as_bytes()
        .iter()
        .take(4)
        .map(|b| format!("{:02x}", b))
        .collect();
    format!("{}:{}", idx_str, hash_hex)
}

/// Decode the last-exported prefix from its 5-hash encoding (port of `createLastPrefix`).
///
/// The input is a peer-supplied resume path, so malformed shapes are an `Err`, not a panic.
fn create_last_prefix(prefix_seq: &[Blake2b256Hash]) -> Result<Option<KeySegment>, String> {
    if prefix_seq.is_empty() {
        return Ok(None);
    }
    if prefix_seq.len() < 5 {
        return Err("Invalid path during export: expected 5 prefix hashes.".to_string());
    }
    let size_prefix = prefix_seq[0].as_bytes()[0] as usize;
    let mut prefix128 = Vec::with_capacity(128);
    for i in 0..4 {
        prefix128.extend_from_slice(prefix_seq[1 + i].as_bytes());
    }
    // The **checked constructor** is the boundary here, not a hand-written bound. A `KeySegment` is
    // at most 127 bytes — the Scala's own `require(bv.size <= 127)` on `KeySegment.apply`, and the
    // radix encoder's 7-bit size field (`radix_tree.rs:121`'s `second & 0x7F`) — and this port of
    // `KeySegment(prefix128.take(sizePrefix))` used to accept a peer-supplied 128: one byte over,
    // which the encoder then silently truncates to *zero*, dropping the whole prefix and
    // desynchronizing the restored tree. `get` bounds the slice too, so a size past `prefix128` is
    // refused rather than sliced out of range. Refused, not panicked, because that is this input's
    // documented stance ("the input is a peer-supplied resume path, so malformed shapes are an `Err`,
    // not a panic") — where the Scala reaches its `require` and throws.
    let Some(bytes) = prefix128.get(..size_prefix) else {
        return Err(format!(
            "Invalid path during export: prefix size {size_prefix} exceeds 127."
        ));
    };
    Ok(Some(
        KeySegment::try_from(bytes.to_vec())
            .map_err(|e| format!("Invalid path during export: {e}"))?,
    ))
}

/// Build leaf/history `TrieNode`s from their hashes (port of `constructNodes`).
fn construct_nodes(
    leaf_keys: Vec<Blake2b256Hash>,
    node_keys: Vec<Blake2b256Hash>,
) -> Vec<TrieNode<Blake2b256Hash>> {
    let mut out = Vec::with_capacity(leaf_keys.len() + node_keys.len());
    for k in leaf_keys {
        out.push(TrieNode {
            hash: k,
            is_leaf: true,
            path: Vec::new(),
        });
    }
    for k in node_keys {
        out.push(TrieNode {
            hash: k,
            is_leaf: false,
            path: Vec::new(),
        });
    }
    out
}

/// Encode the last-exported prefix as the 6-hash path used to resume export (port of
/// `constructLastPath`).
fn construct_last_path(
    last_prefix: &[u8],
    root_hash: Blake2b256Hash,
) -> Vec<(Blake2b256Hash, Option<u8>)> {
    let prefix_size = last_prefix.len();
    let mut size_array = [0u8; 32];
    size_array[0] = prefix_size as u8;
    let mut prefix128_array = [0u8; 128];
    prefix128_array[..prefix_size].copy_from_slice(last_prefix);

    vec![
        (root_hash, None),
        (Blake2b256Hash::from_byte_array(&size_array), None),
        (
            Blake2b256Hash::from_byte_array(&prefix128_array[0..32]),
            None,
        ),
        (
            Blake2b256Hash::from_byte_array(&prefix128_array[32..64]),
            None,
        ),
        (
            Blake2b256Hash::from_byte_array(&prefix128_array[64..96]),
            None,
        ),
        (
            Blake2b256Hash::from_byte_array(&prefix128_array[96..128]),
            None,
        ),
    ]
}

fn construct_last_node(
    last_hash: Blake2b256Hash,
    last_path: Vec<(Blake2b256Hash, Option<u8>)>,
) -> Vec<TrieNode<Blake2b256Hash>> {
    vec![TrieNode {
        hash: last_hash,
        is_leaf: false,
        path: last_path,
    }]
}

/// Walk the trie and convert the exported nodes into path-indexed `TrieNode`s (port of
/// `RSpaceExporter.traverseHistory`).
pub fn traverse_history(
    start_path: &[(Blake2b256Hash, Option<u8>)],
    skip: i32,
    take: i32,
    get_from_history: &dyn Fn(&Blake2b256Hash) -> Result<Option<Vec<u8>>, String>,
) -> Result<Vec<TrieNode<Blake2b256Hash>>, String> {
    let settings = ExportDataSettings {
        flag_node_prefixes: false,
        flag_node_keys: true,
        flag_node_values: false,
        flag_leaf_prefixes: false,
        flag_leaf_values: true,
    };

    if start_path.is_empty() {
        return Ok(Vec::new());
    }

    let path_seq: Vec<Blake2b256Hash> = start_path.iter().map(|(h, _)| *h).collect();
    let root_hash = path_seq[0];
    let last_prefix = create_last_prefix(&path_seq[1..])?;

    let (data, new_last_prefix_opt) = sequential_export(
        root_hash,
        last_prefix,
        skip,
        take,
        get_from_history,
        &settings,
    )?;

    let node_keys = data.node_keys.clone();
    let leaf_keys = data.leaf_values.clone();
    let mut nodes = construct_nodes(leaf_keys, node_keys.clone());
    if !nodes.is_empty() {
        nodes.pop();
    }

    let last_path = match new_last_prefix_opt {
        Some(prefix) => construct_last_path(prefix.as_bytes(), root_hash),
        None => Vec::new(),
    };
    let last_history_node = match node_keys.last() {
        Some(last_hash) => construct_last_node(*last_hash, last_path),
        None => Vec::new(),
    };

    nodes.extend(last_history_node);
    Ok(nodes)
}

/// Validate a chunk of exported history/data items against the trie (port of
/// `RSpaceImporter.validateStateItems`).
/// The oracle's `validateStateItems` takes `getFromHistory: KeyHash => F[Option[ByteVector]]`
/// (`RSpaceImporter.scala`), so "the target store could not be read" and "the node is absent" are
/// different inputs — and without the distinction a store error makes the traversal come up short and
/// the verdict names the peer's data: `"History items are corrupted."` (AUDIT C63). The reader is
/// therefore fallible here too.
pub fn validate_state_items(
    history_items: &[(Blake2b256Hash, Vec<u8>)],
    data_items: &[(Blake2b256Hash, Vec<u8>)],
    start_path: &[(Blake2b256Hash, Option<u8>)],
    chunk_size: i32,
    skip: i32,
    get_from_history: &dyn Fn(&Blake2b256Hash) -> Result<Option<Vec<u8>>, String>,
) -> Result<(), StateValidationError> {
    let received = history_items.len() as i32;
    let is_end = received < chunk_size;
    if !(received == chunk_size || is_end) {
        return Err(StateValidationError(format!(
            "Input size of history items is not valid. Expected chunk size {chunk_size}, received {received}."
        )));
    }

    // Validate tries from the received history items, building the received-node map.
    let mut trie_map: BTreeMap<Blake2b256Hash, Vec<u8>> = BTreeMap::new();
    for (hash, trie_bytes) in history_items {
        let trie_hash = Blake2b256Hash::create(trie_bytes);
        if hash != &trie_hash {
            return Err(StateValidationError(format!(
                "Trie hash does not match decoded trie, key: {}, decoded: {}.",
                hash.to_hex(),
                trie_hash.to_hex()
            )));
        }
        trie_map.insert(trie_hash, trie_bytes.clone());
    }

    let get_node = |hash: &Blake2b256Hash| -> Result<Option<Vec<u8>>, String> {
        match trie_map.get(hash) {
            Some(bytes) => Ok(Some(bytes.clone())),
            None => get_from_history(hash),
        }
    };

    let nodes =
        traverse_history(start_path, skip, chunk_size, &get_node).map_err(StateValidationError)?;

    let mut data_keys = Vec::new();
    let mut history_keys = Vec::new();
    for node in &nodes {
        if node.is_leaf {
            data_keys.push(node.hash);
        } else {
            history_keys.push(node.hash);
        }
    }

    let history_items_keys: Vec<Blake2b256Hash> = history_items.iter().map(|(h, _)| *h).collect();
    if history_items_keys != history_keys {
        return Err(StateValidationError(
            "History items are corrupted.".to_string(),
        ));
    }
    let data_items_keys: Vec<Blake2b256Hash> = data_items.iter().map(|(h, _)| *h).collect();
    if data_items_keys != data_keys {
        return Err(StateValidationError(
            "Data items are corrupted.".to_string(),
        ));
    }

    // Validate data (leaf) item hashes.
    for (hash, value_bytes) in data_items {
        let data_hash = Blake2b256Hash::create(value_bytes);
        if hash != &data_hash {
            return Err(StateValidationError(format!(
                "Data hash does not match decoded data, key: {}, decoded: {}.",
                hash.to_hex(),
                data_hash.to_hex()
            )));
        }
    }

    Ok(())
}

/// The rspace exporter (port of `RSpaceExporter`).
pub trait RSpaceExporter: TrieExporter<Blake2b256Hash> {
    /// The current root, if set (port of `getRoot`; `Ok(None)` is the `NoRootError` case).
    ///
    /// **Fallible, because the oracle is**: `def getRoot: F[KeyHash]` (`RSpaceExporter.scala:15`), so
    /// *absence* is `Ok(None)` and an unreadable roots store is `Err`. Flattening them made
    /// `StateManager::is_empty` answer "the state is empty" for a store that merely failed to answer
    /// (AUDIT C63).
    fn get_root(&self) -> Result<Option<Blake2b256Hash>, String>;
}

/// The rspace state manager (port of `RSpaceStateManager`).
pub trait RSpaceStateManager: StateManager {
    type Exporter: RSpaceExporter;
    type Importer: RSpaceImporter;

    fn exporter(&self) -> &Self::Exporter;
    fn importer(&self) -> &Self::Importer;
}

/// The rspace importer (port of `RSpaceImporter`).
pub trait RSpaceImporter: TrieImporter<Blake2b256Hash> {
    /// One history item, or `Ok(None)` for a hash that is genuinely absent — an unreadable store is
    /// an `Err`. The Scala's is in `F` (`RSpaceImporter.getHistoryItem`), so the port keeps the same
    /// distinction `TrieExporter::get_nodes` states (AUDIT C63).
    fn get_history_item(&self, hash: Blake2b256Hash) -> Result<Option<Vec<u8>>, String>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use crate::history::radix_tree::{empty_node, hash_node, Item};

    /// The resume path is peer-supplied and its `sizePrefix` is a *raw byte*: the segment invariant
    /// caps it at 127 (the encoder writes the size in 7 bits — `radix_tree.rs:121`'s `second & 0x7F` —
    /// so a 128-byte segment is silently truncated to *zero* on the way back out, dropping the prefix
    /// and desynchronizing the restored tree). The port of the Scala's `KeySegment(prefix128.take(sizePrefix))`
    /// bounded it at `> 128`, one byte over: a value the Scala's own `require(bv.size <= 127)`
    /// refuses, and that this port's documented stance refuses with an `Err` rather than a panic.
    ///
    /// Falsifier: with the `> 128` bound restored, this fails — a 128-byte segment comes back `Ok`.
    /// (It once also named putting the total `KeySegment::new` back in place of the checked
    /// constructor; that half is unreachable by construction now — see the type — since the total
    /// constructor no longer exists to put back.)
    #[test]
    fn a_128_byte_resume_prefix_is_refused() {
        let mut hashes = vec![Blake2b256Hash::from_bytes([0u8; 32]); 5];
        hashes[0] = Blake2b256Hash::from_bytes([128u8; 32]);
        assert!(
            create_last_prefix(&hashes).is_err(),
            "a prefix size of 128 is one byte over the segment invariant"
        );

        hashes[0] = Blake2b256Hash::from_bytes([127u8; 32]);
        let segment = create_last_prefix(&hashes)
            .expect("127 fits")
            .expect("a segment of one byte is still a segment");
        assert_eq!(segment.len(), 127);
    }

    fn single_leaf_store() -> (
        HashMap<Blake2b256Hash, Vec<u8>>,
        Blake2b256Hash,
        Blake2b256Hash,
    ) {
        let mut root = empty_node();
        let leaf_hash = Blake2b256Hash::from_bytes([0x42; 32]);
        root[0] = Item::Leaf {
            prefix: KeySegment::try_from(vec![1]).expect("1 byte is at most 127"),
            value: leaf_hash,
        };
        let (root_hash, root_bytes) = hash_node(&root);
        let store = HashMap::from([(root_hash, root_bytes)]);
        (store, root_hash, leaf_hash)
    }

    #[test]
    fn traverse_history_emits_leaf_and_last_node() {
        let (store, root_hash, leaf_hash) = single_leaf_store();
        let get = |h: &Blake2b256Hash| Ok(store.get(h).cloned());
        let nodes = traverse_history(&[(root_hash, None)], 0, 10, &get).unwrap();
        assert_eq!(nodes.len(), 2);
        assert!(nodes[0].is_leaf);
        assert_eq!(nodes[0].hash, leaf_hash);
        assert!(!nodes[1].is_leaf);
        assert_eq!(nodes[1].hash, root_hash);
    }

    /// A single-leaf trie whose leaf value is the hash of `data_value` (so the export/import
    /// round-trip can be validated end-to-end).
    fn leaf_trie(
        data_value: Vec<u8>,
    ) -> (
        HashMap<Blake2b256Hash, Vec<u8>>,
        Blake2b256Hash,
        Vec<u8>,
        Blake2b256Hash,
    ) {
        let data_hash = Blake2b256Hash::create(&data_value);
        let mut root = empty_node();
        root[0] = Item::Leaf {
            prefix: KeySegment::try_from(vec![1]).expect("1 byte is at most 127"),
            value: data_hash,
        };
        let (root_hash, root_bytes) = hash_node(&root);
        let store = HashMap::from([(root_hash, root_bytes.clone())]);
        (store, root_hash, root_bytes, data_hash)
    }

    #[test]
    fn validate_state_items_accepts_valid_round_trip() {
        let data_value = vec![1, 2, 3];
        let (store, root_hash, root_bytes, data_hash) = leaf_trie(data_value.clone());
        let get = |h: &Blake2b256Hash| Ok(store.get(h).cloned());

        let history_items = vec![(root_hash, root_bytes)];
        let data_items = vec![(data_hash, data_value)];
        validate_state_items(
            &history_items,
            &data_items,
            &[(root_hash, None)],
            10,
            0,
            &get,
        )
        .unwrap();
    }

    #[test]
    fn validate_state_items_rejects_corrupted_data() {
        let data_value = vec![1, 2, 3];
        let (store, root_hash, root_bytes, data_hash) = leaf_trie(data_value);
        let get = |h: &Blake2b256Hash| Ok(store.get(h).cloned());

        // The data bytes no longer hash to the claimed `data_hash`.
        let history_items = vec![(root_hash, root_bytes)];
        let data_items = vec![(data_hash, vec![9, 9, 9])];
        assert!(validate_state_items(
            &history_items,
            &data_items,
            &[(root_hash, None)],
            10,
            0,
            &get
        )
        .is_err());
    }
}
