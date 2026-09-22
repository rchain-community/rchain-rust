//! The in-memory DAG view.
//!
//! Mirrors `block-storage/src/main/scala/coop/rchain/blockstorage/dag/DagRepresentation.scala`.

use std::collections::{BTreeMap, BTreeSet};

use rchain_models::block_hash::BlockHash;
use rchain_models::fringe_data::FringeData;
use rchain_models::validator::Validator;
use rchain_shared::base16;
use rchain_shared::refined::{BlockHeight, NonNegI64};

use crate::errors::StorageError;

use super::finalizer::Message;
use super::message_state::DagMessageState;

/// The in-memory state of the DAG — an index of the block metadata store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DagRepresentation {
    pub dag_set: BTreeSet<BlockHash>,
    pub child_map: BTreeMap<BlockHash, BTreeSet<BlockHash>>,
    pub height_map: BTreeMap<BlockHeight, BTreeSet<BlockHash>>,
    pub dag_message_state: DagMessageState<BlockHash, Validator>,
    pub fringe_states: BTreeMap<BTreeSet<BlockHash>, FringeData>,
}

impl DagRepresentation {
    pub fn latest_fringe(&self) -> BTreeSet<Message<BlockHash, Validator>> {
        self.dag_message_state.latest_fringe()
    }

    /// The finalized blocks are the seen-closure of the latest fringe.
    pub fn finalized_blocks_set(&self) -> BTreeSet<BlockHash> {
        self.latest_fringe()
            .iter()
            .flat_map(|m| m.seen.iter().copied())
            .collect()
    }

    pub fn latest_block_number(&self) -> i64 {
        self.height_map
            .keys()
            .last()
            .map(|h| i64::from(*h + NonNegI64::one()))
            .unwrap_or(0)
    }

    pub fn last_finalized_block_hash(&self) -> Option<BlockHash> {
        self.latest_fringe()
            .iter()
            .map(|m| (m.height, m.id))
            .max()
            .map(|(_, id)| id)
    }

    /// The last finalized block hash, or an error if no fringe is available (port of
    /// `DagRepresentationSyntax.lastFinalizedBlockUnsafe`).
    pub fn last_finalized_block_unsafe(&self) -> Result<BlockHash, String> {
        self.last_finalized_block_hash()
            .ok_or_else(|| "Finalized fringe is not available.".to_string())
    }

    pub fn contains(&self, block_hash: &BlockHash) -> bool {
        self.dag_set.contains(block_hash)
    }

    pub fn children(&self, block_hash: &BlockHash) -> Option<&BTreeSet<BlockHash>> {
        self.child_map.get(block_hash)
    }

    pub fn is_finalized(&self, block_hash: &BlockHash) -> bool {
        self.finalized_blocks_set().contains(block_hash)
    }

    /// Blocks grouped by height in the requested range (or `None` for an invalid range).
    pub fn topo_sort(
        &self,
        start_block_number: i64,
        maybe_end_block_number: Option<i64>,
    ) -> Option<Vec<Vec<BlockHash>>> {
        let max_number = self.latest_block_number();
        let start_number = 0.max(start_block_number);
        let end_number = maybe_end_block_number
            .map(|e| e.min(max_number))
            .unwrap_or(max_number);
        let valid_range = start_number >= 0 && start_number <= end_number;
        if valid_range {
            Some(
                self.height_map
                    .iter()
                    .filter(|(h, _)| i64::from(**h) >= start_number && i64::from(**h) <= end_number)
                    .map(|(_, v)| v.iter().copied().collect())
                    .collect(),
            )
        } else {
            None
        }
    }

    /// Blocks grouped by height in the requested range, or an error for an invalid range (port of
    /// `DagRepresentationSyntax.topoSortUnsafe`).
    pub fn topo_sort_unsafe(
        &self,
        start_block_number: i64,
        maybe_end_block_number: Option<i64>,
    ) -> Result<Vec<Vec<BlockHash>>, StorageError> {
        self.topo_sort(start_block_number, maybe_end_block_number)
            .ok_or(StorageError::TopoSortFragmentParameterError {
                start_block_number,
                end_block_number: i64::MAX,
            })
    }

    /// Find a block hash by (possibly truncated) hex prefix.
    pub fn find(&self, truncated_hash: &str) -> Option<BlockHash> {
        // Validate-on-ingress: reject non-hex input (`decode` returns `None`) rather than silently
        // dropping invalid chars via `unsafe_decode`.
        if truncated_hash.len().is_multiple_of(2) {
            let bytes = base16::decode(truncated_hash)?;
            self.dag_set.iter().find(|h| h.starts_with(&bytes)).copied()
        } else {
            let bytes = base16::decode(&truncated_hash[..truncated_hash.len() - 1])?;
            self.dag_set
                .iter()
                .filter(|h| h.starts_with(&bytes))
                .find(|h| h.to_hex().starts_with(truncated_hash))
                .copied()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(byte: u8) -> BlockHash {
        BlockHash::new([byte; 32])
    }

    /// A representation with a small chain 0 → 1 → 2 at heights 0..=2.
    fn chain() -> DagRepresentation {
        let (b0, b1, b2) = (hash(0), hash(1), hash(2));
        DagRepresentation {
            dag_set: [b0, b1, b2].into_iter().collect(),
            child_map: [
                (b0, [b1].into_iter().collect()),
                (b1, [b2].into_iter().collect()),
                (b2, BTreeSet::new()),
            ]
            .into_iter()
            .collect(),
            height_map: [
                (
                    BlockHeight::try_from(0).unwrap(),
                    [b0].into_iter().collect(),
                ),
                (
                    BlockHeight::try_from(1).unwrap(),
                    [b1].into_iter().collect(),
                ),
                (
                    BlockHeight::try_from(2).unwrap(),
                    [b2].into_iter().collect(),
                ),
            ]
            .into_iter()
            .collect(),
            dag_message_state: DagMessageState::empty(),
            fringe_states: BTreeMap::new(),
        }
    }

    /// An empty DAG has no last finalized block, and the `_unsafe` variant says so by name rather
    /// than panicking — the arm a caller reads when the fringe is not available yet.
    #[test]
    fn an_empty_dag_has_no_last_finalized_block() {
        let dag = DagRepresentation {
            dag_set: BTreeSet::new(),
            child_map: BTreeMap::new(),
            height_map: BTreeMap::new(),
            dag_message_state: DagMessageState::empty(),
            fringe_states: BTreeMap::new(),
        };
        assert_eq!(dag.last_finalized_block_hash(), None);
        assert_eq!(
            dag.last_finalized_block_unsafe().expect_err("no fringe"),
            "Finalized fringe is not available."
        );
        assert_eq!(dag.latest_block_number(), 0);
        assert!(dag.latest_fringe().is_empty());
        assert!(dag.finalized_blocks_set().is_empty());
    }

    /// The height index drives `latest_block_number` (one past the highest height) and the range
    /// query: the end bound is clamped to the DAG's height, so a caller asking beyond the tip gets
    /// the tip's group rather than an error.
    #[test]
    fn the_height_index_drives_the_range_query() {
        let dag = chain();
        assert_eq!(dag.latest_block_number(), 3, "one past height 2");

        let all = dag.topo_sort(0, None).expect("a valid range");
        assert_eq!(all, vec![vec![hash(0)], vec![hash(1)], vec![hash(2)]]);

        // A start beyond the tip is an *invalid* range (`start > end`), not an empty answer.
        assert_eq!(dag.topo_sort(9, None), None);
        // A negative start is clamped to zero, so it is valid.
        assert_eq!(dag.topo_sort(-5, None).expect("clamped").len(), 3);
        // An end beyond the tip is clamped.
        assert_eq!(dag.topo_sort(0, Some(99)).expect("clamped").len(), 3);
        // A sub-range.
        assert_eq!(
            dag.topo_sort(1, Some(1)).expect("valid"),
            vec![vec![hash(1)]]
        );
        let err = dag
            .topo_sort_unsafe(9, Some(1))
            .expect_err("an invalid range is an error");
        assert!(format!("{err}").contains("topo-sort"), "{err}");
    }

    /// `find` resolves a hex prefix, and **rejects non-hex input rather than dropping the bad
    /// characters** (the validate-on-ingress rule): an odd-length prefix still resolves, but a
    /// malformed one finds nothing instead of matching a different block.
    #[test]
    fn find_resolves_a_hex_prefix_and_rejects_junk() {
        let dag = chain();
        let full = hash(2).to_hex();

        assert_eq!(dag.find(&full), Some(hash(2)));
        assert_eq!(dag.find(&full[..8]), Some(hash(2)), "an even-length prefix");
        // Odd length: the last nibble is compared as text.
        assert_eq!(dag.find(&full[..9]), Some(hash(2)));
        let odd_junk = format!("{}z", &full[..8]);
        assert_eq!(dag.find(&odd_junk), None, "not hex");
        assert_eq!(
            dag.find(""),
            Some(hash(0)),
            "the empty prefix matches the first block"
        );
        assert_eq!(dag.find("ffffff"), None, "no block starts with that");
    }

    /// `contains`/`children`/`is_finalized` are the DAG queries the block processor uses.
    #[test]
    fn membership_and_children_queries() {
        let dag = chain();
        assert!(dag.contains(&hash(1)));
        assert!(!dag.contains(&hash(9)));
        assert_eq!(
            dag.children(&hash(0)),
            Some(&[hash(1)].into_iter().collect())
        );
        assert_eq!(
            dag.children(&hash(9)),
            None,
            "an unknown block has no children"
        );
        // With no fringe state, nothing is finalized.
        assert!(!dag.is_finalized(&hash(0)));
    }
}
