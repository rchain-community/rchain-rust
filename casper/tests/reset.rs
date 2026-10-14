//! The reset's acting half, driven where it is decided (#320, F-U5-01's acting half, register C274).
//!
//! **What this is for.** The engine half was pinned by reading while the rest of the unit was tested:
//! the loop's third exit (`node_launch::apply`'s `select!`) has no harness — only `node_runtime` calls
//! `apply`, and building the fixture it would need (a transport, an importer, an exporter, a
//! `ShardSpec`, a `NodeRunning`) is a project rather than a test. So the arm's *body* was extracted into
//! `engine::reset::act_on_reset_request`, which takes the four things the decision actually reads — the
//! switch, the two stores and the runtime — and this drives *that*.
//!
//! What it therefore pins: the request is consumed exactly once, the switch gates the act, the act
//! rewinds the DAG and purges the bodies it dropped, and the anchor survives both. What it deliberately
//! does not pin: the five lines of `select!` around the call, which are read rather than run — said out
//! loud here rather than left as an implication that the loop is covered.
//!
//! **The anchor is real.** The fixture uses the runtime manager's own root as the anchor block's
//! post-state, so the runtime reset is the ordinary one (`RhoRuntime::reset` on a root the history
//! knows) and not a fabricated hash — which is what makes the whole act run rather than stopping at the
//! first step.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use rchain_block_storage::block_store::BlockStore;
use rchain_block_storage::dag::codecs::{
    BlockHashCodec, BlockMetadataCodec, SignedDeployDataCodec,
};
use rchain_block_storage::dag::dag_storage::{BlockDagStorage, DeployId, ResetMarker, ResetPhase};
use rchain_casper::block_metadata_store::BlockMetadataStore;
use rchain_casper::dag::BlockDagKeyValueStorage;
use rchain_casper::engine::reset::{act_on_reset_request, resume_after_reset};
use rchain_casper::interpreter_util::{reset_request, take_reset_request, ResetRequest};
use rchain_models::block::state_hash::StateHash;
use rchain_models::block_hash::BlockHash;
use rchain_models::block_metadata::{BlockMetadata, SlashSeverity};
use rchain_models::casper::protocol::casper_message::{
    BlockMessage, RholangState, SignedDeployData,
};
use rchain_models::validator::Validator;
use rchain_shared::log::{LogSource, NopLog};
use rchain_shared::refined::BlockHeight;
use rchain_shared::store::InMemoryKeyValueStore;
use rchain_shared::typed_store::{
    BytesCodec, KeyValueTypedStore, KeyValueTypedStoreCodec, SharedStore,
};

/// Every cell call in this file names one lane (§107's D4): the reset cells are process-wide and
/// keyed by shard, and these tests drive a single shard.
const LANE: &str = "test-shard";

fn hash(byte: u8) -> BlockHash {
    let mut bytes = [0u8; 32];
    bytes[0] = byte;
    BlockHash::new(bytes)
}

/// A block at `hash` whose post-state is `post_state_hash` — so a fixture can make the anchor carry
/// the runtime's *own* root, which is what lets the reset's last step run instead of failing at it.
fn block(hash: BlockHash, post_state_hash: StateHash) -> BlockMessage {
    BlockMessage {
        version: 1,
        shard_id: "/root".to_string(),
        block_hash: hash,
        block_number: BlockHeight::zero(),
        sender: Validator::new([0u8; 65]),
        seq_num: 0.try_into().unwrap(),
        pre_state_hash: StateHash::new([0u8; 32]),
        post_state_hash,
        justifications: vec![],
        bonds: BTreeMap::new(),
        rejected_deploys: BTreeSet::new(),
        rejected_blocks: BTreeSet::new(),
        rejected_senders: BTreeSet::new(),
        state: RholangState::default(),
        sig_algorithm: "secp256k1".to_string(),
        sig: vec![],
        timestamp: 0,
    }
}

fn meta(hash: BlockHash, parents: &[BlockHash], block_num: i64, seq: i64) -> BlockMetadata {
    BlockMetadata {
        block_hash: hash,
        block_num: BlockHeight::try_from(block_num).unwrap(),
        sender: Validator::new([0u8; 65]),
        seq_num: seq.try_into().unwrap(),
        justifications: parents.iter().copied().collect(),
        bonds_map: BTreeMap::new(),
        validated: true,
        validation_failed: false,
        slashable: false,
        failure_cause: None,
        slash_severity: SlashSeverity::Unspecified,
        restore_attempts: 0,
        fringe: BTreeSet::new(),
        fringe_state_hash: StateHash::new([0u8; 32]),
        member_of_fringe: None,
    }
}

/// An empty DAG over in-memory stores. `node_running.rs`'s and `dag.rs`'s test modules each build the
/// same storage for themselves (the helper is `#[cfg(test)]` and so unreachable across modules); this is
/// the third copy, and the reason the duplication is the recorded practice is in `build_dag`'s own note
/// there: widening it into a production constructor would put a test seam in the engine's API.
async fn build_dag() -> Arc<BlockDagKeyValueStorage> {
    let fresh = || -> SharedStore {
        Arc::new(tokio::sync::Mutex::new(Box::new(
            InMemoryKeyValueStore::default(),
        )))
    };
    let metadata = Arc::new(
        BlockMetadataStore::create(Arc::new(KeyValueTypedStoreCodec::new(
            fresh(),
            Arc::new(BlockHashCodec),
            Arc::new(BlockMetadataCodec),
        )))
        .await
        .expect("metadata store"),
    );
    let deploy_index: Arc<dyn KeyValueTypedStore<DeployId, BlockHash>> = Arc::new(
        KeyValueTypedStoreCodec::new(fresh(), Arc::new(BytesCodec), Arc::new(BlockHashCodec)),
    );
    let deploy_store: Arc<dyn KeyValueTypedStore<DeployId, SignedDeployData>> =
        Arc::new(KeyValueTypedStoreCodec::new(
            fresh(),
            Arc::new(BytesCodec),
            Arc::new(SignedDeployDataCodec),
        ));
    // **The reset marker is part of the fixture** (F-U5-01, C274). A reset refuses to run without
    // somewhere to record itself — the whole point of the record is that a crash mid-reset is
    // repairable — so a test that drives one attaches the store, exactly as the node does.
    let reset_marker: Arc<dyn KeyValueTypedStore<Vec<u8>, Vec<u8>>> = Arc::new(
        KeyValueTypedStoreCodec::new(fresh(), Arc::new(BytesCodec), Arc::new(BytesCodec)),
    );
    Arc::new(
        BlockDagKeyValueStorage::create(metadata, deploy_index, deploy_store)
            .await
            .expect("dag storage")
            .with_reset_marker(reset_marker),
    )
}

async fn build_block_store() -> BlockStore {
    let manager = rchain_shared::store_manager::InMemoryStoreManager::default();
    rchain_block_storage::block_store::create(&manager)
        .await
        .expect("block store")
}

async fn has(store: &BlockStore, h: BlockHash) -> bool {
    store
        .contains(&[h])
        .await
        .expect("the store answers")
        .first()
        .copied()
        .unwrap_or(false)
}

/// **The acting half, end to end at the level a test can reach.** One test rather than three, because
/// the reset request is a process-wide cell: the first request wins, so a second test's request would
/// be refused while this one's was pending — the same reason the signal's own contract test is one.
#[tokio::test]
async fn a_reset_request_rewinds_the_stores_when_the_switch_is_on_and_does_nothing_when_it_is_off()
{
    // Whatever a previous run left in the cell: this test owns it from here.
    let _ = take_reset_request(LANE);

    let dag = build_dag().await;
    let block_store = build_block_store().await;
    let manager = Arc::new(common::build_runtime_manager().await);
    // **A root the runtime knows**, so the reset's last step is the ordinary one.
    let known_root = manager.get_history_repo().root();
    let root_state = StateHash::new(known_root.to_byte_array());

    // A chain of four: heights 0..3, the tip above the anchor we will name.
    let mut parents: Vec<BlockHash> = Vec::new();
    for n in 0..4i64 {
        let h = hash(n as u8);
        // The block at height 1 carries the runtime's root; the rest are ordinary.
        let post = if n == 1 {
            root_state
        } else {
            StateHash::new([0u8; 32])
        };
        dag.insert(meta(h, &parents, n, n), block(h, post))
            .await
            .expect("the chain inserts");
        block_store
            .put(&[(h, block(h, post))])
            .await
            .expect("the body is stored");
        parents = vec![h];
    }

    let log: Arc<dyn rchain_shared::log::Log> = Arc::new(NopLog);

    let anchor = hash(1);
    let anchor_height = BlockHeight::try_from(1).unwrap();
    let refused = hash(3);
    let request = ResetRequest {
        anchor,
        anchor_height,
        blocks_above: 2,
        refused,
    };

    // --- the switch off --------------------------------------------------------------------------
    // First half: a node whose operator has not opted in must *not* touch its stores, and must consume
    // the request rather than leaving it pending for ever.
    assert!(
        rchain_casper::interpreter_util::request_reset(LANE.to_string(), request.clone()),
        "the request is raised"
    );
    let off = act_on_reset_request(
        LANE,
        false,
        &(dag.clone() as Arc<dyn BlockDagStorage>),
        &block_store,
        &manager,
        &log,
        LogSource::new("test"),
    )
    .await
    .expect("a gated reset is not an error");
    assert!(off.is_none(), "the switch is off, so nothing acted");
    assert_eq!(
        reset_request(LANE),
        None,
        "and the request was consumed, not left pending"
    );
    assert!(
        has(&block_store, refused).await,
        "the tip's body is still there: nothing was rewound"
    );

    // --- the switch on ---------------------------------------------------------------------------
    assert!(
        rchain_casper::interpreter_util::request_reset(LANE.to_string(), request.clone()),
        "a second request is accepted once the first is consumed"
    );
    let acted = act_on_reset_request(
        LANE,
        true,
        &(dag.clone() as Arc<dyn BlockDagStorage>),
        &block_store,
        &manager,
        &log,
        LogSource::new("test"),
    )
    .await
    .expect("the reset runs over healthy stores and a known root");
    assert_eq!(
        acted.map(|r| r.anchor),
        Some(anchor),
        "the caller is told which anchor to catch up from"
    );

    let repr = dag.get_representation().await;
    assert!(!repr.contains(&refused), "the DAG is at the anchor");
    assert!(
        repr.contains(&anchor),
        "and the anchor survives — it is the state replayed from"
    );
    drop(repr);
    assert!(
        !has(&block_store, refused).await,
        "the dropped body is gone, so it can be neither served nor re-injected"
    );
    assert!(
        has(&block_store, anchor).await,
        "the anchor's own body stays — the reset keeps what it rewinds to"
    );
    assert_eq!(
        reset_request(LANE),
        None,
        "taking the request is not undone by acting on it"
    );

    // --- a failure that destroyed nothing leaves the request pending ---------------------------------
    // **Law 73**: the pre-flight read runs before the take, so a reset whose anchor this node cannot
    // read destroys nothing and leaves the request actionable. Under the old order the take ran first
    // and spent it on a failure that had touched no store. A fresh DAG and block store make the anchor
    // genuinely absent rather than already rewound to.
    let bare_dag = build_dag().await;
    let bare_store = build_block_store().await;
    let pending = ResetRequest {
        anchor: hash(1),
        anchor_height: BlockHeight::try_from(1).unwrap(),
        blocks_above: 0,
        refused: hash(2),
    };
    assert!(
        rchain_casper::interpreter_util::request_reset(LANE.to_string(), pending.clone()),
        "a request is raised again"
    );
    let unacted = act_on_reset_request(
        LANE,
        true,
        &(bare_dag.clone() as Arc<dyn BlockDagStorage>),
        &bare_store,
        &manager,
        &log,
        LogSource::new("test"),
    )
    .await
    .expect("a pre-flight refusal is not an error");
    assert!(
        unacted.is_none(),
        "nothing acted — the anchor could not be read"
    );
    assert_eq!(
        reset_request(LANE),
        Some(pending.clone()),
        "and the request is still pending, not spent on a failure that destroyed nothing"
    );
    assert_eq!(
        take_reset_request(LANE),
        Some(pending),
        "and it is still takeable — the detector's next scan can act on it"
    );
}

/// **The reset's crash window, and the boot guard that closes it** (F-U5-01's owed marker, register
/// C274).
///
/// A reset is two store operations — the DAG's rewind and the removal of the bodies it dropped — and a
/// process that dies between them leaves bodies with no DAG entries. That is not an inert state: the
/// receiver reads such a body as *"stored but not yet validated"* and re-injects it, so the next child
/// of a dropped block re-inserts the dropped block **above the anchor** and undoes the reset. Ordering
/// cannot close the window (the hashes to purge are what the rewind returns) and the reverse order is
/// worse, so the node records what it is doing and this test drives the boot that reads the record.
///
/// **The falsifier, stated.** Without the marker the crash is unrepaired: `resume_after_reset` finds
/// nothing to read and the bodies above the anchor stay in the store, which is what the first two
/// assertions would fail on. The test therefore starts *after* the rewind and *before* the purge —
/// exactly the window — rather than making the crash a fault injection.
///
/// What it does not drive: `node_launch::apply`. The guard is a function of its own for the same reason
/// `act_on_reset_request` is — an `apply`-level fixture needs a transport, an importer, a spec and a
/// `NodeRunning` — so the five lines that call it are read rather than run, said out loud here.
#[tokio::test]
async fn a_crash_after_the_rewind_is_repaired_at_boot_by_the_marker() {
    let dag = build_dag().await;
    let block_store = build_block_store().await;
    let manager = Arc::new(common::build_runtime_manager().await);
    // **A root the runtime knows**, so the guard's *third* effect — the runtime reset (law 72) — is the
    // ordinary one and not a fabricated hash. The anchor carries it, as it does in the act's own test.
    let root_state = StateHash::new(manager.get_history_repo().root().to_byte_array());
    let log: Arc<dyn rchain_shared::log::Log> = Arc::new(NopLog);
    let source = LogSource::new("test");

    // A chain of four, bodies and metadata together — the state a node is in before it resets.
    let mut parents: Vec<BlockHash> = Vec::new();
    for n in 0..4i64 {
        let h = hash(n as u8);
        let post = if n == 1 {
            root_state
        } else {
            StateHash::new([0u8; 32])
        };
        dag.insert(meta(h, &parents, n, n), block(h, post))
            .await
            .expect("the chain inserts");
        block_store
            .put(&[(h, block(h, post))])
            .await
            .expect("the body is stored");
        parents = vec![h];
    }

    let anchor = hash(1);
    let anchor_height = BlockHeight::try_from(1).unwrap();

    // --- the crash --------------------------------------------------------------------------------
    // The reset wrote its intent and rewound the DAG, and the process died before the bodies went. The
    // two calls are `rewind_and_purge` split at its own seam, so this is the window itself and not a
    // reconstruction of it.
    dag.write_reset_marker(ResetMarker {
        anchor,
        height: anchor_height,
        phase: ResetPhase::Rewinding,
    })
    .await
    .expect("the intent is recorded");
    let dropped = dag
        .drop_above(anchor_height)
        .await
        .expect("the rewind runs");
    assert_eq!(dropped.len(), 2, "heights 2 and 3 went from the DAG");
    assert!(
        has(&block_store, hash(2)).await && has(&block_store, hash(3)).await,
        "and their bodies stayed — the crash window the marker exists for"
    );
    assert!(
        !dag.get_representation().await.contains(&hash(3)),
        "the DAG is already at the anchor: the rewind is durable and the purge is not"
    );

    // --- the boot that reads the record -----------------------------------------------------------
    let resumed = resume_after_reset(
        &(dag.clone() as Arc<dyn BlockDagStorage>),
        &block_store,
        &manager,
        &log,
        source,
    )
    .await
    .expect("the guard runs over healthy stores");
    assert_eq!(
        resumed,
        Some(anchor),
        "the guard names the anchor the catch-up walk must start from"
    );
    assert!(
        !has(&block_store, hash(2)).await && !has(&block_store, hash(3)).await,
        "the bodies above the anchor are gone, so the receiver has nothing to re-inject"
    );
    assert!(
        has(&block_store, anchor).await,
        "the anchor's own body stays — the reset keeps what it rewinds to"
    );
    assert!(
        dag.get_representation().await.contains(&anchor),
        "and the anchor survives in the view every dependency check reads"
    );

    // **The phase moved, and that is what stops a second boot from rewinding again.** By the next boot
    // this node may have caught up and produced above the anchor; a guard that re-applied `drop_above`
    // would throw those blocks away. So the completion is recorded, and the second call is a no-op.
    let marker = dag
        .read_reset_marker()
        .await
        .expect("the marker is readable")
        .expect("and still present — the catch-up walk clears it, not the guard");
    assert_eq!(marker.phase, ResetPhase::Rewound);
    assert_eq!((marker.anchor, marker.height), (anchor, anchor_height));

    // A block above the anchor, as a caught-up node would have: the second call must leave it alone.
    let above = hash(9);
    dag.insert(
        meta(above, &[anchor], 2, 9),
        block(above, StateHash::new([0u8; 32])),
    )
    .await
    .expect("the node produces above the anchor again");
    let second = resume_after_reset(
        &(dag.clone() as Arc<dyn BlockDagStorage>),
        &block_store,
        &manager,
        &log,
        source,
    )
    .await
    .expect("the guard runs again");
    assert_eq!(second, Some(anchor), "the anchor is still what it names");
    assert!(
        dag.get_representation().await.contains(&above),
        "and nothing was rewound a second time — the one way this guard could do harm"
    );
}

/// **The ordinary boot is untouched.** A node that has never reset has no marker, so the guard answers
/// `None` and the caller reconnects exactly as every build before this one did. Without this the guard
/// could pass its own test while changing the boot path of every node in the network.
#[tokio::test]
async fn a_node_that_never_reset_is_not_touched_by_the_reset_guard() {
    let dag = build_dag().await;
    let block_store = build_block_store().await;
    let manager = Arc::new(common::build_runtime_manager().await);
    let log: Arc<dyn rchain_shared::log::Log> = Arc::new(NopLog);

    let h = hash(0);
    dag.insert(meta(h, &[], 0, 0), block(h, StateHash::new([0u8; 32])))
        .await
        .expect("the chain inserts");
    block_store
        .put(&[(h, block(h, StateHash::new([0u8; 32])))])
        .await
        .expect("the body is stored");

    let resumed = resume_after_reset(
        &(dag.clone() as Arc<dyn BlockDagStorage>),
        &block_store,
        &manager,
        &log,
        LogSource::new("test"),
    )
    .await
    .expect("the guard runs");
    assert_eq!(resumed, None, "no marker, so the ordinary path");
    assert!(
        has(&block_store, h).await,
        "and nothing was purged: the guard has no reason to touch a healthy store"
    );
    assert_eq!(
        dag.read_reset_marker().await.expect("readable"),
        None,
        "and it wrote nothing"
    );
}

/// **The guard now reaches the anchor's *body*, because the third effect it re-applies is the runtime
/// root** (law 72, `spec/Rchain/Durability.lean`). Before the fix a `Rewinding` guard re-applied two
/// store operations and never read a body, so an anchor whose body was absent from the block store was
/// invisible to it; with the fix the repair is total over the record's effects and the absent body is a
/// refusal. Without the runtime step this test fails with `Ok(Some(anchor))` — the exact state a
/// crash-window repair must not hand to the catch-up walk.
#[tokio::test]
async fn the_guard_refuses_a_rewinding_whose_anchor_body_is_absent() {
    let dag = build_dag().await;
    let block_store = build_block_store().await;
    let manager = Arc::new(common::build_runtime_manager().await);
    let log: Arc<dyn rchain_shared::log::Log> = Arc::new(NopLog);

    let anchor = hash(1);
    let anchor_height = BlockHeight::try_from(1).unwrap();
    // The intent is recorded and nothing above the anchor exists, so the rewind and the purge are
    // no-ops — the only thing that can stop the repair is the missing anchor, which is the point.
    dag.write_reset_marker(ResetMarker {
        anchor,
        height: anchor_height,
        phase: ResetPhase::Rewinding,
    })
    .await
    .expect("the intent is recorded");

    let err = resume_after_reset(
        &(dag.clone() as Arc<dyn BlockDagStorage>),
        &block_store,
        &manager,
        &log,
        LogSource::new("test"),
    )
    .await
    .expect_err("the guard refuses an anchor whose body it cannot read");
    assert!(
        err.contains("is not in this node's block store"),
        "the refusal is the reset's own sentence about the anchor: {err}"
    );
}
