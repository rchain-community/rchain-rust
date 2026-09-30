//! The merge's conflict resolution was re-expanding a search state once per *ordering* that reached it, and
//! that was the node's memory ceiling (#117).
//!
//! `MergeScope::merge` calls `resolve_conflict_set` once per parent, and a heap profile of a node at its
//! cgroup ceiling attributes **97 %** of the live heap to `BTreeSet` clones inside it — the search's
//! frontier and its accumulated sets, grown 343 -> 1488 MiB within one run. The shape that gets there is a
//! **fork**: chains within a branch do not conflict while chains across branches conflict completely.
//!
//! The ported search carried the rejected set in every queue entry beside the accepted set, and cloned both
//! per push. The rejected set is not independent state — it is always `⋃{conflicts(x) : x ∈ accepted}` — so
//! that only made each state reachable by as many queue entries as there are orderings of it:
//! **19,728,200 expansions for 2,046 states** on a 20-chain fork. Carrying the accepted set alone (see
//! `compute_rejection_options`) expands each state once, which is the whole of the fix measured below.
//!
//! **What is left is the search's true shape, not its redundancy.** Reachable states still grow as
//! 2^(n/2 + 1) on a fork — that is every subset of a branch — so the search remains exponential in the
//! number of chains per branch. The dense case is *not* a problem (accepting any chain rejects all the
//! others, so it terminates in one step), which is why the 1000-node full-graph test passes.

use rchain_sdk::dag::merging::compute_rejection_options;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

/// `branches` groups of `per_branch` chains. Chains in a branch are independent; every pair in different
/// branches conflicts. Returns the conflicts map.
fn fork_shape(branches: usize, per_branch: usize) -> BTreeMap<i32, BTreeSet<i32>> {
    let groups: Vec<Vec<i32>> = (0..branches)
        .map(|b| {
            (0..per_branch)
                .map(|i| (b * per_branch + i) as i32)
                .collect()
        })
        .collect();
    let mut map: BTreeMap<i32, BTreeSet<i32>> = BTreeMap::new();
    for (b, group) in groups.iter().enumerate() {
        for k in group {
            let mut conflicts = BTreeSet::new();
            for (o, other) in groups.iter().enumerate() {
                if o != b {
                    conflicts.extend(other.iter().copied());
                }
            }
            map.insert(*k, conflicts);
        }
    }
    map
}

fn set<D: Ord + Clone>(items: impl IntoIterator<Item = D>) -> BTreeSet<D> {
    items.into_iter().collect()
}

/// Any rewrite of the search must satisfy this. It is the counter-example that refutes the tempting
/// least-fixed-point shortcut, and it is here so the next attempt at that shortcut meets it immediately.
///
/// The reasoning a rewrite reaches for is: the search accepts a key, rejects its conflicts, and repeats
/// while any key is neither rejected nor accepted — so every key not rejected is eventually accepted, and
/// the outcome is `⋃{conflicts(j) : j ∉ rejected}` to a fixed point. **That over-approximates.** A key that
/// has been *rejected* is never accepted afterwards, so chains are not all accepted just because nothing
/// rejected them. Here `2` conflicts with nothing, so the fixed point accepts `0` and `1` together and
/// yields the extra `{0, 1}` — which the search never produces, because accepting `0` rejects `1`.
#[test]
fn rejection_options_are_not_the_closure() {
    let map: BTreeMap<i32, BTreeSet<i32>> =
        BTreeMap::from([(0, set([1])), (1, set([0])), (2, set::<i32>([]))]);
    let options = compute_rejection_options(&map);
    assert_eq!(
        options,
        BTreeSet::from([set([0]), set([1])]),
        "the search reaches {{0}} and {{1}}, and never {{0,1}}"
    );
    assert!(
        !options.contains(&set([0, 1])),
        "the least-fixed-point closure yields {{0,1}} here; it is not the same function"
    );
}

/// **The fix, pinned.** A 20-chain fork — the width the heap profile was taken at — used to take
/// **11.3 s** here and push 19,728,200 queue entries for 2,046 states. It now expands each state once.
/// The bound is loose on purpose: this is a regression guard against re-introducing the per-ordering
/// re-expansion (which is the only thing that puts this in seconds), not a performance gate.
#[test]
fn rejection_options_expand_each_state_once_on_a_fork_shape() {
    let map = fork_shape(2, 10);
    let started = Instant::now();
    let options = compute_rejection_options(&map);
    let elapsed = started.elapsed();
    assert_eq!(
        options.len(),
        2,
        "a fork of two branches has exactly two outcomes: reject the other branch"
    );
    assert!(
        elapsed.as_millis() < 5000,
        "20 conflicting chains in two branches took {elapsed:?}; the same input took 11.3 s while each \
         state was expanded once per ordering that reached it"
    );
}

/// **The falsifier for what the fix did not remove.** Ignored, because it is still red: it is the
/// measurement that says whether the search's *remaining* cost has been bounded, not a gate that should
/// block a build.
///
///     cargo test -p rchain-sdk --release --test merging_scaling -- --ignored --nocapture
///
/// It is red for a different reason than before. The dedup removed the redundancy (each state once), and
/// 40 chains — infeasible to run at all before — now completes in **34.6 s at 351 MiB**: memory is
/// bounded, time is not. 40 chains in two branches is 2^21 reachable states, and that count is the
/// search's true shape, so bounding it is an algorithmic decision (enumerate the terminal states
/// directly, or cap the search) and not a dedup.
#[test]
#[ignore = "deliberately red: the state count is exponential in fork breadth, and bounding it is a decision (#117)"]
fn rejection_options_scales_on_a_fork_shape() {
    for per_branch in [10, 20] {
        let map = fork_shape(2, per_branch);
        let started = Instant::now();
        let options = compute_rejection_options(&map);
        let elapsed = started.elapsed();
        assert_eq!(
            options.len(),
            2,
            "a fork of two branches has exactly two outcomes: reject the other branch"
        );
        println!(
            "{per_branch} chains per branch ({} total): {elapsed:?}",
            per_branch * 2
        );
        assert!(
            elapsed.as_millis() < 1000,
            "{} conflicting chains in two branches took {elapsed:?}; this must be bounded",
            per_branch * 2
        );
    }
}
