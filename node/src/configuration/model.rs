//! Node configuration model (port of `configuration/model.scala`).

use std::path::PathBuf;
use std::time::Duration;

use rchain_casper::protocol::client::Name;
use rchain_casper::CasperConf;
use rchain_comm::peer_node::PeerNode;
use rchain_comm::transport::tls_conf::TlsConf;
use rchain_crypto::private_key::PrivateKey;
use rchain_crypto::public_key::PublicKey;

/// Root node configuration (port of the Scala `NodeConf` case class).
#[derive(Clone, Debug, PartialEq)]
pub struct NodeConf {
    pub standalone: bool,
    pub autopropose: bool,
    pub propose_on_deploy: bool,
    /// Attest to a remote block that carries deploys (see `--attest-on-new-blocks`). On by default:
    /// a validator attests without `--autopropose`.
    pub attest_on_new_blocks: bool,
    /// Turn attestation off (`--no-attest-on-new-blocks`) — the operator's escape hatch for a validator
    /// that must not add blocks. Effective attestation is `attest_on_new_blocks && !no_attest_on_new_blocks`
    /// (issue #70).
    pub no_attest_on_new_blocks: bool,
    pub protocol_server: ProtocolServer,
    pub protocol_client: ProtocolClient,
    pub peers_discovery: PeersDiscovery,
    pub api_server: ApiServer,
    pub tls: TlsConf,
    pub storage: Storage,
    pub casper: CasperConf,
    pub metrics: Metrics,
    pub dev_mode: bool,
    pub dev: DevConf,
    pub default_data_dir: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProtocolServer {
    pub network_id: String,
    pub host: Option<String>,
    pub use_random_ports: bool,
    pub dynamic_ip: bool,
    pub no_upnp: bool,
    pub port: i32,
    pub grpc_max_recv_message_size: i64,
    pub grpc_max_recv_stream_message_size: i64,
    pub max_message_consumers: i32,
    pub disable_state_exporter: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProtocolClient {
    pub network_id: String,
    pub bootstrap: PeerNode,
    pub disable_lfs: bool,
    pub batch_max_connections: i32,
    pub network_timeout: Duration,
    pub grpc_max_recv_message_size: i64,
    pub grpc_stream_chunk_size: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PeersDiscovery {
    pub port: i32,
    pub lookup_interval: Duration,
    pub cleanup_interval: Duration,
    pub heartbeat_batch_size: i32,
    pub init_wait_loop_interval: Duration,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ApiServer {
    pub host: String,
    pub port_grpc_external: i32,
    pub port_grpc_internal: i32,
    pub grpc_max_recv_message_size: i64,
    pub port_http: i32,
    pub port_admin_http: i32,
    pub max_blocks_limit: i32,
    pub enable_reporting: bool,
    /// Serve the cross-shard transaction routes (`/api/v1/txn*`) on the **admin** HTTP server. Off by
    /// default; a node that is not a gateway answers 404 either way. They are not on the public API
    /// port at all: the coordinator spends from the node's own validator REV account, so the surface
    /// is privileged like `/api/propose` (AUDIT C121).
    pub enable_txn_api: bool,
    pub enable_devnet_cors: bool,
    /// Bind the **admin** HTTP server (`/api/propose`, port `port_admin_http`) to `api_server.host`
    /// instead of loopback (AUDIT C112).
    ///
    /// Off by default, and that default is the fix: the admin server carries an **unauthenticated**
    /// `POST /api/propose` that triggers block production, and it used to bind `api_server.host` —
    /// `0.0.0.0` — unconditionally. CORS is not authentication (a non-browser client ignores it), so on
    /// a published port any host on the network could make the node propose. The internal gRPC propose
    /// service (`port_grpc_internal`, 40402) was already loopback-bound; this makes the admin HTTP
    /// server agree with it rather than the public one.
    ///
    /// The opt-in exists because the public bind was deliberate: a browser wallet reaches the admin
    /// server through a published port. It is now something an operator asks for, in the same shape as
    /// `enable_devnet_cors` beside it, instead of something every node does.
    pub enable_devnet_admin_public: bool,
    pub keep_alive_time: Duration,
    pub keep_alive_timeout: Duration,
    pub permit_keep_alive_time: Duration,
    pub max_connection_idle: Duration,
    pub max_connection_age: Duration,
    pub max_connection_age_grace: Duration,
    /// Bind an **OCapN** listener on this `host:port` (issue #249), or `None` for no listener —
    /// the default, because the only netlayer implemented is the OCapN project's
    /// `tcp-testing-only`, which is explicitly unencrypted and unauthenticated.
    ///
    /// A non-Shared field, like `enable_txn_api` beside it: the Scala `ApiServer` has no OCapN
    /// listener, and this port's is opt-in rather than always-on for the reason above.
    pub ocapn_listen: Option<String>,
    /// Bind the **`unix`** OCapN listener at this socket path, or `None` for none (issue #249). A
    /// transport rather than a second address for the one above, and the one the node offers that is
    /// **not** a testing transport: what admits a peer is the socket's file mode (`0600`, set at bind),
    /// so `tcp-testing-only`'s "no authentication" does not apply here.
    pub ocapn_listen_unix: Option<String>,
    /// Refuse to dial loopback and private addresses on a peer's word (HAZOP row B4; off by default
    /// because the conformance suite and the ERTP transcript both dial loopback).
    pub ocapn_deny_local_dial: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Storage {
    pub data_dir: PathBuf,
}

/// The Kamon reporter switches (`metrics.*`, AUDIT §6's metrics row), kept so a Scala-shaped config
/// still parses — the reporters it cannot honour are refused at startup and `prometheus` is accepted
/// with a note, which is what that row records.
///
/// Kamon is not part of this port: `GET /metrics` always serves Prometheus text, and there is no
/// InfluxDB or UDP sender, no Zipkin span reporter and no Sigar collector. `Configuration::build`
/// consequently refuses the four reporters when they are set and notes `prometheus` (whose endpoint
/// exists, ungated) — see `check_metrics_config`. Nothing else reads these fields.
#[derive(Clone, Debug, PartialEq)]
pub struct Metrics {
    pub prometheus: bool,
    pub influxdb: bool,
    pub influxdb_udp: bool,
    pub zipkin: bool,
    pub sigar: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DevConf {
    pub deployer_private_key: Option<String>,
}

/// CLI subcommand outcome (port of the Scala `Command` ADT).
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Eval {
        files: Vec<String>,
        print_unmatched_sends_only: bool,
    },
    Repl,
    Deploy {
        phlo_limit: i64,
        phlo_price: i64,
        valid_after_block: i64,
        private_key: Option<PrivateKey>,
        private_key_path: Option<PathBuf>,
        location: String,
        shard_id: String,
    },
    DeployStatus {
        id: Vec<u8>,
    },
    FindDeploy {
        id: Vec<u8>,
    },
    Propose {
        print_unmatched_sends: bool,
    },
    ShowBlock {
        hash: String,
    },
    ShowBlocks {
        depth: i32,
    },
    VisualizeDag {
        depth: i32,
        show_justification_lines: bool,
    },
    MachineVerifiableDag,
    Run,
    Keygen {
        path: PathBuf,
    },
    LastFinalizedBlock,
    IsFinalized {
        hash: String,
    },
    BondStatus {
        public_key: PublicKey,
    },
    Help,
    DataAtName {
        name: Name,
    },
    ContAtName {
        names: Vec<Name>,
    },
    Status,
}
