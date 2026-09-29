#![forbid(unsafe_code)]
//! The node entry point (port of `Main.scala` + `NodeMain.startNode`).

// Heap profiling, for the memory investigations (#117). `dhat::Alloc` implements `GlobalAlloc` inside
// the dhat crate, so installing it needs no `unsafe` here and the crate-level `forbid` still holds.
// It is registered in this *binary* only, never in the library, so a `--all-features` test build
// compiles the dependency without putting a profiling allocator under the test harness.
#[cfg(feature = "dhat-heap")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

use std::sync::Arc;

use clap::Parser;

use rchain_node::configuration::commandline::options::{Commands, Options};
use rchain_node::configuration::configuration::Configuration;
use rchain_node::runtime::{node_environment, node_runtime, run_cli};
use rchain_shared::log::{Log, LogSource, StderrLog};

fn main() {
    // The profiler writes its dump when it drops, so it has to outlive the whole of `main` — and the
    // node has to *return* from `main` for the file to exist at all, which is what AUDIT C144's
    // SIGTERM handler buys: before it, `docker stop` ended at exit 137 and the dump was never
    // written. `--profile docker` puts the data dir at /var/lib/rnode, where the dump is retrieved
    // from after a clean stop.
    #[cfg(feature = "dhat-heap")]
    let _profiler = dhat::Profiler::builder()
        .file_name("/var/lib/rnode/dhat-heap.json")
        .build();

    // Parse options before building the tokio runtime: the thread-pool size must be known up
    // front (the worker count is fixed at runtime construction).
    let options = Options::parse();

    // The number of tokio worker threads (the scheduler's CPU parallelism): the CLI value, else
    // the machine's hardware parallelism. Validated ≥ 1 so a mistyped flag fails fast with a
    // legible error instead of a tokio panic.
    let worker_threads = match &options.subcommand {
        Commands::Run(run) => match run.thread_pool_size {
            Some(n) if n < 1 => {
                eprintln!("Invalid --thread-pool-size {n}: must be at least 1");
                std::process::exit(1);
            }
            Some(n) => n as usize,
            None => default_worker_threads(),
        },
        _ => default_worker_threads(),
    };

    // The log level (AUDIT C145) is resolved here for the same reason the worker count is: it is a
    // property of the *process*, and a typo has to fail before the node starts rather than leave the
    // operator with logs quieter than they asked for.
    let log_level = match &options.subcommand {
        Commands::Run(run) => match &run.log_level {
            Some(value) => match rchain_shared::log::Level::parse(value) {
                Ok(level) => level,
                Err(e) => {
                    eprintln!("Invalid --log-level: {e}");
                    std::process::exit(1);
                }
            },
            None => rchain_shared::log::Level::Info,
        },
        _ => rchain_shared::log::Level::Info,
    };

    // The rholang parser + reducer recurse through deeply-nested contract sources (the genesis
    // blessed terms in particular); a 2 MiB worker stack overflows. Give the runtime a larger
    // per-worker stack (the JVM node runs these paths on a much larger native stack).
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .thread_stack_size(32 * 1024 * 1024)
        .worker_threads(worker_threads)
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("Failed to build the tokio runtime: {e}");
            std::process::exit(1);
        }
    };
    runtime.block_on(async_main(options, log_level));
}

/// The machine's hardware parallelism (the tokio default), falling back to 1.
fn default_worker_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The worker count defaults to the machine's parallelism, and **never zero**: a tokio runtime
    /// built with zero worker threads cannot make progress, and the fallback exists for the
    /// platforms where `available_parallelism` fails. The other half of the rule — an explicit
    /// `--thread-pool-size 0` is refused with a message and `exit(1)` — lives in `main` itself and
    /// cannot be asserted from inside the process; it is what the flag's parse-time validation
    /// covers.
    #[test]
    fn the_default_worker_count_is_at_least_one() {
        let threads = default_worker_threads();
        assert!(
            threads >= 1,
            "a runtime needs at least one worker: {threads}"
        );
        assert!(
            threads <= 1024,
            "a sane machine has fewer than 1024 cores: {threads}"
        );

        // It is the hardware parallelism when the platform reports it…
        if let Ok(available) = std::thread::available_parallelism() {
            assert_eq!(threads, available.get());
        }
        // …and stable across calls, so two builds of the runtime agree.
        assert_eq!(threads, default_worker_threads());
    }
}

async fn async_main(options: Options, log_level: rchain_shared::log::Level) {
    // The one logger the node installs, at the operator's level (AUDIT C145). `StderrLog` writes the
    // wall clock on every line, so this is also what makes a cross-node timeline reconstructable.
    let log: Arc<StderrLog> = Arc::new(StderrLog::new(log_level));

    // Execute a thin-client CLI command (port of `Main.main`'s `runCLI` branch).
    if !matches!(options.subcommand, Commands::Run(_)) {
        if let Err(errors) = run_cli(&options).await {
            for error in &errors {
                println!("{error}");
            }
            std::process::exit(1);
        }
        return;
    }

    let (node_conf, _profile, _config_file) = match Configuration::build(&options) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Configuration error: {e}");
            std::process::exit(1);
        }
    };

    let id = match node_environment::create(&node_conf) {
        Ok(id) => id,
        Err(e) => {
            eprintln!("Initialization error: {e}");
            std::process::exit(1);
        }
    };

    let program = match node_runtime::setup_node_program(&node_conf, &id, log.clone()).await {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Setup error: {e}");
            std::process::exit(1);
        }
    };

    // The stop path (AUDIT C144). Before this, nothing installed a signal handler: the only stop was
    // a `SIGKILL` after the orchestrator's grace period, and **inside the container the node is PID 1,
    // where the kernel does not apply the default terminate disposition** — so `docker stop`,
    // systemd's `ExecStop` and a pod eviction all had their `SIGTERM` delivered and discarded.
    //
    // The listener handles are the sixth thing that can end the wait, and they must keep being
    // reported (AUDIT C142), so the serve future is pinned and raced rather than moved into a
    // `select!` arm: on a signal it still has to be *awaited* to completion, or dropping it would
    // detach the listeners and turn a drain back into a truncation.
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let serving = program.serve(stop_rx);
    tokio::pin!(serving);
    let stopping = tokio::select! {
        result = &mut serving => Stopping::Served(result),
        signal = rchain_node::runtime::shutdown::shutdown_signal() => Stopping::Signal(signal),
    };
    match stopping {
        Stopping::Served(Err(e)) => {
            eprintln!("Server error: {e}");
            std::process::exit(1);
        }
        Stopping::Served(Ok(())) => {}
        Stopping::Signal(signal) => {
            log.info(
                LogSource::new("coop.rchain.node.Main"),
                &format!(
                    "Received {signal}: stopping the listeners and draining (AUDIT C144, bounded at \
                     {}s)",
                    rchain_node::runtime::shutdown::SHUTDOWN_DRAIN_TIMEOUT.as_secs()
                ),
            );
            let _ = stop_tx.send(true);
            if let Err(e) = serving.await {
                eprintln!("Server error while shutting down: {e}");
                std::process::exit(1);
            }
            log.info(
                LogSource::new("coop.rchain.node.Main"),
                "Shutdown complete.",
            );
        }
    }
}

/// Why the wait for the listeners ended.
enum Stopping {
    /// `NodeProgram::serve` returned: a listener stopped, or failed. Its result is the error to
    /// report — a clean `Ok` here means an accept loop returned, which is not a normal stop.
    Served(Result<(), String>),
    /// The operator asked the node to stop, naming the signal so the log can say which one arrived.
    Signal(&'static str),
}
