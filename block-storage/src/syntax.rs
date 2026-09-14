//! Block-storage syntax (port of `BlockStoreSyntax.scala`, `ApprovedStoreSyntax.scala`, and
//! `BlockDagStorageSyntax.scala`).

use std::collections::BTreeSet;

use rchain_models::block_hash::BlockHash;
use rchain_models::block_metadata::BlockMetadata;
use rchain_models::casper::protocol::casper_message::{BlockMessage, FinalizedFringe};
use rchain_shared::base16;

use crate::approved_store::{ApprovedStore, FINALIZED_FRINGE_KEY};
use crate::block_store::BlockStore;
use crate::dag::dag_storage::BlockDagStorage;
use crate::errors::{BlockDagInconsistencyError, BlockStoreInconsistencyError};

/// Truncated base16 for error messages (port of `PrettyPrinter.buildString(ByteString)`).
fn build_string(bytes: &[u8]) -> String {
    let hex = base16::encode(bytes);
    if hex.len() > 10 {
        format!("{}...", &hex[..10])
    } else {
        hex
    }
}

/// Get a block, raising an inconsistency error if it is absent (port of
/// `BlockStoreSyntax.getUnsafe`).
pub async fn get_block_unsafe(
    store: &BlockStore,
    hash: &BlockHash,
) -> Result<BlockMessage, BlockStoreInconsistencyError> {
    let msg = format!(
        "BlockStore is missing hash {}",
        build_string(hash.as_bytes())
    );
    store
        .get(&[*hash])
        .await
        .map_err(BlockStoreInconsistencyError)?
        .into_iter()
        .next()
        .flatten()
        .ok_or(BlockStoreInconsistencyError(msg))
}

/// Put a block into the store (port of `BlockStoreSyntax.put`).
pub async fn put_block(store: &BlockStore, block: BlockMessage) -> Result<(), String> {
    let hash = block.block_hash;
    store.put(&[(hash, block)]).await
}

/// Get the approved block (port of `ApprovedStoreSyntax.getApprovedBlock`).
pub async fn get_approved_block(store: &ApprovedStore) -> Result<Option<FinalizedFringe>, String> {
    let results = store.get(&[FINALIZED_FRINGE_KEY]).await?;
    Ok(results.into_iter().next().flatten())
}

/// Put the approved block (port of `ApprovedStoreSyntax.putApprovedBlock`).
pub async fn put_approved_block(
    store: &ApprovedStore,
    block: FinalizedFringe,
) -> Result<(), String> {
    store.put(&[(FINALIZED_FRINGE_KEY, block)]).await
}

/// Look up block metadata, raising an inconsistency error if absent (port of
/// `BlockDagStorageSyntax.lookupUnsafe`).
pub async fn lookup_unsafe(
    bds: &dyn BlockDagStorage,
    hash: &BlockHash,
) -> Result<BlockMetadata, BlockDagInconsistencyError> {
    let msg = format!(
        "DAG storage is missing hash {}",
        build_string(hash.as_bytes())
    );
    bds.lookup(hash)
        .await
        .map_err(BlockDagInconsistencyError)?
        .ok_or(BlockDagInconsistencyError(msg))
}

/// Insert a genesis block with its validated metadata (port of
/// `BlockDagStorageSyntax.insertGenesis`).
pub async fn insert_genesis(
    bds: &dyn BlockDagStorage,
    genesis_block: BlockMessage,
) -> Result<(), String> {
    let mut bmd = BlockMetadata::from_block(&genesis_block);
    bmd.validated = true;
    bmd.validation_failed = false;
    bmd.fringe = BTreeSet::new();
    bmd.fringe_state_hash = genesis_block.pre_state_hash;
    bds.insert(bmd, genesis_block).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use rchain_models::block_hash::BlockHash;
    use rchain_models::casper::protocol::casper_message::SignedDeployData;
    use rchain_shared::store_manager::InMemoryStoreManager;

    use crate::block_store;
    use crate::dag::dag_storage::DeployId;
    use crate::dag::representation::DagRepresentation;
    use crate::test_support::block;

    /// What `insert` was called with, so a test can assert on the metadata the syntax layer
    /// *derived* rather than only on what came back out of `lookup`.
    #[derive(Clone, Debug)]
    struct Inserted {
        metadata: BlockMetadata,
        block: BlockMessage,
    }

    /// A DAG storage over two maps — the smallest thing that can answer `lookup` and record what
    /// `insert` was given. The real implementation is casper's `BlockDagKeyValueStorage`, which this
    /// crate cannot depend on (it depends on this one), so the trait's contract is pinned here and
    /// its behaviour against a live store is pinned in casper's tests.
    #[derive(Default)]
    struct StubDagStorage {
        blocks: Mutex<BTreeMap<BlockHash, BlockMetadata>>,
        inserted: Mutex<Vec<Inserted>>,
    }

    #[async_trait::async_trait]
    impl BlockDagStorage for StubDagStorage {
        async fn get_representation(&self) -> DagRepresentation {
            unreachable!("the syntax layer never asks for a representation")
        }

        async fn insert(
            &self,
            block_metadata: BlockMetadata,
            block: BlockMessage,
        ) -> Result<(), String> {
            self.inserted.lock().unwrap().push(Inserted {
                metadata: block_metadata.clone(),
                block: block.clone(),
            });
            self.blocks
                .lock()
                .unwrap()
                .insert(block_metadata.block_hash, block_metadata);
            Ok(())
        }

        async fn lookup(&self, block_hash: &BlockHash) -> Result<Option<BlockMetadata>, String> {
            Ok(self.blocks.lock().unwrap().get(block_hash).cloned())
        }

        async fn lookup_by_deploy_id(
            &self,
            _deploy_id: &DeployId,
        ) -> Result<Option<BlockHash>, String> {
            Ok(None)
        }

        async fn add_deploy(&self, _deploy: SignedDeployData) -> Result<(), String> {
            Ok(())
        }

        async fn pooled_deploys(&self) -> Result<BTreeMap<DeployId, SignedDeployData>, String> {
            Ok(BTreeMap::new())
        }

        async fn contains_deploy_in_pool(&self, _deploy_id: &DeployId) -> Result<bool, String> {
            Ok(false)
        }
    }

    /// A hash whose hex is distinctive in its first ten characters, so an error message can be
    /// matched in full: the message truncates at ten (`PrettyPrinter.buildString`), so a message
    /// carrying all 64 characters would not match this string.
    fn hash_starting(prefix: [u8; 5]) -> BlockHash {
        let mut raw = [0u8; 32];
        raw[..5].copy_from_slice(&prefix);
        BlockHash::new(raw)
    }

    /// `get_block_unsafe` is the *unsafe* (in-consensus) accessor: a missing block is an error
    /// carrying the block hash, truncated the way the Scala `PrettyPrinter.buildString(ByteString)`
    /// does. A store that returned `None` as a block, or an error with no hash in it, would leave a
    /// consensus failure undiagnosable.
    #[tokio::test]
    async fn get_block_unsafe_names_the_missing_hash_in_its_error() {
        let kvm = InMemoryStoreManager::default();
        let store = block_store::create(&kvm).await.expect("store");

        let absent = hash_starting([0x11, 0x22, 0x33, 0x44, 0x55]);
        let err = get_block_unsafe(&store, &absent).await.expect_err("absent");
        assert_eq!(err.to_string(), "BlockStore is missing hash 1122334455...");

        // Present: the same accessor returns it, and `put_block` keyed it by its own hash.
        let b = block();
        put_block(&store, b.clone()).await.expect("put");
        assert_eq!(
            get_block_unsafe(&store, &b.block_hash)
                .await
                .expect("present"),
            b
        );

        // A *different* hash is still missing — the store did not answer with the one block it has.
        assert!(get_block_unsafe(&store, &absent).await.is_err());
    }

    /// The approved store holds one keyed item; an empty store answers `None` (there is no genesis
    /// fringe yet) rather than an error, which is what the startup path branches on.
    #[tokio::test]
    async fn the_approved_store_round_trips_and_starts_empty() {
        let kvm = InMemoryStoreManager::default();
        let store = crate::approved_store::create(&kvm).await.expect("store");

        assert_eq!(get_approved_block(&store).await.expect("empty"), None);

        let fringe = crate::test_support::fringe();
        put_approved_block(&store, fringe.clone())
            .await
            .expect("put");
        assert_eq!(
            get_approved_block(&store).await.expect("present"),
            Some(fringe)
        );

        // The key is the Scala's `approvedBlockKey = 42`, so the item is where a node that wrote it
        // under the Scala (or an earlier build of this port) can still find it.
        assert_eq!(FINALIZED_FRINGE_KEY, 42);
    }

    /// `lookup_unsafe` has the same shape as `get_block_unsafe` for the DAG: absent metadata is an
    /// error naming the hash, not a `None` leaking into consensus.
    #[tokio::test]
    async fn lookup_unsafe_names_the_missing_hash_in_its_error() {
        let storage = StubDagStorage::default();
        let absent = hash_starting([0xAA, 0xBB, 0xCC, 0xDD, 0xEE]);

        let err = lookup_unsafe(&storage, &absent).await.expect_err("absent");
        assert_eq!(err.to_string(), "DAG storage is missing hash aabbccddee...");

        let b = block();
        storage
            .insert(BlockMetadata::from_block(&b), b.clone())
            .await
            .expect("insert");
        let found = lookup_unsafe(&storage, &b.block_hash)
            .await
            .expect("present");
        assert_eq!(found.block_hash, b.block_hash);
    }

    /// `insert_genesis` does not store the block's own metadata: a genesis is **validated**, never
    /// validation-failed, has an empty fringe, and takes its fringe state hash from the block's
    /// *pre*-state hash (the fringe records the state a block was built on). Every one of the four is
    /// a consensus-relevant overwrite, and each is asserted on what `insert` actually received —
    /// against a starting metadata that disagrees on all four, so an overwrite that did not happen
    /// cannot pass by coincidence.
    #[tokio::test]
    async fn insert_genesis_stores_validated_metadata_with_the_pre_state_fringe_hash() {
        let b = block();
        let mut misleading = BlockMetadata::from_block(&b);
        misleading.validated = false;
        misleading.validation_failed = true;
        misleading.fringe = BTreeSet::from([BlockHash::new([21u8; 32])]);
        misleading.fringe_state_hash = b.post_state_hash;
        assert_ne!(
            misleading.fringe_state_hash, b.pre_state_hash,
            "the fixture's pre- and post-state hashes must differ, or the assertion below is vacuous"
        );

        let storage = StubDagStorage::default();
        insert_genesis(&storage, b.clone()).await.expect("insert");

        let inserted = storage.inserted.lock().unwrap();
        assert_eq!(inserted.len(), 1);
        let stored = &inserted[0].metadata;
        assert!(stored.validated, "a genesis is validated");
        assert!(!stored.validation_failed);
        assert!(
            stored.fringe.is_empty(),
            "a genesis has no fringe — nothing was finalized before it"
        );
        assert_eq!(
            stored.fringe_state_hash, b.pre_state_hash,
            "the fringe state hash is the *pre*-state hash, not the post-state one"
        );
        assert_eq!(inserted[0].block, b, "the block is stored unchanged");
        drop(inserted);

        // A plain `insert` of the same misleading metadata is not rewritten, so the assertions above
        // measure `insert_genesis` rather than the stub's storage.
        let plain = StubDagStorage::default();
        plain
            .insert(misleading.clone(), b.clone())
            .await
            .expect("insert");
        let recorded = plain.inserted.lock().unwrap();
        assert!(recorded[0].metadata.validation_failed, "not rewritten");
        assert_eq!(recorded[0].metadata.fringe.len(), 1);
        assert!(!recorded[0].metadata.validated);
    }
}
