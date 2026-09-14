//! Pure logic for merging event logs (conflict/dependency analysis).
//!
//! Mirrors `rspace/src/main/scala/coop/rchain/rspace/merger/EventLogMergingLogic.scala`.

use std::collections::BTreeSet;

use rchain_crypto::hash::blake2b256_hash::Blake2b256Hash;

use crate::merger::event_log_index::EventLogIndex;
use crate::trace::event::{Consume, Produce};

fn union<T: Ord + Clone>(a: &BTreeSet<T>, b: &BTreeSet<T>) -> BTreeSet<T> {
    a.union(b).cloned().collect()
}

fn diff<T: Ord + Clone>(a: &BTreeSet<T>, b: &BTreeSet<T>) -> BTreeSet<T> {
    a.difference(b).cloned().collect()
}

fn intersect<T: Ord + Clone>(a: &BTreeSet<T>, b: &BTreeSet<T>) -> BTreeSet<T> {
    a.intersection(b).cloned().collect()
}

/// Produces created inside the event log (port of `producesCreated`).
pub fn produces_created(e: &EventLogIndex) -> BTreeSet<Produce> {
    diff(
        &union(&e.produces_linear, &e.produces_persistent),
        &e.produces_copied_by_peek,
    )
}

/// Consumes created inside the event log (port of `consumesCreated`).
pub fn consumes_created(e: &EventLogIndex) -> BTreeSet<Consume> {
    union(&e.consumes_linear_and_peeks, &e.consumes_persistent)
}

/// Produces created and not destroyed inside the event log (port of
/// `producesCreatedAndNotDestroyed`).
pub fn produces_created_and_not_destroyed(e: &EventLogIndex) -> BTreeSet<Produce> {
    diff(
        &union(
            &diff(&e.produces_linear, &e.produces_consumed),
            &e.produces_persistent,
        ),
        &e.produces_copied_by_peek,
    )
}

/// Consumes created and not destroyed inside the event log (port of
/// `consumesCreatedAndNotDestroyed`).
pub fn consumes_created_and_not_destroyed(e: &EventLogIndex) -> BTreeSet<Consume> {
    union(
        &diff(&e.consumes_linear_and_peeks, &e.consumes_produced),
        &e.consumes_persistent,
    )
}

/// Produces affected by the event log (port of `producesAffected`).
pub fn produces_affected(e: &EventLogIndex) -> BTreeSet<Produce> {
    let created = produces_created(e);
    let external_destroyed: BTreeSet<Produce> = diff(&e.produces_consumed, &created)
        .into_iter()
        .filter(|p| !p.persistent)
        .collect();
    union(&produces_created_and_not_destroyed(e), &external_destroyed)
}

/// Consumes affected by the event log (port of `consumesAffected`).
pub fn consumes_affected(e: &EventLogIndex) -> BTreeSet<Consume> {
    let created = consumes_created(e);
    let external_destroyed: BTreeSet<Consume> = diff(&e.consumes_produced, &created)
        .into_iter()
        .filter(|c| !c.persistent)
        .collect();
    union(&consumes_created_and_not_destroyed(e), &external_destroyed)
}

/// Combine the copied-by-peek sets of two indices (port of `combineProducesCopiedByPeek`).
pub fn combine_produces_copied_by_peek(x: &EventLogIndex, y: &EventLogIndex) -> BTreeSet<Produce> {
    let copied = union(&x.produces_copied_by_peek, &y.produces_copied_by_peek);
    let created = union(&produces_created(x), &produces_created(y));
    diff(&copied, &created)
}

/// Whether `target` depends on `source` (port of `depends`).
pub fn depends(target: &EventLogIndex, source: &EventLogIndex) -> bool {
    let produces_source = diff(
        &produces_created_and_not_destroyed(source),
        &source.produces_mergeable,
    );
    let produces_target = diff(&target.produces_consumed, &source.produces_mergeable);
    let consumes_source = consumes_created_and_not_destroyed(source);
    let consumes_target = &target.consumes_produced;

    !intersect(&produces_source, &produces_target).is_empty()
        || !intersect(&consumes_source, consumes_target).is_empty()
}

/// Whether two event logs conflict (port of `areConflicting`).
pub fn are_conflicting(a: &EventLogIndex, b: &EventLogIndex) -> bool {
    !conflicts(a, b).is_empty()
}

/// The channels conflicting between two event logs (port of `conflicts`).
pub fn conflicts(a: &EventLogIndex, b: &EventLogIndex) -> Vec<Blake2b256Hash> {
    // Check #1: the same non-persistent produce/consume destroyed in both branches.
    let shared_consumes = intersect(&a.consumes_produced, &b.consumes_produced);
    let mergeable_consumes = intersect(&a.consumes_mergeable, &b.consumes_mergeable);
    let consume_races: Vec<Consume> = diff(&shared_consumes, &mergeable_consumes)
        .into_iter()
        .filter(|c| !c.persistent)
        .collect();

    let shared_produces = intersect(&a.produces_consumed, &b.produces_consumed);
    let mergeable_produces = intersect(&a.produces_mergeable, &b.produces_mergeable);
    let produce_races: Vec<Produce> = diff(&shared_produces, &mergeable_produces)
        .into_iter()
        .filter(|p| !p.persistent)
        .collect();

    let mut races: Vec<Blake2b256Hash> = Vec::new();
    for c in consume_races {
        races.extend(c.channels_hashes.iter().copied());
    }
    for p in produce_races {
        races.push(p.channels_hash);
    }

    // Check #2: potential COMMs between creates in one branch and consumes in the other.
    let match_found = |consume: &Consume, produce: &Produce| {
        consume.channels_hashes.contains(&produce.channels_hash)
    };
    let check = |left: &EventLogIndex, right: &EventLogIndex| -> Vec<Blake2b256Hash> {
        let produces = produces_created_and_not_destroyed(left);
        let consumes = consumes_created_and_not_destroyed(right);
        let mut out = Vec::new();
        for p in &produces {
            for c in &consumes {
                if match_found(c, p) {
                    out.push(p.channels_hash);
                }
            }
        }
        out
    };
    races.extend(check(a, b));
    races.extend(check(b, a));

    // Check #3: produces touching base joins.
    for p in a
        .produces_touching_base_joins
        .union(&b.produces_touching_base_joins)
    {
        races.push(p.channels_hash);
    }

    races
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Event values are distinguishable by their inputs, which is all these set operations need
    /// (the `String` types are the ones `Serialize` is implemented for here).
    fn p(byte: u8, persistent: bool) -> Produce {
        Produce::apply(
            &format!("channel-{byte}"),
            &format!("datum-{byte}"),
            persistent,
        )
    }

    fn c(byte: u8, persistent: bool) -> Consume {
        Consume::apply(
            &[format!("channel-{byte}")],
            &[format!("pattern-{byte}")],
            &format!("continuation-{byte}"),
            persistent,
        )
    }

    fn index(f: impl FnOnce(&mut EventLogIndex)) -> EventLogIndex {
        let mut e = EventLogIndex::default();
        f(&mut e);
        e
    }

    fn set(items: &[Produce]) -> BTreeSet<Produce> {
        items.iter().cloned().collect()
    }

    fn cset(items: &[Consume]) -> BTreeSet<Consume> {
        items.iter().cloned().collect()
    }

    /// `producesCreated` is `(linear ∪ persistent) − copiedByPeek`: a peek *copies* a datum rather
    /// than creating one, so a datum seen only through a peek is not this log's creation.
    #[test]
    fn produces_created_unions_the_sources_minus_the_peek_copies() {
        let e = index(|e| {
            e.produces_linear = set(&[p(1, false), p(2, true)]);
            e.produces_persistent = set(&[p(3, true)]);
            e.produces_copied_by_peek = set(&[p(2, true)]);
        });
        assert_eq!(produces_created(&e), set(&[p(1, false), p(3, true)]));
    }

    /// `producesCreatedAndNotDestroyed` additionally subtracts what the log consumed — the linear
    /// ones only; a *persistent* produce is never destroyed, which is why it is re-union'd in.
    #[test]
    fn produces_created_and_not_destroyed_keeps_the_persistent_ones() {
        let e = index(|e| {
            e.produces_linear = set(&[p(1, false), p(2, false)]);
            e.produces_persistent = set(&[p(3, true)]);
            e.produces_consumed = set(&[p(2, false)]);
        });
        assert_eq!(
            produces_created_and_not_destroyed(&e),
            set(&[p(1, false), p(3, true)]),
            "the consumed linear produce is gone; the persistent one stays"
        );
    }

    /// The same shape for consumes: `consumesCreatedAndNotDestroyed` drops what this log *produced*
    /// (a produce in the log can resurrect a persistent consume) and keeps the persistent ones.
    #[test]
    fn consumes_created_and_not_destroyed_keeps_the_persistent_ones() {
        let e = index(|e| {
            e.consumes_linear_and_peeks = cset(&[c(1, false), c(2, false)]);
            e.consumes_persistent = cset(&[c(3, true)]);
            e.consumes_produced = cset(&[c(2, false)]);
        });
        assert_eq!(
            consumes_created_and_not_destroyed(&e),
            cset(&[c(1, false), c(3, true)])
        );
    }

    /// **The conflict rule** (Law 9's merge precondition): two logs conflict when they destroy the
    /// *same*, non-persistent produce/consume — unless both sides recorded it as mergeable, in which
    /// case the merge handles it. Persistent events are never races (they are not destroyed).
    #[test]
    fn a_shared_destroyed_produce_conflicts_unless_it_is_mergeable_on_both_sides() {
        let shared = p(1, false);
        let a = index(|e| e.produces_consumed = set(&[shared.clone()]));
        let b = index(|e| e.produces_consumed = set(&[shared.clone()]));
        assert!(are_conflicting(&a, &b), "both destroy the same produce");

        // The same produce, but recorded as mergeable on *both* sides: no conflict.
        let a2 = index(|e| {
            e.produces_consumed = set(&[shared.clone()]);
            e.produces_mergeable = set(&[shared.clone()]);
        });
        let b2 = index(|e| {
            e.produces_consumed = set(&[shared.clone()]);
            e.produces_mergeable = set(&[shared.clone()]);
        });
        assert!(
            !are_conflicting(&a2, &b2),
            "a produce mergeable on both sides is not a race"
        );

        // Mergeable on only one side is still a conflict: the other side cannot merge it.
        assert!(are_conflicting(&a2, &b));
    }

    /// A persistent produce destroyed on both sides is not a race (a persistent datum is copied,
    /// never removed), and disjoint logs do not conflict at all — the merge monoid's identity.
    #[test]
    fn persistent_and_disjoint_events_do_not_conflict() {
        let persistent = p(1, true);
        let a = index(|e| e.produces_consumed = set(&[persistent.clone()]));
        let b = index(|e| e.produces_consumed = set(&[persistent]));
        assert!(!are_conflicting(&a, &b), "persistent events are not races");

        let left = index(|e| e.produces_consumed = set(&[p(1, false)]));
        let right = index(|e| e.produces_consumed = set(&[p(2, false)]));
        assert!(!are_conflicting(&left, &right), "different channels");
        assert!(!are_conflicting(&left, &EventLogIndex::default()));
        assert!(
            !are_conflicting(&EventLogIndex::default(), &EventLogIndex::default()),
            "the empty log is the identity"
        );
    }

    /// `conflicts` reports the offending **channel hashes** and is symmetric: the merge asks "do
    /// these two conflict", which cannot depend on the argument order.
    #[test]
    fn the_reported_conflicts_are_channel_hashes_and_are_symmetric() {
        let shared = p(1, false);
        let a = index(|e| e.produces_consumed = set(&[shared.clone()]));
        let b = index(|e| e.produces_consumed = set(&[shared.clone()]));

        let ab = conflicts(&a, &b);
        assert_eq!(ab, vec![shared.channels_hash]);
        assert_eq!(ab, conflicts(&b, &a), "the conflict set is symmetric");
    }

    /// A consume race is the symmetric case on the consume side, including the mergeable carve-out.
    #[test]
    fn a_shared_destroyed_consume_conflicts_unless_mergeable_on_both_sides() {
        let shared = c(1, false);
        let a = index(|e| e.consumes_produced = cset(&[shared.clone()]));
        let b = index(|e| e.consumes_produced = cset(&[shared.clone()]));
        assert!(are_conflicting(&a, &b));

        let mergeable = |c: Consume| {
            index(|e| {
                e.consumes_produced = cset(&[c.clone()]);
                e.consumes_mergeable = cset(&[c]);
            })
        };
        assert!(!are_conflicting(
            &mergeable(shared.clone()),
            &mergeable(shared)
        ));
    }

    /// **Dependency** is directional: `depends(target, source)` is true when the target consumed
    /// something the source created (or produced a consume the source created) — the edge that
    /// forces the merge to order one log before the other.
    #[test]
    fn depends_is_directional_and_ignores_mergeable_events() {
        let created = p(1, false);
        let source = index(|e| e.produces_linear = set(&[created.clone()]));
        let target = index(|e| e.produces_consumed = set(&[created.clone()]));

        assert!(
            depends(&target, &source),
            "the target consumed what the source created"
        );
        assert!(
            !depends(&source, &target),
            "the source consumed nothing the target created"
        );

        // If the source marks its creation mergeable, the dependency is not a reason to order them.
        let mergeable_source = index(|e| {
            e.produces_linear = set(&[created.clone()]);
            e.produces_mergeable = set(&[created]);
        });
        assert!(
            !depends(&target, &mergeable_source),
            "a mergeable produce cannot create a dependency"
        );
    }

    /// The consume-side dependency: the target *produced* a consume the source created.
    #[test]
    fn a_consume_created_by_the_source_and_produced_by_the_target_is_a_dependency() {
        let created = c(1, false);
        let source = index(|e| e.consumes_linear_and_peeks = cset(&[created.clone()]));
        let target = index(|e| e.consumes_produced = cset(&[created]));
        assert!(depends(&target, &source));
    }

    /// `combineProducesCopiedByPeek` unions the two peek-copy sets and drops what either log
    /// **created** — a datum one branch created and the other merely peeked is a creation, not a
    /// copy, and the merged index must not double-count it as one.
    #[test]
    fn combining_peek_copies_drops_what_either_log_created() {
        // x created `1` and never peeked it.
        let x = index(|e| e.produces_linear = set(&[p(1, true)]));
        // y peeked `1` (which x created) and `2` (which nobody created here).
        let y = index(|e| e.produces_copied_by_peek = set(&[p(1, true), p(2, true)]));

        assert_eq!(
            combine_produces_copied_by_peek(&x, &y),
            set(&[p(2, true)]),
            "`1` is x's creation (y only peeked it); `2` is a pure peek copy"
        );
        assert_eq!(
            combine_produces_copied_by_peek(&y, &x),
            set(&[p(2, true)]),
            "the combination is symmetric"
        );
    }
}
