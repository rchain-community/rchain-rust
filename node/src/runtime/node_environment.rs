//! Node environment initialization (port of `node/runtime/NodeEnvironment.scala`).

use std::path::Path;

use rchain_comm::peer_node::NodeIdentifier;
use rchain_comm::transport::generate_certificate_if_absent;
use rchain_comm::transport::tls_conf::TlsConf;

use crate::configuration::model::NodeConf;

/// A node-environment initialization error (port of `NodeEnvironment.InitializationException`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InitializationException(pub String);

impl std::fmt::Display for InitializationException {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for InitializationException {}

fn data_dir_error(data_dir: &Path) -> String {
    format!(
        "The data dir must be a directory and have read and write permissions:\n{}",
        data_dir.display()
    )
}

/// Create the data dir if absent (port of `canCreateDataDir`).
pub fn can_create_data_dir(data_dir: &Path) -> Result<(), InitializationException> {
    if !data_dir.exists() {
        std::fs::create_dir(data_dir)
            .map_err(|_| InitializationException(data_dir_error(data_dir)))?;
    }
    Ok(())
}

/// Check the data dir is an accessible directory (port of `haveAccessToDataDir`).
pub fn have_access_to_data_dir(data_dir: &Path) -> Result<(), InitializationException> {
    if !data_dir.is_dir() {
        return Err(InitializationException(data_dir_error(data_dir)));
    }
    Ok(())
}

/// Check the TLS certificate file exists (port of `hasCertificate`).
pub fn has_certificate(tls: &TlsConf) -> Result<(), InitializationException> {
    if !tls.certificate_path.exists() {
        return Err(InitializationException(format!(
            "Certificate file {} not found",
            tls.certificate_path.display()
        )));
    }
    Ok(())
}

/// Check the TLS secret-key file exists (port of `hasKey`).
pub fn has_key(tls: &TlsConf) -> Result<(), InitializationException> {
    if !tls.key_path.exists() {
        return Err(InitializationException(format!(
            "Secret key file {} not found",
            tls.key_path.display()
        )));
    }
    Ok(())
}

/// Compute the node identifier from its key (port of `name`).
fn name(tls: &TlsConf) -> Result<NodeIdentifier, InitializationException> {
    generate_certificate_if_absent::node_address(tls)
        .map(NodeIdentifier::new)
        .map_err(|e| InitializationException(format!("Failed to read the X.509 certificate: {e}")))
}

/// Initialize the node environment and derive its identifier (port of `NodeEnvironment.create`).
pub fn create(conf: &NodeConf) -> Result<NodeIdentifier, InitializationException> {
    let data_dir = &conf.storage.data_dir;
    can_create_data_dir(data_dir)?;
    have_access_to_data_dir(data_dir)?;
    generate_certificate_if_absent::run(&conf.tls).map_err(InitializationException)?;
    has_certificate(&conf.tls)?;
    has_key(&conf.tls)?;
    name(&conf.tls)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A directory of this test's own, under the platform temp dir. No `tempfile` dependency: the
    /// integration tests roll the same thing by hand, and the unique suffix is what keeps parallel
    /// runs of this test from sharing a directory.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "rchain-node-env-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        dir
    }

    fn tls_in(dir: &Path) -> TlsConf {
        TlsConf {
            certificate_path: dir.join("node.certificate.pem"),
            key_path: dir.join("node.key.pem"),
            secure_random_non_blocking: true,
            custom_certificate_location: false,
            custom_key_location: false,
        }
    }

    /// A missing data dir is **created**; an existing one is accepted as it is; and a path that
    /// cannot be created (its parent does not exist) is the `InitializationException` whose message
    /// tells an operator which directory to look at.
    #[test]
    fn the_data_dir_is_created_when_absent_and_reported_when_impossible() {
        let dir = scratch("datadir");
        assert!(!dir.exists());
        can_create_data_dir(&dir).expect("a fresh directory is created");
        assert!(dir.is_dir(), "the directory now exists");

        // Creating it again is fine: the check is `!exists`.
        can_create_data_dir(&dir).expect("an existing directory is accepted");
        have_access_to_data_dir(&dir).expect("and it is accessible");

        // A path whose parent is missing cannot be created, and the error names it.
        let orphan = dir.join("no-such-parent").join("data");
        let err = can_create_data_dir(&orphan).expect_err("the parent is missing");
        let message = err.to_string();
        assert!(
            message.starts_with("The data dir must be a directory"),
            "{message}"
        );
        assert!(message.contains(&orphan.display().to_string()), "{message}");
        assert_eq!(err, InitializationException(data_dir_error(&orphan)));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Access means "is a **directory**": a regular file at the path is refused, and so is a path
    /// that does not exist — the second check is what catches a path that exists as a file.
    #[test]
    fn access_requires_a_directory_not_a_file() {
        let dir = scratch("access");
        std::fs::create_dir_all(&dir).expect("scratch");
        let file = dir.join("a-file");
        std::fs::write(&file, b"data").expect("write");

        assert!(have_access_to_data_dir(&file).is_err(), "a file is not a dir");
        assert!(have_access_to_data_dir(&dir.join("absent")).is_err());
        have_access_to_data_dir(&dir).expect("the directory itself");

        let err = have_access_to_data_dir(&file).expect_err("a file");
        assert!(err.to_string().contains(&file.display().to_string()));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The certificate and key checks name the **path** they could not find, which is the whole
    /// value of the check: "Certificate file … not found" with the path is actionable, without it
    /// it is not.
    #[test]
    fn the_certificate_and_key_checks_name_the_missing_path() {
        let dir = scratch("tls");
        std::fs::create_dir_all(&dir).expect("scratch");
        let tls = tls_in(&dir);

        let err = has_certificate(&tls).expect_err("no certificate yet");
        assert_eq!(
            err.to_string(),
            format!("Certificate file {} not found", tls.certificate_path.display())
        );
        let err = has_key(&tls).expect_err("no key yet");
        assert_eq!(
            err.to_string(),
            format!("Secret key file {} not found", tls.key_path.display())
        );

        std::fs::write(&tls.certificate_path, b"cert").expect("write");
        has_certificate(&tls).expect("the certificate exists");
        assert!(has_key(&tls).is_err(), "the key is still missing");
        std::fs::write(&tls.key_path, b"key").expect("write");
        has_key(&tls).expect("the key exists");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The sequence the environment actually runs: generate the pair into an empty data dir, find
    /// both files, and derive the node's identifier from the certificate's public address — a
    /// 32-byte key that is **stable** across a second read (the node's identity must not change
    /// between restarts).
    ///
    /// `create` itself — the five steps in order — needs a whole `NodeConf`, which is the node's
    /// entire configuration; it is exercised end to end by every integration test, since
    /// `node/tests/common/mod.rs` boots through it (see `node/tests/node_api.rs`).
    #[test]
    fn generating_the_pair_yields_a_stable_node_identifier() {
        let dir = scratch("identity");
        std::fs::create_dir_all(&dir).expect("scratch");
        let tls = tls_in(&dir);

        generate_certificate_if_absent::run(&tls).expect("generate");
        has_certificate(&tls).expect("the certificate was written");
        has_key(&tls).expect("the key was written");

        let first = generate_certificate_if_absent::node_address(&tls).expect("address");
        assert_eq!(first.len(), 20, "a keccak-256 address, truncated to 20 bytes");
        let second = generate_certificate_if_absent::node_address(&tls).expect("address again");
        assert_eq!(first, second, "the identity is stable across reads");

        // A second `run` over an existing pair is a no-op: the node does not re-key itself.
        generate_certificate_if_absent::run(&tls).expect("regenerate is a no-op");
        assert_eq!(
            generate_certificate_if_absent::node_address(&tls).expect("address"),
            first
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A TLS path the process cannot write is an error out of `run` rather than a panic — it is
    /// mapped to `InitializationException` in `create`, and a node must be able to report "the
    /// certificate could not be written" and exit.
    #[test]
    fn an_unwritable_certificate_path_is_an_error() {
        let dir = scratch("unwritable");
        std::fs::create_dir_all(&dir).expect("scratch");
        let tls = TlsConf {
            certificate_path: dir.join("no-such-dir").join("node.certificate.pem"),
            key_path: dir.join("node.key.pem"),
            secure_random_non_blocking: true,
            custom_certificate_location: false,
            custom_key_location: false,
        };
        assert!(
            generate_certificate_if_absent::run(&tls).is_err(),
            "a certificate path in a missing directory cannot be written"
        );
        assert!(has_certificate(&tls).is_err(), "and nothing was created");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
