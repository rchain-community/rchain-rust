//! Property tests for the `sdk` layer's laws.
//!
//! Law 14 (finality needs **more than** two thirds of the bonded stake) and Law 17 (the merge rejects
//! a *unique* minimal-cost conflict set, deterministically). Both are stated in `spec/INVENTORY.md`;
//! these are the randomized half of the evidence, alongside the Lean statements and the unit tests in
//! `sdk/src/consensus.rs` and `sdk/src/dag/merging.rs`.

use std::collections::{BTreeMap, BTreeSet};

use proptest::prelude::*;

use crate::consensus::is_super_majority;
use crate::dag::merging::{
    compute_conflicts_map, compute_optimal_rejection, compute_rejection_options,
};

/// A **legally shaped** conflict map: built through `compute_conflicts_map`, so it is undirected and
/// only contains nodes that actually conflict — the shape the merge hands to
/// `compute_rejection_options`. Generating maps by hand (asymmetric ones) tests a state the callers
/// cannot produce.
fn arb_conflicts() -> impl Strategy<Value = BTreeMap<u8, BTreeSet<u8>>> {
    (
        prop::collection::btree_set(0u8..5, 1..5),
        prop::collection::vec((0u8..5, 0u8..5), 0..5),
    )
        .prop_map(|(deploys, edges)| {
            let edges: BTreeSet<(u8, u8)> = edges.into_iter().collect();
            compute_conflicts_map(&deploys, &deploys, |a, b| {
                edges.contains(&(*a, *b)) || edges.contains(&(*b, *a))
            })
        })
}

proptest! {
    /// **Law 14's boundary, exactly.** A super-majority is `3·stake > 2·total`, so two thirds —
    /// whether floored (`2t/3`) or taken as `t/3·2` — is *not* enough, while the very next unit is.
    /// The difference between `>` and `>=` here is a chain that finalizes on a minority.
    #[test]
    fn law14_super_majority_is_strictly_more_than_two_thirds(total in 1i128..1_000_000i128) {
        let two_thirds = total * 2 / 3;
        prop_assert!(!is_super_majority(two_thirds, total), "2/3 is not enough");
        prop_assert!(!is_super_majority(total / 3 * 2, total));
        prop_assert!(is_super_majority(two_thirds + 1, total), "one more unit is enough");
        // A single validator holding everything always qualifies.
        prop_assert!(is_super_majority(total, total));
    }

    /// Support is monotone: if `s` is a super-majority, so is anything above it (for the same total).
    #[test]
    fn law14_super_majority_is_monotone_in_support(total in 1i128..1_000_000i128, s in 0i128..1_000_000i128) {
        if is_super_majority(s, total) {
            prop_assert!(is_super_majority(s + 1, total));
        }
    }

    /// **Why the port uses exact integers.** Scala computes `stake.toDouble / totalStake > 2d/3`,
    /// which rounds for stakes past 2⁵³ — a validator at exactly the `f64` boundary would be admitted
    /// or refused by rounding rather than by the rule. The integer form cannot round, so the
    /// boundary stays where the law puts it (the `two_thirds` properties above, past the mantissa).
    #[test]
    fn law14_the_two_thirds_boundary_survives_past_the_f64_mantissa(big in 1u64..1_000_000u64) {
        let total = (1i128 << 53) + i128::from(big);
        let two_thirds = total * 2 / 3;
        prop_assert!(!is_super_majority(two_thirds, total));
        prop_assert!(is_super_majority(two_thirds + 1, total));
    }

    /// **Law 17's uniqueness.** `compute_optimal_rejection` is a function of the option *set* — the
    /// same input gives the same rejection, and the rejection is always one of the options offered
    /// (never an invented set).
    #[test]
    fn law17_the_chosen_rejection_is_one_of_the_options(
        options in prop::collection::btree_set(
            prop::collection::btree_set(0u8..6, 0..4),
            1..5,
        )
    ) {
        let cost = |d: &u8| i64::from(*d);
        let first = compute_optimal_rejection(&options, cost);
        let second = compute_optimal_rejection(&options, cost);
        prop_assert_eq!(&first, &second);
        prop_assert!(options.contains(&first), "the rejection must be one of the options");
    }

    /// **Law 17's minimality.** The chosen set's total cost is no greater than any other option's —
    /// that is what "min-cost rejection" means, and it is the property a broken comparator would
    /// violate while still returning *some* option.
    #[test]
    fn law17_the_chosen_rejection_minimizes_the_total_cost(
        options in prop::collection::btree_set(
            prop::collection::btree_set(0u8..6, 0..4),
            1..5,
        ),
        weights in prop::collection::btree_map(0u8..6, 1i64..10, 0..6),
    ) {
        let cost = |d: &u8| *weights.get(d).unwrap_or(&1);
        let chosen = compute_optimal_rejection(&options, cost);
        let chosen_cost: i64 = chosen.iter().map(cost).sum();
        for option in &options {
            let option_cost: i64 = option.iter().map(cost).sum();
            prop_assert!(chosen_cost <= option_cost, "chosen {chosen:?} vs option {option:?}");
        }
    }

    /// **The survivors of a rejection option are mutually conflict-free.** An option is the set to
    /// *drop*: the merge keeps the rest, so the invariant is that nothing kept conflicts with
    /// anything else kept — i.e. every conflict of a survivor is itself rejected.
    ///
    /// (The direction matters and my first version had it backwards: rejecting a deploy does *not*
    /// require rejecting its conflicts too — dropping one side of a conflict is exactly how the
    /// conflict is resolved. The rejected-set form of the invariant is "for every kept `k`, all of
    /// `k`'s conflicts are rejected".)
    #[test]
    fn law17_the_survivors_of_a_rejection_option_are_conflict_free(
        conflicts in arb_conflicts()
    ) {
        let keys: BTreeSet<u8> = conflicts.keys().copied().collect();
        for option in compute_rejection_options(&conflicts) {
            for kept in keys.difference(&option) {
                if let Some(opposed) = conflicts.get(kept) {
                    for other in opposed {
                        prop_assert!(
                            option.contains(other),
                            "keeping {kept} requires rejecting its conflict {other}: \
                             option {option:?}"
                        );
                    }
                }
            }
        }
    }

    /// With deploys but no conflicts, the only rejection option is the empty set — nothing needs to
    /// be dropped, so `compute_optimal_rejection` picks nothing.
    #[test]
    fn law17_deploys_without_conflicts_need_no_rejection(keys in prop::collection::btree_set(0u8..6, 1..5)) {
        let conflicts: BTreeMap<u8, BTreeSet<u8>> =
            keys.into_iter().map(|k| (k, BTreeSet::new())).collect();
        let options = compute_rejection_options(&conflicts);
        prop_assert!(options.contains(&BTreeSet::new()));
        prop_assert_eq!(compute_optimal_rejection(&options, |_| 1), BTreeSet::new());
    }
}

/// An **empty** conflict map — no deploy to consider — yields *no* options at all rather than a
/// single empty one: the search starts from the map's keys, so with none it never runs. Faithful to
/// Scala's `computeRejectionOptions`, whose queue is built from `conflictsMap.keySet`. Not a
/// `proptest` case: the input is a single value, and the point is that it is empty.
#[test]
fn an_empty_conflict_map_yields_no_options() {
    let empty: BTreeMap<u8, BTreeSet<u8>> = BTreeMap::new();
    assert!(compute_rejection_options(&empty).is_empty());
    assert_eq!(
        compute_optimal_rejection(&BTreeSet::<BTreeSet<u8>>::new(), |_: &u8| 1),
        BTreeSet::new(),
        "and the chooser falls back to the empty rejection"
    );
}
