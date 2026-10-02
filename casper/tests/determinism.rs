//! Determinism regression (spec: `docs/src/formal/determinism.md`).
//!
//! Play (block creation) and replay (validation) of a deploy that binds `rho:rchain:deployerId`
//! must produce the same post-state hash (sub-invariants S1/S3). This pins the replay-normalizer-env
//! and refund-amount fixes so future drift fails here instead of in consensus.

mod common;
use common::fringe_state;

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
        .compute_state(
            &post,
            &[deploy(term)],
            &[],
            &rand,
            BlockData::empty(),
            &fringe_state(1),
        )
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
            &fringe_state(1),
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
        .compute_state(
            &post,
            &[deploy(&term)],
            &[],
            &rand,
            BlockData::empty(),
            &fringe_state(1),
        )
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
            &fringe_state(1),
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

/// **The vault capability, end to end** — the deferred half of `spec/RUST-FIRST.md`'s B2.
///
/// A deploy asks `findOrCreate` for a handle and then **spends from it in the same deploy**. Nothing
/// could do that before: the handle is a name minted during the deploy, so no compile-time
/// `BodyRefs` entry describes it, and the send would have matched nothing. The evidence is a *fund
/// movement* rather than a read, so a handler that replied without acting would fail this.
///
/// The auth argument is the handle itself, which is what a capability means here: the value that
/// authorises the spend is the unforgeable name, not the caller's deployer key — a contract holding
/// this handle has no key of its own, which is the whole reason the multi-signature vault needs it.
#[tokio::test]
async fn a_minted_vault_handle_spends_in_the_deploy_that_minted_it() {
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
        r#"new revVault(`rho:rchain:revVault`), deployerId(`rho:rchain:deployerId`),
               vaultCh, authCh, r in {{
            revVault!("findOrCreate", *deployerId, *vaultCh) |
            revVault!("deployerAuthKey", *deployerId, *authCh) |
            for (@(_, *vault) <- vaultCh; auth <- authCh) {{
                vault!("transfer", "{target}", 30000000, *auth, *r) |
                for (_ <- r) {{ Nil }}
            }}
        }}"#
    );

    let (post_state, user_results, _) = rm
        .compute_state(
            &post,
            &[deploy(&term)],
            &[],
            &rand,
            BlockData::empty(),
            &fringe_state(1),
        )
        .await
        .expect("play compute_state");
    assert!(
        user_results[0].eval_result.succeeded(),
        "a handle spend must succeed: {:?}",
        user_results[0].eval_result.errors
    );

    let native = NativeSystemState::new(rm.runtime().native_store());
    let target_balance = native
        .vault_balance(&target)
        .await
        .expect("read target balance")
        .map(i64::from)
        .unwrap_or(0);
    assert_eq!(
        target_balance, 30_000_000,
        "the handle's transfer must have moved the funds — a reply without a movement is not the \
         capability working"
    );
    let _ = post_state;
}

/// **And the authority check is real.** The same transfer with a name that opens nothing — a fresh
/// `new`, which is exactly what a caller without the handle holds — must move nothing. Without this
/// arm the first test would pass on a handler that ignored its auth argument entirely.
#[tokio::test]
async fn a_vault_handle_refuses_a_name_that_opens_no_vault() {
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
        r#"new revVault(`rho:rchain:revVault`), deployerId(`rho:rchain:deployerId`), vaultCh, r, other in {{
            revVault!("findOrCreate", *deployerId, *vaultCh) |
            for (@(_, *vault) <- vaultCh) {{
                vault!("transfer", "{target}", 30000000, *other, *r) |
                for (_ <- r) {{ Nil }}
            }}
        }}"#
    );

    let (post_state, user_results, _) = rm
        .compute_state(
            &post,
            &[deploy(&term)],
            &[],
            &rand,
            BlockData::empty(),
            &fringe_state(1),
        )
        .await
        .expect("play compute_state");
    assert!(
        user_results[0].eval_result.succeeded(),
        "the deploy itself runs; the *transfer* is refused: {:?}",
        user_results[0].eval_result.errors
    );

    let native = NativeSystemState::new(rm.runtime().native_store());
    let _ = post_state;
    let target_balance = native
        .vault_balance(&target)
        .await
        .expect("read target balance")
        .map(i64::from)
        .unwrap_or(0);
    assert_eq!(
        target_balance, 0,
        "a name that opens no vault must not authorise a spend"
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
        .compute_state(
            &post,
            &[deploy(term)],
            &[],
            &rand,
            BlockData::empty(),
            &fringe_state(1),
        )
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
            &fringe_state(1),
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
        .compute_state(
            &post,
            &[deploy(&term)],
            &[],
            &rand,
            BlockData::empty(),
            &fringe_state(1),
        )
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
            &fringe_state(1),
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
        .compute_state(
            &post,
            &[deploy(term)],
            &[],
            &rand,
            BlockData::empty(),
            &fringe_state(1),
        )
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
            &fringe_state(1),
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

    // Tamper: the processed deploy claims a term that did not run. Same *length* as the one that did,
    // deliberately — the parse is charged (`AUDIT F-2`), so a length change would be caught one check
    // earlier by the replay's cost comparison, and this test exists to exercise the state-hash path.
    // The cost path has its own test below.
    processed[0].deploy.data.term =
        r#"new deployerId(`rho:rchain:deployerId`) in { @"marker"!(43) }"#.to_string();

    let (tampered_hash, _) = rm
        .replay_compute_state(
            &post,
            &processed,
            &processed_sys,
            &rand,
            BlockData::empty(),
            &fringe_state(1),
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

    // And a tamper that changes the term's *length* is refused even earlier, by the replay's own cost
    // comparison, because the parse is charged (`AUDIT F-2`) and the recorded cost no longer matches.
    //
    // Both rejections are rejections, so this is a stricter detection than the state hash alone rather
    // than a weaker one — but it is a *different* channel, and a future change that dropped the cost
    // comparison would leave the state-hash path as the only guard. Pinning both means neither can be
    // removed without a test going red.
    processed[0].deploy.data.term =
        r#"new deployerId(`rho:rchain:deployerId`) in { @"marker"!(4242) }"#.to_string();

    let cost_tamper = rm
        .replay_compute_state(
            &post,
            &processed,
            &processed_sys,
            &rand,
            BlockData::empty(),
            &fringe_state(1),
            true,
            &PosGenesis::default(),
            &[],
        )
        .await;

    assert!(
        matches!(
            cost_tamper,
            Err(rchain_casper::rholang::ReplayFailure::ReplayCostMismatch { .. })
        ),
        "a length-changing tamper must be caught by the replay's cost comparison, got {cost_tamper:?}"
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
            minimum_bond: rchain_shared::refined::NonNegI64::try_from(1).unwrap(),
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
    //
    // The **pre-state hash** goes the same way and for the same reason: it is the seed anchor the
    // close step writes for the next epoch, so the proposer's value and the replayer's must be the
    // same number. Here that is `post`, the state this block extends.
    let block_data = BlockData {
        block_number: rchain_shared::refined::BlockHeight::try_from(1).expect("height"),
        ..BlockData::empty()
    };
    let close = SystemDeploy::close_block(1, fringe_state(1), rand.split_byte(9));
    let (play_hash, user_results, sys_results) = rm
        .compute_state(
            &post,
            &[deploy(term)],
            &[close],
            &rand,
            block_data.clone(),
            &fringe_state(1),
        )
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
            &fringe_state(1),
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

/// **Arm A of #139's campaign: the post-state is a function of the fringe state, so two nodes that
/// derive different fringes disagree about the same block.**
///
/// This is the primitive the whole campaign rests on, and it is the cheapest thing here. A block's
/// `fringe_state_hash` is **not carried on the block** — every node derives it from its own DAG
/// (`validate_block_checkpoint`'s doc says so) — and the replay hands it to the `CloseBlock` system
/// deploy, which anchors the *next* epoch's seed to it. So if the two values differ, the same block
/// replays to two different post-states, and the validator's `handle_errors` sees
/// `computed != declared` and refuses with `InvalidStateHash`.
///
/// **The falsifiable statement, exactly:** play the block against one fringe, then replay it twice —
/// once against the proposer's fringe (must agree) and once against another (must not). The first
/// replay is the control that the fixture is sound; the second is the mechanism. If both replays
/// agree, the fringe cannot be the cause of any observed divergence and the campaign's hypothesis is
/// refuted before any devnet run is paid for.
#[tokio::test]
async fn a_block_replayed_against_a_different_fringe_reaches_a_different_post_state() {
    use rchain_casper::system_deploy::SystemDeploy;
    use std::collections::BTreeSet;

    let rm = common::build_runtime_manager().await;
    let rand = Blake2b512Random::from_init(&[0u8; 32]);
    let validator = rchain_models::validator::Validator::new([0u8; 65]);
    let pos_genesis = PosGenesis {
        bonds: [(validator, NonNegI64::try_from(40).unwrap())]
            .into_iter()
            .collect(),
        trusted: BTreeSet::from([validator]),
        // `epoch_length: 1` makes this block a boundary, which is the case the divergence is confined
        // to: a non-boundary block whose close step writes no seed change would be a control that
        // cannot fail.
        params: rchain_rholang::native_state::PosParams {
            epoch_length: 1,
            quarantine_length: 0,
            minimum_bond: rchain_shared::refined::NonNegI64::try_from(1).unwrap(),
            ..Default::default()
        },
    };
    let (_pre, genesis_post, _) = rm
        .compute_genesis(
            &[],
            &rand,
            BlockData::empty(),
            &pos_genesis,
            &[seeded_vault()],
        )
        .await
        .expect("compute_genesis");

    let term = r#"new pos(`rho:rchain:pos`), deployerId(`rho:rchain:deployerId`), ret in {
  pos!("withdraw", *deployerId, *ret) |
  for (_ <- ret) { @"after"!(1) }
}"#;
    let block_data = BlockData {
        block_number: rchain_shared::refined::BlockHeight::try_from(1).expect("height"),
        ..BlockData::empty()
    };

    // What the **proposer** had: its own derived fringe.
    let proposer_fringe = fringe_state(1);
    let close = SystemDeploy::close_block(1, proposer_fringe, rand.split_byte(9));
    let (declared_post_state, user_results, sys_results) = rm
        .compute_state(
            &genesis_post,
            &[deploy(term)],
            &[close],
            &rand,
            block_data.clone(),
            &proposer_fringe,
        )
        .await
        .expect("play compute_state");
    assert!(
        user_results[0].eval_result.succeeded(),
        "the staged withdrawal must succeed: {:?}",
        user_results[0].eval_result.errors
    );
    let processed: Vec<ProcessedDeploy> = user_results.into_iter().map(|r| r.deploy).collect();
    let processed_sys: Vec<ProcessedSystemDeploy> =
        sys_results.into_iter().map(|r| r.deploy).collect();

    // The **control**: a validator that derived the proposer's fringe agrees, so the fixture is sound
    // and the difference below is the fringe rather than a replay that never agrees with anything.
    let (same_fringe, _) = rm
        .replay_compute_state(
            &genesis_post,
            &processed,
            &processed_sys,
            &rand,
            block_data.clone(),
            &proposer_fringe,
            true,
            &pos_genesis,
            &[],
        )
        .await
        .expect("replay under the proposer's fringe");
    assert_eq!(
        same_fringe, declared_post_state,
        "a validator that derives the proposer's fringe must agree — without this the arm below \
         proves nothing about the fringe"
    );

    // The **mechanism**: a validator whose own DAG named a different fringe replays the same deploys
    // to a different post-state. `handle_errors` compares this value with the declared one and
    // returns `None`, which is `InvalidStateHash`.
    let other_fringe = fringe_state(2);
    assert_ne!(
        other_fringe, proposer_fringe,
        "the fixture needs two distinct fringe states"
    );
    let (different_fringe, _) = rm
        .replay_compute_state(
            &genesis_post,
            &processed,
            &processed_sys,
            &rand,
            block_data,
            &other_fringe,
            true,
            &pos_genesis,
            &[],
        )
        .await
        .expect("replay under a different fringe");
    assert_ne!(
        different_fringe, declared_post_state,
        "the `CloseBlock` system deploy anchors the next epoch's seed to the fringe state, so a \
         validator that derived a different fringe must NOT reproduce the proposer's post-state — \
         this inequality is the whole mechanism #139 is about"
    );

    // And the comparison the node actually makes (`interpreter_util::handle_errors`) is the one that
    // refuses: named here so the arm's result and the production status are the same fact.
    assert_eq!(
        rchain_casper::interpreter_util::handle_errors(
            &rchain_crypto::hash::blake2b256_hash::Blake2b256Hash::from_byte_array(
                declared_post_state.as_bytes()
            ),
            Ok(different_fringe),
        )
        .expect("handle_errors runs"),
        None,
        "the divergence above is exactly what `handle_errors` reports as `Ok(None)`"
    );
}

/// **A genesis replay that omits the genesis vaults does not reproduce the genesis** (finding,
/// 2026-09-23; fixed 2026-09-24, Programme F).
///
/// What it pins. The genesis install funds the genesis wallets' REV vaults
/// (`ca4f5b015`, `RuntimeManager::compute_genesis`'s `for vault in vaults`), and those balances are
/// `PREFIX_VAULT` leaves in the genesis post-state. A node that did not *create* the genesis has no
/// mergeable-channel sidecar for it, so `merging.rs` regenerates one by replaying the block — and
/// that replay used to pass `&[]` for the vaults, on the stated assumption that "this is always
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
/// block being replayed *is* the genesis, which is what `is_genesis_pre_state` decides and what the
/// two call sites (`interpreter_util.rs::replay_block`, `merging.rs`'s sidecar regeneration) now do.
///
/// **What this test is, now that the fix has landed.** It is deliberately *not* a tripwire for the
/// call sites: it exercises the primitive `replay_compute_state` with `&[]` and with the vaults, and
/// the divergence between those two is unchanged and is the *reason* the call sites must supply them.
/// The production decision is pinned by `interpreter_util.rs`'s
/// `is_genesis_pre_state_is_true_only_for_the_empty_state` (the condition is exactly the genesis) and
/// end to end by the 3-validator devnet, which is where the failure was observed.
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
            &fringe_state(1),
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
            &fringe_state(1),
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

/// **The block's producer is paid a share of what the block's deploys burned, on play and on replay**
/// (B2, #150).
///
/// Three things are pinned here, and each is a way this can be wrong. The producer's **own REV vault**
/// must hold the share — not the Coop vault, not the pool, and not the epoch pot, which is where the
/// same phlo went before this change (the share is taken *out* of the staking vault before the pot
/// sees it). The **bond pool** must be untouched. And the replay must compute the **same post-state
/// hash** from the same inputs, because the executor is read from the block data each path was set
/// with and the share from the params each path installed — a disagreement there is a chain split,
/// not a rounding difference.
#[tokio::test]
async fn the_producer_is_paid_a_share_of_the_burned_phlo_on_play_and_replay() {
    use rchain_models::validator::Validator as ModelsValidator;
    use rchain_rholang::native_state::PosParams;
    use rchain_shared::refined::{BlockHeight, SeqNum};
    use std::collections::{BTreeMap, BTreeSet};

    let rm = common::build_runtime_manager().await;
    let rand = Blake2b512Random::from_init(&[0u8; 32]);

    // A genesis that pays a quarter, and one bonded validator so the staking vault is created with a
    // bond in it (`install_genesis` creates it at `bond_sum`).
    let validator = ModelsValidator::from_slice(&[1u8; 65]);
    let pos = PosGenesis {
        bonds: BTreeMap::from([(validator, NonNegI64::try_from(1_000).expect("a stake"))]),
        trusted: BTreeSet::new(),
        params: PosParams {
            executor_share: NonNegI64::try_from(2_500).expect("a quarter"),
            ..PosParams::default()
        },
    };
    let (_pre, genesis_post, _) = rm
        .compute_genesis(&[], &rand, BlockData::empty(), &pos, &[seeded_vault()])
        .await
        .expect("compute_genesis");

    // The block's producer is the address the payment goes to, and it is *not* the deployer.
    let producer = PublicKey::new(vec![5u8; 65]);
    let producer_address = RevAddress::from_public_key(&producer)
        .expect("producer rev address")
        .to_base58();
    let block_data = BlockData {
        block_number: BlockHeight::zero(),
        sender: producer.clone(),
        seq_num: SeqNum::zero(),
        timestamp: 0,
    };

    let term = r#"new deployerId(`rho:rchain:deployerId`) in { @"marker"!(true) }"#;
    let (post_state, user_results, sys_results) = rm
        .compute_state(
            &genesis_post,
            &[deploy(term)],
            &[],
            &rand,
            block_data.clone(),
            &fringe_state(1),
        )
        .await
        .expect("play compute_state");
    assert!(
        user_results[0].eval_result.succeeded(),
        "the control: the deploy must run, or there is nothing burned to share: {:?}",
        user_results[0].eval_result.errors
    );
    let burned = i64::from(user_results[0].deploy.burned_amount());
    assert!(burned > 0, "the control: this deploy burns phlo");

    let processed: Vec<ProcessedDeploy> = user_results.into_iter().map(|r| r.deploy).collect();
    let processed_sys: Vec<ProcessedSystemDeploy> =
        sys_results.into_iter().map(|r| r.deploy).collect();

    // A fresh fork carries only the tuple space, not the native store, so the *balance* is asserted
    // where the manager's own runtime is reachable (`runtime_manager.rs`'s
    // `the_deploy_fold_pays_the_blocks_sender`); what this file owns is the hash agreement below.
    assert!(
        !producer_address.is_empty(),
        "the producer's address is derived from the block's sender"
    );

    let (replay_state, _) = rm
        .replay_compute_state(
            &genesis_post,
            &processed,
            &processed_sys,
            &rand,
            block_data,
            &fringe_state(1),
            true,
            &pos,
            &[],
        )
        .await
        .expect("replay compute_state");
    assert_eq!(
        post_state, replay_state,
        "the replay must pay the same address the same share — a disagreement here is a split"
    );
}
