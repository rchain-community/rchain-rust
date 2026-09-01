//! Determinism regression (spec: `docs/src/formal/determinism.md`).
//!
//! Play (block creation) and replay (validation) of a deploy that binds `rho:rchain:deployerId`
//! must produce the same post-state hash (sub-invariants S1/S3). This pins the replay-normalizer-env
//! and refund-amount fixes so future drift fails here instead of in consensus.

mod common;

use std::collections::BTreeMap;

use rchain_casper::genesis::contracts::Vault;
use rchain_crypto::hash::blake2b512_random::Blake2b512Random;
use rchain_crypto::public_key::PublicKey;
use rchain_models::casper::protocol::casper_message::{
    DeployData, ProcessedDeploy, ProcessedSystemDeploy, SignedDeployData,
};
use rchain_rholang::native_state::NativeSystemState;
use rchain_rholang::system_processes::BlockData;
use rchain_rholang::util::rev_address::RevAddress;
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
        // 65-byte public key so RevAddress derivation succeeds (matches the seeded vault).
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

#[tokio::test]
async fn play_and_replay_agree_for_deployer_id_binding_deploy() {
    let rm = common::build_runtime_manager().await;
    let rand = Blake2b512Random::from_init(&[0u8; 32]);

    let (_pre, post, _) = rm
        .compute_genesis(
            &[],
            &rand,
            BlockData::empty(),
            &BTreeMap::new(),
            &[seeded_vault()],
        )
        .await
        .expect("compute_genesis");

    // Binds `rho:rchain:deployerId` (the REV-transfer idiom). Replay must normalize with the SAME
    // env, else `add_urn` fails with `BugFoundError` (S1) — before the fix this returned
    // `InvalidStateHash`.
    let term = r#"new deployerId(`rho:rchain:deployerId`) in { @"marker"!(true) }"#;

    let (post_state, user_results, sys_results) = rm
        .compute_state(&post, &[deploy(term)], &[], &rand, BlockData::empty())
        .await
        .expect("play compute_state");
    assert!(
        user_results[0].eval_result.succeeded(),
        "play deploy must succeed: {:?}",
        user_results[0].eval_result.errors
    );

    let processed: Vec<ProcessedDeploy> = user_results.into_iter().map(|r| r.deploy).collect();
    let processed_sys: Vec<ProcessedSystemDeploy> =
        sys_results.into_iter().map(|r| r.deploy).collect();

    let (replay_state, _) = rm
        .replay_compute_state(
            &post,
            &processed,
            &processed_sys,
            &rand,
            BlockData::empty(),
            true,
            &BTreeMap::new(),
            &[],
        )
        .await
        .expect("replay compute_state");

    assert_eq!(
        post_state, replay_state,
        "play and replay post-state hashes must agree (S1/S3)"
    );
}

#[tokio::test]
async fn play_and_replay_agree_for_transfer_deploy_and_vault_writes_persist() {
    let rm = common::build_runtime_manager().await;
    let rand = Blake2b512Random::from_init(&[0u8; 32]);

    let (_pre, post, _) = rm
        .compute_genesis(
            &[],
            &rand,
            BlockData::empty(),
            &BTreeMap::new(),
            &[seeded_vault()],
        )
        .await
        .expect("compute_genesis");

    let target = RevAddress::from_public_key(&PublicKey::new(vec![1u8; 65]))
        .expect("target address")
        .to_base58();
    let term = format!(
        r#"new revVault(`rho:rchain:revVault`), deployerId(`rho:rchain:deployerId`), r in {{ revVault!("transfer", *deployerId, "{target}", 30000000, *r) | for (_ <- r) {{ Nil }} }}"#
    );

    let (post_state, user_results, sys_results) = rm
        .compute_state(&post, &[deploy(&term)], &[], &rand, BlockData::empty())
        .await
        .expect("play compute_state");
    assert!(
        user_results[0].eval_result.succeeded(),
        "play transfer must succeed: {:?}",
        user_results[0].eval_result.errors
    );

    let processed: Vec<ProcessedDeploy> = user_results.into_iter().map(|r| r.deploy).collect();
    let processed_sys: Vec<ProcessedSystemDeploy> =
        sys_results.into_iter().map(|r| r.deploy).collect();

    let (replay_state, _) = rm
        .replay_compute_state(
            &post,
            &processed,
            &processed_sys,
            &rand,
            BlockData::empty(),
            true,
            &BTreeMap::new(),
            &[],
        )
        .await
        .expect("replay compute_state");

    assert_eq!(
        post_state, replay_state,
        "play and replay post-state hashes must agree for a revVault transfer"
    );

    // The transfer's vault writes must be visible at the committed post-state.
    let fork = rm
        .fork_play_runtime(replay_state)
        .await
        .expect("fork at replay state");
    fork.reset(replay_state).await.expect("reset fork");
    let native = NativeSystemState::new(fork.native_store());
    let target_balance = native
        .vault_balance(&target)
        .await
        .expect("read target balance")
        .map(|b| i64::from(b))
        .unwrap_or(0);
    assert_eq!(
        target_balance, 30_000_000,
        "target vault must hold the transferred 30_000_000"
    );
}

#[tokio::test]
async fn play_and_replay_agree_for_failed_user_deploy_with_recorded_error() {
    // Issue #15: a failed user deploy must record the reducer's error in `system_deploy_error`,
    // and replay must still reproduce the post-state (the recorded failure is the user deploy's,
    // not the pre-charge's, so replay must not skip the user-deploy evaluation).
    let rm = common::build_runtime_manager().await;
    let rand = Blake2b512Random::from_init(&[0u8; 32]);

    let (_pre, post, _) = rm
        .compute_genesis(
            &[],
            &rand,
            BlockData::empty(),
            &BTreeMap::new(),
            &[seeded_vault()],
        )
        .await
        .expect("compute_genesis");

    // Fails at runtime: `1 + "not-a-number"` is a type error.
    let term = r#"new return in { return!(1 + "not-a-number") }"#;

    let (post_state, user_results, sys_results) = rm
        .compute_state(&post, &[deploy(term)], &[], &rand, BlockData::empty())
        .await
        .expect("play compute_state");
    assert!(user_results[0].deploy.is_failed, "deploy must fail");
    assert!(
        user_results[0].deploy.system_deploy_error.is_some(),
        "failed deploy must record its error, got: {:?}",
        user_results[0].deploy.system_deploy_error
    );

    let processed: Vec<ProcessedDeploy> = user_results.into_iter().map(|r| r.deploy).collect();
    let processed_sys: Vec<ProcessedSystemDeploy> =
        sys_results.into_iter().map(|r| r.deploy).collect();

    let (replay_state, _) = rm
        .replay_compute_state(
            &post,
            &processed,
            &processed_sys,
            &rand,
            BlockData::empty(),
            true,
            &BTreeMap::new(),
            &[],
        )
        .await
        .expect("replay compute_state");

    assert_eq!(
        post_state, replay_state,
        "play and replay post-state hashes must agree for a failed user deploy"
    );
}

#[tokio::test]
async fn play_and_replay_agree_for_escrow_round_trip_deploy() {
    let rm = common::build_runtime_manager().await;
    let rand = Blake2b512Random::from_init(&[0u8; 32]);

    let (_pre, post, _) = rm
        .compute_genesis(
            &[],
            &rand,
            BlockData::empty(),
            &BTreeMap::new(),
            &[seeded_vault()],
        )
        .await
        .expect("compute_genesis");

    let target = RevAddress::from_public_key(&PublicKey::new(vec![1u8; 65]))
        .expect("target address")
        .to_base58();
    // The robotics-coordination "escrow" idiom: bind `rho:rchain:deployerId`, produce it onto a
    // fresh channel, and install a persistent contract whose body round-trips that deployerId
    // through the channel and calls the native revVault transfer.
    let term = r#"new deployerId(`rho:rchain:deployerId`), escrowCh in {
  escrowCh!(*deployerId) |
  contract @"raas:escrow:test"(@"complete", @fee, ret) = {
    for (d <- escrowCh) {
      new revVault(`rho:rchain:revVault`), resultCh in {
        revVault!("transfer", *d, "__TARGET__", fee, *resultCh) |
        for (_ <- resultCh) { escrowCh!(*d) | ret!(true) }
      }
    }
  } |
  contract @"raas:escrow:test"(@"query", ret) = { ret!("x") }
}"#
    .replace("__TARGET__", &target);

    let (play_hash, user_results, sys_results) = rm
        .compute_state(&post, &[deploy(&term)], &[], &rand, BlockData::empty())
        .await
        .expect("play compute_state");
    assert!(
        user_results[0].eval_result.succeeded(),
        "play deploy must succeed: {:?}",
        user_results[0].eval_result.errors
    );

    let processed: Vec<ProcessedDeploy> = user_results.into_iter().map(|r| r.deploy).collect();
    let processed_sys: Vec<ProcessedSystemDeploy> =
        sys_results.into_iter().map(|r| r.deploy).collect();

    let replay_hash = match rm
        .replay_compute_state(
            &post,
            &processed,
            &processed_sys,
            &rand,
            BlockData::empty(),
            true,
            &BTreeMap::new(),
            &[],
        )
        .await
    {
        Ok((hash, _)) => hash,
        Err(e) => panic!("replay failed with {e:?} (play_hash = {play_hash:?})"),
    };

    assert_eq!(
        play_hash, replay_hash,
        "play and replay post-state hashes must agree for the escrow round-trip deploy"
    );
}
