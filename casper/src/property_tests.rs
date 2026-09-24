//! Property tests for the casper layer's laws.
//!
//! Law 26 (shard scope determinism — a shard id is a validated, ordered value) and Laws 27–29 (the
//! coordinator's decision is durable and deterministic: an abort on any leg prevents a commit, a
//! terminal decision is absorbing, and the state always agrees with the votes). Randomized evidence
//! alongside the Lean statements (`Rchain/CrossShard.lean`) and the unit tests in
//! `gateway/ledger.rs` and `conf.rs`.

use proptest::prelude::*;

use rchain_crypto::public_key::PublicKey;
use rchain_shared::refined::ShardId;

use crate::gateway::ledger::{CoordRecord, CoordState, LegRecord, Vote};

fn shard(name: &str) -> ShardId {
    ShardId::try_from(name.to_string()).expect("a valid shard name")
}

/// A record with `n` legs and no votes: the starting point of every vote sequence.
fn record(n: u8, names: &[u8]) -> CoordRecord {
    CoordRecord {
        txn_id: b"txn".to_vec(),
        state: CoordState::Proposed,
        coordinator: PublicKey::new(vec![7u8; 65]),
        legs: names
            .iter()
            .take(usize::from(n))
            .map(|b| LegRecord {
                shard_id: shard(&format!("/s{b}")),
                amount: rchain_shared::refined::NonNegI64::try_from(1).expect("1 is non-negative"),
                to: "dest".to_string(),
            })
            .collect(),
        votes: Vec::new(),
        reason: None,
    }
}

/// The three shapes the id predicate distinguishes, in one generator: empty, non-ASCII, and
/// non-empty ASCII **with** separators.
///
/// The separator arm is deliberately *accepted* by the predicate — it is here to pin that `TryFrom`
/// does not reject a `/` — so the generator is named for the shapes it draws, not for a verdict.
/// (C78, in the fixture: a generator called "bad" whose third arm is in fact accepted is the same
/// defect as a constructor named "validated" that does not validate. The arm is kept as an
/// acceptance case rather than made genuinely bad because the genuinely-bad shapes it could draw —
/// control characters, DEL, non-ASCII — are already the second arm's range, so that repair would
/// delete real coverage.)
fn arb_name_shapes() -> impl Strategy<Value = String> {
    prop_oneof![
        Just(String::new()),
        "[^ -~]{1,4}".prop_map(|s| s),       // non-ASCII
        "/[a-z]{0,3}/{0,2}".prop_map(|s| s), // separator-bearing non-empty ASCII: accepted
    ]
}

proptest! {
    /// **Law 26's validation.** A shard id is accepted exactly when it is non-empty ASCII (the
    /// validate-on-ingress rule added for C4 in the security pass). The check is about what can travel
    /// as a **path segment**, not about printable text: it deliberately **admits control characters**
    /// — NUL, DEL and ESC are ASCII, and the witness below draws them — while the empty name is
    /// refused because a segment must be non-empty. The id is compared for equality on the consensus
    /// path, so what the check buys is that a peer-supplied segment and the id it names are the same
    /// string, not that the name is legible (AUDIT C78's neighbourhood; the model's `validShardId` in
    /// `spec/Rchain/CrossShard.lean:118` is this same pair of tests).
    #[test]
    fn law26_a_shard_id_is_accepted_exactly_when_nonempty_ascii(name in ".{0,8}") {
        prop_assert_eq!(ShardId::try_from(name.clone()).is_ok(), !name.is_empty() && name.is_ascii());
    }

    /// Both verdicts spelled out against the predicate: an empty or non-ASCII name is refused, and
    /// a separator-bearing non-empty ASCII name is accepted.
    #[test]
    fn law26_invalid_shard_names_are_refused(name in arb_name_shapes()) {
        let ok = !name.is_empty() && name.is_ascii();
        prop_assert_eq!(ShardId::try_from(name.clone()).is_ok(), ok, "name {:?}", name);
    }

    /// **Law 26's composition.** `parent.child(name)` nests under the parent: the child's id starts
    /// with the parent's, its parent is the parent, and a root child gets the `/name` form. This is
    /// the ordering the shard checks compare, so a composition that lost a separator would make two
    /// different shards compare equal.
    #[test]
    fn law26_a_child_id_nests_under_its_parent(parent_name in "[a-z]{1,4}", child_name in "[a-z]{1,4}") {
        let root = ShardId::root();
        let parent = root.child(&parent_name).expect("a valid shard name");
        let child = parent.child(&child_name).expect("a valid shard name");

        prop_assert!(parent.is_root() == false);
        prop_assert_eq!(parent.parent(), Some(ShardId::root()));
        prop_assert_eq!(child.parent(), Some(parent.clone()));
        prop_assert!(child.to_string().starts_with(&parent.to_string()));
        prop_assert!(child.to_string().ends_with(&child_name));
        prop_assert_ne!(child, parent.clone(), "a child is a different shard from its parent");

        // The composition refuses what the refinement refuses (C78): a name the id predicate
        // rejects cannot be composed into a child, on either branch.
        prop_assert!(root.child("").is_err());
        prop_assert!(root.child("café").is_err());
        prop_assert!(parent.child("").is_err());
        prop_assert!(parent.child("café").is_err());
    }

    /// **Law 27 (no strict-subset commit).** An `Abort` vote on any leg prevents a commit: however
    /// the remaining votes arrive, a record that has seen an abort is `Aborted` and never
    /// `Committed`. That is the coordinator-side half of "commit on every participant or abort on
    /// every participant".
    ///
    /// The abort goes in **first**, and that ordering is the law's shape, not a convenience: a
    /// `Committed` record is terminal (C1's absorbing rule), so an abort arriving *after* the commit
    /// point is correctly ignored — an abort prevents a commit only while the decision is still open.
    /// The absorbing half is pinned by `law29_a_terminal_record_never_changes_again` and by
    /// `a_commit_is_absorbing` in `gateway/ledger.rs`.
    #[test]
    fn law27_an_abort_vote_prevents_a_later_commit(
        n in 1u8..4,
        order in prop::collection::vec((0u8..4, any::<bool>()), 0..6),
    ) {
        let names = [0u8, 1, 2, 3];
        let mut rec = record(n, &names);
        let first = rec.legs[0].shard_id.clone();
        rec.record_vote(first, Vote::Abort, Some("participant error".to_string()));
        prop_assert_eq!(rec.state, CoordState::Aborted);

        for (leg, ready) in &order {
            let idx = usize::from(*leg) % rec.legs.len();
            let id = rec.legs[idx].shard_id.clone();
            rec.record_vote(id, if *ready { Vote::Ready } else { Vote::Abort }, None);
            prop_assert_eq!(rec.state, CoordState::Aborted, "an abort is never followed by a commit");
            prop_assert!(rec.decision_is_deterministic());
        }
    }

    /// **Law 29 (a decided record is absorbing).** Once the record is terminal, no further vote
    /// changes the state *or* the vote list — the property the coordinator's `record_vote` guard
    /// exists for (a late `Ready` must not resurrect an aborted, already-compensated transaction).
    #[test]
    fn law29_a_terminal_record_never_changes_again(
        n in 1u8..4,
        votes in prop::collection::vec(any::<bool>(), 0..4),
        late in prop::collection::vec((0u8..4, any::<bool>()), 0..4),
    ) {
        let names = [0u8, 1, 2, 3];
        let mut rec = record(n, &names);
        for (i, ready) in votes.iter().enumerate() {
            let idx = i % rec.legs.len();
            let id = rec.legs[idx].shard_id.clone();
            rec.record_vote(id, if *ready { Vote::Ready } else { Vote::Abort }, None);
        }
        if !rec.state.is_terminal() {
            // Make it terminal: every leg Ready commits, one Abort aborts.
            for leg in rec.legs.clone() {
                rec.record_vote(leg.shard_id, Vote::Ready, None);
            }
        }
        prop_assert!(rec.state.is_terminal(), "the record must be terminal by now");

        let (state, votes) = (rec.state, rec.votes.clone());
        for (leg, ready) in &late {
            let idx = usize::from(*leg) % rec.legs.len();
            let id = rec.legs[idx].shard_id.clone();
            rec.record_vote(id, if *ready { Vote::Ready } else { Vote::Abort }, None);
            prop_assert_eq!(rec.state, state, "a terminal state must not move");
            prop_assert_eq!(&rec.votes, &votes, "…and its votes must not either");
        }
    }

    /// **Laws 27/29 (the state always agrees with the votes).** For any vote sequence,
    /// `decision_is_deterministic` holds and the two directions of the biconditional are reachable:
    /// `Committed` exactly when every leg voted `Ready`, and no commit while a leg is silent.
    #[test]
    fn law27_and_law29_the_state_agrees_with_the_votes(
        n in 1u8..4,
        order in prop::collection::vec((0u8..4, any::<bool>()), 0..6),
    ) {
        let names = [0u8, 1, 2, 3];
        let mut rec = record(n, &names);
        rec.legs.truncate(usize::from(n));
        for (leg, ready) in &order {
            let idx = usize::from(*leg) % rec.legs.len();
            let id = rec.legs[idx].shard_id.clone();
            rec.record_vote(id, if *ready { Vote::Ready } else { Vote::Abort }, None);

            prop_assert!(rec.decision_is_deterministic());
            let all_ready = rec.votes.len() == rec.legs.len()
                && !rec.legs.is_empty()
                && rec.votes.iter().all(|(_, v)| *v == Vote::Ready);
            prop_assert_eq!(rec.state == CoordState::Committed, all_ready);
            if rec.votes.iter().any(|(_, v)| *v == Vote::Abort) {
                prop_assert_eq!(rec.state, CoordState::Aborted, "an abort is never a commit");
            }
            // The accessor agrees with the vote list it exposes.
            for (id, vote) in &rec.votes {
                prop_assert_eq!(rec.vote_for(id), Some(*vote));
            }
        }
    }

    /// A record with no legs can never commit (`decision_is_deterministic` requires a non-empty leg
    /// list) — the degenerate coordinator case, pinned so a `!self.legs.is_empty()` guard cannot be
    /// dropped silently.
    #[test]
    fn law27_a_legless_record_cannot_commit(unit in Just(())) {
        prop_assert_eq!(unit, ());
        let rec = record(0, &[]);
        prop_assert_eq!(rec.state, CoordState::Proposed);
        prop_assert!(rec.decision_is_deterministic());
        assert!(rec.decision_is_deterministic() != (rec.state == CoordState::Committed));
    }
}
