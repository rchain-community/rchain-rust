//! Block metadata store — in-memory DAG state.
//!
//! Mirrors the pure logic of
//! `block-storage/src/main/scala/coop/rchain/blockstorage/dag/BlockMetadataStore.scala`. The
//! `F[_]`/`Ref`/store wrapper is deferred to the async layer; these pure functions implement
//! `addBlockToDagState` / `validateDagState` / `recreateInMemoryState` (Law 18: height-map
//! contiguity).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use rchain_models::block_hash::BlockHash;
use rchain_models::block_metadata::BlockMetadata;
use rchain_shared::refined::{BlockHeight, NonNegI64};

/// The in-memory DAG state.
///
/// The three maps are `Arc`-shared with the DAG representation that reads them (AUDIT C56's owed
/// paragraph): the store keeps one index and the representation points at the same allocations,
/// instead of every insert cloning the whole index into a second copy. `Arc::make_mut` keeps the
/// pure functions pure — a state that someone else still holds is copied before it is extended —
/// while an unshared state is extended in place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DagState {
    pub dag_set: Arc<BTreeSet<BlockHash>>,
    pub child_map: Arc<BTreeMap<BlockHash, BTreeSet<BlockHash>>>,
    pub height_map: Arc<BTreeMap<BlockHeight, BTreeSet<BlockHash>>>,
}

impl DagState {
    pub fn empty() -> Self {
        Self {
            dag_set: Arc::new(BTreeSet::new()),
            child_map: Arc::new(BTreeMap::new()),
            height_map: Arc::new(BTreeMap::new()),
        }
    }
}

/// A projection of `BlockMetadata` used to rebuild in-memory state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockInfo {
    pub hash: BlockHash,
    pub parents: BTreeSet<BlockHash>,
    pub block_num: BlockHeight,
    pub validation_failed: bool,
}

pub fn block_metadata_to_info(meta: &BlockMetadata) -> BlockInfo {
    BlockInfo {
        hash: meta.block_hash,
        parents: meta.justifications.clone(),
        block_num: meta.block_num,
        validation_failed: meta.validation_failed,
    }
}

pub fn add_block_to_dag_state(block: &BlockInfo, state: &DagState) -> DagState {
    let mut next = state.clone();
    add_block_to_dag_state_mut(block, &mut next);
    next
}

/// Extend a DAG state with one block, in place — the live path's form of
/// [`add_block_to_dag_state`].
///
/// The maps are `Arc`-shared, so this copies a map only if some other holder (a DAG representation
/// snapshot a reader took) still points at it; the storage's own insert is serialized, and the
/// representation is refreshed from the store's new `Arc`s at the end of that insert.
pub fn add_block_to_dag_state_mut(block: &BlockInfo, state: &mut DagState) {
    Arc::make_mut(&mut state.dag_set).insert(block.hash);

    let child_map = Arc::make_mut(&mut state.child_map);
    for parent in &block.parents {
        child_map.entry(*parent).or_default().insert(block.hash);
    }
    child_map.entry(block.hash).or_default();

    if !block.validation_failed {
        Arc::make_mut(&mut state.height_map)
            .entry(block.block_num)
            .or_default()
            .insert(block.hash);
    }
}

/// Validate that the height-map keys form a contiguous range (Law 18).
pub fn validate_dag_state(state: &DagState) -> Result<(), String> {
    let m = &state.height_map;
    let (min, max) = match (m.keys().next(), m.keys().next_back()) {
        (Some(first), Some(last)) => (*first, *last + NonNegI64::one()),
        _ => (BlockHeight::zero(), BlockHeight::zero()),
    };
    if max - min != i64::try_from(m.len()).unwrap_or(i64::MAX) {
        return Err("DAG store height map has numbers not in sequence.".to_string());
    }
    Ok(())
}

/// Rebuild in-memory state from a block-info map, then validate.
pub fn recreate_in_memory_state(
    blocks: &BTreeMap<BlockHash, BlockInfo>,
) -> Result<DagState, String> {
    let mut state = DagState::empty();
    for block in blocks.values() {
        state = add_block_to_dag_state(block, &state);
    }
    validate_dag_state(&state)?;
    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(byte: u8) -> BlockHash {
        let mut bytes = [0u8; 32];
        bytes[0] = byte;
        BlockHash::new(bytes)
    }

    fn info(hash: BlockHash, parents: &[BlockHash], block_num: i64) -> BlockInfo {
        BlockInfo {
            hash,
            parents: parents.iter().copied().collect(),
            block_num: BlockHeight::try_from(block_num).unwrap(),
            validation_failed: false,
        }
    }

    #[test]
    fn law18_contiguous_height_map_is_valid() {
        let mut blocks = BTreeMap::new();
        let h0 = info(hash(0), &[], 0);
        let h1 = info(hash(1), &[hash(0)], 1);
        let h2 = info(hash(2), &[hash(1)], 2);
        blocks.insert(hash(0), h0);
        blocks.insert(hash(1), h1);
        blocks.insert(hash(2), h2);
        recreate_in_memory_state(&blocks).unwrap(); // no error
    }

    #[test]
    fn law18_height_map_with_holes_errors() {
        let mut blocks = BTreeMap::new();
        blocks.insert(hash(0), info(hash(0), &[], 0));
        // block_num 2 with no block_num 1 -> hole.
        blocks.insert(hash(2), info(hash(2), &[hash(0)], 2));
        assert_eq!(
            recreate_in_memory_state(&blocks).unwrap_err(),
            "DAG store height map has numbers not in sequence."
        );
    }

    #[test]
    fn add_block_to_dag_state_builds_child_map() {
        let parent = hash(0);
        let child = hash(1);
        let state = DagState::empty();
        let s1 = add_block_to_dag_state(&info(parent, &[], 0), &state);
        let s2 = add_block_to_dag_state(&info(child, &[parent], 1), &s1);
        assert_eq!(s2.child_map[&parent], [child].into_iter().collect());
        assert!(s2.child_map[&child].is_empty());
        assert_eq!(*s2.dag_set, [parent, child].into_iter().collect());
    }
}
