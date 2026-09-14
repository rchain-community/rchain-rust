//! Property tests for the rholang layer's laws.
//!
//! Law 3 (capture-avoiding de Bruijn substitution, total on `Closed`, and `sort(subst t) =
//! subst(sort t)`) and Law 5 (spatial matching binds a free variable **at most once**). Randomized
//! evidence alongside the Lean statements (`Rchain/Subst.lean`, `Match.lean`) and the unit tests in
//! `substitute.rs`/`matcher/`.

use proptest::prelude::*;
use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestCaseError;

use rchain_models::ast::{AlwaysEqual, EList, ETuple, Expr, Par, ProcSort, Send, Var};
use rchain_models::par_ops::{from_expr, par_concat};
use rchain_models::sorted::SortedProc;
use rchain_models::sorter::sort_par_term;
use rchain_models::types::is_closed;
use std::sync::Arc;

use crate::env::Env;
use crate::matcher::{spatial_match, FreeMap};
use crate::scheduler::EffectMode;

/// An arbitrary **closed** process: grounds, bound vars, wildcards, and collections of them.
fn arb_closed(depth: u32) -> BoxedStrategy<Par<ProcSort>> {
    if depth == 0 {
        return any::<i64>().prop_map(|i| from_expr(Expr::GInt(i))).boxed();
    }
    let inner = arb_closed(depth - 1);
    let expr = prop_oneof![
        any::<i64>().prop_map(Expr::GInt),
        prop::collection::vec(any::<char>(), 0..3)
            .prop_map(|cs| Expr::GString(cs.into_iter().collect())),
        (0i32..3).prop_map(|i| Expr::EVar(Box::new(Var::BoundVar(i)))),
        prop::collection::vec(inner.clone(), 0..3).prop_map(|ps| Expr::EList(EList {
            ps,
            ..Default::default()
        })),
        prop::collection::vec(inner.clone(), 0..3).prop_map(|ps| Expr::ETuple(ETuple {
            ps,
            ..Default::default()
        })),
    ];
    let with_send = (inner.clone(), inner.clone()).prop_map(|(chan, payload)| Par::<ProcSort> {
        sends: vec![Send {
            chan: Box::new(chan.quote()),
            data: vec![payload.quote()],
            persistent: false,
            locally_free: AlwaysEqual(vec![]),
            connective_used: false,
        }],
        ..Default::default()
    });
    prop_oneof![
        expr.prop_map(|e| from_expr(e)),
        with_send,
        (inner.clone(), inner.clone()).prop_map(|(a, b)| par_concat(&a, &b)),
    ]
    .boxed()
}

/// An arbitrary **ground** par: grounds, collections of grounds, and sends of grounds — **no
/// `EVar` at all**.
///
/// This is the class a *datum* may come from. A par holding a `BoundVar` is `is_closed` by the
/// free-variable rule (a bound var is not free) but is not a valid datum: the reference points at a
/// binder outside the term, so the matcher refuses to bind it to a pattern variable. The first
/// version of the property below used `arb_closed`, and proptest's counterexample was exactly such a
/// dangling `BoundVar` — the generator, not the matcher, was wrong.
fn arb_ground_par(depth: u32) -> BoxedStrategy<Par<ProcSort>> {
    if depth == 0 {
        return prop_oneof![
            any::<i64>().prop_map(|i| from_expr(Expr::GInt(i))),
            prop::collection::vec(any::<char>(), 0..3)
                .prop_map(|cs| from_expr(Expr::GString(cs.into_iter().collect()))),
        ]
        .boxed();
    }
    let inner = arb_ground_par(depth - 1);
    prop_oneof![
        any::<i64>().prop_map(|i| from_expr(Expr::GInt(i))),
        prop::collection::vec(inner.clone(), 0..3).prop_map(|ps| from_expr(Expr::EList(EList {
            ps,
            ..Default::default()
        }))),
        prop::collection::vec(inner.clone(), 0..3).prop_map(|ps| from_expr(Expr::ETuple(ETuple {
            ps,
            ..Default::default()
        }))),
        (inner.clone(), inner.clone()).prop_map(|(chan, payload)| Par::<ProcSort> {
            sends: vec![Send {
                chan: Box::new(chan.quote()),
                data: vec![payload.quote()],
                persistent: false,
                locally_free: AlwaysEqual(vec![]),
                connective_used: false,
            }],
            ..Default::default()
        }),
    ]
    .boxed()
}

/// One variable in *pattern* position — the `@x` of `for (@[x, y] <- c)`, which the normalizer
/// builds as a par holding the free var with **`connective_used` set at every level**. That flag is
/// what makes the matcher match rather than compare for equality (`spatial_match`'s first branch,
/// faithful to Scala's `matchPar`), so a hand-built pattern that leaves it false silently becomes an
/// equality test — which is how the first version of these properties quietly failed.
fn var_pattern(level: i32) -> Par<ProcSort> {
    let mut p = from_expr(Expr::EVar(Box::new(Var::FreeVar(level))));
    p.connective_used = true;
    p
}

/// A pattern `[x, y]` over the two given levels, with both the outer par and the elements marked
/// connective.
fn list_pattern(levels: (i32, i32)) -> Par<ProcSort> {
    let mut p = from_expr(Expr::EList(EList {
        ps: vec![var_pattern(levels.0), var_pattern(levels.1)],
        ..Default::default()
    }));
    p.connective_used = true;
    p
}

/// The same collection with *distinct* free variables — the case that must match.
fn distinct_var_pattern(level: i32) -> impl Strategy<Value = Par<ProcSort>> {
    Just(level).prop_map(move |level| list_pattern((level, level + 1)))
}

/// The same collection with the *same* free variable twice — the case that must not match.
fn arb_repeated_var_pattern(level: i32) -> impl Strategy<Value = Par<ProcSort>> {
    Just(level).prop_map(move |level| list_pattern((level, level)))
}

proptest! {
    /// **Law 3, the refinement half.** Substituting a closed value into a closed term yields a closed
    /// term: the substitution is total on `Closed`, so the interpreter never has to invent a value
    /// for a variable that escaped a binder.
    #[test]
    fn law3_substituting_a_closed_value_keeps_the_term_closed(p in arb_closed(3), v in arb_closed(2)) {
        prop_assert!(is_closed(&p));
        let env = Env::make_env([v]);
        let out = crate::substitute::substitute_par(&p, 0, &env).expect("substitution");
        prop_assert!(is_closed(&out), "substitution must not free a variable");
    }

    /// **Law 3, the commutative square.** `sort(subst t) = subst(sort t)`: canonicalizing before or
    /// after substitution gives the same term, which is what lets the sorter and the substituter be
    /// applied in either order on the way to a state hash.
    #[test]
    fn law3_substitution_and_sorting_commute(p in arb_closed(3), v in arb_closed(2)) {
        let env = Env::make_env([v]);
        let sorted_first = crate::substitute::substitute_par(&sort_par_term(&p), 0, &env)
            .expect("substitution");
        let substituted_first =
            sort_par_term(&crate::substitute::substitute_par(&p, 0, &env).expect("substitution"));
        prop_assert_eq!(sorted_first, substituted_first);
    }

    /// **Law 5.** A free variable is bound **at most once**: a pattern that mentions the same
    /// variable twice never matches, even when the datum would make the two bindings *consistent*
    /// (`[x, x]` against `[1, 1]`). Without the rule that test would match and a pattern could pin
    /// two positions together, which the language does not allow.
    #[test]
    fn law5_a_pattern_that_binds_a_variable_twice_never_matches(pattern in arb_repeated_var_pattern(0)) {
        // The discriminating datum: both positions equal, so the naive binding is consistent.
        let data = from_expr(Expr::EList(EList {
            ps: vec![from_expr(Expr::GInt(1)), from_expr(Expr::GInt(1))],
            ..Default::default()
        }));
        let matches = spatial_match(&sort_par_term(&data), &sort_par_term(&pattern), &FreeMap::new())
            .expect("no error");
        prop_assert!(matches.is_empty(), "a doubly-bound pattern must not match: {matches:?}");
    }

    /// The other side of the same rule, through the **receiver's own entry point**: a `BindPattern`
    /// holding one bare variable matches any datum, and the value it binds is closed when the datum
    /// is (`RhoMatch::get` is what the store calls; `spatial_match` on a bare-var *par* is a lower
    /// layer whose contract is the element list, not the whole term).
    #[test]
    fn law5_a_variable_bind_pattern_matches_any_datum(target in arb_ground_par(2)) {
        use rchain_crypto::hash::blake2b512_random::Blake2b512Random;
        use rchain_rspace::match_::Match;
        use rchain_models::runtime::{BindPattern, ListParWithRandom};

        use crate::storage::RhoMatch;

        // Built exactly as the `rho_match_binds_free_vars` unit test builds it — a par holding the
        // free var with `connective_used`, and *no* `locally_free` bookkeeping (`from_expr`'s
        // bookkeeping is what the sorter expects, not what the matcher does).
        let pattern = BindPattern {
            patterns: vec![SortedProc::new(Par {
                exprs: vec![Expr::EVar(Box::new(Var::FreeVar(0)))],
                connective_used: true,
                ..Default::default()
            })],
            remainder: None,
            free_count: 1,
        };
        let data = ListParWithRandom {
            pars: vec![SortedProc::new(target.clone())],
            random_state: Blake2b512Random::from_init(&[0u8; 32]),
        };

        let matched = RhoMatch
            .get(&pattern, &data)
            .unwrap_or_else(|| panic!("a bare variable must match any datum, but did not match {target:?}"));
        for par in &matched.pars {
            prop_assert!(
                is_closed(par.as_par()),
                "the binding must be closed when the datum is: {:?}",
                par.as_par()
            );
        }
    }

    /// A ground pattern matches only an equal datum (`spatial_match` with no connectives is
    /// equality), and a *non-ground* closed pattern does not match an unrelated term — the matcher
    /// does not fall back to "anything goes".
    #[test]
    fn law5_a_ground_pattern_matches_only_itself(i in any::<i64>(), j in any::<i64>()) {
        let pattern = from_expr(Expr::GInt(i));
        let target = from_expr(Expr::GInt(j));
        let matches = spatial_match(&target, &pattern, &FreeMap::new()).expect("no error");
        prop_assert_eq!(!matches.is_empty(), i == j);
    }
}

/// **A wildcard (or free variable) in *term* position is an illegal substitution**, not a silent
/// pass-through: `substitute_par` returns `SubstituteError`, which is what Scala's
/// `maybeSubstitute(Var)` does (`SubstituteError(s"Illegal Substitution [$term]")` for anything that
/// is not a `BoundVar`). The distinction matters: a wildcard is a *pattern* construct (a receive
/// pattern keeps one, and substitution leaves it alone), while a wildcard standing as a term has no
/// value to substitute and no meaning to preserve.
#[test]
fn substituting_a_term_level_wildcard_is_an_error() {
    let term = from_expr(Expr::EVar(Box::new(Var::Wildcard)));
    let env = Env::make_env([from_expr(Expr::GInt(1))]);
    assert!(
        crate::substitute::substitute_par(&term, 0, &env).is_err(),
        "a term-level wildcard has no substitution"
    );
}

// --- Laws 21/25: the schedulers refine the sequential reference -------------------------------

/// A small, always-valid rholang program built from a template: a fresh channel, a few sends on it,
/// a receive that consumes them, and an optional second channel with a matching pair. Generated as
/// *source* because the schedulers are driven through `evaluate`.
fn arb_program() -> impl Strategy<Value = String> {
    (
        prop::collection::vec(any::<i64>(), 1..4),
        any::<bool>(),
        any::<bool>(),
    )
        .prop_map(|(values, persist, nested)| {
            let sends: String = values
                .iter()
                .map(|v| {
                    if persist {
                        format!(r#"c!!({v}) | "#)
                    } else {
                        format!(r#"c!({v}) | "#)
                    }
                })
                .collect();
            if nested {
                format!(
                    // The receive's body produces on a second channel: the shape that gives the
                    // scheduler a *chain* of effects (a receive whose continuation sends) rather than
                    // one flat round of COMMs. The payload is a literal — sending the bound variable
                    // itself would need quoting (`@x` binds a name in this position), which is a
                    // rholang-context subtlety the property is not about.
                    r#"new c, d in {{ {sends}for (@x <- c) {{ d!(0) }} | for (@y <- d) {{ Nil }} }}"#
                )
            } else {
                format!(r#"new c in {{ {sends}for (@x <- c) {{ Nil }} }}"#)
            }
        })
}

/// Build a runtime in the given effect mode, over a fresh store.
async fn runtime_in_mode(mode: EffectMode) -> crate::runtime::RhoRuntime {
    use rchain_models::runtime::{BindPattern, ListParWithRandom, TaggedContinuation};
    use rchain_models::sorted::SortedProc;
    use rchain_rspace::factory::create_history_repository;
    use rchain_rspace::hot_store::InMemHotStore;
    use rchain_rspace::rspace::RSpace;
    use rchain_shared::store_manager::InMemoryStoreManager;

    let manager = InMemoryStoreManager::default();
    let history = create_history_repository::<
        SortedProc,
        BindPattern,
        ListParWithRandom,
        TaggedContinuation,
    >(&manager, "scheduler-property")
    .await
    .expect("history repository");
    let reader = history.get_history_reader(history.root()).await;
    let hot = Arc::new(InMemHotStore::new(reader.base()));
    let (play, _replay) =
        RSpace::create_with_replay(history.clone(), hot, Arc::new(crate::storage::RhoMatch));
    crate::runtime::RhoRuntime::create_with_effect_mode(
        play,
        history,
        SortedProc::default(),
        true,
        mode,
    )
    .await
    .expect("rho runtime")
}

/// One randomized comparison of a scheduler mode against the sequential reference. Runs on its own
/// runtime and returns `(state hash, event-log length)` for each side.
fn compare_mode(program: &str, mode: EffectMode, cases: u32) -> Result<(), TestCaseError> {
    use rchain_crypto::hash::blake2b512_random::Blake2b512Random;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    rt.block_on(async {
        for _ in 0..cases {
            let rand = Blake2b512Random::from_init(&[0u8; 32]);
            let reference = runtime_in_mode(EffectMode::Sequential).await;
            let candidate = runtime_in_mode(mode).await;

            let rr = reference.evaluate(program, &rand).await.expect("reference");
            let cr = candidate.evaluate(program, &rand).await.expect("candidate");
            prop_assert!(rr.succeeded() || !rr.errors.is_empty());
            prop_assert!(
                cr.succeeded() || !cr.errors.is_empty(),
                "the mode must not fail differently: {:?} vs {:?}",
                cr.errors,
                rr.errors
            );

            let rc = reference.create_checkpoint().await.expect("checkpoint");
            let cc = candidate.create_checkpoint().await.expect("checkpoint");
            let (root, log_len) = (rc.root, rc.log.len());
            prop_assert_eq!(
                cc.root,
                root,
                "a scheduler mode must reach the sequential state for {}",
                program
            );
            prop_assert_eq!(
                cc.log.len(),
                log_len,
                "…and emit the same number of events for {}",
                program
            );
        }
        Ok(())
    })
}

/// **Law 21 (gate refines sequential).** The gate scheduler runs the effect at DFS path `i` only
/// after `0..i−1` complete, and must therefore reach the sequential reducer's state and event log —
/// on a *randomized* program, not only on the fixed corpus the integration test uses.
#[test]
fn law21_the_gate_scheduler_refines_the_sequential_reference() {
    let mut runner = proptest::test_runner::TestRunner::deterministic();
    let strategy = arb_program();
    for _ in 0..16 {
        // A low case count: each case builds two runtimes and evaluates twice.
        let program = strategy.new_tree(&mut runner).expect("a sample").current();
        compare_mode(&program, EffectMode::Gate, 1).expect("the gate mode refines sequential");
    }
}

/// **Law 25 (validated speculation refines sequential).** The validated-relaxed mode may commit in
/// any order *iff* each commit validates Law 24; on the block path it must publish exactly the
/// sequential fold's state. Same randomized comparison, one mode over.
#[test]
fn law25_the_validated_relaxed_scheduler_refines_sequential() {
    let mut runner = proptest::test_runner::TestRunner::deterministic();
    let strategy = arb_program();
    for _ in 0..16 {
        let program = strategy.new_tree(&mut runner).expect("a sample").current();
        compare_mode(&program, EffectMode::RelaxedValidated, 1)
            .expect("the validated-relaxed mode refines sequential");
    }
}

/// **Law 4 (COMM determinism).** Reducing the same program twice — fresh runtimes, same seed — must
/// reach the same state hash *and* the same event log. A COMM whose outcome depended on a hash map's
/// iteration order, a wall clock, or the effect interleaving would break this, and the state hash is
/// what consensus compares.
#[test]
fn law4_the_same_program_reduces_to_the_same_state() {
    let mut runner = proptest::test_runner::TestRunner::deterministic();
    let strategy = arb_program();
    for _ in 0..16 {
        let program = strategy.new_tree(&mut runner).expect("a sample").current();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime");
        rt.block_on(async {
            let (first, second) = (
                runtime_in_mode(EffectMode::Sequential).await,
                runtime_in_mode(EffectMode::Sequential).await,
            );
            let rand =
                rchain_crypto::hash::blake2b512_random::Blake2b512Random::from_init(&[0u8; 32]);
            let r1 = first.evaluate(&program, &rand).await.expect("first");
            let r2 = second.evaluate(&program, &rand).await.expect("second");
            assert_eq!(
                r1.errors.len(),
                r2.errors.len(),
                "same errors for {program}"
            );

            let c1 = first.create_checkpoint().await.expect("checkpoint");
            let c2 = second.create_checkpoint().await.expect("checkpoint");
            assert_eq!(
                c1.root, c2.root,
                "the same program must reduce to the same state: {program}"
            );
            assert_eq!(
                c1.log.len(),
                c2.log.len(),
                "…and the same events: {program}"
            );
        });
    }
}
