//! A public key.
//!
//! Mirrors `crypto/src/main/scala/coop/rchain/crypto/PublicKey.scala`.

use std::cmp::Ordering;

/// A public key, as a raw byte array.
///
/// The inner bytes are **private**: the only ways to observe them are the read-only [`bytes`]
/// accessor and the boundary `as_bytes`-style conversions, so the raw key cannot be mutated in
/// place mid-domain (mirrors the refinement "no type escape" rule; the Scala `ByteString` is
/// variable-length, so there is no fixed-width `TryFrom` here).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct PublicKey(Vec<u8>);

impl PublicKey {
    /// Construct from raw bytes.
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// The raw key bytes (read-only view).
    pub fn bytes(&self) -> &[u8] {
        &self.0
    }
}

// The Scala `Sorting.publicKeyOrdering` orders public keys by signed-byte lexicographic comparison.
impl PartialOrd for PublicKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PublicKey {
    fn cmp(&self, other: &Self) -> Ordering {
        crate::util::sorting::compare_byte_arrays(&self.0, &other.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The public key is its bytes, and `bytes()` is a read-only view of them.
    #[test]
    fn a_public_key_is_its_bytes() {
        let key = PublicKey::new(vec![0xAB, 0xCD]);
        assert_eq!(key.bytes(), &[0xAB, 0xCD]);
        assert_eq!(key, PublicKey::new(vec![0xAB, 0xCD]));
        assert_ne!(key, PublicKey::new(vec![0xAB]));
    }

    /// The order is the Scala `Sorting.publicKeyOrdering`: **signed**-byte lexicographic, so a key
    /// whose first byte is `0x80..=0xFF` precedes one starting `0x00..=0x7F`. This is not cosmetic —
    /// the order reaches the sorted validator/bond sets, where an unsigned comparison would produce
    /// a different canonical order and a different state hash.
    #[test]
    fn the_order_is_signed_byte_lexicographic() {
        let high = PublicKey::new(vec![0xFF]);
        let low = PublicKey::new(vec![0x00]);
        assert!(high < low, "0xFF is −1 and sorts before 0x00");

        // A prefix sorts first, and the first differing byte decides.
        assert!(PublicKey::new(vec![0x01]) < PublicKey::new(vec![0x01, 0x02]));
        assert!(PublicKey::new(vec![0x01, 0x02]) < PublicKey::new(vec![0x01, 0x03]));

        // `sort` must produce the same order the comparison defines, not the unsigned one.
        let mut keys = vec![
            PublicKey::new(vec![0x00]),
            PublicKey::new(vec![0xFF]),
            PublicKey::new(vec![0x7F]),
            PublicKey::new(vec![0x80]),
        ];
        keys.sort();
        let order: Vec<u8> = keys.iter().map(|k| k.bytes()[0]).collect();
        assert_eq!(
            order,
            vec![0x80, 0xFF, 0x00, 0x7F],
            "−128, −1, 0, 127 — the signed order, not 0x00, 0x7F, 0x80, 0xFF"
        );
    }

    /// `PartialOrd` agrees with `Ord` (it is defined as `Some(cmp)`), so the two cannot drift and
    /// sorting is a total order.
    #[test]
    fn partial_ord_agrees_with_ord() {
        let a = PublicKey::new(vec![0x10, 0x00]);
        let b = PublicKey::new(vec![0x10, 0xFF]);
        assert_eq!(a.partial_cmp(&b), Some(Ordering::Greater));
        assert_eq!(a.partial_cmp(&a), Some(Ordering::Equal));
        assert_eq!(a.cmp(&b), a.partial_cmp(&b).expect("total"));
    }
}
