//! Node launch (port of `engine/NodeLaunch.scala`).
//!
//! The genesis-from-config helpers and the `apply` mode-dispatch state machine (genesis → syncing →
//! running over the packet stream) are ported here.

use rchain_models::block_hash::BlockHash;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use rchain_block_storage::approved_store::ApprovedStore;
use rchain_block_storage::block_store::BlockStore;
use rchain_block_storage::dag::dag_storage::BlockDagStorage;
use rchain_block_storage::syntax::{insert_genesis, put_approved_block, put_block};
use rchain_comm::peer_node::PeerNode;
use rchain_comm::rp::rp_conf::RPConf;
use rchain_comm::transport::transport_layer::TransportLayer;
use rchain_models::casper::protocol::casper_message::{
    BlockMessage, CasperMessage, FinalizedFringe,
};
use rchain_models::casper::protocol::packet_type_tag::ToPacket;
use rchain_rspace::state::{RSpaceExporter, RSpaceImporter};
use rchain_shared::log::{Log, LogSource};
use rchain_shared::refined::NonNegI64;
use tokio::sync::mpsc;

use crate::blocks::block_retriever::BlockRetriever;
use crate::bonds_parser;
use crate::conf::ShardSpec;
use crate::engine::catchup::{self, CatchupWindow};
use crate::engine::node_running::NodeRunning;
use crate::engine::node_syncing::NodeSyncing;
use crate::genesis::contracts::{ProofOfStake, Registry, Validator};
use crate::genesis::Genesis;
use crate::protocol::casper_message_protocol::FinalizedFringeSerde;
use crate::protocol::comm_util::{CommUtil, ConnectionsCell};
use crate::runtime_manager::RuntimeManager;
use crate::validator_identity::ValidatorIdentity;
use crate::vault_parser;

/// A peer message (port of `NodeLaunch.PeerMessage`).
#[derive(Clone, Debug)]
pub struct PeerMessage {
    pub peer: PeerNode,
    pub message: CasperMessage,
}

/// Create the genesis block from raw config values (port of `NodeLaunch.createGenesisBlock`).
#[allow(clippy::too_many_arguments)]
pub async fn create_genesis_block(
    validator: &ValidatorIdentity,
    shard_id: &str,
    block_number: i64,
    bonds_path: &str,
    autogen_shard_size: i32,
    vaults_path: &str,
    minimum_bond: i64,
    maximum_bond: i64,
    epoch_length: i32,
    quarantine_length: i32,
    number_of_active_validators: i32,
    executor_share: NonNegI64,
    absence_slack: NonNegI64,
    participation_grace: NonNegI64,
    pos_multi_sig_public_keys: &[String],
    pos_multi_sig_quorum: i32,
    pos_vault_pub_key: &str,
    system_contract_pub_key: &str,
    runtime: &RuntimeManager,
) -> Result<BlockMessage, String> {
    // Initial REV vaults.
    let vaults = vault_parser::parse(Path::new(vaults_path))?;

    // Initial validators.
    let bonds = bonds_parser::parse_or_generate(Path::new(bonds_path), autogen_shard_size)?;
    let validators: Vec<Validator> = bonds
        .into_iter()
        .map(|(pk, stake)| Validator { pk, stake })
        .collect();

    // Run the genesis deploys and create the block.
    let genesis = Genesis {
        sender: validator.public_key.clone(),
        shard_id: shard_id.to_string(),
        block_number,
        proof_of_stake: ProofOfStake {
            // The config is where a signed number becomes a protocol parameter, so this is where a
            // negative one is refused rather than carried (deferred item 1d). Not clamped: "no
            // minimum" and "a minimum of -1" are different intentions, and only one of them is
            // expressible here.
            minimum_bond: NonNegI64::try_from(minimum_bond)
                .map_err(|e| format!("casper.genesis.bond-minimum must not be negative: {e}"))?,
            maximum_bond: NonNegI64::try_from(maximum_bond)
                .map_err(|e| format!("casper.genesis.bond-maximum must not be negative: {e}"))?,
            validators,
            epoch_length,
            quarantine_length,
            number_of_active_validators,
            executor_share,
            absence_slack,
            participation_grace,
            pos_multi_sig_public_keys: pos_multi_sig_public_keys.to_vec(),
            pos_multi_sig_quorum,
            pos_vault_pub_key: pos_vault_pub_key.to_string(),
        },
        registry: Registry {
            system_contract_pub_key: system_contract_pub_key.to_string(),
        },
        vaults,
    };

    crate::genesis::create_genesis_block(validator, &genesis, runtime).await
}

/// Create one shard's genesis block from its [`ShardSpec`] (port of
/// `NodeLaunch.createGenesisBlockFromConfig`).
pub async fn create_genesis_block_from_config(
    validator: &ValidatorIdentity,
    spec: &ShardSpec,
    runtime: &RuntimeManager,
) -> Result<BlockMessage, String> {
    let gbd = &spec.genesis_block_data;
    // The block's shard id is the spec's validated *full* id (`{parent-shard-id}/{shard-name}`),
    // matching the proposer (`Proposer::apply`), the block receiver's `check_if_of_interest` and
    // both deploy APIs. Passing the bare `shard_name` here made the genesis block carry an id
    // ("root") that no later block or deploy shares ("/root"), so a genesis block received from a
    // peer was dropped by the receiver's equality check.
    let shard_id = spec.shard_id.to_string();
    // The producer's share, refused rather than clamped like the bond bounds: a share over the whole
    // would pay a producer more than the deploy burned, out of the stake the same vault holds for
    // every other validator.
    let executor_share = NonNegI64::try_from(i64::from(gbd.executor_share))
        .map_err(|e| format!("casper.genesis.executor-share must not be negative: {e}"))?;
    if i64::from(executor_share) > 10_000 {
        return Err(format!(
            "casper.genesis.executor-share is {} basis points, above the whole of what a deploy burns",
            i64::from(executor_share)
        ));
    }
    let absence_slack = NonNegI64::try_from(i64::from(gbd.absence_slack))
        .map_err(|e| format!("casper.genesis.absence-slack must not be negative: {e}"))?;
    // The "absent means the binary rule" reading has already been applied where this `gbd` was parsed
    // (`configuration::hocon`), so this is the value and not a second fallback.
    let participation_grace = NonNegI64::try_from(i64::from(gbd.participation_grace))
        .map_err(|e| format!("casper.genesis.participation-grace must not be negative: {e}"))?;
    create_genesis_block(
        validator,
        &shard_id,
        gbd.genesis_block_number,
        &gbd.bonds_file,
        spec.autogen_shard_size,
        &gbd.wallets_file,
        gbd.bond_minimum,
        gbd.bond_maximum,
        gbd.epoch_length,
        gbd.quarantine_length,
        gbd.number_of_active_validators,
        executor_share,
        absence_slack,
        participation_grace,
        &gbd.pos_multi_sig_public_keys,
        gbd.pos_multi_sig_quorum,
        &gbd.pos_vault_pub_key,
        &gbd.system_contract_pub_key,
        runtime,
    )
    .await
}

/// Wait until at least one peer connection is established (port of `waitForFirstConnection`).
async fn wait_for_first_connection(connections: &ConnectionsCell, log: &dyn Log) {
    let source = LogSource::new("casper.engine.NodeLaunch");
    loop {
        if !connections.read().await.is_empty() {
            return;
        }
        log.debug(source, "Waiting for first connection...");
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// Create, store and broadcast one shard's genesis block (port of
/// `createStoreBroadcastGenesis`).
async fn create_store_broadcast_genesis(
    validator_identity_opt: Option<&ValidatorIdentity>,
    spec: &ShardSpec,
    runtime_manager: &RuntimeManager,
    block_store: &BlockStore,
    approved_store: &ApprovedStore,
    dag: &dyn BlockDagStorage,
    comm_util: &CommUtil,
    log: &dyn Log,
) -> Result<(), String> {
    let source = LogSource::new("casper.engine.NodeLaunch");
    let validator = validator_identity_opt.ok_or_else(|| {
        "To create genesis block node must provide validator private key".to_string()
    })?;

    let genesis_block = create_genesis_block_from_config(validator, spec, runtime_manager).await?;
    log.info(
        source,
        &format!(
            "Sending genesis block {} to peers...",
            genesis_block.block_hash.to_hex()
        ),
    );

    let genesis_fringe = FinalizedFringe {
        hashes: Vec::new(),
        state_hash: genesis_block.pre_state_hash,
        ancestry: Vec::new(),
    };

    put_block(block_store, genesis_block.clone()).await?;
    put_approved_block(approved_store, genesis_fringe.clone()).await?;
    insert_genesis(dag, genesis_block).await?;
    comm_util
        .stream_to_peers(&FinalizedFringeSerde.mk_packet(&genesis_fringe), None)
        .await;
    Ok(())
}

/// The node launch mode dispatch (port of `NodeLaunch.apply`).
#[allow(clippy::too_many_arguments)]
pub async fn apply<I: RSpaceImporter + Send + 'static, E: RSpaceExporter>(
    mut packet_rx: mpsc::Receiver<PeerMessage>,
    incoming_blocks: mpsc::Sender<BlockMessage>,
    spec: ShardSpec,
    trim_state: bool,
    // **An operator-named block to sync to instead of a finalised fringe** (C259). See
    // `CasperConf::sync_anchor`; only consulted when this node's DAG is empty.
    sync_anchor: Option<String>,
    // Operator switch, threaded to `NodeRunning`: when set, this node refuses store-items
    // (state-sync) requests (port of the Scala's `disableStateExporter`).
    disable_state_exporter: bool,
    validator_identity_opt: Option<ValidatorIdentity>,
    standalone: bool,
    transport: Arc<dyn TransportLayer>,
    comm_util: Arc<CommUtil>,
    block_retriever: Arc<BlockRetriever>,
    connections: ConnectionsCell,
    rp_conf: RPConf,
    runtime_manager: Arc<RuntimeManager>,
    block_store: BlockStore,
    approved_store: ApprovedStore,
    dag: Arc<dyn BlockDagStorage>,
    importer: I,
    exporter: E,
    log: Arc<dyn Log>,
) -> Result<(), String> {
    let source = LogSource::new("casper.engine.NodeLaunch");

    let repr = dag.get_representation().await;
    if repr.dag_set.is_empty() && standalone {
        log.info(
            source,
            "Starting as genesis master, creating genesis block...",
        );
        create_store_broadcast_genesis(
            validator_identity_opt.as_ref(),
            &spec,
            runtime_manager.as_ref(),
            &block_store,
            &approved_store,
            dag.as_ref(),
            comm_util.as_ref(),
            log.as_ref(),
        )
        .await?;
    } else if repr.dag_set.is_empty() {
        log.info(source, "Starting from bootstrap node, syncing LFS...");
        let engine = Arc::new(tokio::sync::Mutex::new(NodeSyncing::new(
            transport.clone(),
            rp_conf.clone(),
            block_store.clone(),
            dag.clone(),
            approved_store.clone(),
            comm_util.clone(),
            log.clone(),
            validator_identity_opt.clone(),
            trim_state,
            importer,
        )));
        // **A named anchor replaces the fringe request** (C259). A peer with no finalised fringe answers
        // a `FinalizedFringeRequest` with **genesis** (`node_running.rs`), so a wiped joiner syncs to
        // block 0 and can never re-enter sync — which is the state a chain with frozen finality is in,
        // and the reason this switch exists. The anchor is the block the operator's reconciliation
        // computed: the deepest height a strict supermajority of stake agrees on.
        //
        // Nothing else about the sync changes. `request_blocks` walks from whatever hashes it is given
        // (`lfs_block_requester.rs`), so seeding it with an anchor rather than with a peer's fringe is
        // the same machinery with a different root — and `request_tuple_space` then pulls the state at
        // the anchor's post-state, exactly as it would for a fringe.
        let anchor = match sync_anchor.as_deref() {
            None => None,
            Some(hex) => Some(
                BlockHash::try_from_hex(hex)
                    .map_err(|e| format!("--sync-anchor `{hex}` is not a block hash: {e}"))?,
            ),
        };
        // **The anchor is named *in the request*, not fetched and patched in afterwards** (C259a).
        // Seeding the sync from the anchor block alone left `ancestry` empty, which bypasses exactly the
        // #139 fix that makes a restored block replayable: no block below the anchor then carries the
        // fringe state the joiner cannot derive, the joiner falls back to deriving it, that derivation
        // needs a sidecar the LFS transfer does not send, and the node stalls **at** the anchor —
        // silently near the tip, and loudly on an older one (`validateBlockCheckpoint failed:
        // regenerated mergeable channels…`). The responder now builds the fringe *and its ancestry* from
        // the named block, so a recovery restore carries the same data the ordinary fringe path does.
        //
        // Nothing else about the sync changes. `request_blocks` walks from whatever hashes it is given
        // (`lfs_block_requester.rs`), so seeding it with an anchor rather than with a peer's fringe is
        // the same machinery with a different root — and `request_tuple_space` then pulls the state at
        // the anchor's post-state, exactly as it would for a fringe.
        if let Some(hash) = anchor {
            if rp_conf.bootstrap.is_none() {
                return Err(
                    "--sync-anchor needs a bootstrap to ask for the anchor's fringe, and this node \
                     has none configured"
                        .to_string(),
                );
            }
            log.info(
                source,
                &format!(
                    "Syncing to the operator-named anchor {} rather than to a fringe a peer offers: \
                     with finality frozen no peer has one, and the answer to an ordinary fringe request \
                     would be the genesis block (C259)",
                    hash.to_hex()
                ),
            );
        }
        // With no anchor this *is* the ordinary request; with one, the same request names the root.
        comm_util
            .request_finalized_fringe(trim_state, true, anchor)
            .await
            .map_err(|e| e.to_string())?;

        // Handle packets concurrently with the syncing-finished signal, and with the **terminal**
        // one (AUDIT C181, #125). `finished` is the successful exit — the state was restored, go on
        // to `NodeRunning`. `terminal` is the other one: the sync retries are spent and this node
        // cannot reach the approved state, so it stops serving rather than sitting in `NodeSyncing`
        // for good, which is the state no rule left. Signalling `finished` here instead would move
        // the node into `NodeRunning` on a DAG it never populated, which is what C68 exists to
        // prevent — the two facts are different and get different signals.
        let finished = { engine.lock().await.finished_handle() };
        let terminal = { engine.lock().await.terminal_handle() };
        let handle_loop = async {
            while let Some(pm) = packet_rx.recv().await {
                let mut guard = engine.lock().await;
                if let Err(err) = guard.handle(&pm.peer, &pm.message).await {
                    log.warn(
                        source,
                        &format!("Error handling message from {}: {err}", pm.peer),
                    );
                }
            }
        };
        tokio::select! {
            _ = handle_loop => {}
            _ = finished.notified() => {}
            _ = terminal.notified() => {
                return Err(
                    "LFS state sync is terminal: the approved state could not be restored and the \
                     retries are spent, so this node is stopping rather than serving a chain it \
                     never synced"
                        .to_string(),
                );
            }
        }
    } else {
        log.info(source, "Reconnecting to existing network...");
    }

    // **The catch-up window and the driver that walks it** (C259). A node restored at an anchor below
    // its peers' tip cannot ingest the gap through the hash-keyed pull: the walk runs downward from a
    // peer's tip, each block justifies ones inside the gap, and the receiver's pending set saturates
    // (measured: 173 drops, one block validated, frozen 121 against a master's 577). The window narrows
    // the ingest to a couple of windows' worth while the driver walks outward asking for heights;
    // released — by the driver's every exit path, including its first empty answer — the ingest is
    // exactly what it was before, so a node that never catches up is unaffected.
    let catchup = Arc::new(CatchupWindow::new(catchup::WINDOW_HEIGHTS));
    let (block_range_tx, block_range_rx) = tokio::sync::mpsc::channel(4);

    // Transition to running mode.
    let engine = NodeRunning::new(
        transport.clone(),
        rp_conf.clone(),
        block_store,
        dag.clone(),
        block_retriever,
        log.clone(),
        validator_identity_opt,
        incoming_blocks,
        exporter,
        disable_state_exporter,
        catchup.clone(),
        block_range_tx,
    );
    log.info(source, "Making a transition to Running state.");

    // **Only a node that named an anchor catches up.** An ordinary joiner is level with the fringe it
    // synced to; its first window would come back empty and the driver would release the gate having
    // done nothing, which is a round trip this avoids. If there is no bootstrap there is nobody to
    // walk against, so the driver is not started and the receive end is dropped with it.
    if sync_anchor.is_some() {
        match rp_conf.bootstrap.clone() {
            Some(peer) => {
                let (transport, conf, connections, dag, log) = (
                    transport.clone(),
                    rp_conf.clone(),
                    connections.clone(),
                    dag.clone(),
                    log.clone(),
                );
                tokio::spawn(async move {
                    catchup::run_catchup(
                        transport,
                        conf,
                        connections,
                        dag,
                        log,
                        catchup,
                        peer,
                        block_range_rx,
                    )
                    .await;
                });
            }
            None => {
                log.warn(
                    source,
                    "--sync-anchor was given without a bootstrap: nothing to catch up against",
                );
            }
        }
    }
    wait_for_first_connection(&connections, log.as_ref()).await;
    comm_util.send_fork_choice_tip_request().await;
    while let Some(pm) = packet_rx.recv().await {
        engine.handle(&pm.peer, &pm.message).await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rchain_comm::peer_node::NodeIdentifier;
    use rchain_shared::log::NopLog;
    use rchain_shared::refined::Port;

    fn peer(name: &str) -> PeerNode {
        PeerNode::from(
            NodeIdentifier::new(name.as_bytes().to_vec()),
            "host".to_string(),
            Port::new(40400),
            Port::new(40404),
        )
    }

    fn connections(peers: Vec<PeerNode>) -> ConnectionsCell {
        Arc::new(tokio::sync::RwLock::new(peers))
    }

    /// The wait ends as soon as a peer is there — the check is the first thing the loop does, so an
    /// already-connected node does not spend a poll interval idle before creating its genesis.
    #[tokio::test]
    async fn the_connection_wait_returns_immediately_when_a_peer_exists() {
        let conns = connections(vec![peer("p")]);
        tokio::time::timeout(
            Duration::from_secs(1),
            wait_for_first_connection(&conns, &NopLog),
        )
        .await
        .expect("a connected node must not wait");
    }

    /// With no peers the wait does **not** proceed: it polls, and the caller (`apply`) must not
    /// fall through to genesis creation or to running. The empty cell is never written to, so the
    /// future cannot complete — the timeout is the assertion, not a race.
    #[tokio::test]
    async fn the_connection_wait_blocks_while_there_are_no_peers() {
        let conns = connections(Vec::new());
        let result = tokio::time::timeout(
            // Comfortably beyond one poll interval, so the loop has had several chances to exit.
            Duration::from_millis(900),
            wait_for_first_connection(&conns, &NopLog),
        )
        .await;
        assert!(
            result.is_err(),
            "an unconnected node must keep waiting, not proceed"
        );
    }

    /// A peer arriving *after* the wait started releases it, which is the case the standalone
    /// detection depends on: the node is started, the wait begins, then the first peer connects.
    #[tokio::test]
    async fn the_connection_wait_is_released_by_a_late_peer() {
        let conns = connections(Vec::new());
        let writer = Arc::clone(&conns);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            writer.write().await.push(peer("late"));
        });
        tokio::time::timeout(
            Duration::from_secs(5),
            wait_for_first_connection(&conns, &NopLog),
        )
        .await
        .expect("the late peer must release the wait");
    }
}
