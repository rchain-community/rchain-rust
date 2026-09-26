//! Two-phase lock: acquire phase-A keys, compute phase-B keys while holding A, then acquire B.
//!
//! Mirrors `rspace/src/main/scala/coop/rchain/rspace/concurrent/TwoStepLock.scala`.

use std::future::Future;
use std::hash::Hash;

use crate::concurrent::multi_lock::MultiLock;
use crate::concurrent::BoxFuture;
use crate::errors::RSpaceError;

/// A two-phase lock (port of `TwoStepLock` / `ConcurrentTwoStepLockF`).
pub struct TwoStepLock<K> {
    phase_a: MultiLock<K>,
    phase_b: MultiLock<K>,
}

impl<K> TwoStepLock<K>
where
    K: Eq + Hash + Ord + Clone + Send + Sync + 'static,
{
    pub fn new() -> Self {
        TwoStepLock {
            phase_a: MultiLock::new(),
            phase_b: MultiLock::new(),
        }
    }

    /// Acquire `keys_a`, then run `phase_two` to compute `keys_b`, acquire those, then run `thunk`
    /// (port of `acquire`).
    pub async fn acquire<'a, F>(
        &'a self,
        keys_a: &[K],
        phase_two: BoxFuture<'a, std::result::Result<Vec<K>, RSpaceError>>,
        thunk: F,
    ) -> std::result::Result<F::Output, RSpaceError>
    where
        F: Future + Send + 'a,
    {
        self.phase_a
            .acquire(keys_a, async move {
                let keys_b = phase_two.await?;
                Ok(self.phase_b.acquire(&keys_b, thunk).await)
            })
            .await
    }
}

impl<K> Default for TwoStepLock<K>
where
    K: Eq + Hash + Ord + Clone + Send + Sync + 'static,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<K> TwoStepLock<K>
where
    K: Eq + Hash + Ord + Clone + Send + Sync + 'static,
{
    /// Release both phases' mutexes (port of the Scala's `cleanUp`, which composes the two
    /// `MultiLock`s' own). `RSpace::reset` and `ReplayRSpace::reset` call it, as `RSpaceOps.reset`
    /// calls the Scala's — AUDIT C104.
    pub fn clean_up(&self) {
        self.phase_a.clean_up();
        self.phase_b.clean_up();
    }

    /// The keys both phases are holding — test-only, so `reset` leaving nothing behind is asserted.
    #[cfg(test)]
    pub(crate) fn lock_map_len(&self) -> usize {
        self.phase_a.len() + self.phase_b.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;

    fn keys_b(keys: Vec<u8>) -> BoxFuture<'static, std::result::Result<Vec<u8>, RSpaceError>> {
        Box::pin(async move { Ok(keys) })
    }

    /// `clean_up` reaches **both** phases — the composition the Scala's `TwoStepLock.cleanUp`
    /// performs, and the port had no such method at all (AUDIT C104).
    #[tokio::test]
    async fn clean_up_releases_both_phases() {
        let lock: TwoStepLock<u8> = TwoStepLock::new();
        lock.acquire(&[1], keys_b(vec![2]), async {})
            .await
            .expect("acquired");
        assert_eq!(
            lock.lock_map_len(),
            2,
            "one phase-A key and one phase-B key"
        );
        lock.clean_up();
        assert_eq!(lock.lock_map_len(), 0, "both phases are cleared");
    }

    /// The thunk runs **once both phases are held**, and its output is the `acquire` result.
    #[tokio::test]
    async fn the_thunk_runs_under_both_phases_and_its_output_comes_back() {
        let lock: TwoStepLock<u8> = TwoStepLock::new();
        let out = lock
            .acquire(&[1, 2], keys_b(vec![3]), async { 42u8 })
            .await
            .expect("acquired");
        assert_eq!(out, 42);

        // …and the locks are released afterwards: the same keys are acquirable again.
        assert_eq!(
            lock.acquire(&[1, 2], keys_b(vec![3]), async { 7u8 })
                .await
                .expect("re-acquired"),
            7
        );
        assert_eq!(
            lock.acquire(&[3], keys_b(vec![1]), async { 8u8 })
                .await
                .expect("phase-B key released too"),
            8
        );
    }

    /// A failure while computing the phase-B keys propagates **and releases phase A**, and the thunk
    /// never runs: computing the keys is part of acquiring, not part of the work. A lock that held
    /// phase A after this error would deadlock on the next acquire of the same key.
    #[tokio::test]
    async fn a_phase_two_error_skips_the_thunk_and_releases_phase_a() {
        let lock: TwoStepLock<u8> = TwoStepLock::new();
        let ran = Arc::new(AtomicBool::new(false));
        let flag = ran.clone();

        let phase_two: BoxFuture<'_, std::result::Result<Vec<u8>, RSpaceError>> =
            Box::pin(async { Err(RSpaceError::LockPoisoned) });
        let err = lock
            .acquire(&[1], phase_two, async move {
                flag.store(true, Ordering::SeqCst);
            })
            .await
            .expect_err("phase two failed");
        assert_eq!(err, RSpaceError::LockPoisoned);
        assert!(!ran.load(Ordering::SeqCst), "the thunk must not run");

        // The tell for a leaked lock: the next acquire of the same key would block forever.
        assert_eq!(
            lock.acquire(&[1], keys_b(vec![2]), async { 5u8 })
                .await
                .expect("phase A was released"),
            5
        );
    }

    /// `phase_two` runs **while phase A is held** — that is the point of the two steps: the keys to
    /// lock in the second phase are computed from state that nothing else may change concurrently.
    #[tokio::test]
    async fn phase_two_runs_with_phase_a_already_held() {
        let lock: TwoStepLock<u8> = TwoStepLock::new();
        let inside_a = Arc::new(AtomicUsize::new(0));
        let counter = inside_a.clone();
        let max = Arc::new(AtomicUsize::new(0));
        let max_seen = max.clone();

        // Task 1 holds phase A on key 1 and yields inside phase two.
        let t1 = {
            let lock = &lock;
            let counter = counter.clone();
            let max_seen = max_seen.clone();
            async move {
                let phase_two: BoxFuture<'_, std::result::Result<Vec<u8>, RSpaceError>> =
                    Box::pin(async move {
                        let now = counter.fetch_add(1, Ordering::SeqCst) + 1;
                        max_seen.fetch_max(now, Ordering::SeqCst);
                        tokio::task::yield_now().await;
                        counter.fetch_sub(1, Ordering::SeqCst);
                        Ok(vec![1u8])
                    });
                lock.acquire(&[1], phase_two, async {}).await
            }
        };
        // Task 2 tries the same phase-A key: it cannot start phase two until task 1 finishes.
        let t2 = {
            let lock = &lock;
            let counter = counter.clone();
            let max_seen = max_seen.clone();
            async move {
                let phase_two: BoxFuture<'_, std::result::Result<Vec<u8>, RSpaceError>> =
                    Box::pin(async move {
                        let now = counter.fetch_add(1, Ordering::SeqCst) + 1;
                        max_seen.fetch_max(now, Ordering::SeqCst);
                        counter.fetch_sub(1, Ordering::SeqCst);
                        Ok(vec![1u8])
                    });
                lock.acquire(&[1], phase_two, async {}).await
            }
        };

        let (a, b) = tokio::join!(t1, t2);
        a.expect("first");
        b.expect("second");
        assert_eq!(
            max.load(Ordering::SeqCst),
            1,
            "two phase-twos on the same phase-A key must not overlap"
        );
    }

    /// Across **different** phase-A keys the two steps do run concurrently — the lock is per key,
    /// not a global mutex, which is what lets unrelated channels progress in parallel.
    #[tokio::test]
    async fn different_keys_do_not_block_each_other() {
        let lock = Arc::new(TwoStepLock::<u8>::new());
        let inside = Arc::new(AtomicUsize::new(0));
        let max = Arc::new(AtomicUsize::new(0));

        let run = |key: u8| {
            let lock = lock.clone();
            let inside = inside.clone();
            let max = max.clone();
            tokio::spawn(async move {
                let phase_two: BoxFuture<'static, std::result::Result<Vec<u8>, RSpaceError>> =
                    Box::pin(async move { Ok(vec![key]) });
                lock.acquire(&[key], phase_two, async move {
                    let now = inside.fetch_add(1, Ordering::SeqCst) + 1;
                    max.fetch_max(now, Ordering::SeqCst);
                    tokio::task::yield_now().await;
                    inside.fetch_sub(1, Ordering::SeqCst);
                })
                .await
                .expect("acquired")
            })
        };

        let (a, b) = tokio::join!(run(1), run(2));
        a.expect("task a");
        b.expect("task b");
        assert_eq!(
            max.load(Ordering::SeqCst),
            2,
            "distinct keys must be held simultaneously"
        );
    }
}
