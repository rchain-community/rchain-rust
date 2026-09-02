//! Experimental `TreeProc` execution representation for issue #8.
//!
//! This module is **experimental** and is not on the production reduction path. It explores
//! whether an explicit parallel tree can preserve reduction topology until the Law 1
//! canonicalization boundary.
//!
//! A `TreeProc` is deliberately non-canonical: `Par` children are explicit binary nodes and
//! no sorting is performed until [`TreeProc::into_sorted_par`]. The full design/phase plan is
//! in `docs/src/qucalc/zfa-concurrent-reducer.md`.

use rchain_models::ast::{Bundle, Expr, Match, New, Par, Receive, Send};
use rchain_models::sorted::SortedProc;

/// An owned parallel-process execution tree.
///
/// Leaves mirror the reducer's current `OwnedTerm` choices plus an inert tail for Par fields
/// that are not independently reduced (ground expressions, unforgeables, connectives).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TreeProc {
    /// Parallel composition. Children are deliberately not sorted or absorbed here.
    Par {
        left: Box<TreeProc>,
        right: Box<TreeProc>,
    },
    /// The empty process.
    Nil,
    Send(Send),
    Receive(Receive),
    New(New),
    Match(Match),
    Bundle(Bundle),
    /// A top-level expression the reducer treats as a work unit (`EVar`/`EMethod`).
    Expr(Expr),
    /// Inert `Par` contents preserved for canonical reconstruction.
    Tail(Par),
}

impl TreeProc {
    /// Build a `TreeProc` from a flat `Par` without canonicalizing.
    ///
    /// Sibling order is the source `Par` field order; no `sort_par_term` call is made here.
    pub fn from_par(par: &Par) -> Self {
        let mut leaves: Vec<TreeProc> = Vec::new();

        for s in &par.sends {
            leaves.push(TreeProc::Send(s.clone()));
        }
        for r in &par.receives {
            leaves.push(TreeProc::Receive(r.clone()));
        }
        for n in &par.news {
            leaves.push(TreeProc::New(n.clone()));
        }
        for m in &par.matches {
            leaves.push(TreeProc::Match(m.clone()));
        }
        for b in &par.bundles {
            leaves.push(TreeProc::Bundle(b.clone()));
        }

        let mut inert_exprs = Vec::new();
        for e in &par.exprs {
            match e {
                Expr::EVar(_) | Expr::EMethod(_) => leaves.push(TreeProc::Expr(e.clone())),
                _ => inert_exprs.push(e.clone()),
            }
        }

        if !inert_exprs.is_empty() || !par.unforgeables.is_empty() || !par.connectives.is_empty() {
            let tail = Par {
                exprs: inert_exprs,
                unforgeables: par.unforgeables.clone(),
                connectives: par.connectives.clone(),
                locally_free: par.locally_free.clone(),
                connective_used: par.connective_used,
                ..Default::default()
            };
            leaves.push(TreeProc::Tail(tail));
        }

        Self::from_leaves(leaves)
    }

    /// Law 1 boundary: flatten the tree and canonicalize exactly once.
    pub fn into_sorted_par(self) -> SortedProc {
        let mut pars = Vec::new();
        collect_pars(&self, &mut pars);
        let mut merged = Par::default();
        for p in pars {
            merged = merged.par_merge(&p);
        }
        SortedProc::new(merged)
    }

    /// Returns `true` when this tree has no leaves and is not a `Par` of non-`Nil` children.
    pub fn is_trivial(&self) -> bool {
        matches!(self, TreeProc::Nil)
    }

    fn from_leaves(leaves: Vec<TreeProc>) -> Self {
        let mut leaves = leaves.into_iter().rev().collect::<Vec<_>>();
        let Some(first) = leaves.pop() else {
            return TreeProc::Nil;
        };
        let mut acc = first;
        while let Some(leaf) = leaves.pop() {
            acc = TreeProc::Par {
                left: Box::new(leaf),
                right: Box::new(acc),
            };
        }
        acc
    }
}

fn collect_pars(tree: &TreeProc, out: &mut Vec<Par>) {
    match tree {
        TreeProc::Par { left, right } => {
            collect_pars(left, out);
            collect_pars(right, out);
        }
        TreeProc::Nil => {}
        TreeProc::Send(s) => out.push(Par {
            sends: vec![s.clone()],
            ..Default::default()
        }),
        TreeProc::Receive(r) => out.push(Par {
            receives: vec![r.clone()],
            ..Default::default()
        }),
        TreeProc::New(n) => out.push(Par {
            news: vec![n.clone()],
            ..Default::default()
        }),
        TreeProc::Match(m) => out.push(Par {
            matches: vec![m.clone()],
            ..Default::default()
        }),
        TreeProc::Bundle(b) => out.push(Par {
            bundles: vec![b.clone()],
            ..Default::default()
        }),
        TreeProc::Expr(e) => out.push(Par {
            exprs: vec![e.clone()],
            ..Default::default()
        }),
        TreeProc::Tail(p) => out.push(p.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rchain_models::ast::{AlwaysEqual, Expr, Send, Var};
    use rchain_models::par_ops::is_nil;
    use rchain_models::sorter::sort_par_term;

    fn expr_int(i: i64) -> Par {
        Par {
            exprs: vec![Expr::GInt(i)],
            ..Default::default()
        }
    }

    fn send_on(chan: i64, data: i64) -> Send {
        Send {
            chan: Box::new(expr_int(chan).quote()),
            data: vec![expr_int(data).quote()],
            persistent: false,
            locally_free: AlwaysEqual(vec![]),
            connective_used: false,
        }
    }

    #[test]
    fn canonical_round_trip_preserves_sorted_par() {
        let p = Par {
            sends: vec![send_on(2, 3), send_on(1, 4)],
            exprs: vec![Expr::GInt(9), Expr::EVar(Box::new(Var::BoundVar(0)))],
            ..Default::default()
        };
        let sorted = sort_par_term(&p);
        let restored = TreeProc::from_par(&sorted).into_sorted_par();
        assert_eq!(restored.as_par(), &sorted);
    }

    #[test]
    fn canonical_round_trip_matches_sort_of_unsorted_par() {
        let p = Par {
            sends: vec![send_on(2, 3), send_on(1, 4)],
            exprs: vec![Expr::GInt(9), Expr::GInt(1)],
            receives: vec![],
            ..Default::default()
        };
        let expected = sort_par_term(&p);
        let restored = TreeProc::from_par(&p).into_sorted_par();
        assert_eq!(restored.as_par(), &expected);
    }

    #[test]
    fn swapping_par_children_canonicalizes_identically() {
        let p = Par {
            sends: vec![send_on(1, 2), send_on(3, 4)],
            ..Default::default()
        };
        let original = TreeProc::from_par(&p);
        let swapped = match original.clone() {
            TreeProc::Par { left, right } => TreeProc::Par {
                left: right,
                right: left,
            },
            other => other,
        };
        assert_eq!(
            original.into_sorted_par().as_par(),
            swapped.into_sorted_par().as_par()
        );
    }

    #[test]
    fn nil_absorbs_under_canonicalization() {
        let p = expr_int(7);
        let with_nil = TreeProc::Par {
            left: Box::new(TreeProc::Nil),
            right: Box::new(TreeProc::from_par(&p)),
        };
        assert_eq!(with_nil.into_sorted_par().as_par(), &sort_par_term(&p));
    }

    #[test]
    fn empty_par_is_nil_and_trivial() {
        let p = Par::default();
        assert!(is_nil(&p));
        let tree = TreeProc::from_par(&p);
        assert!(tree.is_trivial());
        assert!(tree.into_sorted_par().as_par().sends.is_empty());
    }

    #[test]
    fn inert_tail_is_retained_through_canonicalization() {
        let p = Par {
            exprs: vec![Expr::GInt(1), Expr::GInt(2)],
            unforgeables: vec![],
            ..Default::default()
        };
        let expected = sort_par_term(&p);
        let restored = TreeProc::from_par(&p).into_sorted_par();
        assert_eq!(restored.as_par(), &expected);
    }
}
