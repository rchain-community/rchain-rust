//! Block DAG storage interface.
//!
//! Mirrors `block-storage/src/main/scala/coop/rchain/blockstorage/dag/BlockDagStorage.scala`. The
//! concrete `BlockDagKeyValueStorage` is casper-owned in Scala, so only the trait is ported here.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;

use rchain_models::block_hash::BlockHash;
use rchain_models::block_metadata::BlockMetadata;
use rchain_models::casper::protocol::casper_message::{BlockMessage, SignedDeployData};

use super::representation::DagRepresentation;

/// A deploy id (the Scala `BlockDagStorage.DeployId = ByteString`).
pub type DeployId = Vec<u8>;

/// The block DAG storage interface (port of `BlockDagStorage[F]`). The concrete implementation is
/// the `casper` crate's `BlockDagKeyValueStorage`.
#[async_trait]
pub trait BlockDagStorage: Send + Sync {
    /// The current DAG representation, **shared rather than copied**.
    ///
    /// A `DagRepresentation` owns every message, and each message's `seen` is its whole ancestry, so
    /// it is Θ(N²) in the number of blocks. Returning it by value made every caller pay that copy —
    /// ~35 call sites, including every per-block path and every peer-request handler, with the read
    /// lock held for the whole copy; on a node serving a syncing peer that was the difference between
    /// a flat curve and ~0.86 GiB per block. Callers read fields through `Deref`, so the change is
    /// invisible to them; the writer takes the copy at most once per insert via `Arc::make_mut`.
    async fn get_representation(&self) -> Arc<DagRepresentation>;

    async fn insert(
        &self,
        block_metadata: BlockMetadata,
        block: BlockMessage,
    ) -> Result<(), String>;

    async fn lookup(&self, block_hash: &BlockHash) -> Result<Option<BlockMetadata>, String>;

    /// Look up a block hash by the deploy id included in the DAG.
    async fn lookup_by_deploy_id(&self, deploy_id: &DeployId) -> Result<Option<BlockHash>, String>;

    /// Add a deploy to the (unprocessed) deploy pool.
    async fn add_deploy(&self, deploy: SignedDeployData) -> Result<(), String>;

    async fn pooled_deploys(&self) -> Result<BTreeMap<DeployId, SignedDeployData>, String>;

    async fn contains_deploy_in_pool(&self, deploy_id: &DeployId) -> Result<bool, String>;
}
