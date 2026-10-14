//! The node's stop path (AUDIT C144).
//!
//! Nothing installed a signal handler before this module, so the only stop was a `SIGKILL` after the
//! orchestrator's termination grace — and inside the container, where the node is PID 1 and the
//! kernel does not apply the default terminate disposition, even `SIGTERM` was delivered and
//! discarded. `docker stop`, systemd's `ExecStop` and a pod eviction all take that path; the register
//! measured it (`docker kill -s TERM` left the container `Running=true` twenty seconds later with no
//! log line, and a plain `docker stop` ended at `ExitCode=137`).
//!
//! The coordinator is **one `watch` channel** rather than a future handed to each server, and the
//! reason is the shape of the thing: a stop is a single event that six listeners must all observe.
//! [`shutdown_signal`] waits for the operator, [`stop_requested`] is what each listener awaits, and
//! [`crate::runtime::node_runtime::NodeProgram::serve`] drains behind them before returning.

use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use rchain_shared::lock::Unpoison;
use tokio::sync::watch;
use tokio::sync::Notify;

/// How long the listeners are given to finish what they have in flight once the operator asks the
/// node to stop. Bounded on purpose: a wedged listener must not be able to hold the process past the
/// orchestrator's termination grace, which is the cost this module exists to stop paying.
pub const SHUTDOWN_DRAIN_TIMEOUT: Duration = Duration::from_secs(10);

/// **The fatal latch** — a fault the node cannot serve through, reported by any task that concludes
/// the node must stop (2026-10-13, §106's owed D3).
///
/// **Why a latch and not an error return.** The node's "this node is stopping" statements are today a
/// `return Err` from a task nobody joins: the launched `node_launch` future's error is only logged
/// (`node_runtime`'s spawn wrapper) and `serve` never awaits it, so the block receiver, the processor,
/// the proposer and the API keep serving a node whose stores may have moved under a view that was not
/// rebuilt. A *concluded* stop has to be a **process** stop, and this is the word that makes it one:
/// the reporter sets it, and `main`'s wait selects on it exactly as it selects on a signal.
static FATAL: Mutex<Option<String>> = Mutex::new(None);
static FATAL_NOTIFY: OnceLock<Arc<Notify>> = OnceLock::new();

fn fatal_notify() -> &'static Arc<Notify> {
    FATAL_NOTIFY.get_or_init(|| Arc::new(Notify::new()))
}

/// Report a fault the node cannot serve through. **The first reason wins** — a later report is the
/// same fault seen again — and the wait in `main` is woken.
///
/// Idempotent and callable from any task: a second reporter changes nothing, so a fault observed
/// twice (the engine's reset arm and the boot path, say) still produces one reason and one latch.
pub fn report_fatal(reason: impl Into<String>) {
    {
        let mut slot = FATAL.lock().unpoison();
        if slot.is_some() {
            return;
        }
        *slot = Some(reason.into());
    }
    // `notify_one`, not `notify_waiters`: a report that arrives *before* the wait begins stores a
    // permit, so a wait that starts late cannot miss the word — the same reason `request_reset` uses
    // it, and the same failure it would otherwise have (`notify_waiters` stores nothing).
    fatal_notify().notify_one();
}

/// Resolves when a fault has been reported, carrying the **first** reason. Returns immediately if one
/// already has been, so nothing can miss the word by arriving late.
pub async fn fatal_requested() -> String {
    let notify = fatal_notify().clone();
    loop {
        if let Some(reason) = FATAL.lock().unpoison().clone() {
            return reason;
        }
        notify.notified().await;
    }
}

/// Resolves when the coordinator asks this listener to stop.
///
/// `wait_for`, not `changed`: it also answers immediately for a receiver created *after* the word
/// went out, so a listener that comes up late cannot miss the stop. A dropped sender resolves it too
/// — nothing is left that could ask — which is safe because the coordinator is held for as long as
/// the servers are.
pub async fn stop_requested(mut stop: watch::Receiver<bool>) {
    let _ = stop.wait_for(|stop| *stop).await;
}

/// Wait for the operator's stop: `SIGTERM` (what `docker stop`, systemd and a pod eviction send) or
/// `SIGINT` (Ctrl-C). Returns which one arrived, so the log line can name it.
pub async fn shutdown_signal() -> &'static str {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};

        // A failure here means the process's signal table cannot be written, which no handler in
        // this process can fix: fall back to Ctrl-C rather than panicking. No `.expect` — the
        // type-system gate's `panic` class scans this crate's production code, and a node that
        // cannot be *started* into a handler is a smaller fault than one that aborts.
        match signal(SignalKind::terminate()) {
            Ok(mut term) => tokio::select! {
                _ = term.recv() => "SIGTERM",
                _ = tokio::signal::ctrl_c() => "SIGINT",
            },
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
                "SIGINT"
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
        "SIGINT"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The listener's half of the stop path, in both directions: the future a server is handed must
    /// **not** resolve while nobody has asked it to stop, and must resolve as soon as somebody has.
    /// The first half is the one worth having — a `stop_requested` that resolved early would take
    /// every listener down the moment the node started serving.
    #[tokio::test]
    async fn stop_requested_waits_for_the_word_and_then_resolves() {
        let (tx, rx) = watch::channel(false);
        assert!(
            tokio::time::timeout(Duration::from_millis(50), stop_requested(rx.clone()))
                .await
                .is_err(),
            "a listener must not stop before the coordinator asks it to"
        );
        tx.send(true).expect("the receiver is held");
        tokio::time::timeout(Duration::from_secs(1), stop_requested(rx))
            .await
            .expect("a listener must stop once the coordinator asks it to");
    }

    /// **The fatal latch, in both directions** (§106's owed D3). The wait must **not** resolve while
    /// no fault has been reported — a latch that resolved early would stop the node the moment it
    /// started — and must resolve as soon as one is, carrying the **first** reason and not a later
    /// one. The latch is process-wide, so this test owns it, and it is the only reporter.
    ///
    /// What it does not run: the `main` arm that selects on this and the `node_runtime` wrapper that
    /// reports. That wiring is read rather than run, for the reason §106 gives — the `main` wait has
    /// no harness — and it is said out loud here rather than left as an implication.
    #[tokio::test]
    async fn a_reported_fatal_resolves_the_wait_with_the_first_reason() {
        assert!(
            tokio::time::timeout(Duration::from_millis(50), fatal_requested())
                .await
                .is_err(),
            "the wait must not resolve before any fault is reported"
        );
        report_fatal("test: the first reason");
        let reason = tokio::time::timeout(Duration::from_secs(1), fatal_requested())
            .await
            .expect("a reported fault must resolve the wait");
        assert_eq!(reason, "test: the first reason");
        report_fatal("test: a later reason");
        let again = tokio::time::timeout(Duration::from_secs(1), fatal_requested())
            .await
            .expect("and the latch stays set");
        assert_eq!(again, "test: the first reason", "the first reason wins");
    }
}
