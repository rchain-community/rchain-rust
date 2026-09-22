//! A minimal fixed-window rate limiter.
//!
//! Shared by the unauthenticated deploy gRPC/HTTP servers and the Kademlia discovery RPC to bound
//! request rate (documented Scala deviations: those surfaces are unlimited in Scala).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A minimal fixed-window rate limiter (bounded requests per second).
pub struct RateLimiter {
    max_per_sec: u64,
    window_start: Mutex<Instant>,
    count: AtomicU64,
}

impl RateLimiter {
    pub fn new(max_per_sec: u64) -> Self {
        RateLimiter {
            max_per_sec,
            window_start: Mutex::new(Instant::now()),
            count: AtomicU64::new(0),
        }
    }

    /// Admit a request if the current one-second window has capacity.
    pub fn allow(&self) -> bool {
        let now = Instant::now();
        let mut start = self.window_start.lock().unwrap_or_else(|p| p.into_inner());
        if now.duration_since(*start) >= Duration::from_secs(1) {
            *start = now;
            self.count.store(0, Ordering::SeqCst);
        }
        self.count.fetch_add(1, Ordering::SeqCst) < self.max_per_sec
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The window admits exactly `max_per_sec` requests: one more and the surface is closed for the
    /// rest of the second. This is the DoS bound the unauthenticated deploy/faucet routes rest on,
    /// and nothing tested it.
    #[test]
    fn admits_exactly_max_per_window_then_refuses() {
        let limiter = RateLimiter::new(3);
        assert!(limiter.allow(), "first");
        assert!(limiter.allow(), "second");
        assert!(limiter.allow(), "third");
        assert!(!limiter.allow(), "the fourth must be refused");
        assert!(!limiter.allow(), "and so must the fifth");
    }

    /// Zero means the surface is closed, not unlimited: `0 < 0` is false, so even the first request
    /// is refused. A limiter configured to zero protecting *less* than one configured to one would
    /// be exactly backwards.
    #[test]
    fn zero_never_admits() {
        let limiter = RateLimiter::new(0);
        for _ in 0..5 {
            assert!(!limiter.allow());
        }
    }

    /// The window rolls over after a second, so a client that was refused is not refused forever.
    /// (The single sleep is the cost of testing a wall-clock window without injecting a clock.)
    #[test]
    fn resets_after_its_window() {
        let limiter = RateLimiter::new(1);
        assert!(limiter.allow(), "the first of the window");
        assert!(!limiter.allow(), "the second is refused");
        std::thread::sleep(Duration::from_millis(1100));
        assert!(limiter.allow(), "the next window admits again");
    }
}
