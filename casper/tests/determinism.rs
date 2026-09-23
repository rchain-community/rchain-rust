//! Determinism regression (spec: `docs/src/formal/determinism.md`).
//!
//! Play (block creation) and replay (validation) of a deploy that binds `rho:rchain:deployerId`
//! must produce the same post-state hash (sub-invariants S1/S3). This pins the replay-normalizer-env
//! and refund-amount fixes so future drift fails here instead of in consensus.

mod common;

use rchain_casper::genesis::contracts::Vault;
use rchain_crypto::hash::blake2b512_random::Blake2b512Random;
use rchain_crypto::public_key::PublicKey;
use rchain_models::casper::protocol::casper_message::{
    DeployData, ProcessedDeploy, ProcessedSystemDeploy, SignedDeployData,
};
use rchain_rholang::native_state::{NativeSystemState, PosGenesis};
use rchain_rholang::system_processes::BlockData;
use rchain_rholang::util::rev_address::RevAddress;
use rchain_shared::refined::NonNegI64;

fn deploy(term: &str) -> SignedDeployData {
    SignedDeployData {
        data: DeployData {
            attachments: Vec::new(),
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
            &PosGenesis::default(),
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
            &PosGenesis::default(),
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
            &PosGenesis::default(),
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
            &PosGenesis::default(),
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
            &PosGenesis::default(),
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
            &PosGenesis::default(),
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
            &PosGenesis::default(),
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
            &PosGenesis::default(),
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

/// **The negative replay path** (the register's G7): a deploy whose replay does not reproduce the
/// recorded state must be **rejected by the validating caller**, not accepted.
///
/// Two things this test establishes, one of which is a finding:
///
/// 1. Tampering with a *processed* deploy — the public `ProcessedDeploy`, no production change —
///    makes the replay produce a **different post-state hash**. The replay itself returns that hash
///    rather than an error, which is by design: `replay_compute_state` computes, and the caller
///    decides.
/// 2. The decision is `interpreter_util::handle_errors`, which compares the replayed hash against
///    the block's claimed `post_state_hash` and returns `Ok(None)` on a mismatch. **That comparison
///    is what carries the invariant**: the inner trace check (`check_replay_data_with_fix`) returns
///    `Ok` for a term tamper here, because it deliberately swallows mismatch for a deploy that is not
///    "eval successful" (the documented RCHAIN-3505 workaround). A test asserting only the inner
///    check would therefore pass while a tampered block was accepted.
#[tokio::test]
async fn a_tampered_deploy_replays_to_a_rejected_state_hash() {
    let rm = common::build_runtime_manager().await;
    let rand = Blake2b512Random::from_init(&[0u8; 32]);

    let (_pre, post, _) = rm
        .compute_genesis(
            &[],
            &rand,
            BlockData::empty(),
            &PosGenesis::default(),
            &[seeded_vault()],
        )
        .await
        .expect("compute_genesis");

    let term = r#"new deployerId(`rho:rchain:deployerId`) in { @"marker"!(42) }"#;
    let (post_state, user_results, sys_results) = rm
        .compute_state(&post, &[deploy(term)], &[], &rand, BlockData::empty())
        .await
        .expect("play compute_state");
    assert!(
        user_results[0].eval_result.succeeded(),
        "the recorded deploy must succeed: {:?}",
        user_results[0].eval_result.errors
    );
    let mut processed: Vec<ProcessedDeploy> = user_results.into_iter().map(|r| r.deploy).collect();
    let processed_sys: Vec<ProcessedSystemDeploy> =
        sys_results.into_iter().map(|r| r.deploy).collect();

    // Control: the untouched set replays to the recorded hash, and the validating comparison accepts
    // it (it returns the hash rather than `None`).
    let (clean_hash, _) = rm
        .replay_compute_state(
            &post,
            &processed,
            &processed_sys,
            &rand,
            BlockData::empty(),
            true,
            &PosGenesis::default(),
            &[],
        )
        .await
        .expect("a clean replay succeeds");
    assert_eq!(
        clean_hash, post_state,
        "a clean replay reproduces the state"
    );
    assert_eq!(
        rchain_casper::interpreter_util::handle_errors(&post_state, Ok(clean_hash))
            .expect("no internal error"),
        Some(clean_hash),
        "the clean replay is accepted"
    );

    // Tamper: the processed deploy claims to be a term that did not run.
    processed[0].deploy.data.term =
        r#"new deployerId(`rho:rchain:deployerId`) in { @"other"!(99) }"#.to_string();

    let (tampered_hash, _) = rm
        .replay_compute_state(
            &post,
            &processed,
            &processed_sys,
            &rand,
            BlockData::empty(),
            true,
            &PosGenesis::default(),
            &[],
        )
        .await
        .expect("the replay computes a hash; the caller judges it");

    assert_ne!(
        tampered_hash, post_state,
        "a tampered deploy must not replay to the recorded state"
    );
    assert_eq!(
        rchain_casper::interpreter_util::handle_errors(&post_state, Ok(tampered_hash))
            .expect("no internal error"),
        None,
        "the validating comparison must reject a state that does not match the block's claim"
    );
}

/// **A block that carries a `CloseBlock` system deploy replays to the state it played.**
///
/// `close_block` is the epoch transition and, since laws 44–47 landed, it writes native PoS state:
/// the committed-rewards map, the withdrawal requests and their claims, and the active set. Every
/// other play/replay test in this file passes an **empty** system-deploy list, so none of them
/// replayed a block with one — which is the gap this test closes, and the reason it is written
/// against a boundary with all four steps doing work rather than against a non-boundary no-op.
///
/// What a regression here looks like in production: a proposer produces a block, and every validator
/// that re-derives its state by replay refuses it — `merging.rs:514`'s
/// "regenerated mergeable channels for block … but replay computed … instead of …". That message is
/// shaped like a mergeable-channel disagreement, but the comparison it fails on is the *post-state
/// hash*, so the symptom is a chain that stops advancing behind a proposer that sees nothing wrong.
#[tokio::test]
async fn play_and_replay_agree_for_a_block_with_a_close_block_deploy() {
    use rchain_casper::system_deploy::SystemDeploy;
    use std::collections::BTreeSet;

    let rm = common::build_runtime_manager().await;
    let rand = Blake2b512Random::from_init(&[0u8; 32]);
    // The validator is the deployer, so its bond, its withdrawal and its reward are all its own.
    let validator = rchain_models::validator::Validator::new([0u8; 65]);
    let pos_genesis = PosGenesis {
        bonds: [(validator, NonNegI64::try_from(40).unwrap())]
            .into_iter()
            .collect(),
        trusted: BTreeSet::from([validator]),
        // `epoch_length: 1` makes the block's own close_block a boundary; a positive `minimum_bond`
        // takes the split out of the case where the contract's formula is undefined.
        params: rchain_rholang::native_state::PosParams {
            epoch_length: 1,
            quarantine_length: 0,
            minimum_bond: 1,
            ..Default::default()
        },
    };
    let (_pre, post, _) = rm
        .compute_genesis(
            &[],
            &rand,
            BlockData::empty(),
            &pos_genesis,
            &[seeded_vault()],
        )
        .await
        .expect("compute_genesis");

    // A bond (activating at the boundary), a staged withdrawal, and a deploy that spends phlo so the
    // epoch's pot is not zero: all four steps of the sequence then do work.
    let term = r#"new pos(`rho:rchain:pos`), deployerId(`rho:rchain:deployerId`), ret in {
  pos!("withdraw", *deployerId, *ret) |
  for (_ <- ret) { @"after"!(1) }
}"#;
    // The block's own number, on **both** paths: `block_creator` builds the close deploy from the
    // block it is creating (`SystemDeploy::close_block(i64::from(block_num), …)`), and the replay
    // reconstructs it from the block's `BlockData` (`runtime_replay.rs:426`). Passing a close deploy
    // whose number disagrees with the block's is not a state a real block can be in, and it is what
    // this test got wrong the first time it ran.
    let block_data = BlockData {
        block_number: rchain_shared::refined::BlockHeight::try_from(1).expect("height"),
        ..BlockData::empty()
    };
    let close = SystemDeploy::close_block(1, rand.split_byte(9));
    let (play_hash, user_results, sys_results) = rm
        .compute_state(&post, &[deploy(term)], &[close], &rand, block_data.clone())
        .await
        .expect("play compute_state");
    assert!(
        user_results[0].eval_result.succeeded(),
        "the staged withdrawal must succeed: {:?}",
        user_results[0].eval_result.errors
    );
    assert_eq!(sys_results.len(), 1, "the block's CloseBlock ran");

    let processed: Vec<ProcessedDeploy> = user_results.into_iter().map(|r| r.deploy).collect();
    let processed_sys: Vec<ProcessedSystemDeploy> =
        sys_results.into_iter().map(|r| r.deploy).collect();

    let (replay_hash, _) = rm
        .replay_compute_state(
            &post,
            &processed,
            &processed_sys,
            &rand,
            block_data,
            true,
            &pos_genesis,
            &[],
        )
        .await
        .expect("replay_compute_state");

    assert_eq!(
        play_hash, replay_hash,
        "a block whose close_block wrote the epoch's state must replay to the same post-state"
    );
}

/// **A genesis replay that omits the genesis vaults does not reproduce the genesis** (finding,
/// 2026-09-23). This test asserts the *divergence*, on purpose: it is the mechanism behind a real
/// failure, and it is here so the day someone fixes the path this test fails and says why.
///
/// What it pins. The genesis install funds the genesis wallets' REV vaults
/// (`ca4f5b015`, `RuntimeManager::compute_genesis`'s `for vault in vaults`), and those balances are
/// `PREFIX_VAULT` leaves in the genesis post-state. A node that did not *create* the genesis has no
/// mergeable-channel sidecar for it, so `merging.rs:486-506` regenerates one by replaying the block —
/// and that replay passes `&[]` for the vaults, on the stated assumption that "this is always
/// non-genesis block replay". It is not: the genesis is what the finalized fringe points at when a
/// validator joins, and the replay computes a different post-state hash
/// (`regenerated mergeable channels for block … but replay computed … instead of …`), which the
/// validator then refuses — so it never indexes block #0 and never advances.
///
/// Observed on `tools/devnet.sh up --validators 3` (2026-09-23): the bootstrap proposes happily, and
/// validators 1 and 2 sit at the height they joined at, logging exactly that message for the genesis
/// hash. `docs/src/formal/determinism.md` recorded this asymmetry as "safe because genesis is trusted
/// and never re-validated (an asserted invariant, not a code path)" — the parenthetical is the part
/// the devnet falsifies.
///
/// The fix is not to pass the vaults unconditionally (a non-genesis block's pre-state already has the
/// post-genesis balances, so re-installing would clobber them): it is to re-install them when the
/// block being replayed *is* the genesis — `pre_state_hash == empty_state_hash_fixed()` is the exact
/// test — which needs the genesis vault list reachable from the replay path.
#[tokio::test]
async fn a_genesis_replay_without_the_vaults_does_not_reproduce_the_genesis() {
    let rm = common::build_runtime_manager().await;
    let rand = Blake2b512Random::from_init(&[0u8; 32]);
    let vaults = [seeded_vault()];
    let (empty, post, results) = rm
        .compute_genesis(
            &[deploy(r#"@"chan"!(42)"#)],
            &rand,
            BlockData::empty(),
            &PosGenesis::default(),
            &vaults,
        )
        .await
        .expect("compute_genesis");
    let processed: Vec<ProcessedDeploy> = results.iter().map(|r| r.deploy.clone()).collect();

    // The genesis replay as `merging.rs` performs it, vaults and all...
    let (with_vaults, _) = rm
        .replay_compute_state(
            &empty,
            &processed,
            &[],
            &rand,
            BlockData::empty(),
            false,
            &PosGenesis::default(),
            &vaults,
        )
        .await
        .expect("replay with the genesis vaults");
    assert_eq!(
        post, with_vaults,
        "the genesis replays to itself once its vaults are re-installed — which is what makes the \
         divergence below a missing input rather than a missing rule"
    );

    // ...and as it performs it today.
    let (without_vaults, _) = rm
        .replay_compute_state(
            &empty,
            &processed,
            &[],
            &rand,
            BlockData::empty(),
            false,
            &PosGenesis::default(),
            &[],
        )
        .await
        .expect("replay without the genesis vaults");
    assert_ne!(
        post, without_vaults,
        "a genesis replay that omits the vaults must not reproduce the genesis; if this now passes, \
         the regeneration path was fixed and this test should become its opposite"
    );
}
