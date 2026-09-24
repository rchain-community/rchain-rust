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
    /// The trait's total arm, kept because `casper` implements that trait (its implementations read
    /// no store and cannot fail). It *delegates* to the checked sibling rather than re-implementing,
    /// so the two cannot drift, and a store error here reads as "no nodes" — which is why every
    /// consumer in this crate calls [`TrieExporter::try_get_nodes`].
    fn get_nodes(
        &self,
        start_path: &[(Blake2b256Hash, Option<u8>)],
        skip: usize,
        take: usize,
    ) -> Vec<TrieNode<Blake2b256Hash>> {
        self.try_get_nodes(start_path, skip, take)
            .unwrap_or_default()
    }

    fn try_get_nodes(
        &self,
        start_path: &[(Blake2b256Hash, Option<u8>)],
        skip: usize,
        take: usize,
    ) -> Result<Vec<TrieNode<Blake2b256Hash>>, String> {
        let history_store: &dyn KeyValueStore = self.history_store.as_ref();
        // `None` from the closure is the *absence* of a node, which the traversal reports as "node…
        // not found"; a store that cannot be read is a different fact and goes through the `Result`
        // (U11) — the flattening of the two was the defect.
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
    ) -> Vec<(Blake2b256Hash, Value)> {
        self.try_get_history_items(keys, from_buffer)
            .unwrap_or_default()
    }

    fn try_get_history_items<Value>(
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
    ) -> Vec<(Blake2b256Hash, Value)> {
        self.try_get_data_items(keys, from_buffer)
            .unwrap_or_default()
    }

    fn try_get_data_items<Value>(
        &self,
        keys: &[Blake2b256Hash],
        from_buffer: impl Fn(&[u8]) -> Value,
    ) -> Result<Vec<(Blake2b256Hash, Value)>, String> {
        get_items(self.value_store.as_ref(), keys, &from_buffer)
    }
}

impl RSpaceExporter for RSpaceExporterStore {
    /// The trait's total arm, as above: delegation to the checked sibling, whose error is where the
    /// port's consumers read. `None` here means "no root", which `StateManager::is_empty` reads as
    /// "the state is empty" — true for a fresh state, a lie for an unreadable store.
    fn get_root(&self) -> Option<Blake2b256Hash> {
        self.try_get_root().ok().flatten()
    }

    fn try_get_root(&self) -> Result<Option<Blake2b256Hash>, String> {
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

/// **The write half of this flatten is left total, with the reason** (U11).
///
/// The four `let _ = store.put(..)` below are the same defect as the reads — an import whose store
/// refuses a write silently restores *less* state than it was given, and reports success — but the
/// checked form (`try_set_*`) would have no caller in this crate: the consumers are
/// `casper/src/engine/lfs_tuple_space_requester.rs:270,328` and `node/src/runtime/node_runtime.rs:1984`,
/// and a checked setter nothing calls enforces nothing. The durable fix is the trait's own signature
/// — the oracle's `TrieImporter` setters are `F`-shaped — which needs those consumers converted with
/// it. Named in AUDIT C53/U11's rows rather than half-done here.
impl TrieImporter<Blake2b256Hash> for RSpaceImporterStore {
    fn set_history_items<Value>(
        &mut self,
        data: &[(Blake2b256Hash, Value)],
        to_buffer: impl Fn(&Value) -> Vec<u8>,
    ) {
        let pairs: Vec<(Vec<u8>, Vec<u8>)> = data
            .iter()
            .map(|(h, v)| (hash_bytes(h), to_buffer(v)))
            .collect();
        let _ = self.history_store.put(pairs);
    }

    fn set_data_items<Value>(
        &mut self,
        data: &[(Blake2b256Hash, Value)],
        to_buffer: impl Fn(&Value) -> Vec<u8>,
    ) {
        let pairs: Vec<(Vec<u8>, Vec<u8>)> = data
            .iter()
            .map(|(h, v)| (hash_bytes(h), to_buffer(v)))
            .collect();
        let _ = self.value_store.put(pairs);
    }

    fn set_root(&mut self, key: Blake2b256Hash) {
        let bytes = hash_bytes(&key);
        let _ = self
            .roots_store
            .put(vec![(bytes.clone(), ROOT_TAG.to_vec())]);
        let _ = self.roots_store.put(vec![(CURRENT_ROOT.to_vec(), bytes)]);
    }
}

impl RSpaceImporter for RSpaceImporterStore {
    /// **Left total, with the reason** (U11): this reads the history store through
    /// `unwrap_or_default()`, so an unreadable store reads as "no such item". Its only consumers are
    /// outside this crate — `casper`'s lfs importer passes it straight into `validate_state_items`
    /// (`lfs_tuple_space_requester.rs:262`) and `node`'s test reads it back — so a checked sibling
    /// would have no caller here, and its durable fix is the trait's signature
    /// (`RSpaceImporter.getHistoryItem: F[Option[ByteVector]]` in the oracle). Named in AUDIT U11's
    /// row; the *read* paths this crate does own are converted (`try_get_history_items`,
    /// `try_get_nodes`, `try_get_root`).
    fn get_history_item(&self, hash: Blake2b256Hash) -> Option<Vec<u8>> {
        self.history_store
            .get(&[hash_bytes(&hash)])
            .unwrap_or_default()
            .into_iter()
            .next()
            .flatten()
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
    /// **Left total, with the reason** (U11): `StateManager::is_empty -> bool` is the shared trait's
    /// signature and has no channel, so an unreadable roots store reads as "the state is empty" here
    /// — the one place the U11 flatten is still reachable through this crate's own API. The checked
    /// form is on the exporter ([`RSpaceExporter::try_get_root`]); a consumer that can report should
    /// call that. The durable fix is the trait's signature, which lives in `rchain_shared` and is
    /// consumed outside this crate.
    fn is_empty(&self) -> bool {
        self.exporter.get_root().is_none()
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
    /// Falsifier in its pre-fix (witnessing) form, because the fix changes the setters' signatures:
    /// all four writes are refused by the store, and the three calls below return `()` — the same
    /// answer a successful import gives, so no caller can tell the difference.
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
        importer.set_history_items(&items, |v: &Vec<u8>| v.clone());
        importer.set_data_items(&items, |v: &Vec<u8>| v.clone());
        importer.set_root(Blake2b256Hash::from_bytes([0x22; 32]));

        // THE WITNESS: four writes attempted, four refused, and `()` returned by all three calls —
        // the import reports exactly what a successful one reports.
        assert_eq!(
            attempts.load(std::sync::atomic::Ordering::SeqCst),
            4,
            "the writes were attempted (and refused): the success claim is the defect"
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
    /// Falsifier in its pre-fix (witnessing) form: the empty traversal is asserted as the *defect* —
    /// it passes, which is the point — and the post-fix form asserts the refused read.
    #[test]
    fn a_store_error_is_not_an_empty_traversal() {
        let exporter = RSpaceExporterStore::new(
            Box::new(FailingStore),
            Box::new(FailingStore),
            Box::new(FailingStore),
        );
        // Pre-fix this asserted the *defect* — `get_nodes` answered an empty traversal and `get_root`
        // answered `None`, both passing on the bug. The checked forms are what the port's consumers
        // read now:
        let err = exporter
            .try_get_nodes(&[(Blake2b256Hash::from_bytes([0x11; 32]), None)], 0, 10)
            .expect_err("a store that is down must not read as an empty traversal");
        assert!(
            err.contains("the store is down"),
            "and it must name the failure, got: {err}"
        );
        let err = exporter.try_get_root().expect_err("…nor as an absent root");
        assert!(
            err.contains("the store is down"),
            "and it must name the failure, got: {err}"
        );

        // The *total* forms stay the trait's arm for implementations that read no store and cannot
        // fail (`casper`'s mocks). A store error there still reads as "nothing" — which is exactly why
        // nothing in this crate calls them any more.
        assert!(exporter
            .get_nodes(&[(Blake2b256Hash::from_bytes([0x11; 32]), None)], 0, 10)
            .is_empty());
        assert_eq!(exporter.get_root(), None);
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
        assert_eq!(exporter.get_root(), Some(root));
    }

    #[test]
    fn state_manager_empty_when_no_root() {
        let exporter = RSpaceExporterStore::new(store(), store(), store());
        let importer = RSpaceImporterStore::new(store(), store(), store());
        let manager = RSpaceStateManagerImpl::new(exporter, importer);
        assert!(manager.is_empty());
    }

    #[test]
    fn importer_sets_root() {
        let mut importer = RSpaceImporterStore::new(store(), store(), store());
        let root = Blake2b256Hash::from_bytes([0x22; 32]);
        importer.set_root(root);
        assert_eq!(importer.get_history_item(root), None);
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

        assert_eq!(exporter.get_root(), Some(root));
        let exported: Vec<(Blake2b256Hash, Vec<u8>)> =
            exporter.get_history_items(&hashes, |bytes: &[u8]| bytes.to_vec());
        assert_eq!(exported.len(), items.len(), "every item was exported");

        // Import the exported set into fresh stores, then compare item for item and the root.
        let mut importer = RSpaceImporterStore::new(store(), store(), store());
        // `get_history_items` already returns key/value pairs, so the exported set is what is
        // imported — no re-association, which is where a round-trip test could quietly drift.
        importer.set_history_items(&exported, |v: &Vec<u8>| v.clone());
        importer.set_root(root);

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
                importer.get_history_item(*hash).as_ref(),
                Some(value),
                "item {hash:?} must survive the round trip"
            );
        }
        // An item that was never exported is absent rather than fabricated.
        assert_eq!(
            importer.get_history_item(Blake2b256Hash::from_bytes([0x7F; 32])),
            None
        );
    }
}
