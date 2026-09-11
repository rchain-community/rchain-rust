//! The effect-scheduler block-path gate (Laws 20–22): a relaxed node refuses consensus work but
//! stays open for off-chain exploratory deploys.

mod common;

use rchain_crypto::hash::blake2b512_random::Blake2b512Random;
use rchain_models::block::state_hash::StateHash;
use rchain_models::casper::protocol::casper_message::{DeployData, SignedDeployData};
use rchain_rholang::scheduler::EffectMode;

use common::build_runtime_manager_with_mode;

fn fixed_rand() -> Blake2b512Random {
    Blake2b512Random::from_init(&[0u8; 32])
}

/// A minimal signed deploy with the given term (mirrors `consensus.rs`; signature verification is
/// deferred to the deploy-acceptance path, so the sig/deployer fields stay empty).
fn deploy(term: &str) -> SignedDeployData {
    SignedDeployData {
        data: DeployData {
            term: term.to_string(),
            timestamp: 0,
            phlo_price: 1,
            phlo_limit: 90_000,
            valid_after_block_number: 0,
            shard_id: "root".to_string(),
        },
        deployer: vec![0u8; 32],
        sig: Vec::new(),
        sig_algorithm: "secp256k1".to_string(),
    }
}

#[tokio::test]
async fn block_paths_reject_relaxed_mode() {
    // Both block-path entry points hard-reject under the relaxed scheduler — a relaxed interleaving
    // must never reach a block's event log (off-chain only) — while the same manager shape under
    // the sequential scheduler accepts the deploy.
    let d = deploy(r#"@"chan"!(42)"#);

    let relaxed = build_runtime_manager_with_mode(EffectMode::Relaxed).await;
    let err = relaxed
        .process_deploy(&d, &fixed_rand())
        .await
        .expect_err("relaxed process_deploy must hard-reject");
    assert!(err.contains("off-chain only"), "unexpected reason: {err}");

    let start = relaxed.get_history_repo().root();
    let err = relaxed
        .play_deploys(&start, std::slice::from_ref(&d), &fixed_rand())
        .await
        .expect_err("relaxed play_deploys must hard-reject");
    assert!(err.contains("off-chain only"), "unexpected reason: {err}");

    let sequential = build_runtime_manager_with_mode(EffectMode::Sequential).await;
    let (_, eval_result) = sequential
        .process_deploy(&d, &fixed_rand())
        .await
        .expect("sequential process_deploy succeeds");
    assert!(
        eval_result.succeeded(),
        "deploy should succeed: {:?}",
        eval_result.errors
    );
}

#[tokio::test]
async fn exploratory_path_stays_open_in_relaxed_mode() {
    // The relaxed manager's only deploy outlet is the exploratory path (`explore-deploy`), which
    // forks an isolated runtime under the manager's mode — it must keep working.
    let relaxed = build_runtime_manager_with_mode(EffectMode::Relaxed).await;
    let start = StateHash::from_slice(relaxed.get_history_repo().root().as_bytes());
    let res = relaxed
        .play_exploratory_deploy(r#"@"chan"!(42)"#, &start)
        .await
        .expect("relaxed exploratory deploy succeeds");
    assert!(res.is_empty(), "no return-channel data expected");
}
