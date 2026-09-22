//! Coverage for the `if`-in-a-`Par` fix (AUDIT C21, issue #59) that does not overlap
//! `rholang/tests/system_process_conformance.rs::an_if_fires_the_same_way_wherever_it_sits_in_a_par`.
//!
//! That test pins the behavioural cases — an `if` after a send at the top level, an `if` after a send
//! inside a receive body, an `if` in first position, and the `else` side of a false condition. What is
//! left here is the structural and scheduler half:
//!
//! * the fault was in *normalization*, so the result must not depend on the runtime's effect mode;
//! * the accumulated `Par` is not only a leading `Send` — an `if` after a `new` comes through the
//!   `news` slot, a different arm of `resolve_children`;
//! * `if` without `else` desugars against `Nil` rather than a second branch;
//! * the explicit `match` an `if`/`else` becomes was already correct, and this pins that it stays that
//!   way, so a future change to `normalize_if` cannot quietly regress its sibling.

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

/// The integers sitting on `out`, sorted — `Par` is `|`, not sequencing, so these assertions are about
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

/// The absorption happened while *normalizing*, before any effect was scheduled, so every effect mode
/// must agree. Each of these is a different carrier for the same sequential fold, and the fix must not
/// be specific to the one CI happens to run.
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

/// What the condition must not absorb is the whole accumulated `Par`, not merely a leading send: a
/// `new` reaches the target through the `news` slot, a different arm of `resolve_children`.
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

/// `if` without `else` desugars against `Nil` instead of a second branch — the same target bug applied,
/// and the branch is dropped the same silent way.
#[tokio::test]
async fn if_without_else_not_first_in_a_par_still_runs() {
    let (rt, _) = build_runtime_pair().await;
    assert_eq!(
        run(&rt, r#"@"out"!(1) | if (true) { @"out"!(2) }"#).await,
        vec![1, 2]
    );
}

/// The explicit `match` this desugars to was already correct; pin it so `normalize_if` and
/// `normalize_match` cannot drift apart again.
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

/// An `if` inside a **receive body**, which is `Group.rho:49`'s shape: there the `if` at `:50` carries
/// the contract's entire creation chain in its `else`, so an absorbed `else` is a silent stall rather
/// than a missing value. The carriers above place the `if` in a `Par`; this one is the arm where a
/// `for` body hands it a fresh continuation. (AUDIT C25's investigation ruled this shape out first —
/// pinning it here is what keeps that ruling true.)
///
/// Both binds are covered because the store's is the linear one: `<<-` does not consume, `<-` does.
#[tokio::test]
async fn if_inside_a_receive_body_runs_its_else_branch() {
    let (rt, _) = build_runtime_pair().await;
    assert_eq!(
        run(
            &rt,
            r#"new box in {
                 box!({}) |
                 for (@m <<- box) {
                   if (m.get("absent") != Nil) { @"out"!(1) } else { @"out"!(2) }
                 }
               }"#
        )
        .await,
        vec![2],
        "a peek's body is a statement position"
    );

    let (rt, _) = build_runtime_pair().await;
    assert_eq!(
        run(
            &rt,
            r#"new a, box in {
                 a!(1) | a!(2) | box!({}) |
                 for (@m <- box) {
                   if (m.get("absent") != Nil) { @"out"!(1) } else { @"out"!(2) }
                 }
               }"#
        )
        .await,
        vec![2],
        "a linear consume's body is a statement position too, with sends ahead of it in the block"
    );
}
