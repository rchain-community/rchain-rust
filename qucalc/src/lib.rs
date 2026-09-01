//! QuCalc closure superposition over `census_inventory.json`.
//!
//! The single invariant this crate enforces structurally: **`ways` is a coefficient, never
//! expanded.** A closure class with 173,280,448 ways is ONE [`WeightedClass`], not 173M terms.
//! This is the ρ-calculus merge monoid (Law 9) applied to the census data: identical terms
//! collapse into a term + multiplicity instead of being duplicated.

use std::collections::BTreeMap;
use std::path::Path;

/// A weighted event class: the phase-excursion class id plus its signed amplitude and its
/// multiplicity (`ways`). `ways` is carried as a `u64` coefficient — it is *never* materialized
/// into individual terms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WeightedClass {
    pub class: u32,
    pub signed: i64,
    pub ways: u64,
}

/// A superposition: a multiset of [`WeightedClass`]. Canonical form is most-ways-first.
pub type Superposition = Vec<WeightedClass>;

/// Merge monoid: combine two superpositions, summing `signed` and `ways` per class.
///
/// This is the Law-9 "merge multiplicities into coefficients" step — the performance-critical
/// guarantee: merging two 600M-way terms is one integer add, not 600M term copies.
pub fn merge(a: &[WeightedClass], b: &[WeightedClass]) -> Superposition {
    let mut m: BTreeMap<u32, WeightedClass> = BTreeMap::new();
    for w in a.iter().chain(b) {
        m.entry(w.class)
            .and_modify(|e| {
                e.signed += w.signed;
                e.ways += w.ways;
            })
            .or_insert(*w);
    }
    m.into_values().collect()
}

/// "Most ways first": the canonical order — sort by `ways` descending, `class` ascending as a
/// deterministic tie-break.
pub fn most_ways_first(mut ws: Superposition) -> Superposition {
    ws.sort_by(|x, y| y.ways.cmp(&x.ways).then_with(|| x.class.cmp(&y.class)));
    ws
}

/// Fold to the scalar receipt: the aggregate `(signed_sum, ways_sum)`.
///
/// The phase (±1 real vs ±i imaginary) is a *separate* Pauli-product predicate over the twist
/// history (`pauli_closed ∧ count_balanced`) and is deliberately out of scope here; this fold
/// emits the multiplicity-weighted aggregates the census stores.
pub fn fold(ws: &[WeightedClass]) -> (i64, u64) {
    let signed = ws.iter().map(|w| w.signed).sum();
    let ways = ws.iter().map(|w| w.ways).sum();
    (signed, ways)
}

// --- Census loading ---------------------------------------------------------

#[derive(serde::Deserialize)]
struct CensusJson {
    closures: BTreeMap<String, ClosureJson>,
}

#[derive(serde::Deserialize)]
struct ClosureJson {
    preparation: String,
    // NB: the census also carries a top-level `branches` list, but it is redundant with
    // the keys of `event_classes` (each branch is already a key below), so it is not
    // deserialized. serde ignores the unknown field.
    #[serde(rename = "event_classes")]
    event_classes: BTreeMap<String, BTreeMap<String, ClassJson>>,
}

#[derive(serde::Deserialize)]
struct ClassJson {
    signed: i64,
    ways: u64,
}

/// A loaded census: closure name -> (preparation, per-branch weighted classes).
pub struct Census {
    pub closures: BTreeMap<String, (String, BTreeMap<String, Vec<WeightedClass>>)>,
}

impl Census {
    /// Load `census_inventory.json`, expanding each `event_classes[branch][class]` entry into a
    /// [`WeightedClass`] (the class-id string parses as `u32`).
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let cj: CensusJson = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        let mut closures = BTreeMap::new();
        for (name, c) in cj.closures {
            let mut branches = BTreeMap::new();
            for (branch, classes) in c.event_classes {
                let mut ws = Vec::new();
                for (class, cc) in classes {
                    ws.push(WeightedClass {
                        class: class
                            .parse()
                            .map_err(|_| format!("bad class id {class:?}"))?,
                        signed: cc.signed,
                        ways: cc.ways,
                    });
                }
                branches.insert(branch, ws);
            }
            closures.insert(name, (c.preparation, branches));
        }
        Ok(Census { closures })
    }

    /// Build the full superposition for a closure (merge all branches), canonicalized most-ways-first.
    pub fn closure(&self, name: &str) -> Option<Superposition> {
        let (_, branches) = self.closures.get(name)?;
        let mut sup: Superposition = Vec::new();
        for ws in branches.values() {
            sup = merge(&sup, ws);
        }
        Some(most_ways_first(sup))
    }
}

// --- Pauli predicate (port of zfa-core `pauli.rs` + `history.rs`) -------------
//
// ZFA (half-spin closure) is the two-faced predicate over a twist history:
//   `achieves_zfa(h) = pauli_closed(h) ∧ count_balanced(h)`.
// The arithmetic is **exact integer complex** (entries in {-1, 0, 1}), not
// floating point, so the predicate is deterministic and replay-safe.

/// An exact integer complex number `(re, im)`; entries stay in {-1, 0, 1}.
type C = (i32, i32);

/// The 8-twist alphabet, value → SU(2) Pauli generator:
///   0 `^` = +σ_y   1 `v` = -σ_y   2 `>` = +σ_x   3 `<` = -σ_x
///   4 `/` = +σ_z   5 `\` = -σ_z   6 `+` = +I      7 `-` = -I
/// Positive = even values (0,2,4,6); negative = odd values (1,3,5,7).
fn twist_matrix(t: u8) -> (C, C, C, C) {
    // returns (a, b, c, d) for [[a, b], [c, d]]
    match t {
        0 => ((0, 0), (0, -1), (0, 1), (0, 0)),  // +σ_y
        1 => ((0, 0), (0, 1), (0, -1), (0, 0)),  // -σ_y
        2 => ((0, 0), (1, 0), (1, 0), (0, 0)),   // +σ_x
        3 => ((0, 0), (-1, 0), (-1, 0), (0, 0)), // -σ_x
        4 => ((1, 0), (0, 0), (0, 0), (-1, 0)),  // +σ_z
        5 => ((-1, 0), (0, 0), (0, 0), (1, 0)),  // -σ_z
        6 => ((1, 0), (0, 0), (0, 0), (1, 0)),   // +I
        7 => ((-1, 0), (0, 0), (0, 0), (-1, 0)), // -I
        _ => ((1, 0), (0, 0), (0, 0), (1, 0)),   // defensive identity
    }
}

fn cmul(a: C, b: C) -> C {
    (a.0 * b.0 - a.1 * b.1, a.0 * b.1 + a.1 * b.0)
}
fn cadd(a: C, b: C) -> C {
    (a.0 + b.0, a.1 + b.1)
}

/// The Pauli matrix product (fold) of a twist history, left-to-right.
pub fn pauli_fold(twists: &[u8]) -> (C, C, C, C) {
    twists
        .iter()
        .fold(((1, 0), (0, 0), (0, 0), (1, 0)), |acc, &t| {
            let (a, b, c, d) = acc;
            let (e, f, g, h) = twist_matrix(t);
            // [[a,b],[c,d]] · [[e,f],[g,h]]
            (
                cadd(cmul(a, e), cmul(b, g)),
                cadd(cmul(a, f), cmul(b, h)),
                cadd(cmul(c, e), cmul(d, g)),
                cadd(cmul(c, f), cmul(d, h)),
            )
        })
}

/// The scalar phase of a Pauli-closed fold: {+I, −I, +iI, −iI}.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    PlusI,
    MinusI,
    PlusImag,
    MinusImag,
}

impl Phase {
    /// Encode as an i64: 1 = +I, −1 = −I, 2 = +iI, −2 = −iI.
    pub fn code(self) -> i64 {
        match self {
            Phase::PlusI => 1,
            Phase::MinusI => -1,
            Phase::PlusImag => 2,
            Phase::MinusImag => -2,
        }
    }

    fn as_c(self) -> C {
        match self {
            Phase::PlusI => (1, 0),
            Phase::MinusI => (-1, 0),
            Phase::PlusImag => (0, 1),
            Phase::MinusImag => (0, -1),
        }
    }
}

const PHASES: [Phase; 4] = [
    Phase::PlusI,
    Phase::MinusI,
    Phase::PlusImag,
    Phase::MinusImag,
];

/// The scalar phase of the fold, or `None` if it is not Pauli-closed.
pub fn pauli_phase(twists: &[u8]) -> Option<Phase> {
    let (a, b, c, d) = pauli_fold(twists);
    if b != (0, 0) || c != (0, 0) || a != d {
        return None;
    }
    PHASES.iter().copied().find(|&p| p.as_c() == a)
}

/// True iff the Pauli fold lands in the scalar group {±I, ±iI}.
pub fn pauli_closed(twists: &[u8]) -> bool {
    pauli_phase(twists).is_some()
}

/// Count balance: `count_pos == count_neg` (even == odd twist values).
pub fn count_balanced(twists: &[u8]) -> bool {
    let (pos, neg) = twists.iter().fold((0i64, 0i64), |(p, n), &t| {
        if t % 2 == 0 {
            (p + 1, n)
        } else {
            (p, n + 1)
        }
    });
    pos == neg
}

/// ZFA = half-spin closure: Pauli-closed AND count-balanced.
pub fn achieves_zfa(twists: &[u8]) -> bool {
    pauli_closed(twists) && count_balanced(twists)
}

#[cfg(test)]
mod pauli_tests {
    use super::*;

    #[test]
    fn empty_history_is_zfa() {
        assert!(pauli_closed(&[]));
        assert!(count_balanced(&[]));
        assert_eq!(pauli_phase(&[]), Some(Phase::PlusI));
    }

    #[test]
    fn up_down_pair_closes_neg_identity() {
        // ^v = σ_y · −σ_y = −I
        assert!(pauli_closed(&[0, 1]));
        assert!(count_balanced(&[0, 1]));
        assert!(achieves_zfa(&[0, 1]));
        assert_eq!(pauli_phase(&[0, 1]), Some(Phase::MinusI));
    }

    #[test]
    fn plus_minus_pair_closes_neg_identity() {
        // +- = I · −I = −I
        assert!(pauli_closed(&[6, 7]));
        assert_eq!(pauli_phase(&[6, 7]), Some(Phase::MinusI));
    }

    #[test]
    fn xy_plane_loop_is_closed() {
        // ^<v> = σ_y · −σ_x · −σ_y · σ_x = −I
        assert!(pauli_closed(&[0, 3, 1, 2]));
        assert!(count_balanced(&[0, 3, 1, 2]));
    }

    #[test]
    fn single_twist_not_closed_and_not_balanced() {
        assert!(!pauli_closed(&[0]));
        assert!(!count_balanced(&[0]));
        assert!(!achieves_zfa(&[0]));
    }

    #[test]
    fn two_non_conjugate_not_closed() {
        // ^> = σ_y σ_x = −iσ_z — off-diagonal, not scalar
        assert!(!pauli_closed(&[0, 2]));
        assert!(pauli_phase(&[0, 2]).is_none());
    }
}

// --- Dialectical synthesis (port of `ai_demonstration.py::qlf_ai_coprocessor`) ---
//
// The neuro-symbolic coprocessor: Thesis and Antithesis are ZFA twist sequences; the
// shared "middle term" is the gauge pair `+-`; Blanket Fusion concatenates the two
// premises and annihilates the gauge pair ("Delayed Choice"), and the residue must be a
// stable ZFA closure (a fluxoid) — the Synthesis.

/// Twist values for the 8-symbol alphabet (see [`to_symbols`]).
pub const UP: u8 = 0;
pub const DOWN: u8 = 1;
pub const RIGHT: u8 = 2;
pub const LEFT: u8 = 3;
pub const SLASH: u8 = 4;
pub const BSLASH: u8 = 5;
pub const PLUS: u8 = 6;
pub const MINUS: u8 = 7;

/// Render a twist sequence back to its `^v<>\ /+-` symbol string.
pub fn to_symbols(twists: &[u8]) -> String {
    twists
        .iter()
        .map(|&t| match t {
            UP => '^',
            DOWN => 'v',
            RIGHT => '>',
            LEFT => '<',
            SLASH => '/',
            BSLASH => '\\',
            PLUS => '+',
            MINUS => '-',
            _ => '?',
        })
        .collect()
}

/// Parse a symbol string into twist values (`None` on an unknown symbol).
pub fn from_symbols(s: &str) -> Option<Vec<u8>> {
    s.chars()
        .map(|c| match c {
            '^' => Some(UP),
            'v' => Some(DOWN),
            '>' => Some(RIGHT),
            '<' => Some(LEFT),
            '/' => Some(SLASH),
            '\\' => Some(BSLASH),
            '+' => Some(PLUS),
            '-' => Some(MINUS),
            _ => None,
        })
        .collect()
}

/// Annihilate the first adjacent gauge pair (`+-` or `-+`) — the "Delayed Choice" step
/// that cancels the shared middle term between the two premises.
pub fn annihilate_gauge(twists: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(twists.len());
    let mut i = 0;
    while i < twists.len() {
        if i + 1 < twists.len()
            && ((twists[i] == PLUS && twists[i + 1] == MINUS)
                || (twists[i] == MINUS && twists[i + 1] == PLUS))
        {
            i += 2; // the gauge pair annihilates
        } else {
            out.push(twists[i]);
            i += 1;
        }
    }
    out
}

/// The result of a dialectical synthesis.
pub struct Synthesis {
    /// The two premises concatenated (before gauge annihilation).
    pub intersection: Vec<u8>,
    /// The residue after the middle-term gauge pair annihilates.
    pub geometry: Vec<u8>,
    /// Whether the residue is a stable ZFA closure (the synthesis holds).
    pub zfa: bool,
    /// The scalar phase of the residue, if Pauli-closed.
    pub phase: Option<Phase>,
}

/// Blanket Fusion of the Aristotle syllogism: Subject (S), Middle term (M = `+`/`-`),
/// Predicate (P). The two premises are `S+` and `-P`; fusing them and annihilating the
/// middle gauge pair yields the synthesis, verified ZFA-closed.
///
/// The middle-term gauge pair sits exactly at the premise seam (the injected `[+, -]`
/// between `subject` and `predicate`), so the residue is simply `subject ++ predicate`.
/// We deliberately do *not* run [`annihilate_gauge`] over the whole concatenation: that
/// helper cancels the first adjacent `+-`/`-+` anywhere, which would wrongly eat an
/// incidental gauge pair inside `subject`/`predicate` (or a subject ending in `-`).
pub fn dialectical_synthesis(subject: &[u8], predicate: &[u8]) -> Synthesis {
    let premise1 = [subject, &[PLUS]].concat(); // S + middle_pos
    let premise2 = [&[MINUS], predicate].concat(); // middle_neg + P
    let mut intersection = premise1;
    intersection.extend_from_slice(&premise2);
    let geometry = [subject, predicate].concat();
    let zfa = achieves_zfa(&geometry);
    let phase = pauli_phase(&geometry);
    Synthesis {
        intersection,
        geometry,
        zfa,
        phase,
    }
}

#[cfg(test)]
mod synthesis_tests {
    use super::*;

    #[test]
    fn annihilates_middle_gauge_pair() {
        // ^<+->v  ->  ^<>v
        assert_eq!(
            annihilate_gauge(&[UP, LEFT, PLUS, MINUS, RIGHT, DOWN]),
            vec![UP, LEFT, RIGHT, DOWN]
        );
    }

    #[test]
    fn socrates_syllogism_fuses_to_stable_fluxoid() {
        // Socrates -> Man -> Mortal : ^< (S) bounded to >v (P) via +- (M)
        let s = dialectical_synthesis(&[UP, LEFT], &[RIGHT, DOWN]);
        assert_eq!(to_symbols(&s.intersection), "^<+->v");
        assert_eq!(to_symbols(&s.geometry), "^<>v");
        assert!(s.zfa, "the R=4 fluxoid must be ZFA-closed");
        assert_eq!(s.phase, Some(Phase::PlusI));
    }

    #[test]
    fn parse_and_render_round_trip() {
        assert_eq!(to_symbols(&from_symbols("^<>v").unwrap()), "^<>v");
    }

    #[test]
    fn premise_internal_gauge_pair_is_preserved() {
        // The subject `+-` contains its own adjacent gauge pair. Blanket fusion must
        // annihilate only the injected middle term, leaving the subject's own twists
        // intact: `+-` ⊕ `>v` -> `+->v`, not `>v`.
        let s = dialectical_synthesis(&[PLUS, MINUS], &[RIGHT, DOWN]);
        assert_eq!(to_symbols(&s.intersection), "+-+->v");
        assert_eq!(to_symbols(&s.geometry), "+->v");
    }

    #[test]
    fn subject_ending_in_minus_keeps_its_tail() {
        // A subject ending in `-` must not pair its tail with the injected `+`: the
        // residue is the untouched subject followed by the predicate.
        let s = dialectical_synthesis(&[UP, MINUS], &[RIGHT, DOWN]);
        assert_eq!(to_symbols(&s.geometry), "^->v");
    }
}

// --- Neuro layer: deterministic name -> topology (port of quantum-os allocateTwists) ---

/// `allocateTwists(name)`: each byte `b` of the name yields one positive twist
/// `(b & 3) * 2` and one negative twist `((b >> 2) & 3) * 2 + 1`.
///
/// Always count-balanced and deterministic — the "neuro" transition map from a semantic
/// name to a ZFA topology. Pauli closure (order-dependent) is checked separately at
/// grant time.
pub fn allocate_twists(name: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(name.len() * 2);
    for &b in name.as_bytes() {
        out.push((b & 3) * 2); // positive (even): 0,2,4,6
        out.push(((b >> 2) & 3) * 2 + 1); // negative (odd): 1,3,5,7
    }
    out
}

#[cfg(test)]
mod neuro_tests {
    use super::*;

    #[test]
    fn allocate_twists_is_count_balanced_and_deterministic() {
        let a = allocate_twists("mortal");
        let b = allocate_twists("mortal");
        assert_eq!(a, b, "deterministic");
        assert!(count_balanced(&a), "count-balanced by construction");
        assert_eq!(a.len(), 6 * 2, "one pos + one neg twist per character");
    }
}

// --- Governance: deterministic liquid-democracy decision machinery --------------
//
// The pure functions backing the `rho:gov:*` system processes (see
// quantum-os `Governance.md` and `gov.ts`). Every function here is a **total,
// deterministic, order-insensitive** fold over canonical (sorted) maps so that
// every peer reproduces the identical result from the same signed envelopes —
// the "no central counter" guarantee. Base voting weight is `1 + trust level`.
//
// Members are identified by a string (their public key / handle). All iteration
// is over `BTreeMap`/`BTreeSet` (sorted) so results never depend on input order.

pub mod gov {
    use std::collections::{BTreeMap, BTreeSet};

    /// Resolve liquid-democracy weights.
    ///
    /// `direct_voters` are the members who cast a ballot; each member's vote walks
    /// its delegation chain to the first direct voter reached (cycles / dead-ends
    /// abstain), and `weight(d) = Σ (1 + level(m))` over members whose chain ends
    /// at `d`. A direct voter's own vote always counts for itself (direct voting
    /// overrides delegation).
    pub fn resolve_weights(
        direct_voters: &[String],
        delegations: &BTreeMap<String, String>,
        trust: &BTreeMap<String, i64>,
    ) -> BTreeMap<String, i64> {
        let dv: BTreeSet<&String> = direct_voters.iter().collect();
        let mut universe: BTreeSet<String> = BTreeSet::new();
        universe.extend(direct_voters.iter().cloned());
        universe.extend(delegations.keys().cloned());
        universe.extend(delegations.values().cloned());
        universe.extend(trust.keys().cloned());

        let mut weight: BTreeMap<String, i64> = BTreeMap::new();
        for m in universe {
            // Clamp: a caller-supplied negative trust level must not produce a
            // negative (or zero) base weight.
            let base = (1 + trust.get(&m).copied().unwrap_or(0)).max(0);
            if dv.contains(&m) {
                *weight.entry(m).or_insert(0) += base;
                continue;
            }
            let mut seen = BTreeSet::from([m.clone()]);
            let mut cur = m.clone();
            loop {
                let Some(next) = delegations.get(&cur) else {
                    break; // dead-end -> abstain
                };
                if dv.contains(next) {
                    *weight.entry(next.clone()).or_insert(0) += base;
                    break;
                }
                if !seen.insert(next.clone()) {
                    break; // cycle -> abstain
                }
                cur = next.clone();
            }
        }
        weight
    }

    /// Compute the admin-rooted web of trust as a least fixed point.
    ///
    /// `ratings` are `(rater, ratee, v)`; self-ratings are ignored and each
    /// conferral is capped at `min(v, level(rater) − 1)` (strictly below the
    /// rater's own level), so two level-0 members cannot bootstrap each other.
    /// Admins are the trust root (level 5). Returns `member -> level` (unrated
    /// members are level 0).
    pub fn trust_levels(
        ratings: &[(String, String, i64)],
        admins: &[String],
    ) -> BTreeMap<String, i64> {
        let mut universe: BTreeSet<String> = BTreeSet::new();
        for (r, e, _) in ratings {
            universe.insert(r.clone());
            universe.insert(e.clone());
        }
        universe.extend(admins.iter().cloned());

        let mut level: BTreeMap<String, i64> = universe.iter().map(|m| (m.clone(), 0)).collect();
        for a in admins {
            level.insert(a.clone(), 5);
        }
        // Monotone relaxation to the (unique) least fixed point; strictly-decreasing
        // conferrals bound the number of iterations by the size of the universe.
        for _ in 0..=universe.len() {
            let mut next = level.clone();
            for (r, e, v) in ratings {
                if r == e {
                    continue;
                }
                let lr = next.get(r).copied().unwrap_or(0);
                if lr >= 1 {
                    let cand = (*v).min(lr - 1).max(0);
                    let entry = next.entry(e.clone()).or_insert(0);
                    if cand > *entry {
                        *entry = cand;
                    }
                }
            }
            if next == level {
                break;
            }
            level = next;
        }
        level
    }

    /// Determine discredited members and the resulting (slashed) trust levels.
    ///
    /// A member `m` is discredited when at least `max(2, ⌈⅔·|eligible(m)|⌉)` members
    /// whose level is `≥ level(m)` have censured them. On discredit their level → 0
    /// and every voucher of `m` is slashed by the level they staked. The slash is a
    /// decreasing fixed point (slashing a voucher can cascade), so the whole step is
    /// iterated to convergence.
    pub fn censure(
        censures: &[(String, String)],
        levels: &BTreeMap<String, i64>,
        vouchers: &[(String, String, i64)],
    ) -> (BTreeSet<String>, BTreeMap<String, i64>) {
        let mut universe: BTreeSet<String> = BTreeSet::new();
        universe.extend(levels.keys().cloned());
        for (c, m) in censures {
            universe.insert(c.clone());
            universe.insert(m.clone());
        }
        for (v, e, _) in vouchers {
            universe.insert(v.clone());
            universe.insert(e.clone());
        }

        let mut level: BTreeMap<String, i64> = levels.clone();
        let mut discredited: BTreeSet<String> = BTreeSet::new();

        for _ in 0..=universe.len() {
            let mut newly: BTreeSet<String> = BTreeSet::new();
            for m in &universe {
                if discredited.contains(m) {
                    continue;
                }
                let lm = level.get(m).copied().unwrap_or(0);
                let eligible: Vec<&String> = universe
                    .iter()
                    .filter(|x| *x != m && level.get(*x).copied().unwrap_or(0) >= lm)
                    .collect();
                let quorum = ((2 * eligible.len() as i64 + 2) / 3).max(2);
                let n_censure = censures
                    .iter()
                    .filter(|(c, t)| t == m && level.get(c).copied().unwrap_or(0) >= lm)
                    .count() as i64;
                if n_censure >= quorum {
                    newly.insert(m.clone());
                }
            }
            if newly.is_empty() {
                break;
            }
            // Apply discredit + slash their vouchers (decreasing fixed point).
            for m in &newly {
                discredited.insert(m.clone());
                level.insert(m.clone(), 0);
                for (v, e, s) in vouchers {
                    if e == m {
                        let cur = level.get(v).copied().unwrap_or(0);
                        level.insert(v.clone(), (cur - *s).max(0));
                    }
                }
            }
        }

        (discredited, level)
    }

    /// Ranked-choice (instant-runoff) tally, weighted. Returns the winner or `None`.
    ///
    /// Each ballot is a member -> ranking (most-preferred first). Strict majority is
    /// `> total/2`; the lowest-count candidate(s) are eliminated each round, and an
    /// all-tied round breaks deterministically to the lexicographically smallest.
    pub fn tally_ranked(
        ballots: &BTreeMap<String, Vec<String>>,
        weights: &BTreeMap<String, i64>,
    ) -> Option<String> {
        let total: i64 = ballots
            .keys()
            .map(|m| weights.get(m).copied().unwrap_or(0))
            .filter(|w| *w > 0)
            .sum();
        if total <= 0 {
            return None;
        }
        let rankings: BTreeMap<String, Vec<String>> = ballots
            .iter()
            .filter(|(_, r)| !r.is_empty())
            .map(|(m, r)| (m.clone(), r.clone()))
            .collect();
        let mut eliminated: BTreeSet<String> = BTreeSet::new();

        loop {
            let mut counts: BTreeMap<String, i64> = BTreeMap::new();
            for (m, r) in &rankings {
                if let Some(first) = r.iter().find(|o| !eliminated.contains(*o)) {
                    *counts.entry(first.clone()).or_insert(0) +=
                        weights.get(m).copied().unwrap_or(0);
                }
            }
            if counts.is_empty() {
                return None;
            }
            let max = *counts.values().max().unwrap();
            if max * 2 > total {
                return counts
                    .iter()
                    .filter(|(_, c)| **c == max)
                    .min_by_key(|(k, _)| *k)
                    .map(|(k, _)| k.clone());
            }
            let min = *counts.values().min().unwrap();
            if min == max {
                // all remaining candidates tied; deterministic lexicographic tie-break
                return counts.keys().min().cloned();
            }
            for (k, _) in counts.iter().filter(|(_, c)| **c == min) {
                eliminated.insert(k.clone());
            }
        }
    }

    /// Approval tally, weighted. Each ballot is a member -> set of approved options.
    pub fn tally_approval(
        ballots: &BTreeMap<String, Vec<String>>,
        weights: &BTreeMap<String, i64>,
    ) -> Option<String> {
        let mut counts: BTreeMap<String, i64> = BTreeMap::new();
        for (m, approved) in ballots {
            let w = weights.get(m).copied().unwrap_or(0);
            for opt in approved {
                *counts.entry(opt.clone()).or_insert(0) += w;
            }
        }
        counts
            .into_iter()
            .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
            .map(|(k, _)| k)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn m(s: &str) -> String {
            s.to_string()
        }

        #[test]
        fn resolve_weights_transitive_delegation() {
            // A, B, C; B delegates A; C delegates B. Only A votes -> A carries 3.
            let direct = vec![m("A")];
            let delegations = BTreeMap::from([(m("B"), m("A")), (m("C"), m("B"))]);
            let trust = BTreeMap::new();
            let w = resolve_weights(&direct, &delegations, &trust);
            assert_eq!(w.get("A"), Some(&3));

            // C votes directly -> C reclaims 1, A drops to 2 (self + B).
            let direct = vec![m("A"), m("C")];
            let w = resolve_weights(&direct, &delegations, &trust);
            assert_eq!(w.get("A"), Some(&2));
            assert_eq!(w.get("C"), Some(&1));
        }

        #[test]
        fn resolve_weights_cycle_abstains() {
            // B -> C -> B, neither votes: the cycle abstains, A's own vote counts once.
            let direct = vec![m("A")];
            let delegations = BTreeMap::from([(m("B"), m("C")), (m("C"), m("B"))]);
            let w = resolve_weights(&direct, &delegations, &BTreeMap::new());
            assert_eq!(w.get("A"), Some(&1));
            assert_eq!(w.len(), 1);
        }

        #[test]
        fn trust_levels_admin_rooted_strictly_decreasing() {
            // Alice (admin) rates Bob 3; Bob rates Carol 2 -> Alice 5, Bob 3, Carol 2.
            let ratings = vec![
                (m("Alice"), m("Bob"), 3),
                (m("Bob"), m("Carol"), 2),
                (m("Carol"), m("Dave"), 5), // capped at Carol's own level - 1 = 1
            ];
            let admins = vec![m("Alice")];
            let lv = trust_levels(&ratings, &admins);
            assert_eq!(lv.get("Alice"), Some(&5));
            assert_eq!(lv.get("Bob"), Some(&3));
            assert_eq!(lv.get("Carol"), Some(&2));
            assert_eq!(
                lv.get("Dave"),
                Some(&1),
                "capped below the rater's own level"
            );
        }

        #[test]
        fn trust_levels_zero_members_cannot_bootstrap() {
            // Bob and Dave rate each other highly, but neither is an admin -> both 0.
            let ratings = vec![(m("Bob"), m("Dave"), 5), (m("Dave"), m("Bob"), 5)];
            let lv = trust_levels(&ratings, &[]);
            assert_eq!(lv.get("Bob"), Some(&0));
            assert_eq!(lv.get("Dave"), Some(&0));
        }

        #[test]
        fn censure_quorum_and_slashing() {
            // Three admins (A, B, C at level 5), D at level 0. A and B censure D.
            let levels = BTreeMap::from([(m("A"), 5), (m("B"), 5), (m("C"), 5), (m("D"), 0)]);
            let censures = vec![(m("A"), m("D")), (m("B"), m("D"))];
            let vouchers = vec![(m("A"), m("D"), 2), (m("B"), m("D"), 1)];
            let (disc, lv) = censure(&censures, &levels, &vouchers);
            assert!(disc.contains("D"));
            assert_eq!(lv.get("D"), Some(&0));
            assert_eq!(
                lv.get("A"),
                Some(&3),
                "A slashed by the 2 levels they staked"
            );
            assert_eq!(
                lv.get("B"),
                Some(&4),
                "B slashed by the 1 level they staked"
            );
            assert_eq!(lv.get("C"), Some(&5));
        }

        #[test]
        fn censure_floor_of_two_blocks_lone_censure() {
            // A single censure never discredits, even from an admin over a level-0 target.
            let levels = BTreeMap::from([(m("A"), 5), (m("D"), 0)]);
            let censures = vec![(m("A"), m("D"))];
            let (disc, lv) = censure(&censures, &levels, &[]);
            assert!(disc.is_empty(), "one vote is short of the floor of 2");
            assert_eq!(lv.get("D"), Some(&0));
        }

        #[test]
        fn tally_ranked_instant_runoff() {
            // 3 candidates; C's voter is eliminated and flows to X.
            let ballots = BTreeMap::from([
                (m("A"), vec![m("X"), m("Y")]),
                (m("B"), vec![m("Y"), m("X")]),
                (m("C"), vec![m("Z"), m("X")]),
            ]);
            let weights = BTreeMap::from([(m("A"), 2), (m("B"), 2), (m("C"), 1)]);
            assert_eq!(tally_ranked(&ballots, &weights).as_deref(), Some("X"));
        }

        #[test]
        fn tally_approval_weighted() {
            let ballots = BTreeMap::from([(m("A"), vec![m("X")]), (m("B"), vec![m("X"), m("Y")])]);
            let weights = BTreeMap::from([(m("A"), 2), (m("B"), 3)]);
            assert_eq!(tally_approval(&ballots, &weights).as_deref(), Some("X"));
        }
    }
}
