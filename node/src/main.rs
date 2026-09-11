//! The node entry point (port of `Main.scala` + `NodeMain.startNode`).

use std::sync::Arc;

use clap::Parser;

use rchain_node::configuration::commandline::options::{Commands, Options};
use rchain_node::configuration::configuration::Configuration;
use rchain_node::runtime::{node_environment, node_runtime, run_cli};
use rchain_shared::log::StderrLog;

fn main() {
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
    runtime.block_on(async_main(options));
}

/// The machine's hardware parallelism (the tokio default), falling back to 1.
fn default_worker_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

async fn async_main(options: Options) {
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

    let program = match node_runtime::setup_node_program(&node_conf, &id, Arc::new(StderrLog)).await
    {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Setup error: {e}");
            std::process::exit(1);
        }
    };

    if let Err(e) = program.serve().await {
        eprintln!("Server error: {e}");
        std::process::exit(1);
    }
}
