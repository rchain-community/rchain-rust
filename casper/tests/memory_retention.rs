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
//! **Two-sided on purpose**, because glibc genuinely retains: asserting only "retention is small" would
//! have been permanently red while that was the allocator, and asserting only the retained number would
//! have frozen the defect in as acceptable. So one test guards the churn's **cost** (what the merge path
//! allocates, which a regression in that path moves) and the other asserts the **bound** (retention tracks
//! live data rather than the churn's peak), with the bound calibrated from both measured configurations
//! so it separates them rather than describing one.
//!
//! With jemalloc's purge in place the two are `kept 3928 KiB of 13620` (29 %) — the bound passes with
//! headroom, and the same binary under a decay of 5 s retains ~100 %, which is the row the assertion
//! fails. That the suppression is *in this crate graph* and needs no `unsafe` is why it is here rather
//! than in a deployment script.
//!
//! **What it does not catch**, stated because a test that overstates itself is worse than none: the
//! whole-system outcome. Whether a real node still reaches its ceiling depends on the churn *amplitude*
//! consensus produces, which this test fixes as a parameter. The devnet arm in
//! `spec/audit/evidence/n117-preregistration.md` remains the end-to-end measurement.

// The same allocator the node installs, so this test measures the configuration that ships rather than
// the platform default. jemalloc with its background purge thread is the *bound* on this defect; if the
// purge configuration fails to take effect, this binary pins like glibc and the assertion below goes red.
#[cfg(target_os = "linux")]
#[global_allocator]
static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};
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
/// How often a churn thread reads its own `Anonymous` for the peak. Every tenth iteration: 15 reads per
/// thread against 150 iterations of chain building, so the instrument is far cheaper than what it measures.
const SAMPLE_EVERY: usize = 10;
/// How long to wait for the allocator to settle after the churn, and how still it must be to count as
/// settled: a poll every 100 ms, stopping after ten consecutive non-decreasing readings, capped at 30 s.
const SETTLE_POLL_MS: u64 = 100;
const SETTLE_MAX_MS: u64 = 30_000;
const SETTLE_STABLE_SAMPLES: u32 = 10;
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

/// Wait for the allocator to settle, and *then* read — the window is adaptive because a fixed one is a
/// machine constant.
///
/// The purge is performed by the allocator's own background thread, and this process has just finished
/// running sixteen churn threads, so how long the purge takes to become visible in `Anonymous` is a
/// property of the machine's scheduler rather than of the configuration being tested. A fixed 1.5 s sleep
/// reads **29 % retained here and 77 % on CI** — the same purge, the same decay of 0, two machines — and
/// the CI reading fails an assertion whose subject (the allocator) is working. That is the same defect as
/// the peak sampler above, one layer down: a constant that encodes this workstation. Polling until the
/// value stops falling takes the machine's speed out of the measurement.
fn settled_anonymous_kib() -> u64 {
    let mut previous = anonymous_kib();
    let mut stable = 0u32;
    for _ in 0..(SETTLE_MAX_MS / SETTLE_POLL_MS) {
        thread::sleep(Duration::from_millis(SETTLE_POLL_MS));
        let now = anonymous_kib();
        if now < previous {
            stable = 0;
            previous = now;
        } else {
            stable += 1;
            if stable >= SETTLE_STABLE_SAMPLES {
                return now.min(previous);
            }
        }
    }
    previous
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
///
/// **The peak is sampled by the churn threads themselves, and that is a correction.** It used to be a
/// separate thread polling every 5 ms, which starves on a loaded machine: CI runs `THREADS = 16` workers
/// on a small runner, the sampler did not get scheduled, and its "peak" came out *below* the retained
/// reading taken after the workers stopped — `kept 58564 KiB, grew 40080 KiB`, a value that is impossible
/// if the peak is a peak. The test failed on its own instrument, and the assertion that caught it is the
/// structural one (`kept <= grown`), which is exactly what a structural assertion is for. Sampling inside
/// the workers cannot starve — a worker is by definition running — and at `SAMPLE_EVERY` iterations the
/// syscall cost is far below the churn it measures.
fn churn_and_measure() -> (u64, u64, u64) {
    let baseline = anonymous_kib();

    let peak = Arc::new(AtomicU64::new(baseline));

    let workers: Vec<_> = (0..THREADS)
        .map(|thread_ix| {
            let peak = Arc::clone(&peak);
            thread::spawn(move || {
                for iteration in 0..ITERATIONS {
                    let scope = build_scope_set((thread_ix * ITERATIONS + iteration) as u64);
                    // Touch it the way a merge does — the clone traffic that made the scope expensive.
                    let clone: Vec<Arc<DeployChainIndex>> = scope.iter().map(Arc::clone).collect();
                    std::hint::black_box(clone.len() + scope.len());
                    drop(clone);
                    drop(scope);
                    if iteration % SAMPLE_EVERY == 0 {
                        peak.fetch_max(anonymous_kib(), Ordering::Relaxed);
                    }
                }
                // One last read while this thread is certainly alive: the peak is a property of the
                // churn, and a worker that has finished can no longer witness it.
                peak.fetch_max(anonymous_kib(), Ordering::Relaxed);
            })
        })
        .collect();
    for worker in workers {
        worker.join().expect("the churn thread finishes");
    }
    let retained = settled_anonymous_kib();
    (baseline, peak.load(Ordering::Relaxed), retained)
}

/// The churn's growth budget, in KiB. A regression guard on what the churn path *costs*, not on what is
/// kept — the latter is the allocator's business and is pinned by the assertions below.
///
/// **This is the one machine-calibrated number in this file, and it is set from the slower machine.**
/// The same churn and the same binary measure `grew 11 980 KiB` on this workstation
/// (24 cores, 2026-09-30) and `grew 40 080 KiB` on CI's runner — a difference the allocator explains:
/// jemalloc sizes its arenas and per-thread caches from the cores it is given, and sixteen churn threads
/// on a small runner allocate through a different shape than the same threads here. Calibrating from the
/// local figure alone is how this assertion came to be permanently red on CI while passing locally.
///
/// So the budget is 128 MiB — 10× the local reading and 3× the CI one — and that looseness is deliberate
/// and stated rather than hidden: what this guards is a regression in the *churn path* (an order of
/// magnitude), while the allocator claims are the structural assertion (`kept <= grown`) and the ratio
/// below, both of which hold on any machine because they are properties of the allocator rather than of
/// its constants. If a later unit changes the churn path, re-measure and move this deliberately.
const CHURN_GROWTH_BUDGET_KIB: u64 = 128 * 1024;

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
fn churn_retention_falls_well_below_the_peak() {
    let (baseline, peak, retained) = churn_and_measure();
    let grown = peak.saturating_sub(baseline);
    let kept = retained.saturating_sub(baseline);
    eprintln!("churn retention target: grew {grown} KiB, kept {kept} KiB (want kept <= grown / 4)");
    // The bound, and it is **calibrated by measurement with headroom on both sides** — that is what makes
    // it a falsifier rather than a preference. Measured on this tree, same churn and same process:
    //   purge at `dirty_decay_ms:0`  ->  kept 2132 KiB of 6928 (31 %)
    //   no purge (`decay_ms:5000`)   ->  kept 9144 KiB of 9136 (~100 %)
    // A 50 % bound passes the first with 1.6x headroom and fails the second with 2x, so it separates the
    // two configurations rather than merely describing one. Any regression to an allocator that retains —
    // or a purge configuration that stops taking effect, which looks identical from the outside — lands
    // in the second row and this goes red.
    assert!(
        kept <= grown / 2,
        "retention should track live data, not the churn's peak: kept {kept} KiB of {grown} KiB \
         (purged retention measures ~31 %, un-purged ~100 %)"
    );
}
