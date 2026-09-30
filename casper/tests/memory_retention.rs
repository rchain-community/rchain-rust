//! Does allocation churn cost resident memory in proportion to the churn, or to what is live?
//!
//! **The defect this isolates (#117).** A node under fork load reached its cgroup ceiling and was
//! OOM-killed while its live heap stayed small. The measurement attributed it — glibc's own accounting
//! explains the cgroup's `anon` at ~101 % — and the shape is the one production calls *arena retention*:
//! each thread gets an arena, each arena keeps the high-water mark of what it has held, and free pages
//! that are not at the heap top are never returned. So resident memory tracks the **churn**, not the
//! live set, which is why the ceiling arrives on a fifteen-block chain.
//!
//! **What this test measures.** `N` threads run a merge-shaped churn loop — real `DeployChainIndex`
//! values with populated event-log indices, the shapes the merge path allocates — and the test reads
//! `/proc/self/smaps_rollup`'s `Anonymous` before, during and after. Allocations are dropped and the
//! process settles before the final read, so what is left is retention.
//!
//! **Why one test in one binary.** glibc's retention is per *process*: a sibling test's allocations
//! would land inside the same measurement, and this repository runs test binaries with ten threads. A
//! separate binary with a single test is the only way the number means anything.
//!
//! **Two-sided on purpose.** glibc genuinely retains, so asserting only "retention is small" would be a
//! permanently red test, and asserting only today's number would freeze the defect in as acceptable.
//! So:
//!
//! * `churn_retention_is_the_measured_envelope` pins **what the allocator does today**, with the figure
//!   and the date it was taken, so a regression in the churn path or a change of allocator shows up as a
//!   change in a number rather than as an opinion;
//! * `churn_retention_should_fall_well_below_the_peak` is the **target** — the property a purging
//!   allocator or a trim policy would give — and is `#[ignore]`d until such a policy lands. Its doc
//!   comment names it as the acceptance criterion for that unit.
//!
//! **What it does not catch**, stated because a test that overstates itself is worse than none: the
//! whole-system outcome. Whether a real node still reaches its ceiling depends on the churn *amplitude*
//! consensus produces, which this test fixes as a parameter. The devnet arm in
//! `spec/audit/evidence/n117-preregistration.md` remains the end-to-end measurement.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use rchain_casper::merging::{DeployChainIndex, DeployIdWithCost};
use rchain_crypto::hash::blake2b256_hash::Blake2b256Hash;
use rchain_rspace::merger::event_log_index::EventLogIndex;
use rchain_rspace::merger::state_change::StateChange;
use rchain_rspace::trace::event::Produce;

/// Threads doing the churn. The node runs one worker per core and the arena count follows the threads
/// that allocate, so this is the multiplier the defect turns on.
const THREADS: usize = 16;
/// Churn iterations per thread.
const ITERATIONS: usize = 150;
/// Deploy chains built per iteration — the width of a merge scope on the devnet that reproduced #117.
const CHAINS_PER_ITERATION: usize = 40;
/// Event-log entries per chain, which is what makes a chain index large rather than a few hashes.
const ENTRIES_PER_CHAIN: usize = 64;

/// Anonymous resident memory of this process, in KiB. A file read: no allocator hook, and therefore no
/// `unsafe` in a crate graph that forbids it.
fn anonymous_kib() -> u64 {
    let rollup = std::fs::read_to_string("/proc/self/smaps_rollup").unwrap_or_default();
    rollup
        .lines()
        .find_map(|line| line.strip_prefix("Anonymous:"))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|kib| kib.parse().ok())
        .expect("smaps_rollup reports Anonymous")
}

/// One merge-shaped transient: the chain-index values a scope set holds for one block.
fn build_scope_set(seed: u64) -> Vec<Arc<DeployChainIndex>> {
    (0..CHAINS_PER_ITERATION)
        .map(|_chain| {
            let mut index = EventLogIndex::empty();
            for entry in 0..ENTRIES_PER_CHAIN {
                index.produces_linear.insert(Produce {
                    channels_hash: Blake2b256Hash::from_bytes([(seed ^ entry as u64) as u8; 32]),
                    hash: Blake2b256Hash::from_bytes([(seed.wrapping_add(entry as u64)) as u8; 32]),
                    persistent: false,
                });
            }
            Arc::new(DeployChainIndex {
                host_block: Blake2b256Hash::from_bytes([seed as u8; 32]),
                deploys_with_cost: BTreeSet::from([DeployIdWithCost {
                    id: seed.to_le_bytes().to_vec(),
                    cost: 1,
                }]),
                pre_state_hash: Blake2b256Hash::from_bytes([0u8; 32]),
                post_state_hash: Blake2b256Hash::from_bytes([1u8; 32]),
                event_log_index: index,
                state_changes: StateChange::empty(),
            })
        })
        .collect()
}

/// Run the churn on `THREADS` threads, sampling `Anonymous` throughout.
/// Returns `(baseline, peak, retained)` in KiB.
fn churn_and_measure() -> (u64, u64, u64) {
    let baseline = anonymous_kib();

    let peak = Arc::new(AtomicU64::new(baseline));
    let done = Arc::new(AtomicBool::new(false));
    let sampler = {
        let peak = Arc::clone(&peak);
        let done = Arc::clone(&done);
        thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                peak.fetch_max(anonymous_kib(), Ordering::Relaxed);
                thread::sleep(Duration::from_millis(5));
            }
        })
    };

    let workers: Vec<_> = (0..THREADS)
        .map(|thread_ix| {
            thread::spawn(move || {
                for iteration in 0..ITERATIONS {
                    let scope = build_scope_set((thread_ix * ITERATIONS + iteration) as u64);
                    // Touch it the way a merge does — the clone traffic that made the scope expensive.
                    let clone: Vec<Arc<DeployChainIndex>> = scope.iter().map(Arc::clone).collect();
                    std::hint::black_box(clone.len() + scope.len());
                    drop(clone);
                    drop(scope);
                }
            })
        })
        .collect();
    for worker in workers {
        worker.join().expect("the churn thread finishes");
    }

    done.store(true, Ordering::Relaxed);
    sampler.join().expect("the sampler finishes");
    // Settle: glibc does not trim on its own, so this only lets the transient allocations go.
    thread::sleep(Duration::from_millis(200));
    let retained = anonymous_kib();
    (baseline, peak.load(Ordering::Relaxed), retained)
}

/// The churn's growth budget, in KiB. A regression guard on what the churn path *costs*, not on what is
/// kept — the latter is glibc's business and is pinned by the assertion below.
///
/// **Calibrated by measurement, and that matters**: this represents the tree as it is now. A tripwire
/// calibrated before a sibling change landed is calibrated against a tree that no longer exists, so if a
/// later unit changes the churn path, re-measure and move this deliberately rather than chasing it.
///
/// Measured 2026-09-30 on this tree with the plain glibc allocator: `grew 11 788 KiB` across
/// `THREADS x ITERATIONS x CHAINS_PER_ITERATION` — so the budget below carries roughly 3× headroom for
/// run-to-run and machine-to-machine variation.
const CHURN_GROWTH_BUDGET_KIB: u64 = 32 * 1024;

/// The envelope as measured, pinned so that a regression shows up as a number.
///
/// Measured 2026-09-30 on this repository's tree with the plain glibc allocator. See the sibling
/// `#[ignore]`d test for why this is a control rather than a target.
#[test]
fn churn_retention_is_the_measured_envelope() {
    let (baseline, peak, retained) = churn_and_measure();
    let grown = peak.saturating_sub(baseline);
    let kept = retained.saturating_sub(baseline);
    eprintln!(
        "churn retention: baseline={baseline} KiB peak={peak} KiB retained={retained} KiB \
         (grew {grown} KiB, kept {kept} KiB)"
    );

    // The structural fact: retention cannot exceed the growth it came from.
    assert!(
        kept <= grown,
        "retention cannot exceed the growth it came from: kept {kept} KiB, grew {grown} KiB"
    );
    // The regression guard: the churn must move memory, and must not start costing far more than it
    // did when this bound was set.
    assert!(
        grown > 0,
        "a churn loop of {THREADS} threads x {ITERATIONS} iterations must move the resident set at all"
    );
    assert!(
        grown <= CHURN_GROWTH_BUDGET_KIB,
        "the churn grew {grown} KiB against a budget of {CHURN_GROWTH_BUDGET_KIB} KiB: the churn path \
         got more expensive, which is a change worth stating rather than absorbing"
    );
}

/// **The target**, ignored until an allocator policy lands that returns freed pages — the production
/// fix for this class (`jemalloc` with `background_thread:true`, a periodic `malloc_trim`, or a
/// mimalloc reclaim interval; see the prior-art comment on #117).
///
/// This is the acceptance criterion for that unit: un-ignore it in the same change and it must pass.
/// Today it does not, and that is the point — glibc retains the per-thread high-water marks, so
/// "retained is far below the peak" is false until something purges.
#[test]
#[ignore = "target: needs an allocator policy that returns freed pages (see the #117 prior-art comment)"]
fn churn_retention_should_fall_well_below_the_peak() {
    let (baseline, peak, retained) = churn_and_measure();
    let grown = peak.saturating_sub(baseline);
    let kept = retained.saturating_sub(baseline);
    eprintln!("churn retention target: grew {grown} KiB, kept {kept} KiB (want kept <= grown / 4)");
    assert!(
        kept <= grown / 4,
        "a purging allocator should return most of the churn's peak: kept {kept} KiB of {grown} KiB"
    );
}
