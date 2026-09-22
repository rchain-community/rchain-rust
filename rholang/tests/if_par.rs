//! Regression tests for issue #59.
//!
//! `SomeProcess | if (cond) {A} else {B}` used to drop the `if` outright whenever it was not the
//! first operand of the `|`: the normalizer built the desugared `Match`'s target by normalizing the
//! condition *into* the surrounding `Par`, so whatever the `Par` had already accumulated became part
//! of the target, nothing matched, and both branches were silently discarded — no error, and
//! `evaluate(..).succeeded()` returned `true`.
//!
//! `Par` is `|`, not sequencing, so operand order must not decide whether a branch runs. These tests
//! pin that for `if`/`if`-`else` in any position, and for the explicit `match` it desugars to.

mod common;

use common::{build_runtime_pair, build_runtime_with_mode};
use rchain_crypto::hash::blake2b512_random::Blake2b512Random;
use rchain_models::ast::{Expr, Par};
use rchain_models::par_ops::from_expr;
use rchain_models::sorted::SortedProc;
use rchain_rholang::runtime::RhoRuntime;
use rchain_rholang::scheduler::EffectMode;

fn rand() -> Blake2b512Random {
    Blake2b512Random::from_init(&[0u8; 32])
}

fn chan(name: &str) -> SortedProc {
    SortedProc::new(from_expr(Expr::GString(name.to_string())))
}

/// The integers sitting on `out`, sorted — `Par` is not sequencing, so the assertions here are about
/// *which* values arrived, never the order they arrived in.
fn ints(data: &[Par]) -> Vec<i64> {
    let mut out: Vec<i64> = data
        .iter()
        .filter_map(|p| match p.exprs.first() {
            Some(Expr::GInt(i)) => Some(*i),
            _ => None,
        })
        .collect();
    out.sort_unstable();
    out
}

async fn run(rt: &RhoRuntime, term: &str) -> Vec<i64> {
    let res = rt.evaluate(term, &rand()).await.expect("evaluate");
    assert!(
        res.succeeded(),
        "unexpected errors for {term}: {:?}",
        res.errors
    );
    ints(&rt.get_data_par(&chan("out")).await.expect("get_data_par"))
}

/// The reported repro: the send comes first, the `if` second.
#[tokio::test]
async fn if_not_first_in_a_par_still_runs() {
    let (rt, _) = build_runtime_pair().await;
    assert_eq!(
        run(
            &rt,
            r#"@"out"!(1) | if (true) { @"out"!(2) } else { @"out"!(3) }"#
        )
        .await,
        vec![1, 2]
    );
}

/// The control from the report: identical term with the operands swapped.
#[tokio::test]
async fn if_first_in_a_par_still_runs() {
    let (rt, _) = build_runtime_pair().await;
    assert_eq!(
        run(
            &rt,
            r#"if (true) { @"out"!(2) } else { @"out"!(3) } | @"out"!(1)"#
        )
        .await,
        vec![1, 2]
    );
}

/// The `else` arm is reached from a non-first position too.
#[tokio::test]
async fn else_branch_of_a_non_first_if_runs() {
    let (rt, _) = build_runtime_pair().await;
    assert_eq!(
        run(
            &rt,
            r#"@"out"!(1) | if (false) { @"out"!(2) } else { @"out"!(3) }"#
        )
        .await,
        vec![1, 3]
    );
}

/// `if` without `else` (desugared against `Nil`) behaves the same way.
#[tokio::test]
async fn if_without_else_not_first_in_a_par_still_runs() {
    let (rt, _) = build_runtime_pair().await;
    assert_eq!(
        run(&rt, r#"@"out"!(1) | if (true) { @"out"!(2) }"#).await,
        vec![1, 2]
    );
}

/// What accumulates is whatever has already been normalized, not only a leading send — an `if` after
/// a scoped term must survive too.
#[tokio::test]
async fn if_after_a_new_in_a_par_still_runs() {
    let (rt, _) = build_runtime_pair().await;
    assert_eq!(
        run(
            &rt,
            r#"new c in { Nil } | @"out"!(1) | if (true) { @"out"!(2) } else { @"out"!(3) }"#
        )
        .await,
        vec![1, 2]
    );
}

/// The explicit `match` that `if`/`else` desugars to was already correct; pin that it stays correct
/// from a non-first position.
#[tokio::test]
async fn explicit_match_not_first_in_a_par_still_runs() {
    let (rt, _) = build_runtime_pair().await;
    assert_eq!(
        run(
            &rt,
            r#"@"out"!(1) | match true { true => { @"out"!(2) } false => { @"out"!(3) } }"#
        )
        .await,
        vec![1, 2]
    );
}

/// The fault was in normalization, not in the effect scheduler — so the result must not depend on the
/// runtime's effect mode. Pin that across the modes the runtime exposes.
#[tokio::test]
async fn if_not_first_in_a_par_holds_across_effect_modes() {
    for (concurrent, mode) in [
        (false, EffectMode::Sequential),
        (true, EffectMode::ForkJoin),
        (true, EffectMode::Gate),
        (false, EffectMode::Relaxed),
    ] {
        let rt = build_runtime_with_mode(concurrent, mode).await;
        assert_eq!(
            run(
                &rt,
                r#"@"out"!(1) | if (true) { @"out"!(2) } else { @"out"!(3) }"#
            )
            .await,
            vec![1, 2],
            "if-not-first must run with concurrent={concurrent} and effects={mode:?}"
        );
    }
}
