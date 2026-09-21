//! Proposer instance (port of `node/instances/ProposerInstance.scala`).
//!
//! Drains propose requests, serializing actual proposal through a semaphore; concurrent attempts
//! resolve to `ProposerResult::Empty` and set the `trigger` flag, so a propose that arrives while
//! one is running is re-enqueued once the running one finishes (the Scala's trigger re-enqueue).
//!
//! **Every propose outcome is logged here, where it is produced.** Most propose requests discard
//! their result — the `proposeOnDeploy` path (`let _ = trigger(true).await`) and both autopropose
//! taps (`try_send` with the reply receiver dropped) — so a proposer that fails on every attempt
//! stops block production *silently*: `/api/status` keeps serving, the container stays healthy, and
//! the log stays empty while the height never moves. Logging at the point of production makes every
//! one of those paths visible at once, including the ones with no caller left to report to.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use futures_util::StreamExt;
use tokio::sync::{mpsc, oneshot, Semaphore};
use tokio_stream::wrappers::ReceiverStream;

use rchain_casper::blocks::proposer::propose_result::{ProposeResult, ProposeStatus};
use rchain_casper::blocks::proposer::proposer::{Proposer, ProposerResult};
use rchain_casper::state::ProposerState;
use rchain_models::casper::protocol::casper_message::BlockMessage;
use rchain_shared::log::{Log, LogSource};

/// Log one propose outcome. Routine outcomes (no new deploys, not bonded, a propose already in
/// flight) are `debug`; every outcome that means *a block was expected and was not produced* is
/// louder, so a stalled proposer cannot hide behind a serving node. A `BugError` is an internal
/// failure — the class the wedged devnet hit with "Fringe state not available in state cache" —
/// and is logged at `error` with the reason, because it never resolves on its own.
fn log_propose_result(log: &Arc<dyn Log>, result: &(ProposeResult, Option<BlockMessage>)) {
    let source = LogSource::new("coop.rchain.node.instances.ProposerInstance");
    let (propose_result, block) = result;
    match block {
        Some(block) => log.info(
            source,
            &format!(
                "proposed and added block #{} (seq {})",
                block.block_number, block.seq_num
            ),
        ),
        None => match &propose_result.propose_status {
            ProposeStatus::ProposeSuccess => log.info(source, "propose succeeded (no new block)"),
            ProposeStatus::BugError(reason) => log.error(
                source,
                &format!(
                    "propose failed with an internal error: {reason} — no block was produced, so \
                     the chain will not advance until this is fixed"
                ),
            ),
            status @ (ProposeStatus::TooFarAheadOfLastFinalized
            | ProposeStatus::InternalDeployError) => {
                log.warn(source, &format!("propose failed: {status}"))
            }
            status => log.debug(source, &format!("propose produced no block: {status}")),
        },
    }
}

/// Create the proposer stream (port of `ProposerInstance.create`).
pub fn create(
    propose_requests_rx: mpsc::Receiver<(bool, oneshot::Sender<ProposerResult>)>,
    propose_requests_tx: mpsc::Sender<(bool, oneshot::Sender<ProposerResult>)>,
    proposer: Proposer,
    state: Arc<tokio::sync::Mutex<ProposerState>>,
    log: Arc<dyn Log>,
) -> impl tokio_stream::Stream<Item = (ProposeResult, Option<BlockMessage>)> + Send + 'static {
    let input = ReceiverStream::new(propose_requests_rx);
    let lock = Arc::new(Semaphore::new(1));
    let trigger = Arc::new(AtomicBool::new(false));
    let proposer = Arc::new(proposer);

    input
        .map(move |(is_async, propose_id_def)| {
            let lock = lock.clone();
            let trigger = trigger.clone();
            let state = state.clone();
            let tx = propose_requests_tx.clone();
            let proposer = proposer.clone();
            let log = log.clone();
            async move {
                let permit = match lock.clone().try_acquire_owned() {
                    Ok(p) => p,
                    Err(_) => {
                        let _ = propose_id_def.send(ProposerResult::Empty);
                        trigger.store(true, Ordering::SeqCst);
                        return None;
                    }
                };

                let (r_tx, r_rx) = oneshot::channel();
                {
                    state.lock().await.curr_propose_result = Some(r_rx);
                }
                let r = proposer.propose(is_async, propose_id_def).await;
                let r = match r {
                    Ok(r) => r,
                    Err(e) => (
                        ProposeResult {
                            propose_status: ProposeStatus::BugError(e),
                        },
                        None,
                    ),
                };
                let _ = r_tx.send(r.clone());
                // The requester may be long gone (both autopropose taps drop the receiver), so this
                // is the only place every outcome is guaranteed to be seen.
                log_propose_result(&log, &r);
                {
                    let mut s = state.lock().await;
                    s.latest_propose_result = Some(r.clone());
                    s.curr_propose_result = None;
                }
                drop(permit);

                // Re-enqueue a follow-up propose if a request arrived while this one was running.
                if trigger.swap(false, Ordering::SeqCst) {
                    let (d_tx, d_rx) = oneshot::channel();
                    let _ = tx.send((false, d_tx)).await;
                    // Keep the receiver alive until the re-queued propose completes it.
                    std::mem::forget(d_rx);
                }
                Some(r)
            }
        })
        .buffer_unordered(100)
        .filter_map(|r| async move { r })
}
