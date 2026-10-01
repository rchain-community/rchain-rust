//! The restoring rule for a view-dependent failure (AUDIT C173, issue #125, law 53a).
//!
//! Law 53a is `Terminal`: `mark_failed` writes a record, no rule clears it, and the rules then refuse
//! every block above it — so one attributable failure estranges a node from its sender for good. The
//! law states its own falsifier: *"the falsifier is a rule that restores, and the port has none."*
//! `multi_parent_casper::validate` now has one, and these tests drive the real `validate` to check
//! **what makes it a rule rather than a retry loop**:
//!
//! - it is **keyed on the cause** — only a `Divergence` is re-validated, never an `Attributable`
//!   failure and never a `Cascade`;
//! - it is **capped per record**, by a count that is persisted;
//! - it is **budgeted per incoming block**, so the work one block can provoke is a constant.
//!
//! The *clearing* branch — a revalidation that passes, and the child then being accepted rather than
//! refused — needs a genuinely valid block chain to observe, and is covered at the level where it is
//! exact: `multi_parent_casper::restore_tests` asserts the record transition, and the two-node devnet
//! in #125's thread is the end-to-end measurement. What is here is the bound, because the bound is
//! what the register's C180 class is about.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use rchain_block_storage::block_store::BlockStore;
use rchain_block_storage::dag::codecs::{BlockHashCodec, BlockMessageCodec};
use rchain_block_storage::dag::dag_storage::{BlockDagStorage, DeployId};
use rchain_block_storage::dag::message_state::DagMessageState;
use rchain_block_storage::dag::representation::DagRepresentation;
use rchain_models::block_hash::BlockHash;
use rchain_models::block_metadata::{BlockMetadata, FailureCause};
use rchain_models::casper::protocol::casper_message::{
    BlockMessage, RholangState, SignedDeployData,
};
use rchain_models::validator::Validator;
use rchain_shared::log::{Log, NopLog};
use rchain_shared::refined::{BlockHeight, SeqNum};
use rchain_shared::store::{InMemoryKeyValueStore, KeyValueStore};
use rchain_shared::typed_store::KeyValueTypedStoreCodec;

use rchain_casper::multi_parent_casper::{
    validate, RESTORE_ATTEMPT_LIMIT, RESTORE_BUDGET_PER_BLOCK,
};

/// A DAG storage that records what `validate` writes, so the restoring rule's effect is observable.
///
/// The metadata map is the DAG's own view of failed blocks; `inserts` is the trace of what the rule
/// did about them. Everything else is a stub — these tests are about *which* records get re-validated
/// and how often, not about the checks' verdicts, which the revalidation is expected to fail here.
struct RecordingDag {
    metadata: Mutex<BTreeMap<BlockHash, BlockMetadata>>,
    inserts: Mutex<Vec<BlockMetadata>>,
}

impl RecordingDag {
    fn new(metadata: BTreeMap<BlockHash, BlockMetadata>) -> Arc<Self> {
        Arc::new(RecordingDag {
            metadata: Mutex::new(metadata),
            inserts: Mutex::new(Vec::new()),
        })
    }

    /// The insert trace, keyed by block hash, for the records this DAG's `insert` was handed.
    fn inserts_for(&self, hash: &BlockHash) -> Vec<BlockMetadata> {
        self.inserts
            .lock()
            .expect("inserts")
            .iter()
            .filter(|m| m.block_hash == *hash)
            .cloned()
            .collect()
    }
}

#[async_trait]
impl BlockDagStorage for RecordingDag {
    async fn get_representation(&self) -> Arc<DagRepresentation> {
        Arc::new(DagRepresentation {
            dag_set: Arc::new(BTreeSet::new()),
            child_map: Arc::new(BTreeMap::new()),
            height_map: Arc::new(BTreeMap::new()),
            dag_message_state: DagMessageState::empty(),
            fringe_states: BTreeMap::new(),
        })
    }
    async fn insert(&self, m: BlockMetadata, _b: BlockMessage) -> Result<(), String> {
        self.inserts.lock().expect("inserts").push(m.clone());
        self.metadata
            .lock()
            .expect("metadata")
            .insert(m.block_hash, m);
        Ok(())
    }
    async fn lookup(&self, h: &BlockHash) -> Result<Option<BlockMetadata>, String> {
        Ok(self.metadata.lock().expect("metadata").get(h).cloned())
    }
    async fn lookup_by_deploy_id(&self, _d: &DeployId) -> Result<Option<BlockHash>, String> {
        Ok(None)
    }
    async fn add_deploy(&self, _d: SignedDeployData) -> Result<(), String> {
        Ok(())
    }
    async fn pooled_deploys(&self) -> Result<BTreeMap<DeployId, SignedDeployData>, String> {
        Ok(BTreeMap::new())
    }
    async fn contains_deploy_in_pool(&self, _d: &DeployId) -> Result<bool, String> {
        Ok(false)
    }
}

fn hash(byte: u8) -> BlockHash {
    BlockHash::new([byte; 32])
}

fn validator(byte: u8) -> Validator {
    Validator::new([byte; 65])
}

/// A stored block that the restoring rule would try to re-validate. Its contents do not matter: the
/// revalidation runs the ordinary checks, and this block fails them (it justifies nothing present and
/// carries no state), which is the *failed* revalidation path — the one that must still spend an
/// attempt and be bounded.
fn stored_block(h: BlockHash, sender: Validator) -> BlockMessage {
    BlockMessage {
        version: 1,
        shard_id: "root".to_string(),
        block_hash: h,
        block_number: BlockHeight::try_from(1).unwrap(),
        sender,
        seq_num: SeqNum::zero(),
        pre_state_hash: rchain_models::block::state_hash::StateHash::new([0u8; 32]),
        post_state_hash: rchain_models::block::state_hash::StateHash::new([0u8; 32]),
        justifications: vec![],
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

/// A block's record, with the fields these tests vary.
fn record(h: BlockHash, sender: Validator, cause: FailureCause, attempts: u32) -> BlockMetadata {
    BlockMetadata {
        block_hash: h,
        block_num: BlockHeight::try_from(1).unwrap(),
        sender,
        seq_num: SeqNum::zero(),
        justifications: BTreeSet::new(),
        bonds_map: BTreeMap::new(),
        validated: true,
        validation_failed: true,
        // A `Divergence` is never attributable, which is what the first commit of this unit pins.
        slashable: matches!(cause, FailureCause::Attributable),
        failure_cause: Some(cause),
        restore_attempts: attempts,
        fringe: BTreeSet::new(),
        fringe_state_hash: rchain_models::block::state_hash::StateHash::new([0u8; 32]),
        member_of_fringe: None,
    }
}

/// The child whose arrival triggers the restore: it justifies every hash it is given.
fn child(justifications: Vec<BlockHash>) -> BlockMessage {
    BlockMessage {
        block_number: BlockHeight::try_from(2).unwrap(),
        justifications,
        ..stored_block(hash(0xee), validator(9))
    }
}

/// Drive the real `validate` on `child`, over a DAG holding `records` and a block store holding
/// `stored`, and return the DAG so its insert trace can be read.
async fn drive(
    records: BTreeMap<BlockHash, BlockMetadata>,
    stored: Vec<BlockMessage>,
    justifications: Vec<BlockHash>,
) -> Arc<RecordingDag> {
    let rm = common::build_runtime_manager().await;
    let dag = RecordingDag::new(records);

    let shared: Arc<tokio::sync::Mutex<Box<dyn KeyValueStore + Send + Sync>>> = Arc::new(
        tokio::sync::Mutex::new(Box::new(InMemoryKeyValueStore::default())),
    );
    let store: BlockStore = Arc::new(KeyValueTypedStoreCodec::new(
        shared,
        Arc::new(BlockHashCodec),
        Arc::new(BlockMessageCodec),
    ));
    for b in stored {
        store
            .put(&[(b.block_hash, b)])
            .await
            .expect("store the failed block");
    }

    let log: Arc<dyn Log> = Arc::new(NopLog);
    let b = child(justifications);

    // The index loader is never reached here: the restore runs first, and the stub blocks fail the
    // checks that would need it.
    let index = |_h: BlockHash| async move { Err("no index in this test".to_string()) };

    // The parent bound is off (`0`) — this test is about the restoring rule, and #153's check is
    // exercised by its own tests in `validate.rs`.
    let _ = validate(&*dag, &store, &rm, &b, "root", 0, 0, &index, &log).await;

    dag
}

/// **The rule is keyed on the cause.** Of three failed justifications the child brings, only the
/// `Divergence` is re-validated — an `Attributable` failure is the block's own fault and stays, and a
/// `Cascade` is not about that block at all.
///
/// Falsifier: drop the cause test from `restore_is_warranted` and all three are re-validated.
#[tokio::test]
async fn only_a_divergence_is_revalidated() {
    let diverged = hash(1);
    let attributed = hash(2);
    let cascaded = hash(3);

    let dag = drive(
        BTreeMap::from([
            (
                diverged,
                record(diverged, validator(1), FailureCause::Divergence, 0),
            ),
            (
                attributed,
                record(attributed, validator(2), FailureCause::Attributable, 0),
            ),
            (
                cascaded,
                record(cascaded, validator(3), FailureCause::Cascade, 0),
            ),
        ]),
        vec![
            stored_block(diverged, validator(1)),
            stored_block(attributed, validator(2)),
            stored_block(cascaded, validator(3)),
        ],
        vec![diverged, attributed, cascaded],
    )
    .await;

    assert_eq!(
        dag.inserts_for(&diverged).len(),
        1,
        "the view-dependent refusal is the one the rule exists for"
    );
    assert!(
        dag.inserts_for(&attributed).is_empty(),
        "the block's own fault is permanent, however many children justify it"
    );
    assert!(
        dag.inserts_for(&cascaded).is_empty(),
        "a cascade is not about this block; it clears when its parent's record does"
    );
}

/// **The attempt cap is a cap.** A record that has been re-validated to the limit is not re-validated
/// again — the count is the one persisted on the metadata, so it survives a restart.
///
/// Falsifier: delete the `restore_attempts < RESTORE_ATTEMPT_LIMIT` clause and the exhaustion case
/// inserts again.
#[tokio::test]
async fn a_record_at_the_attempt_limit_is_not_revalidated_again() {
    let spent = hash(1);
    let within_budget = hash(2);

    let dag = drive(
        BTreeMap::from([
            (
                spent,
                record(
                    spent,
                    validator(1),
                    FailureCause::Divergence,
                    RESTORE_ATTEMPT_LIMIT,
                ),
            ),
            (
                within_budget,
                record(
                    within_budget,
                    validator(2),
                    FailureCause::Divergence,
                    RESTORE_ATTEMPT_LIMIT - 1,
                ),
            ),
        ]),
        vec![
            stored_block(spent, validator(1)),
            stored_block(within_budget, validator(2)),
        ],
        vec![spent, within_budget],
    )
    .await;

    assert!(
        dag.inserts_for(&spent).is_empty(),
        "a record at the limit must not be re-validated again"
    );
    let attempts = dag.inserts_for(&within_budget);
    assert_eq!(
        attempts.len(),
        1,
        "one below the limit still gets its last try"
    );
    assert_eq!(
        attempts[0].restore_attempts, RESTORE_ATTEMPT_LIMIT,
        "and the attempt is spent — the record now reads at the limit"
    );
    assert!(
        attempts[0].validation_failed,
        "the revalidation failed, so the refusal stands"
    );
}

/// **The per-block budget bounds the work one incoming block can provoke** — the property that keeps
/// this out of C180's class, work whose cost grows with an input nothing bounds.
///
/// Falsifier: delete the budget and every eligible justification is re-validated, so the count grows
/// with the justification set rather than staying a constant.
#[tokio::test]
async fn one_block_budget_bounds_the_revalidations() {
    // More eligible records than the budget allows, all restorable in principle.
    let hashes: Vec<BlockHash> = (1..=6u8).map(hash).collect();
    let records: BTreeMap<BlockHash, BlockMetadata> = hashes
        .iter()
        .enumerate()
        .map(|(i, h)| {
            (
                *h,
                record(*h, validator(i as u8 + 1), FailureCause::Divergence, 0),
            )
        })
        .collect();
    let stored: Vec<BlockMessage> = hashes
        .iter()
        .enumerate()
        .map(|(i, h)| stored_block(*h, validator(i as u8 + 1)))
        .collect();

    let dag = drive(records, stored, hashes).await;

    let total: usize = dag.inserts.lock().expect("inserts").len();
    assert_eq!(
        total, RESTORE_BUDGET_PER_BLOCK,
        "one child may provoke at most a constant number of revalidations, however many of its \
         justifications carry an eligible record"
    );
}
