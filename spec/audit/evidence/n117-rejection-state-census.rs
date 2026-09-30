// The census behind #117's fix: how many queue entries the merge's rejection search pushes, against how
// many *distinct states* it is expanding.
//
// `compute_rejection_options` (`sdk/src/dag/merging.rs`) carried the rejected set in each queue entry
// beside the accepted set, so a state was re-expanded once per ordering reaching it and two `BTreeSet`s
// were cloned per push. Those clones are **97 %** of the live heap of a node at its cgroup ceiling
// (`n117-heap-profile-results.md`), which is what made this worth counting rather than reasoning about.
//
// This is a **standalone copy**, not a crate test: it is the pre-fix search kept verbatim so its census
// can be reproduced. It is not compiled by CI — it lives here as the artifact for the numbers below.
//
//     cp spec/audit/evidence/n117-rejection-state-census.rs sdk/tests/state_count.rs
//     cargo test -p rchain-sdk --release --test state_count -- --nocapture
//
// Measured 2026-09-30 on the fork shape (two branches of `per` chains; chains within a branch do not
// conflict, chains across branches conflict completely):
//
//     per_branch=5  (10 chains): popped=650        distinct_states=62
//     per_branch=8  (16 chains): popped=219200     distinct_states=510
//     per_branch=10 (20 chains): popped=19728200   distinct_states=2046
//
// The distinct count is exactly 2^(per+1) - 2 — the nonempty subsets of a branch, twice over — which is
// `rejection_options_scales_on_a_fork_shape`'s "the state count is the search's true shape". The ratio
// between the columns is the redundancy the fix removed: 9,642x at 20 chains, and factorially worse
// beyond it. `per_branch = 12` is left out because the *popped* column for it does not finish.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

fn fork(branches: usize, per: usize) -> BTreeMap<i32, BTreeSet<i32>> {
    let groups: Vec<Vec<i32>> = (0..branches)
        .map(|b| (0..per).map(|i| (b * per + i) as i32).collect())
        .collect();
    let mut m = BTreeMap::new();
    for (b, g) in groups.iter().enumerate() {
        for k in g {
            let mut c = BTreeSet::new();
            for (o, other) in groups.iter().enumerate() {
                if o != b {
                    c.extend(other.iter().copied());
                }
            }
            m.insert(*k, c);
        }
    }
    m
}

#[test]
fn distinct() {
    for per in [5usize, 8, 10] {
        let cm = fork(2, per);
        let all: Vec<i32> = cm.keys().copied().collect();
        let mut q: VecDeque<(i32, BTreeSet<i32>, BTreeSet<i32>)> = all
            .iter()
            .map(|k| (*k, BTreeSet::new(), std::iter::once(*k).collect()))
            .collect();
        let mut seen: BTreeSet<(BTreeSet<i32>, BTreeSet<i32>)> = BTreeSet::new();
        let (mut popped, mut dup) = (0usize, 0usize);
        while let Some((a, rj, ac)) = q.pop_front() {
            popped += 1;
            let mut nrj = rj.clone();
            if let Some(c) = cm.get(&a) {
                nrj.extend(c.iter().copied());
            }
            let mut nac = ac;
            nac.insert(a);
            if !seen.insert((nrj.clone(), nac.clone())) {
                dup += 1;
            }
            let next: Vec<i32> = all
                .iter()
                .filter(|k| !nrj.contains(k) && !nac.contains(k))
                .copied()
                .collect();
            if !next.is_empty() {
                for n in next {
                    q.push_back((n, nrj.clone(), nac.clone()));
                }
            }
        }
        println!(
            "per_branch={per} ({} chains): popped={popped} distinct_states={}",
            per * 2,
            seen.len()
        );
    }
}
