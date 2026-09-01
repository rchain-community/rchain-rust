//! Regression tests for #5/#6: system-process replies must land in a cost-accounted block's
//! post-state, and a runtime must be restartable on a chain containing an executed deploy.

mod common;

use std::sync::Arc;

use rchain_casper::genesis::contracts::Vault;
use rchain_casper::runtime_manager::RuntimeManager;
use rchain_crypto::hash::blake2b256_hash::Blake2b256Hash;
use rchain_crypto::hash::blake2b512_random::Blake2b512Random;
use rchain_crypto::public_key::PublicKey;
use rchain_models::ast::Expr;
use rchain_models::casper::protocol::casper_message::{DeployData, SignedDeployData};
use rchain_models::par_ops::from_expr;
use rchain_models::sorted::SortedProc;
use rchain_rholang::runtime::RhoRuntime;
use rchain_rholang::storage::RhoMatch;
use rchain_rholang::system_processes::BlockData;
use rchain_rholang::util::rev_address::RevAddress;
use rchain_rspace::hot_store::InMemHotStore;
use rchain_rspace::rspace::RSpace;
use rchain_shared::refined::NonNegI64;

fn deploy(term: &str) -> SignedDeployData {
    SignedDeployData {
        data: DeployData {
            term: term.to_string(),
            timestamp: 0,
            phlo_price: 1,
            phlo_limit: 500_000,
            valid_after_block_number: 0,
            shard_id: "root".to_string(),
        },
        // 65-byte public key so RevAddress derivation succeeds.
        deployer: vec![0u8; 65],
        sig: Vec::new(),
        sig_algorithm: "secp256k1".to_string(),
    }
}

fn seeded_vault() -> Vault {
    Vault {
        rev_address: RevAddress::from_public_key(&PublicKey::new(vec![0u8; 65]))
            .expect("valid rev address"),
        initial_balance: NonNegI64::try_from(1_000_000_000).unwrap(),
    }
}

/// Simulate a node restart at `root`: build a fresh play space + runtime reading the given root
/// (the same way a restarted node reopens the committed root).
async fn restart(rm: &RuntimeManager, root: Blake2b256Hash) -> Result<RhoRuntime, String> {
    let history = rm.get_history_repo();
    let reader = history.get_history_reader(root).await;
    let hot = Arc::new(InMemHotStore::new(reader.base()));
    let (play, _replay) = RSpace::create_with_replay(history.clone(), hot, Arc::new(RhoMatch));
    RhoRuntime::create(play, history.clone(), SortedProc::default())
        .await
        .map_err(|e| e.to_string())
}

#[tokio::test]
async fn system_process_replies_in_cost_accounted_block_and_restart_succeeds() {
    let rm = common::build_runtime_manager().await;
    let rand = Blake2b512Random::from_init(&[0u8; 32]);

    // Genesis with the deployer's vault seeded so pre-charge succeeds.
    let (_pre, post, _) = rm
        .compute_genesis(
            &[],
            &rand,
            BlockData::empty(),
            &std::collections::BTreeMap::new(),
            &[seeded_vault()],
        )
        .await
        .expect("compute_genesis");

    let term = r#"new zfa(`rho:qucalc:zfa`), ret in { zfa!([0, 1], *ret) | for (@v <- ret) { @"got-zfa"!(v) } }"#;
    let (post2, results, _) = rm
        .compute_state(&post, &[deploy(term)], &[], &rand, BlockData::empty())
        .await
        .expect("compute_state");
    assert!(
        results[0].eval_result.succeeded(),
        "deploy must succeed: {:?}",
        results[0].eval_result.errors
    );

    // The forwarded reply must be in the block's post-state (#5).
    rm.runtime()
        .reset(post2)
        .await
        .expect("reset to post state");
    let got = rm
        .runtime()
        .get_data_par(&SortedProc::new(from_expr(Expr::GString(
            "got-zfa".to_string(),
        ))))
        .await
        .expect("get_data_par");
    assert!(
        !got.is_empty(),
        "#5: system process reply must reach the block post-state"
    );

    // Restarting on this chain must not fail with InstallNotAllowed (#6).
    restart(&rm, post2)
        .await
        .expect("restart after executed deploy");
}
