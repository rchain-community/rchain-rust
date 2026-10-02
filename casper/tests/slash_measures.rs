//! **The two consensus-rule measurements the risk plan owed**, run over the *real*
//! `BlockDagKeyValueStorage` rather than a model of it.
//!
//! Both rules landed with unit falsifiers and nothing else, and the plan's own standard for a
//! consensus change is a measurement that drives the rule the way a node drives it. These are those
//! measurements, with one honest limitation stated where it belongs rather than in a footnote: they
//! run **two DAGs in one process**, so what they exercise is the storage, the gate, the signature and
//! the rule; what they do **not** exercise is the wire between the two nodes. A block here reaches the
//! second node by a direct call, not by gossip.
//!
//! - **A2** (`an_equivocation_is_proved_on_a_node_that_never_saw_the_offending_block`): a validator
//!   double-signs, one node's gate refuses the second block and keeps its header, and the *other*
//!   node — which never saw that block — proves the offence from the header alone.
//! - **A1** (`a_local_knob_changes_the_refusal_and_never_the_offence_across_a_store`): the same block
//!   under two `min-phlo-price` settings, taken through the real metadata store to the real slash
//!   rule, so what is measured is that the *stored* record of a strictly-refused block carries no
//!   offence.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use rchain_block_storage::dag::codecs::{
    Blake2b256HashCodec, BlockHashCodec, BlockMetadataCodec, FringeDataCodec, SignedDeployDataCodec,
};
use rchain_block_storage::dag::dag_storage::BlockDagStorage;
use rchain_casper::block_metadata_store::BlockMetadataStore;
use rchain_casper::block_status::BlockStatus;
use rchain_casper::dag::{BlockDagKeyValueStorage, EQUIVOCATION_PREFIX};
use rchain_casper::validate::{equivocation_is_proved, slashable_senders};
use rchain_crypto::hash::blake2b256_hash::Blake2b256Hash;
use rchain_crypto::private_key::PrivateKey;
use rchain_models::block_hash::BlockHash;
use rchain_models::block_metadata::BlockMetadata;
use rchain_models::casper::protocol::casper_message::{BlockMessage, RholangState};
use rchain_models::fringe_data::FringeData;
use rchain_models::validator::Validator;
use rchain_shared::refined::{BlockHeight, SeqNum};
use rchain_shared::store::{InMemoryKeyValueStore, KeyValueStore};
use rchain_shared::typed_store::{BytesCodec as RawBytesCodec, KeyValueTypedStoreCodec};

type Shared = Arc<tokio::sync::Mutex<Box<dyn KeyValueStore + Send + Sync>>>;

fn in_memory() -> Shared {
    Arc::new(tokio::sync::Mutex::new(Box::new(
        InMemoryKeyValueStore::default(),
    )))
}

/// The storage a node builds, constructed the way `casper/src/dag.rs`'s own tests construct it — the
/// same four stores, so everything below goes through the production `insert`.
async fn build_storage() -> Arc<BlockDagKeyValueStorage> {
    let metadata_store = Arc::new(
        BlockMetadataStore::create(Arc::new(KeyValueTypedStoreCodec::new(
            in_memory(),
            Arc::new(BlockHashCodec),
            Arc::new(BlockMetadataCodec),
        )))
        .await
        .expect("metadata store"),
    );
    let fringe_store: Arc<
        dyn rchain_shared::typed_store::KeyValueTypedStore<Blake2b256Hash, FringeData>,
    > = Arc::new(KeyValueTypedStoreCodec::new(
        in_memory(),
        Arc::new(Blake2b256HashCodec),
        Arc::new(FringeDataCodec),
    ));
    let deploy_index: Arc<
        dyn rchain_shared::typed_store::KeyValueTypedStore<
            rchain_block_storage::dag::dag_storage::DeployId,
            BlockHash,
        >,
    > = Arc::new(KeyValueTypedStoreCodec::new(
        in_memory(),
        Arc::new(RawBytesCodec),
        Arc::new(BlockHashCodec),
    ));
    let deploy_store: Arc<
        dyn rchain_shared::typed_store::KeyValueTypedStore<
            rchain_block_storage::dag::dag_storage::DeployId,
            rchain_models::casper::protocol::casper_message::SignedDeployData,
        >,
    > = Arc::new(KeyValueTypedStoreCodec::new(
        in_memory(),
        Arc::new(RawBytesCodec),
        Arc::new(SignedDeployDataCodec),
    ));
    Arc::new(
        BlockDagKeyValueStorage::create(metadata_store, fringe_store, deploy_index, deploy_store)
            .await
            .expect("dag storage"),
    )
}

/// A real secp256k1 identity, fixed by a seed byte — the offender's own key, so the evidence it signs
/// is one this test can verify rather than one it asserts.
fn identity(byte: u8) -> rchain_casper::validator_identity::ValidatorIdentity {
    rchain_casper::validator_identity::ValidatorIdentity::from_private_key(PrivateKey::new(vec![
        byte; 32
    ]))
    .expect("a fixed 32-byte scalar is a valid key")
}

/// A **signed** block by `id` at `seq`, distinguishable from its twin by `marker` — which changes its
/// content hash, so the two are a genuine conflict rather than the same block twice.
fn signed_block(
    id: &rchain_casper::validator_identity::ValidatorIdentity,
    seq: i64,
    marker: u8,
) -> BlockMessage {
    let base = BlockMessage {
        version: 1,
        shard_id: "root".to_string(),
        block_hash: BlockHash::new([0u8; 32]),
        block_number: BlockHeight::try_from(0).expect("a height"),
        sender: Validator::from_slice(id.public_key.bytes()),
        seq_num: SeqNum::try_from(seq).expect("a sequence number"),
        pre_state_hash: rchain_models::block::state_hash::StateHash::new([marker; 32]),
        post_state_hash: rchain_models::block::state_hash::StateHash::new([marker; 32]),
        justifications: Vec::new(),
        bonds: BTreeMap::new(),
        rejected_deploys: BTreeSet::new(),
        rejected_blocks: BTreeSet::new(),
        rejected_senders: BTreeSet::new(),
        state: RholangState::default(),
        sig_algorithm: "secp256k1".to_string(),
        sig: Vec::new(),
        timestamp: 0,
    };
    id.sign_block(&base).expect("signing with a real key")
}

async fn insert(dag: &dyn BlockDagStorage, block: &BlockMessage) -> Result<(), String> {
    dag.insert(BlockMetadata::from_block(block), block.clone())
        .await
}

/// **A2, over two real DAGs: the offence is proved by a node that never saw the offending block.**
///
/// The H-1 gate refuses an equivocating block *before any write*, so no node stores it — and until
/// C200 that was the whole story, which is why the one unambiguous Byzantine fault cost nothing. What
/// the fix adds is the header: the gate keeps it, a proposer attaches it to a slash, and every other
/// node re-checks it against **its own** DAG.
///
/// That last clause is what this measures: `n2` never received the offender's second block — it is
/// refused at `n1`'s gate and never travels in this test — and it still proves the offence from the
/// header alone, because the offender's *first* block is in its own DAG and the header carries the
/// offender's own signature.
#[tokio::test]
async fn an_equivocation_is_proved_on_a_node_that_never_saw_the_offending_block() {
    let n1 = build_storage().await;
    let n2 = build_storage().await;
    let honest = identity(0x11);
    let offender = Validator::from_slice(honest.public_key.bytes());

    // The offender's **first** block: both nodes hold it, which is the state a chain is in.
    let first = signed_block(&honest, 0, 0x01);
    insert(&*n1, &first).await.expect("the first block inserts");
    insert(&*n2, &first).await.expect("both nodes hold it");

    // The offender double-signs. **Only n1 is shown the second block.**
    let second = signed_block(&honest, 0, 0x02);
    assert_ne!(
        first.block_hash, second.block_hash,
        "the control: two distinct blocks, or there is no equivocation to prove"
    );
    let refusal = insert(&*n1, &second)
        .await
        .expect_err("the gate must refuse a second block at one sequence number");
    assert!(
        refusal.starts_with(EQUIVOCATION_PREFIX),
        "and refuse it as an equivocation: {refusal}"
    );

    // n1's gate kept the *header* — the only copy of the offence that will ever exist.
    let recorded = n1.recorded_equivocations().await;
    assert_eq!(
        recorded.len(),
        1,
        "one offence recorded, keyed by its sender"
    );
    let (sender, evidence) = &recorded[0];
    assert_eq!(*sender, offender, "keyed by the validator that equivocated");

    // **And n2, which never saw the second block, proves the offence from that header.**
    assert!(
        equivocation_is_proved(&*n2, &offender, evidence)
            .await
            .expect("the check is total"),
        "a node that never held the offending block must still be able to prove the offence"
    );

    // The controls, each one a way the proof could be too permissive.
    let first_header =
        rchain_models::casper::protocol::casper_message::EquivocationEvidence::from_block(&first)
            .encode();
    assert!(
        !equivocation_is_proved(&*n2, &offender, &first_header)
            .await
            .expect("the check is total"),
        "the block the DAG already holds is not a conflict with itself"
    );
    assert!(
        !equivocation_is_proved(&*n2, &Validator::new([0x33; 65]), evidence)
            .await
            .expect("the check is total"),
        "evidence signed by one key proves nothing about another"
    );
    let lonely = build_storage().await;
    assert!(
        !equivocation_is_proved(&*lonely, &offender, evidence)
            .await
            .expect("the check is total"),
        "and a node whose DAG has no first block has nothing to conflict with"
    );
}

/// **A1, through the real store: a knob changes the refusal and never the offence.**
///
/// This is the C198 claim measured the way the receiving side depends on it. `ContainsLowCostDeploy`
/// reads the *node's own* `casper.min-phlo-price`, so a block at the wrong price is refused by a
/// strict node and accepted by a permissive one — and if that refusal were an offence, the strict
/// node's `Slash` would be a block the permissive node refuses (its `slash_is_unjustified` cannot
/// re-derive the offence), which is a permanent split over a local setting.
///
/// What matters here is that the *stored* record carries no offence: the metadata a strict node writes
/// for such a block must come back out of its store with `slashable` false, so the rule that reads the
/// store finds nobody to slash. (The flag not surviving the proto round trip was itself a defect —
/// C110's class — which is why this goes through the store rather than through the constructor.)
#[tokio::test]
async fn a_local_knob_changes_the_refusal_and_never_the_offence_across_a_store() {
    // A block whose deploy is priced below the strict floor and at the permissive one.
    let mut priced = signed_block(&identity(0x22), 0, 0x01);
    priced.state.deploys = vec![
        rchain_models::casper::protocol::casper_message::ProcessedDeploy {
            deploy: rchain_models::casper::protocol::casper_message::SignedDeployData {
                data: rchain_models::casper::protocol::casper_message::DeployData {
                    term: "Nil".to_string(),
                    timestamp: 0,
                    phlo_price: 3,
                    phlo_limit: 100,
                    valid_after_block_number: 0,
                    shard_id: "root".to_string(),
                    attachments: Vec::new(),
                },
                deployer: vec![0u8; 65],
                sig: vec![],
                sig_algorithm: "secp256k1".to_string(),
            },
            cost: rchain_models::casper::protocol::casper_message::PCost { cost: 0 },
            deploy_log: vec![],
            is_failed: false,
            system_deploy_error: None,
        },
    ];

    let strict = rchain_casper::validate::phlo_price(&priced, 5);
    let permissive = rchain_casper::validate::phlo_price(&priced, 3);
    assert_ne!(
        strict, permissive,
        "the knob really does move the refusal, or there is nothing to measure"
    );
    assert_eq!(strict, BlockStatus::ContainsLowCostDeploy);
    assert_eq!(permissive, BlockStatus::Valid);

    // **Neither verdict is an offence**, and the strict node's stored record says so.
    assert!(
        !strict.is_slashing_offence() && !permissive.is_slashing_offence(),
        "a per-node price floor must not be able to take a bond on either side of it"
    );
    let dag = build_storage().await;
    let mut metadata = BlockMetadata::from_block(&priced);
    metadata.validation_failed = true;
    metadata.slashable = strict.is_slashing_offence();
    metadata.slash_severity = strict
        .slash_severity()
        .unwrap_or(rchain_models::block_metadata::SlashSeverity::Unspecified);
    metadata.failure_cause = Some(strict.failure_cause());
    dag.insert(metadata, priced.clone())
        .await
        .expect("the refused block's record inserts");

    let stored = dag
        .lookup(&priced.block_hash)
        .await
        .expect("the store answers")
        .expect("the record is there");
    assert!(
        !stored.slashable,
        "the stored record of a strictly-refused block carries no offence"
    );
    assert!(
        slashable_senders(&[stored]).is_empty(),
        "so the rule that reads the store finds nobody to slash — which is what keeps the two nodes \
         from splitting over the slashes rather than over the block"
    );
}
