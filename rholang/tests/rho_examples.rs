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

/// Files that bind `rho:rchain:deployerId` need the per-deploy binding injected.
const NEEDS_DEPLOYER: &[&str] = &[
    "qucalc/rholang/qucalc.rho",
    "qucalc/rholang/gov.rho",
    "qucalc/examples/multisig.rho",
];

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
];

#[tokio::test]
async fn rho_examples_parse_and_reduce() {
    let (rt, _replay) = common::build_runtime_pair().await;
    let rand = fixed_rand();
    let empty_env = BTreeMap::new();
    let deployer_env = NormalizerEnv::with_deployer_id(&PublicKey::new(vec![7u8; 65]))
        .to_env()
        .clone();

    let mut failures = Vec::new();
    for &rel in EXAMPLES {
        let source = read_rho(rel);
        let env = if NEEDS_DEPLOYER.contains(&rel) {
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
