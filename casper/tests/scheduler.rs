//! The effect-scheduler block-path gate (Laws 20–22): a relaxed node refuses consensus work but
//! stays open for off-chain exploratory deploys.

mod common;

use rchain_casper::event_converter::comm_multisets_match;
use rchain_casper::runtime_manager::RuntimeManager;
use rchain_crypto::hash::blake2b256_hash::Blake2b256Hash;
use rchain_crypto::hash::blake2b512_random::Blake2b512Random;
use rchain_crypto::public_key::PublicKey;
use rchain_models::block::state_hash::StateHash;
use rchain_models::casper::protocol::casper_message::{DeployData, SignedDeployData};
use rchain_rholang::native_state::NativeSystemState;
use rchain_rholang::scheduler::EffectMode;
use rchain_rholang::system_processes::BlockData;
use rchain_rholang::util::rev_address::RevAddress;
use rchain_shared::refined::NonNegI64;

use common::build_runtime_manager_with_mode;

fn fixed_rand() -> Blake2b512Random {
    Blake2b512Random::from_init(&[0u8; 32])
}

/// The test deployer key: `vec![1u8; 65]` parses as a valid uncompressed secp256k1 point (the
/// native-state tests use it), so the pre-charge path of `compute_state` accepts it.
const DEPLOYER: [u8; 65] = [1u8; 65];

/// Seed the deployer's vault into the empty state and checkpoint, returning the pre-state root
/// the cost-accounting block path (`compute_state`) can pre-charge against.
async fn seed_vault(rm: &RuntimeManager) -> Blake2b256Hash {
    let pk = PublicKey::new(DEPLOYER.to_vec());
    let addr = RevAddress::from_public_key(&pk).unwrap().to_base58();
    let native = NativeSystemState::new(rm.runtime().native_store());
    native.set_vault_balance(&addr, NonNegI64::try_from(1_000_000_000).unwrap());
    rm.runtime().create_checkpoint().await.unwrap().root
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
        deployer: DEPLOYER.to_vec(),
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

/// Terms spanning the three validated-speculation outcomes (Laws 23–25): a single-channel term
/// (relaxed and sequential logs agree), a re-entry term whose install/store events may interleave
/// (the per-channel COMM oracle), and the S.3 term whose relaxed outcome is genuinely free (the
/// validation must fall back to the sequential trace).
const VALIDATED_CORPUS: &[&str] = &[
    r#"@"chan"!(42)"#,
    r#"new c in { c!(1) | for (@x <- c) { c!(x + 10) } | for (@y <- c) { @"out"!(y) } }"#,
    r#"new c, d in { c!(1) | d!(2) | d!(3) | for (@x <- c) { d!(x) } | for (@y <- d) { @"out"!(y) } }"#,
];

/// The validated mode passes the block-path entry points (no hard-reject), while the pure
/// relaxed mode stays rejected (`block_paths_reject_relaxed_mode`).
#[tokio::test]
async fn relaxed_validated_accepts_block_paths() {
    let d = deploy(r#"@"chan"!(42)"#);
    let validated = build_runtime_manager_with_mode(EffectMode::RelaxedValidated).await;

    let (_, eval_result) = validated
        .process_deploy(&d, &fixed_rand())
        .await
        .expect("relaxed-validated process_deploy succeeds");
    assert!(
        eval_result.succeeded(),
        "deploy should succeed: {:?}",
        eval_result.errors
    );

    let start = validated.get_history_repo().root();
    validated
        .play_deploys(&start, std::slice::from_ref(&d), &fixed_rand())
        .await
        .expect("relaxed-validated play_deploys succeeds");
}

/// The validated block path ships the sequential reference's state and per-channel COMM order:
/// the post-state hash equals the sequential run's, and the shipped deploy log's per-channel
/// COMM subsequences equal the sequential log's — whether the relaxed trace was accepted or the
/// validation fell back (the S.3 term's relaxed outcome is free, so its result must be the
/// sequential one either way).
#[tokio::test]
async fn relaxed_validated_compute_state_matches_sequential() {
    for term in VALIDATED_CORPUS {
        let validated = build_runtime_manager_with_mode(EffectMode::RelaxedValidated).await;
        let sequential = build_runtime_manager_with_mode(EffectMode::Sequential).await;
        let vstart = seed_vault(&validated).await;
        let sstart = seed_vault(&sequential).await;
        assert_eq!(vstart, sstart, "seeded pre-states must match");
        let start = vstart;
        let rand = fixed_rand();
        let d = deploy(term);

        let (vhash, vuser, vsys) = validated
            .compute_state(
                &start,
                std::slice::from_ref(&d),
                &[],
                &rand,
                BlockData::empty(),
            )
            .await
            .expect("validated compute_state");
        let (shash, suser, ssys) = sequential
            .compute_state(
                &start,
                std::slice::from_ref(&d),
                &[],
                &rand,
                BlockData::empty(),
            )
            .await
            .expect("sequential compute_state");

        assert_eq!(
            vhash, shash,
            "validated vs sequential state hash mismatch for {term}"
        );
        assert_eq!(vsys, ssys, "system deploy results must match for {term}");
        assert_eq!(vuser.len(), 1);
        assert!(
            comm_multisets_match(&vuser[0].deploy.deploy_log, &suser[0].deploy.deploy_log),
            "per-channel COMM multisets must match for {term}"
        );
    }
}
