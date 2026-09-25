//! Deploy → block integration test.
//!
//! This is the coverage `cargo test --workspace` was missing: it boots a real standalone validator,
//! submits a signed deploy, and produces a block containing it. `node_api.rs` only checks *genesis*
//! over HTTP — it never exercises the block-production path, which is where the
//! `BugError (seqNum 1)` proposal failure lived.

mod common;

use std::time::Duration;

use rchain_casper::protocol::client::{
    build_par, DeployRuntime, DeployService, GrpcDeployService, GrpcProposeService, Name,
    ProposeService,
};
use rchain_crypto::private_key::PrivateKey;
use rchain_crypto::signatures::secp256k1::Secp256k1;
use rchain_crypto::signatures::signed::Signed;
use rchain_models::casper::protocol::casper_message::{DeployData, SignedDeployData};
use rchain_models::casper::protocol::deploy_service::{BlocksQuery, DataAtNameQuery};
use rchain_shared::base16;

use common::{deploy_conf, free_ports, temp_dir, test_runtime, VALIDATOR_PRIV_HEX};

/// Poll the node's HTTP `/api/blocks` until the genesis block appears.
async fn wait_for_genesis(base: &str) {
    let client = reqwest::Client::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        if let Ok(resp) = client.get(format!("{base}/api/blocks")).send().await {
            if resp.status().is_success() {
                if let Ok(serde_json::Value::Array(a)) = resp.json::<serde_json::Value>().await {
                    if !a.is_empty() {
                        return;
                    }
                }
            }
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "genesis never appeared: {base}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// **`--autopropose`: the chain grows with nothing sent to it.** The autopropose tap (which enqueues a
/// propose on every validated block) and the interval timer beside it (which covers the case where
/// nothing happens at all — a lone validator has no peers to send it blocks) are what a devnet runs
/// on. **No test set `autopropose`**, so both were unexecuted: `setup_shard_runtime`'s taps in
/// `node/src/runtime/node_runtime.rs` never ran their bodies.
///
/// This boots a standalone validator with it on, sends *nothing*, and waits for a second block. The
/// dev-mode dummy deploy is enabled because that is what the timer's own comment names as the source
/// of a block with no client deploy — without it a propose has nothing to propose.
#[test]
fn autopropose_grows_the_chain_without_a_deploy() {
    test_runtime().block_on(async {
        let dir = temp_dir("autopropose");
        let ports = free_ports(5);
        let mut conf = deploy_conf(&dir, &ports);
        conf.autopropose = true;
        conf.dev_mode = true;
        let node = common::start(&conf, ports[2] as u16, ports[0] as u16).await;
        let base = format!("http://127.0.0.1:{}", ports[0]);

        wait_for_genesis(&base).await;

        // `/api/blocks/:start/:end` is the height range (the depth form answers only the head), so it
        // is the read that can show growth. The timer's interval is 2s; 45s is slack for a loaded
        // machine, and the assertion names the count so a slow run is legible.
        let client = reqwest::Client::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(45);
        let mut blocks = 0usize;
        loop {
            if let Ok(resp) = client.get(format!("{base}/api/blocks/0/16")).send().await {
                if let Ok(serde_json::Value::Array(a)) = resp.json::<serde_json::Value>().await {
                    blocks = blocks.max(a.len());
                    if blocks >= 2 {
                        break;
                    }
                }
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "autopropose produced no second block in 45s ({blocks} block(s))"
            );
            tokio::time::sleep(Duration::from_millis(200)).await;
        }

        node.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    });
}

#[test]
fn deploy_is_processed_into_a_block() {
    test_runtime().block_on(async {
        let dir = temp_dir("deploy-block");
        let ports = free_ports(5); // http, admin-http, grpc-internal, protocol, grpc-external
        let conf = deploy_conf(&dir, &ports);
        let node = common::start(&conf, ports[2] as u16, ports[0] as u16).await;
        let base = format!("http://127.0.0.1:{}", ports[0]);

        wait_for_genesis(&base).await;

        // Deploy `@"hello"!("world")` (deployer = the funded wallet's key).
        let rho = dir.join("hello.rho");
        std::fs::write(&rho, "@\"hello\"!(\"world\")\n").expect("write rho source");
        let deploy = GrpcDeployService::connect("127.0.0.1", ports[4] as i32, 16 * 1024 * 1024)
            .await
            .expect("deploy gRPC");
        DeployRuntime::deploy_file_program(
            &deploy,
            1000000,
            1,
            -1,
            &PrivateKey::new(base16::decode(VALIDATOR_PRIV_HEX).expect("decode key")),
            rho.to_str().expect("rho path"),
            "/root",
        )
        .await
        .expect("deploy accepted");

        // Propose: the block-production path the suite was missing.
        let propose = GrpcProposeService::connect("127.0.0.1", ports[2] as i32, 16 * 1024 * 1024)
            .await
            .expect("propose gRPC");
        propose
            .propose(false)
            .await
            .expect("propose produced a block");

        // The proposed block (with the deploy) is in the DAG.
        let blocks = deploy
            .get_blocks(&BlocksQuery { depth: 5 })
            .await
            .expect("blocks");
        assert!(
            blocks.contains("block 1"),
            "proposed block missing from DAG:\n{blocks}"
        );

        // The deploy's send must be observable: query the `"hello"` channel and assert the `"world"`
        // datum was recorded (exercises the deploy_log → is_listening_name_reduced path).
        let query_par =
            build_par(&Name::PubName("\"hello\"".to_string())).expect("build query name");
        let data = deploy
            .listen_for_data_at_name(&DataAtNameQuery {
                depth: 50,
                name: query_par,
            })
            .await
            .expect("listen-data-at-name");
        let world = build_par(&Name::PubName("\"world\"".to_string())).expect("build world datum");
        assert!(
            data.iter().any(|d| d.post_block_data.contains(&world)),
            "expected `\"world\"` at `\"hello\"`, got: {data:?}"
        );

        node.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    });
}

/// **A failed deploy's status carries the reason the reducer gave it** (`block_api_impl.rs`'s
/// `deploy_error_text`).
///
/// The Scala reads the message out of the node's per-node execution tracker and returns
/// `"<deploy error message not available in cache or deploy executed on another node>"` when it has
/// nothing — which is what a node that did not run the deploy always has. This port records the
/// reason in the block itself (`ProcessedDeploy.system_deploy_error`, issue #15), so the answer is
/// available to every node holding the block. Before this test the read path returned the placeholder
/// unconditionally, which is why a deployer could see a failure and never learn why.
///
/// The deploy is failed the cheapest deterministic way: a phlo limit of 1 against a term that costs
/// more, which the reducer reports as `OutOfPhlogistonsError`.
#[test]
fn a_failed_deploy_reports_its_reason_over_http() {
    test_runtime().block_on(async {
        let dir = temp_dir("deploy-error-status");
        let ports = free_ports(5); // http, admin-http, grpc-internal, protocol, grpc-external
        let conf = deploy_conf(&dir, &ports);
        let node = common::start(&conf, ports[2] as u16, ports[0] as u16).await;
        let base = format!("http://127.0.0.1:{}", ports[0]);
        wait_for_genesis(&base).await;

        // Sign the deploy here rather than through `deploy_file_program`, because the status route is
        // keyed by the deploy's *signature* and the helper does not return it.
        let key = PrivateKey::new(base16::decode(VALIDATOR_PRIV_HEX).expect("decode key"));
        let data = DeployData {
            attachments: Vec::new(),
            term: r#"@"never"!("runs")"#.to_string(),
            timestamp: 0,
            phlo_price: 1,
            // Too little phlo to reduce anything: the pre-charge succeeds, the deploy fails.
            phlo_limit: 1,
            valid_after_block_number: 0,
            shard_id: "/root".to_string(),
        };
        let signed = Signed::new(data, &Secp256k1, &key).expect("sign");
        let deploy_id = signed.sig.clone();
        let deploy = GrpcDeployService::connect("127.0.0.1", ports[4] as i32, 16 * 1024 * 1024)
            .await
            .expect("deploy gRPC");
        deploy
            .deploy(&SignedDeployData {
                data: signed.data,
                deployer: signed.pk.bytes().to_vec(),
                sig: signed.sig,
                sig_algorithm: signed.sig_algorithm.name().to_string(),
            })
            .await
            .expect("deploy accepted");

        let propose = GrpcProposeService::connect("127.0.0.1", ports[2] as i32, 16 * 1024 * 1024)
            .await
            .expect("propose gRPC");
        propose
            .propose(false)
            .await
            .expect("propose produced a block");

        // Poll the route until the deploy has been processed, then read the error text.
        let client = reqwest::Client::new();
        let url = format!("{base}/api/v1/deploy-status/{}", base16::encode(&deploy_id));
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        let error = loop {
            let json: serde_json::Value = client
                .get(&url)
                .send()
                .await
                .expect("GET deploy-status")
                .json()
                .await
                .expect("deploy-status json");
            // The **web** DTO's tags are PascalCase (`node/src/api/dto.rs`'s
            // `DeployExecStatus`, which is what law 43's envelope catalogue pins) while the
            // *casper* DTO of the same name renames them to camelCase — two types, one name, two
            // conventions. The route serves the web one.
            if let Some(error) = json
                .get("ProcessedWithError")
                .and_then(|e| e.get("deployError"))
                .and_then(|e| e.as_str())
            {
                break error.to_string();
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "the deploy never reached processedWithError: {json}"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        };

        assert_eq!(
            error, "Computation ran out of phlogistons.",
            "the status must carry the reducer's reason, not the placeholder the read path used to \
             return for every failed deploy"
        );
        assert!(
            !error.starts_with("<deploy error message not available"),
            "the placeholder is for a record with no message, and this block has one"
        );

        node.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    });
}
