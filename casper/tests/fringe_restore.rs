//! **Arm B of #139's campaign: the LFS restore shape gives every restored block no fringe, so the
//! node reads a zero fringe state where the proposer read a real one.**
//!
//! Arm A (`determinism.rs`) proved the consequence: the epoch seed is anchored to the fringe state,
//! so a node that derives a *different* fringe replays the same block to a different post-state and
//! `handle_errors` reports `InvalidStateHash`. This arm proves the **input difference** — and it is
//! the decisive one, because it is the only arm whose outcome maps to a fix in this tree.
//!
//! **The claim, at the source.** `populate_dag` (`casper/src/engine/node_syncing.rs`) inserts every
//! **non-genesis** block a node restores as `BlockMetadata::from_block`, and the real `fringe` and
//! `fringe_state_hash` are a *local* recomputation produced by validation — they are not on the wire,
//! so `from_block` leaves them empty and zero. The claim **travels with the block** (Law 66/68;
//! C250's residue, C270): a reader that names the block reads its metadata, so the restore shape is a
//! block whose own `fringe_state_hash` is zero, and `get_pre_state_for_parents` begins from an empty
//! fringe and reads that zero. (The `fringe-data` store this file used to point at retired with the
//! record; the input difference is unchanged, and is now a per-block one.)
//!
//! **Why it has never been seen.** A joiner syncing at *genesis* restores block 0 alone, and the
//! genesis is inserted by `insert_genesis` with the correct fringe. This needs a joiner syncing a
//! chain that is already **mature** — which is what `tools/devnet.sh reset <node>` stages for Arm D'.
//!
//! The test is written against the **real** `BlockDagKeyValueStorage` and the **real** helpers
//! `populate_dag` uses, so it observes the storage the node would have rather than a model of it.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use rchain_block_storage::dag::codecs::{
    BlockHashCodec, BlockMetadataCodec, SignedDeployDataCodec,
};
use rchain_block_storage::dag::dag_storage::BlockDagStorage;
use rchain_block_storage::dag::message_map;
use rchain_casper::block_metadata_store::BlockMetadataStore;
use rchain_models::block_hash::BlockHash;
use rchain_models::block_metadata::BlockMetadata;
use rchain_models::casper::protocol::casper_message::{
    BlockMessage, RholangState, SignedDeployData,
};
use rchain_models::validator::Validator;
use rchain_shared::refined::{BlockHeight, SeqNum};
use rchain_shared::store::{InMemoryKeyValueStore, KeyValueStore};
use rchain_shared::typed_store::{BytesCodec, KeyValueTypedStoreCodec};

type Shared = Arc<tokio::sync::Mutex<Box<dyn KeyValueStore + Send + Sync>>>;

fn in_memory() -> Shared {
    Arc::new(tokio::sync::Mutex::new(Box::new(
        InMemoryKeyValueStore::default(),
    )))
}

/// The storage the node builds, constructed the way `casper/src/dag.rs`'s own tests construct it —
/// the metadata store plus the two deploy stores (the `fringe-data` store retired; Law 66/68, C250's
/// residue, C270), so the per-block claim is written by the production `insert` rather than by hand.
async fn build_storage() -> Arc<rchain_casper::dag::BlockDagKeyValueStorage> {
    let metadata_store = Arc::new(
        BlockMetadataStore::create(Arc::new(KeyValueTypedStoreCodec::new(
            in_memory(),
            Arc::new(BlockHashCodec),
            Arc::new(BlockMetadataCodec),
        )))
        .await
        .expect("metadata store"),
    );
    let deploy_index: Arc<
        dyn rchain_shared::typed_store::KeyValueTypedStore<
            rchain_block_storage::dag::dag_storage::DeployId,
            BlockHash,
        >,
    > = Arc::new(KeyValueTypedStoreCodec::new(
        in_memory(),
        Arc::new(BytesCodec),
        Arc::new(BlockHashCodec),
    ));
    let deploy_store: Arc<
        dyn rchain_shared::typed_store::KeyValueTypedStore<
            rchain_block_storage::dag::dag_storage::DeployId,
            SignedDeployData,
        >,
    > = Arc::new(KeyValueTypedStoreCodec::new(
        in_memory(),
        Arc::new(BytesCodec),
        Arc::new(SignedDeployDataCodec),
    ));
    Arc::new(
        rchain_casper::dag::BlockDagKeyValueStorage::create(
            metadata_store,
            deploy_index,
            deploy_store,
        )
        .await
        .expect("dag storage"),
    )
}

fn hash(byte: u8) -> BlockHash {
    BlockHash::new([byte; 32])
}

fn validator(byte: u8) -> Validator {
    Validator::new([byte; 65])
}

/// A block at `height`, justifying `parent`. Its state hashes are placeholders: what this arm reads is
/// the *metadata* the DAG derives, and the two ways of deriving it are the whole subject.
fn block(h: BlockHash, height: i64, sender: Validator, parent: Option<BlockHash>) -> BlockMessage {
    BlockMessage {
        version: 1,
        shard_id: "root".to_string(),
        block_hash: h,
        block_number: BlockHeight::try_from(height).expect("height"),
        sender,
        seq_num: SeqNum::zero(),
        pre_state_hash: rchain_models::block::state_hash::StateHash::new([byte_of(height); 32]),
        post_state_hash: rchain_models::block::state_hash::StateHash::new(
            [byte_of(height) + 1; 32],
        ),
        justifications: parent.into_iter().collect(),
        bonds: BTreeMap::new(),
        rejected_deploys: BTreeSet::new(),
        rejected_blocks: BTreeSet::new(),
        rejected_senders: BTreeSet::new(),
        state: RholangState::default(),
        sig_algorithm: "secp256k1".to_string(),
        sig: vec![1],
        timestamp: 0,
    }
}

fn byte_of(height: i64) -> u8 {
    u8::try_from(height).unwrap_or(0).wrapping_add(1)
}

/// **The restore shape: a block inserted as `populate_dag` inserts it carries no fringe, and the
/// claim on the block itself is the zero it was built with.**
///
/// This is the input difference #139 is about, observed on the real storage. The old form of this
/// test read the retired `fringe-data` record under `fringe_hash_of(∅)`; that store is gone (Law
/// 66/68; C250's residue, C270), so the three steps read the **block's own metadata** instead — the
/// claim travels with the block now, which is the same value the store's record held:
///
/// 1. `BlockMetadata::from_block` — the constructor `populate_dag` uses — leaves `fringe` empty and
///    `fringe_state_hash` zero, because those are a local recomputation and not block fields.
/// 2. `insert` writes that metadata under the block's hash, and `insert` short-circuits on a known
///    block, so the write is append-only — every restored block's claim is the zero it carried, and
///    there is no shared key for them to overwrite one another under.
/// 3. The message map carries that empty fringe into every `Message`, and `latest_fringe` reads it —
///    so a parent set made of restored blocks names the empty fringe.
///
/// **Observed red** by giving `from_block` a fringe (or by inserting with validation-derived
/// metadata): the metadata the reader names then carries a real fringe and a non-zero state, and the
/// assertions fail. The contrast arm at the end is what makes that concrete rather than asserted.
#[tokio::test]
async fn a_restored_block_carries_no_fringe_and_its_claim_is_the_zero_it_carried() {
    let dag = build_storage().await;

    let genesis = block(hash(1), 0, validator(1), None);
    let child = block(hash(2), 1, validator(2), Some(hash(1)));

    // **Exactly what `populate_dag` does**, in its order: genesis through `insert_genesis` (which
    // gives it the correct fringe), then every later block through `from_block`.
    rchain_block_storage::syntax::insert_genesis(&*dag, genesis.clone())
        .await
        .expect("the restored genesis inserts");
    let restored_metadata = BlockMetadata::from_block(&child);
    dag.insert(restored_metadata.clone(), child.clone())
        .await
        .expect("a restored block inserts");

    // 1. The metadata the restore path derives has no fringe, and a zero fringe state.
    assert!(
        restored_metadata.fringe.is_empty(),
        "the restore shape: `BlockMetadata::from_block` cannot know a block's fringe, because the \
         fringe is not a block field"
    );
    assert_eq!(
        restored_metadata.fringe_state_hash,
        rchain_models::block::state_hash::StateHash::new([0u8; 32]),
        "and its fringe state hash is zero, not the state the proposer anchored"
    );

    // 2. **The claim a reader names is the block's own metadata** — the store's record is gone, so
    //    this is where the old test's "under the empty fringe key" assertion is re-expressed: a
    //    reader naming this block reads the zero it carried, not some keyed value.
    let repr = dag.get_representation().await;
    let claim = dag
        .lookup(&hash(2))
        .await
        .expect("the metadata store answers")
        .expect("the restored block is in the DAG");
    assert!(
        claim.fringe.is_empty(),
        "the block's own claim names the empty fringe — every restored block carries the same empty \
         set, because it is the block's derivation and not a shared store key"
    );
    assert_eq!(
        *claim.fringe_state_hash.as_bytes(),
        [0u8; 32],
        "and the state on it is the zero hash — this is the value the replay would anchor the next \
         epoch's seed to"
    );

    // 3. The message map carries the empty fringe onward, so the *parent set* names it too.
    let message = rchain_casper::dag::message_from_block_metadata(
        &restored_metadata,
        &repr.dag_message_state.msg_map,
    )
    .expect("the restored block's justification is in the message map");
    assert!(
        message.fringe.is_empty(),
        "a message from a restored metadata carries an empty fringe, which is what `latest_fringe` \
         reads: the parent set a validator derives names the empty fringe, not the chain's real one"
    );
    let parents: BTreeSet<_> = [message].into_iter().collect();
    let derived_fringe = message_map::latest_fringe(&repr.dag_message_state.msg_map, &parents);
    assert!(
        derived_fringe.is_empty(),
        "so `latest_fringe` over restored parents returns the empty set — and the claim that \
         follows it is the one above, whose value is zero"
    );
}

/// **The contrast, and the reason the test above is not vacuous.**
///
/// The same block, inserted with the metadata a *validating* node derives — a real fringe and the
/// state the merge produced — carries a **different** claim: a real fringe and a **non-zero** state.
/// So the two paths really do disagree about the same block, which is the `Split` shape: two readers
/// of one block answering different questions.
///
/// **The old form compared *cache keys* (`FringeData::fringe_hash_of` of the two fringes).** That
/// assertion no longer means anything — there is no keyed cache to compare, and the whole point of
/// the new shape is that the two paths write the *same block's* metadata, disagreeing in its
/// contents rather than in which key it lands under. It is re-expressed as the disagreement that
/// survives and is what a reader reads: the block's own `fringe` and `fringe_state_hash`.
///
/// Without this arm, "the restored metadata has no fringe" could be a fact about `from_block` that
/// nothing ever consults.
#[tokio::test]
async fn the_two_paths_disagree_about_the_blocks_own_claim() {
    let dag = build_storage().await;

    let genesis = block(hash(1), 0, validator(1), None);
    let genesis_meta = BlockMetadata::from_block(&genesis);
    dag.insert(genesis_meta, genesis.clone())
        .await
        .expect("genesis inserts");

    let child = block(hash(2), 1, validator(2), Some(hash(1)));

    // The restore path: no fringe, zero state.
    let restored_metadata = BlockMetadata::from_block(&child);
    // The validating path: the merge's own answer. A real fringe set and a real state.
    let validating_fringe: BTreeSet<BlockHash> = [hash(1)].into_iter().collect();
    let validating_state = rchain_models::block::state_hash::StateHash::new([0x9a; 32]);
    let validating_metadata = BlockMetadata {
        fringe: validating_fringe.clone(),
        fringe_state_hash: validating_state,
        ..BlockMetadata::from_block(&child)
    };

    dag.insert(validating_metadata.clone(), child.clone())
        .await
        .expect("a validated block inserts");

    // **The disagreement, read from the block.** The two derivations differ on the claim the block
    // carries — the restore path's empty/zero pair against the validating path's real pair — and a
    // reader names the block, so it reads whichever derivation wrote it. That difference is the
    // defect; a store keyed by the fringe *hid* it behind one merged value.
    assert_ne!(
        restored_metadata.fringe, validating_metadata.fringe,
        "the two paths disagree about the fringe the block is in"
    );
    assert_ne!(
        restored_metadata.fringe_state_hash, validating_metadata.fringe_state_hash,
        "…and about the state it derived for that fringe"
    );
    assert!(
        restored_metadata.fringe.is_empty()
            && restored_metadata.fringe_state_hash.as_bytes() == &[0u8; 32],
        "the restore path's claim is the empty fringe and the zero state"
    );

    // The validating path is the one that landed, so that is what a reader reads back — the claim
    // the block carries, under the block's own hash.
    let claim = dag
        .lookup(&hash(2))
        .await
        .expect("the metadata store answers")
        .expect("the block is in the DAG");
    assert_eq!(
        claim.fringe, validating_fringe,
        "the block's claim is the validating one"
    );
    assert_eq!(
        claim.fringe_state_hash.as_bytes(),
        &[0x9a; 32],
        "and behind it is the state the merge produced, not zero"
    );
}
