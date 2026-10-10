//! Node syncing state machine (port of `engine/NodeSyncing.scala`).
//!
//! Drives the Last Finalized State sync (blocks + tuple space) from the bootstrap node: it
//! accepts the finalized fringe from the bootstrap, runs the `LfsBlockRequester` and
//! `LfsTupleSpaceRequester` streams, then populates the DAG from the received blocks.

use rchain_shared::chan;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use rchain_block_storage::approved_store::{ApprovedStore, FINALIZED_FRINGE_KEY};
use rchain_block_storage::block_store::BlockStore;
use rchain_block_storage::dag::dag_storage::BlockDagStorage;
use rchain_block_storage::syntax::insert_genesis;
use rchain_comm::peer_node::PeerNode;
use rchain_comm::rp::rp_conf::RPConf;
use rchain_comm::transport::transport_layer::TransportLayer;
use rchain_crypto::hash::blake2b256_hash::Blake2b256Hash;
use rchain_models::block_hash::BlockHash;
use rchain_models::block_metadata::BlockMetadata;
use rchain_models::casper::protocol::casper_message::{
    BlockFringe, BlockMessage, CasperMessage, FinalizedFringe, StoreItemsMessage,
};
use rchain_rspace::state::RSpaceImporter;
use rchain_shared::log::{Log, LogSource};

use super::lfs_block_requester::request_blocks;
use super::lfs_tuple_space_requester::{
    request_tuple_space, request_tuple_space_roots, REQUEST_TIMEOUT,
};
use crate::protocol::comm_util::CommUtil;
use crate::validator_identity::ValidatorIdentity;

/// The node-syncing engine (port of the `NodeSyncing` class).
pub struct NodeSyncing<I: RSpaceImporter> {
    transport: Arc<dyn TransportLayer>,
    conf: RPConf,
    block_store: BlockStore,
    dag: Arc<dyn BlockDagStorage>,
    approved_store: ApprovedStore,
    comm_util: Arc<CommUtil>,
    log: Arc<dyn Log>,
    log_source: LogSource,
    #[allow(dead_code)] // reserved (Scala stores it; unused in the syncing path)
    validator_id: Option<ValidatorIdentity>,
    #[allow(dead_code)] // reserved (Scala stores it; consumed by the caller)
    trim_state: bool,
    importer: Option<I>,
    incoming_blocks_tx: tokio::sync::mpsc::Sender<BlockMessage>,
    incoming_blocks_rx: Option<tokio::sync::mpsc::Receiver<BlockMessage>>,
    tuple_space_tx: tokio::sync::mpsc::Sender<StoreItemsMessage>,
    tuple_space_rx: Option<tokio::sync::mpsc::Receiver<StoreItemsMessage>>,
    start_requester: bool,
    finished: Arc<tokio::sync::Notify>,
    /// **The shared latest-fringe slot** (AUDIT C181, #125). The retry a failed attempt needs is a
    /// *newer* fringe to sync to, and this is where it comes from: `on_finalized_fringe_message`
    /// writes every bootstrap answer here, so a later one is kept rather than discarded, and the
    /// spawned task reads it between attempts.
    latest_fringe: Arc<tokio::sync::Mutex<Option<FinalizedFringe>>>,
    /// Signalled whenever the slot above is written, so the retry wakes on a new fringe instead of
    /// polling for one.
    fringe_arrived: Arc<tokio::sync::Notify>,
    /// **The terminal action's signal** (AUDIT C181). `finished` means "the state was restored";
    /// this means "the retries are spent and this node cannot be synced" — a state the protocol can
    /// otherwise never leave, which is the defect. It is a signal rather than a `std::process::exit`
    /// so the *caller* owns what to do, and so an exit inside the sync task cannot take a test
    /// binary with it.
    terminal: Arc<tokio::sync::Notify>,
}

/// How many times one sync attempt is retried before the node gives up and signals terminal.
///
/// The count is a bound and not a policy: each attempt already gives up on its own after
/// `lfs_block_requester::MAX_IDLE_ROUNDS` idle rounds, so this caps the *retries*, and the total
/// wait is bounded by the product of the two.
pub const MAX_SYNC_ATTEMPTS: u32 = 3;

/// How long a retry waits for a newer fringe before re-attempting the one it has.
///
/// A new fringe is the trigger worth waiting for — a later bootstrap answer names a state nearer the
/// tip — but waiting for one *only* would turn a bootstrap that has stopped answering into the same
/// latch this fixes, one level up.
const SYNC_RETRY_DELAY: Duration = Duration::from_secs(10);

impl<I: RSpaceImporter + Send + 'static> NodeSyncing<I> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        transport: Arc<dyn TransportLayer>,
        conf: RPConf,
        block_store: BlockStore,
        dag: Arc<dyn BlockDagStorage>,
        approved_store: ApprovedStore,
        comm_util: Arc<CommUtil>,
        log: Arc<dyn Log>,
        validator_id: Option<ValidatorIdentity>,
        trim_state: bool,
        importer: I,
    ) -> Self {
        let (incoming_blocks_tx, incoming_blocks_rx) = tokio::sync::mpsc::channel(50);
        let (tuple_space_tx, tuple_space_rx) = tokio::sync::mpsc::channel(50);
        NodeSyncing {
            transport,
            conf,
            block_store,
            dag,
            approved_store,
            comm_util,
            log,
            log_source: LogSource::new("casper.engine.NodeSyncing"),
            validator_id,
            trim_state,
            importer: Some(importer),
            incoming_blocks_tx,
            incoming_blocks_rx: Some(incoming_blocks_rx),
            tuple_space_tx,
            tuple_space_rx: Some(tuple_space_rx),
            start_requester: true,
            finished: Arc::new(tokio::sync::Notify::new()),
            latest_fringe: Arc::new(tokio::sync::Mutex::new(None)),
            fringe_arrived: Arc::new(tokio::sync::Notify::new()),
            terminal: Arc::new(tokio::sync::Notify::new()),
        }
    }

    /// A future that completes when syncing finishes (port of `finished.get`).
    pub async fn wait(&self) {
        self.finished.notified().await;
    }

    /// A cloneable handle to the syncing-finished notification, for waiting concurrently with the
    /// `handle` loop without holding the engine's mutex.
    pub fn finished_handle(&self) -> Arc<tokio::sync::Notify> {
        self.finished.clone()
    }

    /// A cloneable handle to the **terminal** signal: the retries are spent and this node cannot be
    /// synced (AUDIT C181). `node_launch` selects on it beside `finished`, because without it the
    /// node sits in `NodeSyncing` for good with a serving API — the state C68's sequencing was
    /// careful to keep *out* of `NodeRunning` but never gave a way *out of*.
    ///
    /// **Deliberately not `finished`.** That one means "the state was restored", and firing it here
    /// would move the node into `NodeRunning` on a partial DAG, which is the outcome C68 exists to
    /// prevent. The two are different facts and they get different signals.
    pub fn terminal_handle(&self) -> Arc<tokio::sync::Notify> {
        self.terminal.clone()
    }

    /// Handle an incoming casper message (port of `handle`).
    pub async fn handle(&mut self, peer: &PeerNode, msg: &CasperMessage) -> Result<(), String> {
        match msg {
            CasperMessage::FinalizedFringe(fringe) => {
                self.on_finalized_fringe_message(peer, fringe).await
            }
            CasperMessage::StoreItemsMessage(s) => {
                self.log.info(
                    self.log_source,
                    &format!(
                        "Received StoreItems(history: {}, data: {}) from {peer}.",
                        s.history_items.len(),
                        s.data_items.len()
                    ),
                );
                chan::send(&self.tuple_space_tx, s.clone()).await;
                Ok(())
            }
            CasperMessage::BlockMessage(b) => {
                self.log.info(
                    self.log_source,
                    &format!("BlockMessage received #{} from {peer}.", b.block_number),
                );
                chan::send(&self.incoming_blocks_tx, b.clone()).await;
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Handle a finalized-fringe message, starting the LFS sync once from the bootstrap node (port
    /// of `onFinalizedFringeMessage`).
    async fn on_finalized_fringe_message(
        &mut self,
        sender: &PeerNode,
        fringe: &FinalizedFringe,
    ) -> Result<(), String> {
        let sender_is_bootstrap = self
            .conf
            .bootstrap
            .as_ref()
            .map(|b| b == sender)
            .unwrap_or(false);
        if !sender_is_bootstrap {
            self.log.info(
                self.log_source,
                "Fringe message ignored, not received from bootstrap node.",
            );
        }

        // **A fringe with no hashes names no state to sync to, and it must not consume the one-shot
        // trigger** (issue #100). The genesis master broadcasts exactly that — `hashes: Vec::new()`
        // paired with the genesis *pre*-state hash (`node_launch.rs`'s `create_store_broadcast_genesis`)
        // — as an announcement that the approved state is the genesis, not as a sync target. On a fresh
        // network that broadcast can arrive before the answer to our own request: measured on a devnet,
        // it landed 34 ms after the master sent it and 83 ms *before* the master had even seen the
        // request, so the node consumed the trigger on it, "restored" nothing, and discarded the
        // correct answer in silence (a later fringe from the bootstrap logs nothing at all).
        if fringe.hashes.is_empty() {
            // The wording distinguishes the two senders on purpose: only the genesis master's empty
            // fringe is an announcement, and a log that called a stranger's one that would be
            // inventing an authority for it.
            let what = if sender_is_bootstrap {
                "the genesis master's announcement, not an answer to our request"
            } else {
                "an empty fringe, which names no block"
            };
            self.log.info(
                self.log_source,
                &format!(
                    "Ignoring an empty fringe from {sender}: it is {what}. Still waiting for a fringe \
                     that names a block to sync to."
                ),
            );
            return Ok(());
        }

        // **Every bootstrap answer reaches the slot, not only the first** (AUDIT C181, #125). The
        // old code kept the fringe only when it was about to start a request, so a later — and
        // necessarily *newer* — answer was dropped in silence, which is precisely the thing a retry
        // has to have. The slot is written before the start check, so a fringe arriving mid-attempt
        // is already there when the attempt fails and the loop looks for one.
        {
            let mut slot = self.latest_fringe.lock().await;
            let is_new = slot.as_ref() != Some(fringe);
            *slot = Some(fringe.clone());
            drop(slot);
            if is_new {
                self.fringe_arrived.notify_waiters();
            }
        }

        // The task is spawned once; the *retry* lives inside it, because the importer and both
        // receivers are moved into it and nothing downstream can hand them back (AUDIT C181).
        let start = self.start_requester;
        self.start_requester = false;

        // **Every bootstrap fringe is logged, not only the one that starts an attempt.** This line
        // used to live inside the `if start` branch, so the second and later answers produced no log
        // line at all — the repo's own note calls that "a later fringe from the bootstrap logs nothing
        // at all", and it is half of the silence this row is about: an operator cannot see a fringe
        // that was received and dropped (AUDIT C181).
        //
        // **The oracle puts the state hash in the parentheses and the hashes after**
        // (`NodeSyncing.scala`: `s"Received finalized fringe from bootstrap node
        // ($fringeStateHashStr) $fringeHashesStr."`). The port had the hashes in the parentheses and
        // the state hash nowhere — and the state hash is the one field that tells the genesis master's
        // *announcement* from the *answer* to a request, because the announcement carries the genesis
        // **pre**-state and the response the genesis **post**-state. Diagnosing issue #100 meant
        // distinguishing exactly those two, and this line could not show it.
        self.log.info(
            self.log_source,
            &format!(
                "Received finalized fringe from bootstrap node ({}) {}.",
                rchain_shared::base16::encode(fringe.state_hash.as_bytes()),
                fringe
                    .hashes
                    .iter()
                    .map(|h| h.to_hex())
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
        );

        if start {
            // Spawn the LFS sync in the background. Awaiting it here deadlocks: the sync drains
            // `tuple_space_rx`/`incoming_blocks_rx`, which are only fed by `handle` (the
            // StoreItemsMessage/BlockMessage branches) running in this same dispatch loop, which is
            // currently blocked inside this call. Spawning lets `handle` return and keep routing.
            let transport = self.transport.clone();
            let conf = self.conf.clone();
            let block_store = self.block_store.clone();
            let dag = self.dag.clone();
            let approved_store = self.approved_store.clone();
            let comm_util = self.comm_util.clone();
            let log = self.log.clone();
            let finished = self.finished.clone();
            let latest_fringe = self.latest_fringe.clone();
            let fringe_arrived = self.fringe_arrived.clone();
            let terminal = self.terminal.clone();
            let importer = match self.importer.take() {
                Some(importer) => importer,
                None => {
                    self.log.error(
                        self.log_source,
                        "LFS sync requested twice; importer already taken",
                    );
                    return Err("LFS sync already started: importer already taken".to_string());
                }
            };
            let incoming_blocks_rx = match self.incoming_blocks_rx.take() {
                Some(rx) => rx,
                None => {
                    self.log.error(
                        self.log_source,
                        "LFS sync requested twice; incoming-blocks receiver already taken",
                    );
                    return Err(
                        "LFS sync already started: incoming-blocks receiver already taken"
                            .to_string(),
                    );
                }
            };
            let tuple_space_rx = match self.tuple_space_rx.take() {
                Some(rx) => rx,
                None => {
                    self.log.error(
                        self.log_source,
                        "LFS sync requested twice; tuple-space receiver already taken",
                    );
                    return Err(
                        "LFS sync already started: tuple-space receiver already taken".to_string(),
                    );
                }
            };
            // **The retry lives inside the task, because nothing outside it can re-arm the attempt.**
            // The importer and both receivers are moved in here and have no inverse: `handle` holds
            // only the senders, `self` never reaches the task, and `finished` carries no payload.
            // So the loop takes them by `&mut` across attempts instead of re-taking them (AUDIT C181).
            tokio::spawn(async move {
                let source = LogSource::new("casper.engine.NodeSyncing");
                let mut importer = importer;
                let mut incoming_blocks_rx = incoming_blocks_rx;
                let mut tuple_space_rx = tuple_space_rx;
                let mut attempts: u32 = 0;

                loop {
                    // The target is re-read every attempt: a later bootstrap answer is newer, and
                    // syncing to the newest known state is the point of retrying at all.
                    let target = latest_fringe.lock().await.clone();
                    let Some(target) = target else {
                        // Nothing to sync to at all. This is not a failure to signal terminal for:
                        // the slot is written before the task is spawned, so it is unreachable in
                        // practice, and a node with no fringe is one that never received a request.
                        log.error(source, "LFS sync task started with no fringe to sync to");
                        return;
                    };

                    let outcome = run_approved_state_sync(
                        &target,
                        transport.clone(),
                        conf.clone(),
                        block_store.clone(),
                        dag.clone(),
                        comm_util.clone(),
                        log.clone(),
                        &mut importer,
                        &mut incoming_blocks_rx,
                        &mut tuple_space_rx,
                    )
                    .await;

                    match &outcome {
                        Ok(()) => {
                            if let Err(e) = approved_store
                                .put(&[(FINALIZED_FRINGE_KEY, target.clone())])
                                .await
                            {
                                log.error(source, &format!("Failed to store approved block: {e}"));
                            }
                            log.info(source, "LFS state is successfully restored.");
                            notify_when_restored(&outcome, &finished);
                            return;
                        }
                        Err(e) => {
                            attempts += 1;
                            log.error(
                                source,
                                &format!(
                                    "LFS state sync failed (attempt {attempts} of \
                                     {MAX_SYNC_ATTEMPTS}): {e}"
                                ),
                            );
                            if attempts >= MAX_SYNC_ATTEMPTS {
                                // **The terminal state** (AUDIT C181). The retries are spent and this
                                // node cannot reach the approved state, so it stops rather than
                                // serving from a DAG it never populated. The signal is the caller's,
                                // not a `process::exit` here, so the test binary survives and the
                                // node owns the policy.
                                log.error(
                                    source,
                                    "LFS state sync is terminal: no attempt restored the approved \
                                     state and the retries are spent. Signalling shutdown — a node in \
                                     this state cannot serve a chain it never synced.",
                                );
                                terminal.notify_waiters();
                                return;
                            }
                            // Wait for a newer fringe, or a bounded pause, then try again. The two
                            // are both needed: a new fringe is the better target, and waiting only
                            // for one turns a bootstrap that has stopped answering into the same
                            // latch one level up.
                            let _ =
                                tokio::time::timeout(SYNC_RETRY_DELAY, fringe_arrived.notified())
                                    .await;
                        }
                    }
                }
            });
        }
        Ok(())
    }
}

/// Signal the syncing-finished handle **only when the state was actually restored** (AUDIT C68).
///
/// The oracle sequences `finished.complete(())` *after* the sync's drain, so a failed attempt leaves
/// the node in `NodeSyncing` — and its own comment says why the approved block is stored only after
/// the state is received ("to restart requesting if interrupted with incomplete state"). Notifying
/// unconditionally means a failed sync moves the node to `NodeRunning` with an incomplete DAG, which
/// is the one outcome the sequencing exists to prevent.
fn notify_when_restored(outcome: &Result<(), String>, finished: &tokio::sync::Notify) {
    if outcome.is_ok() {
        finished.notify_waiters();
    }
}

/// Download the approved (last finalized) state — blocks + tuple space in parallel — and populate the
/// DAG (port of `requestApprovedState`). Free function so it can be spawned off the dispatch loop.
#[allow(clippy::too_many_arguments)]
async fn run_approved_state_sync<I: RSpaceImporter + Send + 'static>(
    fringe: &FinalizedFringe,
    transport: Arc<dyn TransportLayer>,
    conf: RPConf,
    block_store: BlockStore,
    dag: Arc<dyn BlockDagStorage>,
    comm_util: Arc<CommUtil>,
    log: Arc<dyn Log>,
    // **Borrowed, not moved** (AUDIT C181): the retry loop owns the importer and both receivers and
    // an attempt must leave them usable for the next one. They have no inverse — `handle` holds only
    // the senders — so an attempt that consumed them would latch the node exactly as the old code did.
    importer: &mut I,
    incoming_blocks_rx: &mut tokio::sync::mpsc::Receiver<BlockMessage>,
    tuple_space_rx: &mut tokio::sync::mpsc::Receiver<StoreItemsMessage>,
) -> Result<(), String> {
    let source = LogSource::new("casper.engine.NodeSyncing");
    let block_fut = request_blocks(
        fringe,
        incoming_blocks_rx,
        Duration::from_secs(30),
        &block_store,
        comm_util.as_ref(),
        log.as_ref(),
    );
    let tuple_fut = request_tuple_space(
        fringe,
        tuple_space_rx,
        REQUEST_TIMEOUT,
        transport.as_ref(),
        &conf,
        importer,
        log.as_ref(),
    );

    let (block_st, tuple_res) = tokio::join!(block_fut, tuple_fut);
    // **The tuple leg's state is bound, not dropped** (C271). This used to be
    // `tuple_res.map_err(|e| e.to_string())?;` — the error was propagated and the state thrown away, so
    // a walk that stopped early was indistinguishable from one that completed. Law 69
    // (`spec/Rchain/Sync/Walk.lean`) is a statement about the walk's *exit*, and the consumer's half of
    // it is reading the state that exit returned.
    let tuple_st = tuple_res.map_err(|e| e.to_string())?;
    // A store failure while walking the blocks fails the sync attempt (the oracle's stream fails
    // the same way) rather than leaving a block marked done that was never persisted (AUDIT C65).
    let block_st = block_st.map_err(|e| e.to_string())?;

    // **An unfinished page walk is not a restored state** (C271) — the tuple leg's twin of the block
    // guard below, in the same shape: what was expected, what was found. `request_tuple_space_roots`
    // now refuses its own non-completion exits by name (Law 69), so this is the consumer's independent
    // reading of the same contract, and it documents at the call site what "restored" means: a node that
    // left `NodeSyncing` on anything but a complete state would serve an API over a partial one.
    if !tuple_st.is_finished() {
        return Err(format!(
            "the tuple-space walk finished without completing every page: {} key(s) still \
             outstanding (expected the walk to reach `is_finished` before the state is called \
             restored)",
            tuple_st.outstanding_count()
        ));
    }

    // **A finished walk that received nothing is not a restored state** (issue #100). The block walk
    // starts from the fringe's own hashes, so any fringe that names a block records at least that block
    // — `LfsState::received` writes a `height_map` entry only for a key it actually requested, and a
    // non-empty `latest` cannot empty without one. So an empty map means the fringe we synced to named
    // nothing, and the caller must fail: otherwise the node enters `NodeRunning` on an empty DAG and
    // logs "LFS state is successfully restored." having restored nothing. AUDIT C68's claim that the
    // notify handle fires "only when the state was actually restored" was implemented as `is_ok()`
    // only, and this is the check that makes it true.
    if block_st.height_map.is_empty() {
        return Err(format!(
            "the block walk finished without receiving a single block: the fringe ({} hash(es)) named \
             no state to sync to",
            fringe.hashes.len()
        ));
    }

    // The fringe tuple-space request above only hydrates the finalized-fringe root itself. Casper's
    // read/validation APIs (explore, data-at-name, and mergeable-sidecar regeneration during block
    // indexing) open the pre/post RSpace root of specific downloaded blocks directly, not just the
    // fringe root - request those too, or an observer's local history reader has no root to open for
    // anything but the exact fringe state once restore finishes.
    let fringe_root = Blake2b256Hash::from_byte_array(fringe.state_hash.as_bytes());
    let block_roots = collect_block_state_roots(&block_store, &block_st.height_map).await?;
    let extra_roots: Vec<Blake2b256Hash> = block_roots
        .into_iter()
        .filter(|root| *root != fringe_root)
        .collect();
    if !extra_roots.is_empty() {
        log.info(
            source,
            &format!(
                "Requesting tuple-space data for {} approved block state roots.",
                extra_roots.len()
            ),
        );
        // This walk's returned state is not bound here, and unlike the joined leg above that is safe
        // (C271): `request_tuple_space_roots` now returns `Ok` **only** for a walk that reached
        // `is_finished`, so there is no partial state left to read — the check that mattered is the one
        // inside the walk, and the one above is its twin at the call site.
        request_tuple_space_roots(
            &extra_roots,
            tuple_space_rx,
            REQUEST_TIMEOUT,
            transport.as_ref(),
            &conf,
            importer,
            log.as_ref(),
        )
        .await
        .map_err(|e| e.to_string())?;
    }

    log.info(source, "Rholang state received and saved to store.");
    populate_dag(
        dag.as_ref(),
        &block_store,
        log.as_ref(),
        &block_st.height_map,
        &fringe.ancestry,
    )
    .await?;
    Ok(())
}

/// Collect the pre/post RSpace state roots of every downloaded approved block, so the caller can
/// request tuple-space data for any that aren't the finalized-fringe root already hydrated above.
async fn collect_block_state_roots(
    block_store: &BlockStore,
    height_map: &BTreeMap<i64, BTreeSet<BlockHash>>,
) -> Result<BTreeSet<Blake2b256Hash>, String> {
    let mut roots = BTreeSet::new();

    for hash in height_map.values().flat_map(|s| s.iter().copied()) {
        let block = block_store
            .get(&[hash])
            .await?
            .into_iter()
            .flatten()
            .next()
            .ok_or_else(|| format!("missing block {}", hash.to_hex()))?;
        roots.insert(Blake2b256Hash::from_byte_array(
            block.pre_state_hash.as_bytes(),
        ));
        roots.insert(Blake2b256Hash::from_byte_array(
            block.post_state_hash.as_bytes(),
        ));
    }

    Ok(roots)
}

/// Insert the received blocks into the DAG (port of `populateDag`, minus the Scala `minHeight`
/// filter — the full ancestry chain is now downloaded and must be inserted).
async fn populate_dag(
    dag: &dyn BlockDagStorage,
    block_store: &BlockStore,
    log: &dyn Log,
    height_map: &BTreeMap<i64, BTreeSet<BlockHash>>,
    // **The per-block fringe state, carried on the fringe the sync was started from** (AUDIT C188,
    // #139). Empty when the responder did not send it — an older peer, or a requester that did not
    // ask — in which case every block falls back to the pre-#139 derivation and the log says so per
    // block. That is a degradation, not a failure: the sync still completes.
    ancestry: &[BlockFringe],
) -> Result<(), String> {
    let source = LogSource::new("casper.engine.NodeSyncing");
    log.info(source, "Adding blocks for approved state to DAG.");
    let fringe_of: BTreeMap<BlockHash, &BlockFringe> =
        ancestry.iter().map(|b| (b.block_hash, b)).collect();
    if ancestry.is_empty() {
        log.warn(
            source,
            "The fringe response carried no per-block fringe state (#139): every restored block \
             will be inserted with an empty fringe, and a block whose close deploy anchors the next \
             epoch's seed to the fringe state will not replay as the proposer did it. Ask a peer \
             that knows `includeFringeMetadata`.",
        );
    }

    // Insert blocks in ascending height order (parents before children): `dag.insert` requires each
    // block's justifications to already be present in the message map, and a block's justifications
    // always sit at strictly lower heights (block height = 1 + max justification height). The prior
    // `.reverse()` inserted the newest block first — whose justification was not yet in the map —
    // so LFS sync failed with "justification not present in message map".
    for hash in height_map.values().flat_map(|s| s.iter().copied()) {
        let block = block_store
            .get(&[hash])
            .await?
            .into_iter()
            .flatten()
            .next()
            .ok_or_else(|| format!("missing block {}", hash.to_hex()))?;
        let block_height = i64::from(block.block_number);
        log.info(
            source,
            &format!("Adding #{} {}.", block.block_number, hash.to_hex()),
        );
        if block_height == 0 {
            // Genesis block: insert with validated metadata (fringe empty, fringe_state =
            // pre_state), matching `insert_genesis`, so the validator is bonded and can build on
            // block 0.
            insert_genesis(dag, block).await?;
        } else {
            // **The metadata a validating node would have derived**, when the responder sent it
            // (#139). `from_block` cannot know it: `fringe` and `fringe_state_hash` are a node's own
            // recomputation and are not block fields, so a restored node otherwise carries an empty
            // fringe for every block it restores and replays boundary blocks from a zero seed.
            let bmd = match fringe_of.get(&hash) {
                Some(bf) => BlockMetadata {
                    fringe: bf.fringe.clone(),
                    fringe_state_hash: bf.fringe_state_hash,
                    ..BlockMetadata::from_block(&block)
                },
                None => {
                    log.warn(
                        source,
                        &format!(
                            "No per-block fringe state for {} (#139): inserting it with an empty \
                             fringe, which is the pre-#139 behaviour for this block alone",
                            hash.to_hex()
                        ),
                    );
                    BlockMetadata::from_block(&block)
                }
            };
            dag.insert(bmd, block).await?;
        }
    }

    log.info(source, "Blocks for approved state added to DAG.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use rchain_block_storage::dag::codecs::{
        BlockHashCodec, BlockMessageCodec, BlockMetadataCodec, SignedDeployDataCodec,
    };
    use rchain_block_storage::dag::dag_storage::DeployId;
    use rchain_comm::errors::CommErr;
    use rchain_comm::peer_node::{NodeIdentifier, PeerNode};
    use rchain_comm::rp::rp_conf::{ClearConnectionsConf, RPConf};
    use rchain_comm::transport::chunker::Blob;
    use rchain_comm::transport::transport_layer::TransportLayer;
    use rchain_crypto::hash::blake2b256_hash::Blake2b256Hash;
    use rchain_models::block::state_hash::StateHash;
    use rchain_models::casper::protocol::casper_message::{
        BlockMessage, RholangState, SignedDeployData,
    };
    use rchain_models::comm::protocol::Protocol;
    use rchain_models::validator::Validator;
    use rchain_rspace::state::RSpaceImporter;
    use rchain_shared::log::{Log, LogSource, NopLog};
    use rchain_shared::refined::BlockHeight;
    use rchain_shared::state::TrieImporter;
    use rchain_shared::store::{InMemoryKeyValueStore, KeyValueStore};
    use rchain_shared::typed_store::{BytesCodec, KeyValueTypedStore, KeyValueTypedStoreCodec};
    use std::time::Duration;

    use crate::protocol::comm_util::{CommUtil, ConnectionsCell};
    use rchain_block_storage::approved_store::{ApprovedStore, FINALIZED_FRINGE_KEY};
    use rchain_block_storage::dag::codecs::{ByteCodec, FringeCodec};

    use crate::block_metadata_store::BlockMetadataStore;
    use crate::dag::BlockDagKeyValueStorage;

    type Shared = Arc<tokio::sync::Mutex<Box<dyn KeyValueStore + Send + Sync>>>;

    fn in_memory() -> Shared {
        Arc::new(tokio::sync::Mutex::new(Box::new(
            InMemoryKeyValueStore::default(),
        )))
    }

    fn hash(byte: u8) -> BlockHash {
        let mut bytes = [0u8; 32];
        bytes[0] = byte;
        BlockHash::new(bytes)
    }

    fn chain_block(block_num: i64, justification: Option<BlockHash>) -> BlockMessage {
        let justifications: Vec<BlockHash> = justification.into_iter().collect();
        BlockMessage {
            version: 1,
            shard_id: "root".to_string(),
            block_hash: hash(block_num as u8),
            block_number: BlockHeight::try_from(block_num).unwrap(),
            sender: Validator::new([0u8; 65]),
            seq_num: block_num.try_into().unwrap(),
            pre_state_hash: StateHash::new([0u8; 32]),
            post_state_hash: StateHash::new([0u8; 32]),
            justifications,
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

    async fn build_dag() -> Arc<BlockDagKeyValueStorage> {
        let metadata_store = Arc::new(
            BlockMetadataStore::create(Arc::new(KeyValueTypedStoreCodec::new(
                in_memory(),
                Arc::new(BlockHashCodec),
                Arc::new(BlockMetadataCodec),
            )))
            .await
            .unwrap(),
        );
        let deploy_index: Arc<dyn KeyValueTypedStore<DeployId, BlockHash>> =
            Arc::new(KeyValueTypedStoreCodec::new(
                in_memory(),
                Arc::new(BytesCodec),
                Arc::new(BlockHashCodec),
            ));
        let deploy_store: Arc<dyn KeyValueTypedStore<DeployId, SignedDeployData>> =
            Arc::new(KeyValueTypedStoreCodec::new(
                in_memory(),
                Arc::new(BytesCodec),
                Arc::new(SignedDeployDataCodec),
            ));
        Arc::new(
            BlockDagKeyValueStorage::create(metadata_store, deploy_index, deploy_store)
                .await
                .unwrap(),
        )
    }

    fn peer(name: &str) -> PeerNode {
        PeerNode::from(
            NodeIdentifier::new(name.as_bytes().to_vec()),
            "host".to_string(),
            rchain_shared::refined::Port::new(40400),
            rchain_shared::refined::Port::new(40404),
        )
    }

    /// A logger that keeps its lines: the end-to-end test needs the *evidence that the attempt ran and
    /// failed*, without which "no notification arrived" could pass because nothing happened at all.
    #[derive(Default)]
    struct RecordingLog {
        lines: std::sync::Mutex<Vec<String>>,
    }

    impl RecordingLog {
        fn contains(&self, needle: &str) -> bool {
            self.lines
                .lock()
                .expect("log lock")
                .iter()
                .any(|l| l.contains(needle))
        }
    }

    impl Log for RecordingLog {
        fn is_trace_enabled(&self, _source: LogSource) -> bool {
            false
        }
        fn trace(&self, _source: LogSource, _msg: &str) {}
        fn debug(&self, _source: LogSource, _msg: &str) {}
        fn info(&self, _source: LogSource, msg: &str) {
            self.lines.lock().expect("log lock").push(msg.to_string());
        }
        fn warn(&self, _source: LogSource, msg: &str) {
            self.lines.lock().expect("log lock").push(msg.to_string());
        }
        fn error(&self, _source: LogSource, msg: &str) {
            self.lines.lock().expect("log lock").push(msg.to_string());
        }
    }

    /// A transport that answers nothing: this fixture's failure comes from the stores, not the wire.
    struct SilentTransport;

    #[async_trait]
    impl TransportLayer for SilentTransport {
        async fn send(&self, _peer: &PeerNode, _msg: Protocol) -> CommErr<()> {
            Ok(())
        }
        async fn broadcast(&self, _peers: &[PeerNode], _msg: Protocol) -> Vec<CommErr<()>> {
            Vec::new()
        }
        async fn stream(&self, _peers: &[PeerNode], _blob: Blob) {}
    }

    /// A block store whose reads fail — the block walk's first store touch is a `contains`, so the
    /// attempt fails immediately rather than waiting out the walk's idle timeout.
    struct UnreadableBlockStore {
        inner: BlockStore,
    }

    #[async_trait]
    impl KeyValueTypedStore<BlockHash, BlockMessage> for UnreadableBlockStore {
        async fn get(&self, keys: &[BlockHash]) -> Result<Vec<Option<BlockMessage>>, String> {
            self.inner.get(keys).await
        }
        async fn put(&self, pairs: &[(BlockHash, BlockMessage)]) -> Result<(), String> {
            self.inner.put(pairs).await
        }
        async fn delete(&self, keys: &[BlockHash]) -> Result<usize, String> {
            self.inner.delete(keys).await
        }
        async fn contains(&self, _keys: &[BlockHash]) -> Result<Vec<bool>, String> {
            Err("block store is unreadable".to_string())
        }
        async fn to_map(&self) -> Result<BTreeMap<BlockHash, BlockMessage>, String> {
            self.inner.to_map().await
        }
    }

    /// An importer that refuses to open a root, so the tuple-space leg fails at once instead of
    /// waiting out its 120 s timeout — `set_root` is fallible since AUDIT C67's sibling work.
    struct RefusingImporter;

    impl TrieImporter<Blake2b256Hash> for RefusingImporter {
        fn set_history_items<Value>(
            &mut self,
            _data: &[(Blake2b256Hash, Value)],
            _to_buffer: impl Fn(&Value) -> Vec<u8>,
        ) -> Result<(), String> {
            Ok(())
        }
        fn set_data_items<Value>(
            &mut self,
            _data: &[(Blake2b256Hash, Value)],
            _to_buffer: impl Fn(&Value) -> Vec<u8>,
        ) -> Result<(), String> {
            Ok(())
        }
        fn set_root(&mut self, _root: Blake2b256Hash) -> Result<(), String> {
            Err("state store unavailable".to_string())
        }
    }

    impl RSpaceImporter for RefusingImporter {
        fn get_history_item(&self, _hash: Blake2b256Hash) -> Result<Option<Vec<u8>>, String> {
            Ok(None)
        }
    }

    /// The fixture the retry tests share (AUDIT C181): a block store whose reads fail — the walk's
    /// first store touch is a `contains`, so an attempt fails at once and for a reason the log names,
    /// rather than waiting out the walk's idle timeout — an importer that refuses every root, and a
    /// transport that answers nothing.
    async fn failing_engine(
        log: &Arc<RecordingLog>,
    ) -> (NodeSyncing<RefusingImporter>, PeerNode, ApprovedStore) {
        let bootstrap = peer("bootstrap");
        let inner_store: BlockStore = Arc::new(KeyValueTypedStoreCodec::new(
            in_memory(),
            Arc::new(BlockHashCodec),
            Arc::new(BlockMessageCodec),
        ));
        let block_store: BlockStore = Arc::new(UnreadableBlockStore { inner: inner_store });
        let dag = build_dag().await;
        let approved_store: ApprovedStore = Arc::new(KeyValueTypedStoreCodec::new(
            in_memory(),
            Arc::new(ByteCodec),
            Arc::new(FringeCodec),
        ));

        let transport: Arc<dyn TransportLayer> = Arc::new(SilentTransport);
        let conf = RPConf {
            local: peer("local"),
            network_id: "testnet".to_string(),
            bootstrap: Some(bootstrap.clone()),
            default_timeout: Duration::from_secs(10),
            max_num_of_connections: 10,
            clear_connections: ClearConnectionsConf {
                num_of_connections_pinged: 10,
            },
        };
        let connections: ConnectionsCell =
            Arc::new(tokio::sync::RwLock::new(vec![bootstrap.clone()]));
        let comm_util = Arc::new(CommUtil::new(
            transport.clone(),
            conf.clone(),
            connections,
            log.clone(),
        ));

        let engine = NodeSyncing::new(
            transport,
            conf,
            block_store,
            dag,
            approved_store.clone(),
            comm_util,
            log.clone(),
            None,
            false,
            RefusingImporter,
        );
        (engine, bootstrap, approved_store)
    }

    /// Regression test for the LFS-sync "justification not present in message map" failure: the
    /// DAG must be populated parents-before-children, i.e. in ascending block height order. The
    /// prior code reversed the height map and inserted the newest block first, whose justification
    /// was not yet in the message map.
    #[tokio::test]
    async fn populate_dag_inserts_parents_before_children() {
        let n = 5i64;
        // A single-validator chain, each block justifying its predecessor.
        let blocks: Vec<BlockMessage> = (0..n)
            .map(|i| chain_block(i, (i > 0).then(|| hash(i as u8 - 1))))
            .collect();

        let block_store: BlockStore = Arc::new(KeyValueTypedStoreCodec::new(
            in_memory(),
            Arc::new(BlockHashCodec),
            Arc::new(BlockMessageCodec),
        ));
        for b in &blocks {
            block_store.put(&[(b.block_hash, b.clone())]).await.unwrap();
        }

        let dag = build_dag().await;
        let mut height_map: BTreeMap<i64, BTreeSet<BlockHash>> = BTreeMap::new();
        for b in &blocks {
            height_map
                .entry(i64::from(b.block_number))
                .or_default()
                .insert(b.block_hash);
        }

        populate_dag(dag.as_ref(), &block_store, &NopLog, &height_map, &[])
            .await
            .expect("populate_dag must succeed when blocks are inserted parents-first");

        // The whole chain is in the DAG, so the latest block number equals the chain length.
        assert_eq!(dag.get_representation().await.latest_block_number(), n);
    }
    /// **AUDIT C68**: a failed LFS sync must not signal the node out of `NodeSyncing`.
    ///
    /// Both directions in one test, because the fix is a decision and the decision has two outcomes:
    /// `Err` must leave the waiter untouched (the oracle's `complete` is sequenced after the drain), and
    /// `Ok` must wake it (otherwise the node would never leave syncing at all, which the second half
    /// rules out). Falsified against this tree: with the unconditional `finished.notify_waiters()`
    /// restored, the first half fails — the waiter is woken by a failed attempt.
    ///
    /// What this pins is the *decision*, not the transition: that a failed attempt leaves the node in
    /// syncing end to end needs a `NodeSyncing` fixture with a failing store and a mock transport,
    /// which is the next unit's work rather than this one's.
    #[tokio::test]
    async fn a_failed_sync_does_not_signal_the_node_out_of_syncing() {
        // A failed attempt: the waiter must stay asleep.
        let finished = tokio::sync::Notify::new();
        let mut waiter = std::pin::pin!(finished.notified());
        notify_when_restored(&Err("LFS state sync failed".to_string()), &finished);
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut waiter)
                .await
                .is_err(),
            "a failed sync signalled the node to leave syncing"
        );

        // A successful attempt: the waiter must wake — so the assertion above cannot pass by the
        // signal never being wired at all.
        let finished = tokio::sync::Notify::new();
        let mut waiter = std::pin::pin!(finished.notified());
        notify_when_restored(&Ok(()), &finished);
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut waiter)
                .await
                .is_ok(),
            "a restored state must signal the node out of syncing"
        );
    }
    /// **U16 / AUDIT C68 *and* C181, end to end: a failed attempt is retried, the retries are bounded,
    /// and the bound has a terminal state.**
    ///
    /// Two properties, and the old version of this test pinned only the first — under a name that
    /// asserted the *defect*. C68: `node_launch` waits on the `finished` handle as the only signal to
    /// leave `NodeSyncing` for `NodeRunning`, so "the node stayed out of running" is exactly "that
    /// handle was never notified"; the attempt must really run and really fail (the recording log
    /// proves it, so the assertion cannot pass because nothing ran). C181: the node must not sit there
    /// for ever — the retries end in a **terminal** signal, which is the state this row is about ("a
    /// serving API and no error line").
    ///
    /// Falsified against this tree by restoring the pre-fix latch (the attempt takes the importer and
    /// both receivers with no retry): the attempt counter never reaches [`MAX_SYNC_ATTEMPTS`] and the
    /// terminal assertion hangs out its bound. The second half — that `finished` still never fires —
    /// is what keeps the fix from being "signal success on failure", which is the C68 regression.
    #[tokio::test]
    async fn a_failed_attempt_is_retried_and_the_retries_end_in_a_terminal_signal() {
        let bootstrap = peer("bootstrap");
        let log = Arc::new(RecordingLog::default());

        let inner_store: BlockStore = Arc::new(KeyValueTypedStoreCodec::new(
            in_memory(),
            Arc::new(BlockHashCodec),
            Arc::new(BlockMessageCodec),
        ));
        let block_store: BlockStore = Arc::new(UnreadableBlockStore { inner: inner_store });
        let dag = build_dag().await;
        let approved_store: ApprovedStore = Arc::new(KeyValueTypedStoreCodec::new(
            in_memory(),
            Arc::new(ByteCodec),
            Arc::new(FringeCodec),
        ));

        let transport: Arc<dyn TransportLayer> = Arc::new(SilentTransport);
        let conf = RPConf {
            local: peer("local"),
            network_id: "testnet".to_string(),
            bootstrap: Some(bootstrap.clone()),
            default_timeout: Duration::from_secs(10),
            max_num_of_connections: 10,
            clear_connections: ClearConnectionsConf {
                num_of_connections_pinged: 10,
            },
        };
        let connections: ConnectionsCell =
            Arc::new(tokio::sync::RwLock::new(vec![bootstrap.clone()]));
        let comm_util = Arc::new(CommUtil::new(
            transport.clone(),
            conf.clone(),
            connections,
            log.clone(),
        ));

        let mut engine = NodeSyncing::new(
            transport,
            conf,
            block_store,
            dag,
            approved_store.clone(),
            comm_util,
            log.clone(),
            None,
            false,
            RefusingImporter,
        );
        let finished = engine.finished_handle();
        // Both waiters exist before anything can signal: `Notify::notify_waiters` only wakes waiters
        // already registered, so a tripwire created after the fact observes nothing (the comment
        // below is this test's own history of getting exactly that wrong).
        let terminal = engine.terminal_handle();
        let mut terminal_waiter = std::pin::pin!(terminal.notified());

        let fringe = FinalizedFringe {
            hashes: vec![hash(7)],
            state_hash: StateHash::new([7u8; 32]),
            ancestry: Vec::new(),
        };

        // The waiter is registered *before* the attempt can signal — polled by the same `select!` that
        // drives the handler — because `Notify::notify_waiters` only wakes waiters already registered.
        // The first version of this test created the waiter *after* the failure and passed with the
        // defect restored: the notification had already fired into an empty waiter set. This is the
        // pass's own rule again (a falsifier must fail against the tree with the defect in it), and the
        // reason the structure below is a `select!` rather than a sleep-then-check.
        // The waiter captures the `notify_waiters` counter **at creation** (tokio's `notified()`
        // reads it then and completes on the next poll if it moved), so creating it before the attempt
        // can signal is what makes the observation independent of *when* the future is polled — no
        // grace period, and no dependence on the scheduler. The first version created it *after* the
        // failure and was blind to the defect for exactly that reason; the second bounded the
        // fixture's progress with a 2 s sleep, which a loaded parallel runner can miss (a flaky
        // failure rather than a missed defect). Both are the same lesson: what a tripwire observes,
        // and when it starts observing, are part of its calibration.
        let mut waiter = std::pin::pin!(finished.notified());
        // A generous bound, because this waits on the *fixture* (a spawned task's failure path), not
        // on anything about the code under test — and it is declared here because the retry drive
        // below shares it.
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        // **Scoped on purpose.** The pinned future borrows `engine` mutably for as long as it lives,
        // and the retry below has to call `handle` again on the same engine; a future that outlives
        // its driving loop would keep the borrow open.
        {
            let handle = async {
                engine
                    .handle(&bootstrap, &CasperMessage::FinalizedFringe(fringe))
                    .await
            };
            tokio::pin!(handle);
            let mut handled = false;
            while !handled {
                tokio::select! {
                    result = &mut handle => {
                        result.expect("the fringe message is handled");
                        handled = true;
                    }
                    _ = tokio::time::sleep(Duration::from_millis(10)) => {
                        assert!(
                            std::time::Instant::now() < deadline,
                            "the fringe handler never returned"
                        );
                    }
                }
            }
        }
        // The attempt must actually run and fail: without this the "no notification" assertion below
        // would pass on a node that never tried, which is the vacuous shape this test exists to avoid.
        while !log.contains("LFS state sync failed") {
            assert!(
                std::time::Instant::now() < deadline,
                "the sync attempt did not fail as the fixture intends, so this test proves nothing"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        // …and with the attempt failed, the node must stay where it is. One `biased` poll of the
        // waiter: it is ready if any `notify_waiters` happened after the waiter was created, which is
        // checked without a time bound.
        let notified = tokio::select! {
            biased;
            _ = &mut waiter => true,
            _ = tokio::task::yield_now() => false,
        };
        assert!(
            !notified,
            "a failed sync notified the sync-finished handle: `node_launch` would take the node into \
             NodeRunning on an empty or partial DAG"
        );

        // The approved block is not recorded either — the oracle sequences that after the state, and a
        // recorded fringe is what a restart would treat as a restored state.
        assert_eq!(
            approved_store
                .get(&[FINALIZED_FRINGE_KEY])
                .await
                .expect("approved store readable")[0],
            None,
            "a failed sync must not record the fringe as approved"
        );

        // --- C181: the attempt is retried, and the retries are bounded. ------------------------
        //
        // Each further bootstrap fringe triggers the next attempt without waiting out
        // `SYNC_RETRY_DELAY`, which is also the property the slot exists for: a later answer is a
        // *newer* target, so it is both the retry's trigger and the better thing to sync to. The
        // **timing assertion at the end of this test is what pins that**: retries driven by the
        // timeout alone would spend two `SYNC_RETRY_DELAY`s here, so a fringe that was received and
        // thrown away — the pre-fix behaviour — cannot get under the bound.
        let retry_started = std::time::Instant::now();
        for attempt in 1..=MAX_SYNC_ATTEMPTS {
            while !log.contains(&format!("attempt {attempt} of {MAX_SYNC_ATTEMPTS}")) {
                assert!(
                    std::time::Instant::now() < deadline,
                    "the fixture never reached attempt {attempt}: the attempt was not retried, which \
                     is the latch this row is about"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            if attempt < MAX_SYNC_ATTEMPTS {
                engine
                    .handle(
                        &bootstrap,
                        &CasperMessage::FinalizedFringe(FinalizedFringe {
                            hashes: vec![hash(7 + attempt as u8)],
                            state_hash: StateHash::new([7 + attempt as u8; 32]),
                            ancestry: Vec::new(),
                        }),
                    )
                    .await
                    .expect("a later bootstrap fringe is handled");
            }
        }

        // The bound is reached and it has a state to reach: `terminal` fires.
        assert!(
            tokio::time::timeout(Duration::from_secs(5), &mut terminal_waiter)
                .await
                .is_ok(),
            "the retries were spent and nothing signalled terminal: the node is left in the state \
             this row names — a serving API and no way out"
        );
        assert!(
            log.contains("is terminal"),
            "the terminal state must be logged, not only signalled"
        );

        // **The retries were driven by the fringes, not by the timeout.** Two `SYNC_RETRY_DELAY`s
        // would be 20 s; anything under one of them means the later fringes were received *and kept*,
        // which is the half the log line alone cannot show. A 2× margin over a path that fails in
        // milliseconds is wide enough for a loaded runner and far too narrow to be satisfied by the
        // timeout.
        assert!(
            retry_started.elapsed() < SYNC_RETRY_DELAY,
            "the retries took {:?}: they waited out the retry delay instead of waking on the later \
             fringes, which means the fringes were not kept",
            retry_started.elapsed()
        );
    }

    /// **AUDIT C181: a later bootstrap fringe is not discarded in silence — the *silence* half.**
    ///
    /// The pre-fix code logged the received fringe *inside* the `if start` branch, so once the trigger
    /// was consumed a later — and necessarily newer — answer produced no log line at all. The repo's
    /// own note calls that "a later fringe from the bootstrap logs nothing at all", and an operator
    /// cannot diagnose a fringe that was received and dropped: this pins that half, and only that half.
    ///
    /// **The "and kept" half is the retry test's timing assertion**, not this one: a log line says the
    /// fringe reached the handler, not that the sync task can still see it. Falsified against this
    /// tree by moving the log back inside the start branch: the second state hash appears nowhere.
    #[tokio::test]
    async fn a_later_bootstrap_fringe_is_not_discarded_in_silence() {
        let log = Arc::new(RecordingLog::default());
        let (mut engine, bootstrap, _approved) = failing_engine(&log).await;

        let first = FinalizedFringe {
            hashes: vec![hash(1)],
            state_hash: StateHash::new([1u8; 32]),
            ancestry: Vec::new(),
        };
        engine
            .handle(&bootstrap, &CasperMessage::FinalizedFringe(first))
            .await
            .expect("the first fringe is handled");

        let second = FinalizedFringe {
            hashes: vec![hash(2)],
            state_hash: StateHash::new([2u8; 32]),
            ancestry: Vec::new(),
        };
        let second_state = rchain_shared::base16::encode(second.state_hash.as_bytes());
        engine
            .handle(&bootstrap, &CasperMessage::FinalizedFringe(second))
            .await
            .expect("the second fringe is handled");

        assert!(
            log.contains(&second_state),
            "the second bootstrap fringe was not even logged: it was discarded in silence, which is \
             exactly what a retry has to have and what this row is about"
        );
    }

    /// **Issue #100: the genesis master's empty fringe must not consume the one-shot sync trigger.**
    ///
    /// The master broadcasts `FinalizedFringe { hashes: Vec::new(), state_hash: genesis.pre_state }` as
    /// an *announcement* (`node_launch.rs`'s `create_store_broadcast_genesis`), not as a sync target, and
    /// on a fresh network it arrives **before** the answer to the joiner's own request — measured on a
    /// devnet at 34 ms after the master sent it and 83 ms before the master had even seen the request.
    /// Consuming the trigger on it made the node "restore" nothing and then discard the real answer in
    /// silence, because a second fringe from the bootstrap logs nothing at all.
    ///
    /// The evidence is a two-phase log observation, and the *first* phase is the discriminator: the
    /// ignore line is written synchronously inside `handle` before it returns, so its presence is
    /// exactly "the empty fringe was seen and refused" and cannot depend on scheduling. Phase 2 is the
    /// positive control — a store that cannot be read makes a *started* sync fail at once and say so, so
    /// the failure line can only appear if the trigger was still available. (Without Unit 1 the sync
    /// starts on the empty fringe in phase 1, and phase 2 produces nothing.)
    #[tokio::test]
    async fn an_empty_fringe_does_not_consume_the_sync_trigger() {
        let bootstrap = peer("bootstrap");
        let log = Arc::new(RecordingLog::default());

        let inner_store: BlockStore = Arc::new(KeyValueTypedStoreCodec::new(
            in_memory(),
            Arc::new(BlockHashCodec),
            Arc::new(BlockMessageCodec),
        ));
        let block_store: BlockStore = Arc::new(UnreadableBlockStore { inner: inner_store });
        let dag = build_dag().await;
        let approved_store: ApprovedStore = Arc::new(KeyValueTypedStoreCodec::new(
            in_memory(),
            Arc::new(ByteCodec),
            Arc::new(FringeCodec),
        ));

        let transport: Arc<dyn TransportLayer> = Arc::new(SilentTransport);
        let conf = RPConf {
            local: peer("local"),
            network_id: "testnet".to_string(),
            bootstrap: Some(bootstrap.clone()),
            default_timeout: Duration::from_secs(10),
            max_num_of_connections: 10,
            clear_connections: ClearConnectionsConf {
                num_of_connections_pinged: 10,
            },
        };
        let connections: ConnectionsCell =
            Arc::new(tokio::sync::RwLock::new(vec![bootstrap.clone()]));
        let comm_util = Arc::new(CommUtil::new(
            transport.clone(),
            conf.clone(),
            connections,
            log.clone(),
        ));

        let mut engine = NodeSyncing::new(
            transport,
            conf,
            block_store,
            dag,
            approved_store,
            comm_util,
            log.clone(),
            None,
            false,
            RefusingImporter,
        );

        // Phase 1: the master's announcement, exactly as `create_store_broadcast_genesis` sends it.
        let announcement = FinalizedFringe {
            hashes: Vec::new(),
            state_hash: StateHash::new([0u8; 32]),
            ancestry: Vec::new(),
        };
        engine
            .handle(&bootstrap, &CasperMessage::FinalizedFringe(announcement))
            .await
            .expect("the empty fringe is handled");

        assert!(
            log.contains("Ignoring an empty fringe"),
            "an empty fringe must be refused as a sync target — it is the genesis master's \
             announcement, not an answer. If this is absent the fringe was consumed as one."
        );

        // Phase 2: a fringe that names a block. It must still be able to start the sync.
        let fringe = FinalizedFringe {
            hashes: vec![hash(7)],
            state_hash: StateHash::new([7u8; 32]),
            ancestry: Vec::new(),
        };
        engine
            .handle(&bootstrap, &CasperMessage::FinalizedFringe(fringe))
            .await
            .expect("the named fringe is handled");

        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while !log.contains("LFS state sync failed") {
            assert!(
                std::time::Instant::now() < deadline,
                "the fringe that named a block did not start a sync: the one-shot trigger was \
                 consumed by the empty announcement, which is issue #100"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}
