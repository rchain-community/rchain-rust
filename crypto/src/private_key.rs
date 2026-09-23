//! A private key.
//!
//! Mirrors `crypto/src/main/scala/coop/rchain/crypto/PrivateKey.scala`.

/// A private key, as a raw byte array.
///
/// **No `Debug`, and zeroed on drop** (AUDIT §12's "missing key zeroization / `Debug` on `PrivateKey`").
/// `Debug` is implemented by hand below and prints a redaction, because the derived one printed the raw
/// secret: one `{:?}` in a log line, an error message or a test failure was a leaked key. The `Drop`
/// zeroes the buffer.
///
/// Both are *mitigations, not guarantees*, and saying so is the point: `Clone` survives (a clone is a
/// second copy of the secret, and there are 23 call sites that clone a key — audited and left alone,
/// since the alternative is threading ownership through the deploy and signing paths), the allocator
/// may move the buffer, and Rust does not promise that a zeroing write is not optimised into a dead
/// store. What the change removes is the *silent* leak — a key in a log — which is the one that
/// actually happens.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PrivateKey(pub Vec<u8>);

impl std::fmt::Debug for PrivateKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The length is printed because it identifies the curve (32 vs 64 bytes) without identifying
        // the key, and a test that fails on a wrong-length key should say so.
        write!(f, "PrivateKey(<redacted, {} bytes>)", self.0.len())
    }
}

impl Drop for PrivateKey {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.0.zeroize();
    }
}

impl PrivateKey {
    /// Construct from raw bytes.
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// The raw key bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The redaction is **observable**, so it is pinned. Zeroization is not — the buffer is gone by
    /// the time a test could look at it — so there is deliberately no test for it; a test that claimed
    /// to check it would be asserting something it cannot see.
    #[test]
    fn the_debug_output_redacts_the_key() {
        let key = PrivateKey::new(vec![0xAB, 0xCD, 0xEF]);
        assert_eq!(format!("{key:?}"), "PrivateKey(<redacted, 3 bytes>)");
        let text = format!("{key:?}");
        assert!(
            !text.contains("171") && !text.contains("ab") && !text.contains("AB"),
            "the debug text must not carry the key's bytes: {text}"
        );
    }

    /// `new`/`bytes` is an identity pair, and the key compares by **value**: two keys built from the
    /// same bytes are the same key (which is what makes a key usable as a map key), and one byte of
    /// difference makes them different.
    #[test]
    fn a_private_key_is_its_bytes_and_compares_by_value() {
        let bytes = vec![0x01, 0x02, 0xFF];
        let key = PrivateKey::new(bytes.clone());
        assert_eq!(key.bytes(), bytes.as_slice());

        assert_eq!(key, PrivateKey::new(bytes.clone()));
        assert_ne!(key, PrivateKey::new(vec![0x01, 0x02, 0x00]));
        assert_ne!(
            key,
            PrivateKey::new(vec![0x01, 0x02]),
            "a prefix is a different key, not the same one"
        );

        // Hashable so it can key a `HashMap`; equal values must hash equally.
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let h = |k: &PrivateKey| {
            let mut s = DefaultHasher::new();
            k.hash(&mut s);
            s.finish()
        };
        assert_eq!(h(&key), h(&PrivateKey::new(bytes)));
        assert_eq!(
            key.bytes().len(),
            3,
            "`bytes` is a view, not a copy of the length"
        );
    }
}
