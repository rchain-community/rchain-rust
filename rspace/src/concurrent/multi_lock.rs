//! Lock a set of keys with ordered acquisition to avoid deadlock.
//!
//! Mirrors `rspace/src/main/scala/coop/rchain/rspace/concurrent/MultiLock.scala`. The per-key
//! `Semaphore[F]` becomes `Arc<tokio::sync::Mutex<()>>` in a `DashMap`.

use std::future::Future;
use std::hash::Hash;
use std::sync::Arc;

use dashmap::DashMap;

/// A set of per-key mutexes, acquired in sorted order (port of `MultiLock`).
pub struct MultiLock<K> {
    locks: DashMap<K, Arc<tokio::sync::Mutex<()>>>,
}

impl<K> MultiLock<K>
where
    K: Eq + Hash + Ord + Clone + Send + Sync + 'static,
{
    pub fn new() -> Self {
        MultiLock {
            locks: DashMap::new(),
        }
    }

    /// Acquire the locks for `keys` (sorted, deduped), run `thunk`, then release (port of `acquire`).
    pub async fn acquire<F>(&self, keys: &[K], thunk: F) -> F::Output
    where
        F: Future + Send,
    {
        let mut sorted: Vec<K> = keys.to_vec();
        sorted.sort();
        sorted.dedup();

        let mut arcs: Vec<Arc<tokio::sync::Mutex<()>>> = Vec::new();
        for key in sorted {
            // **One shard lock across the miss and the insert** — `entry` is what makes this an
            // atomic get-or-insert. The `get`-then-`insert` this replaces let two racers that both
            // missed build two different `Arc<Mutex<()>>` for the same key and lock each of them, so
            // a key's mutual exclusion could be *silently* absent — measured, and the reason the
            // test below releases its racers through a barrier (AUDIT C104).
            let lock = self
                .locks
                .entry(key)
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone();
            arcs.push(lock);
        }

        let mut guards = Vec::new();
        for lock in &arcs {
            guards.push(lock.lock().await);
        }

        let result = thunk.await;
        drop(guards);
        drop(arcs);
        result
    }

    /// Release the per-key mutexes (port of the Scala's `cleanUp`: `Sync[F].delay(locks.clear)`).
    ///
    /// The Scala calls it from `RSpaceOps.reset` ("Clean channel locks"), and the port had dropped
    /// that call — so the map kept one entry for **every distinct channel the node had ever touched**,
    /// for the life of the process, and never released them. Not a leak of memory so much as a leak of
    /// *state*: the entries are what the lock map is, and nothing removed them (AUDIT C104).
    ///
    /// Callers must hold no guards: `reset` is the only caller, and it runs when no operation is in
    /// flight.
    pub fn clean_up(&self) {
        self.locks.clear();
    }

    /// The number of keys the map is holding. Test-only, so that the invariant the Scala's `cleanUp`
    /// exists for — a `reset` leaves nothing behind — can be asserted rather than assumed.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.locks.len()
    }
}

impl<K> Default for MultiLock<K>
where
    K: Eq + Hash + Ord + Clone + Send + Sync + 'static,
{
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn acquire_runs_thunk() {
        let lock = MultiLock::<u8>::new();
        let value = lock.acquire(&[3, 1, 2], async { 42 }).await;
        assert_eq!(value, 42);
    }

    /// **One entry per key, not one per acquire — and `clean_up` leaves nothing behind.**
    ///
    /// `clean_up` is the port of the Scala's `cleanUp` (`locks.clear`), and the reason it exists is
    /// that a long-lived node acquires locks on every channel it ever sees: without the call the map
    /// is a record of every distinct channel since start-up, which is what AUDIT C104 found the port
    /// doing (the Scala calls it from `RSpaceOps.reset`).
    #[tokio::test]
    async fn the_lock_map_holds_one_entry_per_key_and_clean_up_empties_it() {
        let lock = MultiLock::<u8>::new();
        for _ in 0..5 {
            lock.acquire(&[1, 2], async {}).await;
        }
        assert_eq!(
            lock.len(),
            2,
            "one entry per distinct key, not one per acquire"
        );

        lock.clean_up();
        assert_eq!(
            lock.len(),
            0,
            "clean_up is the Scala's cleanUp: the map is cleared"
        );
    }

    /// **One key, one mutex, however many racers miss the `get` at once.**
    ///
    /// A `get` followed by a separate `insert` is not an atomic get-or-insert: two tasks that both
    /// miss build *two different* `Arc<Mutex<()>>` for the same key, and each then locks its own — so
    /// both are inside the critical section at the same time and the mutual exclusion the lock exists
    /// for is gone, silently, for that key. The `Barrier` is what makes this a test rather than a
    /// lottery: every racer is released at the same instant, so they hit the miss window together.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_misses_on_one_key_still_share_one_mutex() {
        const TASKS: usize = 8;
        const ROUNDS: usize = 32;
        let lock = Arc::new(MultiLock::<u8>::new());
        let inside = Arc::new(AtomicUsize::new(0));
        let overlaps = Arc::new(AtomicUsize::new(0));

        for round in 0..ROUNDS {
            // **A fresh key every round**, so every round is a genuine *miss*: a key that is already
            // in the map makes the `get` succeed and the race unreachable — which is how the first
            // draft of this test passed against the broken code.
            let key = round as u8;
            let barrier = Arc::new(tokio::sync::Barrier::new(TASKS));
            let mut handles = Vec::new();
            for _ in 0..TASKS {
                let lock = lock.clone();
                let barrier = barrier.clone();
                let inside = inside.clone();
                let overlaps = overlaps.clone();
                handles.push(tokio::spawn(async move {
                    barrier.wait().await;
                    lock.acquire(&[key], async {
                        if inside.fetch_add(1, Ordering::SeqCst) != 0 {
                            overlaps.fetch_add(1, Ordering::SeqCst);
                        }
                        // Widen the window inside the critical section, so an overlap is observed
                        // rather than merely possible.
                        tokio::task::yield_now().await;
                        inside.fetch_sub(1, Ordering::SeqCst);
                    })
                    .await;
                }));
            }
            for h in handles {
                h.await.expect("no task may panic");
            }
        }

        assert_eq!(
            overlaps.load(Ordering::SeqCst),
            0,
            "two tasks held the same key's lock at the same time: the get-or-insert is not atomic"
        );
    }

    #[tokio::test]
    async fn acquire_serializes_conflicting_keys() {
        let lock = Arc::new(MultiLock::<u8>::new());
        let counter = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::new();
        for _ in 0..10 {
            let lock = lock.clone();
            let counter = counter.clone();
            handles.push(tokio::spawn(async move {
                lock.acquire(&[1u8], async {
                    let v = counter.load(Ordering::SeqCst);
                    counter.store(v + 1, Ordering::SeqCst);
                })
                .await;
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        assert_eq!(counter.load(Ordering::SeqCst), 10);
    }
}
