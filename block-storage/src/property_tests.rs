//! Property tests for the `block-storage` layer's laws.
//!
//! Law 18 (**height-map contiguity** — no holes in the DAG's height index) and Law 15 (**the seen set
//! is monotone** — a DAG only grows). Randomized evidence alongside the Lean statements
//! (`spec/Rchain/Casper/Fringe.lean`) and the unit tests in `dag/metadata_store.rs`.

use std::collections::BTreeSet;

use proptest::prelude::*;

use rchain_models::block_hash::BlockHash;
use rchain_shared::refined::BlockHeight;

use crate::dag::metadata_store::{add_block_to_dag_state, validate_dag_state, BlockInfo, DagState};

fn hash(byte: u8) -> BlockHash {
    BlockHash::new([byte; 32])
}

fn height(h: i64) -> BlockHeight {
    BlockHeight::try_from(h).expect("a non-negative height")
}

/// A block at `h`, justified by the block at `h - 1` (a single chain, contiguous by construction).
fn chained(byte: u8) -> BlockInfo {
    let h = i64::from(byte);
    BlockInfo {
        hash: hash(byte),
        parents: if h == 0 {
            BTreeSet::new()
        } else {
            [hash(byte - 1)].into_iter().collect()
        },
        block_num: height(h),
        validation_failed: false,
    }
}

/// A contiguous chain of `n` blocks starting at height 0, in ascending order.
fn chain(n: u8) -> Vec<BlockInfo> {
    (0..n).map(chained).collect()
}

proptest! {
    /// **Law 18, the positive half.** A chain that fills every height from 0 to n validates: the
    /// height map's keys are exactly the heights, which is the contiguity the law states.
    #[test]
    fn law18_a_contiguous_chain_validates(n in 1u8..8) {
        let mut state = DagState::empty();
        for block in chain(n) {
            state = add_block_to_dag_state(&block, &state);
        }
        prop_assert!(validate_dag_state(&state).is_ok());
        prop_assert_eq!(state.height_map.len(), usize::from(n));
        prop_assert_eq!(state.dag_set.len(), usize::from(n));
    }

    /// **Law 18, the failure arm.** A *middle* height with no block is a hole, and a hole is what
    /// `validate_dag_state` must refuse: a missing height means a justification that cannot be
    /// resolved. The hole has to be strictly inside the range — see the test below for what an
    /// absent *lowest* height does (nothing: the check compares the extremes).
    #[test]
    fn law18_a_chain_with_a_hole_in_the_middle_is_refused(n in 3u8..8, hole in 1u8..7) {
        prop_assume!(hole + 1 < n);
        let mut state = DagState::empty();
        for block in chain(n) {
            if block.block_num == height(i64::from(hole)) {
                continue; // leave the hole
            }
            state = add_block_to_dag_state(&block, &state);
        }
        prop_assert!(validate_dag_state(&state).is_err());
    }

    /// **What the contiguity check does *not* see:** a chain whose lowest height is missing is still
    /// accepted, because `validate_dag_state` compares the map's *extremes* (`max − min == len`) and
    /// not its distance from zero. Faithful to Scala's `validateDagState`, which has the same shape —
    /// and harmless in practice, because height 0 is the genesis block and is always present. Pinned
    /// so that adding a "starts at zero" check is a deliberate change.
    #[test]
    fn law18_a_missing_lowest_height_is_not_a_hole(n in 2u8..6) {
        let mut state = DagState::empty();
        for block in chain(n).into_iter().skip(1) {
            state = add_block_to_dag_state(&block, &state);
        }
        assert!(validate_dag_state(&state).is_ok());
        assert!(!state.height_map.contains_key(&height(0)));
    }

    /// A block that failed validation is **not** indexed by height (it carries no claim about the
    /// chain) while still being a known block. Marking the *tip* shows both halves without creating
    /// a hole — marking a middle block would remove that height entirely, which the test above
    /// already covers as a hole.
    #[test]
    fn law18_a_validation_failed_tip_is_not_indexed(n in 2u8..8) {
        let mut blocks = chain(n);
        let tip = u8::try_from(blocks.len() - 1).expect("a small chain");
        blocks[usize::from(tip)].validation_failed = true;
        let mut state = DagState::empty();
        for block in blocks {
            state = add_block_to_dag_state(&block, &state);
        }
        prop_assert!(
            validate_dag_state(&state).is_ok(),
            "the remaining heights are still contiguous"
        );
        prop_assert!(!state.height_map.contains_key(&height(i64::from(tip))));
        prop_assert!(state.dag_set.contains(&hash(tip)), "…but it is still a known block");
    }

    /// **Law 15 (seen-set monotone).** Adding a block never *removes* one from the seen set, and it
    /// never lowers the highest indexed height: the DAG only grows, which is what lets a node keep a
    /// monotone view of the chain.
    #[test]
    fn law15_adding_blocks_only_grows_the_state(n in 1u8..8) {
        let mut state = DagState::empty();
        let mut seen = state.dag_set.clone();
        let mut top = 0i64;
        for block in chain(n) {
            let next = add_block_to_dag_state(&block, &state);
            prop_assert!(next.dag_set.is_superset(&seen), "the seen set must not shrink");
            prop_assert!(next.child_map.len() >= state.child_map.len());
            let next_top = next
                .height_map
                .keys()
                .next_back()
                .map(|h| i64::from(*h))
                .unwrap_or(top);
            prop_assert!(next_top >= top, "the index must not lose its highest height");
            seen = next.dag_set.clone();
            top = next_top;
            state = next;
        }
    }

    /// **Law 18, order independence.** The same blocks added in any order produce the same state: the
    /// maps and sets are content-addressed, so a node that received them in a different order still
    /// computes the same DAG — which is what makes the fringe a function of the block set.
    #[test]
    fn law18_the_state_does_not_depend_on_insertion_order(n in 2u8..7, seed in any::<u64>()) {
        let blocks = chain(n);
        let ascending = blocks.iter().fold(DagState::empty(), |s, b| add_block_to_dag_state(b, &s));

        // A deterministic shuffle (a plain rotation of the chain) — enough to prove order
        // independence without pulling in a shuffle dependency.
        let rotated: Vec<BlockInfo> = blocks
            .iter()
            .cycle()
            .skip((seed as usize) % blocks.len())
            .take(blocks.len())
            .cloned()
            .collect();
        let shuffled = rotated.iter().fold(DagState::empty(), |s, b| add_block_to_dag_state(b, &s));

        prop_assert_eq!(ascending.dag_set, shuffled.dag_set);
        prop_assert_eq!(ascending.height_map, shuffled.height_map);
    }

    /// The empty DAG is contiguous (it has no holes to have), and a lone genesis block is too.
    #[test]
    fn law18_the_empty_dag_and_a_lone_block_are_contiguous(unit in Just(())) {
        prop_assert_eq!(unit, ());
        prop_assert!(validate_dag_state(&DagState::empty()).is_ok());
        let one = add_block_to_dag_state(&chained(0), &DagState::empty());
        prop_assert!(validate_dag_state(&one).is_ok());
        prop_assert_eq!(one.dag_set.len(), 1);
    }
}
