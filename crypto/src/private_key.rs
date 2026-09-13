//! A private key.
//!
//! Mirrors `crypto/src/main/scala/coop/rchain/crypto/PrivateKey.scala`.

/// A private key, as a raw byte array.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct PrivateKey(pub Vec<u8>);

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
        assert_eq!(key.bytes().len(), 3, "`bytes` is a view, not a copy of the length");
    }
}
