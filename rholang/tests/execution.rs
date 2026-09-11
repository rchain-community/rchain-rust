//! End-to-end execution-pipeline integration tests (parse → normalize → reduce → rspace → hash).
//!
//! These assemble a real `RhoRuntime` + `ReplayRhoRuntime` over an in-memory store and drive a
//! deploy through the full stack, pinning post-state hashes against committed golden vectors and
//! asserting the Scala-independent `replay == play` determinism invariant (Law 11).

mod common;

use rchain_crypto::hash::blake2b256_hash::Blake2b256Hash;
use rchain_crypto::hash::blake2b512_random::Blake2b512Random;
use rchain_models::ast::Expr;
use rchain_models::par_ops::from_expr;
use rchain_models::sorted::SortedProc;
use rchain_models::types::Closed;
use rchain_rholang::accounting::Cost;
use rchain_rholang::env::Env;
use rchain_rholang::registry::registry_bootstrap_ast;
use rchain_rholang::scheduler::EffectMode;
use rchain_rspace::history::history::empty_root_hash_value;
use rchain_rspace::trace::event::Event;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use common::{build_runtime, build_runtime_pair, build_runtime_with_mode, load_golden};

/// A fixed, deterministic random seed so post-state hashes are reproducible.
fn fixed_rand() -> Blake2b512Random {
    Blake2b512Random::from_init(&[0u8; 32])
}

fn chan(name: &str) -> SortedProc {
    SortedProc::new(from_expr(Expr::GString(name.to_string())))
}

/// Assert `hash` matches the committed golden vector for `case`.
fn assert_state_hash(case: &str, hash: &[u8]) {
    let want =
        load_golden(case, "execution").unwrap_or_else(|| panic!("missing golden case {case}"));
    assert_eq!(
        rchain_shared::base16::encode(hash),
        want,
        "golden mismatch for {case}"
    );
}

#[tokio::test]
async fn execute_deploy_produces_datum() {
    let (rt, _replay) = build_runtime_pair().await;
    let res = rt
        .evaluate(r#"@"chan"!(42)"#, &fixed_rand())
        .await
        .expect("evaluate");
    assert!(res.succeeded(), "unexpected errors: {:?}", res.errors);
    let data = rt.get_data_par(&chan("chan")).await.expect("get_data_par");
    assert_eq!(data, vec![from_expr(Expr::GInt(42))]);
}

#[tokio::test]
async fn execute_deploy_state_hash_is_deterministic() {
    let (a, _) = build_runtime_pair().await;
    let (b, _) = build_runtime_pair().await;
    a.evaluate(r#"@"chan"!(42)"#, &fixed_rand()).await.unwrap();
    b.evaluate(r#"@"chan"!(42)"#, &fixed_rand()).await.unwrap();
    let ha = a.create_checkpoint().await.unwrap().root;
    let hb = b.create_checkpoint().await.unwrap().root;
    assert_eq!(ha, hb);
    assert_state_hash("exec_deploy_42", ha.as_bytes());
}

#[tokio::test]
async fn replay_matches_play() {
    let (rt, rrt) = build_runtime_pair().await;
    let rand = fixed_rand();
    rt.evaluate(r#"@"chan"!(42)"#, &rand).await.unwrap();
    let cp = rt.create_checkpoint().await.unwrap();

    // Replay from the empty pre-state against the recorded play log.
    rrt.reset(empty_root_hash_value()).await.unwrap();
    rrt.rig(cp.log.clone()).await;
    rrt.evaluate(r#"@"chan"!(42)"#, &rand).await.unwrap();
    rrt.check_replay_data()
        .await
        .expect("replay data consistent");

    let replay_cp = rrt.create_checkpoint().await.unwrap();
    assert_eq!(cp.root, replay_cp.root);
    assert_state_hash("replay_deploy_42", replay_cp.root.as_bytes());
}

#[tokio::test]
async fn empty_state_bootstrap_is_deterministic() {
    let (rt, _replay) = build_runtime_pair().await;
    rt.reset(empty_root_hash_value()).await.unwrap();
    let bootstrap = Closed::new(registry_bootstrap_ast()).expect("registry bootstrap is closed");
    rt.inj(&bootstrap, &Env::new(), &fixed_rand())
        .await
        .unwrap();
    let root = rt.create_checkpoint().await.unwrap().root;
    assert_state_hash("empty_state", root.as_bytes());
}

#[tokio::test]
async fn failing_deploy_is_captured_not_propagated() {
    let (rt, _replay) = build_runtime_pair().await;
    // `1 + "a"` is a well-formed term whose reduction is a type error (Int + String).
    let res = rt
        .evaluate(r#"@"chan"!(1 + "a")"#, &fixed_rand())
        .await
        .expect("evaluate returns Ok");
    assert!(res.failed(), "expected a captured failure");
    assert!(!res.errors.is_empty());
    // The post-state is still checkpointable.
    rt.create_checkpoint()
        .await
        .expect("checkpoint after failed deploy");
}

#[tokio::test]
async fn peek_and_persistent_work() {
    let (rt, _) = build_runtime_pair().await;
    let rand = fixed_rand();

    // Peek (`<<-`): read without consuming.
    rt.evaluate(
        r#"new c in { c!(42) | for (@x <<- c) { @"peek"!(x) } }"#,
        &rand,
    )
    .await
    .unwrap();
    assert_eq!(
        rt.get_data_par(&chan("peek")).await.unwrap(),
        vec![from_expr(Expr::GInt(42))]
    );

    // Persistent send (`!!`): datum stays across two consumes.
    rt.evaluate(
        r#"new c in { c!!(42) | for (@x <- c) { @"p1"!(x) } | for (@y <- c) { @"p2"!(y) } }"#,
        &rand,
    )
    .await
    .unwrap();
    assert_eq!(
        rt.get_data_par(&chan("p1")).await.unwrap(),
        vec![from_expr(Expr::GInt(42))]
    );
    assert_eq!(
        rt.get_data_par(&chan("p2")).await.unwrap(),
        vec![from_expr(Expr::GInt(42))]
    );
}

#[tokio::test]
async fn list_channel_matches() {
    let (rt, _) = build_runtime_pair().await;
    let rand = fixed_rand();

    // The blessed `MakeNode` shape: a contract binds `@node` (a PROC var) and sends on the
    // list-as-channel `@[node, *storeToken]`; a receive on `@["key", *storeToken]` must see it.
    let r = rt
        .evaluate(r#"new storeToken, Make in { contract Make(@initVal, @node) = { @[node, *storeToken]!(initVal) } | Make!(7, "key") | for (@x <- @["key", *storeToken]) { @"listch"!(x) } }"#, &rand)
        .await
        .unwrap();
    assert!(r.succeeded(), "list-as-channel term errors: {:?}", r.errors);
    assert_eq!(
        rt.get_data_par(&chan("listch")).await.unwrap(),
        vec![from_expr(Expr::GInt(7))]
    );
}

/// The reduction corpus shared by the scheduler differential tests: same-channel races, joins,
/// re-entry, nested barriers, persistent/peek re-produces, and the S.3 cross-channel
/// counterexample term.
const REDUCTION_CORPUS: &[&str] = &[
    r#"@"chan"!(42)"#,
    r#"new c in { c!(42) | for (@x <- c) { @"out"!(x) } }"#,
    r#"new c in { c!!(42) | for (@x <- c) { @"p1"!(x) } | for (@y <- c) { @"p2"!(y) } }"#,
    r#"new c in { c!(42) | for (@x <<- c) { @"peek"!(x) } }"#,
    r#"new storeToken, Make in { contract Make(@initVal, @node) = { @[node, *storeToken]!(initVal) } | Make!(7, "key") | for (@x <- @["key", *storeToken]) { @"listch"!(x) } }"#,
    r#"@"a"!(1) | @"b"!(2) | @"c"!(3) | @"d"!(4) | @"e"!(5) | @"f"!(6) | @"g"!(7) | @"h"!(8) | @"i"!(9) | @"j"!(10)"#,
    // Same-channel race: several produces compete for one receive. Sorted (content-addressed)
    // selection picks the same sorted-first datum under both schedulers when the receive
    // commits consume-side; under relaxed a produce-side match prepends the in-flight datum,
    // so the matched datum is landing-order dependent — the final state is free (the term
    // sits in RELAXED_FREE_TERMS).
    r#"new c in { c!(1) | c!(2) | c!(3) | for (@x <- c) { @"race"!(x) } }"#,
    // Join: a multi-channel receive overlapping two sibling sends on disjoint channels.
    r#"new c, d in { c!(1) | d!(2) | for (@x <- c; @y <- d) { @"join"!([x, y]) } }"#,
    // Join race: multiple data on both join channels; sorted selection fixes the winner.
    r#"new c, d in { c!(1) | c!(2) | d!(10) | d!(20) | for (@x <- c; @y <- d) { @"joinrace"!([x, y]) } }"#,
    // Transitive re-entry: a continuation sends back on its trigger channel before the next sibling.
    r#"new c in { c!(1) | for (@x <- c) { c!(x + 10) } | for (@y <- c) { @"out"!(y) } }"#,
    // Nested `new` is a scheduling barrier between disjoint sibling effects.
    r#"new x in { @"a"!(1) | new y in { @"b"!(2) } | @"c"!(3) }"#,
    // Persistent re-produce lands after the continuation subtree, before the disjoint sibling.
    r#"new c in { c!!(42) | for (@x <- c) { @"p"!(x) } | @"after"!(0) }"#,
    // Disjoint-channel continuations reduce independently.
    r#"new c, d in { c!(1) | d!(2) | for (@x <- c) { @"oc"!(x) } | for (@y <- d) { @"od"!(y) } }"#,
    // Cross-channel race (the continuation-footprint counterexample): the receive on `c` produces
    // on `d`, a channel a *disjoint-looking* sibling also touches. A static channel-sharded
    // fork-join would let the `for(@y<-d)` consume before `d!(x)` lands; the path-ordered scheduler
    // must instead apply d!(2), d!(3), d!(1), then the receive, so `@"out"` gets 1 (sorted-first).
    r#"new c, d in { c!(1) | d!(2) | d!(3) | for (@x <- c) { d!(x) } | for (@y <- d) { @"out"!(y) } }"#,
    // Cross-channel re-entry into a join: a continuation feeds a sibling join's channel.
    r#"new c, d in { c!(1) | d!(2) | for (@x <- c) { d!(x + 10) } | for (@y <- d; @z <- c) { @"join"!([y, z]) } }"#,
];

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_and_sequential_state_hashes_match() {
    // The concurrent reducer (fork-join) must produce the same post-state hash as the sequential
    // reference (concurrency off) — the executable form of the linearization theorem (Laws 4/8).
    for term in REDUCTION_CORPUS {
        let rt_c = build_runtime(true).await;
        let rt_s = build_runtime(false).await;
        let rand = fixed_rand();
        let rc = rt_c.evaluate(term, &rand).await.unwrap();
        assert!(
            rc.succeeded(),
            "concurrent deploy errors for {term}: {:?}",
            rc.errors
        );
        let rs = rt_s.evaluate(term, &rand).await.unwrap();
        assert!(
            rs.succeeded(),
            "sequential deploy errors for {term}: {:?}",
            rs.errors
        );
        let hc = rt_c.create_checkpoint().await.unwrap().root;
        let hs = rt_s.create_checkpoint().await.unwrap().root;
        assert_eq!(
            hc, hs,
            "concurrent vs sequential state hash mismatch for {term}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn gate_and_sequential_state_hashes_match() {
    // Law 21 (`gate_exec_refines_apply`): the gate scheduler — the task for effect `i` runs only
    // after the tasks for effects `0..i−1` complete — must reach the sequential reducer's state
    // exactly, including the event log (the executable half of the gate-equivalence proof).
    for term in REDUCTION_CORPUS {
        let rt_g = build_runtime_with_mode(true, EffectMode::Gate).await;
        let rt_s = build_runtime(false).await;
        let rand = fixed_rand();
        let rg = rt_g.evaluate(term, &rand).await.unwrap();
        assert!(
            rg.succeeded(),
            "gate deploy errors for {term}: {:?}",
            rg.errors
        );
        let rs = rt_s.evaluate(term, &rand).await.unwrap();
        assert!(
            rs.succeeded(),
            "sequential deploy errors for {term}: {:?}",
            rs.errors
        );
        let cg = rt_g.create_checkpoint().await.unwrap();
        let cs = rt_s.create_checkpoint().await.unwrap();
        assert_eq!(
            cg.root, cs.root,
            "gate vs sequential state hash mismatch for {term}"
        );
        assert_eq!(
            cg.log, cs.log,
            "gate vs sequential event log mismatch for {term}"
        );
    }
}

/// The channels an event touches (the per-channel projection of the event log, Law 20): the
/// trigger channel of a produce, the full source set of a consume, and both for a COMM.
fn event_channels(event: &Event) -> Vec<Blake2b256Hash> {
    match event {
        Event::Produce(p) => vec![p.channels_hash],
        Event::Consume(c) => c.channels_hashes.clone(),
        Event::Comm(comm) => {
            let mut channels = comm.consume.channels_hashes.clone();
            channels.extend(comm.produces.iter().map(|p| p.channels_hash));
            channels
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn relaxed_mode_runs_all_corpus_terms_without_error() {
    // The relaxed scheduler must reduce every corpus term to completion without error — including
    // the S.3 cross-channel counterexample and the join re-entry terms — and `evaluate` returns
    // only after the root drain has joined every spawned task, leaving a checkpointable post-state.
    for term in REDUCTION_CORPUS {
        let rt = build_runtime_with_mode(true, EffectMode::Relaxed).await;
        let res = rt.evaluate(term, &fixed_rand()).await.unwrap();
        assert!(
            res.succeeded(),
            "relaxed deploy errors for {term}: {:?}",
            res.errors
        );
        rt.create_checkpoint()
            .await
            .expect("checkpoint after relaxed deploy");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn relaxed_validated_mode_runs_corpus_without_error() {
    // The validated block-path mode (Laws 23–25) dispatches as relaxed at the rholang level —
    // the sequential-reference oracle and fallback live on the casper block path
    // (`RuntimeManager::validate_relaxed_block`) — so at this level it must reduce the whole
    // corpus without error, exactly like the relaxed arm, including the free terms whose
    // block-path runs fall back to the sequential trace.
    for term in REDUCTION_CORPUS {
        let rt = build_runtime_with_mode(true, EffectMode::RelaxedValidated).await;
        let res = rt.evaluate(term, &fixed_rand()).await.unwrap();
        assert!(
            res.succeeded(),
            "relaxed-validated deploy errors for {term}: {:?}",
            res.errors
        );
        rt.create_checkpoint()
            .await
            .expect("checkpoint after relaxed-validated deploy");
    }
}

// The former RELAXED_COMM_ONLY_TERMS classification — re-entry and root-sibling terms whose
// standalone install/store events may interleave — is subsumed by the COMM-only projection:
// those terms' per-channel COMM sequences match the sequential reference (only COMM events are
// compared at all), so `relaxed_preserves_same_channel_order` asserts them together with the
// rest of the corpus.

/// Corpus terms whose relaxed outcome is genuinely *free*: their final state — and for some,
/// even their per-channel COMM order — depends on the landing order of independent tasks, so no
/// sequential-reference assertion can be made. They are asserted only to reduce without error
/// (`relaxed_mode_runs_all_corpus_terms_without_error`); the validated block-path mode
/// (`EffectMode::RelaxedValidated`) ships the sequential fallback trace for exactly these.
///
/// The S.3 cross-channel counterexample: the continuation of the receive on `c` produces on
/// `d`, which a later-path sibling also consumes from — `@"out"` receives 1 or 2 depending on
/// the interleaving.
const RELAXED_FREE_TERMS: &[&str] = &[
    r#"new c, d in { c!(1) | d!(2) | d!(3) | for (@x <- c) { d!(x) } | for (@y <- d) { @"out"!(y) } }"#,
    // Join race: whichever produce lands first on `c` (or `d`) determines which pair the join
    // consumes, so the final state itself is free — the S.3 class, join form.
    r#"new c, d in { c!(1) | c!(2) | d!(10) | d!(20) | for (@x <- c; @y <- d) { @"joinrace"!([x, y]) } }"#,
    // Transitive re-entry: the produce matches whichever of the two receives installed first,
    // so the consumed datum (and which continuation fires) is landing-order dependent — the
    // final state is free, the S.3 class, re-entry form.
    r#"new c in { c!(1) | for (@x <- c) { c!(x + 10) } | for (@y <- c) { @"out"!(y) } }"#,
    // Cross-channel re-entry into a join: the join may match the raw `c!(1)`/`d!(2)` pair
    // before the re-entry continuation fires, so whether the continuation ever runs (and the
    // final join output) is landing-order dependent — the final state is free.
    r#"new c, d in { c!(1) | d!(2) | for (@x <- c) { d!(x + 10) } | for (@y <- d; @z <- c) { @"join"!([y, z]) } }"#,
    // Same-channel race: whichever produce lands first after the receive installs COMMs first
    // (a produce-side match prepends the in-flight datum, so the matched datum is
    // landing-order dependent, not sorted-first) — the final state is free, the S.3 class,
    // same-channel form.
    r#"new c in { c!(1) | c!(2) | c!(3) | for (@x <- c) { @"race"!(x) } }"#,
];

/// Project both logs onto every touched channel and compare the per-channel COMM *sequences*
/// (Law 20, `queue_commit_path_ordered`): same-channel commits follow the path-sorted claim
/// order, so the per-channel COMM projections are equal in sequence, not just as multisets.
/// Dispatch-time pre-claiming is what makes this executable — every claim lands in the queue at
/// dispatch time, so a late-landing earlier-path claim (the former persistent-produce
/// inversion) cannot occur. Only COMM events are compared at all: the standalone
/// install/store events interleave around a `Comm` and are not part of the relaxed contract.
fn assert_per_channel_comm_order(relaxed: &[Event], sequential: &[Event], label: &str) {
    let is_comm = |e: &Event| matches!(e, Event::Comm(_));
    let channels: BTreeSet<Blake2b256Hash> = sequential
        .iter()
        .chain(relaxed.iter())
        .filter(|e| is_comm(e))
        .flat_map(event_channels)
        .collect();
    for channel in &channels {
        let relaxed_sub: Vec<&Event> = relaxed
            .iter()
            .filter(|e| is_comm(e) && event_channels(e).contains(channel))
            .collect();
        let sequential_sub: Vec<&Event> = sequential
            .iter()
            .filter(|e| is_comm(e) && event_channels(e).contains(channel))
            .collect();
        assert_eq!(
            relaxed_sub, sequential_sub,
            "per-channel COMM order mismatch for {label} on channel {channel:?}"
        );
    }
}

/// The order-insensitive cross-check: the per-channel COMM *multisets* also match (implied by
/// the sequence equality above; kept as the independent form the relaxed-validated block path's
/// `comm_multisets_match` uses).
fn assert_per_channel_comm_multiset(relaxed: &[Event], sequential: &[Event], label: &str) {
    let is_comm = |e: &Event| matches!(e, Event::Comm(_));
    let channels: BTreeSet<Blake2b256Hash> = sequential
        .iter()
        .chain(relaxed.iter())
        .filter(|e| is_comm(e))
        .flat_map(event_channels)
        .collect();
    for channel in &channels {
        let mut relaxed_sub: Vec<&Event> = relaxed
            .iter()
            .filter(|e| is_comm(e) && event_channels(e).contains(channel))
            .collect();
        let mut sequential_sub: Vec<&Event> = sequential
            .iter()
            .filter(|e| is_comm(e) && event_channels(e).contains(channel))
            .collect();
        relaxed_sub.sort_by_key(|e| format!("{e:?}"));
        sequential_sub.sort_by_key(|e| format!("{e:?}"));
        assert_eq!(
            relaxed_sub, sequential_sub,
            "per-channel COMM multiset mismatch for {label} on channel {channel:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn relaxed_preserves_same_channel_order() {
    // Law 20 (executable form): the relaxed scheduler reaches the sequential reference
    // post-state and per-channel COMM *sequence* — same-channel commits follow the path-sorted
    // claim order (`queue_commit_path_ordered`), pinned by dispatch-time pre-claiming: every
    // claim lands in the queue at dispatch, so the persistent-produce inversion (a
    // late-landing earlier-path claim) cannot occur. Only COMM events are compared — the
    // standalone install/store events interleave around a Comm and are not part of the relaxed
    // contract — and the multiset cross-check runs alongside. The S.3 counterexample and the
    // other free terms are asserted to reduce without error, never here — their final state
    // is free.
    for term in REDUCTION_CORPUS {
        if RELAXED_FREE_TERMS.contains(term) {
            continue;
        }
        let rt_r = build_runtime_with_mode(true, EffectMode::Relaxed).await;

        let rt_s = build_runtime(false).await;
        let rand = fixed_rand();
        let rr = rt_r.evaluate(term, &rand).await.unwrap();
        assert!(
            rr.succeeded(),
            "relaxed deploy errors for {term}: {:?}",
            rr.errors
        );
        let rs = rt_s.evaluate(term, &rand).await.unwrap();
        assert!(
            rs.succeeded(),
            "sequential deploy errors for {term}: {:?}",
            rs.errors
        );
        let cr = rt_r.create_checkpoint().await.unwrap();
        let cs = rt_s.create_checkpoint().await.unwrap();
        assert_eq!(
            cr.root, cs.root,
            "relaxed vs sequential state hash mismatch for {term}"
        );
        assert_per_channel_comm_order(&cr.log, &cs.log, term);
        assert_per_channel_comm_multiset(&cr.log, &cs.log, term);
    }
}

#[tokio::test]
async fn match_takes_first_written_branch() {
    // Issue #10: branches were tried in reverse written order. The first branch (literal 1) must
    // win over the later catch-all.
    let (rt, _) = build_runtime_pair().await;
    let res = rt
        .evaluate(
            r#"new x in { match 1 { 1 => { @"ch"!("literal") } x => { @"ch"!("catchall") } } }"#,
            &fixed_rand(),
        )
        .await
        .expect("evaluate");
    assert!(res.succeeded(), "unexpected errors: {:?}", res.errors);
    assert_eq!(
        rt.get_data_par(&chan("ch")).await.unwrap(),
        vec![from_expr(Expr::GString("literal".to_string()))]
    );
}

#[tokio::test]
async fn match_wildcard_branch_matches() {
    // Issue #10: a `_` wildcard pattern never matched. It must catch the non-literal case.
    let (rt, _) = build_runtime_pair().await;
    let res = rt
        .evaluate(
            r#"new x in { match 5 { 0 => { @"ch"!("zero") } _ => { @"ch"!("wild") } } }"#,
            &fixed_rand(),
        )
        .await
        .expect("evaluate");
    assert!(res.succeeded(), "unexpected errors: {:?}", res.errors);
    assert_eq!(
        rt.get_data_par(&chan("ch")).await.unwrap(),
        vec![from_expr(Expr::GString("wild".to_string()))]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn unbounded_recursion_hits_depth_limit_not_stack_overflow() {
    // Issue #11: exploratory deploy of an unbounded recursive term used to overflow the tokio
    // worker stack and abort the whole process. With a generous phlo budget the recursion-depth
    // budget must stop it with a legible error, and the runtime must survive.
    let (rt, _) = build_runtime_pair().await;
    rt.cost().set(Cost::new(1_000_000_000, "test"));
    // Small budget so the test trips it quickly; the node default is much larger.
    rt.set_max_reduce_steps(2_000);
    let res = rt
        .evaluate(
            r#"new return, c in { c!(1) | contract c(@n) = { return!(n) | c!(n - 1) } }"#,
            &fixed_rand(),
        )
        .await
        .expect("evaluate returns Ok");
    assert!(
        res.failed(),
        "expected the reduction-step budget to fail the deploy"
    );
    assert!(
        res.errors
            .iter()
            .any(|e| e.to_string().contains("reduction step budget")),
        "expected a step-budget error, got: {:?}",
        res.errors
    );

    // The node-equivalent invariant: the runtime still reduces ordinary deploys afterwards.
    let ok = rt
        .evaluate(r#"@"ch2"!(42)"#, &fixed_rand())
        .await
        .expect("evaluate");
    assert!(ok.succeeded(), "runtime should still work: {:?}", ok.errors);
    assert_eq!(
        rt.get_data_par(&chan("ch2")).await.unwrap(),
        vec![from_expr(Expr::GInt(42))]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn nested_one_binder_contract_terminates() {
    // Issue #19: a one-binder contract inside a nested `new` re-fired forever. Root cause: `new`
    // passed the *pre-allocation* RNG to its body, so a nested `new` drew the same fresh-name bytes
    // as its parent; `c` collided with the outer `out`, and the contract body re-sent on its own
    // channel. The body must run with the RNG advanced past the freshly-allocated names.
    let (rt, _) = build_runtime_pair().await;
    rt.set_max_reduce_steps(2_000);
    let res = rt
        .evaluate(
            r#"new out in { new c, r in { contract c(x) = { out!("ran") } | c!(*r) } }"#,
            &fixed_rand(),
        )
        .await
        .expect("evaluate");
    assert!(
        res.succeeded(),
        "nested one-binder contract should terminate, got: {:?}",
        res.errors
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn nested_one_binder_contract_terminates_sequentially() {
    // The same issue #19 shape with the concurrent reducer disabled: the RNG bug is in the
    // scheduler-independent `New` resolution, not in the fork-join layer.
    let rt = build_runtime(false).await;
    rt.set_max_reduce_steps(2_000);
    let res = rt
        .evaluate(
            r#"new out in { new c, r in { contract c(x) = { out!("ran") } | c!(*r) } }"#,
            &fixed_rand(),
        )
        .await
        .expect("evaluate");
    assert!(
        res.succeeded(),
        "sequential nested one-binder contract should terminate, got: {:?}",
        res.errors
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn flat_one_binder_contract_terminates() {
    // Control for the issue #19 minimal pair: the same contract under a single `new`.
    let (rt, _) = build_runtime_pair().await;
    rt.set_max_reduce_steps(2_000);
    let res = rt
        .evaluate(
            r#"new out, c, r in { contract c(x) = { out!("ran") } | c!(*r) }"#,
            &fixed_rand(),
        )
        .await
        .expect("evaluate");
    assert!(
        res.succeeded(),
        "flat one-binder contract should terminate, got: {:?}",
        res.errors
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dropping_runtime_releases_its_space() {
    // Issues #18/#23: a forked exploratory runtime was kept alive forever by two Arc cycles —
    // dispatcher → dispatch-table handler → ContractCall → dispatcher, and dispatcher → eval
    // closure → reducer → dispatcher — so every request retained its whole RSpace/hot store.
    // Dropping the runtime must now release the space.
    let rt = build_runtime(true).await;
    let weak = Arc::downgrade(rt.space());
    drop(rt);
    assert!(
        weak.upgrade().is_none(),
        "runtime kept its RSpace alive through a reference cycle"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn contract_serves_repeated_calls_via_published_name() {
    // Issue #21: a persistent receive (contract) installed by one deploy became unreachable after
    // its first produce-side COMM because RSpace removed its join records while leaving the
    // continuation installed. Two calls to the same published contract must both be served.
    let (rt, _) = build_runtime_pair().await;
    let rand = fixed_rand();

    let publish = r#"new c in { contract c(@x, ret) = { ret!(x) } | @"svc"!!(*c) }"#;
    let res = rt.evaluate(publish, &rand).await.expect("publish evaluate");
    assert!(res.succeeded(), "publish failed: {:?}", res.errors);

    // Two separate programs each look the persistently-published name up and call it once. Both
    // produce-side COMMs against the installed contract must be served (issue #21).
    for n in [1, 2] {
        let term = format!(
            r#"for (c <- @"svc") {{ new r in {{ c!({n}, *r) | for (@v <- r) {{ @"seen{n}"!(v) }} }} }}"#
        );
        let res = rt.evaluate(&term, &rand).await.expect("call evaluate");
        assert!(res.succeeded(), "call {n} failed: {:?}", res.errors);
        assert_eq!(
            rt.get_data_par(&chan(&format!("seen{n}"))).await.unwrap(),
            vec![from_expr(Expr::GInt(n))]
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn cancellation_stops_runaway_reduction() {
    // Issue #12: dropping the outer evaluation future (a wall-clock timeout) does not stop the
    // spawned continuation tasks. The reducer's cooperative cancellation flag must make the
    // detached task tree unwind, and the runtime must remain usable afterwards.
    let (rt, _) = build_runtime_pair().await;
    let rt = Arc::new(rt);
    rt.cost().set(Cost::new(1_000_000_000, "test"));
    // Step budget far away: cancellation is the only thing that can stop this.
    rt.set_max_reduce_steps(1_000_000);

    let canceller = rt.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        canceller.cancel_reduce();
    });

    let res = rt
        .evaluate(
            r#"new return, loop in { loop!(0) | contract loop(@x) = { loop!(x + 1) } }"#,
            &fixed_rand(),
        )
        .await
        .expect("evaluate returns Ok");
    assert!(res.failed(), "expected cancellation to fail the deploy");
    assert!(
        res.errors
            .iter()
            .any(|e| e.to_string().contains("cancelled")),
        "expected a cancellation error, got: {:?}",
        res.errors
    );

    // The runtime survives and still reduces ordinary deploys.
    let ok = rt
        .evaluate(r#"@"ch3"!(42)"#, &fixed_rand())
        .await
        .expect("evaluate");
    assert!(ok.succeeded(), "runtime should still work: {:?}", ok.errors);
    assert_eq!(
        rt.get_data_par(&chan("ch3")).await.unwrap(),
        vec![from_expr(Expr::GInt(42))]
    );
}
