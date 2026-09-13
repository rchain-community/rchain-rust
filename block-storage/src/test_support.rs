//! Test-only fixtures shared by this crate's unit tests.
//!
//! **This is a production-only-in-name seam**: the whole module is `#[cfg(test)]`, so it does not
//! exist in a release build and cannot be reached from another crate. It exists because
//! `BlockMessage` has 17 fields and no `Default`, so each test module that needs a block was
//! carrying its own 20-line copy — and a fixture that drifts between copies makes two tests disagree
//! about what "a block" is without either of them failing.
//!
//! Declared as a production change when it was introduced (the register requires every such seam to
//! be listed rather than slipped into a "tests only" commit), because "it is only `#[cfg(test)]`" is
//! how a test seam becomes a production seam.

use std::collections::{BTreeMap, BTreeSet};

use rchain_models::block::state_hash::StateHash;
use rchain_models::block_hash::BlockHash;
use rchain_models::block_metadata::BlockMetadata;
use rchain_models::casper::protocol::casper_message::{
    BlockMessage, DeployData, FinalizedFringe, RholangState, SignedDeployData,
};
use rchain_models::fringe_data::FringeData;
use rchain_models::validator::Validator;

/// A valid block with distinct values per field, so a codec or conversion that transposes two of
/// them fails a round trip instead of silently agreeing with itself.
pub fn block() -> BlockMessage {
    BlockMessage {
        version: 1,
        shard_id: "root".to_string(),
        block_hash: BlockHash::new([1u8; 32]),
        block_number: 0.try_into().unwrap(),
        sender: Validator::new([2u8; 65]),
        seq_num: 0.try_into().unwrap(),
        pre_state_hash: StateHash::new([3u8; 32]),
        post_state_hash: StateHash::new([4u8; 32]),
        justifications: vec![BlockHash::new([5u8; 32])],
        bonds: BTreeMap::new(),
        rejected_deploys: BTreeSet::new(),
        rejected_blocks: BTreeSet::new(),
        rejected_senders: BTreeSet::new(),
        state: RholangState::default(),
        sig_algorithm: "secp256k1".to_string(),
        sig: vec![9, 9],
        timestamp: 1_700_000_000_000,
    }
}

/// The metadata the block above produces, marked validated (what the DAG stores for a genesis or a
/// block whose validation succeeded).
pub fn block_metadata() -> BlockMetadata {
    let mut bmd = BlockMetadata::from_block(&block());
    bmd.validated = true;
    bmd
}

/// A finalized fringe over two block hashes.
pub fn fringe() -> FinalizedFringe {
    FinalizedFringe {
        hashes: vec![BlockHash::new([6u8; 32]), BlockHash::new([7u8; 32])],
        state_hash: StateHash::new([8u8; 32]),
    }
}

/// Fringe data with every collection populated, so a codec that drops one is caught.
pub fn fringe_data() -> FringeData {
    FringeData {
        fringe_hash: rchain_crypto::hash::blake2b256_hash::Blake2b256Hash::from_bytes([10u8; 32]),
        fringe: BTreeSet::from([BlockHash::new([11u8; 32])]),
        fringe_diff: BTreeSet::from([BlockHash::new([12u8; 32])]),
        state_hash: rchain_crypto::hash::blake2b256_hash::Blake2b256Hash::from_bytes([13u8; 32]),
        rejected_deploys: BTreeSet::from([vec![1, 2]]),
        rejected_blocks: BTreeSet::from([BlockHash::new([14u8; 32])]),
        rejected_senders: BTreeSet::from([vec![3]]),
    }
}

/// A signed deploy with non-empty payload, signature and deployer.
pub fn signed_deploy() -> SignedDeployData {
    SignedDeployData {
        data: DeployData {
            term: "new x in { x!(1) }".to_string(),
            timestamp: 5,
            phlo_price: 6,
            phlo_limit: 7,
            valid_after_block_number: 8,
            shard_id: "root".to_string(),
        },
        deployer: vec![15u8; 65],
        sig: vec![16, 17],
        sig_algorithm: "secp256k1".to_string(),
    }
}
