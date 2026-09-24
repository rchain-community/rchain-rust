//! Per-network DAG message state.
//!
//! Mirrors `block-storage/src/main/scala/coop/rchain/blockstorage/dag/DagMessageState.scala`.

use std::collections::{BTreeMap, BTreeSet};
use std::hash::Hash;

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
}

impl<M, S> DagMessageState<M, S>
where
    M: Ord + Clone + Eq + Hash,
    S: Ord + Clone + Eq + Hash,
{
    pub fn empty() -> Self {
        Self {
            latest_msgs: BTreeMap::new(),
            msg_map: BTreeMap::new(),
        }
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
        let (parent_fringe, new_fringe_opt) =
            finalizer.calculate_finalization(justifications, &fin_bonds_map);

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
            seen: new_seen,
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
        let justifications: BTreeSet<Message<M, S>> = self.latest_msgs.values().cloned().collect();
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

    fn msg(id: i32, sender: i32, sender_seq: i64) -> Message<i32, i32> {
        Message {
            id,
            height: BlockHeight::zero(),
            sender,
            sender_seq: SeqNum::try_from(sender_seq).unwrap(),
            bonds_map: BTreeMap::new(),
            parents: BTreeSet::new(),
            fringe: BTreeSet::new(),
            seen: [id].into_iter().collect(),
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
        assert_eq!(new_msg.seen, [0, 1].into_iter().collect());
        assert_eq!(new_msg.parents, [0].into_iter().collect());
    }
}
