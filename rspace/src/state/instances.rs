//! Store-backed exporter/importer/state-manager instances (port of
//! `RSpaceExporterStore`/`RSpaceImporterStore`/`RSpaceStateManagerImpl`).

use rchain_crypto::hash::blake2b256_hash::Blake2b256Hash;
use rchain_shared::state::{StateManager, TrieExporter, TrieImporter, TrieNode};
use rchain_shared::store::KeyValueStore;

use crate::state::{traverse_history, RSpaceExporter, RSpaceImporter, RSpaceStateManager};

const CURRENT_ROOT: &[u8] = b"current-root";
const ROOT_TAG: &[u8] = b"root";

fn hash_bytes(h: &Blake2b256Hash) -> Vec<u8> {
    h.to_byte_array().to_vec()
}

fn get_items<Value>(
    store: &dyn KeyValueStore,
    keys: &[Blake2b256Hash],
    from_buffer: &dyn Fn(&[u8]) -> Value,
) -> Result<Vec<(Blake2b256Hash, Value)>, String> {
    let key_bytes: Vec<Vec<u8>> = keys.iter().map(hash_bytes).collect();
    // The store's error is *kept*: `unwrap_or_default()` read it as "no items", so an export of a
    // state whose store is unreadable came back short — which the caller then reported as an empty
    // history or as corruption (AUDIT C53's class in a consumer; U11). A key that is genuinely
    // absent is still simply skipped, which is what `zip`/`filter_map` say.
    let loaded = store.get(&key_bytes)?;
    Ok(keys
        .iter()
        .zip(loaded)
        .filter_map(|(h, v)| v.map(|b| (*h, from_buffer(&b))))
        .collect())
}

/// An exporter over three byte stores: history (branch nodes), value (leaf nodes), roots (port of
/// `RSpaceExporterStore`).
pub struct RSpaceExporterStore {
    history_store: Box<dyn KeyValueStore + Send + Sync>,
    value_store: Box<dyn KeyValueStore + Send + Sync>,
    roots_store: Box<dyn KeyValueStore + Send + Sync>,
}

impl RSpaceExporterStore {
    pub fn new(
        history_store: Box<dyn KeyValueStore + Send + Sync>,
        value_store: Box<dyn KeyValueStore + Send + Sync>,
        roots_store: Box<dyn KeyValueStore + Send + Sync>,
    ) -> Self {
        RSpaceExporterStore {
            history_store,
            value_store,
            roots_store,
        }
    }
}

impl TrieExporter<Blake2b256Hash> for RSpaceExporterStore {
    fn get_nodes(
        &self,
        start_path: &[(Blake2b256Hash, Option<u8>)],
        skip: usize,
        take: usize,
    ) -> Result<Vec<TrieNode<Blake2b256Hash>>, String> {
        let history_store: &dyn KeyValueStore = self.history_store.as_ref();
        // `Ok(None)` from the closure is the *absence* of a node, which the traversal reports as
        // "node … not found"; a store that cannot be read is a different fact and goes through the
        // `Err` (AUDIT C63) — the flattening of the two was the defect.
        let get_node = |h: &Blake2b256Hash| -> Result<Option<Vec<u8>>, String> {
            Ok(history_store
                .get(&[hash_bytes(h)])?
                .into_iter()
                .next()
                .flatten())
        };
        traverse_history(start_path, skip as i32, take as i32, &get_node)
    }

    fn get_history_items<Value>(
        &self,
        keys: &[Blake2b256Hash],
        from_buffer: impl Fn(&[u8]) -> Value,
    ) -> Result<Vec<(Blake2b256Hash, Value)>, String> {
        get_items(self.history_store.as_ref(), keys, &from_buffer)
    }

    fn get_data_items<Value>(
        &self,
        keys: &[Blake2b256Hash],
        from_buffer: impl Fn(&[u8]) -> Value,
    ) -> Result<Vec<(Blake2b256Hash, Value)>, String> {
        get_items(self.value_store.as_ref(), keys, &from_buffer)
    }
}

impl RSpaceExporter for RSpaceExporterStore {
    /// `Ok(None)` is "no root" (a fresh state); an unreadable roots store is an `Err`, which is what
    /// `StateManager::is_empty` needs in order not to answer "the state is empty" for it.
    fn get_root(&self) -> Result<Option<Blake2b256Hash>, String> {
        Ok(self
            .roots_store
            .get(&[CURRENT_ROOT.to_vec()])?
            .into_iter()
            .next()
            .flatten()
            .map(|b| Blake2b256Hash::from_byte_array(&b)))
    }
}

/// An importer over three byte stores (port of `RSpaceImporterStore`).
pub struct RSpaceImporterStore {
    history_store: Box<dyn KeyValueStore + Send + Sync>,
    value_store: Box<dyn KeyValueStore + Send + Sync>,
    roots_store: Box<dyn KeyValueStore + Send + Sync>,
}

impl RSpaceImporterStore {
    pub fn new(
        history_store: Box<dyn KeyValueStore + Send + Sync>,
        value_store: Box<dyn KeyValueStore + Send + Sync>,
        roots_store: Box<dyn KeyValueStore + Send + Sync>,
    ) -> Self {
        RSpaceImporterStore {
            history_store,
            value_store,
            roots_store,
        }
    }
}

impl TrieImporter<Blake2b256Hash> for RSpaceImporterStore {
    fn set_history_items<Value>(
        &mut self,
        data: &[(Blake2b256Hash, Value)],
        to_buffer: impl Fn(&Value) -> Vec<u8>,
    ) -> Result<(), String> {
        let pairs: Vec<(Vec<u8>, Vec<u8>)> = data
            .iter()
            .map(|(h, v)| (hash_bytes(h), to_buffer(v)))
            .collect();
        // A refused write is not a completed import: `let _ = put(..)` reported the same `()` a
        // successful import reports, so the LFS sync marked a chunk done over a root whose nodes were
        // never written (AUDIT C63's write half).
        self.history_store.put(pairs)
    }

    fn set_data_items<Value>(
        &mut self,
        data: &[(Blake2b256Hash, Value)],
        to_buffer: impl Fn(&Value) -> Vec<u8>,
    ) -> Result<(), String> {
        let pairs: Vec<(Vec<u8>, Vec<u8>)> = data
            .iter()
            .map(|(h, v)| (hash_bytes(h), to_buffer(v)))
            .collect();
        self.value_store.put(pairs)
    }

    fn set_root(&mut self, key: Blake2b256Hash) -> Result<(), String> {
        let bytes = hash_bytes(&key);
        self.roots_store
            .put(vec![(bytes.clone(), ROOT_TAG.to_vec())])?;
        self.roots_store.put(vec![(CURRENT_ROOT.to_vec(), bytes)])
    }
}

impl RSpaceImporter for RSpaceImporterStore {
    /// `Ok(None)` is a hash that is genuinely absent; an unreadable store is an `Err` (AUDIT C63).
    fn get_history_item(&self, hash: Blake2b256Hash) -> Result<Option<Vec<u8>>, String> {
        Ok(self
            .history_store
            .get(&[hash_bytes(&hash)])?
            .into_iter()
            .next()
            .flatten())
    }
}

/// The state manager pairing an exporter and importer (port of `RSpaceStateManagerImpl`).
pub struct RSpaceStateManagerImpl<E: RSpaceExporter, I: RSpaceImporter> {
    exporter: E,
    importer: I,
}

impl<E: RSpaceExporter, I: RSpaceImporter> RSpaceStateManagerImpl<E, I> {
    pub fn new(exporter: E, importer: I) -> Self {
        RSpaceStateManagerImpl { exporter, importer }
    }
}

impl<E: RSpaceExporter, I: RSpaceImporter> StateManager for RSpaceStateManagerImpl<E, I> {
    /// An unreadable roots store is an `Err` rather than "the state is empty" (AUDIT C63's residue).
    fn is_empty(&self) -> Result<bool, String> {
        Ok(self.exporter.get_root()?.is_none())
    }
}

impl<E: RSpaceExporter, I: RSpaceImporter> RSpaceStateManager for RSpaceStateManagerImpl<E, I> {
    type Exporter = E;
    type Importer = I;

    fn exporter(&self) -> &E {
        &self.exporter
    }

    fn importer(&self) -> &I {
        &self.importer
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rchain_shared::store::InMemoryKeyValueStore;

    /// A `KeyValueStore` whose every operation fails, so a store error is *observable* as one.
    struct FailingStore;

    impl KeyValueStore for FailingStore {
        fn get(&self, _keys: &[Vec<u8>]) -> Result<Vec<Option<Vec<u8>>>, String> {
            Err("the store is down".to_string())
        }
        fn put(&mut self, _pairs: Vec<(Vec<u8>, Vec<u8>)>) -> Result<(), String> {
            Err("the store is down".to_string())
        }
        fn delete(&mut self, _keys: &[Vec<u8>]) -> Result<usize, String> {
            Err("the store is down".to_string())
        }
        fn entries(&self) -> Result<Vec<(Vec<u8>, Vec<u8>)>, String> {
            Err("the store is down".to_string())
        }
    }

    /// A store that refuses every write and counts the attempts, so an importer's success claim can be
    /// checked against what the store was actually asked to do.
    struct RefusingStore {
        attempts: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    impl KeyValueStore for RefusingStore {
        fn get(&self, _keys: &[Vec<u8>]) -> Result<Vec<Option<Vec<u8>>>, String> {
            Err("the store is down".to_string())
        }
        fn put(&mut self, _pairs: Vec<(Vec<u8>, Vec<u8>)>) -> Result<(), String> {
            self.attempts
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err("the store refuses writes".to_string())
        }
        fn delete(&mut self, _keys: &[Vec<u8>]) -> Result<usize, String> {
            Err("the store is down".to_string())
        }
        fn entries(&self) -> Result<Vec<(Vec<u8>, Vec<u8>)>, String> {
            Err("the store is down".to_string())
        }
    }

    /// **An import that cannot write is not a successful import** — U12's write half, and the worse
    /// half of AUDIT C63: an import that silently restores *less* state than it was given is a
    /// wrong-state claim, not a missing read. `RSpaceImporterStore`'s setters dropped the store's
    /// result (`let _ = store.put(..)`), so the LFS sync marks a chunk done, resumes from a root
    /// whose nodes were never written, and reports success.
    ///
    /// Falsifier, both forms. Pre-fix (witnessing, U12): all four writes were refused by the store
    /// and the three calls returned `()` — the same answer a successful import gives, so no caller
    /// could tell the difference, and the assertion below passed **on that defect**. Post-fix: each
    /// call is an `Err`, which is what the LFS sync's `?` now reads.
    #[test]
    fn an_import_that_cannot_write_is_not_a_successful_import() {
        let attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut importer = RSpaceImporterStore::new(
            Box::new(RefusingStore {
                attempts: attempts.clone(),
            }),
            Box::new(RefusingStore {
                attempts: attempts.clone(),
            }),
            Box::new(RefusingStore {
                attempts: attempts.clone(),
            }),
        );
        let items = vec![(Blake2b256Hash::from_bytes([0x11; 32]), vec![1u8, 2, 3])];
        for (what, result) in [
            (
                "history items",
                importer.set_history_items(&items, |v: &Vec<u8>| v.clone()),
            ),
            (
                "data items",
                importer.set_data_items(&items, |v: &Vec<u8>| v.clone()),
            ),
            (
                "root",
                importer.set_root(Blake2b256Hash::from_bytes([0x22; 32])),
            ),
        ] {
            let err = result.expect_err("a refused write must not read as a successful import");
            assert!(
                err.contains("the store refuses writes"),
                "the {what} write names its failure, got: {err}"
            );
        }
        // Three attempts, not four: `set_root`'s first write is refused, so its second (recording
        // `current-root`) is never reached — which is the point of the `?` there.
        assert_eq!(
            attempts.load(std::sync::atomic::Ordering::SeqCst),
            3,
            "every attempted write was refused, and `set_root` stopped at its first refusal"
        );
    }

    /// **A store error is not an empty traversal, and not an empty state.**
    ///
    /// `RSpaceExporterStore` read its stores through `unwrap_or_default()`, so a store that is down
    /// produced **no nodes** and **no root** — the export path then reports `EmptyHistoryException`
    /// for a state that exists, and `StateManager::is_empty` (`:180`) answers "yes" for it. The
    /// oracle's exporter is `F`-shaped (`RSpaceExporter.scala:15`, `def getRoot: F[KeyHash]`;
    /// `traverseHistory` is `F[Vector[TrieNode]]` at `:29`), so this is AUDIT C53's class in a
    /// *consumer* rather than in the radix tree (U11).
    ///
    /// Falsifier, both forms. Pre-fix (witnessing, U11): `get_nodes` answered an *empty traversal*
    /// and `get_root` answered `None`, and the assertions passed **on that defect**. Post-fix (U12,
    /// the outright conversion): the same two calls are `Err`, and the total arms no longer exist for
    /// a store error to hide in.
    #[test]
    fn a_store_error_is_not_an_empty_traversal() {
        let exporter = RSpaceExporterStore::new(
            Box::new(FailingStore),
            Box::new(FailingStore),
            Box::new(FailingStore),
        );
        let err = exporter
            .get_nodes(&[(Blake2b256Hash::from_bytes([0x11; 32]), None)], 0, 10)
            .expect_err("a store that is down must not read as an empty traversal");
        assert!(
            err.contains("the store is down"),
            "and it must name the failure, got: {err}"
        );
        let err = exporter.get_root().expect_err("…nor as an absent root");
        assert!(
            err.contains("the store is down"),
            "and it must name the failure, got: {err}"
        );
    }

    fn store() -> Box<dyn KeyValueStore + Send + Sync> {
        Box::new(InMemoryKeyValueStore::default())
    }

    #[test]
    fn exporter_reads_root_and_items() {
        let mut roots = InMemoryKeyValueStore::default();
        let root = Blake2b256Hash::from_bytes([0x11; 32]);
        roots
            .put(vec![(CURRENT_ROOT.to_vec(), root.to_byte_array().to_vec())])
            .unwrap();

        let exporter = RSpaceExporterStore::new(store(), store(), Box::new(roots));
        assert_eq!(exporter.get_root().expect("an in-memory store"), Some(root));
    }

    #[test]
    fn state_manager_empty_when_no_root() {
        let exporter = RSpaceExporterStore::new(store(), store(), store());
        let importer = RSpaceImporterStore::new(store(), store(), store());
        let manager = RSpaceStateManagerImpl::new(exporter, importer);
        assert!(manager.is_empty().expect("an in-memory store"));
    }

    #[test]
    fn importer_sets_root() {
        let mut importer = RSpaceImporterStore::new(store(), store(), store());
        let root = Blake2b256Hash::from_bytes([0x22; 32]);
        importer.set_root(root).expect("an in-memory store");
        assert_eq!(
            importer.get_history_item(root).expect("an in-memory store"),
            None
        );
        // root is recorded under "current-root", not as a history item
        let roots = &*importer.roots_store;
        let current = roots
            .get(&[CURRENT_ROOT.to_vec()])
            .unwrap()
            .into_iter()
            .next()
            .flatten()
            .unwrap();
        assert_eq!(current, root.to_byte_array().to_vec());
    }

    /// A **populated** store exports and imports: every history item and the root survive a round
    /// trip through the real `RSpaceExporterStore`/`RSpaceImporterStore` rather than a mock leaf. The
    /// register's G5 deferred exactly this ("full store export→import→compare"); the tests here
    /// previously covered only a single leaf and the empty case.
    #[test]
    fn a_populated_store_export_import_round_trips() {
        let items: Vec<(Blake2b256Hash, Vec<u8>)> = (0u8..5)
            .map(|i| (Blake2b256Hash::from_bytes([i; 32]), vec![i, i + 1, i + 2]))
            .collect();
        let hashes: Vec<Blake2b256Hash> = items.iter().map(|(h, _)| *h).collect();
        let root = Blake2b256Hash::from_bytes([0xEE; 32]);

        // Source stores, as the on-chain history would hold them.
        let mut history = InMemoryKeyValueStore::default();
        history
            .put(
                items
                    .iter()
                    .map(|(h, v)| (h.to_byte_array().to_vec(), v.clone()))
                    .collect(),
            )
            .unwrap();
        let mut roots = InMemoryKeyValueStore::default();
        roots
            .put(vec![(CURRENT_ROOT.to_vec(), root.to_byte_array().to_vec())])
            .unwrap();
        let exporter = RSpaceExporterStore::new(Box::new(history), store(), Box::new(roots));

        assert_eq!(exporter.get_root().expect("in-memory store"), Some(root));
        let exported: Vec<(Blake2b256Hash, Vec<u8>)> = exporter
            .get_history_items(&hashes, |bytes: &[u8]| bytes.to_vec())
            .expect("in-memory store");
        assert_eq!(exported.len(), items.len(), "every item was exported");

        // Import the exported set into fresh stores, then compare item for item and the root.
        let mut importer = RSpaceImporterStore::new(store(), store(), store());
        // `get_history_items` already returns key/value pairs, so the exported set is what is
        // imported — no re-association, which is where a round-trip test could quietly drift.
        importer
            .set_history_items(&exported, |v: &Vec<u8>| v.clone())
            .expect("in-memory store");
        importer.set_root(root).expect("in-memory store");

        // The importer records the root under `current-root` (it has no `get_root` of its own), so
        // read it back the way the store holds it.
        let recorded = importer
            .roots_store
            .get(&[CURRENT_ROOT.to_vec()])
            .expect("roots read")
            .into_iter()
            .next()
            .flatten()
            .expect("a root was recorded");
        assert_eq!(recorded, root.to_byte_array().to_vec());
        for (hash, value) in &items {
            assert_eq!(
                importer
                    .get_history_item(*hash)
                    .expect("in-memory store")
                    .as_ref(),
                Some(value),
                "item {hash:?} must survive the round trip"
            );
        }
        // An item that was never exported is absent rather than fabricated.
        assert_eq!(
            importer
                .get_history_item(Blake2b256Hash::from_bytes([0x7F; 32]))
                .expect("in-memory store"),
            None
        );
    }
}
