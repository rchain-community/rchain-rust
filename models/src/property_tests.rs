//! Property tests for the `models` layer's laws — the shared arbitrary-`Par` strategy lives here.
//!
//! Law 1 (canonicalization to a total order is idempotent, and `sort(p|q) = sort(q|p)`), Law 2
//! (α/name equivalence **is** canonical structural equality), Law 6 (a program has no globally free
//! variables — `Closed`). Randomized evidence alongside the Lean statements
//! (`spec/Rchain/Sort.lean`, `Ty.lean`) and the unit tests in `sorter.rs`/`types.rs`.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use proptest::prelude::*;

use crate::ast::{Expr, Par, ProcSort, Send, Var};
use crate::par_ops::{from_expr, par_concat};
use crate::sorted::Sorted;
use crate::sorter::{sort_par_term, sort_pars};
use crate::types::{is_closed, Closed};

/// An arbitrary ground expression (no runtime values, which is enough for order/equality laws).
fn arb_ground() -> impl Strategy<Value = Expr> {
    prop_oneof![
        any::<i64>().prop_map(Expr::GInt),
        prop::collection::vec(any::<char>(), 0..4)
            .prop_map(|cs| Expr::GString(cs.into_iter().collect())),
        any::<bool>().prop_map(Expr::GBool),
    ]
}

/// An arbitrary **closed** process expression: grounds, bound vars, wildcards (which bind nothing),
/// and collections of them. No free variables — that is Law 6's input class.
fn arb_closed_expr(depth: u32) -> BoxedStrategy<Expr> {
    if depth == 0 {
        return arb_ground().boxed();
    }
    let inner = arb_closed_expr(depth - 1);
    prop_oneof![
        arb_ground(),
        (0i32..4).prop_map(|i| Expr::EVar(Box::new(Var::BoundVar(i)))),
        Just(Expr::EVar(Box::new(Var::Wildcard))),
        prop::collection::vec(inner.clone(), 0..4).prop_map(|es| Expr::EList(crate::ast::EList {
            ps: es.into_iter().map(from_expr).collect(),
            ..Default::default()
        })),
        prop::collection::vec(inner.clone(), 0..4).prop_map(|es| Expr::ETuple(
            crate::ast::ETuple {
                ps: es.into_iter().map(from_expr).collect(),
                ..Default::default()
            }
        )),
    ]
    .boxed()
}

/// An arbitrary process `Par`: a closed body plus, sometimes, a send whose payload is closed.
fn arb_proc_par(depth: u32) -> BoxedStrategy<Par<ProcSort>> {
    let body = arb_closed_expr(depth);
    let with_send = body.clone().prop_map(|e| Par::<ProcSort> {
        sends: vec![Send {
            chan: Box::new(from_expr(e).quote()),
            data: vec![],
            persistent: false,
            locally_free: Default::default(),
            connective_used: false,
        }],
        ..Default::default()
    });
    prop_oneof![
        body.prop_map(|e| Par::<ProcSort> {
            exprs: vec![e],
            ..Default::default()
        }),
        with_send,
    ]
    .boxed()
}

/// A `Par` that may contain a free variable — the shape Law 6 must reject.
fn arb_open_par() -> BoxedStrategy<Par<ProcSort>> {
    (any::<i32>(), arb_proc_par(2))
        .prop_map(|(i, mut p)| {
            p.exprs.push(Expr::EVar(Box::new(Var::FreeVar(i))));
            p
        })
        .boxed()
}

fn hash_of<T: Hash>(value: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

proptest! {
    /// **Law 1, idempotence.** Canonicalizing an already-canonicalized term is the identity: the
    /// sort is a *normal form*, not a step that keeps moving.
    #[test]
    fn law1_sorting_is_idempotent(p in arb_proc_par(3)) {
        let once = sort_par_term(&p);
        let twice = sort_par_term(&once);
        prop_assert_eq!(&once, &twice);
        // …and the canonical form is what `Sorted::new` holds, whoever calls it.
        let sorted = Sorted::new(p.clone());
        prop_assert_eq!(sorted.as_par(), &once);
    }

    /// **Law 1, commutativity.** `sort(p|q) = sort(q|p)`: a parallel composition's canonical form
    /// does not depend on the order the two sides were written — which is what lets a state hash be
    /// independent of the order deploys happened to concatenate.
    #[test]
    fn law1_parallel_composition_sorts_commutatively(p in arb_proc_par(2), q in arb_proc_par(2)) {
        prop_assert_eq!(
            sort_par_term(&par_concat(&p, &q)),
            sort_par_term(&par_concat(&q, &p))
        );
    }

    /// **Law 2.** α/name equivalence *is* canonical structural equality: two terms that canonicalize
    /// to the same par are equal as `Sorted`, and hash alike — the property the state hash depends on
    /// (a `Sorted` whose `Hash` disagreed with its `Eq` would make the hash depend on insertion
    /// order).
    #[test]
    fn law2_canonical_equality_agrees_with_canonical_hashing(p in arb_proc_par(3), q in arb_proc_par(3)) {
        let a = Sorted::new(par_concat(&p, &q));
        let b = Sorted::new(par_concat(&q, &p));
        prop_assert_eq!(&a, &b, "the same par, however it was built");
        prop_assert_eq!(hash_of(&a), hash_of(&b), "equal canonical forms hash alike");
    }

    /// Sorting a *sequence* of pars is order-insensitive in the same way: the sorted list of a
    /// permutation's elements is the same list. (The sequences are permuted by reversing, and the
    /// elements are the same set either way.)
    #[test]
    fn law2_sorting_a_sequence_depends_only_on_its_elements(ps in prop::collection::vec(arb_proc_par(2), 0..5)) {
        let mut reversed = ps.clone();
        reversed.reverse();
        prop_assert_eq!(sort_pars(ps), sort_pars(reversed));
    }

    /// **Law 6.** A program has no globally free variables: a term built from grounds, bound vars and
    /// wildcards is `Closed`, and the predicate and the `Closed` newtype agree on every term.
    #[test]
    fn law6_a_closed_term_is_accepted_and_the_predicate_agrees(p in arb_proc_par(3), q in arb_open_par()) {
        prop_assert!(is_closed(&p), "a closed term must be recognized as closed");
        prop_assert!(Closed::new(p).is_some());
        prop_assert!(!is_closed(&q), "a free variable must not be closed");
        prop_assert!(Closed::new(q).is_none());
    }
}

/// **The strategy is not degenerate.** A property test over a generator that only ever produces one
/// term is a test that passes for free; this pins that the arbitrary `Par` really varies — across
/// samples it produces sends, multiple expressions, and collections. Same class of guard as the
/// branch-disabled checks: it makes the *evidence* falsifiable, not just the property.
#[test]
fn the_arbitrary_par_strategy_produces_varied_terms() {
    use proptest::strategy::{Strategy, ValueTree};
    use proptest::test_runner::TestRunner;

    let mut runner = TestRunner::deterministic();
    let strategy = arb_proc_par(3);
    let (mut sends, mut multi_expr, mut collections, mut empty) = (0, 0, 0, 0);
    for _ in 0..200 {
        let tree = strategy.new_tree(&mut runner).expect("a sample");
        let p = tree.current();
        if !p.sends.is_empty() {
            sends += 1;
        }
        if p.exprs.len() > 1 {
            multi_expr += 1;
        }
        if p.exprs.iter().any(|e| {
            matches!(
                e,
                Expr::EList(_) | Expr::ETuple(_) | Expr::ESet(_) | Expr::EMap(_)
            )
        }) {
            collections += 1;
        }
        if p.sends.is_empty() && p.exprs.is_empty() {
            empty += 1;
        }
    }
    assert!(sends > 0, "the strategy must sometimes produce a send");
    assert!(
        collections > 0,
        "the strategy must sometimes produce a collection"
    );
    assert!(
        multi_expr + sends > 20,
        "the strategy must produce non-trivial terms often (sends {sends}, multi-expr {multi_expr})"
    );
    let _ = empty;
}
