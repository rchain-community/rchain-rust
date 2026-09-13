//! Maximum bipartite matching (port of `matcher/MaximumBipartiteMatch.scala`).
//!
//! The Scala `StateT[F, S, A]` backtracking is ported to an explicit `&mut State` recursion. All
//! patterns must be assigned a match; otherwise `find_matches` returns `None`.

use std::collections::{HashMap, HashSet};

type Pattern<P> = (P, Vec<usize>);

struct State<P, R> {
    matches: HashMap<usize, (Pattern<P>, R)>,
    seen: HashSet<usize>,
}

fn find_match<P, T, R>(
    pattern: &Pattern<P>,
    state: &mut State<P, R>,
    targets: &[T],
    match_fn: &dyn Fn(&P, &T) -> Option<R>,
) -> bool
where
    P: Clone,
    R: Clone,
{
    let (p, candidates) = pattern;
    if candidates.is_empty() {
        return false;
    }
    let candidate = candidates[0];
    let rest = &candidates[1..];
    if state.seen.contains(&candidate) {
        return find_match(&(p.clone(), rest.to_vec()), state, targets, match_fn);
    }
    match match_fn(p, &targets[candidate]) {
        Some(result) => {
            state.seen.insert(candidate);
            try_claim_match(candidate, pattern, result, state, targets, match_fn)
        }
        None => find_match(&(p.clone(), rest.to_vec()), state, targets, match_fn),
    }
}

fn try_claim_match<P, T, R>(
    candidate: usize,
    pattern: &Pattern<P>,
    result: R,
    state: &mut State<P, R>,
    targets: &[T],
    match_fn: &dyn Fn(&P, &T) -> Option<R>,
) -> bool
where
    P: Clone,
    R: Clone,
{
    match state.matches.get(&candidate).cloned() {
        None => {
            state.matches.insert(candidate, (pattern.clone(), result));
            true
        }
        Some((previous_pattern, _)) => {
            if find_match(&previous_pattern, state, targets, match_fn) {
                state.matches.insert(candidate, (pattern.clone(), result));
                true
            } else {
                let rest = pattern.1[1..].to_vec();
                find_match(&(pattern.0.clone(), rest), state, targets, match_fn)
            }
        }
    }
}

/// Find a maximum matching where every pattern is matched (port of `findMatches`).
pub fn find_matches<P, T, R>(
    patterns: &[P],
    targets: &[T],
    match_fn: &dyn Fn(&P, &T) -> Option<R>,
) -> Option<Vec<(T, P, R)>>
where
    P: Clone,
    T: Clone,
    R: Clone,
{
    let candidates: Vec<usize> = (0..targets.len()).collect();
    let mut state = State {
        matches: HashMap::new(),
        seen: HashSet::new(),
    };
    for p in patterns {
        state.seen.clear();
        let pattern = (p.clone(), candidates.clone());
        if !find_match(&pattern, &mut state, targets, match_fn) {
            return None;
        }
    }
    // Sort the matches by candidate index: `HashMap::into_iter()` order is per-process-randomized,
    // and the output Vec order must be canonical (Law 8 deterministic COMM).
    let mut matched: Vec<(usize, T, P, R)> = state
        .matches
        .into_iter()
        .map(|(idx, (pat, res))| (idx, targets[idx].clone(), pat.0, res))
        .collect();
    matched.sort_by_key(|(idx, _, _, _)| *idx);
    let out: Vec<(T, P, R)> = matched.into_iter().map(|(_, t, p, r)| (t, p, r)).collect();
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pattern is a name; a target is a name; a pattern matches a target when their sets
    /// intersect. The match result is the intersection, so the returned `R` is checkable.
    fn matcher() -> impl Fn(&&str, &&str) -> Option<String> {
        |p: &&str, t: &&str| {
            let hit: Vec<&str> = p
                .split('|')
                .filter(|name| t.split('|').any(|other| other == *name))
                .collect();
            if hit.is_empty() {
                None
            } else {
                Some(hit.join("+"))
            }
        }
    }

    /// The one-pattern case, with the match result carried through: the triple is
    /// `(target, pattern, result)`, so the caller can see *which* target was claimed.
    #[test]
    fn a_single_pattern_claims_the_first_target_it_matches() {
        let match_fn = matcher();
        let out = find_matches(&["a"], &["x", "a"], &match_fn).expect("a matches the second");
        assert_eq!(
            out,
            vec![("a", "a", "a".to_string())],
            "the second target is the only match"
        );
    }

    /// Every pattern must be matched, so one that matches nothing makes the whole call `None` —
    /// the caller (a COMM) must not proceed with a partial assignment.
    #[test]
    fn one_unmatchable_pattern_fails_the_whole_match() {
        let match_fn = matcher();
        assert_eq!(find_matches(&["a", "zzz"], &["a"], &match_fn), None);
        assert_eq!(find_matches(&["zzz"], &["a"], &match_fn), None);
        // Two patterns competing for one target: only one can have it.
        assert_eq!(find_matches(&["a", "a"], &["a"], &match_fn), None);
        // No patterns at all is a trivially satisfied match.
        assert_eq!(find_matches::<&str, &str, String>(&[], &["a"], &match_fn), Some(Vec::new()));
        // …and a pattern with no target to try is unmatchable too (its candidate list is empty).
        assert_eq!(find_matches(&["a"], &[], &match_fn), None);
    }

    /// The reassignment path, **forced**: the flexible pattern claims `t0` on its first pass (it
    /// takes candidates in index order), and the pattern that can only take `t0` has nowhere else to
    /// go — so it must push the flexible one onto `t1`.
    ///
    /// This is the case the algorithm exists for, and it is also the case a `seen` set that survived
    /// across patterns would break: the fixed pattern would *skip* the already-claimed `t0` and fail
    /// the whole match, turning a successful COMM into no match at all.
    #[test]
    fn a_claimed_target_is_taken_back_when_the_other_pattern_cannot_move() {
        let match_fn = matcher();
        let out = find_matches(&["t0|t1", "t0"], &["t0", "t1"], &match_fn)
            .expect("both are matchable — the flexible pattern must give way");
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].0, "t0", "the fixed pattern keeps the target it needs");
        assert_eq!(out[0].1, "t0");
        assert_eq!(out[1].0, "t1", "the flexible one was pushed to the other target");
        assert_eq!(out[1].1, "t0|t1");
        assert_eq!(out[1].2, "t1", "the result is for the target it actually got");

        // The mirror order: a pattern that can only take `t1` while the flexible one still starts
        // at `t0` — no reassignment is needed and both keep what they took first.
        let out = find_matches(&["t0|t1", "t1"], &["t0", "t1"], &match_fn).expect("matchable");
        assert_eq!(out[0].1, "t0|t1");
        assert_eq!(out[1].1, "t1");
    }

    /// The output is **sorted by target index**, because the intermediate map is a `HashMap` whose
    /// iteration order is randomized per process — and a COMM's produce order (Law 8) must not
    /// depend on that. Asserted over a case with several matches, where the sort has work to do.
    #[test]
    fn the_output_is_ordered_by_target_index_not_by_map_iteration() {
        let match_fn = matcher();
        let targets = ["t0", "t1", "t2", "t3"];
        let out = find_matches(&["t0|t1|t2|t3", "t1", "t3"], &targets, &match_fn)
            .expect("all three are matchable");
        let claimed: Vec<&str> = out.iter().map(|(t, _, _)| *t).collect();
        assert_eq!(claimed, vec!["t0", "t1", "t3"], "in target order");

        // Running it again gives the identical vector: nothing here depends on hash order.
        let again = find_matches(&["t0|t1|t2|t3", "t1", "t3"], &targets, &match_fn).expect("again");
        assert_eq!(again, out);
    }

    /// A target can be claimed by exactly one pattern, and the number of matches equals the number
    /// of patterns — not the number of candidate pairs.
    #[test]
    fn each_target_is_claimed_at_most_once() {
        let match_fn = matcher();
        let out = find_matches(&["a|b|c", "a|b|c", "a|b|c"], &["a", "b", "c"], &match_fn)
            .expect("matchable");
        assert_eq!(out.len(), 3, "three patterns, three matches");
        let mut targets: Vec<&str> = out.iter().map(|(t, _, _)| *t).collect();
        targets.sort();
        targets.dedup();
        assert_eq!(targets, vec!["a", "b", "c"], "each target claimed exactly once");
    }

}
