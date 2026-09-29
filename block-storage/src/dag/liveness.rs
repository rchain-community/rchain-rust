//! The **live weight set** — one liveness rule, and the two consumers that must not disagree (#70).
//!
//! Finality's fringe gate asks two different questions of the same bonds map, and they were answered by
//! one map until 2026-09-29:
//!
//! - **who must have seen a candidate message** — `calculate_fringe`'s full-partition filter, whose
//!   `bonded_senders` set is the map's *keys*;
//! - **what the quorum is measured against** — `total_stake`, the map's *values*.
//!
//! With one map, a bonded validator that produces no message can never be "seen by every seer", so the
//! partition is unsatisfiable and finality stops **whatever share of the stake the survivors hold**.
//! Measured on a three-validator devnet at `100/100/50` on 2026-09-29: with the 50-stake validator
//! stopped, the two survivors at 80 % of the pool did not resume finality ([#70]).
//!
//! The fix separates the two questions, and keeping them separate is the whole design:
//!
//! - the **partition** ranges over the live set — the bonded validators whose latest message is within
//!   [`LIVENESS_WINDOW`] heights of the tip. A validator that has stopped is not required to be seen, so
//!   the survivors can complete a partition;
//! - the **quorum** stays the whole bonded map. A quorum over the live set instead would let *any*
//!   self-consistent subset finalise — under a partition each side would finalise its own view, two
//!   conflicting finalisations, and safety would be gone. "Finality needs quorum stake, not live nodes"
//!   is the requirement, and it is the denominator that keeps it.
//!
//! This is a **deliberate departure from the Scala**, which has no liveness predicate at all: its
//! `calculateFinalization` takes the bonds map as given and uses it for both questions
//! (`Finalizer.scala`). The §6 row carries the reason and the case that motivated it.
//!
//! Determinism needs no new state: the inputs are the bonds and the heights the DAG already records for
//! the justifications, so every node validating the same block derives the same live set — and
//! reversibility is automatic, because a returning validator's message is at the tip and its stake
//! re-enters both sides on the next block.
//!
//! [#70]: https://github.com/rchain-community/rchain-rust/issues/70

use std::collections::{BTreeMap, BTreeSet};
use std::hash::Hash;

use rchain_shared::refined::{BlockHeight, NonNegI64};

use super::finalizer::{Finalizer, Message};

/// How many heights of silence a bonded validator may accumulate before its stake leaves the live
/// weight set — the one number the proposer's attestation guard and the finalizer's fringe gate share.
///
/// **Provisional, and deliberately a constant rather than a genesis parameter.** As a `PosParams` field
/// it would enter `spec/GENESIS.md` and move the genesis block — a hard fork, #51 category A — so
/// parameterising it belongs with #24 (on-chain shard configuration storage), which is already in that
/// category. The value tolerates a peer lagging by a few blocks while dropping one that has stopped for
/// more than a handful; what it *should* be is a measurement.
pub const LIVENESS_WINDOW: i64 = 5;

/// `tip - message`, saturating rather than wrapping: a message cannot be ahead of the tip, but a
/// `BlockHeight` subtraction that wrapped would read as maximally *stale* and drop a live validator.
///
/// The saturation is defensive — both operands are non-negative, so the range is
/// `[-(2⁶³-1), 2⁶³-1]` and it cannot trigger — but the **sign** is not: a message ahead of the tip
/// yields a negative, which is within any non-negative window, so `heights_behind` must not be read as
/// a magnitude by a caller deciding staleness.
pub fn heights_behind(tip: BlockHeight, message: BlockHeight) -> i64 {
    i64::from(tip).saturating_sub(i64::from(message))
}

/// Each sender's **newest** height among these `(sender, height)` pairs.
///
/// The two callers hold different shapes — the proposer has `BlockMetadata`, the finalizer `Message` —
/// so the rule takes pairs and neither type. Taking the *newest* per sender matters: a block's
/// justifications are one message per validator by construction, but nothing in the format forbids two
/// from one sender, and the maximum is the value the rule means.
pub fn latest_heights<S, I>(messages: I) -> BTreeMap<S, BlockHeight>
where
    S: Ord + Clone,
    I: IntoIterator<Item = (S, BlockHeight)>,
{
    let mut latest: BTreeMap<S, BlockHeight> = BTreeMap::new();
    for (sender, height) in messages {
        latest
            .entry(sender)
            .and_modify(|h| *h = (*h).max(height))
            .or_insert(height);
    }
    latest
}

/// The bonded stake whose latest message is within `window` heights of the tip.
///
/// A validator with **no** message at all is not live — that is the case the rule exists for, and it is
/// why the lookup is an `is_some_and` rather than an `unwrap_or`: a default height would make silence
/// read as liveness.
pub fn live_weight_set<S>(
    bonds: &BTreeMap<S, NonNegI64>,
    latest: &BTreeMap<S, BlockHeight>,
    tip: BlockHeight,
    window: i64,
) -> BTreeMap<S, NonNegI64>
where
    S: Ord + Clone,
{
    bonds
        .iter()
        .filter(|(v, _)| {
            latest
                .get(*v)
                .is_some_and(|h| heights_behind(tip, *h) <= window)
        })
        .map(|(v, s)| (v.clone(), *s))
        .collect()
}

/// **The finalization a node computes for these justifications** — the one entry point, so the block
/// creator ([`super::message_state::DagMessageState::create_message`]) and the validator
/// (`casper::multi_parent_casper`) cannot answer differently. They must not: the creator writes the
/// result into the block's `fringe`, and a validator that derived a different fringe would refuse the
/// block it was handed.
///
/// `bonds` is the whole bonded map (the quorum's denominator); the partition is derived from it and the
/// justifications' heights.
pub fn calculate_finalization<M, S>(
    finalizer: &Finalizer<'_, M, S>,
    justifications: &BTreeSet<Message<M, S>>,
    bonds: &BTreeMap<S, NonNegI64>,
) -> (BTreeSet<Message<M, S>>, Option<BTreeSet<Message<M, S>>>)
where
    M: Ord + Clone + Eq + Hash,
    S: Ord + Clone + Eq + Hash,
{
    let tip = justifications
        .iter()
        .map(|m| m.height)
        .max()
        .unwrap_or_else(BlockHeight::zero);
    let latest = latest_heights(justifications.iter().map(|m| (m.sender.clone(), m.height)));
    let live = live_weight_set(bonds, &latest, tip, LIVENESS_WINDOW);
    finalizer.calculate_finalization(justifications, &live, bonds)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(n: i64) -> BlockHeight {
        BlockHeight::try_from(n).expect("height")
    }

    fn bonds(entries: &[(u8, i64)]) -> BTreeMap<u8, NonNegI64> {
        entries
            .iter()
            .map(|(v, s)| (*v, NonNegI64::try_from(*s).expect("stake")))
            .collect()
    }

    /// **The window is a window**: a validator whose latest message is at the tip or within the window
    /// is live; one beyond it is not.
    #[test]
    fn the_window_is_heights_behind_the_tip() {
        let bonds = bonds(&[(1, 10), (2, 10), (3, 10)]);
        let latest = latest_heights([(1u8, h(20)), (2, h(20 - 5)), (3, h(20 - 6))]);

        let live = live_weight_set(&bonds, &latest, h(20), 5);
        assert_eq!(
            live.keys().copied().collect::<Vec<_>>(),
            vec![1, 2],
            "the tip and the window's edge are live; one height beyond is not"
        );
    }

    /// **A validator that has never spoken is not live.** Its absence is the case the rule exists for,
    /// and a default height would have made silence read as liveness.
    #[test]
    fn a_validator_with_no_message_at_all_is_not_live() {
        let bonds = bonds(&[(1, 10), (2, 10)]);
        let latest = latest_heights([(1u8, h(20))]);

        let live = live_weight_set(&bonds, &latest, h(20), 5);
        assert_eq!(live.keys().copied().collect::<Vec<_>>(), vec![1]);
    }

    /// **A message ahead of the tip is not stale**: `tip - message` goes negative, and a negative is
    /// within any non-negative window. Only the sign matters to the rule, and this is the case where
    /// reading the *magnitude* would drop a validator that is ahead of us.
    #[test]
    fn a_message_ahead_of_the_tip_is_not_stale() {
        assert_eq!(
            heights_behind(h(3), h(9)),
            -6,
            "plain subtraction, not a clamp"
        );
        let live = live_weight_set(&bonds(&[(1, 10)]), &latest_heights([(1u8, h(9))]), h(3), 5);
        assert!(
            live.contains_key(&1),
            "an ahead message is at worst at the tip"
        );
    }

    /// **A returning validator re-enters with no extra state**: its message jumps to the tip and its
    /// stake is back on the same call.
    #[test]
    fn a_returning_validator_re_enters_on_the_next_block() {
        let bonds = bonds(&[(1, 100), (2, 100), (3, 50)]);
        let silent = latest_heights([(1u8, h(30)), (2, h(30))]);
        assert_eq!(live_weight_set(&bonds, &silent, h(30), 5).len(), 2);

        let returned = latest_heights([(1u8, h(30)), (2, h(30)), (3, h(31))]);
        assert_eq!(live_weight_set(&bonds, &returned, h(31), 5).len(), 3);
    }
}
