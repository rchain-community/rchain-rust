//! The scheduled tuple-space capability of Laws 20–22 (`spec/Rchain/Scheduler.lean`): the
//! claim-queue-aware `produce_at`/`consume_at` entry points the relaxed effect scheduler drives.
//!
//! The relaxed arm of the rholang reducer holds a
//! [`ChannelClaimQueue`](crate::concurrent::channel_queue::ChannelClaimQueue) guard over the
//! effect's static footprint, waits for the head lease, and then performs the op *at* its DFS path
//! through these methods. They return a [`ReleaseToken`] the caller drops only after enqueueing the
//! continuation's next-step effects (the queue-level continuation-prepend of Law 20).
//!
//! The default implementations of the
//! [`Tuplespace`](crate::tuple_space::Tuplespace) methods these back are plain produce/consume plus
//! `ReleaseToken::detached()` — that is how `ReplayRSpace`, `ReportingRspace` and the test mocks
//! "don't implement" scheduling: they never hold claims, and their ops are the plain sequential
//! ones.
//!
//! [`RSpace`](crate::rspace::RSpace) overrides them. Its produce splits the plain `locked_produce`
//! flow (`rspace.rs:370`) into two phases because the relaxed reducer must release the rspace locks
//! *between* them:
//!
//! * **Phase one** (trigger-channel lock only): log the produce, read the join groups, extract a
//!   candidate. No match → store the datum and finish (one lock, no phase two). Match → return the
//!   matched join channels WITHOUT committing, so the reducer can `claim_more` them and re-wait at
//!   head while parked on the claim queue (risk R3 of the channel-scheduler plan: deliberate
//!   hold-and-wait across the trigger + join channels).
//! * **Phase two** ([`Tuplespace::commit_produce`](crate::tuple_space::Tuplespace::commit_produce)):
//!   re-acquire the two-step lock over the trigger plus *all current* join channels and
//!   **re-validate** — the candidate may have been consumed by a cross-channel op in the gap, so
//!   the commit re-extracts under the full lock set and falls back to storing the datum. This is
//!   what makes the relaxed interleaving sound: reads in phase one are advisory; the commit
//!   re-checks atomically.
//!
//! The consume needs no split: its full static source set is claimed up front and the plain
//! `locked_consume` flow (already one two-step acquisition over every channel hash) commits it.

use std::collections::BTreeSet;

use rchain_shared::serialize::Serialize;

use crate::hashing::stable_hash_provider::hash_channel;
use crate::internal::Datum;
use crate::rspace::{MaybeActionResult, RSpace};
use crate::trace::event::{Consume, Produce};

/// The completion token of a scheduled op: the relaxed reducer drops it only after enqueueing the
/// continuation's next-step effects, so the token's scope *is* the claim's scope. `detached()` is
/// the token form the non-scheduling default implementations return, whose ops complete inline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReleaseToken(());

impl ReleaseToken {
    /// The token returned by the plain (non-scheduling) default `produce_at`/`consume_at`.
    pub fn detached() -> Self {
        ReleaseToken(())
    }
}

/// The produce between its two phases: what phase two needs to re-validate and commit.
#[derive(Debug, Clone)]
pub struct PendingProduce<C, A> {
    /// The trigger channel the produce was made on (phase one's matched channel set contains it,
    /// but the phase-two lock reads the join groups off it again).
    pub trigger: C,
    /// The in-flight datum (not yet stored; phase two re-extracts with it prepended).
    pub data: A,
    pub persist: bool,
}

/// The result of `produce_at`: either the op committed inline (`phase_two: None`, the datum was
/// stored and `result` is final), or the matched join channels are returned for `claim_more` and
/// the commit is deferred to `commit_produce`.
#[derive(Debug, Clone)]
pub struct ScheduledProduce<C, P, A, K> {
    /// The matched continuation's channel set (Law 20's phase-two claim set). Empty when the datum
    /// was stored with no match.
    pub joins: Vec<C>,
    /// Final when `phase_two` is `None`; a placeholder otherwise.
    pub result: MaybeActionResult<C, P, A, K>,
    /// `Some` when the commit is deferred to `commit_produce`.
    pub phase_two: Option<PendingProduce<C, A>>,
    /// Dropped by the caller after enqueueing the continuation's next-step effects.
    pub release: ReleaseToken,
}

/// The result of `consume_at` (no split: the full static source set was claimed up front).
#[derive(Debug, Clone)]
pub struct ScheduledConsume<C, P, A, K> {
    pub result: MaybeActionResult<C, P, A, K>,
    /// Dropped by the caller after enqueueing the continuation's next-step effects.
    pub release: ReleaseToken,
}

impl<C, P, A, K> RSpace<C, P, A, K>
where
    C: Ord + Clone + Serialize<C> + Send + Sync + 'static,
    P: Clone + Serialize<P> + Send + Sync + 'static,
    A: Clone + Serialize<A> + Send + Sync + 'static,
    K: Clone + Serialize<K> + Send + Sync + 'static,
{
    /// Phase one of the scheduled produce (see the module docs): commit under the trigger-channel
    /// lock only. A match defers the commit and returns the matched join channels; no match stores
    /// the datum inline.
    pub async fn scheduled_produce_at(
        &self,
        _path: Vec<u16>,
        channel: C,
        data: A,
        persist: bool,
    ) -> std::result::Result<ScheduledProduce<C, P, A, K>, crate::errors::RSpaceError> {
        let produce_ref = Produce::apply(&channel, &data, persist);
        let hash = hash_channel(&channel);
        let thunk = {
            let channel = channel.clone();
            let data = data.clone();
            async move {
                let grouped_channels = self.current_store().get_joins(&channel).await?;
                // Logged exactly once, here: phase two re-extracts but must not re-log (the event
                // log order is the produce's commit-start, as in `locked_produce`).
                self.log_produce(produce_ref.clone(), persist);
                let datum = Datum {
                    a: data.clone(),
                    persist,
                    source: produce_ref.clone(),
                };
                match self
                    .extract_produce_candidate(&grouped_channels, &channel, datum)
                    .await?
                {
                    None => {
                        self.store_data(&channel, data, persist, produce_ref)
                            .await?;
                        Ok(ScheduledProduce {
                            joins: vec![],
                            result: Ok(None),
                            phase_two: None,
                            release: ReleaseToken::detached(),
                        })
                    }
                    // The candidate itself is advisory (its reads raced cross-channel ops): the
                    // commit re-extracts under the full lock set. Only the matched channel set is
                    // carried forward, as the claim queue's phase-two claim set.
                    Some(pc) => Ok(ScheduledProduce {
                        joins: pc.channels.clone(),
                        result: Ok(None),
                        phase_two: Some(PendingProduce {
                            trigger: channel,
                            data,
                            persist,
                        }),
                        release: ReleaseToken::detached(),
                    }),
                }
            }
        };
        self.lock_f
            .acquire(&[hash], Box::pin(async { Ok(vec![]) }), thunk)
            .await?
    }

    /// Phase two of the scheduled produce: re-acquire the two-step lock over the trigger plus all
    /// current join channels and re-validate (the phase-one candidate may have been consumed in the
    /// gap). Falls back to storing the datum when nothing matches anymore.
    pub async fn scheduled_produce_commit(
        &self,
        pending: PendingProduce<C, A>,
    ) -> MaybeActionResult<C, P, A, K> {
        let produce_ref = Produce::apply(&pending.trigger, &pending.data, pending.persist);
        let hash = hash_channel(&pending.trigger);
        let phase_two = {
            let store = self.current_store();
            let trigger = pending.trigger.clone();
            Box::pin(async move {
                Ok(store
                    .get_joins(&trigger)
                    .await?
                    .into_iter()
                    .flatten()
                    .map(|c| hash_channel(&c))
                    .collect())
            })
        };
        let thunk = self.scheduled_produce_commit_body(pending, produce_ref);
        self.lock_f.acquire(&[hash], phase_two, thunk).await?
    }

    /// The `locked_produce` body minus the produce log (already written in phase one): re-extract
    /// with the in-flight datum prepended, then commit the match or store the datum.
    async fn scheduled_produce_commit_body(
        &self,
        pending: PendingProduce<C, A>,
        produce_ref: Produce,
    ) -> MaybeActionResult<C, P, A, K> {
        let grouped_channels = self.current_store().get_joins(&pending.trigger).await?;
        let datum = Datum {
            a: pending.data.clone(),
            persist: pending.persist,
            source: produce_ref.clone(),
        };
        match self
            .extract_produce_candidate(&grouped_channels, &pending.trigger, datum)
            .await?
        {
            None => {
                self.store_data(&pending.trigger, pending.data, pending.persist, produce_ref)
                    .await
            }
            Some(pc) => self.process_match_found(pc).await,
        }
    }

    /// The scheduled consume: the plain `locked_consume` flow (one two-step acquisition over every
    /// channel hash — the full static source set is already claimed on the claim queue), wrapped
    /// with the completion token.
    pub async fn scheduled_consume_at(
        &self,
        _path: Vec<u16>,
        channels: &[C],
        patterns: &[P],
        continuation: K,
        persist: bool,
        peeks: BTreeSet<usize>,
    ) -> std::result::Result<ScheduledConsume<C, P, A, K>, crate::errors::RSpaceError> {
        assert!(!channels.is_empty(), "channels can't be empty");
        assert_eq!(
            channels.len(),
            patterns.len(),
            "channels.length must equal patterns.length"
        );
        let consume_ref = Consume::apply(channels, patterns, &continuation, persist);
        let hashes: Vec<_> = channels.iter().map(hash_channel).collect();
        let thunk = self.locked_consume(
            channels,
            patterns,
            continuation,
            persist,
            peeks,
            consume_ref,
        );
        let result = self
            .lock_f
            .acquire(&hashes, Box::pin(async { Ok(hashes.clone()) }), thunk)
            .await?;
        Ok(ScheduledConsume {
            result,
            release: ReleaseToken::detached(),
        })
    }
}
