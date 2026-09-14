//! HOCON → config conversion (port of the pureconfig readers in `Configuration.scala`).
//!
//! The `hocon` crate parses HOCON text into a `Hocon` tree (including `${...}` substitution), but
//! keeps sizes/durations as strings, so those are parsed here — mirroring the Scala custom
//! `myIntReader` (size-in-bytes `Long`) and the duration/`PeerNode` readers.

use std::path::PathBuf;
use std::time::Duration;

use hocon::Hocon;
use rchain_casper::{CasperConf, GenesisBlockData, ShardMemberships, ShardSpec};
use rchain_comm::peer_node::PeerNode;
use rchain_comm::transport::tls_conf::TlsConf;

use super::model::{
    ApiServer, DevConf, Metrics, NodeConf, PeersDiscovery, ProtocolClient, ProtocolServer, Storage,
};

/// Parse a size-in-bytes string (`256K`, `16M`, `256M`) into a byte count (port of
/// typesafe-config `getBytes`). Accepts bare integers and `K`/`M`/`G`/`T`/`P` units (binary,
/// powers of 1024) with optional `B`/`i` suffixes.
pub fn parse_size(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if let Ok(n) = s.parse::<i64>() {
        return Some(n);
    }
    let mut split = 0;
    for (i, c) in s.char_indices() {
        if c.is_ascii_digit() || c == '.' || c == '-' || c == '+' || c == 'e' || c == 'E' {
            split = i + c.len_utf8();
        } else {
            break;
        }
    }
    let (num_s, unit) = s.split_at(split);
    let n: f64 = num_s.trim().parse().ok()?;
    let unit = unit.trim();
    let mult: f64 = match unit {
        "" | "B" | "b" => 1.0,
        "k" | "K" | "KiB" | "Ki" => 1024.0,
        "kB" => 1000.0,
        "m" | "M" | "MiB" | "Mi" => 1024.0_f64.powi(2),
        "MB" => 1000.0_f64.powi(2),
        "g" | "G" | "GiB" | "Gi" => 1024.0_f64.powi(3),
        "GB" => 1000.0_f64.powi(3),
        "t" | "T" | "TiB" | "Ti" => 1024.0_f64.powi(4),
        "TB" => 1000.0_f64.powi(4),
        "p" | "P" | "PiB" | "Pi" => 1024.0_f64.powi(5),
        "PB" => 1000.0_f64.powi(5),
        _ => return None,
    };
    Some((n * mult) as i64)
}

/// Parse a duration string (`20 seconds`, `5 minutes`, `111111seconds`) into a `Duration` (port of
/// `scala.concurrent.duration.Duration`).
pub fn parse_duration(s: &str) -> Option<Duration> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut split = 0;
    for (i, c) in s.char_indices() {
        if c.is_ascii_digit() || c == '.' || c == '-' || c == '+' || c == 'e' || c == 'E' {
            split = i + c.len_utf8();
        } else {
            break;
        }
    }
    let (num_s, unit) = s.split_at(split);
    let n: f64 = num_s.trim().parse().ok()?;
    let unit = unit.trim().to_lowercase();
    let nanos: f64 = match unit.as_str() {
        "ns" | "nano" | "nanos" | "nanosecond" | "nanoseconds" => 1.0,
        "us" | "µs" | "micro" | "micros" | "microsecond" | "microseconds" => 1_000.0,
        "ms" | "milli" | "millis" | "millisecond" | "milliseconds" => 1_000_000.0,
        "s" | "sec" | "secs" | "second" | "seconds" => 1_000_000_000.0,
        "m" | "min" | "mins" | "minute" | "minutes" => 60_000_000_000.0,
        "h" | "hour" | "hours" => 3_600_000_000_000.0,
        "d" | "day" | "days" => 86_400_000_000_000.0,
        _ => return None,
    };
    Some(Duration::from_nanos((n * nanos) as u64))
}

// --- primitive getters -------------------------------------------------------

fn err<T>(expected: &str, got: &Hocon) -> Result<T, String> {
    Err(format!("expected {expected}, got {got:?}"))
}

fn get<'a>(h: &'a Hocon, key: &str) -> Result<&'a Hocon, String> {
    match h {
        Hocon::Hash(map) => map
            .get(key)
            .ok_or_else(|| format!("missing config key `{key}`")),
        _ => err("object", h),
    }
}

fn get_opt<'a>(h: &'a Hocon, key: &str) -> Option<&'a Hocon> {
    match h {
        Hocon::Hash(map) => map.get(key),
        _ => None,
    }
}

fn to_string(h: &Hocon) -> Result<String, String> {
    match h {
        Hocon::String(s) => Ok(s.clone()),
        _ => err("string", h),
    }
}

fn to_bool(h: &Hocon) -> Result<bool, String> {
    match h {
        Hocon::Boolean(b) => Ok(*b),
        _ => err("boolean", h),
    }
}

fn to_i32(h: &Hocon) -> Result<i32, String> {
    match h {
        Hocon::Integer(n) => i32::try_from(*n).map_err(|_| format!("integer out of range: {n}")),
        _ => err("integer", h),
    }
}

/// `Long` reader — accepts a plain number or a size-in-bytes string (Scala `myIntReader`).
fn to_i64(h: &Hocon) -> Result<i64, String> {
    match h {
        Hocon::Integer(n) => Ok(*n),
        Hocon::Real(x) => Ok(*x as i64),
        Hocon::String(s) => parse_size(s).ok_or_else(|| format!("invalid size value `{s}`")),
        _ => err("integer or size string", h),
    }
}

fn to_f64(h: &Hocon) -> Result<f64, String> {
    match h {
        Hocon::Real(x) => Ok(*x),
        Hocon::Integer(n) => Ok(*n as f64),
        _ => err("number", h),
    }
}

fn to_duration(h: &Hocon) -> Result<Duration, String> {
    match h {
        Hocon::Integer(nanos) => {
            let nanos = u64::try_from(*nanos).map_err(|_| format!("negative duration: {nanos}"))?;
            Ok(Duration::from_nanos(nanos))
        }
        Hocon::String(s) => parse_duration(s).ok_or_else(|| format!("invalid duration `{s}`")),
        _ => err("duration", h),
    }
}

fn to_path(h: &Hocon) -> Result<PathBuf, String> {
    to_string(h).map(PathBuf::from)
}

fn to_peer_node(h: &Hocon) -> Result<PeerNode, String> {
    let s = to_string(h)?;
    PeerNode::from_address(&s).map_err(|e| format!("invalid peer node address `{s}`: {e:?}"))
}

fn to_string_list(h: &Hocon) -> Result<Vec<String>, String> {
    match h {
        Hocon::Array(items) => items.iter().map(to_string).collect(),
        Hocon::String(s) => Ok(s.split(' ').map(|p| p.to_string()).collect()),
        _ => err("array or space-separated string", h),
    }
}

fn to_optional_string(h: &Hocon, key: &str) -> Result<Option<String>, String> {
    get_opt(h, key).map(to_string).transpose()
}

fn to_optional_path(h: &Hocon, key: &str) -> Result<Option<PathBuf>, String> {
    get_opt(h, key).map(to_path).transpose()
}

// --- struct converters -------------------------------------------------------

fn protocol_server_from_hocon(h: &Hocon) -> Result<ProtocolServer, String> {
    Ok(ProtocolServer {
        network_id: to_string(get(h, "network-id")?)?,
        host: to_optional_string(h, "host")?,
        use_random_ports: to_bool(get(h, "use-random-ports")?)?,
        dynamic_ip: to_bool(get(h, "dynamic-ip")?)?,
        no_upnp: to_bool(get(h, "no-upnp")?)?,
        port: to_i32(get(h, "port")?)?,
        grpc_max_recv_message_size: to_i64(get(h, "grpc-max-recv-message-size")?)?,
        grpc_max_recv_stream_message_size: to_i64(get(h, "grpc-max-recv-stream-message-size")?)?,
        max_message_consumers: to_i32(get(h, "max-message-consumers")?)?,
        disable_state_exporter: to_bool(get(h, "disable-state-exporter")?)?,
    })
}

fn protocol_client_from_hocon(h: &Hocon) -> Result<ProtocolClient, String> {
    Ok(ProtocolClient {
        network_id: to_string(get(h, "network-id")?)?,
        bootstrap: to_peer_node(get(h, "bootstrap")?)?,
        disable_lfs: to_bool(get(h, "disable-lfs")?)?,
        batch_max_connections: to_i32(get(h, "batch-max-connections")?)?,
        network_timeout: to_duration(get(h, "network-timeout")?)?,
        grpc_max_recv_message_size: to_i64(get(h, "grpc-max-recv-message-size")?)?,
        grpc_stream_chunk_size: to_i64(get(h, "grpc-stream-chunk-size")?)?,
    })
}

fn peers_discovery_from_hocon(h: &Hocon) -> Result<PeersDiscovery, String> {
    Ok(PeersDiscovery {
        port: to_i32(get(h, "port")?)?,
        lookup_interval: to_duration(get(h, "lookup-interval")?)?,
        cleanup_interval: to_duration(get(h, "cleanup-interval")?)?,
        heartbeat_batch_size: to_i32(get(h, "heartbeat-batch-size")?)?,
        init_wait_loop_interval: to_duration(get(h, "init-wait-loop-interval")?)?,
    })
}

fn api_server_from_hocon(h: &Hocon) -> Result<ApiServer, String> {
    Ok(ApiServer {
        host: to_string(get(h, "host")?)?,
        port_grpc_external: to_i32(get(h, "port-grpc-external")?)?,
        port_grpc_internal: to_i32(get(h, "port-grpc-internal")?)?,
        grpc_max_recv_message_size: to_i64(get(h, "grpc-max-recv-message-size")?)?,
        port_http: to_i32(get(h, "port-http")?)?,
        port_admin_http: to_i32(get(h, "port-admin-http")?)?,
        max_blocks_limit: to_i32(get(h, "max-blocks-limit")?)?,
        enable_reporting: to_bool(get(h, "enable-reporting")?)?,
        enable_txn_api: to_bool(get(h, "enable-txn-api")?)?,
        enable_devnet_cors: to_bool(get(h, "enable-devnet-cors")?)?,
        keep_alive_time: to_duration(get(h, "keep-alive-time")?)?,
        keep_alive_timeout: to_duration(get(h, "keep-alive-timeout")?)?,
        permit_keep_alive_time: to_duration(get(h, "permit-keep-alive-time")?)?,
        max_connection_idle: to_duration(get(h, "max-connection-idle")?)?,
        max_connection_age: to_duration(get(h, "max-connection-age")?)?,
        max_connection_age_grace: to_duration(get(h, "max-connection-age-grace")?)?,
    })
}

fn storage_from_hocon(h: &Hocon) -> Result<Storage, String> {
    Ok(Storage {
        data_dir: to_path(get(h, "data-dir")?)?,
    })
}

fn metrics_from_hocon(h: &Hocon) -> Result<Metrics, String> {
    Ok(Metrics {
        prometheus: to_bool(get(h, "prometheus")?)?,
        influxdb: to_bool(get(h, "influxdb")?)?,
        influxdb_udp: to_bool(get(h, "influxdb-udp")?)?,
        zipkin: to_bool(get(h, "zipkin")?)?,
        sigar: to_bool(get(h, "sigar")?)?,
    })
}

fn dev_conf_from_hocon(h: &Hocon) -> Result<DevConf, String> {
    Ok(DevConf {
        deployer_private_key: to_optional_string(h, "deployer-private-key")?,
    })
}

fn tls_conf_from_hocon(h: &Hocon) -> Result<TlsConf, String> {
    Ok(TlsConf {
        certificate_path: to_path(get(h, "certificate-path")?)?,
        key_path: to_path(get(h, "key-path")?)?,
        secure_random_non_blocking: to_bool(get(h, "secure-random-non-blocking")?)?,
        custom_certificate_location: to_bool(get(h, "custom-certificate-location")?)?,
        custom_key_location: to_bool(get(h, "custom-key-location")?)?,
    })
}

fn genesis_block_data_from_hocon(h: &Hocon) -> Result<GenesisBlockData, String> {
    Ok(GenesisBlockData {
        genesis_data_dir: to_path(get(h, "genesis-data-dir")?)?,
        bonds_file: to_string(get(h, "bonds-file")?)?,
        wallets_file: to_string(get(h, "wallets-file")?)?,
        bond_minimum: to_i64(get(h, "bond-minimum")?)?,
        bond_maximum: to_i64(get(h, "bond-maximum")?)?,
        epoch_length: to_i32(get(h, "epoch-length")?)?,
        quarantine_length: to_i32(get(h, "quarantine-length")?)?,
        genesis_block_number: to_i64(get(h, "genesis-block-number")?)?,
        number_of_active_validators: to_i32(get(h, "number-of-active-validators")?)?,
        pos_multi_sig_public_keys: to_string_list(get(h, "pos-multi-sig-public-keys")?)?,
        pos_multi_sig_quorum: to_i32(get(h, "pos-multi-sig-quorum")?)?,
        pos_vault_pub_key: to_string(get(h, "pos-vault-pub-key")?)?,
        system_contract_pub_key: to_string(get(h, "system-contract-pub-key")?)?,
    })
}

/// Overlay a `genesis-block-data` block's *present* keys onto `default`, field by field — the same
/// field-level fallback HOCON applies across config layers. A shard entry that sets only
/// `bonds-file` must not have to restate the other twelve fields.
fn genesis_block_data_over(
    h: &Hocon,
    default: &GenesisBlockData,
) -> Result<GenesisBlockData, String> {
    let mut out = default.clone();
    if let Some(v) = get_opt(h, "genesis-data-dir") {
        out.genesis_data_dir = to_path(v)?;
    }
    if let Some(v) = get_opt(h, "bonds-file") {
        out.bonds_file = to_string(v)?;
    }
    if let Some(v) = get_opt(h, "wallets-file") {
        out.wallets_file = to_string(v)?;
    }
    if let Some(v) = get_opt(h, "bond-minimum") {
        out.bond_minimum = to_i64(v)?;
    }
    if let Some(v) = get_opt(h, "bond-maximum") {
        out.bond_maximum = to_i64(v)?;
    }
    if let Some(v) = get_opt(h, "epoch-length") {
        out.epoch_length = to_i32(v)?;
    }
    if let Some(v) = get_opt(h, "quarantine-length") {
        out.quarantine_length = to_i32(v)?;
    }
    if let Some(v) = get_opt(h, "genesis-block-number") {
        out.genesis_block_number = to_i64(v)?;
    }
    if let Some(v) = get_opt(h, "number-of-active-validators") {
        out.number_of_active_validators = to_i32(v)?;
    }
    if let Some(v) = get_opt(h, "pos-multi-sig-public-keys") {
        out.pos_multi_sig_public_keys = to_string_list(v)?;
    }
    if let Some(v) = get_opt(h, "pos-multi-sig-quorum") {
        out.pos_multi_sig_quorum = to_i32(v)?;
    }
    if let Some(v) = get_opt(h, "pos-vault-pub-key") {
        out.pos_vault_pub_key = to_string(v)?;
    }
    if let Some(v) = get_opt(h, "system-contract-pub-key") {
        out.system_contract_pub_key = to_string(v)?;
    }
    Ok(out)
}

fn casper_conf_from_hocon(h: &Hocon) -> Result<CasperConf, String> {
    // The node-level `genesis-block-data` / `autogen-shard-size` are the defaults for every
    // membership; a `casper.shards` entry may override them wholesale.
    let default_genesis = genesis_block_data_from_hocon(get(h, "genesis-block-data")?)?;
    let default_autogen_shard_size = to_i32(get(h, "autogen-shard-size")?)?;
    Ok(CasperConf {
        validator_public_key: to_optional_string(h, "validator-public-key")?,
        validator_private_key: to_optional_string(h, "validator-private-key")?,
        validator_private_key_path: to_optional_path(h, "validator-private-key-path")?,
        shards: shard_memberships_from_hocon(h, default_genesis, default_autogen_shard_size)?,
        casper_loop_interval: to_duration(get(h, "casper-loop-interval")?)?,
        requested_blocks_timeout: to_duration(get(h, "requested-blocks-timeout")?)?,
        max_number_of_parents: to_i32(get(h, "max-number-of-parents")?)?,
        fork_choice_stale_threshold: to_duration(get(h, "fork-choice-stale-threshold")?)?,
        fork_choice_check_if_stale_interval: to_duration(get(
            h,
            "fork-choice-check-if-stale-interval",
        )?)?,
        synchrony_constraint_threshold: to_f64(get(h, "synchrony-constraint-threshold")?)?,
        height_constraint_threshold: to_i64(get(h, "height-constraint-threshold")?)?,
        min_phlo_price: to_i64(get(h, "min-phlo-price")?)?,
        effect_mode: to_optional_string(h, "effect-scheduler")?
            .unwrap_or_else(|| "dfs".to_string()),
    })
}

/// The node's shard memberships (Law 26).
///
/// `casper.shards` — an array of `{ shard-name, parent-shard-id, genesis-block-data?,
/// autogen-shard-size? }` objects — is the multi-shard form, and its first entry is the primary
/// shard. When it is absent, the legacy top-level `shard-name`/`parent-shard-id` (with the
/// node-level genesis data) define a one-element list, so a single-shard node's configuration —
/// including every command-line flag, which keeps mapping into those scalar keys — is unchanged.
///
/// An *array* rather than a name-keyed object: order carries meaning (the first entry is the
/// primary), and it does not depend on the `LinkedHashMap` iteration order of `Hocon::Hash`.
fn shard_memberships_from_hocon(
    h: &Hocon,
    default_genesis: GenesisBlockData,
    default_autogen_shard_size: i32,
) -> Result<ShardMemberships, String> {
    match get_opt(h, "shards") {
        None => {
            let spec = ShardSpec::new(
                to_string(get(h, "shard-name")?)?,
                to_string(get(h, "parent-shard-id")?)?,
                default_genesis,
                default_autogen_shard_size,
            )?;
            ShardMemberships::new(vec![spec])
        }
        Some(Hocon::Array(entries)) => {
            let specs = entries
                .iter()
                .enumerate()
                .map(|(i, entry)| {
                    shard_spec_from_hocon(entry, &default_genesis, default_autogen_shard_size)
                        .map_err(|e| format!("casper.shards[{i}]: {e}"))
                })
                .collect::<Result<Vec<ShardSpec>, String>>()?;
            ShardMemberships::new(specs)
        }
        Some(other) => err("an array of shard objects", other),
    }
}

/// One `casper.shards` entry. An entry-level `genesis-block-data` overlays the node-level one field
/// by field; `autogen-shard-size` likewise falls back to the node-level value.
fn shard_spec_from_hocon(
    h: &Hocon,
    default_genesis: &GenesisBlockData,
    default_autogen_shard_size: i32,
) -> Result<ShardSpec, String> {
    let genesis_block_data = match get_opt(h, "genesis-block-data") {
        Some(entry) => genesis_block_data_over(entry, default_genesis)?,
        None => default_genesis.clone(),
    };
    let autogen_shard_size = match get_opt(h, "autogen-shard-size") {
        Some(entry) => to_i32(entry)?,
        None => default_autogen_shard_size,
    };
    ShardSpec::new(
        to_string(get(h, "shard-name")?)?,
        to_string(get(h, "parent-shard-id")?)?,
        genesis_block_data,
        autogen_shard_size,
    )
}

/// Reject an operator configuration that sets both the multi-shard list and the legacy scalar shard
/// keys — silently letting one win would run a different set of shards than the operator wrote.
///
/// Checked per operator-supplied layer, before merging: `defaults.conf` always sets
/// `casper.shard-name`, so its presence in the merged tree is not a conflict. `source` names the
/// layer for the error message.
pub fn check_shard_config_exclusivity(config: &Hocon, source: &str) -> Result<(), String> {
    let Some(casper) = get_opt(config, "casper") else {
        return Ok(());
    };
    let has_shards = get_opt(casper, "shards").is_some();
    let has_scalar =
        get_opt(casper, "shard-name").is_some() || get_opt(casper, "parent-shard-id").is_some();
    if has_shards && has_scalar {
        return Err(format!(
            "{source}: `casper.shards` and `casper.shard-name`/`casper.parent-shard-id` are \
             mutually exclusive; use `casper.shards` (an array) for a multi-shard node and leave \
             the scalar keys unset"
        ));
    }
    Ok(())
}

/// Build a `NodeConf` from a merged `Hocon` tree (port of `mergedConf.load[NodeConf]`).
pub fn node_conf_from_hocon(h: &Hocon) -> Result<NodeConf, String> {
    Ok(NodeConf {
        standalone: to_bool(get(h, "standalone")?)?,
        autopropose: to_bool(get(h, "autopropose")?)?,
        propose_on_deploy: to_bool(get(h, "propose-on-deploy")?)?,
        protocol_server: protocol_server_from_hocon(get(h, "protocol-server")?)?,
        protocol_client: protocol_client_from_hocon(get(h, "protocol-client")?)?,
        peers_discovery: peers_discovery_from_hocon(get(h, "peers-discovery")?)?,
        api_server: api_server_from_hocon(get(h, "api-server")?)?,
        tls: tls_conf_from_hocon(get(h, "tls")?)?,
        storage: storage_from_hocon(get(h, "storage")?)?,
        casper: casper_conf_from_hocon(get(h, "casper")?)?,
        metrics: metrics_from_hocon(get(h, "metrics")?)?,
        dev_mode: to_bool(get(h, "dev-mode")?)?,
        dev: dev_conf_from_hocon(get(h, "dev")?)?,
        default_data_dir: to_string(get(h, "default-data-dir")?)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use linked_hash_map::LinkedHashMap;

    fn h(pairs: Vec<(&str, Hocon)>) -> Hocon {
        Hocon::Hash(
            pairs
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect::<LinkedHashMap<String, Hocon>>(),
        )
    }

    fn s(v: &str) -> Hocon {
        Hocon::String(v.to_string())
    }

    fn i(v: i64) -> Hocon {
        Hocon::Integer(v)
    }

    // --- parse_size ----------------------------------------------------------

    /// The unit table mixes binary and decimal: `K`/`M`/`G` are powers of 1024, but `kB`/`MB`/`GB`
    /// are powers of 1000. That asymmetry is typesafe-config's, and it is the reason
    /// `grpc-max-recv-message-size = 256M` is not equal to `256MB`.
    #[test]
    fn parse_size_mixes_binary_and_decimal_units() {
        assert_eq!(parse_size("1"), Some(1));
        assert_eq!(parse_size("1024"), Some(1024));
        assert_eq!(parse_size("1K"), Some(1024));
        assert_eq!(parse_size("1k"), Some(1024));
        assert_eq!(parse_size("1KiB"), Some(1024));
        assert_eq!(parse_size("1kB"), Some(1000));
        assert_eq!(parse_size("256M"), Some(256 * 1024 * 1024));
        assert_eq!(parse_size("256MB"), Some(256_000_000));
        assert_eq!(parse_size("1G"), Some(1024 * 1024 * 1024));
        assert_eq!(parse_size("1T"), Some(1024i64.pow(4)));
        assert_eq!(parse_size("1P"), Some(1024i64.pow(5)));
        // Whitespace and a trailing bare `B` are tolerated.
        assert_eq!(parse_size("  16 M "), Some(16 * 1024 * 1024));
        assert_eq!(parse_size("512B"), Some(512));
    }

    /// A fractional mantissa is integer-truncated, not rounded — a value the config parser will
    /// happily accept, so the truncation is pinned rather than assumed.
    #[test]
    fn parse_size_truncates_a_fractional_mantissa() {
        assert_eq!(parse_size("1.5K"), Some(1536));
        assert_eq!(parse_size("1.9"), Some(1));
        assert_eq!(parse_size("0.5"), Some(0));
    }

    #[test]
    fn parse_size_rejects_what_it_cannot_parse() {
        for bad in [
            "", "   ", "abc", "12QB", "K", "1 K B", "1,000", "--5", "1x2",
        ] {
            assert_eq!(parse_size(bad), None, "{bad:?} must not parse");
        }
    }

    // --- parse_duration ------------------------------------------------------

    /// Unlike a size, a duration **requires** a unit: the bare `"5"` case is `None`, which is why
    /// `casper-loop-interval = 5` is a config error rather than five of something.
    #[test]
    fn parse_duration_accepts_every_unit_spelling_and_requires_a_unit() {
        assert_eq!(parse_duration("20 seconds"), Some(Duration::from_secs(20)));
        assert_eq!(
            parse_duration("111111seconds"),
            Some(Duration::from_secs(111111))
        );
        assert_eq!(parse_duration("5 minutes"), Some(Duration::from_secs(300)));
        assert_eq!(parse_duration("1 m"), Some(Duration::from_secs(60)));
        assert_eq!(parse_duration("1h"), Some(Duration::from_secs(3600)));
        assert_eq!(parse_duration("1d"), Some(Duration::from_secs(86400)));
        assert_eq!(parse_duration("500ms"), Some(Duration::from_millis(500)));
        assert_eq!(parse_duration("100uS"), Some(Duration::from_micros(100)));
        assert_eq!(parse_duration("7ns"), Some(Duration::from_nanos(7)));
        assert_eq!(parse_duration("1.5s"), Some(Duration::from_millis(1500)));

        assert_eq!(
            parse_duration("5"),
            None,
            "a nonzero bare number has no unit"
        );
        assert_eq!(parse_duration(""), None);
        assert_eq!(parse_duration("1 fortnight"), None);
        assert_eq!(parse_duration("seconds"), None);
        assert_eq!(parse_duration("s"), None);
    }

    // --- the primitive getters ----------------------------------------------

    /// `get` is the total-access path: a missing key must name the key, and a non-object must say
    /// what it expected. Both arms are reachable from a hand-written config file.
    #[test]
    fn get_names_a_missing_key_and_rejects_a_non_object() {
        let root = h(vec![("present", i(1))]);
        assert_eq!(get(&root, "present").unwrap(), &i(1));
        let missing = get(&root, "absent").expect_err("let it error");
        assert!(missing.contains("absent"), "{missing}");
        let wrong = get(&i(1), "any").expect_err("let it error");
        assert!(wrong.contains("expected object"), "{wrong}");
        assert_eq!(get_opt(&i(1), "any"), None, "get_opt is total");
    }

    #[test]
    fn to_i32_rejects_an_integer_that_does_not_fit() {
        assert_eq!(to_i32(&i(7)), Ok(7));
        let big = to_i32(&i(i64::from(i32::MAX) + 1)).expect_err("let it error");
        assert!(big.contains("out of range"), "{big}");
        let wrong = to_i32(&s("7")).expect_err("let it error");
        assert!(wrong.contains("expected integer"), "{wrong}");
    }

    /// `to_i64` is the size-aware `Long` reader: a number, a real, or a size string — three arms
    /// that a single `as i64` would collapse into one.
    #[test]
    fn to_i64_reads_a_number_a_real_and_a_size_string() {
        assert_eq!(to_i64(&i(42)), Ok(42));
        assert_eq!(to_i64(&Hocon::Real(2.9)), Ok(2), "a real truncates");
        assert_eq!(to_i64(&s("256M")), Ok(256 * 1024 * 1024));
        let bad = to_i64(&s("256Q")).expect_err("let it error");
        assert!(bad.contains("invalid size value"), "{bad}");
        let wrong = to_i64(&Hocon::Boolean(true)).expect_err("let it error");
        assert!(wrong.contains("expected integer or size string"), "{wrong}");
    }

    /// A negative duration is rejected rather than wrapping (`u64::try_from`), so a config file
    /// saying `-1` cannot become ~584 years.
    #[test]
    fn to_duration_rejects_a_negative_number_and_an_unparseable_string() {
        assert_eq!(to_duration(&i(1000)), Ok(Duration::from_nanos(1000)));
        let negative = to_duration(&i(-1)).expect_err("let it error");
        assert!(negative.contains("negative duration"), "{negative}");
        assert_eq!(
            to_duration(&s("2 minutes")),
            Ok(Duration::from_secs(120)),
            "the string arm goes through parse_duration"
        );
        let bad = to_duration(&s("2 fortnights")).expect_err("let it error");
        assert!(bad.contains("invalid duration"), "{bad}");
        let wrong = to_duration(&Hocon::Boolean(false)).expect_err("let it error");
        assert!(wrong.contains("expected duration"), "{wrong}");
    }

    /// A string list is either an array or a space-separated string (the form the Scala reader
    /// accepts for `pos-multi-sig-public-keys`), and nothing else.
    #[test]
    fn to_string_list_accepts_an_array_or_a_space_separated_string() {
        let array = Hocon::Array(vec![s("a"), s("b")]);
        assert_eq!(to_string_list(&array), Ok(vec!["a".into(), "b".into()]));
        assert_eq!(
            to_string_list(&s("a b c")),
            Ok(vec!["a".into(), "b".into(), "c".into()])
        );
        assert_eq!(
            to_string_list(&Hocon::Array(vec![i(1)])),
            Err("expected string, got Integer(1)".to_string())
        );
        let wrong = to_string_list(&i(1)).expect_err("let it error");
        assert!(
            wrong.contains("expected array or space-separated string"),
            "{wrong}"
        );
    }

    /// A malformed peer address reports the offending string, not just "parse error" — the message
    /// is the only thing telling an operator which of several bootstrap settings is wrong.
    #[test]
    fn to_peer_node_quotes_the_address_it_could_not_parse() {
        let good = to_peer_node(&s("rnode://de6eed5d00cf080fc587eeb412cb31a75fd10358@127.0.0.1?protocol=40400&discovery=40404"));
        assert!(good.is_ok(), "{good:?}");
        let bad = to_peer_node(&s("not-an-address")).expect_err("let it error");
        assert!(bad.contains("not-an-address"), "{bad}");
        let wrong = to_peer_node(&i(1)).expect_err("let it error");
        assert!(wrong.contains("expected string"), "{wrong}");
    }

    // --- the field-level fallback -------------------------------------------

    fn genesis() -> GenesisBlockData {
        GenesisBlockData {
            genesis_data_dir: PathBuf::from("/genesis"),
            bonds_file: "/genesis/bonds.txt".to_string(),
            wallets_file: "/genesis/wallets.txt".to_string(),
            bond_minimum: 1,
            bond_maximum: 100,
            epoch_length: 10,
            quarantine_length: 10,
            genesis_block_number: 0,
            number_of_active_validators: 10,
            pos_multi_sig_public_keys: vec!["k".to_string()],
            pos_multi_sig_quorum: 0,
            pos_vault_pub_key: "vault".to_string(),
            system_contract_pub_key: "system".to_string(),
        }
    }

    /// A shard entry that sets one field must not have to restate the other twelve: the overlay
    /// copies the default and replaces only the keys the entry actually has. This is what makes a
    /// short `casper.shards` entry legal, and it is the behaviour a "parse a fresh struct" rewrite
    /// would break.
    #[test]
    fn a_shard_entry_overrides_only_the_fields_it_sets() {
        let default = genesis();
        let entry = h(vec![
            ("bonds-file", s("/child/bonds.txt")),
            ("epoch-length", i(20)),
        ]);
        let over = genesis_block_data_over(&entry, &default).expect("overlay");
        assert_eq!(over.bonds_file, "/child/bonds.txt");
        assert_eq!(over.epoch_length, 20);
        // Everything else is the node-level default, unchanged.
        assert_eq!(over.wallets_file, default.wallets_file);
        assert_eq!(over.bond_maximum, default.bond_maximum);
        assert_eq!(over.quarantine_length, default.quarantine_length);
        assert_eq!(
            over.pos_multi_sig_public_keys,
            default.pos_multi_sig_public_keys
        );
        assert_eq!(over.genesis_data_dir, default.genesis_data_dir);

        // An entry that sets nothing is the default exactly.
        assert_eq!(
            genesis_block_data_over(&h(Vec::new()), &default).unwrap(),
            default
        );
    }

    /// A present-but-wrongly-typed override is an error naming the field path, rather than being
    /// quietly ignored in favour of the default.
    #[test]
    fn an_override_of_the_wrong_type_is_an_error() {
        let default = genesis();
        let entry = h(vec![("epoch-length", s("twenty"))]);
        let err = genesis_block_data_over(&entry, &default).expect_err("let it error");
        assert!(err.contains("expected integer"), "{err}");

        let entry = h(vec![("bonds-file", i(3))]);
        let err = genesis_block_data_over(&entry, &default).expect_err("let it error");
        assert!(err.contains("expected string"), "{err}");
    }

    /// `to_optional_string`/`to_optional_path` distinguish "absent" from "present but wrong", the
    /// same way the overlay does: a missing key is `None`, a malformed one is an error.
    #[test]
    fn an_optional_key_is_absent_or_an_error_but_never_silently_null() {
        let root = h(vec![("present", s("v")), ("bad", i(1))]);
        assert_eq!(
            to_optional_string(&root, "present"),
            Ok(Some("v".to_string()))
        );
        assert_eq!(to_optional_string(&root, "absent"), Ok(None));
        let err = to_optional_string(&root, "bad").expect_err("let it error");
        assert!(err.contains("expected string"), "{err}");

        assert_eq!(
            to_optional_path(&root, "present"),
            Ok(Some(PathBuf::from("v")))
        );
        assert_eq!(to_optional_path(&root, "absent"), Ok(None));
        assert!(to_optional_path(&root, "bad").is_err());
    }

    /// A whole scalar section, read end to end: the single-shard form has no `shards` array, and
    /// its `effect-scheduler` is optional with a documented default of `dfs`.
    #[test]
    fn a_scalar_casper_section_reads_as_a_one_shard_membership() {
        let casper = h(vec![
            ("shard-name", s("root")),
            ("parent-shard-id", s("/")),
            ("casper-loop-interval", s("100 milliseconds")),
            ("requested-blocks-timeout", s("5 seconds")),
            ("max-number-of-parents", i(10)),
            ("fork-choice-stale-threshold", s("5 minutes")),
            ("fork-choice-check-if-stale-interval", s("10 seconds")),
            ("synchrony-constraint-threshold", Hocon::Real(0.67)),
            ("height-constraint-threshold", i(100)),
            ("min-phlo-price", i(1)),
            ("autogen-shard-size", i(5)),
            (
                "genesis-block-data",
                h(vec![
                    ("genesis-data-dir", s("/genesis")),
                    ("bonds-file", s("bonds.txt")),
                    ("wallets-file", s("wallets.txt")),
                    ("bond-minimum", i(1)),
                    ("bond-maximum", i(100)),
                    ("epoch-length", i(10)),
                    ("quarantine-length", i(10)),
                    ("genesis-block-number", i(0)),
                    ("number-of-active-validators", i(10)),
                    ("pos-multi-sig-public-keys", s("k1 k2")),
                    ("pos-multi-sig-quorum", i(1)),
                    ("pos-vault-pub-key", s("vault")),
                    ("system-contract-pub-key", s("system")),
                ]),
            ),
        ]);
        let conf = casper_conf_from_hocon(&casper).expect("casper conf");
        assert_eq!(conf.effect_mode, "dfs", "the documented default");
        assert_eq!(conf.synchrony_constraint_threshold, 0.67);
        assert_eq!(conf.casper_loop_interval, Duration::from_millis(100));
        assert_eq!(conf.shards.len(), 1);
        assert_eq!(conf.shards.primary().shard_name, "root");
        assert_eq!(
            conf.shards
                .primary()
                .genesis_block_data
                .pos_multi_sig_public_keys,
            vec!["k1".to_string(), "k2".to_string()]
        );
    }

    /// `casper.shards` is the multi-shard form, and its *first* entry is the primary — the ordering
    /// contract `ShardMemberships` carries. An entry-level `genesis-block-data` overlays the
    /// node-level block field by field.
    #[test]
    fn an_explicit_shards_array_orders_the_primary_first() {
        let genesis_block = h(vec![
            ("genesis-data-dir", s("/genesis")),
            ("bonds-file", s("bonds.txt")),
            ("wallets-file", s("wallets.txt")),
            ("bond-minimum", i(1)),
            ("bond-maximum", i(100)),
            ("epoch-length", i(10)),
            ("quarantine-length", i(10)),
            ("genesis-block-number", i(0)),
            ("number-of-active-validators", i(10)),
            ("pos-multi-sig-public-keys", Hocon::Array(vec![])),
            ("pos-multi-sig-quorum", i(0)),
            ("pos-vault-pub-key", s("vault")),
            ("system-contract-pub-key", s("system")),
        ]);
        let casper = h(vec![
            ("casper-loop-interval", s("100 ms")),
            ("requested-blocks-timeout", s("5 s")),
            ("max-number-of-parents", i(10)),
            ("fork-choice-stale-threshold", s("5 min")),
            ("fork-choice-check-if-stale-interval", s("10 s")),
            ("synchrony-constraint-threshold", Hocon::Real(0.67)),
            ("height-constraint-threshold", i(100)),
            ("min-phlo-price", i(1)),
            ("autogen-shard-size", i(5)),
            ("genesis-block-data", genesis_block.clone()),
            (
                "shards",
                Hocon::Array(vec![
                    h(vec![("shard-name", s("root")), ("parent-shard-id", s("/"))]),
                    h(vec![
                        ("shard-name", s("child")),
                        ("parent-shard-id", s("root")),
                        (
                            "genesis-block-data",
                            h(vec![("bonds-file", s("child.txt"))]),
                        ),
                        ("autogen-shard-size", i(7)),
                    ]),
                ]),
            ),
        ]);
        let conf = casper_conf_from_hocon(&casper).expect("casper conf");
        assert_eq!(conf.shards.len(), 2);
        assert_eq!(
            conf.shards.primary().shard_name,
            "root",
            "entry 0 is the primary"
        );
        let child = conf.shards.iter().nth(1).expect("the child");
        assert_eq!(child.shard_name, "child");
        assert_eq!(child.autogen_shard_size, 7, "the entry overrides it");
        assert_eq!(child.genesis_block_data.bonds_file, "child.txt");
        assert_eq!(
            child.genesis_block_data.wallets_file, "wallets.txt",
            "unset fields fall back to the node-level block"
        );
        assert_eq!(
            conf.shards.primary().autogen_shard_size,
            5,
            "an entry without the key keeps the node-level value"
        );

        // The wrong shape for `shards` is an error, not a silent single-shard fallback.
        let mut bad = casper.clone();
        if let Hocon::Hash(map) = &mut bad {
            map.insert("shards".to_string(), s("root"));
        }
        let err = casper_conf_from_hocon(&bad).expect_err("let it error");
        assert!(err.contains("an array of shard objects"), "{err}");
    }

    /// The mutual-exclusion guard (Law 26) is reachable from a merged config file *and* from the
    /// command line, so the error names the source it came from.
    #[test]
    fn the_shard_forms_are_mutually_exclusive() {
        let casper = h(vec![("shard-name", s("root")), ("parent-shard-id", s("/"))]);
        // A scalar-only section is fine.
        let scalar_only = h(vec![("casper", casper.clone())]);
        assert!(check_shard_config_exclusivity(&scalar_only, "test").is_ok());

        let both = h(vec![(
            "casper",
            h(vec![
                ("shard-name", s("root")),
                ("parent-shard-id", s("/")),
                ("shards", Hocon::Array(vec![])),
            ]),
        )]);
        let err = check_shard_config_exclusivity(&both, "rnode.conf").expect_err("let it error");
        assert!(err.contains("rnode.conf"), "{err}");
        assert!(err.contains("mutually exclusive"), "{err}");
    }
}
