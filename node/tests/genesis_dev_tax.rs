//! **Law 78 on a real node** — the genesis-creation gate fires when a genesis is created, and only
//! then.
//!
//! The unit tests in `casper/src/genesis/dev_tax.rs` drive the gate's *condition*; what they cannot
//! show is that the branch it sits on is reached only by a node that is actually creating a genesis.
//! That is the property a reviewer asked to see pinned, and it is the one that matters operationally:
//! a standalone node **restarting** onto a chain it already built — the public testnet's bootstrap,
//! whose genesis predates this key — must not be refused for a decision it already made.
//!
//! The falsifier is sharp. Move the check into `Configuration::build`, where the first draft argued it
//! belonged, and the second boot in the test below never reaches `/api/status`: the node is refused on
//! a config it has been running on for the whole of its first life.

mod common;

use std::time::Duration;

use common::{deploy_conf, free_ports, start, temp_dir, test_runtime};

/// Poll until the node's HTTP API answers at all — the node is serving, not merely spawned.
async fn wait_for_status(client: &reqwest::Client, base: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        if let Ok(resp) = client.get(format!("{base}/api/status")).send().await {
            if resp.status().is_success() {
                return;
            }
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the node never answered /api/status: {base}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Boot a node on `conf`, wait until it serves, ask it to stop, and return when it has.
async fn boot_and_stop(conf: &rchain_node::configuration::model::NodeConf, ports: &[u16]) {
    let mut node = start(conf, ports[4], ports[0]).await;
    let base = format!("http://127.0.0.1:{}", ports[0]);
    let client = reqwest::Client::new();
    wait_for_status(&client, &base).await;

    node.request_stop();
    let served = tokio::time::timeout(Duration::from_secs(30), node.handle)
        .await
        .expect("a node that was asked to stop must return rather than serve on (AUDIT C144)")
        .expect("the serve task must not panic");
    served.expect("a stop on the operator's word is not a server error");
}

/// **A chain created without an allocation restarts outside dev mode without being re-refused.**
///
/// The first boot is the ordinary devnet shape — standalone, an empty DAG, an empty
/// `dev-tax-address`, dev mode — and it creates genesis. The second is the same chain, on the same
/// data directory, **outside** dev mode: the DAG is no longer empty, so no genesis is created, and
/// the gate must not fire on the empty address the config still carries.
#[test]
fn an_existing_allocation_free_chain_restarts_outside_dev_mode() {
    test_runtime().block_on(async {
        let dir = temp_dir("dev-tax-restart");
        let ports = free_ports(5);
        let mut conf = deploy_conf(&dir, &ports);
        assert!(
            conf.dev_mode,
            "the harness starts in dev mode, which is how this chain gets its genesis at all"
        );
        assert!(
            conf.casper
                .shards
                .primary()
                .genesis_block_data
                .dev_tax_address
                .is_empty(),
            "an allocation-free chain is the case the restart must not re-decide"
        );

        boot_and_stop(&conf, &ports).await;

        // The same chain, the same data directory — and therefore the same, now non-empty DAG —
        // outside dev mode. **Fresh ports, written into the config**: `free_ports` returns numbers and
        // drops the listeners that reserved them, so a port a stopped node just released can still be
        // in `TIME_WAIT`, and the node would fail to *bind* rather than fail the assertion under test.
        // Nothing about the gate depends on the ports.
        let ports = free_ports(5);
        conf.api_server.port_http = ports[0] as i32;
        conf.api_server.port_admin_http = ports[1] as i32;
        conf.api_server.port_grpc_internal = ports[2] as i32;
        conf.protocol_server.port = ports[3] as i32;
        conf.api_server.port_grpc_external = ports[4] as i32;
        conf.dev_mode = false;
        boot_and_stop(&conf, &ports).await;
    });
}
