//! DAG merge conflict-resolution logic.
//!
//! Faithful port of
//! `sdk/src/main/scala/coop/rchain/sdk/dag/merging/ConflictResolutionLogic.scala`.
//!
//! Law 17: the merge outcome is deterministic — `resolve_conflict_set` chooses the unique
//! optimal rejection (min total cost, then min size, then lexicographically smallest set),
//! independent of arrival order; mergeable (numeric) channels must stay non-negative and must
//! not overflow.
//!
//! Note on determinism: the Scala source uses `Set`/`Map` (unordered) and relies on explicit
//! `.sorted` in the hot paths. This port uses `BTreeSet`/`BTreeMap` throughout, which makes
//! iteration deterministic by construction — matching the *intent* of Law 17 and the callers
//! that already sort (`compute_greedy_non_intersecting_branches`, `add_mergeable_overflow_rejections`).

use std::borrow::Borrow;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// `cats` `|+|` for `Map[D, Set[D]]`: union the value sets under shared keys.
fn union_maps<D: Ord + Clone>(
    mut a: BTreeMap<D, BTreeSet<D>>,
    b: &BTreeMap<D, BTreeSet<D>>,
) -> BTreeMap<D, BTreeSet<D>> {
    for (k, v) in b {
        a.entry(k.clone()).or_default().extend(v.iter().cloned());
    }
    a
}

/// All items in dependency chains reachable from `of` (including `of` itself).
pub fn with_dependencies<D: Ord + Clone>(
    of: &BTreeSet<D>,
    dependency_map: &BTreeMap<D, BTreeSet<D>>,
) -> BTreeSet<D> {
    let mut result = of.clone();
    let mut frontier: BTreeSet<D> = of.clone();
    loop {
        let next: BTreeSet<D> = frontier
            .iter()
            .flat_map(|d| dependency_map.get(d).cloned().unwrap_or_default())
            .collect();
        let new: BTreeSet<D> = next.difference(&result).cloned().collect();
        if new.is_empty() {
            break;
        }
        result.extend(new.iter().cloned());
        frontier = new;
    }
    result
}

/// Items incompatible with the finalized body: conflicts with finally-accepted, or
/// dependents of finally-rejected.
pub fn incompatible_with_final<D: Ord + Clone>(
    accepted_finally: &BTreeSet<D>,
    rejected_finally: &BTreeSet<D>,
    conflicts_map: &BTreeMap<D, BTreeSet<D>>,
    dependency_map: &BTreeMap<D, BTreeSet<D>>,
) -> BTreeSet<D> {
    let mut out = BTreeSet::new();
    for a in accepted_finally {
        if let Some(v) = conflicts_map.get(a) {
            out.extend(v.iter().cloned());
        }
    }
    for r in rejected_finally {
        if let Some(v) = dependency_map.get(r) {
            out.extend(v.iter().cloned());
        }
    }
    out
}

/// Split the scope into non-overlapping partitions, greedily allocating intersecting chunks to
/// the bigger (earlier) view.
pub fn partition_scope<D: Ord + Clone>(views: &[BTreeSet<D>]) -> Vec<BTreeSet<D>> {
    let mut result = Vec::new();
    let mut remaining: Vec<BTreeSet<D>> = views.to_vec();
    while let Some(head) = remaining.first().cloned() {
        result.push(head.clone());
        let tail: Vec<BTreeSet<D>> = remaining[1..]
            .iter()
            .map(|v| v.difference(&head).cloned().collect())
            .collect();
        remaining = tail;
    }
    result
}

/// Build a relation map over `target_set × source_set`. Keys are `source`; values are the
/// related `target`s (plus, when `directed` is false, the symmetric edge).
pub fn compute_relation_map<D, F>(
    directed: bool,
    target_set: &BTreeSet<D>,
    source_set: &BTreeSet<D>,
    relation: F,
) -> BTreeMap<D, BTreeSet<D>>
where
    D: Ord + Clone,
    F: Fn(&D, &D) -> bool,
{
    let mut acc: BTreeMap<D, BTreeSet<D>> = BTreeMap::new();
    for target in target_set {
        for source in source_set {
            if relation(target, source) && target != source {
                acc.entry(source.clone())
                    .or_default()
                    .insert(target.clone());
                if !directed {
                    acc.entry(target.clone())
                        .or_default()
                        .insert(source.clone());
                }
            }
        }
    }
    acc
}

/// Build the (undirected) conflicts map.
pub fn compute_conflicts_map<D, F>(
    target_set: &BTreeSet<D>,
    source_set: &BTreeSet<D>,
    conflicts: F,
) -> BTreeMap<D, BTreeSet<D>>
where
    D: Ord + Clone,
    F: Fn(&D, &D) -> bool,
{
    compute_relation_map(false, target_set, source_set, conflicts)
}

/// Build the (directed) dependency map.
pub fn compute_dependency_map<D, F>(
    target_set: &BTreeSet<D>,
    source_set: &BTreeSet<D>,
    depends: F,
) -> BTreeMap<D, BTreeSet<D>>
where
    D: Ord + Clone,
    F: Fn(&D, &D) -> bool,
{
    compute_relation_map(true, target_set, source_set, depends)
}

/// Compute branches of depending items: each root's dependents are folded into their dependers,
/// so every tip/root becomes concurrent; target items outside any dependency become empty branches.
pub fn compute_branches<D: Ord + Clone>(
    target: &BTreeSet<D>,
    dependency_map: &BTreeMap<D, BTreeSet<D>>,
) -> BTreeMap<D, BTreeSet<D>> {
    let mut acc = dependency_map.clone();
    let entries: Vec<(D, BTreeSet<D>)> = dependency_map
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    for (root, depending) in &entries {
        let root_dependencies: Vec<D> = acc
            .iter()
            .filter(|(_, v)| v.contains(root))
            .map(|(k, _)| k.clone())
            .collect();
        if !root_dependencies.is_empty() {
            acc.remove(root);
            let merged: BTreeSet<D> = depending
                .iter()
                .cloned()
                .chain(std::iter::once(root.clone()))
                .collect();
            for k in &root_dependencies {
                acc.entry(k.clone())
                    .or_default()
                    .extend(merged.iter().cloned());
            }
        }
    }
    // Target items that appear neither as a key nor a value get an empty branch.
    let mut all: BTreeSet<D> = BTreeSet::new();
    for (k, v) in dependency_map {
        all.insert(k.clone());
        all.extend(v.iter().cloned());
    }
    for t in target.difference(&all) {
        acc.entry(t.clone()).or_insert_with(BTreeSet::new);
    }
    acc
}

/// Compute branches of depending items that do not intersect, partitioning greedily.
pub fn compute_greedy_non_intersecting_branches<D: Ord + Clone>(
    target: &BTreeSet<D>,
    dependency_map: &BTreeMap<D, BTreeSet<D>>,
) -> Vec<BTreeSet<D>> {
    let concurrent_roots = compute_branches(target, dependency_map);
    let mut sorted: Vec<(usize, D, BTreeSet<D>)> = concurrent_roots
        .iter()
        .map(|(k, v)| (v.len(), k.clone(), v.clone()))
        .collect();
    // sort by (descending size, ascending key)
    sorted.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let views: Vec<BTreeSet<D>> = sorted
        .into_iter()
        .map(|(_, k, mut v)| {
            v.insert(k);
            v
        })
        .collect();
    partition_scope(&views)
}

/// Relation map sufficient for a merge set: conflicts/dependencies inside the conflict set and
/// between the conflict set and the final set.
pub fn compute_relation_map_for_merge_set<D, F1, F2>(
    conflict_set: &BTreeSet<D>,
    final_set: &BTreeSet<D>,
    conflicts: F1,
    depends: F2,
) -> (BTreeMap<D, BTreeSet<D>>, BTreeMap<D, BTreeSet<D>>)
where
    D: Ord + Clone,
    F1: Fn(&D, &D) -> bool,
    F2: Fn(&D, &D) -> bool,
{
    let conflicts_map = union_maps(
        compute_conflicts_map(conflict_set, final_set, &conflicts),
        &compute_conflicts_map(conflict_set, conflict_set, &conflicts),
    );
    let dependency_map = union_maps(
        compute_dependency_map(conflict_set, final_set, &depends),
        &compute_dependency_map(conflict_set, conflict_set, &depends),
    );
    (conflicts_map, dependency_map)
}

/// What one run of the rejection search cost, in **counts**. The search's size is a deterministic
/// function of the conflict map, where a stopwatch and an RSS reading are functions of the machine as
/// well — so this is what a test pins, and it is the quantity memory is made of: `expanded` states, each
/// holding one owned `BTreeSet` of up to `keys` elements, plus the frontier.
/// **How much work a merge search may do before it refuses.**
///
/// This is a **node-local policy, not a protocol constant**, and that distinction is the whole reason it
/// can ship without a fork: a search that hits its budget returns **no answer at all**, and the caller
/// drops the block locally (`ValidateError::Internal`) instead of recording it failed. Every node that
/// *does* run the search gets the identical result, because the bound never truncates one — a partial
/// enumeration is discarded, never reported. Truncating and answering would be a consensus change: the
/// option set is what `compute_optimal_rejection` minimises over, so a short set can pick a different
/// rejection (law 17a).
///
/// **Why a budget and not a predictor.** There is no cheap scalar that predicts the cost: at 30–37 chains
/// the same width produced 389,977 / 442,202 / 985,391 / 1,726,295 / 2,026,511 states across nine node
/// runs, so both a width threshold and a conflict-count threshold would be keyed on the wrong quantity.
/// The cost is counted where it is spent — `SearchCensus::expanded` — and bounded there, which is what
/// `candidate:bounded-work-per-step` asks for: *where a bound exists it is applied before the work, not
/// after it*.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchBudget {
    /// Search steps: queue pops on the enumeration path, recursion nodes on the fast path. This is
    /// `SearchCensus::expanded`, the quantity #117's memory is a function of.
    pub max_steps: usize,
    /// Distinct rejection options. This is what bounds **memory** on the fast path, which has no frontier
    /// at all — its residency is the option set, and a perfect matching of `m` pairs has `2^m` of them.
    pub max_options: usize,
}

impl SearchBudget {
    /// **No bound — for the differential oracle and for tests, never for a node.** An unbounded search
    /// over a conflict set whose size the DAG decides is the defect this type exists to remove.
    pub const UNBOUNDED: SearchBudget = SearchBudget {
        max_steps: usize::MAX,
        max_options: usize::MAX,
    };

    /// The node's own budget. Provisional, and stated as such: the honest envelope measured so far is a
    /// largest single merge of **2,026,511 steps** (nine node-runs, 2026-09-30), so this is ~5× the worst
    /// honest case observed and it must be re-read from a campaign on C171's own arm before it is relied
    /// on. It caps an attack at a bounded multiple of the honest cost, where the enumerated case is
    /// `2^43 = 8.8e12`.
    pub const NODE: SearchBudget = SearchBudget {
        max_steps: 10_000_000,
        max_options: 1_000_000,
    };
}

/// A search that hit its [`SearchBudget`]. **It carries no answer**, deliberately: see the type above for
/// why a truncated option set must be discarded rather than returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchBudgetExceeded {
    pub steps: usize,
    pub options: usize,
    pub budget: SearchBudget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SearchCensus {
    /// Keys in the conflict map.
    pub keys: usize,
    /// `(key, conflict)` pairs — the conflict map's density.
    pub conflicts: usize,
    /// Ordered pairs `(x, y)` **among the keys** with `y ∈ conflicts(x)` but `x ∉ conflicts(y)`.
    /// **Zero means the relation restricted to the keys is symmetric**, which is what decides the
    /// algorithm: it makes a state's reachability exactly "independent set" and a terminal state exactly
    /// a **maximal independent set**, so `maximal_independent_sets` applies and the 2^n intermediate
    /// states are never touched. Non-zero falls back to the enumeration, which is correct for any map.
    /// (Pairs where `y` is not a key are excluded deliberately: `y` can never be accepted, so it
    /// changes only what a rejection option *contains*, not which sets are reachable.)
    pub asymmetric: usize,
    /// Keys that conflict with themselves. The fast path declines these: a self-conflict is a one-cycle,
    /// so the key is never acceptable, which the maximal-independent-set form cannot express.
    pub self_conflicts: usize,
    /// Search steps taken. **This is the search's cost**, and the only thing #117's memory is a function
    /// of: queue pops on the enumeration path, recursion nodes on the maximal-independent-set path.
    pub expanded: usize,
    /// The largest the queue ever grew. Always 0 on the maximal-independent-set path, which has no
    /// frontier — memory there is the recursion depth and the option set.
    pub max_frontier: usize,
    /// Distinct rejection options found.
    pub options: usize,
}

impl SearchCensus {
    /// Whether the relation restricted to the keys is symmetric and irreflexive — the precondition of
    /// [`maximal_independent_sets`]. One place, so the condition the algorithm branches on and the
    /// condition the census reports cannot disagree.
    fn keys_are_a_symmetric_irreflexive_relation(&self) -> bool {
        self.asymmetric == 0 && self.self_conflicts == 0
    }
}

/// All rejection combinations (sets of rejected items) that resolve the conflict map.
///
/// This is the Scala `computeRejectionOptions` search: a breadth-first enumeration over acceptance
/// states, kept *result-identical* to the Scala while visiting each state once.
///
/// **A state is the accepted set alone.** The rejected set is not independent information — the only
/// way a key enters it is as the conflict of an accepted one, so `rejected = ⋃{conflicts(x) : x ∈
/// accepted}` after every step, and `rejected` is a pure function of `accepted`. The ported search
/// carried both sets in each queue entry anyway, which made it re-visit a state once per **ordering**
/// that reaches it, and clone two sets per visit: `20 chains in two branches: 19,728,200 expansions for
/// 2,046 states` (measured; `sdk/tests/merging_scaling.rs`). Those clones are **97 % of the live heap**
/// of a node at its cgroup ceiling, grown 343 -> 1488 MiB within one run (#117).
///
/// Carrying `accepted` alone collapses that to one visit per state, which is what the search's own
/// state graph always was. It is exact rather than a heuristic: the successor set and the answer both
/// depend on the state only through `accepted`, so skipping a state already expanded cannot remove a
/// reachable answer.
///
/// **The residual, stated exactly — because "fork" was too narrow a word for it.** A set `S` of keys is
/// reachable iff its keys can be added one at a time with each new key absent from the conflicts of
/// those already accepted, i.e. iff the conflicts *within* `S` have no directed cycle. So the search
/// expands **one state per nonempty subset that induces an acyclic subgraph** (the empty set is never a
/// state — the seeds are singletons), and:
///
/// | conflict map | states expanded | rejection options |
/// |---|---|---|
/// | complete (`K \ {k}` for each `k`) | `n` | `n` |
/// | **no conflicts at all** | **`2^n - 1`** | **1** |
/// | two branches, complete across (`fork`) | `2^(p+1) - 2` | `2` |
/// | a perfect matching of `m` pairs | `3^m - 1` | `2^m` |
///
/// The first row is why the 1000-node full-graph oracle passes while the node grows to gigabytes; the
/// **second is the node's normal case** — chains that do not conflict cost `2^n` states to report a
/// single option — and it is worse than the fork shape this defect was first described with. `2^n` states
/// each holding a cloned set is the astonishing bloat, and it is not a leak, a cache, or an allocator
/// policy: it is this enumeration, deterministically. Pinned by
/// `the_search_expands_one_state_per_acyclic_subset` and its siblings, which assert these four counts.
///
/// **A least-fixed-point rewrite was attempted and is wrong.** It is tempting to compute
/// `rejected ⊇ ⋃{conflicts(j) : j ∉ rejected}` directly, on the reasoning that every key not rejected is
/// eventually accepted. That over-approximates, because a key that has been *rejected* can never
/// subsequently be accepted: for `{0: {1}, 1: {0}, 2: {}}` the search yields `{{0}, {1}}`, while the
/// closure also yields `{0, 1}`. The counter-example is pinned by
/// `rejection_options_are_not_the_closure` so that the next attempt at this meets it. Any rewrite must
/// preserve the outcome exactly — the merge result is consensus-visible (law 17) — which means
/// differential testing against a literal enumeration of this search, not reasoning about it. That
/// differential is `rejection_options_match_a_literal_enumeration` in `sdk/src/property_tests.rs`.
pub fn compute_rejection_options<D: Ord + Clone>(
    conflicts_map: &BTreeMap<D, BTreeSet<D>>,
) -> BTreeSet<BTreeSet<D>> {
    // **Unbounded, and that is the point of this entry point**: the differential tests must see the exact
    // search, so a defect in the budget cannot hide behind a truncated answer. Every production caller
    // goes through a budgeted path.
    search(conflicts_map, SearchBudget::UNBOUNDED)
        .expect("an unbounded search cannot exceed its budget")
        .0
}

/// [`compute_rejection_options`], plus the [`SearchCensus`] of what it cost. The same function — one
/// implementation, not a copy that can drift — with the counters kept, because the search's size is a
/// deterministic function of its input and the way to say so is a number a test can assert rather than a
/// stopwatch reading. This is the variable #117 was argued about without ever pinning.
pub fn compute_rejection_options_with_census<D: Ord + Clone>(
    conflicts_map: &BTreeMap<D, BTreeSet<D>>,
) -> (BTreeSet<BTreeSet<D>>, SearchCensus) {
    search(conflicts_map, SearchBudget::UNBOUNDED)
        .expect("an unbounded search cannot exceed its budget")
}

/// [`compute_rejection_options`] **under a budget** — the node's path.
///
/// `Err` means the search was **abandoned**, not that it found nothing: the caller must drop the work,
/// never act on a partial answer. See [`SearchBudget`] for why that is what makes this a node-local policy
/// rather than a fork.
pub fn compute_rejection_options_with_budget<D: Ord + Clone>(
    conflicts_map: &BTreeMap<D, BTreeSet<D>>,
    budget: SearchBudget,
) -> Result<BTreeSet<BTreeSet<D>>, SearchBudgetExceeded> {
    Ok(search(conflicts_map, budget)?.0)
}

fn search<D: Ord + Clone>(
    conflicts_map: &BTreeMap<D, BTreeSet<D>>,
    budget: SearchBudget,
) -> Result<(BTreeSet<BTreeSet<D>>, SearchCensus), SearchBudgetExceeded> {
    let all_keys: Vec<D> = conflicts_map.keys().cloned().collect();
    let mut census = SearchCensus {
        keys: all_keys.len(),
        conflicts: conflicts_map.values().map(BTreeSet::len).sum(),
        asymmetric: {
            let mut count = 0;
            for (x, ys) in conflicts_map {
                for y in ys {
                    // `y` must be a key to matter: a non-key can never be accepted, so whether `x` is in
                    // *its* conflict set changes nothing about which sets are reachable.
                    if let Some(back) = conflicts_map.get(y) {
                        if !back.contains(x) {
                            count += 1;
                        }
                    }
                }
            }
            count
        },
        self_conflicts: all_keys
            .iter()
            .filter(|k| conflicts_map.get(*k).is_some_and(|c| c.contains(*k)))
            .count(),
        ..SearchCensus::default()
    };

    // The ported search starts from the map's keys, so with none it never runs and there are no options
    // at all — not one empty option. Preserved deliberately: `an_empty_conflict_map_yields_no_options`.
    if all_keys.is_empty() {
        return Ok((BTreeSet::new(), census));
    }

    // **The fix.** When the relation on the keys is symmetric and irreflexive, a reachable state is an
    // independent set and a terminal state is exactly a *maximal* one, so the answer can be read off
    // maximal independent sets directly — never touching the 2^n intermediate states, which is where
    // #117's memory went. Otherwise the enumeration below, which is correct for any relation.
    if census.keys_are_a_symmetric_irreflexive_relation() {
        let (result, nodes) = maximal_independent_sets(conflicts_map, &all_keys, budget)?;
        census.expanded = nodes;
        census.options = result.len();
        return Ok((result, census));
    }
    let (result, expanded, max_frontier) = enumerate_states(conflicts_map, &all_keys, budget)?;
    census.expanded = expanded;
    census.max_frontier = max_frontier;
    census.options = result.len();
    Ok((result, census))
}

/// The ported search, over every reachable state. Correct for **any** relation — this is the fallback
/// [`search`] takes when the fast path's precondition does not hold — and it is also the oracle the
/// maximal-independent-set path is differed against, via the literal transcription in
/// `sdk/src/property_tests.rs`.
fn enumerate_states<D: Ord + Clone>(
    conflicts_map: &BTreeMap<D, BTreeSet<D>>,
    all_keys: &[D],
    budget: SearchBudget,
) -> Result<(BTreeSet<BTreeSet<D>>, usize, usize), SearchBudgetExceeded> {
    let mut queue: VecDeque<BTreeSet<D>> = VecDeque::new();
    let mut visited: BTreeSet<BTreeSet<D>> = BTreeSet::new();
    for k in all_keys {
        let seed: BTreeSet<D> = std::iter::once(k.clone()).collect();
        if visited.insert(seed.clone()) {
            queue.push_back(seed);
        }
    }
    let mut expanded = 0usize;
    let mut max_frontier = queue.len();
    let mut result: BTreeSet<BTreeSet<D>> = BTreeSet::new();

    while let Some(accepted) = queue.pop_front() {
        expanded += 1;
        // **The bound, at the step that costs.** Checked before the state is expanded rather than after,
        // so the budget counts work that was about to be done and the refusal happens *before* it — which
        // is the whole of `candidate:bounded-work-per-step`.
        if expanded > budget.max_steps {
            return Err(SearchBudgetExceeded {
                steps: expanded,
                options: result.len(),
                budget,
            });
        }

        // Derived, not carried: see the note above. `flat_map` over the missing-key case is the
        // empty set, which is what the ported `if let Some(c) = conflicts_map.get(&a)` did.
        let rejected: BTreeSet<D> = accepted
            .iter()
            .filter_map(|k| conflicts_map.get(k))
            .flat_map(|c| c.iter().cloned())
            .collect();

        let next: Vec<D> = all_keys
            .iter()
            .filter(|k| !rejected.contains(k) && !accepted.contains(k))
            .cloned()
            .collect();
        if next.is_empty() {
            result.insert(rejected);
            if result.len() > budget.max_options {
                return Err(SearchBudgetExceeded {
                    steps: expanded,
                    options: result.len(),
                    budget,
                });
            }
        } else {
            for n in next {
                let mut grown = accepted.clone();
                grown.insert(n);
                if visited.insert(grown.clone()) {
                    queue.push_back(grown);
                }
            }
            max_frontier = max_frontier.max(queue.len());
        }
    }
    Ok((result, expanded, max_frontier))
}

/// The maximal independent sets of the conflict relation restricted to `keys`, as rejection options —
/// one option per set, `option = ⋃{conflicts(x) : x ∈ S}`.
///
/// **Why this is the same function, exactly.** Caller-checked precondition: the relation on `keys` is
/// symmetric and irreflexive (see `SearchCensus::keys_are_a_symmetric_irreflexive_relation`). Then:
///
/// - a set `S ⊆ keys` is *reachable* iff its keys can be added one at a time with each new one absent
///   from the conflicts of those before it, i.e. iff the conflicts within `S` contain no directed cycle,
///   which for a symmetric irreflexive relation is exactly **`S` is independent**;
/// - it is *terminal* iff additionally every key outside it is rejected, i.e. every `x ∉ S` conflicts
///   with some element of `S` — in the symmetric case, exactly **`S` is dominating**.
///
/// Independent and dominating is the definition of a maximal independent set, so the terminal states
/// are precisely the maximal independent sets and the option set is their image under `⋃ conflicts`.
/// Nothing else about the enumeration is used — in particular the 2^n *intermediate* states are not,
/// which is the whole point: they are what #117's memory was made of.
///
/// Enumerated by Bron–Kerbosch with pivoting (a maximal independent set of a relation is a maximal
/// clique of its complement), the standard way to visit only the maximal ones. Returns the options and
/// the number of recursion nodes, so one census field means "search steps" on both paths.
fn maximal_independent_sets<D: Ord + Clone>(
    conflicts_map: &BTreeMap<D, BTreeSet<D>>,
    all_keys: &[D],
    budget: SearchBudget,
) -> Result<(BTreeSet<BTreeSet<D>>, usize), SearchBudgetExceeded> {
    // `conf(x) ∪ {x}`: everything a set may not contain alongside `x`. Working on *this* rather than on
    // the complement's neighbour sets is what keeps the cost tied to the conflict map rather than to the
    // key count — a complement is dense exactly when the conflicts are sparse, which is the case that
    // reaches this function in the first place. (Measured: a 2,000-chain fork is 3,000 recursion nodes,
    // and rebuilding the complement per call cost 107 s where this costs 1.6 ms at 40 chains.)
    let blocked_by = |x: &D| -> BTreeSet<D> {
        let mut blocked = conflicts_map.get(x).cloned().unwrap_or_default();
        blocked.insert(x.clone());
        blocked
    };

    let mut sets: BTreeSet<BTreeSet<D>> = BTreeSet::new();
    let mut nodes = 0usize;
    let all: BTreeSet<D> = all_keys.iter().cloned().collect();
    let empty: BTreeSet<D> = BTreeSet::new();
    bron_kerbosch(
        &blocked_by,
        &empty,
        &all,
        &empty,
        &mut sets,
        &mut nodes,
        budget,
    )?;

    let options = sets
        .into_iter()
        .map(|set| {
            set.iter()
                .filter_map(|k| conflicts_map.get(k))
                .flat_map(|c| c.iter().cloned())
                .collect()
        })
        .collect();
    Ok((options, nodes))
}

/// Bron–Kerbosch with pivoting, written on the conflict relation: `r` the clique so far (an independent
/// set of conflicts), `p` its candidates, `x` the candidates already explored at this level. Pivoting is
/// what keeps this from being a walk over all independent sets — the branch set is the candidates that
/// *conflict* with the pivot, so a vertex that blocks every candidate collapses the branching instead of
/// being enumerated through.
///
/// In complement terms this is the textbook algorithm; the identities used are `p ∩ N̅(u)` complementing
/// into `p ∩ (conf(u) ∪ {u})`, and `p \ N̅(v)` into `p \ (conf(v) ∪ {v})`.
fn bron_kerbosch<D: Ord + Clone>(
    blocked_by: &dyn Fn(&D) -> BTreeSet<D>,
    r: &BTreeSet<D>,
    p: &BTreeSet<D>,
    x: &BTreeSet<D>,
    out: &mut BTreeSet<BTreeSet<D>>,
    nodes: &mut usize,
    budget: SearchBudget,
) -> Result<(), SearchBudgetExceeded> {
    *nodes += 1;
    // The same bound, on the recursion node that costs: a perfect matching of `m` pairs has `2^m` maximal
    // independent sets, so this path needs the *option* bound as much as the step one — it has no
    // frontier, and its residency is `out`.
    if *nodes > budget.max_steps || out.len() > budget.max_options {
        return Err(SearchBudgetExceeded {
            steps: *nodes,
            options: out.len(),
            budget,
        });
    }
    if p.is_empty() && x.is_empty() {
        out.insert(r.clone());
        return Ok(());
    }

    // The pivot with the smallest blocked neighbourhood *within* `p` — the branch set is exactly that
    // intersection, so this is the fewest branches. The textbook rule, and it must be this one: a
    // cheaper proxy (the conflict degree, one lookup per candidate) was tried and *raised* the node
    // count past the bound this function's tests assert, which is the wrong trade — a worse search that
    // scans faster is still worse. The cost is `O(|p| · log)` per candidate, which is microseconds at
    // the widths the node produces and seconds only past a thousand chains; `degree_by` is kept as the
    // tie-break so equal-width pivots are chosen deterministically rather than by key order.
    let pivot = p
        .iter()
        .chain(x.iter())
        .min_by_key(|u| p.intersection(&blocked_by(u)).count())
        .cloned();
    let candidates: Vec<D> = match &pivot {
        Some(u) => p.intersection(&blocked_by(u)).cloned().collect(),
        None => p.iter().cloned().collect(),
    };

    let mut p = p.clone();
    let mut x = x.clone();
    for v in candidates {
        let blocked = blocked_by(&v);
        let mut grown = r.clone();
        grown.insert(v.clone());
        bron_kerbosch(
            blocked_by,
            &grown,
            &p.difference(&blocked).cloned().collect(),
            &x.difference(&blocked).cloned().collect(),
            out,
            nodes,
            budget,
        )?;
        p.remove(&v);
        x.insert(v);
    }
    Ok(())
}

/// Pick the rejection option minimizing (total cost, size, sorted set) lexicographically.
pub fn compute_optimal_rejection<D, F>(options: &BTreeSet<BTreeSet<D>>, target_f: F) -> BTreeSet<D>
where
    D: Ord + Clone,
    F: Fn(&D) -> i64,
{
    options
        .iter()
        .min_by(|a, b| {
            let ca: i64 = a.iter().map(&target_f).sum();
            let cb: i64 = b.iter().map(&target_f).sum();
            let sa: Vec<&D> = a.iter().collect();
            let sb: Vec<&D> = b.iter().collect();
            (ca, a.len(), sa).cmp(&(cb, b.len(), sb))
        })
        .cloned()
        .unwrap_or_default()
}

fn calc_merged_result<D: Ord + Clone, CH: Ord + Clone, V: Borrow<BTreeMap<CH, i64>>>(
    deploy: &D,
    balances: &BTreeMap<CH, i64>,
    mergeable_diffs: &BTreeMap<D, V>,
) -> Option<BTreeMap<CH, i64>> {
    // Borrowed, not copied: the diff is only read, and the caller may hold it behind a reference (a
    // devnet fork storm's merge path did this per chain per merge — #117).
    let no_diff = BTreeMap::new();
    let diff: &BTreeMap<CH, i64> = mergeable_diffs
        .get(deploy)
        .map(Borrow::borrow)
        .unwrap_or(&no_diff);
    let mut acc = balances.clone();
    for (channel, change) in diff {
        let current = acc.get(channel).copied().unwrap_or(0);
        let result = current.checked_add(*change)?; // None on overflow
        if result < 0 {
            return None;
        }
        acc.insert(channel.clone(), result);
    }
    Some(acc)
}

fn traverse_tree<D: Ord + Clone, F: Fn(&D) -> BTreeSet<D>>(root: &D, next: &F) -> Vec<D> {
    let mut result = Vec::new();
    let mut frontier: Vec<D> = vec![root.clone()];
    while !frontier.is_empty() {
        frontier.sort();
        result.extend(frontier.iter().cloned());
        let next_frontier: BTreeSet<D> = frontier.iter().flat_map(|d| next(d)).collect();
        frontier = next_frontier.into_iter().collect();
    }
    result
}

fn fold_rejection<D: Ord + Clone, CH: Ord + Clone, V: Borrow<BTreeMap<CH, i64>>>(
    base_balance: &BTreeMap<CH, i64>,
    to_merge: &BTreeSet<D>,
    dependency_map: &BTreeMap<D, BTreeSet<D>>,
    mergeable_diffs: &BTreeMap<D, V>,
) -> BTreeSet<D> {
    let branches = compute_branches(to_merge, dependency_map);
    let mut concurrent_roots: Vec<D> = branches.keys().cloned().collect();
    concurrent_roots.sort();

    let mut balances = base_balance.clone();
    let mut rejected: BTreeSet<D> = BTreeSet::new();
    for root in &concurrent_roots {
        let deps_of = |d: &D| dependency_map.get(d).cloned().unwrap_or_default();
        let branch = traverse_tree(root, &deps_of);
        for deploy in &branch {
            if rejected.contains(deploy) {
                continue;
            }
            match calc_merged_result(deploy, &balances, mergeable_diffs) {
                Some(new_balances) => balances = new_balances,
                None => {
                    let singleton: BTreeSet<D> = std::iter::once(deploy.clone()).collect();
                    let deps = with_dependencies(&singleton, dependency_map);
                    rejected.insert(deploy.clone());
                    rejected.extend(deps);
                }
            }
        }
    }
    rejected
}

/// Extend the rejection options with rejections forced by mergeable-value overflow.
pub fn add_mergeable_overflow_rejections<
    D: Ord + Clone,
    CH: Ord + Clone,
    V: Borrow<BTreeMap<CH, i64>>,
>(
    conflict_set: &BTreeSet<D>,
    dependency_map: &BTreeMap<D, BTreeSet<D>>,
    reject_options: &BTreeSet<BTreeSet<D>>,
    init_mergeable_values: &BTreeMap<CH, i64>,
    mergeable_diffs: &BTreeMap<D, V>,
) -> BTreeSet<BTreeSet<D>> {
    if reject_options.is_empty() {
        let r = fold_rejection(
            init_mergeable_values,
            conflict_set,
            dependency_map,
            mergeable_diffs,
        );
        std::iter::once(r).collect()
    } else {
        reject_options
            .iter()
            .map(|rj| {
                let diff: BTreeSet<D> = conflict_set.difference(rj).cloned().collect();
                let fr = fold_rejection(
                    init_mergeable_values,
                    &diff,
                    dependency_map,
                    mergeable_diffs,
                );
                rj.union(&fr).cloned().collect()
            })
            .collect()
    }
}

/// Compute the resolution for a conflict set: `(accepted, rejected)`.
#[allow(clippy::too_many_arguments)]
pub fn resolve_conflict_set<D, CH, F, V>(
    conflict_set: &BTreeSet<D>,
    accepted_finally: &BTreeSet<D>,
    rejected_finally: &BTreeSet<D>,
    cost: F,
    conflicts_map: &BTreeMap<D, BTreeSet<D>>,
    dependency_map: &BTreeMap<D, BTreeSet<D>>,
    mergeable_diffs: &BTreeMap<D, V>,
    init_mergeable_values: &BTreeMap<CH, i64>,
    budget: SearchBudget,
) -> Result<(BTreeSet<D>, BTreeSet<D>), SearchBudgetExceeded>
where
    D: Ord + Clone,
    CH: Ord + Clone,
    V: Borrow<BTreeMap<CH, i64>>,
    F: Fn(&D) -> i64,
{
    Ok(resolve_conflict_set_with_census(
        conflict_set,
        accepted_finally,
        rejected_finally,
        cost,
        conflicts_map,
        dependency_map,
        mergeable_diffs,
        init_mergeable_values,
        budget,
    )?
    .0)
}

/// [`resolve_conflict_set`] plus the [`SearchCensus`] of the conflict search it ran. The same function —
/// the plain one delegates here — so a caller that wants to know what the search cost cannot be reading
/// a copy that has drifted from the one that decided the merge.
#[allow(clippy::too_many_arguments)]
pub fn resolve_conflict_set_with_census<D, CH, F, V>(
    conflict_set: &BTreeSet<D>,
    accepted_finally: &BTreeSet<D>,
    rejected_finally: &BTreeSet<D>,
    cost: F,
    conflicts_map: &BTreeMap<D, BTreeSet<D>>,
    dependency_map: &BTreeMap<D, BTreeSet<D>>,
    mergeable_diffs: &BTreeMap<D, V>,
    init_mergeable_values: &BTreeMap<CH, i64>,
    budget: SearchBudget,
) -> Result<((BTreeSet<D>, BTreeSet<D>), SearchCensus), SearchBudgetExceeded>
where
    D: Ord + Clone,
    CH: Ord + Clone,
    V: Borrow<BTreeMap<CH, i64>>,
    F: Fn(&D) -> i64,
{
    let enforce_rejected = with_dependencies(
        &incompatible_with_final(
            accepted_finally,
            rejected_finally,
            conflicts_map,
            dependency_map,
        ),
        dependency_map,
    );
    let conflict_set_compatible: BTreeSet<D> = conflict_set
        .difference(&enforce_rejected)
        .cloned()
        .collect();

    let full_conflicts_map: BTreeMap<D, BTreeSet<D>> = conflicts_map
        .iter()
        .map(|(k, vs)| {
            let deps = with_dependencies(vs, dependency_map);
            (k.clone(), vs.union(&deps).cloned().collect())
        })
        .collect();

    let (rejection_options, census) = search(&full_conflicts_map, budget)?;
    let mergeable_overflow_rejection_options = add_mergeable_overflow_rejections(
        conflict_set,
        dependency_map,
        &rejection_options,
        init_mergeable_values,
        mergeable_diffs,
    );
    let resolved = compute_optimal_rejection(&mergeable_overflow_rejection_options, &cost);
    Ok((
        (
            conflict_set_compatible
                .difference(&resolved)
                .cloned()
                .collect(),
            resolved.union(&enforce_rejected).cloned().collect(),
        ),
        census,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    fn set<D: Ord + Clone>(items: impl IntoIterator<Item = D>) -> BTreeSet<D> {
        items.into_iter().collect()
    }

    fn map<D: Ord + Clone>(
        items: impl IntoIterator<Item = (D, BTreeSet<D>)>,
    ) -> BTreeMap<D, BTreeSet<D>> {
        items.into_iter().collect()
    }

    #[test]
    fn with_dependencies_collects_transitive_closure() {
        let dependents_map = map([
            (1, set([3, 9])),
            (3, set([5])),
            (5, set([6])),
            (4, set([6])),
        ]);
        let rejects = with_dependencies(&set([1]), &dependents_map);
        assert_eq!(rejects, set([1, 3, 9, 5, 6]));
    }

    #[test]
    fn incompatible_with_final_combines_conflicts_and_dependents() {
        let accepted_finally = set([1, 2]);
        let rejected_finally = set([5, 6]);
        let conflicts_map = map([(1, set([11, 12])), (2, set([21, 22])), (3, set([31, 32]))]);
        let dependency_map = map([(5, set([51, 52])), (6, set([61, 62])), (7, set([71, 72]))]);
        let r = incompatible_with_final(
            &accepted_finally,
            &rejected_finally,
            &conflicts_map,
            &dependency_map,
        );
        assert_eq!(r, set([11, 12, 21, 22, 51, 52, 61, 62]));
    }

    #[test]
    fn partition_scope_yields_non_intersecting_partitions() {
        let views = vec![
            set([1, 2, 3, 4]),
            set([4, 5, 6, 7]),
            set([7, 8, 9]),
            set([9, 10]),
        ];
        let r = partition_scope(&views);
        assert_eq!(
            r,
            vec![set([1, 2, 3, 4]), set([5, 6, 7]), set([8, 9]), set([10])]
        );
    }

    #[test]
    fn compute_conflicts_map_is_bidirectional_without_self() {
        let set_all = set([1, 2, 3, 4, 5, 6]);
        let conflicts_map = map([(1, set([2, 3])), (4, set([5])), (6, set([6]))]);
        // mirror: v -> {k} for each (k, v)
        let mut reference: BTreeMap<i32, BTreeSet<i32>> = BTreeMap::new();
        for (k, vs) in &conflicts_map {
            for v in vs {
                reference.entry(*v).or_default().insert(*k);
            }
        }
        for (k, vs) in &conflicts_map {
            reference.entry(*k).or_default().extend(vs.iter().cloned());
        }
        reference.remove(&6); // self-conflict excluded

        let conflicts = |a: &i32, b: &i32| reference.get(a).map(|s| s.contains(b)).unwrap_or(false);
        assert_eq!(
            compute_conflicts_map(&set_all, &set_all, conflicts),
            reference
        );
    }

    #[test]
    fn compute_dependency_map_keys_are_dependencies() {
        let set_all = set([1, 2, 3, 4, 5, 6]);
        let dependency_map = map([
            (1, set([2, 3])),
            (4, set([5])),
            (3, set([1])),
            (6, set([6])),
        ]);
        let depends = |target: &i32, maybe_dependency: &i32| {
            dependency_map
                .get(maybe_dependency)
                .map(|s| s.contains(target))
                .unwrap_or(false)
        };
        let mut expected = dependency_map.clone();
        expected.remove(&6);
        assert_eq!(
            compute_dependency_map(&set_all, &set_all, depends),
            expected
        );
    }

    #[test]
    fn compute_branches_covers_target_with_concurrent_roots() {
        let set_all = set([1, 2, 3, 4, 5, 6, 7, 100, 101]);
        let dependency_map = map([
            (1, set([4, 5])),
            (4, set([5, 6])),
            (2, set([4, 5, 6])),
            (3, set([6, 7])),
        ]);
        let expected = map([
            (1, set([4, 5, 6])),
            (2, set([4, 5, 6])),
            (3, set([6, 7])),
            (100, set([])),
            (101, set([])),
        ]);
        assert_eq!(compute_branches(&set_all, &dependency_map), expected);
    }

    #[test]
    fn compute_relation_map_for_merge_set_combines_internal_and_final() {
        let conflict_set = set([1, 2]);
        let final_set = set([3, 4]);
        let conflicts_map = map([(1, set([2])), (2, set([1]))]);
        let depends_map = map([(2, set([1])), (3, set([2]))]);
        let conflicts =
            |a: &i32, b: &i32| conflicts_map.get(a).map(|s| s.contains(b)).unwrap_or(false);
        let depends = |t: &i32, m: &i32| depends_map.get(m).map(|s| s.contains(t)).unwrap_or(false);
        let r = compute_relation_map_for_merge_set(&conflict_set, &final_set, conflicts, depends);
        assert_eq!(r, (conflicts_map, depends_map));
    }

    #[test]
    fn compute_rejection_options_matches_oracle() {
        assert_eq!(
            compute_rejection_options(&map([
                (1, set([2, 3, 4])),
                (2, set([1])),
                (3, set([1, 2])),
                (4, set([1])),
            ])),
            set([set([1, 2]), set([2, 3, 4])])
        );

        assert_eq!(
            compute_rejection_options(&map([
                (1, set([2, 3, 4])),
                (2, set([1, 3, 4])),
                (3, set([1, 2, 4])),
                (4, set([1, 2, 3])),
            ])),
            set([
                set([2, 3, 4]),
                set([1, 3, 4]),
                set([1, 2, 4]),
                set([1, 2, 3])
            ])
        );

        assert_eq!(
            compute_rejection_options(&map([
                (1, set([2, 3, 4])),
                (2, set([1])),
                (3, set([1, 4])),
                (4, set([1, 3])),
            ])),
            set([set([2, 3, 4]), set([1, 3]), set([1, 4])])
        );

        assert_eq!(
            compute_rejection_options(&map([
                (1, set::<i32>([])),
                (2, set([3])),
                (3, set([2, 4])),
                (4, set([3])),
            ])),
            set([set([3]), set([2, 4])])
        );

        // Full graph on 1000 nodes.
        let all: BTreeSet<i32> = (1..=1000).collect();
        let conflicts_map: BTreeMap<i32, BTreeSet<i32>> = (1..=1000)
            .map(|i| {
                let mut v = all.clone();
                v.remove(&i);
                (i, v)
            })
            .collect();
        let expected: BTreeSet<BTreeSet<i32>> = (1..=1000)
            .map(|i| {
                let mut v = all.clone();
                v.remove(&i);
                v
            })
            .collect();
        assert_eq!(compute_rejection_options(&conflicts_map), expected);
    }

    /// **#117's root cause, pinned as a closed form.** The *enumeration* expands one state per subset of
    /// keys that induces an acyclic subgraph of the conflict relation, and reports far fewer options than
    /// that — so almost all of its work and all of its memory goes on states it will never report. These
    /// are counts, not seconds: a stopwatch measures the machine, and a counter measures the search.
    ///
    /// The four rows are the whole RCA. `dense` is why the 1000-key oracle passes while the node grows
    /// to gigabytes. **`free` is the node's normal case** — a merge scope whose chains do not conflict
    /// costs `2^n` states to report the single option "reject nothing" — and it is worse than the fork
    /// shape this defect was first written up with, which is `2^(p+1) - 2` for two branches of `p`.
    ///
    /// It calls `enumerate_states` directly, deliberately: the shipped `compute_rejection_options` now
    /// reaches for the maximal-independent-set path on every one of these shapes, and this test is the
    /// record of what the *enumeration* does, which is what the fix is measured against.
    /// **The bound, and the two ways it must not be one.**
    ///
    /// `candidate:bounded-work-per-step` asks for the work to be bounded **where it is spent** and for
    /// the refusal to land *before* the work rather than after it. Two properties decide whether that is
    /// honest, and each is the failure mode of a different naive design:
    ///
    /// 1. **An exceeded budget refuses and carries no answer.** A truncated enumeration that still
    ///    answered would pick a different rejection from the full one — the option set is what
    ///    `compute_optimal_rejection` minimises over — so a budgeted search that returned its partial
    ///    results would be a **consensus change**, not a resource policy (law 17a). `Err` must therefore
    ///    be the only outcome, never "the options found so far".
    /// 2. **A budget that is not hit must not change the answer.** The node's own budget and the exact
    ///    search must return the *identical* option set on every shape, or the "node-local, no fork"
    ///    claim is false.
    ///
    /// The shapes are the four the enumeration's doc comment pins — complete, no-conflicts, fork and
    /// matching — because they are the extremes of the cost, and a bound that is safe on one and not
    /// another is not a bound.
    #[test]
    fn a_budget_refuses_without_answering_and_never_changes_the_answer() {
        let complete: BTreeMap<i32, BTreeSet<i32>> = (0..6)
            .map(|k| (k, (0..6).filter(|o| *o != k).collect()))
            .collect();
        // No conflicts at all: 2^n states to report one option. The node's normal case.
        let free: BTreeMap<i32, BTreeSet<i32>> = (0..12).map(|k| (k, set([]))).collect();
        // Two branches, complete across — a fork of two chains of three.
        let fork: BTreeMap<i32, BTreeSet<i32>> = map([
            (0, set([3, 4, 5])),
            (1, set([3, 4, 5])),
            (2, set([3, 4, 5])),
            (3, set([0, 1, 2])),
            (4, set([0, 1, 2])),
            (5, set([0, 1, 2])),
        ]);
        // A perfect matching of three pairs: 3^m states, 2^m options.
        let matching: BTreeMap<i32, BTreeSet<i32>> = map([
            (0, set([1])),
            (1, set([0])),
            (2, set([3])),
            (3, set([2])),
            (4, set([5])),
            (5, set([4])),
        ]);

        for (name, shapes) in [
            ("complete", complete),
            ("free", free),
            ("fork", fork),
            ("matching", matching),
        ] {
            let exact = compute_rejection_options(&shapes);

            // 2. Not hit -> identical answer, on the node's own budget.
            let node = compute_rejection_options_with_budget(&shapes, SearchBudget::NODE)
                .unwrap_or_else(|e| {
                    panic!("{name}: SearchBudget::NODE refused an honest shape: {e:?}")
                });
            assert_eq!(
                node, exact,
                "{name}: a budget that is not hit must not change the answer, or this is a fork"
            );

            // 1. Hit -> refused, and nothing returned that could be mistaken for an answer.
            let refused = compute_rejection_options_with_budget(
                &shapes,
                SearchBudget {
                    max_steps: 0,
                    max_options: 0,
                },
            );
            let e = refused.expect_err(&format!(
                "{name}: a budget of zero must refuse rather than answer"
            ));
            assert_eq!(e.budget.max_steps, 0);
        }
    }

    #[test]
    fn the_enumeration_expands_one_state_per_acyclic_subset() {
        // Complete conflict relation: no subset of size >= 2 is acyclic, so each key terminates alone.
        let n = 8i32;
        let dense: BTreeMap<i32, BTreeSet<i32>> = (0..n)
            .map(|k| (k, (0..n).filter(|o| *o != k).collect()))
            .collect();
        let keys: Vec<i32> = dense.keys().copied().collect();
        let (options, expanded, _) =
            enumerate_states(&dense, &keys, SearchBudget::UNBOUNDED).expect("unbounded");
        assert_eq!(expanded, n as usize, "one state per key, each terminal");
        assert_eq!(options.len(), n as usize);

        // No conflicts: every subset is acyclic, so every nonempty subset is a state...
        let free: BTreeMap<i32, BTreeSet<i32>> = (0..n).map(|k| (k, set::<i32>([]))).collect();
        let keys: Vec<i32> = free.keys().copied().collect();
        let (options, expanded, _) =
            enumerate_states(&free, &keys, SearchBudget::UNBOUNDED).expect("unbounded");
        assert_eq!(
            expanded,
            (1usize << n) - 1,
            "every nonempty subset of {n} non-conflicting keys is a state"
        );
        // ...and exactly one option comes out of all of them. This is the ratio that is the defect:
        // 2^n - 1 states expanded to report 1 option.
        assert_eq!(options, set([set::<i32>([])]));

        // Two branches of `p`, complete across: everything in one branch, nothing in the other.
        for p in [1usize, 4, 10] {
            let fork = fork_shape(2, p);
            let keys: Vec<i32> = fork.keys().copied().collect();
            let (options, expanded, _) =
                enumerate_states(&fork, &keys, SearchBudget::UNBOUNDED).expect("unbounded");
            assert_eq!(
                expanded,
                (1usize << (p + 1)) - 2,
                "fork of two branches of {p}"
            );
            assert_eq!(options.len(), 2, "reject one branch or the other");
        }

        // A perfect matching of `m` pairs: acyclic subsets are those with at most one endpoint per pair
        // (3^m of them, less the empty one), and the terminal ones take exactly one endpoint per pair,
        // so there are 2^m options — each a set of the *other* endpoints.
        for m in [2usize, 3, 4] {
            let matching = matching_shape(m);
            let keys: Vec<i32> = matching.keys().copied().collect();
            let (options, expanded, _) =
                enumerate_states(&matching, &keys, SearchBudget::UNBOUNDED).expect("unbounded");
            assert_eq!(expanded, 3usize.pow(m as u32) - 1, "matching of {m} pairs");
            assert_eq!(options.len(), 1usize << m, "one option per choice of side");
        }
    }

    /// The same four shapes, through the **shipped** function: the options must be identical (that is the
    /// whole contract — the merge outcome is consensus-visible, law 17a) and the work must be the number
    /// of maximal independent sets rather than the number of acyclic subsets.
    ///
    /// Two things are pinned, and they are the two that do not depend on the pivot rule:
    ///
    /// - the **options**, exactly, because they are the contract (the merge outcome is consensus-visible,
    ///   law 17a) and an exact equality is the only assertion that can catch a missing or invented option;
    /// - the **work**, as `expanded ≤ (keys + 1) × (options + 1)`. That bound is decisive rather than
    ///   decorative: the enumeration violates it on every one of these shapes — `free` needs `2^n - 1 =
    ///   255` against `18` at `n = 8` — so this is the assertion that fails if the fast path is ever
    ///   lost. Exact recursion counts are *not* pinned, deliberately: they are the pivot heuristic's
    ///   business, and a test that pinned them would fail on a better pivot.
    ///
    /// `max_frontier` is 0 throughout — this path has no queue at all, and that is the #117 property
    /// stated as a count: memory here is the option set, and nothing else.
    #[test]
    fn the_maximal_independent_set_path_reports_the_same_options_for_far_less_work() {
        let bounded =
            |census: &SearchCensus| census.expanded <= (census.keys + 1) * (census.options + 1);

        let n = 8i32;
        let dense: BTreeMap<i32, BTreeSet<i32>> = (0..n)
            .map(|k| (k, (0..n).filter(|o| *o != k).collect()))
            .collect();
        let (options, census) = compute_rejection_options_with_census(&dense);
        assert_eq!(options.len(), n as usize, "one option per singleton");
        assert!(bounded(&census), "dense: {census:?}");

        let free: BTreeMap<i32, BTreeSet<i32>> = (0..n).map(|k| (k, set::<i32>([]))).collect();
        let (options, census) = compute_rejection_options_with_census(&free);
        assert_eq!(options, set([set::<i32>([])]));
        assert!(
            bounded(&census),
            "no conflicts: 2^{n} - 1 states before, {} now ({census:?})",
            census.expanded
        );

        for p in [1usize, 4, 10] {
            let (options, census) = compute_rejection_options_with_census(&fork_shape(2, p));
            assert_eq!(options.len(), 2, "reject one branch or the other");
            assert!(
                bounded(&census),
                "fork of two branches of {p}: expanded {} for {} options on {} keys",
                census.expanded,
                census.options,
                census.keys
            );
        }

        for m in [2usize, 3, 4] {
            let (options, census) = compute_rejection_options_with_census(&matching_shape(m));
            assert_eq!(options.len(), 1usize << m, "one option per choice of side");
            // Inherently exponential — the options *are* that many — but no longer times 3^m alive at
            // once, which is what the bound rules out.
            assert!(bounded(&census), "matching of {m}: {census:?}");
        }

        // The fast path has no queue, so the frontier field is 0 rather than "not measured".
        let (_, census) = compute_rejection_options_with_census(&fork_shape(2, 10));
        assert_eq!(census.max_frontier, 0);
        assert_eq!(census.asymmetric, 0);
        assert_eq!(census.self_conflicts, 0);
    }

    /// Two branches of `p` chains, complete across: the shape `sdk/tests/merging_scaling.rs` uses.
    fn fork_shape(branches: usize, per_branch: usize) -> BTreeMap<i32, BTreeSet<i32>> {
        (0..branches * per_branch)
            .map(|k| {
                let branch = k / per_branch;
                (
                    k as i32,
                    (0..branches * per_branch)
                        .filter(|o| *o / per_branch != branch)
                        .map(|o| o as i32)
                        .collect(),
                )
            })
            .collect()
    }

    /// `m` disjoint conflicting pairs.
    fn matching_shape(m: usize) -> BTreeMap<i32, BTreeSet<i32>> {
        (0..2 * m)
            .map(|k| {
                let partner = if k % 2 == 0 { k + 1 } else { k - 1 };
                (k as i32, set([partner as i32]))
            })
            .collect()
    }

    /// The census must not be able to disagree with the function it describes: `with_census` is the
    /// same search, and this is the assertion that keeps it from becoming a copy that drifts. It also
    /// pins the *general* path's census fields on an asymmetric map, which is the path the fast one is
    /// meant to leave alone: there the frontier is real and the counters must track it.
    #[test]
    fn the_census_agrees_with_the_function_it_describes() {
        let conflicts_map = map([
            (1, set([2, 3])),
            (2, set([1])),
            (3, set([1, 4])),
            (4, set([3])),
        ]);
        let expected = compute_rejection_options(&conflicts_map);
        let (options, census) = compute_rejection_options_with_census(&conflicts_map);
        assert_eq!(options, expected, "the census form is the same function");
        assert_eq!(census.keys, 4);
        assert_eq!(census.conflicts, 2 + 1 + 2 + 1);
        assert_eq!(census.options, options.len());
        assert!(census.expanded > 0 && census.max_frontier <= census.expanded);

        // Asymmetric on the keys (1 conflicts with 3, 3 does not conflict with 1), so this is the
        // enumeration: a non-zero count in this field is what sends a map down that path.
        let asymmetric = map([(1, set([2])), (2, set([1])), (3, set([1]))]);
        let (_, census) = compute_rejection_options_with_census(&asymmetric);
        assert_eq!(census.asymmetric, 1, "3 -> 1 with no edge back");
        assert!(
            census.expanded > 0 && census.max_frontier > 0,
            "the general path has a frontier"
        );
    }

    #[test]
    fn compute_optimal_rejection_minimizes_cost_then_size() {
        let rejection_options = set([
            set([1, 2, 3]),
            set([2, 3, 4]),
            set([1, 2]),
            set([2]),
            set([1]),
        ]);
        // Every deploy costs 1; among the minimal-cost options the smallest set (then the
        // lexicographically smallest) wins — {1}.
        let cost_fn = |_d: &i32| 1i64;
        assert_eq!(
            compute_optimal_rejection(&rejection_options, cost_fn),
            set([1])
        );
    }

    #[test]
    fn add_mergeable_overflow_rejections_folds_by_branch() {
        let conflict_set = set([1, 2, 3, 4, 5, 6, 7]);
        let dependency_map = map([(1, set([2])), (3, set([4])), (4, set([5]))]);
        let reject_options: BTreeSet<BTreeSet<i32>> = set([]);
        let init: BTreeMap<String, i64> = [("a".to_string(), 0)].into_iter().collect();
        let mut md: BTreeMap<i32, BTreeMap<String, i64>> = BTreeMap::new();
        md.insert(1, [("a".to_string(), 10)].into_iter().collect());
        md.insert(2, [("a".to_string(), -5)].into_iter().collect());
        md.insert(3, [("a".to_string(), 15)].into_iter().collect());
        md.insert(4, [("a".to_string(), 10)].into_iter().collect());
        md.insert(5, [("a".to_string(), -20)].into_iter().collect());
        md.insert(6, [("a".to_string(), -10)].into_iter().collect());
        md.insert(7, [("a".to_string(), -10)].into_iter().collect());
        let r = add_mergeable_overflow_rejections(
            &conflict_set,
            &dependency_map,
            &reject_options,
            &init,
            &md,
        );
        assert_eq!(r, set([set([7])]));
    }

    #[test]
    fn add_mergeable_overflow_rejections_rejects_dependent_tree() {
        let conflict_set = set([1, 2, 3, 4, 5, 6, 7, 12]);
        let dependency_map = map([
            (1, set([2])),
            (2, set([12])),
            (3, set([4, 12])),
            (4, set([5])),
        ]);
        let reject_options: BTreeSet<BTreeSet<i32>> = set([]);
        let init: BTreeMap<String, i64> = [("a".to_string(), 5)].into_iter().collect();
        let mut md: BTreeMap<i32, BTreeMap<String, i64>> = BTreeMap::new();
        md.insert(1, [("a".to_string(), -10)].into_iter().collect());
        md.insert(2, [("a".to_string(), -5)].into_iter().collect());
        md.insert(3, [("a".to_string(), 15)].into_iter().collect());
        md.insert(4, [("a".to_string(), 10)].into_iter().collect());
        md.insert(5, [("a".to_string(), -20)].into_iter().collect());
        md.insert(6, [("a".to_string(), -10)].into_iter().collect());
        md.insert(7, [("a".to_string(), -10)].into_iter().collect());
        md.insert(12, [("a".to_string(), 10)].into_iter().collect());
        let r = add_mergeable_overflow_rejections(
            &conflict_set,
            &dependency_map,
            &reject_options,
            &init,
            &md,
        );
        assert_eq!(r, set([set([1, 2, 12, 7])]));
    }
}
