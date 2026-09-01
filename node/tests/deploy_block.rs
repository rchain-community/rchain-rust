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
            "root",
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
