//! The merge's **block index**, built against a live runtime (`casper/src/merging.rs`).
//!
//! `BlockIndex::get_block_index` and the `apply` constructors under it read the mergeable-channel
//! sidecar and the block's **pre-state**, so they cannot be pinned from the pure fixtures the file's
//! own test module uses — the classification table in `spec/TEST-COVERAGE.md` called this whole path
//! `harness-bound`, and named the fixture it would need. This is that fixture: the same
//! `common::build_runtime_manager()` the determinism tests use, a block produced by `compute_state`,
//! and the **first** lookup of a block — whose sidecar has never been persisted. That last detail is
//! not incidental: it is exactly the state an LFS-restored or deep-replayed block arrives in, and it
//! is the arm where the index regenerates the sidecar by replaying the block rather than failing.
//!
//! The random seed is derived **from the block** (`BlockRandomSeed`), not chosen by the test, because
//! the regenerate arm replays the block with that same derivation: a test that picked its own seed
//! would produce a block whose replay computes a different state, which is a fixture bug that would
//! look like a merge bug.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use rchain_block_storage::block_store::BlockStore;
use rchain_block_storage::dag::codecs::{BlockHashCodec, BlockMessageCodec};
use rchain_casper::block_random_seed::BlockRandomSeed;
use rchain_casper::merging::{BlockIndex, MergeScope};
use rchain_crypto::hash::blake2b256_hash::Blake2b256Hash;
use rchain_crypto::hash::blake2b512_random::Blake2b512Random;
use rchain_crypto::public_key::PublicKey;
use rchain_models::block::state_hash::StateHash;
use rchain_models::block_hash::BlockHash;
use rchain_models::casper::protocol::casper_message::{
    BlockMessage, DeployData, RholangState, SignedDeployData,
};
use rchain_models::fringe_data::FringeData;
use rchain_models::validator::Validator;
use rchain_rholang::native_state::PosGenesis;
use rchain_rholang::system_processes::BlockData;
use rchain_shared::refined::NonNegI64;
use rchain_shared::store::InMemoryKeyValueStore;
use rchain_shared::typed_store::{KeyValueTypedStoreCodec, SharedStore};

use rchain_casper::genesis::contracts::Vault;
use rchain_rholang::util::rev_address::RevAddress;

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
        deployer: vec![0u8; 65],
        sig: Vec::new(),
        sig_algorithm: "secp256k1".to_string(),
    }
}

/// The deployer's vault: the deploy above is signed-by-nobody and unfunded, so the block's deploy has
/// to be payable — the same seeded vault `casper/tests/determinism.rs` builds.
fn seeded_vault() -> Vault {
    Vault {
        rev_address: RevAddress::from_public_key(&PublicKey::new(vec![0u8; 65]))
            .expect("valid rev address"),
        initial_balance: NonNegI64::try_from(1_000_000_000).unwrap(),
    }
}

/// A block whose hash and sender are fixed and whose *state* is filled in after the deploy is run —
/// the shell exists first because its seed comes from it.
fn block_shell() -> BlockMessage {
    BlockMessage {
        version: 1,
        shard_id: "root".to_string(),
        block_hash: BlockHash::new([0x11; 32]),
        block_number: 1.try_into().expect("height 1"),
        sender: Validator::new([0u8; 65]),
        seq_num: 1.try_into().expect("seq 1"),
        pre_state_hash: StateHash::new([0u8; 32]),
        post_state_hash: StateHash::new([0u8; 32]),
        // Not empty on purpose: the regenerate arm replays **with cost accounting** iff the block has
        // justifications (`merging.rs`'s `with_cost_accounting = !block.justifications.is_empty()`), and
        // the play this block's state came from charged costs. A block with no justifications is the
        // *other* arm's shape — "an equivalent empty block, nothing to replay" — and replaying it here
        // would compute a state hash that differs from the declared one by exactly the charges.
        justifications: vec![BlockHash::new([0x22; 32])],
        bonds: BTreeMap::new(),
        rejected_deploys: BTreeSet::new(),
        rejected_blocks: BTreeSet::new(),
        rejected_senders: BTreeSet::new(),
        state: RholangState::default(),
        sig_algorithm: "secp256k1".to_string(),
        sig: Vec::new(),
        timestamp: 0,
    }
}

/// **The index's first lookup of a block, whose sidecar does not exist yet.** The load fails with
/// "Mergeable store invalid state hash", and the arm that handles it does not give up: it either
/// treats an empty block as trivially empty or replays the block to regenerate the sidecar, then
/// saves it. Everything under that — the deploy-chain index, its event-log index built from the
/// pre-state, and the block index that carries them — is what this asserts.
///
/// The alternative it rules out is the one that matters operationally: an index that *failed* here
/// would make an LFS-restored block unmergeable, which is the class AUDIT C57's neighbourhood is
/// about.
#[tokio::test]
async fn the_block_index_regenerates_a_missing_sidecar_and_indexes_the_block() {
    let rm = common::build_runtime_manager().await;
    let rand = Blake2b512Random::from_init(&[0u8; 32]);

    let (genesis_pre, genesis_post, _) = rm
        .compute_genesis(
            &[],
            &rand,
            BlockData::empty(),
            &PosGenesis::default(),
            &[seeded_vault()],
        )
        .await
        .expect("compute_genesis");

    // The shell first, so the seed the *block* implies is the seed the deploy is run under.
    let mut block = block_shell();
    block.pre_state_hash = StateHash::new(*genesis_post.as_bytes());
    let block_rand = BlockRandomSeed::random_generator_from_block(&block);

    let (post_state, user_results, sys_results) = rm
        .compute_state(
            &genesis_post,
            &[deploy(r#"@"marker"!(1)"#)],
            &[],
            &block_rand,
            BlockData::empty(),
        )
        .await
        .expect("compute_state");
    assert!(
        user_results[0].eval_result.succeeded(),
        "the deploy must succeed or the index has nothing to index: {:?}",
        user_results[0].eval_result.errors
    );

    block.post_state_hash = StateHash::new(*post_state.as_bytes());
    block.state = RholangState {
        deploys: user_results.into_iter().map(|r| r.deploy).collect(),
        system_deploys: sys_results.into_iter().map(|r| r.deploy).collect(),
    };

    let store: BlockStore = Arc::new(KeyValueTypedStoreCodec::new(
        {
            let shared: SharedStore = Arc::new(tokio::sync::Mutex::new(Box::new(
                InMemoryKeyValueStore::default(),
            )));
            shared
        },
        Arc::new(BlockHashCodec),
        Arc::new(BlockMessageCodec),
    ));
    store
        .put(&[(block.block_hash, block.clone())])
        .await
        .expect("put the block");

    let index = BlockIndex::get_block_index(&rm, &store, block.block_hash)
        .await
        .expect("the block index regenerates the sidecar rather than failing");

    assert_eq!(index.block_hash, block.block_hash);
    assert_eq!(
        index.deploy_chains.len(),
        1,
        "one deploy, one chain: {:?}",
        index.deploy_chains.len()
    );
    let chain = &index.deploy_chains[0];
    assert_eq!(
        chain.host_block,
        Blake2b256Hash::from_byte_array(block.block_hash.as_bytes()),
        "the chain's host block"
    );
    assert_eq!(
        chain.pre_state_hash,
        Blake2b256Hash::from_byte_array(genesis_post.as_bytes()),
        "the chain carries the block's own pre-state"
    );
    assert_eq!(
        chain.post_state_hash,
        Blake2b256Hash::from_byte_array(post_state.as_bytes()),
        "…and its post-state"
    );
    assert_eq!(
        chain.deploys_with_cost.len(),
        1,
        "the deploy is indexed with its cost"
    );
    assert!(
        chain.deploys_with_cost.iter().any(|d| d.cost > 0),
        "and the cost is the one the deploy was charged — which is what the merge's rejection cost is
         computed from, so a zeroed charge would under-report every rejection: {:?}",
        chain.deploys_with_cost
    );
    // **What this fixture does not carry**: the deploy here is unsigned (`sig: Vec::new()`, the same
    // shape `casper/tests/determinism.rs` uses), so its id — derived from the signature — is empty. The
    // *cost* half of `DeployIdWithCost` is fully asserted; the id half is a carrier the fixture leaves
    // empty, and a test that read it could not tell one deploy from another here.

    // The genesis pre-state is not the block's, and the index is not confused about that.
    assert_ne!(
        Blake2b256Hash::from_byte_array(genesis_pre.as_bytes()),
        chain.pre_state_hash
    );
}

/// **A merge of one branch reproduces that branch's post-state — native writes included.**
///
/// The end-to-end falsifier for #74. The block carries a cost-accounted deploy, whose pre-charge and
/// refund move REV (`PREFIX_VAULT`) and whose fee moves the PoS vault (`PREFIX_POS`) — native writes
/// with no tuple-space event to carry them. Merging the branch over the genesis must reconstruct the
/// block's own post-state; if the merge applies only the tuple-space `StateChange`s it reconstructs
/// that state **minus** the native leaves, and the hash differs. Before the fix this test fails with
/// a hash mismatch, which *is* the silent state loss #74 reported.
///
/// It exercises the whole path: the play path's native capture, the sidecar, `get_block_index`'s load
/// of it, and `MergeScope::merge`'s application of it.
#[tokio::test]
async fn a_merge_reproduces_a_branchs_post_state_including_its_native_writes() {
    let rm = common::build_runtime_manager().await;
    let rand = Blake2b512Random::from_init(&[0u8; 32]);
    let (_genesis_pre, genesis_post, _) = rm
        .compute_genesis(
            &[],
            &rand,
            BlockData::empty(),
            &PosGenesis::default(),
            &[seeded_vault()],
        )
        .await
        .expect("compute_genesis");

    // The shell first, so the block's own seed, sender and seq_num drive both the play and the
    // sidecar key the index will look it up under.
    let mut block = block_shell();
    block.pre_state_hash = StateHash::new(*genesis_post.as_bytes());
    let block_rand = BlockRandomSeed::random_generator_from_block(&block);
    let (post_state, user_results, sys_results) = rm
        .compute_state(
            &genesis_post,
            &[deploy(r#"@"marker"!(1)"#)],
            &[],
            &block_rand,
            BlockData::from_block(&block),
        )
        .await
        .expect("compute_state");
    block.post_state_hash = StateHash::new(*post_state.as_bytes());
    block.state = RholangState {
        deploys: user_results.into_iter().map(|r| r.deploy).collect(),
        system_deploys: sys_results.into_iter().map(|r| r.deploy).collect(),
    };

    // The play path recorded native writes for this block, under the key the index looks up.
    let recorded = rm
        .load_native_changes(
            post_state.as_bytes(),
            block.sender.as_bytes(),
            i64::from(block.seq_num),
        )
        .await
        .expect("a readable native sidecar");
    assert!(
        recorded.as_ref().is_some_and(|a| !a.is_empty()),
        "a cost-accounted deploy writes native state; the play path must record it (#74)"
    );

    let store: BlockStore = Arc::new(KeyValueTypedStoreCodec::new(
        {
            let shared: SharedStore = Arc::new(tokio::sync::Mutex::new(Box::new(
                InMemoryKeyValueStore::default(),
            )));
            shared
        },
        Arc::new(BlockHashCodec),
        Arc::new(BlockMessageCodec),
    ));
    store
        .put(&[(block.block_hash, block.clone())])
        .await
        .expect("put the block");

    let index = BlockIndex::get_block_index(&rm, &store, block.block_hash)
        .await
        .expect("the block index");
    assert!(
        !index.native_changes.is_empty(),
        "the index must carry the block's native writes for the merge (#74)"
    );

    // Merge the single branch over the genesis: nothing has finalised, so the branch is the whole
    // conflict scope and the base is the genesis.
    let scope = MergeScope {
        final_scope: BTreeSet::new(),
        conflict_scope: BTreeSet::from([block.block_hash]),
    };
    let block_index = {
        let index = index.clone();
        move |h: BlockHash| {
            let index = index.clone();
            async move {
                if h == index.block_hash {
                    Ok(index)
                } else {
                    Err(format!("no index for {h:?}"))
                }
            }
        }
    };
    let (merged, _rejected) = MergeScope::merge(
        &scope,
        Blake2b256Hash::from_byte_array(genesis_post.as_bytes()),
        &BTreeMap::<Blake2b256Hash, FringeData>::new(),
        rm.get_history_repo(),
        &block_index,
        |_| 0,
    )
    .await
    .expect("the merge");

    assert_eq!(
        merged,
        Blake2b256Hash::from_byte_array(post_state.as_bytes()),
        "the merge must reproduce the branch's post-state, its native writes included: a match is the \
         fix for #74, and a mismatch is the state loss it reported"
    );
}
