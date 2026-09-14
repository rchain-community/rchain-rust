//! Counting the fields of a `Par` for spatial matching (port of `matcher/ParCount.scala`).

use rchain_models::ast::{Connective, ConnectiveBody, Expr, Par, Sort, Var};

use crate::matcher::par_spatial_matcher_utils::no_frees;

/// The count of each `Par` field (port of `ParCount`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParCount {
    pub sends: i32,
    pub receives: i32,
    pub news: i32,
    pub exprs: i32,
    pub matches: i32,
    pub unforgeables: i32,
    pub bundles: i32,
}

/// Saturating add: saturates to `i32::MAX` on positive overflow (port of `saturatingAdd`).
fn saturating_add(l: i32, r: i32) -> i32 {
    let res = l.wrapping_add(r);
    let mask = if res < l { -1 } else { 0 };
    (res | mask) & !i32::MIN
}

impl ParCount {
    fn bin_op(&self, op: fn(i32, i32) -> i32, other: &ParCount) -> ParCount {
        ParCount {
            sends: op(self.sends, other.sends),
            receives: op(self.receives, other.receives),
            news: op(self.news, other.news),
            exprs: op(self.exprs, other.exprs),
            matches: op(self.matches, other.matches),
            unforgeables: op(self.unforgeables, other.unforgeables),
            bundles: op(self.bundles, other.bundles),
        }
    }

    pub fn min(&self, other: &ParCount) -> ParCount {
        self.bin_op(i32::min, other)
    }

    pub fn max(&self, other: &ParCount) -> ParCount {
        self.bin_op(i32::max, other)
    }

    pub fn add(&self, other: &ParCount) -> ParCount {
        self.bin_op(saturating_add, other)
    }

    pub fn from_par<S: Sort>(par: &Par<S>) -> ParCount {
        ParCount {
            sends: par.sends.len() as i32,
            receives: par.receives.len() as i32,
            news: par.news.len() as i32,
            matches: par.matches.len() as i32,
            exprs: par.exprs.len() as i32,
            unforgeables: par.unforgeables.len() as i32,
            bundles: par.bundles.len() as i32,
        }
    }

    pub fn max_count() -> ParCount {
        ParCount {
            sends: i32::MAX,
            receives: i32::MAX,
            news: i32::MAX,
            matches: i32::MAX,
            exprs: i32::MAX,
            unforgeables: i32::MAX,
            bundles: i32::MAX,
        }
    }

    fn is_free_var(expr: &Expr) -> bool {
        match expr {
            Expr::EVar(v) => matches!(**v, Var::FreeVar(_) | Var::Wildcard),
            _ => false,
        }
    }

    /// Compute the min/max field counts a `Par` pattern can match (port of `ParCount.minMax`).
    pub fn min_max(par: &Par) -> (ParCount, ParCount) {
        let pc = ParCount::from_par(&no_frees(par));
        let wildcard = par.exprs.iter().any(ParCount::is_free_var);
        let min_init = pc.clone();
        let max_init = if wildcard { ParCount::max_count() } else { pc };
        par.connectives
            .iter()
            .fold((min_init, max_init), |(min, max), con| {
                let (cmin, cmax) = ParCount::min_max_connective(con);
                (min.add(&cmin), max.add(&cmax))
            })
    }

    pub fn min_max_connective(con: &Connective) -> (ParCount, ParCount) {
        match con {
            Connective::ConnAnd(ConnectiveBody { ps }) => {
                let p_min_max: Vec<(ParCount, ParCount)> =
                    ps.iter().map(ParCount::min_max).collect();
                let min = p_min_max
                    .iter()
                    .fold(ParCount::default(), |acc, (m, _)| acc.max(m));
                let max = p_min_max
                    .iter()
                    .fold(ParCount::max_count(), |acc, (_, m)| acc.min(m));
                (min, max)
            }
            Connective::ConnOr(ConnectiveBody { ps }) => {
                let p_min_max: Vec<(ParCount, ParCount)> =
                    ps.iter().map(ParCount::min_max).collect();
                let min = p_min_max
                    .iter()
                    .fold(ParCount::max_count(), |acc, (m, _)| acc.min(m));
                let max = p_min_max
                    .iter()
                    .fold(ParCount::default(), |acc, (_, m)| acc.max(m));
                (min, max)
            }
            Connective::ConnNot(_) => (ParCount::default(), ParCount::max_count()),
            Connective::Empty | Connective::VarRef(_) => (ParCount::default(), ParCount::default()),
            Connective::ConnBool(_)
            | Connective::ConnInt(_)
            | Connective::ConnBigInt(_)
            | Connective::ConnString(_)
            | Connective::ConnUri(_)
            | Connective::ConnByteArray(_) => (
                ParCount {
                    exprs: 1,
                    ..ParCount::default()
                },
                ParCount {
                    exprs: 1,
                    ..ParCount::default()
                },
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rchain_models::ast::{Proc, VarRef};

    /// A one-field count, so an assertion can name the field it is about.
    fn count_exprs(n: i32) -> ParCount {
        ParCount {
            exprs: n,
            ..ParCount::default()
        }
    }

    fn free_var() -> Expr {
        Expr::EVar(Box::new(Var::FreeVar(0)))
    }

    fn wildcard() -> Expr {
        Expr::EVar(Box::new(Var::Wildcard))
    }

    fn bound_var() -> Expr {
        Expr::EVar(Box::new(Var::BoundVar(0)))
    }

    /// The arithmetic is **component-wise**: each of the seven fields is combined independently, so
    /// a `min` that ran over one field twice (a copy-paste in `bin_op`) is visible.
    #[test]
    fn min_max_and_add_are_component_wise() {
        let a = ParCount {
            sends: 1,
            receives: 5,
            news: 3,
            exprs: 2,
            matches: 9,
            unforgeables: 4,
            bundles: 7,
        };
        let b = ParCount {
            sends: 6,
            receives: 2,
            news: 3,
            exprs: 8,
            matches: 1,
            unforgeables: 4,
            bundles: 0,
        };

        let min = a.min(&b);
        assert_eq!((min.sends, min.receives, min.news, min.exprs), (1, 2, 3, 2));
        assert_eq!((min.matches, min.unforgeables, min.bundles), (1, 4, 0));

        let max = a.max(&b);
        assert_eq!((max.sends, max.receives, max.news, max.exprs), (6, 5, 3, 8));
        assert_eq!((max.matches, max.unforgeables, max.bundles), (9, 4, 7));

        let sum = a.add(&b);
        assert_eq!(
            (sum.sends, sum.receives, sum.news, sum.exprs),
            (7, 7, 6, 10)
        );
        assert_eq!((sum.matches, sum.unforgeables, sum.bundles), (10, 8, 7));

        // min/max saturate at the extremes without needing to; `add` is the saturating one.
        assert_eq!(a.max(&ParCount::max_count()), ParCount::max_count());
        assert_eq!(a.min(&ParCount::max_count()), a);
    }

    /// `add` saturates at `i32::MAX` rather than wrapping — a wrapped count would turn a huge
    /// pattern into a small one, which is exactly the comparison the spatial matcher makes.
    #[test]
    fn add_saturates_positively_and_only_positively() {
        assert_eq!(saturating_add(i32::MAX, 1), i32::MAX);
        assert_eq!(saturating_add(i32::MAX, i32::MAX), i32::MAX);
        assert_eq!(saturating_add(1, i32::MAX), i32::MAX);
        assert_eq!(saturating_add(i32::MAX - 1, 1), i32::MAX);
        assert_eq!(saturating_add(3, 4), 7);
        // …and the wart, precisely: the overflow test is `res < l`, so a sum that lands *below* the
        // left operand saturates too, which for a negative right operand is a spurious `MAX` and
        // not a wrap. `ParCount.scala` carries the same behaviour and the same comment ("Only
        // saturates going from positive to negative"). Unreachable in practice: counts come from
        // `from_par`, so both operands are non-negative.
        assert_eq!(saturating_add(-3, -4), i32::MAX);
        // The one case that wraps rather than saturating, because `0 < i32::MIN` is false so the
        // mask never fires — pinned so that "fixing" it is a deliberate act, not a drive-by.
        assert_eq!(saturating_add(0, i32::MIN), i32::MAX);
        assert_eq!(saturating_add(i32::MIN, i32::MIN), 0);

        let mut full = ParCount::max_count();
        assert_eq!(
            full.add(&ParCount {
                sends: 1,
                ..ParCount::default()
            })
            .sends,
            i32::MAX
        );
        full.sends = i32::MAX - 1;
        assert_eq!(
            full.add(&ParCount {
                sends: 5,
                ..ParCount::default()
            })
            .sends,
            i32::MAX
        );
    }

    /// `from_par` counts the seven fields of a par, each from its own list.
    #[test]
    fn from_par_counts_each_field() {
        let par: Par = Par {
            exprs: vec![bound_var()],
            ..Par::default()
        };
        let c = ParCount::from_par(&par);
        assert_eq!(c.exprs, 1);
        assert_eq!(c.sends, 0);
        assert_eq!(
            ParCount::from_par(&Proc::default()),
            ParCount::default(),
            "the empty par is the zero count"
        );
        // Every field is counted from its own list; the ones a hand-built par cannot easily have
        // are checked through the struct's own max, which sets all seven.
        assert_eq!(ParCount::from_par(&Proc::default()).receives, 0);
        assert_eq!(ParCount::from_par(&Proc::default()).news, 0);
        assert_eq!(ParCount::from_par(&Proc::default()).matches, 0);
        assert_eq!(ParCount::from_par(&Proc::default()).unforgeables, 0);
        assert_eq!(ParCount::from_par(&Proc::default()).bundles, 0);
        assert_eq!(
            ParCount::max_count(),
            ParCount {
                sends: i32::MAX,
                receives: i32::MAX,
                news: i32::MAX,
                exprs: i32::MAX,
                matches: i32::MAX,
                unforgeables: i32::MAX,
                bundles: i32::MAX,
            }
        );
    }

    /// A **free variable** (or wildcard) in the pattern is what makes the maximum unbounded: it can
    /// match any number of fields, so the max is `max_count` while the min is still the concrete
    /// count of everything *else*. A bound variable is not free and is counted normally.
    #[test]
    fn a_free_variable_widens_only_the_maximum() {
        let par: Par = Par {
            exprs: vec![bound_var()],
            ..Par::default()
        };
        let (min, max) = ParCount::min_max(&par);
        assert_eq!(min, count_exprs(1));
        assert_eq!(max, count_exprs(1), "a bound var is counted");

        let par: Par = Par {
            exprs: vec![free_var()],
            ..Par::default()
        };
        let (min, max) = ParCount::min_max(&par);
        assert_eq!(
            min,
            ParCount::default(),
            "the free variable itself is removed before counting (no_frees)"
        );
        assert_eq!(max, ParCount::max_count(), "…but it can match anything");

        let par: Par = Par {
            exprs: vec![wildcard()],
            ..Par::default()
        };
        let (min, max) = ParCount::min_max(&par);
        assert_eq!(min, ParCount::default());
        assert_eq!(max, ParCount::max_count());
    }

    /// Connectives: a conjunction's minimum is the **max** of its parts' minima (every part must be
    /// satisfied, so the demand is the largest one) and its maximum is the **min** of the parts'
    /// maxima; a disjunction is the mirror image. Getting these two the wrong way round is the
    /// classic error here, and it silently admits or rejects spatial matches.
    #[test]
    fn conjunction_and_disjunction_take_opposite_folds() {
        let two = |n: i32| Par {
            exprs: vec![bound_var(); n as usize],
            ..Par::default()
        };

        let and = Connective::ConnAnd(ConnectiveBody {
            ps: vec![two(1), two(4)],
        });
        let (min, max) = ParCount::min_max_connective(&and);
        assert_eq!(min.exprs, 4, "a conjunction demands the larger minimum");
        assert_eq!(max.exprs, 1, "…and is bounded by the smaller maximum");

        let or = Connective::ConnOr(ConnectiveBody {
            ps: vec![two(1), two(4)],
        });
        let (min, max) = ParCount::min_max_connective(&or);
        assert_eq!(
            min.exprs, 1,
            "a disjunction demands only the smaller minimum"
        );
        assert_eq!(max.exprs, 4, "…and admits up to the larger maximum");

        // A single-part connective is that part, for both.
        let single_and = Connective::ConnAnd(ConnectiveBody { ps: vec![two(3)] });
        assert_eq!(ParCount::min_max_connective(&single_and).0.exprs, 3);
        assert_eq!(ParCount::min_max_connective(&single_and).1.exprs, 3);

        // An empty one folds from the identity of each side: `add`'s zero for the minimum and
        // `max_count` for the maximum.
        let empty_and = Connective::ConnAnd(ConnectiveBody { ps: Vec::new() });
        assert_eq!(
            ParCount::min_max_connective(&empty_and).0,
            ParCount::default()
        );
        assert_eq!(
            ParCount::min_max_connective(&empty_and).1,
            ParCount::max_count()
        );
    }

    /// Negation can match anything (`0 .. max`), a literal connective is one `expr`, and the
    /// structural connectives (a `var` reference, the empty connective) contribute nothing.
    #[test]
    fn the_remaining_connectives_have_their_own_counts() {
        let negated = Connective::ConnNot(Box::default());
        assert_eq!(
            ParCount::min_max_connective(&negated),
            (ParCount::default(), ParCount::max_count())
        );

        for literal in [
            Connective::ConnBool(true),
            Connective::ConnInt(true),
            Connective::ConnBigInt(true),
            Connective::ConnString(true),
            Connective::ConnUri(true),
            Connective::ConnByteArray(true),
        ] {
            assert_eq!(
                ParCount::min_max_connective(&literal),
                (count_exprs(1), count_exprs(1)),
                "a literal connective is one expr: {literal:?}"
            );
        }

        for structural in [
            Connective::Empty,
            Connective::VarRef(VarRef { index: 0, depth: 0 }),
        ] {
            assert_eq!(
                ParCount::min_max_connective(&structural),
                (ParCount::default(), ParCount::default()),
                "no fields of its own: {structural:?}"
            );
        }
    }

    /// `min_max` adds each connective's range to the par's own count, so a par with one field and
    /// one connective carries both — the sum is where an off-by-one in the fold would show.
    #[test]
    fn a_pars_own_count_adds_to_its_connectives() {
        let par: Par = Par {
            exprs: vec![bound_var(), bound_var()],
            connectives: vec![Connective::ConnAnd(ConnectiveBody {
                ps: vec![Par {
                    exprs: vec![bound_var()],
                    ..Par::default()
                }],
            })],
            ..Par::default()
        };

        let (min, max) = ParCount::min_max(&par);
        assert_eq!(min.exprs, 3, "two of its own plus one from the connective");
        assert_eq!(max.exprs, 3);

        // A free variable in the par's own exprs makes the whole maximum unbounded, connectives
        // included.
        let par: Par = Par {
            exprs: vec![free_var(), bound_var()],
            connectives: vec![Connective::ConnAnd(ConnectiveBody {
                ps: vec![Par {
                    exprs: vec![bound_var()],
                    ..Par::default()
                }],
            })],
            ..Par::default()
        };
        let (min, max) = ParCount::min_max(&par);
        assert_eq!(min.exprs, 2, "the bound var and the connective's par");
        assert_eq!(max, ParCount::max_count());
    }
}
