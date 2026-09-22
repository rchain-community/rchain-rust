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

/// A deploy signed by a **real** key pair, so `rho:rchain:deployerId` is a genuine public key. Terms
/// that derive a REV address from it need that: `RevAddress!("fromPublicKey", …)` matches nothing
/// for a placeholder byte-array, and the call then stalls silently — which is what a governance
/// handshake does before it answers.
fn deploy_signed_by(term: &str, seed: u8) -> SignedDeployData {
    use rchain_crypto::private_key::PrivateKey;
    use rchain_crypto::signatures::secp256k1::Secp256k1;
    use rchain_crypto::signatures::signatures_alg::SignaturesAlg;
    let sk = PrivateKey::new(vec![seed; 32]);
    let pk = Secp256k1
        .to_public(&sk)
        .expect("a fixed 32-byte scalar is a valid secp256k1 key");
    let mut d = deploy(term);
    d.deployer = pk.bytes().to_vec();
    d
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

/// A probe for a contract reached by its **constant URI**.
///
/// `destructure` is the pattern the *consumer* uses, and it differs by contract family: the node's
/// own blessed contracts register signed, so they destructure `@(_, X)` (a `(nonce, value)` pair),
/// while the vendored rgov classes register exactly as upstream does (`insertArbitrary`) and
/// destructure the **bare** value `X`. Using the wrong one is exactly the failure this file exists to
/// catch: it matches nothing and the probe simply never reports.
fn lookup_probe(uri: &str, tag: &str, destructure: &str, call: &str, reply_names: &str) -> String {
    format!(
        r#"new rl(`rho:registry:lookup`), ch, ret, log in {{
             rl!(`{uri}`, *ch) |
             for ({destructure} <- ch) {{
               @"out"!("{tag}:resolved") |
               {call} |
               for ({reply_names} <- ret) {{ @"out"!("{tag}:called") }}
             }}
           }}"#
    )
}

/// On a fresh chain every vendored rgov class contract is installed under its constant URI and
/// answers a call. This is what a governance client gets instead of running
/// `scripts/bootstrap-rgov.ts` and recording whatever URI the deployment happened to produce.
#[test]
fn a_fresh_chain_installs_the_rgov_contracts_and_they_answer() {
    with_big_stack(async {
        let rm = build_runtime_manager().await;
        let rand = fixed_rand();
        let uris = rchain_casper::genesis::rgov::contract_uris().expect("the vendored URIs");
        let uri = |name: &str| {
            uris.iter()
                .find(|(n, _)| *n == name)
                .map(|(_, u)| u.clone())
                .unwrap_or_else(|| panic!("{name} must be vendored"))
        };

        // Each call is upstream's own shape (its contract signature, or its self-test's call), and
        // `reply_names` is that contract's reply *arity*: `Kudos` and the member directory answer
        // with one value, `Directory` with its capability map, `Inbox` with three capabilities and
        // `Issue` with `(admin, tally)`. A pattern of the wrong arity matches nothing, which is the
        // same class of silence this file exists to turn into a failure.
        let probes: Vec<(&str, String)> = vec![
            (
                "kudos",
                lookup_probe(&uri("kudos"), "kudos", "@X", r#"@X!("peek", *ret)"#, "@_"),
            ),
            (
                "inbox",
                lookup_probe(&uri("inbox"), "inbox", "@X", r#"@X!(*ret)"#, "@_, @_, @_"),
            ),
            (
                "directory",
                lookup_probe(
                    &uri("directory"),
                    "directory",
                    "@X",
                    r#"@X!(Nil, *ret)"#,
                    "@_",
                ),
            ),
            (
                "roll",
                lookup_probe(&uri("roll"), "roll", "@X", r#"@X!("make", {}, *ret)"#, "@_"),
            ),
            (
                "issue",
                lookup_probe(
                    &uri("issue"),
                    "issue",
                    "@X",
                    r#"@X!(["proposal"], *ret, *log)"#,
                    "@_, @_",
                ),
            ),
            // The three classes upstream's template does not list, but the wallet's editor asks the
            // directory for by name — installed and callable like the rest.
            (
                "chat",
                lookup_probe(&uri("chat"), "chat", "@X", r#"@X!(*ret)"#, "@_, @_, @_"),
            ),
            (
                "ballot",
                lookup_probe(
                    &uri("ballot"),
                    "ballot",
                    "@X",
                    r#"@X!(["p1"], *ret, *log)"#,
                    "@_, @_",
                ),
            ),
            (
                "group",
                lookup_probe(&uri("group"), "group", "@X", r#"@X!("lookup", *ret)"#, "@_"),
            ),
        ];

        let mut terms = default_blessed_terms(
            &proof_of_stake(),
            &Registry {
                system_contract_pub_key: String::new(),
            },
            &[],
            "root",
        )
        .expect("blessed terms");
        let blessed = terms.len();
        assert!(
            blessed >= 8,
            "the blessed set includes the libraries and the five vendored contracts (got {blessed})"
        );
        terms.extend(probes.iter().map(|(_, term)| deploy(term)));

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

        // Installed, separately from callable: a direct native read attributes a failure to the
        // registration rather than to the probe.
        let native =
            rchain_rholang::native_state::NativeSystemState::new(rm.runtime().native_store());
        for (name, uri) in &uris {
            assert!(
                native.registry_lookup(uri).await.expect("lookup").is_some(),
                "{name} must be registered under {uri}"
            );
        }
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
        // Two stages per probe, so a failure says *where*: the lookup/destructure, or the call.
        let mut expected: Vec<String> = probes
            .iter()
            .flat_map(|(name, _)| [format!("{name}:resolved"), format!("{name}:called")])
            .collect();
        expected.sort();
        assert_eq!(
            tags, expected,
            "every vendored contract must be installed, resolvable by its constant URI, and callable"
        );
    });
}

/// The wallet's first governance step, on a fresh chain and with no bootstrap: resolve the master
/// read cap by its constant key and get the directory to answer `GetMe`.
///
/// This half is what `scripts/bootstrap-rgov.ts` used to arrange at runtime (with a recorded URI
/// that goes stale per chain). Genesis arranges it instead, for the fixed testnet key, so a client
/// hardcodes `readcap_uri()` and needs no bootstrap.
///
/// The remainder of the handshake — calling `GetMe` and receiving the deployer's stuff — is verified
/// on a node rather than here: the feature's own contract logs are where a stall in that half is
/// legible, and in-process a failed call and an unanswered one look identical (both are silence).
/// See `docs/src/node/devnet.md`.
#[test]
fn a_fresh_chain_serves_the_wallets_new_inbox_handshake() {
    with_big_stack(async {
        let rm = build_runtime_manager().await;
        let rand = fixed_rand();
        let readcap = rchain_casper::genesis::rgov::readcap_uri().expect("the read cap key");
        let handshake = format!(
            r#"new rl(`rho:registry:lookup`), deployerId(`rho:rchain:deployerId`),
                 capCh, getMeCh, stuffCh
               in {{
                 rl!(`{readcap}`, *capCh) |
                 // Bind bare, call bare — the convention the rgov contracts themselves use
                 // (`memberIdGovRev`'s imports, the master-directory template). Mixing it with the
                 // wallet's `for (@X <- ch)` + `@X!` is a parse error, not a silent miss.
                 for (MCAread <- capCh) {{
                   MCAread!("GetMe", *getMeCh) |
                   for (GetMe <- getMeCh) {{
                     @"out"!("got-getme") |
                     new logCh in {{
                       // A *drain*, and a repeated one: the feature logs multi-element lines, and
                       // `rho:io:stdout` takes one datum — pointing the log at stdout makes the
                       // contract error mid-flow, which is a foot-gun for any caller.
                       for (@_line <= logCh) {{ @"out"!("getme-logged") }} |
                       GetMe!(*deployerId, *stuffCh, *logCh) |
                       for (@_reply <- stuffCh) {{ @"out"!("getme-answered") }}
                     }}
                   }}
                 }}
               }}"#
        );

        let mut terms = default_blessed_terms(
            &proof_of_stake(),
            &Registry {
                system_contract_pub_key: String::new(),
            },
            &[],
            "root",
        )
        .expect("blessed terms");
        terms.push(deploy_signed_by(&handshake, 7));

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

        let produced = rm
            .runtime()
            .get_data_par(&rchain_models::sorted::SortedProc::new(
                rchain_models::par_ops::from_expr(rchain_models::ast::Expr::GString(
                    "out".to_string(),
                )),
            ))
            .await
            .expect("read the handshake's output channel");
        let tags: Vec<String> = produced
            .iter()
            .filter_map(|p| {
                rchain_models::rholang::RhoType::RhoString::unapply(p).map(str::to_string)
            })
            .collect();
        assert!(
            tags.contains(&"got-getme".to_string()),
            "the read cap must resolve to a directory that answers `GetMe`: {tags:?}"
        );
        assert!(
            tags.contains(&"getme-logged".to_string()),
            "`getMe` must run — it is reached through the directory and logs before the first step \
             that can stall: {tags:?}"
        );

        // What is *not* yet pinned here, and the honest boundary of this test: `getMe` then enters
        // the feature's own `createMe` flow (it logs four lines) and stops before answering, so
        // `getme-answered` does not appear. That is upstream contract logic, not the genesis
        // installation — on a node the feature's log lines are where it is legible, and the open
        // item is recorded in `spec/GENESIS.md`. Asserting it here today would only assert the
        // silence this file exists to distinguish from a failure.
    });
}

/// The one order that matters, demonstrated: `MakeMint` resolves `rho:lang:nonNegativeNumber`
/// **during its own deploy** (`MakeMint.rho:27`), and `rho:registry:lookup` answers `Nil` when the
/// counter is not installed yet. Its install gate waits on a reply pattern a `Nil` cannot match, so
/// with the wrong order the deploy *succeeds* and `MakeMint` never registers — the silence this
/// whole task has been about. What ends that silence is the genesis ceremony's completeness check,
/// and this test pins both halves: the silence, and the check that turns it into a failed genesis.
///
/// (`roll` needs no such ordering: it resolves its `directory`/`inbox` imports per *call*, so its
/// position in the list is free — a negative test for it is what established that.)
#[test]
fn installing_make_mint_before_its_dependency_is_caught_by_the_genesis_check() {
    with_big_stack(async {
        let rm = build_runtime_manager().await;
        let rand = fixed_rand();
        use rchain_casper::genesis::standard_deploys::StandardDeploys;
        // `make_mint` before `non_negative_number`: the only difference from the blessed order.
        let terms = vec![
            StandardDeploys::list_ops("root").expect("list_ops"),
            StandardDeploys::make_mint("root").expect("make_mint"),
            StandardDeploys::non_negative_number("root").expect("non_negative_number"),
        ];

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
                "no deploy may fail — the misordering is silent, which is the point \
                 (deploy #{i}: {:?})",
                r.eval_result.errors
            );
        }

        // The class never registered, so its shorthand could not be seeded…
        let native =
            rchain_rholang::native_state::NativeSystemState::new(rm.runtime().native_store());
        let missing = rchain_casper::genesis::missing_genesis_aliases(&native)
            .await
            .expect("the completeness check runs");
        assert!(
            missing.contains(&"rho:rchain:makeMint"),
            "the misordered make_mint must be reported missing, got {missing:?}"
        );
        // …which is exactly what the genesis ceremony refuses to start with.
        assert!(
            !missing.is_empty(),
            "a chain that started here would answer `Nil` to that lookup forever"
        );
    });
}
