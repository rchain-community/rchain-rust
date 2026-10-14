//! A token-bucket rate limiter.
//!
//! Shared by the unauthenticated deploy gRPC/HTTP servers and the Kademlia discovery RPC to bound
//! request rate (documented Scala deviations: those surfaces are unlimited in Scala), and by the
//! OCapN listener's audit lines.
//!
//! **It was a fixed window until 2026-10-07** (AUDIT C232). A window admits its whole allowance at
//! once and then resets on the wall clock, so a client that keeps asking is granted a fresh allowance
//! at each boundary — up to **twice** the configured rate over a sliding second, indefinitely, on a
//! surface whose whole purpose is to bound a rate. A bucket refills only what was spent, so the same
//! client is held to the rate it was given; the burst a fixed window allowed after an *idle* period
//! is preserved, because the bucket starts full.

use crate::lock::Unpoison;
use std::collections::HashMap;
use std::sync::Mutex;

// The wasm build takes the host's clock (`web-time`); `std`'s panics there (issue #98).
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

/// A token bucket holding at most one second's allowance, refilling continuously at the configured
/// rate.
pub struct RateLimiter {
    max_per_sec: u64,
    /// The tokens in hand, and when they were last accounted for. One lock rather than an `AtomicU64`
    /// beside a `Mutex`: the count and the instant are two halves of one fact, and a fixed window
    /// could get away with splitting them only because its reset was a store.
    state: Mutex<(f64, Instant)>,
}

impl RateLimiter {
    pub fn new(max_per_sec: u64) -> Self {
        RateLimiter {
            max_per_sec,
            // **Full at construction**, so a surface that has been quiet grants its whole allowance at
            // once — the burst the fixed window's first tick gave, and the reason a limiter sized for
            // an interactive caller does not make the first call wait.
            state: Mutex::new((max_per_sec as f64, Instant::now())),
        }
    }

    /// Admit a request if a token is available, refilling at the configured rate.
    pub fn allow(&self) -> bool {
        let now = Instant::now();
        let mut state = self.state.lock().unpoison();
        let (tokens, last) = &mut *state;
        let refill = now.duration_since(*last).as_secs_f64() * self.max_per_sec as f64;
        *tokens = (*tokens + refill).min(self.max_per_sec as f64);
        *last = now;
        if *tokens >= 1.0 {
            *tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// A token bucket **per key**, so one caller cannot spend another's allowance.
///
/// The deploy ingress's bound was a single bucket for the whole node: it was keyed on the node, not
/// on the caller, so one identity's burst refused every other identity's deploys — the bound measured
/// the wrong subject. Here the subject is explicit, which is also what makes the bound *per reachable
/// identity* rather than per process.
///
/// **The map is itself a resource**, and on an unauthenticated surface it would be the next unbounded
/// one, so `max_keys` caps it. A bucket that has refilled to full carries no information — it is
/// indistinguishable from a key never seen — so it is dropped rather than held; and when the map is
/// at its cap with no such bucket to drop, a new key is refused rather than admitted. Fail-closed,
/// because the alternative is an allocation the caller sizes.
pub struct KeyedRateLimiter {
    max_per_sec: u64,
    max_keys: usize,
    buckets: Mutex<HashMap<Vec<u8>, (f64, Instant)>>,
}

impl KeyedRateLimiter {
    pub fn new(max_per_sec: u64, max_keys: usize) -> Self {
        KeyedRateLimiter {
            max_per_sec,
            max_keys,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Admit a request from `key` if that key has a token. An unseen key starts full.
    pub fn allow(&self, key: &[u8]) -> bool {
        let now = Instant::now();
        let full = self.max_per_sec as f64;
        let mut buckets = self.buckets.lock().unpoison();

        // The cap is enforced before the entry is taken, so the `None` arm below means "not seen"
        // rather than "refused" — and it is total: no panic, and no unreachable arm to leave.
        if !buckets.contains_key(key) && buckets.len() >= self.max_keys {
            // Drop the buckets that have refilled to full: they say nothing an unseen key does not,
            // so keeping them is what would let the map grow without bound.
            buckets.retain(|_, (tokens, last)| {
                *tokens + now.duration_since(*last).as_secs_f64() * full < full
            });
            if buckets.len() >= self.max_keys {
                return false;
            }
        }

        match buckets.get_mut(key) {
            Some((tokens, last)) => {
                let refill = now.duration_since(*last).as_secs_f64() * full;
                *tokens = (*tokens + refill).min(full);
                *last = now;
                if *tokens >= 1.0 {
                    *tokens -= 1.0;
                    true
                } else {
                    false
                }
            }
            None => {
                // **An unseen key starts with a full bucket, and a full bucket of less than one
                // token admits nothing.** This is `RateLimiter`'s own "zero means closed, not
                // unlimited" rule, and the keyed form broke it: it spent a token from `full`
                // unconditionally, so a limiter configured to `0` — a plausible way to turn one
                // ingress's bound off — admitted the first request from *every* key, which is a
                // fail-open on the surface whose whole purpose is to bind a rate. Refused keys are
                // not inserted, so the map still cannot be grown by a caller whose requests are all
                // refused.
                if full >= 1.0 {
                    buckets.insert(key.to_vec(), (full - 1.0, now));
                    true
                } else {
                    false
                }
            }
        }
    }

    /// The number of keys currently tracked. For tests and for the node's own gauges.
    pub fn tracked_keys(&self) -> usize {
        self.buckets.lock().unpoison().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The tests are the only code that sleeps; the limiter itself reads the clock through `Instant`.
    use std::time::Duration;

    /// The bucket admits exactly `max_per_sec` requests: one more and the surface is closed until a
    /// token refills. This is the DoS bound the unauthenticated deploy/faucet routes rest on, and
    /// nothing tested it.
    #[test]
    fn admits_exactly_max_per_window_then_refuses() {
        let limiter = RateLimiter::new(3);
        assert!(limiter.allow(), "first");
        assert!(limiter.allow(), "second");
        assert!(limiter.allow(), "third");
        assert!(!limiter.allow(), "the fourth must be refused");
        assert!(!limiter.allow(), "and so must the fifth");
    }

    /// Zero means the surface is closed, not unlimited: the bucket holds nothing and refills at a
    /// rate of zero, so even the first request is refused. A limiter configured to zero protecting
    /// *less* than one configured to one would be exactly backwards.
    #[test]
    fn zero_never_admits() {
        let limiter = RateLimiter::new(0);
        for _ in 0..5 {
            assert!(!limiter.allow());
        }
    }

    /// **And neither does the keyed form** — the same rule, on the limiter whose whole reason for
    /// existing is that one caller must not spend another's allowance.
    ///
    /// This was a fail-open: the unseen-key arm spent a token from `full` unconditionally, so
    /// `KeyedRateLimiter::new(0, …)` admitted the *first* request from every key it ever saw — a
    /// limiter configured to zero admitting one request per caller, on an unauthenticated surface,
    /// with the map still refusing to grow so nothing looked wrong. Found while wiring the HTTP
    /// ingress onto this limiter, which is exactly where a bound of zero would have been used.
    #[test]
    fn a_keyed_limiter_of_zero_admits_nothing_either() {
        let limiter = KeyedRateLimiter::new(0, 8);
        for key in [b"alice".as_slice(), b"bob".as_slice(), b"carol".as_slice()] {
            assert!(
                !limiter.allow(key),
                "a limiter of zero must refuse every key, including an unseen one"
            );
        }
        assert_eq!(
            limiter.tracked_keys(),
            0,
            "and a refused key must not be tracked, or the map is grown by callers it refuses"
        );
    }

    /// The bucket refills, so a client that was refused is not refused forever.
    /// (The single sleep is the cost of testing a wall clock without injecting one.)
    #[test]
    fn refills_after_its_period() {
        let limiter = RateLimiter::new(1);
        assert!(limiter.allow(), "the first token");
        assert!(!limiter.allow(), "the second has nothing to spend");
        std::thread::sleep(Duration::from_millis(1100));
        assert!(limiter.allow(), "a second has refilled one");
    }

    /// **The allowance is never granted twice** (AUDIT C232). Under the fixed window this limiter
    /// used to be, a client that kept asking was handed a whole fresh allowance at each window
    /// boundary — twice the configured rate over a sliding second, on the surfaces whose entire
    /// purpose is to bound a rate. A bucket refills only what was spent, so a fraction of the period
    /// buys a fraction of a token and the whole allowance is never handed out twice. **Measured
    /// failing against the fixed window**: half a second into a one-second window it refused, because
    /// nothing refills until the boundary.
    #[test]
    fn the_allowance_is_never_granted_twice() {
        let limiter = RateLimiter::new(2);
        assert!(limiter.allow() && limiter.allow(), "the whole allowance");
        assert!(!limiter.allow(), "spent");
        std::thread::sleep(Duration::from_millis(500));
        assert!(
            limiter.allow(),
            "half the period at two per second buys one token"
        );
        assert!(
            !limiter.allow(),
            "and one only: the allowance was refilled, not re-granted"
        );
    }

    /// **One key cannot spend another key's allowance.** This is the defect the keyed limiter exists
    /// for: as a single bucket for the whole node, one identity's burst refused every other identity's
    /// deploys — the bound held, and it held the wrong subject. Exhaust `alice` completely; `bob` must
    /// be untouched.
    #[test]
    fn one_key_cannot_spend_another_keys_allowance() {
        let limiter = KeyedRateLimiter::new(2, 8);
        assert!(limiter.allow(b"alice"), "alice's first");
        assert!(limiter.allow(b"alice"), "alice's second");
        assert!(!limiter.allow(b"alice"), "alice is spent");
        assert!(
            limiter.allow(b"bob"),
            "bob's allowance is his own; alice's flood must not refuse him"
        );
    }

    /// **The map is bounded**: an unauthenticated caller can present keys as fast as it likes, so the
    /// tracking map is the next unbounded resource. At `max_keys` distinct keys, a further new key is
    /// refused rather than admitted.
    #[test]
    fn the_map_is_bounded_and_refuses_new_keys_at_capacity() {
        let limiter = KeyedRateLimiter::new(2, 2);
        assert!(limiter.allow(b"a"));
        assert!(limiter.allow(b"b"));
        assert_eq!(limiter.tracked_keys(), 2);
        assert!(
            !limiter.allow(b"c"),
            "a third distinct key is refused rather than growing the map"
        );
        assert!(
            limiter.allow(b"a"),
            "and a known key is unaffected by the cap"
        );
    }

    /// A bucket that has refilled to full carries nothing an unseen key does not, so it is dropped —
    /// which is what keeps a stream of one-shot keys from filling the map permanently.
    #[test]
    fn a_refilled_bucket_is_dropped_so_one_shot_keys_do_not_fill_the_map() {
        let limiter = KeyedRateLimiter::new(100, 2);
        assert!(limiter.allow(b"one-shot"));
        std::thread::sleep(Duration::from_millis(30)); // 3 tokens at 100/s: back to full
        assert!(limiter.allow(b"b"));
        assert!(
            limiter.allow(b"c"),
            "the spent-and-refilled key was dropped, so the map was not full"
        );
    }
}
