//! Poison-aware lock accessors.
//!
//! The engine uses `std::sync::RwLock`/`Mutex` for shared mutable state. A lock is only poisoned if
//! a panic occurred while it was held; these accessors recover the guard via `PoisonError::into_inner`
//! instead of panicking, making the poison recovery explicit and total (per `TYPE-SYSTEM.md` §3.2).
//!
//! **Recovering is deliberate; recovering *invisibly* was not.** A poison means a panic happened while
//! shared state was held, which is a serious event for a node — and until 2026-10-09 nothing recorded
//! it: no log, no counter, no surface, so an operator could not tell a node that had taken a panic
//! inside a store lock from one that had not. The failure-mode HAZOP named that as one of only two
//! genuinely silent paths in the node (`C249`'s F-U9-03). This counter is the **source half** of that
//! finding: the event is now recorded. Surfacing it where an operator reads it is the next unit — the
//! same one that puts the finality-stall reason on `/api/status` — so "counted but not yet published"
//! is a stated step, not a claim that this is loud.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// How many times this process has recovered a poisoned lock.
///
/// Process-wide because the poisoning is: the panic happened in whichever thread held the guard.
/// Monotone, so a reader can compare two observations rather than race one.
static POISON_RECOVERIES: AtomicU64 = AtomicU64::new(0);

/// The number of poisoned-lock recoveries this process has performed.
pub fn poison_recoveries() -> u64 {
    POISON_RECOVERIES.load(Ordering::Relaxed)
}

fn note_poison() {
    POISON_RECOVERIES.fetch_add(1, Ordering::Relaxed);
}

/// Acquire a read guard, recovering from poison **and counting the recovery**.
pub(crate) fn rlock<T>(l: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    match l.read() {
        Ok(guard) => guard,
        Err(poisoned) => {
            note_poison();
            poisoned.into_inner()
        }
    }
}

/// Acquire a write guard, recovering from poison **and counting the recovery**.
pub(crate) fn wlock<T>(l: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    match l.write() {
        Ok(guard) => guard,
        Err(poisoned) => {
            note_poison();
            poisoned.into_inner()
        }
    }
}

/// Acquire a mutex guard, recovering from poison **and counting the recovery**.
pub(crate) fn mlock<T>(l: &Mutex<T>) -> MutexGuard<'_, T> {
    match l.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            note_poison();
            poisoned.into_inner()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Poison a `Mutex` the way the real code would: panic while a guard is held.
    fn poison(mutex: &Mutex<u8>) {
        let _ = std::thread::scope(|s| {
            s.spawn(|| {
                let _guard = mutex.lock().expect("unpoisoned");
                panic!("while holding the lock");
            })
            .join()
        });
    }

    /// Poison an `RwLock` by panicking while a write guard is held.
    fn poison_rw(lock: &RwLock<u8>) {
        let _ = std::thread::scope(|s| {
            s.spawn(|| {
                let _guard = lock.write().expect("unpoisoned");
                panic!("while holding the write lock");
            })
            .join()
        });
    }

    /// A panic while the lock is held poisons it; `std`'s own accessors then return `Err` and
    /// `unwrap` would panic *again*, turning a recovered-from fault into a crash loop. These
    /// accessors recover the guard instead (TYPE-SYSTEM.md §3.2: no silent partiality, and no
    /// gratuitous panic either) — the data is still there, so the operation is total.
    #[test]
    fn a_poisoned_lock_still_yields_its_guard() {
        let mutex = Mutex::new(7u8);
        poison(&mutex);
        assert!(mutex.lock().is_err(), "the mutex really is poisoned");
        assert_eq!(*mlock(&mutex), 7, "the value survived the poisoning");

        let rw = RwLock::new(9u8);
        poison_rw(&rw);
        assert!(rw.read().is_err(), "the rwlock really is poisoned");
        assert_eq!(*rlock(&rw), 9);
        assert_eq!(*wlock(&rw), 9);
    }

    /// **A poisoned recovery must not be silent.**
    ///
    /// The guard still comes back — that is the module's deliberate decision, pinned by the test above —
    /// but the *event* is now counted, so a node that has taken a panic inside a store lock can be told
    /// from one that has not (C249's F-U9-03; the two silent paths the failure-mode HAZOP found).
    ///
    /// Falsifier, both forms. On the old accessors (`unwrap_or_else(PoisonError::into_inner)`) there was
    /// nothing to observe: no counter existed and no accessor exposed one, so this test could not be
    /// written at all — which is exactly what "silent" meant. Post-fix the count moves once per
    /// recovery, on each of the three accessors.
    ///
    /// The assertions are `>` rather than `==` because the counter is **process-wide** and libtest runs
    /// this module's tests concurrently, so another test's poisoning is a legitimate concurrent
    /// increment. That a recovery moves the count is the property; its exact value is shared state.
    #[test]
    fn a_poisoned_recovery_is_counted() {
        let before = poison_recoveries();

        // **Every guard here is a temporary, and that is load-bearing.** A named binding would hold the
        // read guard across the `wlock` below, and `std::sync::RwLock` is not reentrant — upgrading
        // deadlocks the thread. The sibling test above uses the same form; this note is here because the
        // first draft of *this* test named its guards and hung for fifteen minutes.
        let mutex = Mutex::new(1u8);
        poison(&mutex);
        assert_eq!(*mlock(&mutex), 1, "the guard still comes back");
        assert!(
            poison_recoveries() > before,
            "…and the recovery is counted ({before} -> {})",
            poison_recoveries()
        );

        let after_mutex = poison_recoveries();
        let rw = RwLock::new(2u8);
        poison_rw(&rw);
        assert_eq!(*rlock(&rw), 2, "the read guard still comes back");
        assert!(
            poison_recoveries() > after_mutex,
            "…and the read recovery is counted"
        );

        let after_read = poison_recoveries();
        assert_eq!(*wlock(&rw), 2, "the write guard still comes back");
        assert!(
            poison_recoveries() > after_read,
            "…and the write recovery is counted"
        );
    }

    /// The unpoisoned path is the ordinary one, and the guards are real: a write through `wlock` is
    /// visible to a later `rlock`, which is what every caller assumes.
    #[test]
    fn an_unpoisoned_lock_behaves_like_the_std_one() {
        let mutex = Mutex::new(1u8);
        *mlock(&mutex) += 1;
        assert_eq!(*mlock(&mutex), 2);

        let rw = RwLock::new(1u8);
        *wlock(&rw) += 41;
        assert_eq!(*rlock(&rw), 42);
    }
}
