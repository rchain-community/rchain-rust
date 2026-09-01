//! End-to-end conformance test for the native system processes (issue #24).
//!
//! Each process is called from rholang the way the documentation describes — by its `rho:*` urn,
//! with an acknowledgement channel — and the test asserts on the *shape of the answer*, not only
//! that the call succeeded. The unit tests in `system_processes.rs` exercise the handlers directly;
//! these tests additionally pin the urn wiring (`rho:qucalc:zfa`, `rho:gov:*`, `rho:registry:*`)
//! and the rholang-level argument shapes a caller actually uses.

mod common;

use std::collections::BTreeMap;

use rchain_crypto::hash::blake2b512_random::Blake2b512Random;
use rchain_models::ast::{Expr, Par};
use rchain_models::par_ops::from_expr;
use rchain_models::rholang::RhoType::{
    RhoBoolean, RhoDeployerId, RhoList, RhoMap, RhoNil, RhoNumber, RhoString, RhoTupleN, RhoUri,
};
use rchain_models::sorted::SortedProc;
use rchain_rholang::runtime::RhoRuntime;

use common::build_runtime_pair;

/// A fixed, deterministic random seed so fresh-name allocation is reproducible.
fn fixed_rand() -> Blake2b512Random {
    Blake2b512Random::from_init(&[0u8; 32])
}

fn chan(name: &str) -> SortedProc {
    SortedProc::new(from_expr(Expr::GString(name.to_string())))
}

/// Evaluate `term` with the normalizer `env` and read the (single) result produced at `out_name`.
async fn eval_out(
    rt: &RhoRuntime,
    term: &str,
    env: &BTreeMap<String, Par>,
    out_name: &str,
) -> Vec<Par> {
    let res = rt
        .evaluate_with_env(term, env, &fixed_rand())
        .await
        .expect("evaluate returns Ok");
    assert!(
        res.succeeded(),
        "conformance term failed: {term:?}\nerrors: {:?}",
        res.errors
    );
    let data = rt
        .get_data_par(&chan(out_name))
        .await
        .expect("read out channel");
    assert!(
        !data.is_empty(),
        "term produced nothing at @{out_name:?}: {term:?}"
    );
    data
}

fn map_get_string_int(p: &Par, key: &str) -> Option<i64> {
    let kvs = RhoMap::unapply(p)?;
    kvs.iter().find_map(|(k, v)| {
        RhoString::unapply(k)
            .filter(|s| *s == key)
            .and_then(|_| RhoNumber::unapply(v))
    })
}

fn list_strings(p: &Par) -> Option<Vec<String>> {
    let ps = RhoList::unapply(p)?;
    ps.iter()
        .map(|p| RhoString::unapply(p).map(|s| s.to_string()))
        .collect()
}

#[tokio::test]
async fn qucalc_zfa_reports_zfa_and_phase_for_closed_and_open_histories() {
    let (rt, _) = build_runtime_pair().await;
    let env = BTreeMap::new();

    // ^v = [0, 1] is Pauli-closed and count-balanced: (true, -1).
    let closed = eval_out(
        &rt,
        r#"new zfa(`rho:qucalc:zfa`), ack in { zfa!([0, 1], *ack) | for (@res <- ack) { @"out"!(res) } }"#,
        &env,
        "out",
    )
    .await;
    let parts = RhoTupleN::unapply(&closed[0]).expect("qucalc:zfa returns a (zfa, phase) tuple");
    assert_eq!(parts.len(), 2);
    assert_eq!(RhoBoolean::unapply(&parts[0]), Some(true));
    assert_eq!(RhoNumber::unapply(&parts[1]), Some(-1));

    // A single twist is neither Pauli-closed nor count-balanced: (false, 0).
    let open = eval_out(
        &rt,
        r#"new zfa(`rho:qucalc:zfa`), ack in { zfa!([0], *ack) | for (@res <- ack) { @"out2"!(res) } }"#,
        &env,
        "out2",
    )
    .await;
    let parts = RhoTupleN::unapply(&open[0]).expect("qucalc:zfa returns a (zfa, phase) tuple");
    assert_eq!(RhoBoolean::unapply(&parts[0]), Some(false));
    assert_eq!(RhoNumber::unapply(&parts[1]), Some(0));
}

#[tokio::test]
async fn qucalc_grant_and_verify_round_trip() {
    let (rt, _) = build_runtime_pair().await;
    let env = BTreeMap::new();

    // A ZFA-closed history mints a capability URI.
    let uri_par = eval_out(
        &rt,
        r#"new grant(`rho:qucalc:grant`), ack in { grant!([0, 1], *ack) | for (@uri <- ack) { @"out"!(uri) } }"#,
        &env,
        "out",
    )
    .await;
    let uri = RhoUri::unapply(&uri_par[0])
        .expect("qucalc:grant returns a uri for a ZFA-closed history")
        .to_string();

    // A history that is not ZFA-closed yields Nil.
    let nil = eval_out(
        &rt,
        r#"new grant(`rho:qucalc:grant`), ack in { grant!([0], *ack) | for (@res <- ack) { @"out2"!(res) } }"#,
        &env,
        "out2",
    )
    .await;
    assert!(
        RhoNil::unapply(&nil[0]),
        "qucalc:grant returns Nil for a non-ZFA history"
    );

    // The minted capability verifies; an unknown uri does not.
    let ok = eval_out(
        &rt,
        &format!(
            r#"new verify(`rho:qucalc:verify`), ack in {{ verify!(`{uri}`, *ack) | for (@res <- ack) {{ @"out3"!(res) }} }}"#
        ),
        &env,
        "out3",
    )
    .await;
    assert_eq!(RhoBoolean::unapply(&ok[0]), Some(true));

    let bad = eval_out(
        &rt,
        r#"new verify(`rho:qucalc:verify`), ack in { verify!(`rho:id:deadbeef`, *ack) | for (@res <- ack) { @"out4"!(res) } }"#,
        &env,
        "out4",
    )
    .await;
    assert_eq!(RhoBoolean::unapply(&bad[0]), Some(false));
}

#[tokio::test]
async fn qucalc_fuse_returns_geometry_and_capability_or_nil() {
    let (rt, _) = build_runtime_pair().await;
    let env = BTreeMap::new();

    // Socrates syllogism: subject ^< ([0,3]) + predicate >v ([2,1]) -> ^<>v, a stable fluxoid.
    let fused = eval_out(
        &rt,
        r#"new fuse(`rho:qucalc:fuse`), ack in { fuse!([0, 3], [2, 1], *ack) | for (@res <- ack) { @"out"!(res) } }"#,
        &env,
        "out",
    )
    .await;
    let parts = RhoTupleN::unapply(&fused[0]).expect("qucalc:fuse returns (geometry, cap)");
    assert_eq!(parts.len(), 2);
    let geometry = RhoList::unapply(&parts[0]).expect("geometry is a twist list");
    let twists: Vec<i64> = geometry
        .iter()
        .map(|p| RhoNumber::unapply(p).expect("twist number"))
        .collect();
    assert_eq!(twists, vec![0, 3, 2, 1]);
    assert!(
        RhoUri::unapply(&parts[1]).is_some(),
        "fuse mints a capability uri"
    );

    // A subject/predicate whose residue is not ZFA-closed yields Nil.
    let nil = eval_out(
        &rt,
        r#"new fuse(`rho:qucalc:fuse`), ack in { fuse!([0], [2], *ack) | for (@res <- ack) { @"out2"!(res) } }"#,
        &env,
        "out2",
    )
    .await;
    assert!(
        RhoNil::unapply(&nil[0]),
        "qucalc:fuse returns Nil for a non-ZFA residue"
    );
}

#[tokio::test]
async fn gov_resolve_weights_reports_weight_map_from_tuple_shaped_inputs() {
    let (rt, _) = build_runtime_pair().await;
    let env = BTreeMap::new();

    let weights = eval_out(
        &rt,
        r#"new resolve(`rho:gov:resolveWeights`), ack in { resolve!(["A", "C"], {"B": "A", "C": "B"}, {}, *ack) | for (@res <- ack) { @"out"!(res) } }"#,
        &env,
        "out",
    )
    .await;
    assert!(
        RhoMap::unapply(&weights[0]).is_some(),
        "resolveWeights returns a map"
    );
    assert_eq!(map_get_string_int(&weights[0], "A"), Some(2));
    assert_eq!(map_get_string_int(&weights[0], "C"), Some(1));
}

#[tokio::test]
async fn gov_trust_levels_reports_admin_rooted_level_map() {
    let (rt, _) = build_runtime_pair().await;
    let env = BTreeMap::new();

    let levels = eval_out(
        &rt,
        r#"new trust(`rho:gov:trustLevels`), ack in { trust!([("Alice", "Bob", 3), ("Bob", "Carol", 2)], ["Alice"], *ack) | for (@res <- ack) { @"out"!(res) } }"#,
        &env,
        "out",
    )
    .await;
    assert!(
        RhoMap::unapply(&levels[0]).is_some(),
        "trustLevels returns a map"
    );
    assert_eq!(map_get_string_int(&levels[0], "Alice"), Some(5));
    assert_eq!(map_get_string_int(&levels[0], "Bob"), Some(3));
    assert_eq!(map_get_string_int(&levels[0], "Carol"), Some(2));
}

#[tokio::test]
async fn gov_censure_reports_discredited_and_slashed_levels() {
    let (rt, _) = build_runtime_pair().await;
    let env = BTreeMap::new();

    let out = eval_out(
        &rt,
        r#"new censure(`rho:gov:censure`), ack in { censure!([("A", "D"), ("B", "D")], {"A": 5, "B": 5, "C": 5, "D": 0}, [("A", "D", 2), ("B", "D", 1)], *ack) | for (@res <- ack) { @"out"!(res) } }"#,
        &env,
        "out",
    )
    .await;
    let parts = RhoTupleN::unapply(&out[0]).expect("gov:censure returns (discredited, newLevels)");
    assert_eq!(parts.len(), 2);
    assert_eq!(list_strings(&parts[0]), Some(vec!["D".to_string()]));
    assert_eq!(map_get_string_int(&parts[1], "A"), Some(3));
    assert_eq!(map_get_string_int(&parts[1], "B"), Some(4));
    assert_eq!(map_get_string_int(&parts[1], "C"), Some(5));
}

#[tokio::test]
async fn gov_tally_reports_winner_for_ranked_and_approval_modes() {
    let (rt, _) = build_runtime_pair().await;
    let env = BTreeMap::new();

    // A, B favour X; C favours Z, then X. Weighted ranked-choice elects X.
    let ranked = eval_out(
        &rt,
        r#"new tally(`rho:gov:tally`), ack in { tally!({"A": ["X", "Y"], "B": ["Y", "X"], "C": ["Z", "X"]}, {"A": 2, "B": 2, "C": 1}, "ranked", *ack) | for (@res <- ack) { @"out"!(res) } }"#,
        &env,
        "out",
    )
    .await;
    assert_eq!(RhoString::unapply(&ranked[0]), Some("X"));

    // Approval mode with the same ballots also elects X (X appears on every ballot).
    let approval = eval_out(
        &rt,
        r#"new tally(`rho:gov:tally`), ack in { tally!({"A": ["X", "Y"], "B": ["Y", "X"], "C": ["Z", "X"]}, {"A": 2, "B": 2, "C": 1}, "approval", *ack) | for (@res <- ack) { @"out2"!(res) } }"#,
        &env,
        "out2",
    )
    .await;
    assert_eq!(RhoString::unapply(&approval[0]), Some("X"));
}

#[tokio::test]
async fn registry_insert_arbitrary_and_lookup_round_trip() {
    let (rt, _) = build_runtime_pair().await;
    let env = BTreeMap::new();

    let uri_par = eval_out(
        &rt,
        r#"new insert(`rho:registry:insertArbitrary`), ack in { insert!(42, *ack) | for (@uri <- ack) { @"out"!(uri) } }"#,
        &env,
        "out",
    )
    .await;
    let uri = RhoUri::unapply(&uri_par[0])
        .expect("insertArbitrary returns a uri")
        .to_string();

    let looked_up = eval_out(
        &rt,
        &format!(
            r#"new lookup(`rho:registry:lookup`), ack in {{ lookup!(`{uri}`, *ack) | for (@res <- ack) {{ @"out2"!(res) }} }}"#
        ),
        &env,
        "out2",
    )
    .await;
    let parts = RhoTupleN::unapply(&looked_up[0]).expect("lookup returns (uri, value)");
    assert_eq!(parts.len(), 2);
    assert_eq!(RhoUri::unapply(&parts[0]), Some(uri.as_str()));
    assert_eq!(RhoNumber::unapply(&parts[1]), Some(42));

    // A uri that was never registered answers Nil (not silence).
    let missing = eval_out(
        &rt,
        r#"new lookup(`rho:registry:lookup`), ack in { lookup!(`rho:id:never-registered`, *ack) | for (@res <- ack) { @"out3"!(res) } }"#,
        &env,
        "out3",
    )
    .await;
    assert!(
        RhoNil::unapply(&missing[0]),
        "lookup of an unknown uri returns Nil"
    );
}

#[tokio::test]
async fn registry_insert_signed_binds_deployer_id_from_the_normalizer_env() {
    let (rt, _) = build_runtime_pair().await;
    // `rho:rchain:deployerId` is unbound in an exploratory deploy; a signed deploy gets it from the
    // normalizer env. Inject the same shape the deploy path uses.
    let mut env = BTreeMap::new();
    env.insert(
        "rho:rchain:deployerId".to_string(),
        RhoDeployerId::apply(vec![0x11; 65]),
    );

    let uri_par = eval_out(
        &rt,
        r#"new insert(`rho:registry:insertSigned:secp256k1`), deployerId(`rho:rchain:deployerId`), ack in { insert!((1, "data"), *deployerId, *ack) | for (@uri <- ack) { @"out"!(uri) } }"#,
        &env,
        "out",
    )
    .await;
    let uri = RhoUri::unapply(&uri_par[0])
        .expect("insertSigned returns a uri")
        .to_string();

    let looked_up = eval_out(
        &rt,
        &format!(
            r#"new lookup(`rho:registry:lookup`), ack in {{ lookup!(`{uri}`, *ack) | for (@res <- ack) {{ @"out2"!(res) }} }}"#
        ),
        &env,
        "out2",
    )
    .await;
    let parts = RhoTupleN::unapply(&looked_up[0]).expect("lookup returns (uri, value)");
    assert_eq!(parts.len(), 2);
    assert_eq!(RhoUri::unapply(&parts[0]), Some(uri.as_str()));
    // The stored value is the (nonce, data) tuple recorded under the deployer-derived uri.
    let stored = RhoTupleN::unapply(&parts[1]).expect("insertSigned stores a (nonce, data) tuple");
    assert_eq!(RhoNumber::unapply(&stored[0]), Some(1));
    assert_eq!(RhoString::unapply(&stored[1]), Some("data"));
}
