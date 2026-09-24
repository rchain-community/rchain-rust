//! Helpers over a message map (the Scala `MessageMapSyntax`).
//!
//! Mirrors `block-storage/src/main/scala/coop/rchain/blockstorage/dag/MessageMapSyntax.scala`.

use std::collections::{BTreeMap, BTreeSet};

use rchain_shared::refined::BlockHeight;

use super::finalizer::Message;

fn msg_at<M, S>(msg_map: &BTreeMap<M, Message<M, S>>, id: &M) -> Option<Message<M, S>>
where
    M: Ord + Clone,
    S: Clone,
{
    msg_map.get(id).cloned()
}

/// The ids of the messages between `upper_bound` and `lower_bound` (including upper-bound
/// messages) — `upper.seen \ lower.seen`, restricted to the map.
///
/// **Ids in, ids out — no message is copied.** The Scala's `upperSeen -- lowerSeen` is a set
/// difference over `Set[Message]`, and Scala's immutable `Set` holds *references* to the same
/// case-class instances, so the oracle never copies a message to answer this. Returning the
/// messages from Rust does copy them, and a `Message` carries its `seen` set — of size Θ(N) for a
/// chain of N blocks — so building the operands and the result cost Θ(N) *per message*. Two calls
/// per block made the merge scope Θ(N²) in copies: measured at 0.78 GiB of churn per block for
/// N = 5,855, pinning a core and growing without bound (AUDIT C56). Every caller needs only
/// `is_empty` and the ids, and a message's id determines its content — the same injectivity
/// `msg_map` already relies on by keying on the id.
pub fn between<M, S>(
    msg_map: &BTreeMap<M, Message<M, S>>,
    upper_bound: &BTreeSet<M>,
    lower_bound: &BTreeSet<M>,
) -> BTreeSet<M>
where
    M: Ord + Clone,
    S: Ord + Clone,
{
    let seen_ids = |bound: &BTreeSet<M>| -> BTreeSet<M> {
        bound
            .iter()
            .filter_map(|id| msg_map.get(id))
            .flat_map(|m| m.seen.iter().filter(|id| msg_map.contains_key(id)).cloned())
            .collect()
    };
    let upper_seen = seen_ids(upper_bound);
    let lower_seen = seen_ids(lower_bound);
    upper_seen.difference(&lower_seen).cloned().collect()
}

/// The max height of a justification's fringe members, or `None` when the fringe is empty (`None`
/// sorts below any `Some` height, preserving the previous `-1` sentinel ordering).
fn fringe_height<M, S>(
    msg_map: &BTreeMap<M, Message<M, S>>,
    j: &Message<M, S>,
) -> Option<BlockHeight>
where
    M: Ord + Clone,
    S: Clone,
{
    j.fringe
        .iter()
        .filter_map(|id| msg_at(msg_map, id))
        .map(|m| m.height)
        .max()
}

/// Latest fringe seen from justifications (may be empty).
pub fn latest_fringe<M, S>(
    msg_map: &BTreeMap<M, Message<M, S>>,
    justifications: &BTreeSet<Message<M, S>>,
) -> BTreeSet<Message<M, S>>
where
    M: Ord + Clone,
    S: Ord + Clone,
{
    match justifications
        .iter()
        .max_by_key(|j| fringe_height(msg_map, j))
    {
        Some(j) => j
            .fringe
            .iter()
            .filter_map(|id| msg_at(msg_map, id))
            .collect(),
        None => BTreeSet::new(),
    }
}

/// Lowest fringe for input messages.
pub fn lowest_fringe<M, S>(
    msg_map: &BTreeMap<M, Message<M, S>>,
    msgs: &BTreeSet<Message<M, S>>,
) -> BTreeSet<Message<M, S>>
where
    M: Ord + Clone,
    S: Ord + Clone,
{
    match msgs.iter().min_by_key(|j| fringe_height(msg_map, j)) {
        Some(j) => j
            .fringe
            .iter()
            .filter_map(|id| msg_at(msg_map, id))
            .collect(),
        None => BTreeSet::new(),
    }
}

/// A message with empty parents.
pub fn find_with_empty_parents<M, S>(msg_map: &BTreeMap<M, Message<M, S>>) -> Option<Message<M, S>>
where
    M: Ord + Clone,
    S: Clone,
{
    msg_map.values().find(|m| m.parents.is_empty()).cloned()
}

/// The highest fringe not required for merging.
pub fn prune_fringe<M, S>(
    msg_map: &BTreeMap<M, Message<M, S>>,
    final_fringe: &BTreeSet<M>,
    child_map: &BTreeMap<M, BTreeSet<M>>,
) -> BTreeSet<Message<M, S>>
where
    M: Ord + Clone,
    S: Ord + Clone,
{
    let children: BTreeSet<Message<M, S>> = final_fringe
        .iter()
        .flat_map(|id| child_map.get(id).into_iter().flatten())
        .filter_map(|id| msg_at(msg_map, id))
        .collect();
    lowest_fringe(msg_map, &children)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rchain_shared::refined::SeqNum;

    fn msg(id: i32, fringe: &[i32], seen: &[i32]) -> Message<i32, i32> {
        Message {
            id,
            height: BlockHeight::zero(),
            sender: 0,
            sender_seq: SeqNum::zero(),
            bonds_map: BTreeMap::new(),
            parents: BTreeSet::new(),
            fringe: fringe.iter().copied().collect(),
            seen: seen.iter().copied().collect(),
        }
    }

    #[test]
    fn find_with_empty_parents_works() {
        let m0 = msg(0, &[], &[]);
        let m1 = Message {
            parents: [0].into_iter().collect(),
            ..m0.clone()
        };
        let map: BTreeMap<i32, Message<i32, i32>> =
            [(0, m0.clone()), (1, m1)].into_iter().collect();
        assert_eq!(find_with_empty_parents(&map).unwrap().id, 0);
    }

    /// `between` is `upper.seen \ lower.seen`, restricted to ids the map holds — and it answers
    /// with ids, so no message (and no `seen` set) is copied to produce it (AUDIT C56).
    #[test]
    fn between_is_the_id_set_difference_restricted_to_the_map() {
        let ids = |xs: &[i32]| -> BTreeSet<i32> { xs.iter().copied().collect() };
        let m0 = msg(0, &[], &[0]);
        let m1 = msg(1, &[], &[0, 1]);
        // Block 2 is *not* in the map: its id must be dropped from the result, exactly as the
        // `filter_map` over the map did before.
        let map: BTreeMap<i32, Message<i32, i32>> =
            [(0, m0.clone()), (1, m1.clone())].into_iter().collect();

        // {m1}.seen \ {m0}.seen = {0,1} \ {0} = {1}
        assert_eq!(between(&map, &ids(&[1]), &ids(&[0])), ids(&[1]));
        // {m1}.seen \ {} = {0,1}
        assert_eq!(between(&map, &ids(&[1]), &ids(&[])), ids(&[0, 1]));
        // The upper bound may be an id the map does not hold: it contributes nothing.
        assert_eq!(
            between(&map, &ids(&[9]), &ids(&[])),
            BTreeSet::new(),
            "an upper-bound id outside the map must contribute no seen ids"
        );
    }

    #[test]
    fn latest_fringe_picks_max_height() {
        let f1 = msg(10, &[], &[]);
        let f2 = msg(11, &[], &[]);
        // Two justifications with different fringe heights.
        let j_low = Message {
            fringe: [10].into_iter().collect(),
            ..msg(1, &[], &[])
        };
        let j_high = Message {
            fringe: [11].into_iter().collect(),
            ..msg(2, &[], &[])
        };
        let map: BTreeMap<i32, Message<i32, i32>> = [(10, f1), (11, f2), (1, j_low), (2, j_high)]
            .into_iter()
            .collect();
        let justifications: BTreeSet<_> = [map[&1].clone(), map[&2].clone()].into_iter().collect();
        let latest = latest_fringe(&map, &justifications);
        assert_eq!(latest.len(), 1);
        assert_eq!(latest.iter().next().unwrap().id, 11);
    }
}
