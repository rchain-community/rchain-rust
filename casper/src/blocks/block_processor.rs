//! Block processing (port of `blocks/BlockProcessor.scala`).

use std::sync::Arc;

use rchain_block_storage::block_store::BlockStore;
use rchain_block_storage::dag::dag_storage::BlockDagStorage;
use rchain_models::block_hash::BlockHash;
use rchain_models::casper::protocol::casper_message::BlockMessage;
use rchain_shared::log::{Log, LogSource};
use tokio::sync::mpsc;

use crate::block_status::BlockStatus;
use crate::merging::BlockIndex;
use crate::multi_parent_casper::ValidateError;
use crate::protocol::comm_util::CommUtil;
use crate::runtime_manager::RuntimeManager;

/// Cap on the number of blocks validated concurrently (per batch). The receiver only emits
/// dependency-free blocks, so a batch of siblings is mutually independent and replayable in parallel.
fn max_parallel_block_validation() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
}

/// Validate a block and insert it into the DAG (port of `validateAndAddToDag`).
pub async fn validate_and_add_to_dag<F, Fut>(
    dag: &dyn BlockDagStorage,
    block_store: &BlockStore,
    runtime: &RuntimeManager,
    block: BlockMessage,
    shard_id: &str,
    min_phlo_price: i64,
    block_index: &F,
) -> Result<Result<(), BlockStatus>, String>
where
    F: Fn(BlockHash) -> Fut,
    Fut: std::future::Future<Output = Result<BlockIndex, String>>,
{
    let result = crate::multi_parent_casper::validate(
        dag,
        block_store,
        runtime,
        &block,
        shard_id,
        min_phlo_price,
        block_index,
    )
    .await;
    let (block_meta, status) = match result {
        Ok(meta) => (meta, Ok(())),
        Err(ValidateError::ValidationFailed(meta, status)) => (meta, Err(status)),
        Err(ValidateError::Internal(e)) => return Err(e),
    };
    dag.insert(block_meta, block).await?;
    Ok(status)
}

/// Process incoming blocks: validate a batch concurrently, insert serially in topological order,
/// notify the validated queue, and broadcast the block hash (port of `BlockProcessor.apply`).
pub async fn apply<F, Fut>(
    mut input_blocks: mpsc::Receiver<BlockMessage>,
    validated_tx: mpsc::UnboundedSender<BlockMessage>,
    shard_id: String,
    min_phlo_price: i64,
    dag: Arc<dyn BlockDagStorage>,
    block_store: BlockStore,
    runtime: Arc<RuntimeManager>,
    comm_util: Arc<CommUtil>,
    block_index: F,
    log: Arc<dyn Log>,
) where
    F: Fn(BlockHash) -> Fut + Clone + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<BlockIndex, String>> + Send + 'static,
{
    let source = LogSource::new("casper.blocks.BlockProcessor");
    while let Some(first) = input_blocks.recv().await {
        // Drain a batch of dependency-free blocks, bounded by the concurrency cap.
        let mut batch = vec![first];
        while batch.len() < max_parallel_block_validation() {
            match input_blocks.try_recv() {
                Ok(block) => batch.push(block),
                Err(_) => break,
            }
        }

        // Validate each block concurrently. Validation is verify-only (replay) and forks its own
        // replay runtime per block, so blocks in the batch are independent.
        let mut handles = Vec::with_capacity(batch.len());
        for block in &batch {
            let dag = dag.clone();
            let block_store = block_store.clone();
            let runtime = runtime.clone();
            let shard_id = shard_id.clone();
            let block_index = block_index.clone();
            let block = block.clone();
            handles.push(tokio::spawn(async move {
                crate::multi_parent_casper::validate(
                    dag.as_ref(),
                    &block_store,
                    runtime.as_ref(),
                    &block,
                    &shard_id,
                    min_phlo_price,
                    &block_index,
                )
                .await
            }));
        }

        // Insert serially in drained order (a valid topological order: parents are emitted before
        // children), then forward/broadcast only blocks that validated successfully.
        for (block, handle) in batch.into_iter().zip(handles) {
            let result = match handle.await {
                Ok(r) => r,
                Err(e) => {
                    log.error(source, &format!("validator task panicked: {e}"));
                    continue;
                }
            };
            let (block_meta, status) = match result {
                Ok(meta) => (meta, Ok(())),
                Err(ValidateError::ValidationFailed(meta, status)) => (meta, Err(status)),
                Err(ValidateError::Internal(e)) => {
                    log.error(
                        source,
                        &format!("Block {} processing error: {e}", block.block_hash.to_hex()),
                    );
                    continue;
                }
            };
            if let Err(e) = dag.insert(block_meta, block.clone()).await {
                log.error(
                    source,
                    &format!("Block {} insert error: {e}", block.block_hash.to_hex()),
                );
                continue;
            }
            match status {
                Ok(()) => {
                    let _ = validated_tx.send(block.clone());
                    comm_util
                        .send_block_hash(&block.block_hash, block.sender.as_bytes())
                        .await;
                }
                Err(status) => log.warn(
                    source,
                    &format!(
                        "Block {} failed validation: {status:?}",
                        block.block_hash.to_hex()
                    ),
                ),
            }
        }
    }
}
