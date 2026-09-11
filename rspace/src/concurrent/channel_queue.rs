//! The per-channel claim queue of Law 20 (`spec/Rchain/Scheduler.lean`): "1 channel = 1 logical
//! task". Every effect claims the channels its op touches at its DFS path; a claim may run only
//! when it is the path-smallest *pending* claim on every claimed channel **and** no other claim is
//! executing on any of them.
//!
//! Path-ordered insertion is what closes the S.3 enqueue race
//! (`docs/src/formal/effect-scheduling.md`): a continuation enqueued with a DFS-earlier path
//! overtakes later-path siblings that are already pending — or even running — on the same channel,
//! while the "at most one op executes per channel at any instant" exclusion of Law 20 still holds:
//! the running claim finishes, then the overtaking claim runs. Dropping a guard removes the claim
//! from every claimed channel and wakes the next head.
//!
//! Generic over the path type `P` so rspace stays independent of rholang's `DfsPath`.
//!
//! Note (risk R3 of the channel-scheduler plan): a produce's phase-two re-wait after `claim_more`
//! holds its trigger-channel lease while waiting on join channels — deliberate hold-and-wait.
//! It cannot self-deadlock (the re-wait passes channels the claim already holds, regardless of
//! position), but cross-channel cycles are the reducer's responsibility
//! (`law20_deadlock_freedom` + stress tests; fallback: per-key `TwoStepLock`).

use std::hash::Hash;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use dashmap::mapref::entry::Entry;
use dashmap::DashMap;
use tokio::sync::Notify;

/// One channel's claim queue: claims sorted by path (stable — equal paths keep arrival order),
/// plus the claim currently executing its op, if any.
struct ChannelQueue<P> {
    entries: Vec<ClaimEntry<P>>,
    /// The claim holding the head lease on this channel (executing its op). While set, no other
    /// claim may run on the channel — not even `entries[0]` — which keeps same-channel execution
    /// exclusive when a DFS-earlier claim overtakes a running head.
    active: Option<u64>,
}

impl<P> Default for ChannelQueue<P> {
    fn default() -> Self {
        ChannelQueue {
            entries: Vec::new(),
            active: None,
        }
    }
}

/// One claim's slot in a channel queue.
struct ClaimEntry<P> {
    path: P,
    id: u64,
    /// Wakes the claim's parked `wait_at_head` when the channel's head changes.
    notify: Arc<Notify>,
}

struct QueueInner<K, P> {
    channels: DashMap<K, Arc<Mutex<ChannelQueue<P>>>>,
    next_id: AtomicU64,
}

/// The per-channel claim queue (Law 20). Claims are keyed by channel `K` and ordered by path `P`.
pub struct ChannelClaimQueue<K, P> {
    inner: Arc<QueueInner<K, P>>,
}

impl<K, P> ChannelClaimQueue<K, P>
where
    K: Eq + Hash + Ord + Clone + Send + Sync + 'static,
    P: Ord + Clone + Send + Sync + 'static,
{
    pub fn new() -> Self {
        ChannelClaimQueue {
            inner: Arc::new(QueueInner {
                channels: DashMap::new(),
                next_id: AtomicU64::new(0),
            }),
        }
    }

    /// Claim `channels` at `path` (sorted, deduped; equal paths keep arrival order). Returns a
    /// guard that releases the claim on drop. The inserting task is not woken by this call — it
    /// checks for itself in `wait_at_head`.
    pub fn claim(&self, path: P, channels: &[K]) -> ClaimGuard<K, P> {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let notify = Arc::new(Notify::new());
        let mut sorted: Vec<K> = channels.to_vec();
        sorted.sort();
        sorted.dedup();
        for ch in &sorted {
            self.inner.insert(ch.clone(), id, &path, &notify);
        }
        ClaimGuard {
            inner: self.inner.clone(),
            path,
            channels: sorted,
            id,
            notify,
        }
    }
}

impl<K, P> QueueInner<K, P>
where
    K: Eq + Hash + Ord + Clone + Send + Sync + 'static,
    P: Ord + Clone + Send + Sync + 'static,
{
    /// Insert (or create) the channel's queue and insert the claim at its path position. The
    /// dashmap shard lock is released before the queue mutex is taken, so the two can never be
    /// held in opposite orders.
    fn insert(&self, channel: K, id: u64, path: &P, notify: &Arc<Notify>) {
        let arc = match self.channels.entry(channel) {
            Entry::Occupied(occupied) => occupied.get().clone(),
            Entry::Vacant(vacant) => vacant
                .insert(Arc::new(Mutex::new(ChannelQueue::default())))
                .clone(),
        };
        let mut queue = arc.lock().unwrap_or_else(|p| p.into_inner());
        let at = queue.entries.partition_point(|e| e.path < *path);
        queue.entries.insert(
            at,
            ClaimEntry {
                path: path.clone(),
                id,
                notify: notify.clone(),
            },
        );
    }
}

impl<K, P> Default for ChannelClaimQueue<K, P>
where
    K: Eq + Hash + Ord + Clone + Send + Sync + 'static,
    P: Ord + Clone + Send + Sync + 'static,
{
    fn default() -> Self {
        Self::new()
    }
}

/// A claim held by one effect task. Dropping the guard removes the claim from every claimed
/// channel and wakes the next head — so the reducer holds it until the effect's continuation
/// effects are enqueued (the queue-level continuation-prepend of Law 20).
///
/// The bounds live on the struct (not just the methods) because `Drop` needs them.
pub struct ClaimGuard<K, P>
where
    K: Eq + Hash + Ord + Clone + Send + Sync + 'static,
    P: Ord + Clone + Send + Sync + 'static,
{
    inner: Arc<QueueInner<K, P>>,
    path: P,
    /// Claimed channels, kept sorted (the lock order of `try_acquire`).
    channels: Vec<K>,
    id: u64,
    notify: Arc<Notify>,
}

impl<K, P> ClaimGuard<K, P>
where
    K: Eq + Hash + Ord + Clone + Send + Sync + 'static,
    P: Ord + Clone + Send + Sync + 'static,
{
    /// Add channels to the claim (produce phase two: the joins discovered after the trigger
    /// committed). The claim keeps its path, so it overtakes later-path claims on the new
    /// channels too.
    pub fn claim_more(&mut self, channels: &[K]) {
        let mut extra: Vec<K> = channels.to_vec();
        extra.sort();
        extra.dedup();
        extra.retain(|ch| self.channels.binary_search(ch).is_err());
        for ch in &extra {
            self.inner
                .insert(ch.clone(), self.id, &self.path, &self.notify);
        }
        self.channels.extend(extra);
        self.channels.sort();
    }

    /// Wait until this claim is the head — the path-smallest pending claim of every claimed
    /// channel and alone executing on them — and return the head lease (Law 20's commit).
    pub async fn wait_at_head(&self) -> HeadLease {
        loop {
            // Register interest *before* checking so a head change between check and park cannot
            // be missed: a stale permit only causes a harmless spurious wakeup (we recheck).
            let notified = self.notify.notified();
            match self.try_acquire() {
                Ok(lease) => return lease,
                Err(()) => notified.await,
            }
        }
    }

    /// One acquisition attempt: lock every claimed channel in sorted order, verify the claim is
    /// the first pending entry and no other claim is active on any of them, mark it active on all,
    /// and release. Holding every channel lock through the check-and-mark keeps the linearization
    /// point gap-free (an earlier-path claim cannot slip in mid-check).
    fn try_acquire(&self) -> Result<HeadLease, ()> {
        let mut arcs: Vec<Arc<Mutex<ChannelQueue<P>>>> = Vec::with_capacity(self.channels.len());
        for channel in &self.channels {
            // The claim's channels are inserted at `claim`/`claim_more` time and only removed on
            // guard drop, so a missing entry means this claim is being torn down concurrently — treat
            // it as "not yet acquirable" and let `wait_at_head` re-check rather than panicking.
            let Some(arc) = self.inner.channels.get(channel).map(|entry| entry.clone()) else {
                return Err(());
            };
            arcs.push(arc);
        }

        let mut locks = Vec::with_capacity(arcs.len());
        for arc in &arcs {
            let queue = arc.lock().unwrap_or_else(|p| p.into_inner());
            // A claim already holding the lease here (produce phase two: `claim_more` after
            // `wait_at_head` re-waits without releasing the trigger channel) keeps running
            // regardless of position — a DFS-earlier pending claim must still wait for our
            // release. Only the new channels get the head check.
            let already_running = queue.active == Some(self.id);
            if !already_running
                && (queue.active.is_some() || queue.entries.first().map(|e| e.id) != Some(self.id))
            {
                return Err(());
            }
            locks.push(queue);
        }
        for queue in &mut locks {
            queue.active = Some(self.id);
        }
        Ok(HeadLease { _private: () })
    }

    /// The path this claim was made at.
    pub fn path(&self) -> &P {
        &self.path
    }

    /// The claimed channels (sorted).
    pub fn channels(&self) -> &[K] {
        &self.channels
    }
}

impl<K, P> Drop for ClaimGuard<K, P>
where
    K: Eq + Hash + Ord + Clone + Send + Sync + 'static,
    P: Ord + Clone + Send + Sync + 'static,
{
    fn drop(&mut self) {
        for channel in &self.channels {
            let Some(arc) = self.inner.channels.get(channel) else {
                continue;
            };
            let mut queue = arc.lock().unwrap_or_else(|p| p.into_inner());
            queue.entries.retain(|e| e.id != self.id);
            if queue.active == Some(self.id) {
                queue.active = None;
            }
            // The head (or its executability) may have changed: wake the new first entry. A stale
            // permit is harmless — waiters recheck eligibility.
            if let Some(head) = queue.entries.first() {
                head.notify.notify_one();
            }
        }
    }
}

/// The head lease: the token `wait_at_head` returns once the claim may execute its op on every
/// claimed channel. Advisory — the real exclusion is the queue's `active` mark, which the
/// guard's drop releases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeadLease {
    _private: (),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;

    use tokio::sync::Notify;

    /// Claim every path up front (in scrambled arrival order), then run the waiters: they must
    /// commit strictly in path order.
    #[tokio::test(flavor = "multi_thread")]
    async fn entries_process_in_path_order() {
        let queue = Arc::new(ChannelClaimQueue::<u8, u64>::new());
        let guards: Vec<_> = [5u64, 1, 3, 2, 4]
            .iter()
            .map(|path| queue.claim(*path, &[7u8]))
            .collect();
        let order = Arc::new(Mutex::new(Vec::new()));

        let mut tasks = Vec::new();
        for guard in guards {
            let order = order.clone();
            tasks.push(tokio::spawn(async move {
                let _lease = guard.wait_at_head().await;
                order.lock().unwrap().push(*guard.path());
            }));
        }
        for task in tasks {
            task.await.unwrap();
        }
        assert_eq!(*order.lock().unwrap(), vec![1, 2, 3, 4, 5]);
    }

    /// The S.3 enqueue race: a running head with a *later* path is overtaken by a DFS-earlier
    /// claim, which inserts before it and runs next — but only after the running head finishes
    /// (Law 20's same-channel exclusion), and before still-later pending claims.
    #[tokio::test(flavor = "multi_thread")]
    async fn later_path_inserts_before_running_head() {
        let queue = Arc::new(ChannelClaimQueue::<u8, u64>::new());
        let order = Arc::new(Mutex::new(Vec::new()));
        let head_ran = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());

        // The running head: path 5, later than everything that follows it.
        let head = queue.claim(5, &[1u8]);
        let (h_ran, h_release, h_order) = (head_ran.clone(), release.clone(), order.clone());
        let head_task = tokio::spawn(async move {
            let _lease = head.wait_at_head().await;
            h_order.lock().unwrap().push(5);
            h_ran.notify_one();
            h_release.notified().await; // hold the channel until released
        });
        head_ran.notified().await;

        // A later-path sibling is already pending when a DFS-earlier continuation arrives: it
        // must insert *before* the running head's entry.
        let middle = queue.claim(6, &[1u8]);
        let continuation = queue.claim(4, &[1u8]);
        let (c_order, c_guard) = (order.clone(), continuation);
        let continuation_task = tokio::spawn(async move {
            let _lease = c_guard.wait_at_head().await;
            c_order.lock().unwrap().push(4);
        });
        let (m_order, m_guard) = (order.clone(), middle);
        let middle_task = tokio::spawn(async move {
            let _lease = m_guard.wait_at_head().await;
            m_order.lock().unwrap().push(6);
        });

        // While the head runs, no other claim may execute on the channel.
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(*order.lock().unwrap(), vec![5]);

        release.notify_one(); // the head drops: 4 runs next, then 6
        let (h, c, m) = tokio::time::timeout(Duration::from_secs(5), async {
            tokio::join!(head_task, continuation_task, middle_task)
        })
        .await
        .expect("overtaken claims must not deadlock");
        h.unwrap();
        c.unwrap();
        m.unwrap();
        assert_eq!(*order.lock().unwrap(), vec![5, 4, 6]);
    }

    /// Produce phase two (`claim_more` after `wait_at_head`): the re-wait must not deadlock on
    /// the trigger channel even when a DFS-earlier claim is pending on it — the claim already
    /// holds the lease there and finishes, then the earlier claim runs.
    #[tokio::test(flavor = "multi_thread")]
    async fn phase_two_rewait_runs_despite_earlier_pending_claim() {
        let queue = Arc::new(ChannelClaimQueue::<u8, u64>::new());
        let order = Arc::new(Mutex::new(Vec::new()));

        // Synchronize on the produce's first acquisition: the DFS-earlier claim must arrive
        // while the produce holds the trigger lease (otherwise it may legitimately acquire
        // first and the expected order reverses).
        let acquired = Arc::new(Notify::new());
        let (o1, o2, a1) = (order.clone(), order.clone(), acquired.clone());
        let mut produce = queue.claim(5, &[1u8]);
        let produce_task = tokio::spawn(async move {
            let _lease = produce.wait_at_head().await;
            a1.notify_one();
            produce.claim_more(&[2u8]); // join discovered after the trigger committed
            let _lease = produce.wait_at_head().await;
            o1.lock().unwrap().push(5);
        });
        acquired.notified().await;
        // A DFS-earlier claim arrives on the trigger channel while the produce runs.
        let continuation = queue.claim(4, &[1u8]);
        let continuation_task = tokio::spawn(async move {
            let _lease = continuation.wait_at_head().await;
            o2.lock().unwrap().push(4);
        });

        let (p, c) = tokio::time::timeout(Duration::from_secs(5), async {
            tokio::join!(produce_task, continuation_task)
        })
        .await
        .expect("phase-two re-wait deadlocked against a pending earlier claim");
        p.unwrap();
        c.unwrap();
        assert_eq!(*order.lock().unwrap(), vec![5, 4]);
    }

    /// Hold-and-wait (risk R3): a produce parked on a blocked join channel keeps its trigger
    /// lease, and everything still commits in path order once the join channel frees.
    #[tokio::test(flavor = "multi_thread")]
    async fn phase_two_rewait_parks_on_blocked_join_then_continues() {
        let queue = Arc::new(ChannelClaimQueue::<u8, u64>::new());
        let order = Arc::new(Mutex::new(Vec::new()));
        let join_ran = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());

        // Another claim executes on the join channel first.
        let join = queue.claim(3, &[2u8]);
        let (j_ran, j_release, j_order) = (join_ran.clone(), release.clone(), order.clone());
        let join_task = tokio::spawn(async move {
            let _lease = join.wait_at_head().await;
            j_order.lock().unwrap().push(3);
            j_ran.notify_one();
            j_release.notified().await;
        });
        join_ran.notified().await;

        // Synchronize on the produce's trigger acquisition so the DFS-earlier continuation is
        // guaranteed to arrive after it (and thus park behind the active lease).
        let acquired = Arc::new(Notify::new());
        let (produce_order, a1) = (order.clone(), acquired.clone());
        let mut produce = queue.claim(5, &[1u8]);
        let produce_task = tokio::spawn(async move {
            let _lease = produce.wait_at_head().await;
            a1.notify_one();
            produce.claim_more(&[2u8]);
            let _lease = produce.wait_at_head().await; // parks: join channel is active
            produce_order.lock().unwrap().push(5);
        });
        acquired.notified().await;
        let continuation_order = order.clone();
        let continuation = queue.claim(4, &[1u8]);
        let continuation_task = tokio::spawn(async move {
            let _lease = continuation.wait_at_head().await;
            continuation_order.lock().unwrap().push(4);
        });

        // The produce parks holding the trigger lease; nothing else commits meanwhile.
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(*order.lock().unwrap(), vec![3]);

        release.notify_one(); // join channel frees: produce completes, then the continuation
        let (j, p, c) = tokio::time::timeout(Duration::from_secs(5), async {
            tokio::join!(join_task, produce_task, continuation_task)
        })
        .await
        .expect("hold-and-wait deadlocked");
        j.unwrap();
        p.unwrap();
        c.unwrap();
        assert_eq!(*order.lock().unwrap(), vec![3, 5, 4]);
    }

    /// Law 20's exclusion: with N claims on one channel, no two critical sections overlap and
    /// they commit in path order.
    #[tokio::test(flavor = "multi_thread")]
    async fn claims_on_shared_channel_are_mutually_exclusive() {
        const N: u64 = 8;
        let queue = Arc::new(ChannelClaimQueue::<u8, u64>::new());
        let guards: Vec<_> = (0..N).map(|path| queue.claim(path, &[9u8])).collect();
        let inside = Arc::new(AtomicUsize::new(0));
        let order = Arc::new(Mutex::new(Vec::new()));

        let mut tasks = Vec::new();
        for guard in guards {
            let inside = inside.clone();
            let order = order.clone();
            tasks.push(tokio::spawn(async move {
                let _lease = guard.wait_at_head().await;
                let was = inside.fetch_add(1, Ordering::SeqCst);
                assert_eq!(was, 0, "two claims ran on the same channel at once");
                order.lock().unwrap().push(*guard.path());
                tokio::task::yield_now().await;
                inside.fetch_sub(1, Ordering::SeqCst);
            }));
        }
        for task in tasks {
            task.await.unwrap();
        }
        assert_eq!(*order.lock().unwrap(), (0..N).collect::<Vec<_>>());
    }
}
