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

use std::sync::Arc;
use std::time::Duration;

use common::{deploy_conf, free_ports, start, temp_dir, test_runtime};
use rchain_crypto::public_key::PublicKey;
use rchain_node::runtime::node_environment;
use rchain_node::runtime::node_runtime::setup_node_program;
use rchain_rholang::util::rev_address::RevAddress;
use rchain_shared::log::StderrLog;
use rchain_shared::refined::NonNegI64;

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

/// Poll until the node's `/api/blocks` reports a genesis block.
async fn wait_for_genesis(client: &reqwest::Client, base: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        if let Ok(resp) = client.get(format!("{base}/api/blocks")).send().await {
            if let Ok(serde_json::Value::Array(a)) = resp.json::<serde_json::Value>().await {
                if !a.is_empty() {
                    return;
                }
            }
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the node never produced a genesis block: {base}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Boot a node on `conf`, wait until it serves, ask it to stop, and return when it has.
async fn boot_and_stop(conf: &rchain_node::configuration::model::NodeConf, ports: &[u16]) {
    let node = start(conf, ports[4], ports[0]).await;
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

fn dev_address() -> String {
    RevAddress::from_public_key(&PublicKey::new(vec![2u8; 65]))
        .expect("dev address")
        .to_base58()
}

/// **A fresh production genesis with a valid recipient installs exactly one derived allocation.**
///
/// This is the missing row of Aria's startup table: the descriptor test proves the arithmetic, but
/// only a real node boot outside dev mode proves the allocation reaches the genesis state. The
/// wallets file is the fixture's 10,000 REV, below the 1B-REV crossover, so the floor is the amount.
/// The recipient's ordinary vault must stay empty — the allocation vests, it is not spendable at
/// genesis.
#[test]
fn a_fresh_production_genesis_installs_the_developer_allocation() {
    test_runtime().block_on(async {
        let dir = temp_dir("dev-tax-fresh-production");
        let ports = free_ports(5);
        let mut conf = deploy_conf(&dir, &ports);
        conf.dev_mode = false;
        let dev = dev_address();
        {
            let gbd = &mut conf.casper.shards.primary_mut().genesis_block_data;
            gbd.dev_tax_address = dev.clone();
            gbd.dev_tax_vesting_blocks = 18_934_560;
        }

        let node = start(&conf, ports[4], ports[0]).await;
        let base = format!("http://127.0.0.1:{}", ports[0]);
        let client = reqwest::Client::new();
        wait_for_status(&client, &base).await;
        wait_for_genesis(&client, &base).await;

        let leaf = node
            .dev_vesting()
            .await
            .expect("a valid production genesis installs the allocation");
        assert_eq!(leaf.address, dev);
        let floor = NonNegI64::try_from(20_000_000i64 * 100_000_000).unwrap();
        assert_eq!(
            leaf.allocation, floor,
            "the wallet total is below the crossover"
        );
        assert_eq!(leaf.unreleased, floor, "nothing has vested at genesis");
        assert_eq!(
            i64::from(node.pos_params().await.dev_tax_vesting_blocks),
            18_934_560
        );

        let balance = node
            .native_state()
            .vault_balance(&dev)
            .await
            .expect("read the recipient vault")
            .map(|b| i64::from(b))
            .unwrap_or(0);
        assert_eq!(
            balance, 0,
            "the allocation is a vesting account, not a spendable genesis balance"
        );

        node.request_stop();
        let _ = tokio::time::timeout(Duration::from_secs(30), node.handle)
            .await
            .expect("a node that was asked to stop must return")
            .expect("the serve task must not panic");
    });
}

/// **A wallets file that names the recipient is refused before a production genesis exists.**
///
/// The refusal can happen while the descriptors are built or when the genesis is created; both are
/// the same rule, and the test accepts either rather than pinning an implementation seam. What it
/// will not accept is a node that serves a genesis with the recipient also holding a wallet row.
#[test]
fn a_production_genesis_whose_wallets_file_names_the_recipient_is_refused() {
    test_runtime().block_on(async {
        let dir = temp_dir("dev-tax-recipient-row");
        let ports = free_ports(5);
        let mut conf = deploy_conf(&dir, &ports);
        conf.dev_mode = false;
        let dev = dev_address();
        let wallets = dir.join("wallets.txt");
        let mut rows = std::fs::read_to_string(&wallets).expect("read wallets");
        rows.push_str(&format!("{dev},1000000000000\n"));
        std::fs::write(&wallets, rows).expect("write wallets");
        conf.casper
            .shards
            .primary_mut()
            .genesis_block_data
            .dev_tax_address = dev;

        let id = node_environment::create(&conf).expect("node environment");
        match setup_node_program(&conf, &id, Arc::new(StderrLog::default())).await {
            Err(e) => assert!(
                e.contains("already names the developer address"),
                "the refusal must name the recipient row: {e}"
            ),
            Ok(program) => {
                let (stop, stop_rx) = tokio::sync::watch::channel(false);
                let handle = tokio::spawn(program.serve(stop_rx));
                let served = tokio::time::timeout(Duration::from_secs(30), handle)
                    .await
                    .expect("the refused node must return rather than serve")
                    .expect("the serve task must not panic");
                let err =
                    served.expect_err("a wallets row naming the recipient must refuse the genesis");
                assert!(
                    err.contains("already names the developer address"),
                    "the refusal must name the recipient row: {err}"
                );
                drop(stop);
            }
        }
    });
}
