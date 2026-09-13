//! Higher-level root commit/validation wrapper.
//!
//! Mirrors `rspace/src/main/scala/coop/rchain/rspace/history/RootRepository.scala`.

use rchain_crypto::hash::blake2b256_hash::Blake2b256Hash;

use crate::history::history::empty_root_hash_value;
use crate::history::roots_store::RootsStore;

/// The root repository (port of `RootRepository`).
pub struct RootRepository {
    roots_store: RootsStore,
}

impl RootRepository {
    pub fn new(roots_store: RootsStore) -> Self {
        RootRepository { roots_store }
    }

    pub async fn commit(&self, root: Blake2b256Hash) -> Result<(), String> {
        self.roots_store.record_root(root).await
    }

    /// The current root, recording the empty root on first use (port of `currentRoot`).
    pub async fn current_root(&self) -> Result<Blake2b256Hash, String> {
        match self.roots_store.current_root().await? {
            None => {
                let empty = empty_root_hash_value();
                self.roots_store.record_root(empty).await?;
                Ok(empty)
            }
            Some(root) => Ok(root),
        }
    }

    /// Validate `root` is known and set it current; error otherwise (port of
    /// `validateAndSetCurrentRoot`).
    pub async fn validate_and_set_current_root(&self, root: Blake2b256Hash) -> Result<(), String> {
        match self.roots_store.validate_and_set_current_root(root).await? {
            Some(_) => Ok(()),
            None => Err("unknown root".to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rchain_shared::store::InMemoryKeyValueStore;
    use std::sync::Arc;

    fn repository() -> RootRepository {
        let store: Box<dyn rchain_shared::store::KeyValueStore + Send + Sync> =
            Box::new(InMemoryKeyValueStore::default());
        RootRepository::new(RootsStore::new(Arc::new(tokio::sync::Mutex::new(store))))
    }

    /// A store with no root is not an error: the **empty root** is recorded and returned, so the
    /// first read establishes the genesis of the trie rather than failing. Pinned because the
    /// alternative (an error on an empty store) would be indistinguishable from a corrupt one.
    #[tokio::test]
    async fn an_empty_store_yields_the_empty_root_and_records_it() {
        let repo = repository();
        let empty = empty_root_hash_value();

        assert_eq!(repo.current_root().await.expect("root"), empty);
        // Now that it has been recorded, the validated setter accepts it — the lazily recorded root
        // is a *known* root, not a special case the setter has to make an exception for.
        assert_eq!(repo.current_root().await.expect("root"), empty);
        assert!(repo.validate_and_set_current_root(empty).await.is_ok());
    }

    /// The wrapper turns the store's refusal into an error rather than a silent no-op, and leaves the
    /// current root where it was — so a caller cannot believe it moved to a root that does not exist.
    #[tokio::test]
    async fn an_unknown_root_is_an_error_and_does_not_move_the_current_root() {
        let repo = repository();
        let known = rchain_crypto::hash::blake2b256_hash::Blake2b256Hash::from_bytes([0x11; 32]);
        repo.commit(known).await.expect("commit");

        let unknown = rchain_crypto::hash::blake2b256_hash::Blake2b256Hash::from_bytes([0xAB; 32]);
        let err = repo
            .validate_and_set_current_root(unknown)
            .await
            .expect_err("an unknown root must be an error");
        assert!(err.contains("unknown root"), "{err}");
        assert_eq!(
            repo.current_root().await.expect("root"),
            known,
            "a refused root must not become current"
        );
    }
}
