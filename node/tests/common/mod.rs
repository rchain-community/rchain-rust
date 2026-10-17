//! Shared harness for node-level integration tests.
//!
//! Assembles a real standalone node in-process (mirroring `main.rs`: `Configuration::build` →
//! `node_environment::create` → `setup_node_program` → `serve`) over an ephemeral data dir and
//! loopback ports, then drives its gRPC + HTTP surfaces.
//!
//! Each `node/tests/*.rs` binary compiles this module separately, so helpers a given binary does not
//! use would warn there — hence the module-wide allowance.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rchain_casper::conf::{ShardMemberships, ShardSpec};
use rchain_casper::validator_identity::ValidatorIdentity;
use rchain_comm::peer_node::NodeIdentifier;
use rchain_node::configuration::configuration::parse_defaults;
use rchain_node::configuration::hocon::node_conf_from_hocon;
use rchain_node::configuration::model::NodeConf;
use rchain_node::runtime::node_environment;
use rchain_node::runtime::node_runtime::{setup_node_program, NodeProgram};
use rchain_shared::base16;
use rchain_shared::log::StderrLog;

/// The default secp256k1 validator private key (hex), port of `ConstructDeploy.defaultSec`.
pub const VALIDATOR_PRIV_HEX: &str =
    "a68a6e6cca30f81bd24a719f3145d20e8424bd7b396309b0708a16c7d8000b76";

/// The deployer (validator-0) REV address, funded in the genesis wallets file. Derived from
/// `VALIDATOR_PRIV_HEX`'s secp256k1 pubkey; a deploy signed with `VALIDATOR_PRIV_HEX` pays phlo from
/// this vault.
pub const DEPLOYER_REV_ADDR: &str = "11112VYAt8rUGNRRZX3eJdgagaAhtWTK8Js7F7X5iqddMVqyDTtYau";

/// Build a standalone `NodeConf` bound to 5 loopback ports `[http, admin-http, grpc-internal,
/// protocol, grpc-external]`, with a bonded validator (`VALIDATOR_PRIV_HEX`) and a funded deployer
/// wallet so signed deploys can pay phlo.
pub fn deploy_conf(dir: &Path, ports: &[u16]) -> NodeConf {
    assert!(
        ports.len() >= 5,
        "need [http, admin-http, grpc-internal, protocol, grpc-external]"
    );
    let mut conf = standalone_conf(dir, &ports[0..4], Some(VALIDATOR_PRIV_HEX));
    conf.api_server.port_grpc_external = ports[4] as i32;
    let wallets = conf
        .casper
        .shards
        .primary()
        .genesis_block_data
        .wallets_file
        .clone();
    std::fs::write(&wallets, format!("{DEPLOYER_REV_ADDR},1000000000000\n"))
        .expect("write wallets");
    conf
}

/// A multi-threaded tokio runtime with a large per-worker stack. The genesis blessed terms recurse
/// deeper than the default 2 MiB worker stack allows (matching the node binary's runtime).
pub fn test_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .thread_stack_size(32 * 1024 * 1024)
        .enable_all()
        .build()
        .expect("build test runtime")
}

/// Create a temporary directory for a test (caller removes it after dropping the node).
pub fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rchain-it-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Allocate `n` free loopback TCP ports. The ports are released when the returned listeners drop.
pub fn free_ports(n: usize) -> Vec<u16> {
    let mut listeners = Vec::with_capacity(n);
    let mut ports = Vec::with_capacity(n);
    for _ in 0..n {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
        ports.push(listener.local_addr().expect("local addr").port());
        listeners.push(listener);
    }
    ports
}

/// Build a standalone `NodeConf` bound to loopback with the given `[http, admin-http, grpc-internal,
/// protocol]` ports. When `validator_hex` is set, a matching bonds file is written and wired into
/// genesis so the validator is bonded.
pub fn standalone_conf(dir: &Path, ports: &[u16], validator_hex: Option<&str>) -> NodeConf {
    assert!(
        ports.len() >= 4,
        "need [http, admin-http, grpc-internal, protocol] ports"
    );
    let defaults = parse_defaults(dir.to_str().unwrap()).expect("parse defaults");
    let mut conf = node_conf_from_hocon(&defaults).expect("node conf from hocon");

    conf.storage.data_dir = dir.to_path_buf();
    conf.api_server.host = "127.0.0.1".to_string();
    conf.api_server.port_http = ports[0] as i32;
    conf.api_server.port_admin_http = ports[1] as i32;
    conf.api_server.port_grpc_internal = ports[2] as i32;
    conf.protocol_server.port = ports[3] as i32;

    conf.standalone = true;
    // **A test node is a dev network**, exactly as `tools/devnet.sh` treats every node it starts.
    // This is what exempts it from the production-genesis gate: Law 78 refuses to create a genesis
    // outside dev mode without a `dev-tax-address`, and a test chain is not a network that owes one
    // (its allocation is covered by `casper`'s own tests, not here).
    conf.dev_mode = true;
    conf.protocol_server.no_upnp = true;
    conf.protocol_server.host = Some("127.0.0.1".to_string());

    if let Some(hex) = validator_hex {
        conf.casper.validator_private_key = Some(hex.to_string());
        let identity = ValidatorIdentity::from_hex(hex).expect("validator identity");
        let pub_hex = base16::encode(identity.public_key.bytes());
        let bonds = dir.join("bonds.txt");
        std::fs::write(&bonds, format!("{pub_hex} 100\n")).expect("write bonds file");
        conf.casper
            .shards
            .primary_mut()
            .genesis_block_data
            .bonds_file = bonds.to_string_lossy().into_owned();
    }

    // `create_genesis_block` calls `vault_parser::parse` (not `parse_if_exists`), so the wallets
    // file must exist — an empty one yields no initial vaults.
    let wallets = dir.join("wallets.txt");
    if !wallets.exists() {
        std::fs::write(&wallets, "").expect("write wallets file");
    }
    conf.casper
        .shards
        .primary_mut()
        .genesis_block_data
        .wallets_file = wallets.to_string_lossy().into_owned();

    conf
}

/// A running in-process node: the served-program task handle and the node identifier.
pub struct TestNode {
    pub handle: tokio::task::JoinHandle<Result<(), String>>,
    pub id: NodeIdentifier,
    pub grpc_port: u16,
    pub http_port: u16,
    /// The admin HTTP port — the listener that binds loopback by default and carries the surfaces
    /// that act with the node's own key (`/api/propose`, and since AUDIT C121 the cross-shard
    /// transaction routes). Read from the config rather than passed in, so a test that needs it does
    /// not have to thread another argument through `start`.
    pub admin_port: u16,
    /// The node's stop path (AUDIT C144), held for the node's lifetime. It is a field rather than a
    /// local in `start` because a **dropped sender resolves every listener's `stop_requested` at
    /// once** — letting it fall out of scope would stop the node the instant it started, and the
    /// symptom (every test timing out on a node that answers nothing) would point at everything
    /// except this. Tests that do not stop the node never touch it.
    stop: tokio::sync::watch::Sender<bool>,
    /// **One heavyweight node program at a time** — held from setup until the node is dropped.
    ///
    /// See [`heavy_node_lock`]. It is a field for the same reason `stop` is: the lock has to live
    /// exactly as long as the node, and a local in `start` would release it while the node ran.
    ///
    /// An `Arc` because a test that runs **two** nodes ([`start_pair`]) takes the lock *once* and
    /// hands the same guard to both, so the pair holds it until the last of them drops.
    _heavy: Arc<tokio::sync::OwnedMutexGuard<()>>,
}

/// **One heavyweight node program at a time.**
///
/// Every [`start`] builds a full node: an LMDB environment with a large map, a Kademlia gRPC server,
/// an HTTP API, and the OCapN listener. `cargo test` runs one binary's tests on parallel threads, so
/// the OCapN listener's nine of them start nine nodes at once — and two things go wrong that are
/// **not** defects in the node, which is what makes them worth a lock rather than a retry:
///
/// * the runner runs out of address space for the maps, and the failure is
///   `open LMDB environment reporting: Cannot allocate memory` — measured in CI on 2026-10-07, and
///   locally;
/// * [`free_ports`] *releases* the ports it probed, so two tests that start at the same moment can be
///   handed the same one: the second fails with `Kademlia RPC server failed: Address already in use`.
///
/// Both make a red build say something untrue. Holding the lock for the node's whole life means a
/// test that starts a node runs alone from setup to teardown. It serialises within one test binary —
/// which is where both failures happen, because the binaries themselves run in sequence — and a test
/// that needs two nodes takes the lock once and shares the guard ([`start_pair`]), so there is
/// nothing for it to deadlock against.
fn heavy_node_lock() -> Arc<tokio::sync::Mutex<()>> {
    static LOCK: std::sync::OnceLock<Arc<tokio::sync::Mutex<()>>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

impl TestNode {
    /// Abort the server task outright — the blunt stop, for tests that want the process killed the
    /// way a `SIGKILL` does.
    pub fn shutdown(self) {
        self.handle.abort();
    }

    /// Ask the node to stop the way an operator's `SIGTERM` does: every listener is told, they
    /// drain, and `serve` returns. Unlike [`TestNode::shutdown`] this is the graceful path, so a
    /// test can await the handle afterwards and see what the node reported.
    pub fn request_stop(&self) {
        let _ = self.stop.send(true);
    }
}

/// Initialize the environment, assemble the node, and start serving it.
///
/// **Taken one at a time** — see [`heavy_node_lock`], which is acquired first and held until the
/// returned node is dropped.
pub async fn start(conf: &NodeConf, grpc_port: u16, http_port: u16) -> TestNode {
    let heavy = Arc::new(heavy_node_lock().lock_owned().await);
    build(conf, grpc_port, http_port, heavy).await
}

/// **Two nodes at once**, for the tests that need one node to *reach* another rather than a test
/// client to reach one.
///
/// [`start`] takes the heavy lock for the node's whole life, so calling it twice in one test would
/// wait for ever on the first node. This takes the lock once and shares the guard between the pair,
/// which is the same serialisation against every other test with neither node waiting on the other.
///
/// The two nodes must be given **disjoint** ports and data directories — `free_ports` and
/// `temp_dir` twice, not once.
pub async fn start_pair(
    first: (&NodeConf, u16, u16),
    second: (&NodeConf, u16, u16),
) -> (TestNode, TestNode) {
    let heavy = Arc::new(heavy_node_lock().lock_owned().await);
    let a = build(first.0, first.1, first.2, heavy.clone()).await;
    let b = build(second.0, second.1, second.2, heavy).await;
    (a, b)
}

/// The body of [`start`], with the heavy guard already in hand so a pair can share it.
async fn build(
    conf: &NodeConf,
    grpc_port: u16,
    http_port: u16,
    heavy: Arc<tokio::sync::OwnedMutexGuard<()>>,
) -> TestNode {
    let id = node_environment::create(conf).expect("node environment");
    let program: NodeProgram = setup_node_program(conf, &id, Arc::new(StderrLog::default()))
        .await
        .expect("setup node program");
    let (stop, stop_rx) = tokio::sync::watch::channel(false);
    let handle = tokio::spawn(program.serve(stop_rx));
    TestNode {
        handle,
        id,
        grpc_port,
        http_port,
        admin_port: conf.api_server.port_admin_http as u16,
        stop,
        _heavy: heavy,
    }
}

/// A standalone **gateway** `NodeConf`: one node that is a member of two shards (`/root` and
/// `/root/child`), each with its own genesis — bonds and a wallet funding the validator's REV
/// address, so the cross-shard escrow can pay phlo and hold a balance on both.
///
/// `propose-on-deploy` is on because a cross-shard leg is an ordinary deploy: it takes effect when a
/// block containing it is produced, which is what the gateway's coordinator waits for.
pub fn gateway_conf(dir: &Path, ports: &[u16], validator_hex: &str) -> NodeConf {
    gateway_conf_with_txn_api(dir, ports, validator_hex, true)
}

/// As [`gateway_conf`], with the cross-shard transaction API switched on or off — the flag an
/// operator controls, and therefore a gate a test has to drive from a real config.
pub fn gateway_conf_with_txn_api(
    dir: &Path,
    ports: &[u16],
    validator_hex: &str,
    enable_txn_api: bool,
) -> NodeConf {
    assert!(
        ports.len() >= 4,
        "need [http, admin-http, grpc-internal, protocol]"
    );
    let mut conf = standalone_conf(dir, ports, Some(validator_hex));
    conf.propose_on_deploy = true;
    conf.api_server.enable_txn_api = enable_txn_api;

    let identity = ValidatorIdentity::from_hex(validator_hex).expect("validator identity");
    let pub_hex = base16::encode(identity.public_key.bytes());
    let primary = conf.casper.shards.primary().clone();

    let mut specs = Vec::new();
    for (index, (name, parent)) in [("root", "/"), ("child", "/root")].into_iter().enumerate() {
        // Each shard gets its own genesis files: its own bonds, and a wallet funding the validator
        // (the node's coordinator key) so it can escrow on that shard.
        let shard_dir = dir.join(format!("genesis-{name}"));
        std::fs::create_dir_all(&shard_dir).expect("genesis dir");
        let bonds = shard_dir.join("bonds.txt");
        std::fs::write(&bonds, format!("{pub_hex} 100\n")).expect("write bonds");
        let wallets = shard_dir.join("wallets.txt");
        std::fs::write(&wallets, format!("{DEPLOYER_REV_ADDR},1000000000000\n"))
            .expect("write wallets");

        let mut genesis = primary.genesis_block_data.clone();
        genesis.bonds_file = bonds.to_string_lossy().into_owned();
        genesis.wallets_file = wallets.to_string_lossy().into_owned();
        let _ = index;
        specs.push(
            ShardSpec::new(name.to_string(), parent.to_string(), genesis, 5).expect("shard spec"),
        );
    }
    conf.casper.shards = ShardMemberships::new(specs).expect("memberships");
    conf
}

/// Assemble a play runtime over a fresh in-memory store — the same shape `rholang/tests/common`
/// uses, for tests that need the interpreter but not a node. The corpus consumers in this crate call
/// the API's own codecs over terms, so they need a store to run a term in and nothing more.
pub async fn rho_runtime() -> rchain_rholang::runtime::RhoRuntime {
    use rchain_models::runtime::{BindPattern, ListParWithRandom, TaggedContinuation};
    use rchain_models::sorted::SortedProc;
    use rchain_rholang::storage::RhoMatch;
    use rchain_rspace::factory::create_history_repository;
    use rchain_rspace::hot_store::InMemHotStore;
    use rchain_rspace::rspace::RSpace;
    use rchain_shared::store_manager::InMemoryStoreManager;

    let manager = InMemoryStoreManager::default();
    let history = create_history_repository::<
        SortedProc,
        BindPattern,
        ListParWithRandom,
        TaggedContinuation,
    >(&manager, "rspace")
    .await
    .expect("history repository");
    let reader = history.get_history_reader(history.root()).await;
    let hot = Arc::new(InMemHotStore::new(reader.base()));
    let (play, _replay) = RSpace::create_with_replay(history.clone(), hot, Arc::new(RhoMatch));
    rchain_rholang::runtime::RhoRuntime::create(play, history, SortedProc::default())
        .await
        .expect("rho runtime")
}
