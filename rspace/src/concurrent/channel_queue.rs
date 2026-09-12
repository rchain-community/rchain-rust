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
//! The queue also carries the Laws 23–25 versioned write-record layer
//! (`docs/src/formal/onchain-scheduling.md`, `spec/Rchain/SchedulerOnchain.lean`'s `SpecState`):
//! per channel, the newest committed write (writer path + version + polarity). With validation
//! enabled (`set_validation_enabled`), `try_acquire` enforces Law 24's per-commit
//! prefix-visibility certificate at the linearization point — a commit may read only the state
//! DFS-earlier effects produced, so every claimed channel's newest write must be strictly
//! path-earlier than the claim. `record_write` stamps a commit's writes (called by the reducer
//! at its op's write points); `reset_write_record` clears the layer per evaluation.
//!
//! Note (risk R3 of the channel-scheduler plan): a produce's phase-two re-wait after `claim_more`
//! holds its trigger-channel lease while waiting on join channels — deliberate hold-and-wait.
//! It cannot self-deadlock (the re-wait passes channels the claim already holds, regardless of
//! position), but cross-channel cycles are the reducer's responsibility
//! (`law20_deadlock_freedom` + stress tests; fallback: per-key `TwoStepLock`).

use std::hash::Hash;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use dashmap::mapref::entry::Entry;
use dashmap::DashMap;
use tokio::sync::Notify;

/// One committed write in the versioned write-record layer (Law 24, `SpecState` of
/// `spec/Rchain/SchedulerOnchain.lean`): the writer's DFS path, the per-channel version (write
/// count), and the post-write polarity (produce `true`, matched consume `false`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteRecord<P> {
    pub path: P,
    pub version: u64,
    pub polarity: bool,
}

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
    /// The versioned write-record layer (Laws 23–25, the Lean `SpecState`): per channel, the
    /// newest committed write. Stamped by `record_write` (the reducer's write points); read by
    /// the prefix-visibility check in `try_acquire` when validation is enabled.
    writes: DashMap<K, WriteRecord<P>>,
    /// Whether `try_acquire` enforces the Law 24 per-commit certificate. Off by default; the
    /// relaxed-validated block-path mode enables it.
    validation_enabled: AtomicBool,
    next_id: AtomicU64,
    /// Set when a DFS-earlier claim is inserted while another claim is executing (the S.3
    /// enqueue window) — a heuristic divergence signal. Reset per evaluation via `reset_skew`; the
    /// validated block path reads it after evaluation as a fallback trigger (not a sound
    /// replacement for the whole-set sequential oracle).
    skewed: AtomicBool,
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
                writes: DashMap::new(),
                validation_enabled: AtomicBool::new(false),
                next_id: AtomicU64::new(0),
                skewed: AtomicBool::new(false),
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

    /// The per-channel write count (the Laws 23–25 write-record layer's version): the number of
    /// committed writes recorded on `channel` so far.
    pub fn channel_version(&self, channel: &K) -> u64 {
        self.inner
            .writes
            .get(channel)
            .map(|w| w.version)
            .unwrap_or(0)
    }

    /// The newest committed write on `channel`, if any (the Lean `SpecState` lookup).
    pub fn last_write(&self, channel: &K) -> Option<WriteRecord<P>> {
        self.inner.writes.get(channel).map(|w| w.clone())
    }

    /// Record a committed write on `channel`: the writer path, the next version (previous + 1,
    /// as in the Lean `applyAt`), and the post-write polarity. Called by the reducer at its
    /// op's write points, before the claim guard drops.
    pub fn record_write(&self, channel: &K, path: &P, polarity: bool) {
        self.inner
            .writes
            .entry(channel.clone())
            .and_modify(|w| {
                w.path = path.clone();
                w.version += 1;
                w.polarity = polarity;
            })
            .or_insert(WriteRecord {
                path: path.clone(),
                version: 1,
                polarity,
            });
    }

    /// Clear the write-record layer. The reducer resets it per evaluation — without it, writes
    /// from a prior deploy in the same runtime would spuriously invalidate the next one.
    pub fn reset_write_record(&self) {
        self.inner.writes.clear();
    }

    /// Clear the S.3 enqueue-window skew signal. Reset per evaluation by the reducer (alongside
    /// `reset_write_record`) so a skew in one deploy doesn't spuriously invalidate the next.
    pub fn reset_skew(&self) {
        self.inner.skewed.store(false, Ordering::Relaxed);
    }

    /// Enable or disable the Law 24 prefix-visibility check in `try_acquire` (the
    /// relaxed-validated block-path mode enables it; every other mode leaves it off).
    pub fn set_validation_enabled(&self, enabled: bool) {
        self.inner
            .validation_enabled
            .store(enabled, Ordering::Relaxed);
    }

    /// Whether any channel observed the S.3 enqueue window — a DFS-earlier claim inserted while
    /// another claim was executing. Reset per evaluation via `reset_skew`; the validated block path
    /// reads it after evaluation as a heuristic fallback trigger (the certificate's blind-spot
    /// complement, not a sound replacement for the whole-set sequential oracle).
    pub fn observed_skew(&self) -> bool {
        self.inner.skewed.load(Ordering::Relaxed)
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
        // Ties keep arrival order (FIFO), so an equal-path claim lands *behind* the executing
        // entry — only a strictly DFS-earlier path reaches the front of a held channel.
        let at = queue.entries.partition_point(|e| e.path <= *path);
        queue.entries.insert(
            at,
            ClaimEntry {
                path: path.clone(),
                id,
                notify: notify.clone(),
            },
        );
        // The S.3 enqueue window: a DFS-earlier claim arriving while a later-path claim is
        // executing (the phase-two re-wait inserts under its own `active` id and is excluded)
        // is the divergence signal of Law 24 (`docs/src/formal/onchain-scheduling.md`).
        if at == 0 && queue.active.is_some() && queue.active != Some(id) {
            self.skewed.store(true, Ordering::Relaxed);
        }
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

/// Why a head acquisition failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcquireError<K, P> {
    /// The claim is not (yet) the head — `wait_at_head` parks and retries.
    NotHead,
    /// Law 24 prefix visibility failed: a claimed channel's newest committed write was made by
    /// a DFS-later (or equal) path, so this commit read state no DFS-earlier effect produced.
    /// Permanent for the run — the caller must fall back, never retry.
    ValidationFailed {
        /// The channel whose newest write invalidates the claim.
        channel: K,
        /// The invalidating writer's path.
        writer_path: P,
    },
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
    /// channel and alone executing on them — and return the head lease (Law 20's commit). With
    /// validation enabled, a Law 24 prefix-visibility failure returns immediately (permanent —
    /// the run must fall back) instead of retrying.
    pub async fn wait_at_head(&self) -> Result<HeadLease, AcquireError<K, P>> {
        loop {
            // Register interest *before* checking so a head change between check and park cannot
            // be missed: a stale permit only causes a harmless spurious wakeup (we recheck).
            let notified = self.notify.notified();
            match self.try_acquire() {
                Ok(lease) => return Ok(lease),
                Err(AcquireError::NotHead) => notified.await,
                Err(validation) => return Err(validation),
            }
        }
    }

    /// One acquisition attempt: lock every claimed channel in sorted order, verify the claim is
    /// the first pending entry and no other claim is active on any of them, check the Law 24
    /// write-record certificate (when enabled), mark it active on all, and release. Holding
    /// every channel lock through the check-and-mark keeps the linearization point gap-free (an
    /// earlier-path claim cannot slip in mid-check).
    fn try_acquire(&self) -> Result<HeadLease, AcquireError<K, P>> {
        let mut arcs: Vec<Arc<Mutex<ChannelQueue<P>>>> = Vec::with_capacity(self.channels.len());
        for channel in &self.channels {
            // The claim's channels are inserted at `claim`/`claim_more` time and only removed on
            // guard drop, so a missing entry means this claim is being torn down concurrently — treat
            // it as "not yet acquirable" and let `wait_at_head` re-check rather than panicking.
            let Some(arc) = self.inner.channels.get(channel).map(|entry| entry.clone()) else {
                return Err(AcquireError::NotHead);
            };
            arcs.push(arc);
        }

        let mut locks = Vec::with_capacity(arcs.len());
        for (channel, arc) in self.channels.iter().zip(&arcs) {
            let queue = arc.lock().unwrap_or_else(|p| p.into_inner());
            // A claim already holding the lease here (produce phase two: `claim_more` after
            // `wait_at_head` re-waits without releasing the trigger channel) keeps running
            // regardless of position — a DFS-earlier pending claim must still wait for our
            // release. Only the new channels get the head check.
            let already_running = queue.active == Some(self.id);
            if !already_running
                && (queue.active.is_some() || queue.entries.first().map(|e| e.id) != Some(self.id))
            {
                return Err(AcquireError::NotHead);
            }
            // Law 24's per-commit certificate: the commit may read only the state DFS-earlier
            // effects produced — every claimed channel's newest write must be strictly
            // path-earlier than this claim. Skipped for channels the claim already holds (the
            // produce phase-two re-wait: while the lease is held no other write can land there,
            // and the claim's own trigger write stamps at its own — equal — path).
            if self.inner.validation_enabled.load(Ordering::Relaxed) && !already_running {
                if let Some(w) = self.inner.writes.get(channel) {
                    if w.path >= self.path {
                        return Err(AcquireError::ValidationFailed {
                            channel: channel.clone(),
                            writer_path: w.path.clone(),
                        });
                    }
                }
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
                let _lease = guard.wait_at_head().await.unwrap();
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
            let _lease = head.wait_at_head().await.unwrap();
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
            let _lease = c_guard.wait_at_head().await.unwrap();
            c_order.lock().unwrap().push(4);
        });
        let (m_order, m_guard) = (order.clone(), middle);
        let middle_task = tokio::spawn(async move {
            let _lease = m_guard.wait_at_head().await.unwrap();
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
            let _lease = produce.wait_at_head().await.unwrap();
            a1.notify_one();
            produce.claim_more(&[2u8]); // join discovered after the trigger committed
            let _lease = produce.wait_at_head().await.unwrap();
            o1.lock().unwrap().push(5);
        });
        acquired.notified().await;
        // A DFS-earlier claim arrives on the trigger channel while the produce runs.
        let continuation = queue.claim(4, &[1u8]);
        let continuation_task = tokio::spawn(async move {
            let _lease = continuation.wait_at_head().await.unwrap();
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
            let _lease = join.wait_at_head().await.unwrap();
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
            let _lease = produce.wait_at_head().await.unwrap();
            a1.notify_one();
            produce.claim_more(&[2u8]);
            let _lease = produce.wait_at_head().await.unwrap(); // parks: join channel is active
            produce_order.lock().unwrap().push(5);
        });
        acquired.notified().await;
        let continuation_order = order.clone();
        let continuation = queue.claim(4, &[1u8]);
        let continuation_task = tokio::spawn(async move {
            let _lease = continuation.wait_at_head().await.unwrap();
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

    /// The S.3 enqueue window sets the skew signal: a DFS-earlier claim inserted while a
    /// later-path claim is executing marks the channel (Law 24's divergence signal), while the
    /// executing claim's own phase-two `claim_more` re-insert does not. The version counter is
    /// the write-record layer's write count (`record_write`), not an acquisition count.
    #[tokio::test(flavor = "multi_thread")]
    async fn enqueue_window_sets_skew_and_write_versions() {
        let queue = Arc::new(ChannelClaimQueue::<u8, u64>::new());
        let acquired = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());

        // The running head at path 5: acquired, holds the channel.
        let head = queue.claim(5, &[1u8]);
        let (h_acq, h_rel) = (acquired.clone(), release.clone());
        let head_task = tokio::spawn(async move {
            let _lease = head.wait_at_head().await.unwrap();
            h_acq.notify_one();
            h_rel.notified().await;
        });
        acquired.notified().await;
        // Acquisitions do not write: the record counts committed writes only.
        assert_eq!(queue.channel_version(&1u8), 0);
        queue.record_write(&1u8, &5, true);
        assert_eq!(queue.channel_version(&1u8), 1);
        assert_eq!(
            queue.last_write(&1u8),
            Some(WriteRecord {
                path: 5,
                version: 1,
                polarity: true
            })
        );
        assert!(!queue.observed_skew());

        // A DFS-earlier claim arrives while the later-path head executes: the enqueue window.
        let earlier = queue.claim(4, &[1u8]);
        assert!(
            queue.observed_skew(),
            "the enqueue window must set the skew signal"
        );

        // The executing claim's own phase-two re-insert is not a divergence.
        let mut self_claim = queue.claim(6, &[2u8]);
        self_claim.wait_at_head().await.unwrap(); // channel 2 is free
        let skew_before = queue.observed_skew();
        self_claim.claim_more(&[1u8]); // re-insert on channel 1 under its own active id
        assert_eq!(
            queue.observed_skew(),
            skew_before,
            "phase-two re-wait must not skew"
        );

        release.notify_one();
        let _ = earlier.wait_at_head().await.unwrap();
        head_task.await.unwrap();
        // The overtaking claim's write bumps the record: two writes, two versions.
        queue.record_write(&1u8, &4, true);
        assert_eq!(queue.channel_version(&1u8), 2);
        assert_eq!(
            queue.last_write(&1u8),
            Some(WriteRecord {
                path: 4,
                version: 2,
                polarity: true
            })
        );
    }

    /// `reset_skew` clears the S.3 enqueue-window signal: trip it with a DFS-earlier claim landing
    /// while a later-path claim holds the lease, then reset and observe it cleared (per-evaluation
    /// reset so one deploy's skew doesn't invalidate the next).
    #[tokio::test(flavor = "multi_thread")]
    async fn reset_skew_clears_the_signal() {
        let queue = Arc::new(ChannelClaimQueue::<u8, u64>::new());
        let acquired = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());

        // A later-path claim holds the head lease on channel 1.
        let head = queue.claim(5, &[1u8]);
        let (h_acq, h_rel) = (acquired.clone(), release.clone());
        let head_task = tokio::spawn(async move {
            let _lease = head.wait_at_head().await.unwrap();
            h_acq.notify_one();
            h_rel.notified().await;
        });
        acquired.notified().await;

        // A DFS-earlier claim lands while the later-path head executes: the enqueue window.
        let earlier = queue.claim(4, &[1u8]);
        assert!(
            queue.observed_skew(),
            "the enqueue window must set the skew signal"
        );

        queue.reset_skew();
        assert!(
            !queue.observed_skew(),
            "reset_skew must clear the skew signal"
        );

        release.notify_one();
        let _ = earlier.wait_at_head().await.unwrap();
        head_task.await.unwrap();
    }

    /// Law 24's certificate: with validation enabled, a claim whose channel's newest write is
    /// DFS-later fails at the linearization point and returns promptly instead of retrying.
    #[tokio::test(flavor = "multi_thread")]
    async fn validation_fails_on_later_write_and_does_not_retry() {
        let queue = ChannelClaimQueue::<u8, u64>::new();
        queue.set_validation_enabled(true);
        queue.record_write(&1u8, &5, true);
        let guard = queue.claim(4, &[1u8]);
        let err = tokio::time::timeout(Duration::from_millis(200), guard.wait_at_head())
            .await
            .expect("a validation failure must return promptly, not retry")
            .unwrap_err();
        assert_eq!(
            err,
            AcquireError::ValidationFailed {
                channel: 1u8,
                writer_path: 5,
            }
        );
    }

    /// A DFS-earlier write validates: the commit may read what DFS-earlier effects produced.
    #[tokio::test(flavor = "multi_thread")]
    async fn validation_accepts_earlier_write() {
        let queue = ChannelClaimQueue::<u8, u64>::new();
        queue.set_validation_enabled(true);
        queue.record_write(&1u8, &2, true);
        let guard = queue.claim(4, &[1u8]);
        guard.wait_at_head().await.unwrap();
    }

    /// An unwritten channel validates (its datum, if any, is the initial state).
    #[tokio::test(flavor = "multi_thread")]
    async fn unwritten_channel_acquires() {
        let queue = ChannelClaimQueue::<u8, u64>::new();
        queue.set_validation_enabled(true);
        let guard = queue.claim(4, &[1u8]);
        guard.wait_at_head().await.unwrap();
    }

    /// Validation off (the default; sequential, gate, and pure relaxed modes) accepts later-path
    /// writes — the certificate fires only in the validated block-path mode.
    #[tokio::test(flavor = "multi_thread")]
    async fn validation_off_accepts_later_write() {
        let queue = ChannelClaimQueue::<u8, u64>::new();
        queue.record_write(&1u8, &5, true);
        let guard = queue.claim(4, &[1u8]);
        guard.wait_at_head().await.unwrap();
    }

    /// The produce phase-two re-wait skips validation on channels the claim already holds: the
    /// claim's own trigger write (stamped at its own — equal — path) must not invalidate the
    /// re-wait, while the new join channel is validated normally.
    #[tokio::test(flavor = "multi_thread")]
    async fn phase_two_rewait_skips_own_trigger_write() {
        let queue = ChannelClaimQueue::<u8, u64>::new();
        queue.set_validation_enabled(true);
        let mut guard = queue.claim(5, &[1u8]);
        let _lease = guard.wait_at_head().await.unwrap();
        // The reducer stamps the trigger write (writer path == the claim's own path — equal,
        // not strictly earlier) after the produce commits.
        queue.record_write(&1u8, &5, true);
        guard.claim_more(&[2u8]); // join channel: unwritten, validates
        let _lease = guard.wait_at_head().await.unwrap();
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
                let _lease = guard.wait_at_head().await.unwrap();
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
