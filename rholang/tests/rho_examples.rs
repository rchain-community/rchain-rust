//! Smoke-test the rholang example/library programs: each `.rho` file must parse,
//! normalize, and reduce without error through the real runtime.

use std::collections::BTreeMap;

use rchain_crypto::hash::blake2b512_random::Blake2b512Random;
use rchain_crypto::public_key::PublicKey;
use rchain_models::normalizer_env::NormalizerEnv;

mod common;

fn fixed_rand() -> Blake2b512Random {
    Blake2b512Random::from_init(&[0u8; 32])
}

fn read_rho(rel: &str) -> String {
    let path = format!("{}/../{rel}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

const EXAMPLES: &[&str] = &[
    "qucalc/rholang/Directory.rho",
    "qucalc/rholang/Inbox.rho",
    "qucalc/rholang/Chat.rho",
    "qucalc/rholang/qucalc.rho",
    "qucalc/rholang/gov.rho",
    "qucalc/examples/syllogism.rho",
    "qucalc/examples/multisig.rho",
    "qucalc/examples/promissory_note.rho",
    "qucalc/examples/atomic_swap.rho",
    "qucalc/examples/dining_philosophers.rho",
    "qucalc/examples/liquid_democracy.rho",
    "qucalc/examples/shard_exchange.rho",
    "examples/wallet.rho",
];

/// An explicit stack, for the reason `legacy_contracts.rs` records: `qucalc/rholang/gov.rho` nests
/// deeply enough that parsing it is **marginal** on the default 2 MiB test stack — it parses on 8
/// MiB and aborts below it, and the margin is small enough that an unrelated change elsewhere in the
/// parser (a few bytes per frame) decides whether it survives. A test that aborts with a stack
/// overflow in that regime reports nothing about the parse; the requirement belongs here, where it is
/// needed, not in a `RUST_MIN_STACK` invocation nobody will remember. Both stacks matter:
/// `block_on` drives the future on the *calling* thread, and each program's reduction runs on a
/// runtime worker.
const STACK: usize = 8 << 20;

#[test]
fn rho_examples_parse_and_reduce() {
    std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(|| {
            tokio::runtime::Builder::new_multi_thread()
                .thread_stack_size(STACK)
                .enable_all()
                .build()
                .expect("a tokio runtime")
                .block_on(run_examples())
        })
        .expect("spawn the corpus thread")
        .join()
        .expect("the corpus thread panicked");
}

async fn run_examples() {
    let (rt, _replay) = common::build_runtime_pair().await;
    let rand = fixed_rand();
    let empty_env = BTreeMap::new();
    let deployer_env = NormalizerEnv::with_deployer_id(&PublicKey::new(vec![7u8; 65]))
        .to_env()
        .clone();

    let mut failures = Vec::new();
    for &rel in EXAMPLES {
        let source = read_rho(rel);
        // The deployer binding is injected when the *content* binds it. A hand-maintained list of
        // filenames has to be edited whenever a file's content changes, and it fails by reducing
        // without the binding (a confusing error in the file) rather than by name; the scan cannot
        // go stale. The corpus test (`legacy_contracts.rs`) uses the same rule.
        let env = if source.contains("rho:rchain:deployerId") {
            &deployer_env
        } else {
            &empty_env
        };
        match rt.evaluate_with_env(&source, env, &rand).await {
            Ok(r) if r.errors.is_empty() => println!("OK   {rel}"),
            Ok(r) => failures.push(format!("{rel}: reduce errors: {:?}", r.errors)),
            Err(e) => failures.push(format!("{rel}: parse/normalize error: {e}")),
        }
    }

    for f in &failures {
        println!("FAIL {f}");
    }
    assert!(
        failures.is_empty(),
        "{} of {} examples failed",
        failures.len(),
        EXAMPLES.len()
    );
}
