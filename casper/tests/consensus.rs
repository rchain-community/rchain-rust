//! End-to-end consensus-pipeline integration tests (genesis → block → replay).

mod common;

use std::collections::BTreeSet;

use rchain_casper::genesis::contracts::Vault;
use rchain_casper::system_deploy::SystemDeploy;
use rchain_crypto::hash::blake2b512_random::Blake2b512Random;
use rchain_crypto::public_key::PublicKey;
use rchain_models::block::state_hash::StateHash;
use rchain_models::casper::protocol::casper_message::{
    DeployData, ProcessedDeploy, ProcessedSystemDeploy, SignedDeployData,
};
use rchain_models::validator::Validator;
use rchain_rholang::native_state::{PosGenesis, PosParams};
use rchain_rholang::system_processes::BlockData;
use rchain_rholang::util::rev_address::RevAddress;
use rchain_shared::refined::NonNegI64;

use common::build_runtime_manager;

fn fixed_rand() -> Blake2b512Random {
    Blake2b512Random::from_init(&[0u8; 32])
}

/// A minimal signed deploy with the given term (signature verification is deferred to the
/// deploy-acceptance path, so the sig/deployer fields are left empty here).
fn deploy(term: &str) -> SignedDeployData {
    deploy_with_limit(term, 90_000)
}

fn deploy_with_limit(term: &str, limit: i64) -> SignedDeployData {
    SignedDeployData {
        data: DeployData {
            attachments: Vec::new(),
            term: term.to_string(),
            timestamp: 0,
            phlo_price: 1,
            phlo_limit: limit,
            valid_after_block_number: 0,
            shard_id: "root".to_string(),
        },
        deployer: vec![0u8; 32],
        sig: Vec::new(),
        sig_algorithm: "secp256k1".to_string(),
    }
}

#[tokio::test]
async fn genesis_deploy_replay_recomputes_state() {
    let rm = build_runtime_manager().await;
    let rand = fixed_rand();
    let (pre, post, results) = rm
        .compute_genesis(
            &[deploy(r#"@"chan"!(42)"#)],
            &rand,
            BlockData::empty(),
            &PosGenesis::default(),
            &[],
        )
        .await
        .expect("compute_genesis");
    assert_eq!(results.len(), 1);
    assert!(results[0].eval_result.succeeded(), "deploy should succeed");

    // Law 11: replay recomputes the same post-state hash from the recorded log.
    let processed: Vec<ProcessedDeploy> = results.iter().map(|r| r.deploy.clone()).collect();
    let (replay_post, _) = rm
        .replay_compute_state(
            &pre,
            &processed,
            &[],
            &rand,
            BlockData::empty(),
            false,
            &PosGenesis::default(),
            &[],
        )
        .await
        .expect("replay_compute_state");
    assert_eq!(
        post, replay_post,
        "replay must reproduce the play post-state"
    );
}

#[tokio::test]
async fn empty_state_hash_fixed_matches_runtime() {
    let rm = build_runtime_manager().await;
    let hash = rm
        .runtime()
        .empty_state_hash()
        .await
        .expect("empty state hash");
    assert_eq!(
        hash,
        rchain_casper::interpreter_util::empty_state_hash_fixed(),
        "the hard-coded genesis pre-state hash must match the computed empty state"
    );
}

#[tokio::test]
async fn deploy_exceeding_phlo_limit_fails_and_next_runs() {
    let rm = build_runtime_manager().await;
    let rand = fixed_rand();
    let starving = deploy_with_limit(r#"@"chan"!(42)"#, 1);
    let normal = deploy(r#"@"chan2"!(43)"#);

    let (_, _, results) = rm
        .compute_genesis(
            &[starving, normal],
            &rand,
            BlockData::empty(),
            &PosGenesis::default(),
            &[],
        )
        .await
        .expect("compute_genesis");

    assert!(
        results[0].deploy.is_failed,
        "phlo-exhausted deploy must be failed"
    );
    assert!(
        results[0].eval_result.errors.iter().any(|e| matches!(
            e,
            rchain_rholang::errors::RholangError::OutOfPhlogistonsError
        )),
        "failure must be an OutOfPhlogistonsError"
    );

    // The next deploy still runs: the per-deploy phlo `set` resets the balance.
    assert!(
        !results[1].deploy.is_failed,
        "subsequent deploy must succeed"
    );
}

#[tokio::test]
async fn replay_matches_play_for_persistent_and_peek() {
    let rm = build_runtime_manager().await;
    let rand = fixed_rand();
    // A non-trivial deploy: persistent send + peek receive (Law 11 replay must reproduce the play
    // post-state, not just a single trivial send).
    let term = r#"new c in { c!!(42) | for (@x <<- c) { @"out"!(x) } }"#;
    let (pre, post, results) = rm
        .compute_genesis(
            &[deploy(term)],
            &rand,
            BlockData::empty(),
            &PosGenesis::default(),
            &[],
        )
        .await
        .expect("compute_genesis");
    assert!(results[0].eval_result.succeeded(), "deploy should succeed");

    let processed: Vec<ProcessedDeploy> = results.iter().map(|r| r.deploy.clone()).collect();
    let (replay_post, _) = rm
        .replay_compute_state(
            &pre,
            &processed,
            &[],
            &rand,
            BlockData::empty(),
            false,
            &PosGenesis::default(),
            &[],
        )
        .await
        .expect("replay_compute_state");
    assert_eq!(
        post, replay_post,
        "replay must reproduce the play post-state"
    );
}

/// A signed deploy with an explicit 65-byte deployer key (the bond path derives the validator from
/// the deployer id).
fn deploy_with_key(term: &str, deployer: Vec<u8>) -> SignedDeployData {
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
        deployer,
        sig: Vec::new(),
        sig_algorithm: "secp256k1".to_string(),
    }
}

/// The dynamic-validator lifecycle end to end: a trusted observer bonds, its stake enters the pool,
/// it becomes an active validator, and replay reproduces the same post-state.
#[tokio::test]
async fn bond_deploy_updates_the_active_validator_set() {
    let rm = build_runtime_manager().await;
    let rand = fixed_rand();
    let deployer = Validator::new([0u8; 65]);
    let rev_address =
        RevAddress::from_public_key(&PublicKey::new(vec![0u8; 65])).expect("valid rev address");
    let pos_genesis = PosGenesis {
        bonds: std::collections::BTreeMap::new(),
        trusted: BTreeSet::from([deployer]),
        params: PosParams {
            minimum_bond: 1,
            ..PosParams::default()
        },
    };
    let (_pre, post, _) = rm
        .compute_genesis(
            &[],
            &rand,
            BlockData::empty(),
            &pos_genesis,
            &[Vault {
                rev_address,
                initial_balance: NonNegI64::try_from(1_000_000_000).unwrap(),
            }],
        )
        .await
        .expect("compute_genesis");

    let term = r#"new pos(`rho:rchain:pos`), deployerId(`rho:rchain:deployerId`), ret in {
  pos!("bond", [*deployerId, 30, *ret]) |
  for (_ <- ret) { Nil }
}"#;
    // The block's closing system deploy, which `block_creator` appends to every block. It is what
    // makes the bond *active*: the contract's `bond` only joins the pool (`Pos.rhox:355`), and the
    // active set is recomputed inside `closeBlock` (`:546`) — an epoch boundary, which with these
    // permissive parameters every block is. Without it the deploy would pool the stake and leave the
    // validator out of the consensus set.
    let close = SystemDeploy::close_block(1, fixed_rand().split_byte(2));
    let (post_state, user_results, sys_results) = rm
        .compute_state(
            &post,
            &[deploy_with_key(term, vec![0u8; 65])],
            &[close],
            &rand,
            BlockData::empty(),
        )
        .await
        .expect("play compute_state");
    assert!(
        user_results[0].eval_result.succeeded(),
        "bond deploy must succeed: {:?}",
        user_results[0].eval_result.errors
    );

    let post_state_hash = StateHash::from_slice(post_state.as_bytes());
    assert!(
        rm.compute_bonds(&post_state_hash)
            .await
            .unwrap()
            .contains_key(&deployer),
        "the bonded observer is now an active validator"
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
        "replay must reproduce the bond post-state"
    );
}

/// The admission flow as a network performs it: a trustee admits a key in one block, and that key bonds in
/// a later block. This is the flow the testnet could not complete — `trust` reported success and the next
/// block's `bond` answered "Validator is not trusted" — and the single-block test above cannot see it,
/// because there the trustee is both the deployer and the genesis bond.
#[tokio::test]
async fn a_trustee_admits_an_observer_and_it_bonds_in_the_next_block() {
    let rm = build_runtime_manager().await;
    let rand = fixed_rand();
    let trustee = Validator::new([1u8; 65]);
    let newcomer = Validator::new([2u8; 65]);
    let trustee_address =
        RevAddress::from_public_key(&PublicKey::new(vec![1u8; 65])).expect("trustee rev address");
    let newcomer_address =
        RevAddress::from_public_key(&PublicKey::new(vec![2u8; 65])).expect("newcomer rev address");
    let vaults = vec![
        Vault {
            rev_address: trustee_address,
            initial_balance: NonNegI64::try_from(1_000_000).unwrap(),
        },
        Vault {
            rev_address: newcomer_address,
            initial_balance: NonNegI64::try_from(1_000_000).unwrap(),
        },
    ];
    let pos_genesis = PosGenesis {
        bonds: [(trustee, NonNegI64::try_from(1000).unwrap())]
            .into_iter()
            .collect(),
        trusted: BTreeSet::from([trustee]),
        params: PosParams {
            minimum_bond: 1,
            ..PosParams::default()
        },
    };
    let (_pre, genesis_post, _) = rm
        .compute_genesis(&[], &rand, BlockData::empty(), &pos_genesis, &vaults)
        .await
        .expect("compute_genesis");

    // Block 1: the trustee admits the newcomer. Nothing about the newcomer is in the genesis.
    let newcomer_hex = rchain_shared::base16::encode(newcomer.as_bytes());
    let trust_term = format!(
        "new pos(`rho:rchain:pos`), deployerId(`rho:rchain:deployerId`), ret in {{\n  \
         pos!(\"trust\", [*deployerId, \"{newcomer_hex}\".hexToBytes(), *ret]) | for (_ <- ret) {{ Nil }}\n}}"
    );
    let (state1, user1, _) = rm
        .compute_state(
            &genesis_post,
            &[deploy_with_key(&trust_term, vec![1u8; 65])],
            &[SystemDeploy::close_block(1, fixed_rand().split_byte(2))],
            &rand,
            BlockData::empty(),
        )
        .await
        .expect("play block 1");
    assert!(
        user1[0].eval_result.succeeded(),
        "the trust deploy must succeed: {:?}",
        user1[0].eval_result.errors
    );

    // Block 2: the newcomer bonds, in a later block with a pre-state that should carry the trust.
    let bond_term = "new pos(`rho:rchain:pos`), deployerId(`rho:rchain:deployerId`), ret in {\n  \
                     pos!(\"bond\", [*deployerId, 100, *ret]) | for (_ <- ret) { Nil }\n}";
    let (state2, user2, _) = rm
        .compute_state(
            &state1,
            &[deploy_with_key(bond_term, vec![2u8; 65])],
            &[SystemDeploy::close_block(2, fixed_rand().split_byte(3))],
            &rand,
            BlockData::empty(),
        )
        .await
        .expect("play block 2");
    assert!(
        user2[0].eval_result.succeeded(),
        "the bond deploy must succeed: {:?}",
        user2[0].eval_result.errors
    );
    assert!(
        rm.compute_bonds(&StateHash::from_slice(state2.as_bytes()))
            .await
            .unwrap()
            .contains_key(&newcomer),
        "the admitted newcomer is now an active validator"
    );
}
