//! Reading secret material from disk without losing its confidentiality on the way in.
//!
//! Every writer in this workspace already narrows a private key to `0600` — [`super::key_util`]'s
//! `create_owner_only` and the OCapN identity's `write_new_private` both do — but *reading* checked
//! nothing. A key that arrived by any other route (a copy, a restore from a backup, a file another
//! tool made, an operator's `custom_key_location`) was therefore adopted silently, and the whole
//! containment the write side buys was undone on the read side. A validator key a second local
//! account can read is a validator key a second local account can sign with.
//!
//! The check is the one `node/src/api/ocapn.rs` already ships for the Noise identity, applied to
//! every secret loader in the workspace so the rule has one home: refuse rather than repair, because
//! an operator who sees this has a key they should **re-issue**, and quietly tightening the mode
//! would hide that it had been exposed.
//!
//! **Not for the certificate.** `certificate_helper::from_file` reads a public X.509 certificate,
//! which is meant to be shared; gating it would refuse every legitimate deployment.

use std::path::Path;

/// Read a file that holds secret material, refusing any mode another uid can reach.
pub fn read_owner_only(path: &Path) -> Result<Vec<u8>, String> {
    read_after_permission_check(path)
}

/// The string form, for the PEM loaders.
pub fn read_owner_only_to_string(path: &Path) -> Result<String, String> {
    let bytes = read_after_permission_check(path)?;
    String::from_utf8(bytes).map_err(|e| format!("{}: not valid UTF-8: {e}", path.display()))
}

/// Open, check and read — **in that order, on the open descriptor**.
///
/// The mode is read with `fstat` on the very file that is then read, not by a second lookup of the
/// path, so there is no window in which the path is re-pointed at a different file between the check
/// and the read. Small, but it is the difference between a check and a gesture.
#[cfg(unix)]
fn read_after_permission_check(path: &Path) -> Result<Vec<u8>, String> {
    use std::io::Read;
    use std::os::unix::fs::PermissionsExt;

    let read_err = |e: std::io::Error| format!("could not read {}: {e}", path.display());
    let mut file = std::fs::File::open(path).map_err(read_err)?;
    let mode = file.metadata().map_err(read_err)?.permissions().mode();
    // `0o077` is every bit that lets a group or another uid have anything to do with the file. A key
    // needs none of them: it is read by the account that runs the node and by nothing else.
    if mode & 0o077 != 0 {
        return Err(format!(
            "{} is mode {:03o}: a file holding secret material must not be reachable by another uid \
             — re-issue the key under a 0600 mode rather than widening who can read it",
            path.display(),
            mode & 0o777
        ));
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(read_err)?;
    Ok(bytes)
}

/// Non-Unix (and wasm) has no mode to check; read as before.
#[cfg(not(unix))]
fn read_after_permission_check(path: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("could not read {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;

    /// A directory of this test's own, so parallel runs never share a file.
    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("rchain-secret-file-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        dir
    }

    #[cfg(unix)]
    fn write_with_mode(path: &Path, bytes: &[u8], mode: u32) {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(mode)
            .open(path)
            .expect("open");
        // `OpenOptions::mode` is masked by the process umask (this host's is `0002`, so a `0o602`
        // fixture would land `0o600` and the test would pass without the guard). Set it explicitly so
        // the fixture is the mode it says it is.
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("chmod");
        file.write_all(bytes).expect("write");
    }

    /// **A group-readable secret file is refused**, naming the path and the mode it was found at —
    /// the operator needs the mode to know what to fix, and the path to know what to fix.
    #[cfg(unix)]
    #[test]
    fn a_group_readable_file_is_refused() {
        let dir = scratch("loose");
        let path = dir.join("validator.key");
        write_with_mode(&path, b"a secret", 0o644);

        let err = read_owner_only(&path).expect_err("0644 must be refused");
        assert!(err.contains("validator.key"), "{err}");
        assert!(err.contains("644"), "the mode must be named: {err}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The refusal covers **every** group/other bit, not only the read ones: a world-*writable* key is
    /// the one an attacker does not have to read first.
    #[cfg(unix)]
    #[test]
    fn a_world_writable_file_is_refused() {
        let dir = scratch("writable");
        let path = dir.join("validator.key");
        write_with_mode(&path, b"a secret", 0o602);
        assert!(
            read_owner_only(&path).is_err(),
            "group-write only is still another uid reaching the key"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The mode the writers produce is the mode the reader accepts — `0600` round-trips.
    #[cfg(unix)]
    #[test]
    fn an_owner_only_file_is_read() {
        let dir = scratch("owner");
        let path = dir.join("validator.key");
        write_with_mode(&path, b"a secret", 0o600);
        assert_eq!(read_owner_only(&path).expect("0600 is fine"), b"a secret");
        assert_eq!(
            read_owner_only_to_string(&path).expect("0600 is fine"),
            "a secret"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file that is not there is an io error naming the path, not a permission refusal — the two
    /// failures send an operator to different places.
    #[test]
    fn a_missing_file_names_the_path() {
        let dir = scratch("missing");
        let path = dir.join("absent.key");
        let err = read_owner_only(&path).expect_err("no such file");
        assert!(err.contains("absent.key"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
