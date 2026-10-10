//! The ordered, windowed catch-up a node runs after restoring at an anchor below its peers' tip (C259).
//!
//! **Why this exists.** Every block request in this protocol is keyed by hash, so a node that has just
//! restored at an anchor can only learn the gap by walking *downward* from a peer's tip through
//! `justifications`. Each new block justifies blocks inside the gap, so that walk pulls the whole gap
//! in at once — and `MAX_PENDING_BLOCKS` then turns progress into drops. Measured 2026-10-10 on a
//! four-validator devnet: `block receiver state full (1024 blocks); dropping` 173 times, **one** block
//! validated, the node frozen at height 121 while the master ran to 577
//! (`spec/audit/evidence/n-anchor-drill/run-2-catchup-stall.txt`). A wiped joiner on the *ordinary*
//! path behaves the same way on that rig, which is what rules the anchor out as the cause: the ingest
//! is what cannot take a gap.
//!
//! **What this does instead.** Walk the gap *upward* in windows: ask the peer for the heights
//! `[H+1, H+W]` (`BlockRangeRequest`), ask for those blocks by hash, and only ask for the next window
//! once the node's own frontier has reached the last one. Two things make it hold:
//!
//! - the answer is **topological** (ascending height, parent before child), and the requests go out in
//!   that order, so a window's blocks validate as they arrive instead of piling up;
//! - the [`CatchupWindow`] the driver holds narrows the ingest to `frontier + W`, so the gossip that
//!   keeps arriving — the tip's hashes, whose justifications point into the gap — cannot refill the
//!   pending set behind the walk. Blocks above the window are left **un-acked**, not refused: the
//!   retriever re-requests them once the frontier has moved past, which is what makes the window a
//!   pace rather than a loss.
//!
//! Nothing above the anchor is *installed*: every block the walk delivers goes through the ordinary
//! validation path, so only the anchor's state is adopted — #287's first invariant.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rchain_block_storage::dag::dag_storage::BlockDagStorage;
use rchain_comm::peer_node::PeerNode;
use rchain_comm::rp::rp_conf::RPConf;
use rchain_comm::transport::transport_layer::TransportLayer;
use rchain_models::casper::protocol::casper_message::{BlockRange, BlockRangeRequest};
use rchain_shared::log::{Log, LogSource};

use crate::protocol::comm_util::{CommUtil, ConnectionsCell};

/// How many heights one window covers. A four-validator shard holds at most four blocks per height, so
/// one window is at most 32 blocks — far below `MAX_PENDING_BLOCKS`, which is the whole point: the
/// pending set has to hold a window, never a gap.
pub const WINDOW_HEIGHTS: i64 = 8;

/// How long one window may take before the walk gives up and hands the node back to the ordinary
/// ingest. Bounded so a silent or stalling peer cannot hold the gate shut — the node is never *worse*
/// off than before the catch-up, which is the property that makes this safe to run automatically.
const WINDOW_TIMEOUT: Duration = Duration::from_secs(45);

/// The most windows one catch-up will walk, so a peer answering `[H+1, H+W]` forever cannot keep the
/// walk (and the gate) alive without bound.
const MAX_WINDOWS: u32 = 4096;

/// How often the driver looks at its own frontier while a window is in flight.
const FRONTIER_POLL: Duration = Duration::from_millis(200);

/// The temporary ingest window a node holds while it catches up.
///
/// **Released is the ordinary node.** `top` is `i64::MAX` unless a driver is walking, so an
/// unattended node admits everything exactly as it did before this module existed, and the gate can
/// only ever be as narrow as a catch-up makes it.
pub struct CatchupWindow {
    top: AtomicI64,
    width: i64,
}

impl CatchupWindow {
    pub fn new(width: i64) -> Self {
        CatchupWindow {
            top: AtomicI64::new(i64::MAX),
            width,
        }
    }

    /// **The one question the ingest asks**: may a block at this height be pended *now*?
    ///
    /// A block above the window is one the walk has not reached yet, so pending it would be holding a
    /// block whose parents are not validated — the shape that fills `MAX_PENDING_BLOCKS`. It is left
    /// for later rather than refused (the caller does not ack it), so nothing is lost.
    pub fn admits(&self, height: i64) -> bool {
        height <= self.top.load(Ordering::Relaxed)
    }

    /// Whether a walk is holding the window shut. For logs and tests.
    pub fn is_active(&self) -> bool {
        self.top.load(Ordering::Relaxed) != i64::MAX
    }

    /// Narrow the ingest to `frontier + width`. `pub(crate)` only so the ingest's own test can hold a
    /// window without a driver; the walk in this module is the only production caller.
    pub(crate) fn hold(&self, frontier: i64) {
        self.top
            .store(frontier.saturating_add(self.width), Ordering::Relaxed);
    }

    /// Hand the node back: every height is admitted again.
    pub(crate) fn release(&self) {
        self.top.store(i64::MAX, Ordering::Relaxed);
    }
}

/// Walk the gap above this node's frontier, in windows, against `peer`.
///
/// Returns the height the walk reached (the frontier it saw last), which is what the caller logs. The
/// window is released on **every** exit path — a catch-up that cannot finish must not leave the node
/// with a narrowed ingest.
#[allow(clippy::too_many_arguments)]
pub async fn run_catchup(
    transport: Arc<dyn TransportLayer>,
    conf: RPConf,
    connections: ConnectionsCell,
    dag: Arc<dyn BlockDagStorage>,
    log: Arc<dyn Log>,
    window: Arc<CatchupWindow>,
    peer: PeerNode,
    mut answers: tokio::sync::mpsc::Receiver<BlockRange>,
) -> i64 {
    let source = LogSource::new("casper.engine.Catchup");
    let comm = CommUtil::new(transport, conf, connections, log.clone());
    let mut frontier = frontier_of(dag.as_ref()).await;
    window.hold(frontier);
    log.info(
        source,
        &format!(
            "catch-up against {}: this node's frontier is height {}, walking outward in windows of {} \
             heights (nothing above the anchor is installed — every window is validated)",
            peer.endpoint.host, frontier, window.width
        ),
    );
    let mut walked: u32 = 0;
    loop {
        if walked >= MAX_WINDOWS {
            log.warn(
                source,
                &format!("catch-up stopped after {MAX_WINDOWS} windows at height {frontier}"),
            );
            break;
        }
        walked += 1;
        let from = frontier.saturating_add(1);
        let req = BlockRangeRequest {
            from,
            to: from.saturating_add(window.width - 1),
        };
        comm.request_block_range(&peer, &req).await;
        let answer = match tokio::time::timeout(WINDOW_TIMEOUT, answers.recv()).await {
            Ok(Some(answer)) => answer,
            Ok(None) => break,
            Err(_) => {
                // A silent peer is the end of the walk, not a failure of the node: the gate opens and
                // the ordinary ingest resumes exactly where it was.
                log.warn(
                    source,
                    &format!("no window arrived for heights {from}..={} in {WINDOW_TIMEOUT:?}: stopping the catch-up at height {frontier}", req.to),
                );
                break;
            }
        };
        // **An empty window is the end of the gap**, and it is also what this node gets from a peer
        // that does not know the message — the same degraded exchange the anchored fringe request
        // gets, and the reason the message is a non-breaking addition.
        if answer.hashes.is_empty() {
            log.info(
                source,
                &format!("the peer has nothing above height {frontier}; catch-up finished"),
            );
            break;
        }
        for hash in &answer.hashes {
            comm.request_for_block(&peer, hash).await;
        }
        // Ask for the window, then let it validate: the next window starts only once this node's own
        // frontier has reached the height this one covered.
        let reached = wait_for_frontier(dag.as_ref(), answer.to, frontier).await;
        frontier = reached;
        window.hold(frontier);
    }
    window.release();
    log.info(
        source,
        &format!("catch-up released the ingest window at height {frontier}"),
    );
    frontier
}

/// The node's own frontier: the highest height in its DAG. `latest_block_number` is one past it, and
/// an empty DAG reads as -1 so a caller can always say "the next height is frontier + 1".
async fn frontier_of(dag: &dyn BlockDagStorage) -> i64 {
    let repr = dag.get_representation().await;
    repr.latest_block_number() - 1
}

/// Wait until the DAG has reached `target`, bounded by [`WINDOW_TIMEOUT`]; returns the frontier it
/// saw. A window that never arrives in time yields the frontier as it stands — the walk then asks
/// again for the heights it still lacks, which is the pace, not a failure.
async fn wait_for_frontier(dag: &dyn BlockDagStorage, target: i64, current: i64) -> i64 {
    let deadline = tokio::time::Instant::now() + WINDOW_TIMEOUT;
    let mut frontier = current;
    while frontier < target && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(FRONTIER_POLL).await;
        frontier = frontier_of(dag).await;
    }
    frontier
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **Released, a window admits everything** — an ordinary node's ingest must be exactly what it
    /// was before the driver existed, or this would be a new bound on every node rather than a pace
    /// for one that is catching up.
    #[test]
    fn a_released_window_admits_every_height() {
        let w = CatchupWindow::new(WINDOW_HEIGHTS);
        assert!(!w.is_active());
        for height in [0, 1, 1_000_000, i64::MAX - 1] {
            assert!(
                w.admits(height),
                "height {height} was refused by a released window"
            );
        }
    }

    /// Held at a frontier, a window admits up to `frontier + width` and nothing above — and it admits
    /// blocks *below* the frontier too (a block that arrives late, or an ancestor, is not what fills
    /// the pending set).
    #[test]
    fn a_held_window_admits_the_window_and_not_the_gap_above_it() {
        let w = CatchupWindow::new(WINDOW_HEIGHTS);
        w.hold(100);
        assert!(w.is_active());
        assert!(w.admits(100), "the frontier itself");
        assert!(w.admits(108), "the top of the window");
        assert!(!w.admits(109), "one past the window is the gap");
        assert!(
            !w.admits(577),
            "the tip is the gap while the frontier is at 100"
        );
        assert!(w.admits(0), "an ancestor is always admissible");
    }

    /// The window advances with the frontier, and releasing it restores the ordinary ingest.
    #[test]
    fn the_window_advances_with_the_frontier_and_release_restores_it() {
        let w = CatchupWindow::new(WINDOW_HEIGHTS);
        w.hold(100);
        assert!(!w.admits(109));
        w.hold(108);
        assert!(w.admits(116), "the window moved with the frontier");
        w.release();
        assert!(!w.is_active());
        assert!(w.admits(i64::MAX - 1));
    }

    /// A frontier at the top of the range saturates rather than wrapping: the width is added with
    /// `saturating_add`, so the only thing that can happen at the top is a *wider* window, and there is
    /// no height above `i64::MAX` to admit anyway. An overflow that wrapped would turn the window
    /// negative and refuse everything, which is the failure this pins.
    #[test]
    fn a_frontier_near_the_top_of_the_range_saturates_rather_than_wrapping() {
        let w = CatchupWindow::new(WINDOW_HEIGHTS);
        w.hold(i64::MAX - 2);
        assert!(
            w.admits(i64::MAX - 2),
            "the frontier itself is always admitted"
        );
        assert!(w.admits(i64::MAX), "saturating, not wrapping");
    }
}
