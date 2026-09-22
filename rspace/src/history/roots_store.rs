//! Persists the current root hash under fixed keys.
//!
//! Mirrors `rspace/src/main/scala/coop/rchain/rspace/history/RootsStore.scala`.

use rchain_crypto::hash::blake2b256_hash::Blake2b256Hash;
use rchain_shared::typed_store::SharedStore;

const CURRENT_ROOT: &[u8] = b"current-root";
const ROOT_TAG: &[u8] = b"root";

/// The roots store (port of `RootsStore`).
pub struct RootsStore {
    store: SharedStore,
}

impl RootsStore {
    pub fn new(store: SharedStore) -> Self {
        RootsStore { store }
    }

    async fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, String> {
        let vals = self.store.lock().await.get(&[key.to_vec()])?;
        Ok(vals.into_iter().next().flatten())
    }

    async fn put(&self, key: Vec<u8>, value: Vec<u8>) -> Result<(), String> {
        self.store.lock().await.put(vec![(key, value)])
    }

    /// The current root, if set (port of `currentRoot`).
    pub async fn current_root(&self) -> Result<Option<Blake2b256Hash>, String> {
        Ok(self
            .get(CURRENT_ROOT)
            .await?
            .map(|b| Blake2b256Hash::from_byte_array(&b)))
    }

    /// Set the current root if `key` is a known root (port of `validateAndSetCurrentRoot`).
    pub async fn validate_and_set_current_root(
        &self,
        key: Blake2b256Hash,
    ) -> Result<Option<Blake2b256Hash>, String> {
        let bytes = key.to_byte_array().to_vec();
        if self.get(&bytes).await?.is_some() {
            self.put(CURRENT_ROOT.to_vec(), bytes).await?;
            Ok(Some(key))
        } else {
            Ok(None)
        }
    }

    /// Record `key` as a known root and set it as current (port of `recordRoot`).
    pub async fn record_root(&self, key: Blake2b256Hash) -> Result<(), String> {
        let bytes = key.to_byte_array().to_vec();
        self.put(bytes.clone(), ROOT_TAG.to_vec()).await?;
        self.put(CURRENT_ROOT.to_vec(), bytes).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rchain_shared::store::InMemoryKeyValueStore;
    use std::sync::Arc;

    fn store() -> RootsStore {
        RootsStore::new(Arc::new(tokio::sync::Mutex::new(
            Box::new(InMemoryKeyValueStore::default())
                as Box<dyn rchain_shared::store::KeyValueStore + Send + Sync>,
        )))
    }

    #[tokio::test]
    async fn current_root_is_none_before_anything_is_recorded() {
        assert_eq!(store().current_root().await.expect("read"), None);
    }

    /// The guard: a root the store has never seen is **refused**, and refusing it leaves the current
    /// root untouched. If this returned the root anyway, a node could adopt a root it cannot walk —
    /// a state pointing at history it does not have.
    #[tokio::test]
    async fn validate_and_set_refuses_an_unknown_root() {
        let roots = store();
        let unknown = Blake2b256Hash::from_bytes([0xAB; 32]);

        assert_eq!(
            roots
                .validate_and_set_current_root(unknown)
                .await
                .expect("read"),
            None,
            "an unknown root must not be adopted"
        );
        assert_eq!(
            roots.current_root().await.expect("read"),
            None,
            "a refused root must not become current"
        );
    }

    /// Recording a root makes it known *and* current, and afterwards the validated setter accepts it
    /// — the two paths agree on what "known" means.
    #[tokio::test]
    async fn a_recorded_root_round_trips_and_is_then_acceptable() {
        let roots = store();
        let root = Blake2b256Hash::from_bytes([0x11; 32]);

        roots.record_root(root).await.expect("record");
        assert_eq!(roots.current_root().await.expect("read"), Some(root));

        // Recording a *later* root must not forget the earlier one: the store keeps every recorded
        // root as known and moves only the current pointer, so a validation that walks back to a
        // previous root still succeeds.
        let later = Blake2b256Hash::from_bytes([0x22; 32]);
        roots.record_root(later).await.expect("record later");
        assert_eq!(roots.current_root().await.expect("read"), Some(later));

        assert_eq!(
            roots
                .validate_and_set_current_root(root)
                .await
                .expect("read"),
            Some(root),
            "the earlier root is still known"
        );
        assert_eq!(
            roots.current_root().await.expect("read"),
            Some(root),
            "and becomes current again"
        );
    }
}
