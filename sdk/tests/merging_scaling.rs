//! The merge's conflict resolution was the node's memory ceiling (#117), in two steps, and both are measured
//! here.
//!
//! `MergeScope::merge` calls `resolve_conflict_set` once per parent, and a heap profile of a node at its
//! cgroup ceiling attributes **97 %** of the live heap to `BTreeSet` clones inside it, grown 343 -> 1488 MiB
//! within one run.
//!
//! **First the redundancy.** The ported search carried the rejected set in every queue entry beside the
//! accepted set, and cloned both per push. The rejected set is not independent state — it is always
//! `⋃{conflicts(x) : x ∈ accepted}` — so that only made a state reachable by one entry per *ordering* of it:
//! **19,728,200 expansions for 2,046 states** on a 20-chain fork. Carrying the accepted set alone fixed that
//! (11.3 s -> 13.4 ms) and left the enumeration itself untouched.
//!
//! **Then the enumeration.** It expands one state per subset of chains that can be accepted together, which
//! is `2^n` when the chains do not conflict and `2^(n/2 + 1)` on a fork — exponential in the number of chains
//! a merge scope holds, and unrelated to how many rejection options come out. When the conflict relation is
//! symmetric and irreflexive on its keys, the terminal states *are* the maximal independent sets, so
//! `compute_rejection_options` now enumerates those instead (Bron–Kerbosch with pivoting) and never touches
//! the intermediate states. The counts below are that: 40 chains went from an infeasible enumeration to 60
//! recursion nodes, and the growth with width is linear rather than exponential.
//!
//! The dense case was never the problem — accepting any chain rejects all the others — which is why the
//! 1000-node full-graph test passed throughout.

use rchain_sdk::dag::merging::{compute_rejection_options, compute_rejection_options_with_census};
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

/// **The first fix, pinned.** A 20-chain fork — the width the heap profile was taken at — used to take
/// **11.3 s** here and push 19,728,200 queue entries for 2,046 states, and the enumeration still expands
/// those 2,046 (see `the_enumeration_expands_one_state_per_acyclic_subset` in the sdk). This is the
/// per-*ordering* re-expansion guard: the same input that took 11.3 s while a state was visited once per
/// ordering reaching it. The time bound is loose on purpose; the count bound next door is the real one.
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

/// **The gate for the bound, and it is counts rather than seconds.** A fork of two branches has exactly
/// two maximal independent sets — one branch or the other — so the exact rewrite's work is a walk down
/// each branch, not `2^(p+1)`: 40 chains goes from an infeasible enumeration to two options in about a
/// millisecond, and the *count* stays linear out to 2,000 chains.
///
/// The count asserted is `expanded ≤ (keys + 1) × (options + 1)`, which the enumeration misses by an
/// exponential on this shape, so this fails loudly if the fast path is ever lost — on any machine, at
/// any speed. That is why it is asserted at every width, including widths where the *time* is no longer
/// small: the two are different claims, and only the count is a claim about the algorithm.
///
/// **The time, and its honest limit.** The pivot scan costs `O(|p| · log)` per candidate, so the search
/// is output-sensitive but not free per node: measured on this tree, 200 chains takes 22 ms and 2,000
/// takes 21 s. The widths asserted here therefore stop at 200 — a hundred times past anything observed
/// (the widest real merge scope in this tree's fixtures is 2 chains, and the node's true widths are what
/// `search_census` reports on a devnet run) — while 2,000 was run and its count is linear (3,000 nodes),
/// because half a minute in every `cargo test` is not worth the extra column.
#[test]
fn rejection_options_are_bounded_on_a_fork_shape() {
    for per_branch in [10, 20, 100, 200] {
        let map = fork_shape(2, per_branch);
        let started = Instant::now();
        let (options, census) = compute_rejection_options_with_census(&map);
        let elapsed = started.elapsed();
        assert_eq!(
            options.len(),
            2,
            "a fork of two branches has exactly two outcomes: reject the other branch"
        );
        assert!(
            census.expanded <= (census.keys + 1) * (census.options + 1),
            "{} conflicting chains in two branches expanded {} states for {} options",
            per_branch * 2,
            census.expanded,
            census.options
        );
        assert_eq!(
            census.max_frontier, 0,
            "the exact path keeps no frontier at all"
        );
        println!(
            "{} chains per branch ({} total): {} states, {elapsed:?}",
            per_branch,
            per_branch * 2,
            census.expanded
        );
        if per_branch <= 100 {
            assert!(
                elapsed.as_millis() < 1000,
                "{} conflicting chains in two branches took {elapsed:?}; this must be bounded",
                per_branch * 2
            );
        }
    }
}
