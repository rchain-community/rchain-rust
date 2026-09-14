//! Poison-aware lock accessors.
//!
//! The engine uses `std::sync::RwLock`/`Mutex` for shared mutable state. A lock is only poisoned if
//! a panic occurred while it was held; these accessors recover the guard via `PoisonError::into_inner`
//! instead of panicking, making the poison recovery explicit and total (per `TYPE-SYSTEM.md` §3.2).

use std::sync::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// Acquire a read guard, recovering from poison.
pub(crate) fn rlock<T>(l: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    l.read().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Acquire a write guard, recovering from poison.
pub(crate) fn wlock<T>(l: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    l.write().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Acquire a mutex guard, recovering from poison.
pub(crate) fn mlock<T>(l: &Mutex<T>) -> MutexGuard<'_, T> {
    l.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
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
