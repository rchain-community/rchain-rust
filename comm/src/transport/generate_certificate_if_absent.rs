//! Self-signed X.509 certificate generation (when absent).
//!
//! Mirrors `comm/src/main/scala/coop/rchain/comm/transport/GenerateCertificateIfAbsent.scala` and
//! `crypto/.../util/CertificateHelper.generate`. The node certificate is a self-signed P-256 cert
//! whose CommonName is the node's base16 address (keccak-20 of the EC public point).

use std::fs;
use std::path::Path;

use rcgen::{CertificateParams, DnType, KeyPair, PKCS_ECDSA_P256_SHA256};

use crate::transport::tls_conf::TlsConf;

/// The elliptic curve used for node identity (port of `CertificateHelper.EllipticCurveName`).
pub const ELLIPTIC_CURVE_NAME: &str = "secp256r1";

/// Generate a self-signed P-256 certificate and PKCS#8 key. Returns `(cert_pem, key_pem)`.
pub fn generate_certificate() -> Result<(String, String), String> {
    let key_pair = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).map_err(|e| e.to_string())?;
    let raw = key_pair.public_key_raw();
    // `raw` is the uncompressed point `04 || x || y`; the node address hashes `x || y`.
    let address = rchain_crypto::util::certificate_helper::public_address(&raw[1..]);
    let cn = rchain_shared::base16::encode(&address);

    let mut params = CertificateParams::new(Vec::new()).map_err(|e| e.to_string())?;
    params.distinguished_name.push(DnType::CommonName, &cn);
    let cert = params.self_signed(&key_pair).map_err(|e| e.to_string())?;
    Ok((cert.pem(), key_pair.serialize_pem()))
}

/// Generate a certificate at `tls.certificate_path` (and key at `tls.key_path`) if one is absent
/// and not custom-provided (port of `GenerateCertificateIfAbsent.run`).
pub fn run(tls: &TlsConf) -> Result<(), String> {
    if tls.custom_certificate_location || Path::new(&tls.certificate_path).exists() {
        return Ok(());
    }
    let (cert_pem, key_pem) = generate_certificate()?;
    fs::write(&tls.certificate_path, cert_pem).map_err(|e| e.to_string())?;
    // The node private key is secret material: write it owner-only (0600), not with default perms.
    rchain_crypto::util::key_util::write_private_key(Path::new(&tls.key_path), key_pem.as_bytes())?;
    Ok(())
}

/// Compute the node identity (the 20-byte keccak-20 address) from the generated key (port of
/// `NodeEnvironment.name`; the Scala reads the certificate, whose public key equals the key's).
pub fn node_address(tls: &TlsConf) -> Result<Vec<u8>, String> {
    // The node identity key is secret material, so the read is checked (W-N6-05): `run` above writes
    // it `0600`, and a key at any other mode was not written by this node and is refused rather than
    // trusted — a node identity another account can read is one it can impersonate.
    let key_pem =
        rchain_crypto::util::secret_file::read_owner_only_to_string(Path::new(&tls.key_path))?;
    let key_pair = KeyPair::from_pem(&key_pem).map_err(|e| e.to_string())?;
    let raw = key_pair.public_key_raw();
    Ok(rchain_crypto::util::certificate_helper::public_address(
        &raw[1..],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_pem_pair() {
        let (cert, key) = generate_certificate().unwrap();
        assert!(cert.starts_with("-----BEGIN CERTIFICATE-----"));
        assert!(cert.ends_with("-----END CERTIFICATE-----\n"));
        assert!(key.starts_with("-----BEGIN PRIVATE KEY-----"));
    }

    #[test]
    fn node_address_is_20_bytes() {
        let (_, key_pem) = generate_certificate().unwrap();
        let key_pair = KeyPair::from_pem(&key_pem).unwrap();
        let raw = key_pair.public_key_raw();
        let addr = rchain_crypto::util::certificate_helper::public_address(&raw[1..]);
        assert_eq!(addr.len(), 20);
    }

    /// **A node identity key another account can read is refused** (W-N6-05). `run` writes this file
    /// `0600`; anything else was not written by this node. The key names the node on every transport,
    /// so a reader for it means anyone on the host can impersonate the node — and `run` returns early
    /// on an existing certificate, so a key that arrived by copy or restore is read exactly here.
    ///
    /// The wide mode is the falsifier: with the check removed the key parses and the address is
    /// returned, so this test goes red rather than silently passing.
    #[cfg(unix)]
    #[test]
    fn node_address_refuses_a_group_readable_key() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!("rchain-node-addr-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let key_path = dir.join("node.key.pem");

        let (_, key_pem) = generate_certificate().unwrap();
        std::fs::write(&key_path, &key_pem).unwrap();
        std::fs::set_permissions(&key_path, fs::Permissions::from_mode(0o644)).unwrap();

        let tls = TlsConf {
            certificate_path: dir.join("node.certificate.pem"),
            key_path,
            secure_random_non_blocking: true,
            custom_certificate_location: false,
            custom_key_location: true,
        };
        let err = node_address(&tls).expect_err("a readable node key must not be adopted");
        assert!(err.contains("node.key.pem"), "{err}");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
