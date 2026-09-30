//! Per-network DAG message state.
//!
//! Mirrors `block-storage/src/main/scala/coop/rchain/blockstorage/dag/DagMessageState.scala`.

use std::collections::{BTreeMap, BTreeSet};
use std::hash::Hash;
use std::sync::Arc;

use rchain_shared::refined::{BlockHeight, NonNegI64, SeqNum};

use super::finalizer::{Finalizer, Message};
use super::message_map;
use crate::errors::StorageError;

/// The per-sender latest messages and the full message map.
///
/// Invariant: `latest_msgs` is keyed by sender, so the one-message-per-sender invariant is
/// structural (a map cannot hold two entries for one sender); every entry is also present in
/// `msg_map`. `insert_msg_mut` enforces the subset invariant with a `debug_assert!`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DagMessageState<M, S> {
    pub latest_msgs: BTreeMap<S, Message<M, S>>,
    pub msg_map: BTreeMap<M, Message<M, S>>,
    /// **The parent set a new block justifies** — the `latest_msgs` as of the last round boundary, and
    /// not `latest_msgs` itself.
    ///
    /// This is not a heuristic: the fringe gate refuses `latest_msgs` outright. `calculate_fringe`
    /// counts a candidate only if the whole live partition has seen every next-layer message *through
    /// messages beyond that layer* (`calculate_next_fringe_support_map` reads `parents ∖ next_layer`),
    /// and a parent set that is every sender's newest message has no such remainder — the head of each
    /// round has an empty one and the later movers have seen a prefix. Measured in
    /// `casper/tests/finalization.rs`: the chain built from `latest_msgs` never finalises, and the same
    /// chain with a per-round snapshot finalises at a constant 12-height lag (48/60, 168/180, 348/360).
    ///
    /// A **round boundary** is the first message at which every sender of the live weight set has a
    /// message above the previous boundary. That is a function of the DAG alone — the bond map rides on
    /// every message — so every node computes the same boundary from the same history, and a validator
    /// that stops proposing drops out of the live set on the usual `LIVENESS_WINDOW` and stops holding
    /// the boundary back.
    pub round_parents: BTreeMap<S, Message<M, S>>,
    /// The height `round_parents` was taken at — the last round boundary. Zero before the first.
    pub round_height: BlockHeight,
}

impl<M, S> DagMessageState<M, S>
where
    M: Ord + Clone + Eq + Hash,
    S: Ord + Clone + Eq + Hash,
{
    pub fn empty() -> Self {
        Self::from_parts(BTreeMap::new(), BTreeMap::new())
    }

    /// Build a state from its two parts, with the round boundary **derived** rather than left for the
    /// caller to get right — the fields are public, and a state whose `round_parents` disagrees with its
    /// `latest_msgs` would hand a proposer parents the gate cannot use.
    pub fn from_parts(
        latest_msgs: BTreeMap<S, Message<M, S>>,
        msg_map: BTreeMap<M, Message<M, S>>,
    ) -> Self {
        let mut state = Self {
            latest_msgs,
            msg_map,
            round_parents: BTreeMap::new(),
            round_height: BlockHeight::zero(),
        };
        state.advance_round();
        state
    }

    /// Take a new round boundary if the current tip closes one.
    ///
    /// Called after every insert ([`Self::insert_msg_mut`]), so the boundary is always current when a
    /// proposer asks. The bonds come off any message (they ride on all of them).
    ///
    /// **A bonded sender closes the boundary when it has a message above the last one — and it stops
    /// holding it back once [`super::liveness::LIVENESS_WINDOW`] heights have passed without one.** The
    /// two halves are both needed and neither alone is right:
    ///
    /// - *waiting for a sender that has not spoken yet* is what makes a round a round. Closing on the
    ///   first message of a fresh chain puts `round_parents` at that single message, and the fringe then
    ///   refuses with `150 of 250` — the snapshot has to be a **set**, not a stack;
    /// - *waiting for ever* is #70 again: a validator that never comes back would freeze the parent set,
    ///   so a sender silent past the window is retired and the boundary closes without it. That is the
    ///   same window and the same rule the fringe gate uses, which is the point of reusing it.
    fn advance_round(&mut self) {
        let Some(tip) = self.latest_msgs.values().map(|m| m.height).max() else {
            return;
        };
        if self.round_parents.is_empty() {
            self.round_parents = self.latest_msgs.clone();
            self.round_height = tip;
            return;
        }
        if tip <= self.round_height {
            return;
        }
        let Some(bonds) = self
            .latest_msgs
            .values()
            .next()
            .map(|m| m.bonds_map.clone())
        else {
            return;
        };
        let window = super::liveness::LIVENESS_WINDOW;
        let closed = bonds.keys().all(|s| {
            match self.latest_msgs.get(s).map(|m| m.height) {
                // Spoken and past the boundary: this sender is done for this round.
                Some(height) if height > self.round_height => true,
                // Spoken, but not past it: the round waits for it — unless the window has retired it.
                Some(height) => super::liveness::heights_behind(tip, height) > window,
                // Never spoken: the round waits, on the same clock the window runs.
                None => i64::from(tip) - i64::from(self.round_height) > window,
            }
        });
        if closed {
            self.round_parents = self.latest_msgs.clone();
            self.round_height = tip;
        }
    }

    /// **The parent set for a new block by `sender`** — [`Self::round_parents`], with **the sender's own
    /// entry replaced by its newest message**.
    ///
    /// The substitution is not a refinement, it is what keeps the chain a chain. The block creator derives
    /// both the new block's `block_num` and its `seq_num` **from this set** (`block_creator.rs:58-75`), so a
    /// parent set that omits the proposer's own newest message makes a second proposal in the same round
    /// carry the same `(sender, seq_num)` as the first — and the DAG refuses it, correctly, as an
    /// equivocation:
    ///
    /// ```text
    /// Self-created block #93 (seq 92) failed validation: failed to insert block into DAG:
    /// equivocation detected: sender produced two blocks with the same sequence number
    /// ```
    ///
    /// That is measured, not anticipated: it is what a devnet run against the first version of this
    /// function logged, and it is why the fixture in `casper/tests/finalization.rs` derives `sender_seq`
    /// from the parent set the way the block creator does. The first block of a round is unaffected — the
    /// sender's newest *is* the snapshot's entry for it — so the cross-sender snapshot, which is what the
    /// fringe gate needs, is untouched.
    pub fn parents_for_new_block(&self, sender: &S) -> BTreeSet<Message<M, S>> {
        if self.round_parents.is_empty() {
            return self.latest_msgs.values().cloned().collect();
        }
        let mut parents = self.round_parents.clone();
        if let Some(mine) = self.latest_msgs.get(sender) {
            parents.insert(sender.clone(), mine.clone());
        }
        parents.values().cloned().collect()
    }

    /// Create a new message, generating its finalization fringe.
    #[allow(clippy::too_many_arguments)]
    pub fn create_message(
        &self,
        id: M,
        height: BlockHeight,
        sender: S,
        sender_seq: SeqNum,
        fin_bonds_map: BTreeMap<S, NonNegI64>,
        justifications: &BTreeSet<Message<M, S>>,
    ) -> Message<M, S> {
        let finalizer = Finalizer::new(&self.msg_map);
        // Through the liveness rule, not the raw gate: the creator writes this fringe into the block,
        // and a validator that derived a different one would refuse the block it was handed — so both
        // sides must come through here (see `super::liveness`).
        let (parent_fringe, new_fringe_opt) =
            super::liveness::calculate_finalization(&finalizer, justifications, &fin_bonds_map);

        let new_fringe = new_fringe_opt.unwrap_or(parent_fringe);
        let new_fringe_ids: BTreeSet<M> = new_fringe.iter().map(|m| m.id.clone()).collect();

        let mut new_seen: BTreeSet<M> = justifications
            .iter()
            .flat_map(|j| j.seen.iter().cloned())
            .collect();
        new_seen.insert(id.clone());

        let justification_keys: BTreeSet<M> = justifications.iter().map(|j| j.id.clone()).collect();

        Message {
            id,
            height,
            sender,
            sender_seq,
            bonds_map: fin_bonds_map,
            parents: justification_keys,
            fringe: new_fringe_ids,
            seen: Arc::new(new_seen),
        }
    }

    /// Insert a message **in place** (no-op if its id is already present). Only a higher
    /// `sender_seq` replaces the sender's latest message (the Law 15 monotonicity invariant).
    ///
    /// The in-place form is what a *bulk* caller must use. The Scala's `insertMsg` returns a state
    /// built with `Map + (k -> v)` over an **immutable** map, whose structural sharing makes that
    /// O(log N); the Rust `BTreeMap` has no structural sharing, so a persistent-shaped
    /// implementation that clones the map per insert costs O(N) per call — and O(N²) when each
    /// entry's own `seen` set is deep-copied with it. Rebuilding a stored chain through
    /// [`Self::insert_msg`] that way was Θ(N³): a 5,844-block devnet restart spent longer than the
    /// devnet's own 120 s healthcheck inside `BlockDagKeyValueStorage::create` and looked like a
    /// hang (AUDIT C55).
    pub fn insert_msg_mut(&mut self, msg: &Message<M, S>) {
        if self.msg_map.contains_key(&msg.id) {
            return;
        }
        let replace = self
            .latest_msgs
            .get(&msg.sender)
            .map(|cur| msg.sender_seq > cur.sender_seq)
            .unwrap_or(true);
        self.msg_map.insert(msg.id.clone(), msg.clone());
        if replace {
            self.latest_msgs.insert(msg.sender.clone(), msg.clone());
        }

        debug_assert!(
            self.latest_msgs
                .values()
                .all(|m| self.msg_map.contains_key(&m.id)),
            "latest_msgs must be a subset of msg_map"
        );
        // The boundary is recomputed here, after the insert, so a proposer reading `round_parents`
        // cannot see a stale one.
        self.advance_round();
    }

    /// [`Self::insert_msg_mut`] as a value: one clone, then the insert. Kept for the callers that
    /// hand the next state on (the per-block path and `create_msg_and_update_sender`), so the
    /// acceptance rule lives in exactly one place.
    pub fn insert_msg(&self, msg: &Message<M, S>) -> Self {
        let mut next = self.clone();
        next.insert_msg_mut(msg);
        next
    }

    /// [`Self::insert_msg_without_latest`] in place.
    ///
    /// Used for validation-failed blocks: they must be recorded in the map (so
    /// `neglectedInvalidBlock` and justification-regression can see them) but must not become a
    /// proposer's parent, otherwise a single failed block would wedge block production (H-2).
    pub fn insert_msg_without_latest_mut(&mut self, msg: &Message<M, S>) {
        if self.msg_map.contains_key(&msg.id) {
            return;
        }
        self.msg_map.insert(msg.id.clone(), msg.clone());
    }

    /// [`Self::insert_msg_without_latest_mut`] as a value: one clone, then the insert.
    pub fn insert_msg_without_latest(&self, msg: &Message<M, S>) -> Self {
        let mut next = self.clone();
        next.insert_msg_without_latest_mut(msg);
        next
    }

    /// Create a new message for `creator` and insert it.
    pub fn create_msg_and_update_sender<F>(
        &self,
        creator: &S,
        gen_msg_id: F,
    ) -> Result<(Self, Message<M, S>), StorageError>
    where
        F: FnOnce(&S, BlockHeight) -> M,
    {
        let max_height = self
            .latest_msgs
            .values()
            .map(|m| m.height)
            .max()
            .ok_or(StorageError::EmptyLatestMessages)?;
        let new_height = max_height + NonNegI64::one();
        let seq_num = self
            .latest_msgs
            .get(creator)
            .map(|m| m.sender_seq)
            .unwrap_or_else(SeqNum::zero);
        let new_seq_num = seq_num + NonNegI64::one();
        let justifications: BTreeSet<Message<M, S>> = self.parents_for_new_block(creator);
        let bonds_map = self
            .latest_msgs
            .values()
            .next()
            .ok_or(StorageError::EmptyLatestMessages)?
            .bonds_map
            .clone();

        let msg_id = gen_msg_id(creator, new_height);
        let new_msg = self.create_message(
            msg_id,
            new_height,
            creator.clone(),
            new_seq_num,
            bonds_map,
            &justifications,
        );
        Ok((self.insert_msg(&new_msg), new_msg))
    }

    /// The latest fringe, using the latest messages as parents.
    pub fn latest_fringe(&self) -> BTreeSet<Message<M, S>> {
        let latest: BTreeSet<Message<M, S>> = self.latest_msgs.values().cloned().collect();
        message_map::latest_fringe(&self.msg_map, &latest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn msg(id: i32, sender: i32, sender_seq: i64) -> Message<i32, i32> {
        Message {
            id,
            height: BlockHeight::zero(),
            sender,
            sender_seq: SeqNum::try_from(sender_seq).unwrap(),
            bonds_map: BTreeMap::new(),
            parents: BTreeSet::new(),
            fringe: BTreeSet::new(),
            seen: Arc::new([id].into_iter().collect()),
        }
    }

    #[test]
    fn law15_insert_msg_is_monotone() {
        let state: DagMessageState<i32, i32> = DagMessageState::empty();
        let genesis = msg(0, 0, 0);
        let s1 = state.insert_msg(&genesis);
        assert_eq!(s1.latest_msgs.get(&0), Some(&genesis));

        // Same sender_seq does NOT replace the latest (monotone, no regression).
        let stale = msg(1, 0, 0);
        let s2 = s1.insert_msg(&stale);
        assert_eq!(s2.latest_msgs.get(&0), Some(&genesis));

        // A higher sender_seq replaces the latest.
        let newer = msg(2, 0, 1);
        let s3 = s2.insert_msg(&newer);
        assert_eq!(s3.latest_msgs.get(&0), Some(&newer));

        // The message map still holds every message.
        assert_eq!(s3.msg_map.len(), 3);
    }

    #[test]
    fn insert_msg_is_idempotent() {
        let state: DagMessageState<i32, i32> = DagMessageState::empty();
        let genesis = msg(0, 0, 0);
        let s1 = state.insert_msg(&genesis);
        let s2 = s1.insert_msg(&genesis);
        assert_eq!(s1, s2);
    }

    #[test]
    fn create_message_seen_is_parents_seen_plus_id() {
        let state: DagMessageState<i32, i32> = DagMessageState::empty();
        let genesis = msg(0, 0, 0);
        let justifications: BTreeSet<_> = [genesis].into_iter().collect();
        let new_msg = state.create_message(
            1,
            BlockHeight::try_from(1).unwrap(),
            1,
            SeqNum::try_from(1).unwrap(),
            BTreeMap::new(),
            &justifications,
        );
        assert_eq!(*new_msg.seen, [0, 1].into_iter().collect());
        assert_eq!(new_msg.parents, [0].into_iter().collect());
    }
}
