//! UPnP port forwarding (port of `comm/UPnP.scala`).
//!
//! The port-forwarding orchestration, port-mapping formatting, and IPv4 private-address
//! classification are ported here; the weupnp SSDP/SOAP gateway discovery lives in [`gateway`].
//!
//! The cats-effect `Log[F]` is simplified to a single `FnMut(String)` callback (log level is folded
//! into the message), matching the `WhoAmI` port.

pub mod gateway;

use std::net::Ipv4Addr;
use std::sync::Arc;

use crate::errors::{CommErr, CommError};

/// Classify an IP address as private (port of `UPnP.isPrivateIpAddress`).
///
/// Returns `None` when the input is not a valid IPv4 address; otherwise `Some(true)` for
/// private/loopback/link-local/unspecified addresses and `Some(false)` for public ones.
pub fn is_private_ip_address(ip: &str) -> Option<bool> {
    let addr: Ipv4Addr = ip.parse().ok()?;
    let private = match addr.octets() {
        [10, _, _, _] => true,
        [127, _, _, _] => true,
        [192, 168, _, _] => true,
        [172, b, _, _] if (16..=31).contains(&b) => true,
        [169, 254, _, _] => true,
        [0, 0, 0, 0] => true,
        _ => false,
    };
    Some(private)
}

/// A port-mapping entry (port of weupnp `PortMappingEntry`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PortMappingEntry {
    pub protocol: String,
    pub external_port: i32,
    pub internal_port: i32,
    pub internal_client: String,
    pub description: String,
}

/// A gateway device (port of the weupnp `GatewayDevice` API surface used by `UPnP.scala`).
pub trait GatewayDevice: Send + Sync {
    fn friendly_name(&self) -> String;
    fn model_name(&self) -> String;
    fn manufacturer(&self) -> String;
    fn model_description(&self) -> String;
    fn device_type(&self) -> String;
    fn search_type(&self) -> String;
    fn service_type(&self) -> String;
    fn location(&self) -> String;
    fn external_ip_address(&self) -> String;
    fn is_connected(&self) -> bool;
    fn local_address(&self) -> String;
    fn add_port_mapping(
        &self,
        external_port: i32,
        internal_port: i32,
        internal_client: &str,
        protocol: &str,
        description: &str,
    ) -> Result<bool, String>;
    fn delete_port_mapping(&self, external_port: i32, protocol: &str) -> Result<(), String>;
    fn get_generic_port_mapping_entry(&self, index: i32, entry: &mut PortMappingEntry) -> bool;
}

/// Discovered UPnP devices (port of `UPnPDevices`).
#[derive(Default)]
pub struct UPnPDevices {
    /// All discovered devices, keyed by their interface IP.
    pub all: Vec<(String, Arc<dyn GatewayDevice>)>,
    /// The gateway devices (a subset of `all`).
    pub gateways: Vec<Arc<dyn GatewayDevice>>,
    /// The preferred/valid gateway.
    pub valid_gateway: Option<Arc<dyn GatewayDevice>>,
}

/// Discover gateway devices (port of `UPnP.discover`).
pub fn discover() -> UPnPDevices {
    gateway::discover()
}

/// Open `ports` via UPnP (port of `UPnP.assurePortForwarding`), returning the gateway external IP.
pub fn assure_port_forwarding(
    ports: &[i32],
    devices: UPnPDevices,
    log: &mut dyn FnMut(String),
) -> Option<String> {
    log("trying to open ports using UPnP....".to_string());
    if devices.gateways.is_empty() {
        log_gateway_empty(&devices, log);
        None
    } else {
        try_open_ports(ports, &devices, log)
    }
}

fn log_gateway_empty(devices: &UPnPDevices, log: &mut dyn FnMut(String)) {
    log("INFO - No gateway devices found".to_string());
    if devices.all.is_empty() {
        log("No need to open any port".to_string());
    } else {
        print_devices(devices, log);
    }
}

fn print_devices(devices: &UPnPDevices, log: &mut dyn FnMut(String)) {
    let s = devices
        .all
        .iter()
        .map(|(ip, d)| show_device(ip, d.as_ref()))
        .collect::<Vec<_>>()
        .join("\n");
    log(format!("\n{s}\n"));
}

fn try_open_ports(
    ports: &[i32],
    devices: &UPnPDevices,
    log: &mut dyn FnMut(String),
) -> Option<String> {
    let names = devices
        .gateways
        .iter()
        .map(|d| d.friendly_name())
        .collect::<Vec<_>>()
        .join(", ");
    log(format!("Available gateway devices: {names}"));

    let gateway = devices
        .valid_gateway
        .clone()
        .or_else(|| devices.gateways.first().cloned())?;
    log(format!("Picking {} as gateway", gateway.friendly_name()));

    match is_private_ip_address(&gateway.external_ip_address()) {
        Some(true) => log(format!(
            "Gateway's external IP address {} is from a private address block. This machine is behind more than one NAT.",
            gateway.external_ip_address()
        )),
        Some(_) => log("Gateway's external IP address is from a public address block.".to_string()),
        None => log("Can't parse gateway's external IP address. It's maybe IPv6.".to_string()),
    }

    let mappings: Vec<PortMappingEntry> = get_port_mappings(gateway.as_ref())
        .into_iter()
        .filter(|m| ports.contains(&m.external_port))
        .collect();
    remove_ports(&mappings, gateway.as_ref(), log);
    let res = add_ports(ports, gateway.as_ref(), log);

    if res.iter().any(|r| !r) {
        log(
            "Could not open the ports via UPnP. Please open it manually on your router!"
                .to_string(),
        );
    } else {
        log("UPnP port forwarding was most likely successful!".to_string());
    }

    log(show_port_mapping_header());
    for v in get_port_mappings(gateway.as_ref())
        .iter()
        .map(show_port_mapping)
    {
        log(v);
    }
    Some(gateway.external_ip_address())
}

fn remove_ports(
    mappings: &[PortMappingEntry],
    gateway: &dyn GatewayDevice,
    log: &mut dyn FnMut(String),
) {
    for m in mappings {
        let res = remove_port(gateway, m);
        let msg = if res.is_ok() { "[success]" } else { "[failed]" };
        log(format!(
            "Removing an existing port mapping for port {}/{} {msg}",
            m.protocol, m.external_port
        ));
    }
}

fn add_ports(ports: &[i32], gateway: &dyn GatewayDevice, log: &mut dyn FnMut(String)) -> Vec<bool> {
    ports
        .iter()
        .map(|p| {
            let res = add_port(gateway, *p, "TCP", "RChain");
            let msg = if res.is_ok() { "[success]" } else { "[failed]" };
            log(format!("Adding a port mapping for port TCP/{p} {msg}"));
            res.is_ok()
        })
        .collect()
}

/// Read all port-mapping entries in index order (port of `UPnP.getPortMappings`).
fn get_port_mappings(device: &dyn GatewayDevice) -> Vec<PortMappingEntry> {
    let mut mappings = Vec::new();
    let mut i = 0;
    loop {
        let mut entry = PortMappingEntry::default();
        if device.get_generic_port_mapping_entry(i, &mut entry) {
            mappings.push(entry);
            i += 1;
        } else {
            return mappings;
        }
    }
}

fn add_port(
    device: &dyn GatewayDevice,
    port: i32,
    protocol: &str,
    description: &str,
) -> CommErr<bool> {
    let client = device.local_address();
    device
        .add_port_mapping(port, port, &client, protocol, description)
        .map_err(CommError::UnknownCommError)
}

fn remove_port(device: &dyn GatewayDevice, mapping: &PortMappingEntry) -> CommErr<()> {
    device
        .delete_port_mapping(mapping.external_port, &mapping.protocol)
        .map_err(CommError::UnknownCommError)
}

fn show_device(ip: &str, device: &dyn GatewayDevice) -> String {
    let connected = if device.is_connected() { "yes" } else { "no" };
    format!(
        "\nInterface:    {ip}\nName:         {}\nModel:        {}\nManufacturer: {}\nDescription:  {}\nType:         {}\nSearch type:  {}\nService type: {}\nLocation:     {}\nExternal IP:  {}\nConnected:    {connected}\n",
        device.friendly_name(),
        device.model_name(),
        device.manufacturer(),
        device.model_description(),
        device.device_type(),
        device.search_type(),
        device.service_type(),
        device.location(),
        device.external_ip_address(),
    )
}

fn show_port_mapping_header() -> String {
    format!(
        "{:<10} {:<8} {:<15} {:<8} {}",
        "Protocol", "Extern", "Host", "Intern", "Description"
    )
}

fn show_port_mapping(m: &PortMappingEntry) -> String {
    format!(
        "{:<10} {:<8} {:<15} {:<8} {}",
        m.protocol, m.external_port, m.internal_client, m.internal_port, m.description
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_private_ranges() {
        assert_eq!(is_private_ip_address("10.0.0.1"), Some(true));
        assert_eq!(is_private_ip_address("127.0.0.1"), Some(true));
        assert_eq!(is_private_ip_address("192.168.1.1"), Some(true));
        assert_eq!(is_private_ip_address("172.16.0.1"), Some(true));
        assert_eq!(is_private_ip_address("172.31.255.255"), Some(true));
        assert_eq!(is_private_ip_address("169.254.0.1"), Some(true));
        assert_eq!(is_private_ip_address("0.0.0.0"), Some(true));
    }

    #[test]
    fn classifies_public_ranges() {
        assert_eq!(is_private_ip_address("8.8.8.8"), Some(false));
        assert_eq!(is_private_ip_address("172.15.0.1"), Some(false));
        assert_eq!(is_private_ip_address("172.32.0.1"), Some(false));
        assert_eq!(is_private_ip_address("192.169.1.1"), Some(false));
    }

    #[test]
    fn returns_none_for_non_ipv4() {
        assert_eq!(is_private_ip_address("not-an-ip"), None);
        assert_eq!(is_private_ip_address("256.0.0.1"), None);
        assert_eq!(is_private_ip_address("::1"), None);
    }

    struct MockGateway {
        friendly: String,
        external_ip: String,
        mappings: Vec<PortMappingEntry>,
    }

    impl GatewayDevice for MockGateway {
        fn friendly_name(&self) -> String {
            self.friendly.clone()
        }
        fn model_name(&self) -> String {
            "model".to_string()
        }
        fn manufacturer(&self) -> String {
            "vendor".to_string()
        }
        fn model_description(&self) -> String {
            "desc".to_string()
        }
        fn device_type(&self) -> String {
            "type".to_string()
        }
        fn search_type(&self) -> String {
            "st".to_string()
        }
        fn service_type(&self) -> String {
            "service".to_string()
        }
        fn location(&self) -> String {
            "loc".to_string()
        }
        fn external_ip_address(&self) -> String {
            self.external_ip.clone()
        }
        fn is_connected(&self) -> bool {
            true
        }
        fn local_address(&self) -> String {
            "192.168.1.5".to_string()
        }
        fn add_port_mapping(
            &self,
            _external_port: i32,
            _internal_port: i32,
            _internal_client: &str,
            _protocol: &str,
            _description: &str,
        ) -> Result<bool, String> {
            Ok(true)
        }
        fn delete_port_mapping(&self, _external_port: i32, _protocol: &str) -> Result<(), String> {
            Ok(())
        }
        fn get_generic_port_mapping_entry(&self, index: i32, entry: &mut PortMappingEntry) -> bool {
            if let Some(m) = self.mappings.get(index as usize) {
                *entry = m.clone();
                true
            } else {
                false
            }
        }
    }

    #[test]
    fn show_port_mapping_formats_columns() {
        let m = PortMappingEntry {
            protocol: "TCP".to_string(),
            external_port: 40400,
            internal_port: 40400,
            internal_client: "192.168.1.5".to_string(),
            description: "RChain".to_string(),
        };
        let header = show_port_mapping_header();
        assert!(header.contains("Protocol"));
        assert!(header.contains("Description"));
        let row = show_port_mapping(&m);
        assert!(row.contains("TCP"));
        assert!(row.contains("40400"));
        assert!(row.contains("RChain"));
    }

    #[test]
    fn get_port_mappings_reads_in_index_order() {
        let gateway = MockGateway {
            friendly: "gw".to_string(),
            external_ip: "8.8.8.8".to_string(),
            mappings: vec![
                PortMappingEntry {
                    protocol: "TCP".to_string(),
                    external_port: 1,
                    internal_port: 1,
                    internal_client: "c".to_string(),
                    description: "d1".to_string(),
                },
                PortMappingEntry {
                    protocol: "UDP".to_string(),
                    external_port: 2,
                    internal_port: 2,
                    internal_client: "c".to_string(),
                    description: "d2".to_string(),
                },
            ],
        };
        let mappings = get_port_mappings(&gateway);
        assert_eq!(mappings.len(), 2);
        assert_eq!(mappings[0].external_port, 1);
        assert_eq!(mappings[1].external_port, 2);
    }

    #[test]
    fn assure_port_forwarding_returns_none_without_gateways() {
        let mut logs = Vec::new();
        let res = assure_port_forwarding(&[40400, 40404], UPnPDevices::default(), &mut |m| {
            logs.push(m)
        });
        assert_eq!(res, None);
        assert!(logs.iter().any(|m| m.contains("No gateway devices found")));
    }
}

#[cfg(test)]
mod port_forwarding_tests {
    use super::*;

    /// A gateway double that records the calls the orchestration makes, so "remove then add" and
    /// "one call per port" are assertions rather than inferences. `add_fails` selects which ports the
    /// gateway refuses.
    #[derive(Default)]
    struct FakeGateway {
        name: String,
        external_ip: String,
        /// Mappings the gateway already holds (read back by `get_generic_port_mapping_entry`).
        existing: Vec<PortMappingEntry>,
        /// Ports whose `add_port_mapping` fails.
        failing: Vec<i32>,
        added: std::sync::Mutex<Vec<(i32, String)>>,
        removed: std::sync::Mutex<Vec<(i32, String)>>,
    }

    impl GatewayDevice for FakeGateway {
        fn friendly_name(&self) -> String {
            self.name.clone()
        }
        fn model_name(&self) -> String {
            "model".to_string()
        }
        fn manufacturer(&self) -> String {
            "maker".to_string()
        }
        fn model_description(&self) -> String {
            "description".to_string()
        }
        fn device_type(&self) -> String {
            "type".to_string()
        }
        fn search_type(&self) -> String {
            "search".to_string()
        }
        fn service_type(&self) -> String {
            "service".to_string()
        }
        fn location(&self) -> String {
            "http://192.168.1.1/desc.xml".to_string()
        }
        fn external_ip_address(&self) -> String {
            self.external_ip.clone()
        }
        fn is_connected(&self) -> bool {
            true
        }
        fn local_address(&self) -> String {
            "192.168.1.2".to_string()
        }
        fn add_port_mapping(
            &self,
            external_port: i32,
            _internal_port: i32,
            _internal_client: &str,
            protocol: &str,
            _description: &str,
        ) -> Result<bool, String> {
            if self.failing.contains(&external_port) {
                return Err("gateway refused".to_string());
            }
            self.added
                .lock()
                .unwrap()
                .push((external_port, protocol.to_string()));
            Ok(true)
        }
        fn delete_port_mapping(&self, external_port: i32, protocol: &str) -> Result<(), String> {
            self.removed
                .lock()
                .unwrap()
                .push((external_port, protocol.to_string()));
            Ok(())
        }
        fn get_generic_port_mapping_entry(&self, index: i32, entry: &mut PortMappingEntry) -> bool {
            match usize::try_from(index)
                .ok()
                .and_then(|i| self.existing.get(i))
            {
                Some(found) => {
                    *entry = found.clone();
                    true
                }
                None => false,
            }
        }
    }

    fn mapping(port: i32, protocol: &str) -> PortMappingEntry {
        PortMappingEntry {
            protocol: protocol.to_string(),
            external_port: port,
            internal_port: port,
            internal_client: "192.168.1.2".to_string(),
            description: "RChain".to_string(),
        }
    }

    fn devices(gateway: Arc<FakeGateway>, valid: bool) -> UPnPDevices {
        let any: Arc<dyn GatewayDevice> = gateway.clone();
        UPnPDevices {
            all: vec![("192.168.1.1".to_string(), any.clone())],
            gateways: vec![any.clone()],
            valid_gateway: if valid { Some(any) } else { None },
        }
    }

    /// **The port-opening order is remove-then-add.** An existing mapping for a requested port must
    /// be deleted first (a router that already holds the mapping would otherwise answer `Conflict`
    /// and the port stays closed), and every requested port is attempted exactly once — the
    /// orchestration the `UPnP` documentation's "please open it manually" message depends on.
    #[test]
    fn opening_ports_removes_existing_mappings_then_adds_every_port() {
        let gateway = Arc::new(FakeGateway {
            name: "gw".to_string(),
            external_ip: "203.0.113.7".to_string(),
            existing: vec![mapping(40400, "TCP")],
            ..Default::default()
        });
        let mut logs = Vec::new();
        let exposed = try_open_ports(&[40400, 40404], &devices(gateway.clone(), true), &mut |m| {
            logs.push(m)
        })
        .expect("a gateway was found");

        assert_eq!(
            exposed, "203.0.113.7",
            "the gateway's external address is returned"
        );
        assert_eq!(
            *gateway.removed.lock().unwrap(),
            vec![(40400, "TCP".to_string())],
            "the pre-existing mapping for a requested port is removed first"
        );
        assert_eq!(
            *gateway.added.lock().unwrap(),
            vec![(40400, "TCP".to_string()), (40404, "TCP".to_string())],
            "each requested port is opened once, in order"
        );
        assert!(
            logs.iter().any(|l| l.contains("most likely successful")),
            "{logs:?}"
        );
        assert!(
            logs.iter().any(|l| l.contains("public address block")),
            "a public external address is reported as such: {logs:?}"
        );
    }

    /// A gateway whose external address is **private** means the machine is behind a second NAT,
    /// which is what the message warns about; an unparsable (IPv6) address is reported separately.
    #[test]
    fn the_external_address_classification_is_reported() {
        for (external, expected) in [
            ("10.0.0.1", "behind more than one NAT"),
            ("192.168.1.1", "behind more than one NAT"),
            ("203.0.113.7", "public address block"),
            ("2001:db8::1", "maybe IPv6"),
        ] {
            let gateway = Arc::new(FakeGateway {
                name: "gw".to_string(),
                external_ip: external.to_string(),
                ..Default::default()
            });
            let mut logs = Vec::new();
            try_open_ports(&[40400], &devices(gateway, true), &mut |m| logs.push(m))
                .expect("a gateway");
            assert!(
                logs.iter().any(|l| l.contains(expected)),
                "for {external} expected a message containing {expected:?}: {logs:?}"
            );
        }
    }

    /// A port the gateway refuses produces the manual-opened warning; the ports that succeeded are
    /// still recorded (the loop does not stop at the first failure).
    #[test]
    fn a_refused_port_reports_that_the_ports_must_be_opened_manually() {
        let gateway = Arc::new(FakeGateway {
            name: "gw".to_string(),
            external_ip: "203.0.113.7".to_string(),
            failing: vec![40404],
            ..Default::default()
        });
        let mut logs = Vec::new();
        try_open_ports(&[40400, 40404], &devices(gateway.clone(), true), &mut |m| {
            logs.push(m)
        })
        .expect("a gateway");

        assert_eq!(
            gateway.added.lock().unwrap().len(),
            1,
            "the port that succeeded is still mapped"
        );
        assert!(
            logs.iter().any(|l| l.contains("manually")),
            "the failure is reported: {logs:?}"
        );
        assert!(
            logs.iter().all(|l| !l.contains("most likely successful")),
            "…and not as a success: {logs:?}"
        );
    }

    /// With no *valid* gateway the first discovered one is used (a gateway that is not "connected"
    /// per SSDP may still forward), and with no gateway at all the function returns `None` rather
    /// than panicking — the arm that makes UPnP best-effort.
    #[test]
    fn a_fallback_gateway_is_used_and_no_gateway_is_answered_with_none() {
        let gateway = Arc::new(FakeGateway {
            name: "gw".to_string(),
            external_ip: "203.0.113.7".to_string(),
            ..Default::default()
        });
        let mut logs = Vec::new();
        assert!(
            try_open_ports(&[40400], &devices(gateway.clone(), false), &mut |m| logs
                .push(m))
            .is_some(),
            "the first gateway is the fallback"
        );
        assert!(logs.iter().any(|l| l.contains("Picking gw")), "{logs:?}");

        let mut logs = Vec::new();
        assert!(try_open_ports(&[40400], &UPnPDevices::default(), &mut |m| logs.push(m)).is_none());
        assert!(
            logs.iter()
                .any(|l| l.contains("Available gateway devices: ")),
            "{logs:?}"
        );
    }

    /// The device table and the port-mapping table are fixed-width reports; the *header* is part of
    /// what an operator reads, so its column order is pinned.
    #[test]
    fn the_reports_have_their_documented_columns() {
        let header = show_port_mapping_header();
        let columns: Vec<&str> = header.split_whitespace().collect();
        assert_eq!(
            columns,
            vec!["Protocol", "Extern", "Host", "Intern", "Description"]
        );

        let row = show_port_mapping(&mapping(40400, "TCP"));
        assert!(row.starts_with("TCP"), "{row}");
        assert!(row.contains("40400"), "{row}");
        assert!(row.contains("192.168.1.2"), "{row}");
        assert!(row.contains("RChain"), "{row}");
    }
}
