//! Genesis registry content.
//!
//! A fresh chain's `rho:registry:lookup` must resolve the shorthands consumers actually reach for,
//! and the value it hands back must be **callable** — not merely non-`Nil`. The distinction is the
//! whole point: an unmatched `for` is not an error in rholang, so a lookup that resolves to nothing
//! usable produces a silent no-op that reads exactly like a client bug (this is how the rgov family
//! came to return `[]`). `spec/GENESIS.md` is the manifest these tests pin, with the consumer
//! evidence for every entry.
//!
//! Each probe below is written the way the *consumer* writes it, with the consumer named — a shape
//! this port accepts but a client does not would be worthless.

mod common;

use rchain_casper::genesis::contracts::{ProofOfStake, Registry};
use rchain_casper::genesis::default_blessed_terms;
use rchain_crypto::hash::blake2b512_random::Blake2b512Random;
use rchain_models::casper::protocol::casper_message::{DeployData, SignedDeployData};
use rchain_rholang::native_state::PosGenesis;
use rchain_rholang::system_processes::BlockData;

use common::build_runtime_manager;

fn fixed_rand() -> Blake2b512Random {
    Blake2b512Random::from_init(&[7u8; 32])
}

fn deploy(term: &str) -> SignedDeployData {
    SignedDeployData {
        data: DeployData {
            attachments: Vec::new(),
            term: term.to_string(),
            timestamp: 0,
            phlo_price: 1,
            phlo_limit: 900_000,
            valid_after_block_number: 0,
            shard_id: "root".to_string(),
        },
        deployer: vec![0u8; 32],
        sig: Vec::new(),
        sig_algorithm: "secp256k1".to_string(),
    }
}

/// Run an async body on a worker thread with the node's 32 MiB stack. The blessed genesis terms
/// recurse deeper than the default 2 MiB test stack allows, and so does normalizing them — this
/// mirrors `node/src/main.rs` and `node/tests/common/mod.rs`.
fn with_big_stack<F>(body: F)
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    const STACK: usize = 32 * 1024 * 1024;
    std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(|| {
            tokio::runtime::Builder::new_multi_thread()
                .thread_stack_size(STACK)
                .enable_all()
                .build()
                .expect("a tokio runtime")
                .block_on(body)
        })
        .expect("spawn the test thread")
        .join()
        .expect("the test thread panicked");
}

fn proof_of_stake() -> ProofOfStake {
    ProofOfStake {
        minimum_bond: 1,
        maximum_bond: 100,
        validators: Vec::new(),
        epoch_length: 1,
        quarantine_length: 1,
        number_of_active_validators: 1,
        pos_multi_sig_public_keys: Vec::new(),
        pos_multi_sig_quorum: 1,
        pos_vault_pub_key: String::new(),
    }
}

/// One probe per seeded shorthand: look it up, destructure the `(nonce, value)` pair the way the
/// consumer does, **call** the value, and only then report the tag. The call's reply is what proves
/// reachability — a lookup that yields a name nothing answers on never reaches the tag.
///
/// The two destructuring conventions are both real consumer code: `@(_, X)` + `@X!(…)` is the
/// wallet (`r-wallet/src/utils/rho.ts:31`), `@(_, *X)` + `X!(…)` is rgov
/// (`rgov/rholang/core/Issue.rho`). Each probe uses the form of the consumer it is named for.
fn probes() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "rho:rchain:revVault",
            r#"new rl(`rho:registry:lookup`), ch, ret in {
                 rl!(`rho:rchain:revVault`, *ch) |
                 for (@(_, RevVault) <- ch) {
                   @RevVault!("getBalance", "111111111111111111111111111111111111111111111111111111", *ret) |
                   for (@_ <- ret) { @"out"!("rho:rchain:revVault") }
                 }
               }"#,
        ),
        (
            "rho:rchain:pos",
            r#"new rl(`rho:registry:lookup`), ch, ret in {
                 rl!(`rho:rchain:pos`, *ch) |
                 for (@(_, PoS) <- ch) {
                   @PoS!("getBonds", *ret) |
                   for (@_ <- ret) { @"out"!("rho:rchain:pos") }
                 }
               }"#,
        ),
        (
            "rho:lang:listOps",
            r#"new rl(`rho:registry:lookup`), ch, ret in {
                 rl!(`rho:lang:listOps`, *ch) |
                 for (@(_, *ListOps) <- ch) {
                   ListOps!("range", 0, 2, *ret) |
                   for (@_ <- ret) { @"out"!("rho:lang:listOps") }
                 }
               }"#,
        ),
        (
            "rho:lang:nonNegativeNumber",
            r#"new rl(`rho:registry:lookup`), ch, ret in {
                 rl!(`rho:lang:nonNegativeNumber`, *ch) |
                 for (@(_, *NonNegativeNumber) <- ch) {
                   NonNegativeNumber!(3, *ret) |
                   for (@_ <- ret) { @"out"!("rho:lang:nonNegativeNumber") }
                 }
               }"#,
        ),
        (
            "rho:rchain:makeMint",
            r#"new rl(`rho:registry:lookup`), ch, ret in {
                 rl!(`rho:rchain:makeMint`, *ch) |
                 for (@(nonce, *MakeMint) <- ch) {
                   MakeMint!(*ret) |
                   for (@_ <- ret) { @"out"!("rho:rchain:makeMint") }
                 }
               }"#,
        ),
    ]
}

/// On a fresh chain every seeded shorthand resolves **and its value answers a call**.
#[test]
fn a_fresh_chain_resolves_and_can_call_every_seeded_shorthand() {
    with_big_stack(async {
        let rm = build_runtime_manager().await;
        let rand = fixed_rand();

        let mut terms = default_blessed_terms(
            &proof_of_stake(),
            &Registry {
                system_contract_pub_key: String::new(),
            },
            &[],
            "root",
        )
        .expect("the blessed term list builds");
        assert!(
            !terms.is_empty(),
            "genesis must install something — an empty list is the defect this test exists for"
        );
        let expected: Vec<&str> = probes().iter().map(|(name, _)| *name).collect();
        terms.extend(probes().iter().map(|(_, term)| deploy(term)));

        let (_, _, results) = rm
            .compute_genesis(
                &terms,
                &rand,
                BlockData::empty(),
                &PosGenesis::default(),
                &[],
            )
            .await
            .expect("compute_genesis");
        for (i, r) in results.iter().enumerate() {
            assert!(
                r.eval_result.succeeded(),
                "genesis deploy #{i} failed: {:?}",
                r.eval_result.errors
            );
        }

        // Each probe reported its tag only after its call was answered.
        let produced = rm
            .runtime()
            .get_data_par(&rchain_models::sorted::SortedProc::new(
                rchain_models::par_ops::from_expr(rchain_models::ast::Expr::GString(
                    "out".to_string(),
                )),
            ))
            .await
            .expect("read the probes' output channel");
        let mut tags: Vec<String> = produced
            .iter()
            .filter_map(|p| {
                rchain_models::rholang::RhoType::RhoString::unapply(p).map(str::to_string)
            })
            .collect();
        tags.sort();
        let mut expected = expected;
        expected.sort();
        assert_eq!(
            tags, expected,
            "every seeded shorthand must resolve AND answer a call"
        );
    });
}

/// Two genesis ceremonies over fresh stores must agree on the registered values — the property that
/// makes the aliases chain-independent (a consumer hardcodes them; `spec/GENESIS.md`).
#[test]
fn the_seeded_registry_is_identical_across_fresh_genesis_ceremonies() {
    with_big_stack(async {
        let mut digests = Vec::new();
        for _ in 0..2 {
            let rm = build_runtime_manager().await;
            let rand = fixed_rand();
            let terms = default_blessed_terms(
                &proof_of_stake(),
                &Registry {
                    system_contract_pub_key: String::new(),
                },
                &[],
                "root",
            )
            .expect("blessed terms");
            rm.compute_genesis(
                &terms,
                &rand,
                BlockData::empty(),
                &PosGenesis::default(),
                &[],
            )
            .await
            .expect("compute_genesis");
            let native =
                rchain_rholang::native_state::NativeSystemState::new(rm.runtime().native_store());
            let mut entries = Vec::new();
            for alias in rchain_casper::genesis::standard_deploys::GENESIS_ALIASES {
                let value = native
                    .registry_lookup(alias.shorthand)
                    .await
                    .expect("lookup")
                    .unwrap_or_else(|| panic!("{} must be seeded", alias.shorthand));
                entries.push(format!("{} = {value:?}", alias.shorthand));
            }
            digests.push(entries.join("\n"));
        }
        assert_eq!(
            digests[0], digests[1],
            "genesis registry content must not depend on the ceremony"
        );
        assert!(
            digests[0].lines().count() >= 5,
            "all seeded aliases are covered"
        );
    });
}
