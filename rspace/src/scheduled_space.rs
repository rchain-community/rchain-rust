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
        if channels.is_empty() || channels.len() != patterns.len() {
            return Err(crate::errors::RSpaceError::ConsumeArity(
                "a scheduled consume requires a non-empty channel set with matching patterns",
            ));
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use rchain_shared::store_manager::InMemoryStoreManager;

    use crate::factory::create_history_repository;
    use crate::hot_store::InMemHotStore;
    use crate::i_space::ISpace;
    use crate::match_::Match;
    use crate::tuple_space::Tuplespace;

    /// Any pattern matches any datum — enough to produce a COMM and therefore a deferred commit.
    struct StrMatch;
    impl Match<String, String> for StrMatch {
        fn get(&self, _p: &String, a: &String) -> Option<String> {
            Some(a.clone())
        }
    }

    /// A fresh `RSpace` (the only implementation that overrides the scheduled ops).
    async fn space() -> Arc<RSpace<String, String, String, String>> {
        let manager = InMemoryStoreManager::default();
        let history = create_history_repository::<String, String, String, String>(
            &manager,
            "scheduled-space",
        )
        .await
        .expect("history repository");
        let reader = history.get_history_reader(history.root()).await;
        let hot = Arc::new(InMemHotStore::new(reader.base()));
        let (play, _replay) = RSpace::create_with_replay(history, hot, Arc::new(StrMatch));
        play
    }

    /// **Phase one, no match.** With nothing waiting on the channel the datum is stored inline and
    /// there is no phase two to run: `phase_two` is `None`, `joins` is empty, and the produce is
    /// already final. This is the arm that keeps the common case to a single lock acquisition.
    #[tokio::test]
    async fn a_produce_with_no_match_stores_inline_and_defers_nothing() {
        let space = space().await;
        let scheduled = space
            .scheduled_produce_at(vec![], "c".to_string(), "data".to_string(), false)
            .await
            .expect("scheduled produce");

        assert!(
            scheduled.phase_two.is_none(),
            "no match means no deferred commit: {scheduled:?}"
        );
        assert!(scheduled.joins.is_empty(), "{scheduled:?}");
        assert_eq!(scheduled.result, Ok(None));
        assert_eq!(scheduled.release, ReleaseToken::detached());
        // The datum really is in the space — the inline path stored it.
        let stored = space.get_data(&"c".to_string()).await.expect("get data");
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].a, "data");
    }

    /// **Phase one, match.** With a consume waiting, phase one returns the matched channel set for
    /// `claim_more` and *does not* commit: `phase_two` carries the trigger, the in-flight datum and
    /// the persistence flag, and nothing has been delivered yet.
    #[tokio::test]
    async fn a_produce_that_matches_returns_the_join_set_and_defers_the_commit() {
        let space = space().await;
        space
            .consume(
                &["c".to_string()],
                &["p".to_string()],
                "cont".to_string(),
                false,
                BTreeSet::new(),
            )
            .await
            .expect("waiting consume");

        let scheduled = space
            .scheduled_produce_at(vec![], "c".to_string(), "data".to_string(), true)
            .await
            .expect("scheduled produce");

        assert_eq!(
            scheduled.joins,
            vec!["c".to_string()],
            "the matched channel set is what the caller claims"
        );
        let pending = scheduled
            .phase_two
            .as_ref()
            .expect("a match defers the commit");
        assert_eq!(pending.trigger, "c");
        assert_eq!(pending.data, "data");
        assert!(pending.persist, "the persistence flag must be carried over");
        // Not delivered yet: the continuation is still waiting, and no datum was stored.
        assert_eq!(
            space
                .get_waiting_continuations(&["c".to_string()])
                .await
                .expect("continuations")
                .len(),
            1,
            "phase one must not deliver; that is phase two's job"
        );
        assert!(space
            .get_data(&"c".to_string())
            .await
            .expect("data")
            .is_empty());
    }

    /// **Phase two.** Committing the pending produce delivers it: the continuation the consume
    /// parked is returned and the datum is not left in the space.
    #[tokio::test]
    async fn the_deferred_commit_delivers_the_match() {
        let space = space().await;
        space
            .consume(
                &["c".to_string()],
                &["p".to_string()],
                "cont".to_string(),
                false,
                BTreeSet::new(),
            )
            .await
            .expect("waiting consume");

        let scheduled = space
            .scheduled_produce_at(vec![], "c".to_string(), "data".to_string(), false)
            .await
            .expect("scheduled produce");
        let pending = scheduled.phase_two.expect("deferred");
        let committed = space
            .scheduled_produce_commit(pending)
            .await
            .expect("commit");
        let (cont_result, _) = committed.expect("a match");
        assert_eq!(cont_result.continuation, "cont");
        assert_eq!(cont_result.channels, vec!["c".to_string()]);
    }

    /// **Phase two re-validates.** The phase-one candidate is advisory: if a cross-channel op takes
    /// the waiting consume in the gap, the commit must *store* the datum rather than deliver to a
    /// continuation that is no longer there. This is the property that makes the relaxed
    /// interleaving sound, and the one a phase two that trusted its phase-one reads would break.
    #[tokio::test]
    async fn a_commit_whose_candidate_vanished_stores_instead_of_delivering() {
        let space = space().await;
        space
            .consume(
                &["c".to_string()],
                &["p".to_string()],
                "cont".to_string(),
                false,
                BTreeSet::new(),
            )
            .await
            .expect("waiting consume");
        let scheduled = space
            .scheduled_produce_at(vec![], "c".to_string(), "data".to_string(), false)
            .await
            .expect("scheduled produce");
        let pending = scheduled.phase_two.expect("deferred");

        // The gap: a produce on a *different* channel can take the continuation... but this matcher
        // matches anything, so produce a datum that would match the same consume and commit it, and
        // the phase-one candidate is gone.
        space
            .produce("c".to_string(), "first".to_string(), false)
            .await
            .expect("the interleaved produce takes the continuation");

        let committed = space
            .scheduled_produce_commit(pending)
            .await
            .expect("commit");
        assert!(
            committed.is_none(),
            "the candidate was consumed in the gap, so the commit must store: {committed:?}"
        );
        let stored = space.get_data(&"c".to_string()).await.expect("data");
        assert!(
            stored.iter().any(|d| d.a == "data"),
            "the re-validated produce stores its own datum: {stored:?}"
        );
    }

    /// **The consume's validation arm.** A scheduled consume claims the effect's full static source
    /// set, so an empty channel list or a pattern/channel arity mismatch is a caller bug, not an
    /// empty space: it is reported as `ConsumeArity` rather than acquiring a lock on nothing.
    #[tokio::test]
    async fn a_scheduled_consume_rejects_an_empty_or_mismatched_source_set() {
        let space = space().await;

        let empty = space
            .scheduled_consume_at(vec![], &[], &[], "cont".to_string(), false, BTreeSet::new())
            .await
            .expect_err("an empty channel set is an error");
        assert!(
            format!("{empty}").contains("non-empty channel set"),
            "{empty}"
        );

        let mismatched = space
            .scheduled_consume_at(
                vec![],
                &["c".to_string()],
                &["p".to_string(), "q".to_string()],
                "cont".to_string(),
                false,
                BTreeSet::new(),
            )
            .await
            .expect_err("one channel, two patterns is an error");
        assert!(
            format!("{mismatched}").contains("non-empty channel set"),
            "{mismatched}"
        );
    }

    /// The happy consume path: one acquisition over the whole source set, and the token the caller
    /// drops once it has enqueued the continuation's next-step effects.
    #[tokio::test]
    async fn a_scheduled_consume_commits_its_full_source_set() {
        let space = space().await;
        space
            .produce("c".to_string(), "data".to_string(), false)
            .await
            .expect("produce");

        let scheduled = space
            .scheduled_consume_at(
                vec![],
                &["c".to_string()],
                &["p".to_string()],
                "cont".to_string(),
                false,
                BTreeSet::new(),
            )
            .await
            .expect("scheduled consume");
        let (cont_result, _) = scheduled.result.expect("a match").expect("data");
        assert_eq!(cont_result.continuation, "cont");
        assert_eq!(scheduled.release, ReleaseToken::detached());
    }
}
